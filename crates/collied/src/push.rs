use std::collections::VecDeque;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::Context;
use apns_h2::request::payload::PayloadLike;
use apns_h2::{
    Client, ClientConfig, CollapseId, Endpoint, ErrorReason, NotificationOptions, Priority,
    PushType,
};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use futures_util::future::BoxFuture;
use protocol::{
    ActivityId, ApnsEnvironment, Approval, ApprovalId, Decision, NotificationKey, PushToken,
    TerminalId,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::config::{self, ApnsConfig, ApnsKey};
#[cfg(target_os = "macos")]
use crate::keychain;
use crate::peers::{self, Store};

const QUEUE: usize = 64;
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
pub const CATEGORY: &str = "APPROVAL";
pub const MAX_ACTIVITIES: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub stable_id: String,
    pub token: PushToken,
    pub environment: ApnsEnvironment,
    // Absent from stores written before the alert context was encrypted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_key: Option<NotificationKey>,
    pub registered_at: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mute_done: bool,
}

/// A Live Activity update token, bound to the paired device that registered it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activity {
    pub stable_id: String,
    pub activity_id: ActivityId,
    pub terminal_id: TerminalId,
    pub token: PushToken,
    pub environment: ApnsEnvironment,
    /// When the activity first registered: re-registering it keeps this, since ActivityKit
    /// counts its 8 h from the start.
    pub registered_at: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shows_approvals: bool,
}

impl Activity {
    /// The token as a delivery target, with no notification key.
    pub fn target(&self) -> Device {
        Device {
            stable_id: self.stable_id.clone(),
            token: self.token.clone(),
            environment: self.environment,
            notification_key: None,
            registered_at: self.registered_at,
            mute_done: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Devices {
    pub devices: Vec<Device>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activities: Vec<Activity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Alert,
    LiveActivity { urgent: bool },
    Background,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub payload: Value,
    pub collapse_id: Option<String>,
    pub expiration: Option<u64>,
    /// Sealed per device into `enc`, never sent in clear: `(approval_id, context)`.
    pub context: Option<(String, String)>,
    pub delivery: Delivery,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Headers {
    pub push_type: PushType,
    pub topic: String,
    pub priority: u8,
    pub expiration: Option<u64>,
    pub collapse_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ActivityError {
    #[error("peer is no longer authorized")]
    NotPaired,
    #[error("no push registration for this device")]
    NoDevice,
    #[error("at most {MAX_ACTIVITIES} Live Activities per device")]
    TooMany,
    #[error("could not store the token")]
    Store,
}

enum Job {
    Alert(Vec<Device>, Alert),
    Activity(Activity, Alert, Option<Box<Routed>>),
    Routed(Box<Routed>),
}

/// An approval alert held back from a device that follows the approval's terminal, for
/// `activity::Live` to deliver on that device's Live Activity instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Routed {
    pub device: Device,
    pub terminal_id: TerminalId,
    pub approval_id: ApprovalId,
    pub alert: Alert,
    /// False for a reissue that `alert_due` keeps quiet: the activity only learns the new id.
    pub due: bool,
}

#[derive(Serialize)]
struct Sealed<'a> {
    v: u32,
    body: &'a str,
}

/// `nonce || ChaCha20-Poly1305(key, nonce, aad = approval_id, {"v":1,"body":...}) || tag`,
/// base64 with padding, as ColliePush expects it.
pub fn seal(
    key: &NotificationKey,
    nonce: [u8; 12],
    approval_id: &str,
    body: &str,
) -> Option<String> {
    let key = Zeroizing::new(URL_SAFE_NO_PAD.decode(key.as_str()).ok()?);
    let cipher = ChaCha20Poly1305::new_from_slice(&key).ok()?;
    let plaintext = Zeroizing::new(serde_json::to_vec(&Sealed { v: 1, body }).ok()?);
    let sealed = cipher
        .encrypt(
            &nonce.into(),
            Payload {
                msg: &plaintext,
                aad: approval_id.as_bytes(),
            },
        )
        .ok()?;
    Some(STANDARD.encode([nonce.as_slice(), &sealed].concat()))
}

/// `seal` under a fresh random nonce.
pub fn seal_fresh(key: &NotificationKey, approval_id: &str, body: &str) -> Option<String> {
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).ok()?;
    seal(key, nonce, approval_id, body)
}

impl Alert {
    pub fn headers(&self, bundle_id: &str) -> Headers {
        let (push_type, topic, priority) = match self.delivery {
            Delivery::Alert => (PushType::Alert, bundle_id.to_owned(), 10),
            Delivery::LiveActivity { urgent } => (
                PushType::LiveActivity,
                format!("{bundle_id}.push-type.liveactivity"),
                if urgent { 10 } else { 5 },
            ),
            Delivery::Background => (PushType::Background, bundle_id.to_owned(), 5),
        };
        Headers {
            push_type,
            topic,
            priority,
            expiration: self.expiration,
            collapse_id: self.collapse_id.clone(),
        }
    }

    /// The alert as one device receives it: the context sealed with that device's key
    /// under a fresh nonce, or the plaintext fallback alone.
    pub fn for_device(&self, device: &Device) -> Alert {
        let mut alert = Alert {
            context: None,
            ..self.clone()
        };
        if let (Some((approval_id, body)), Some(key)) = (&self.context, &device.notification_key) {
            match seal_fresh(key, approval_id, body) {
                Some(enc) => alert.payload["enc"] = json!(enc),
                None => {
                    tracing::warn!(peer = %device.stable_id, "could not seal the alert context")
                }
            }
        }
        alert
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Rejection {
    #[error("device token is no longer registered")]
    Unregistered,
    #[error("bad device token")]
    BadDeviceToken,
    #[error("{0}")]
    Other(String),
}

pub type Paired = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub trait Sender: Send + Sync {
    fn send<'a>(
        &'a self,
        device: &'a Device,
        alert: &'a Alert,
    ) -> BoxFuture<'a, Result<(), Rejection>>;
}

/// The notification and the Live Activity carry Approve and Deny only for an approval
/// that offers both.
fn approve_or_deny(a: &Approval) -> bool {
    a.options.contains(&Decision::Approve) && a.options.contains(&Decision::Deny)
}

/// Carries labels only in clear: never the snippet, the nonce or any terminal text, since
/// the payload transits Apple. `title` must not come from a terminal title either.
/// `context` only leaves sealed to each device's notification key.
pub fn approval_alert(a: &Approval, title: &str, node_id: &str, context: &str) -> Alert {
    let mut aps = json!({
        "alert": {
            "title": title,
            "body": format!("Blocked in {}", a.workspace_label),
        },
        "thread-id": a.terminal_id.as_str(),
        "mutable-content": 1,
    });
    if approve_or_deny(a) {
        aps["category"] = json!(CATEGORY);
    }
    Alert {
        payload: json!({
            "aps": aps,
            "approval_id": a.approval_id.as_str(),
            "node_id": node_id,
        }),
        // One alert per terminal: a reissued approval replaces the dead one.
        collapse_id: Some(a.terminal_id.as_str().to_owned()),
        expiration: Some(a.expires_at_ms / 1000),
        context: (!context.is_empty())
            .then(|| (a.approval_id.as_str().to_owned(), context.to_owned())),
        delivery: Delivery::Alert,
    }
}

/// Tells the app to remove an approval's delivered alert once its agent moved on: the two
/// lookup keys only.
pub fn approval_clear(node_id: &str, approval_id: &ApprovalId) -> Alert {
    Alert {
        payload: json!({
            "aps": {"content-available": 1},
            "approval_id": approval_id.as_str(),
            "node_id": node_id,
        }),
        collapse_id: None,
        // Never stored: APNs keeps one undelivered push per app, so a stored clear would
        // displace a pending approval alert.
        expiration: Some(0),
        context: None,
        delivery: Delivery::Background,
    }
}

/// Labels only, like an approval alert, and no `enc`: nothing about the turn is sent.
pub fn done_alert(terminal_id: &TerminalId, title: &str, workspace: &str, node_id: &str) -> Alert {
    Alert {
        payload: json!({
            "aps": {
                "alert": {"title": title, "body": format!("Done in {workspace}")},
                "thread-id": terminal_id.as_str(),
            },
            "node_id": node_id,
            "terminal_id": terminal_id.as_str(),
        }),
        collapse_id: Some(terminal_id.as_str().to_owned()),
        // Never stored, as `approval_clear`: a stored one would displace a pending approval alert.
        expiration: Some(0),
        context: None,
        delivery: Delivery::Alert,
    }
}

pub fn test_alert() -> Alert {
    Alert {
        payload: json!({
            "aps": {
                "alert": {"title": "collie", "body": "Test notification from collied"},
                "thread-id": "collie-test",
            },
        }),
        collapse_id: None,
        expiration: None,
        context: None,
        delivery: Delivery::Alert,
    }
}

pub struct Push {
    path: PathBuf,
    devices: Mutex<Devices>,
    queue: Option<mpsc::Sender<Job>>,
    paired: Paired,
    routed: Mutex<Vec<Routed>>,
}

impl Push {
    pub fn open(
        path: PathBuf,
        sender: Option<Arc<dyn Sender>>,
        paired: Paired,
    ) -> Result<Arc<Self>, peers::Error> {
        let devices = peers::load_json(&path)?;
        let (queue, worker) = match sender {
            Some(s) => {
                let (tx, rx) = mpsc::channel(QUEUE);
                (Some(tx), Some((rx, s)))
            }
            None => (None, None),
        };
        let push = Arc::new(Self {
            path,
            devices: Mutex::new(devices),
            queue,
            paired: paired.clone(),
            routed: Mutex::new(Vec::new()),
        });
        if let Some((rx, sender)) = worker {
            tokio::spawn(deliver(Arc::downgrade(&push), rx, sender, paired));
        }
        Ok(push)
    }

    pub fn devices(&self) -> Vec<Device> {
        self.lock().devices.clone()
    }

    pub fn register(
        &self,
        stable_id: &str,
        token: PushToken,
        environment: ApnsEnvironment,
        notification_key: NotificationKey,
        mute_done: bool,
    ) -> Result<(), peers::Error> {
        // Checked under the devices lock: revoke removes the peer before its `forget`
        // takes this lock, so a racing registration is either skipped here or forgotten.
        self.update(|d| {
            if !(self.paired)(stable_id) {
                return;
            }
            d.devices
                .retain(|x| x.stable_id != stable_id && x.token != token);
            d.devices.push(Device {
                stable_id: stable_id.to_owned(),
                token,
                environment,
                notification_key: Some(notification_key),
                registered_at: crate::now_ms(),
                mute_done,
            });
        })
    }

    pub fn activities(&self) -> Vec<Activity> {
        self.lock().activities.clone()
    }

    /// Replaces the device's activity with the same id or terminal: the phone keeps one
    /// activity per followed agent, and an old one it never ended must not hold a slot.
    /// The environment is the one the device registered with `push.register`.
    pub fn register_activity(
        &self,
        stable_id: &str,
        activity_id: ActivityId,
        terminal_id: TerminalId,
        token: PushToken,
        shows_approvals: bool,
    ) -> Result<(), ActivityError> {
        let mut outcome = Ok(());
        // Checked under the devices lock, as in `register`.
        self.update(|d| {
            if !(self.paired)(stable_id) {
                outcome = Err(ActivityError::NotPaired);
                return;
            }
            let Some(environment) = d
                .devices
                .iter()
                .find(|x| x.stable_id == stable_id)
                .map(|x| x.environment)
            else {
                outcome = Err(ActivityError::NoDevice);
                return;
            };
            let registered_at = d
                .activities
                .iter()
                .find(|x| x.stable_id == stable_id && x.activity_id == activity_id)
                .map_or_else(crate::now_ms, |x| x.registered_at);
            d.activities.retain(|x| {
                !(x.stable_id == stable_id
                    && (x.activity_id == activity_id || x.terminal_id == terminal_id))
                    && x.token != token
            });
            if d.activities
                .iter()
                .filter(|x| x.stable_id == stable_id)
                .count()
                >= MAX_ACTIVITIES
            {
                outcome = Err(ActivityError::TooMany);
                return;
            }
            d.activities.push(Activity {
                stable_id: stable_id.to_owned(),
                activity_id,
                terminal_id,
                token,
                environment,
                registered_at,
                shows_approvals,
            });
        })
        .map_err(|e| {
            tracing::error!(error = %e, "push.activity_token");
            ActivityError::Store
        })?;
        outcome
    }

    /// Only the device that registered an activity can end it. Returns the removed one.
    pub fn end_activity(
        &self,
        stable_id: &str,
        activity_id: &ActivityId,
    ) -> Result<Option<Activity>, peers::Error> {
        let mut ended = None;
        self.update(|d| {
            if let Some(i) = d
                .activities
                .iter()
                .position(|x| x.stable_id == stable_id && x.activity_id == *activity_id)
            {
                ended = Some(d.activities.remove(i));
            }
        })?;
        Ok(ended)
    }

    /// Removes the activities registered before `cutoff_ms` and returns them.
    pub fn expire_activities(&self, cutoff_ms: u64) -> Result<Vec<Activity>, peers::Error> {
        let mut expired = Vec::new();
        self.update(|d| {
            d.activities.retain(|x| {
                let keep = x.registered_at >= cutoff_ms;
                if !keep {
                    expired.push(x.clone());
                }
                keep
            });
        })?;
        Ok(expired)
    }

    pub fn forget(&self, stable_id: &str) -> Result<(), peers::Error> {
        self.update(|d| {
            d.devices.retain(|x| x.stable_id != stable_id);
            d.activities.retain(|x| x.stable_id != stable_id);
        })
    }

    pub fn retain_paired(&self, store: &Store) -> Result<(), peers::Error> {
        self.update(|d| {
            d.devices.retain(|x| store.get(&x.stable_id).is_some());
            d.activities.retain(|x| store.get(&x.stable_id).is_some());
        })
    }

    pub fn notify(&self, alert: Alert) {
        self.enqueue(Job::Alert(self.devices(), alert));
    }

    pub fn notify_done(&self, alert: Alert) {
        let devices: Vec<Device> = self
            .devices()
            .into_iter()
            .filter(|d| !d.mute_done)
            .collect();
        if !devices.is_empty() {
            self.enqueue(Job::Alert(devices, alert));
        }
    }

    /// Sends `fallback` as an approval alert if `alert` reaches no Live Activity.
    pub fn notify_activity(&self, activity: Activity, alert: Alert, fallback: Option<Routed>) {
        self.enqueue(Job::Activity(activity, alert, fallback.map(Box::new)));
    }

    /// A routed approval as an approval alert, unless its device already got that one.
    pub fn notify_routed(&self, routed: Routed) {
        self.enqueue(Job::Routed(Box::new(routed)));
    }

    /// The approval alert for every device, except that a device whose Live Activity for
    /// the approval's terminal shows approvals gets it through `take_routed` instead. With
    /// `due` false only those activities hear of the approval.
    pub fn notify_approval(&self, approval: &Approval, alert: Alert, due: bool) {
        if self.queue.is_none() {
            return;
        }
        let Devices {
            devices,
            activities,
        } = self.lock().clone();
        let mut regular = Vec::new();
        let mut routed = Vec::new();
        for device in devices {
            if approve_or_deny(approval)
                && activities.iter().any(|a| {
                    a.shows_approvals
                        && a.stable_id == device.stable_id
                        && a.terminal_id == approval.terminal_id
                })
            {
                routed.push(Routed {
                    device,
                    terminal_id: approval.terminal_id.clone(),
                    approval_id: approval.approval_id.clone(),
                    alert: alert.clone(),
                    due,
                });
            } else if due {
                regular.push(device);
            }
        }
        self.routed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(routed);
        if !regular.is_empty() {
            self.enqueue(Job::Alert(regular, alert));
        }
    }

    pub fn take_routed(&self) -> Vec<Routed> {
        std::mem::take(&mut *self.routed.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn enqueue(&self, job: Job) {
        if let Some(q) = &self.queue
            && q.try_send(job).is_err()
        {
            tracing::warn!("APNs queue full or closed; alert dropped");
        }
    }

    fn drop_token(&self, token: &PushToken) {
        if let Err(e) = self.update(|d| {
            d.devices.retain(|x| x.token != *token);
            d.activities.retain(|x| x.token != *token);
        }) {
            tracing::error!(error = %e, "could not remove a dead APNs token");
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Devices> {
        self.devices.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn update(&self, f: impl FnOnce(&mut Devices)) -> Result<(), peers::Error> {
        let mut devices = self.lock();
        let mut next = devices.clone();
        f(&mut next);
        if next != *devices {
            peers::save_json(&self.path, &next)?;
            *devices = next;
        }
        Ok(())
    }
}

// `paired` is checked on every send: a token stored by a registration that raced a revoke
// must never receive an alert.
async fn deliver(
    push: Weak<Push>,
    mut rx: mpsc::Receiver<Job>,
    sender: Arc<dyn Sender>,
    paired: Paired,
) {
    // Routed approvals already alerted, so a late fallback never repeats an early one.
    let mut alerted: VecDeque<(String, ApprovalId)> = VecDeque::new();
    while let Some(job) = rx.recv().await {
        let Some(push) = push.upgrade() else {
            return;
        };
        let routed = match job {
            Job::Alert(devices, alert) => {
                for device in devices {
                    let alert = alert.for_device(&device);
                    send(&push, &*sender, &paired, &device, &alert).await;
                }
                continue;
            }
            Job::Activity(activity, alert, fallback) => {
                if send(&push, &*sender, &paired, &activity.target(), &alert).await {
                    continue;
                }
                let Some(r) = fallback else {
                    continue;
                };
                tracing::info!(peer = %r.device.stable_id, "Live Activity alert failed; sending the approval alert");
                r
            }
            Job::Routed(r) => r,
        };
        let key = (routed.device.stable_id.clone(), routed.approval_id.clone());
        if alerted.contains(&key) {
            continue;
        }
        if alerted.len() == QUEUE {
            alerted.pop_front();
        }
        alerted.push_back(key);
        let alert = routed.alert.for_device(&routed.device);
        send(&push, &*sender, &paired, &routed.device, &alert).await;
    }
}

async fn send(
    push: &Push,
    sender: &dyn Sender,
    paired: &Paired,
    device: &Device,
    alert: &Alert,
) -> bool {
    if !paired(&device.stable_id) {
        return false;
    }
    match sender.send(device, alert).await {
        Ok(()) => true,
        Err(e @ (Rejection::Unregistered | Rejection::BadDeviceToken)) => {
            tracing::info!(peer = %device.stable_id, reason = %e, "removing APNs token");
            push.drop_token(&device.token);
            false
        }
        Err(e) => {
            tracing::warn!(peer = %device.stable_id, error = %e, "APNs send failed");
            false
        }
    }
}

pub struct Apns {
    sandbox: Client,
    production: Client,
    bundle_id: String,
}

impl Apns {
    pub fn new(cfg: &ApnsConfig) -> anyhow::Result<Self> {
        check_ids(cfg)?;
        let key = load_key(cfg)?;
        Self::with_key(cfg, &key)
    }

    /// Signs a provider JWT with the key right away, without any network use. Fails
    /// unless the key is a PKCS#8 P-256 (ES256) key.
    pub fn with_key(cfg: &ApnsConfig, key: &[u8]) -> anyhow::Result<Self> {
        let client = |endpoint| {
            Client::token(
                key,
                cfg.key_id.as_str(),
                cfg.team_id.as_str(),
                ClientConfig {
                    request_timeout: Some(SEND_TIMEOUT),
                    ..ClientConfig::new(endpoint)
                },
            )
            .context("APNs key")
        };
        Ok(Self {
            sandbox: client(Endpoint::Sandbox)?,
            production: client(Endpoint::Production)?,
            bundle_id: cfg.bundle_id.clone(),
        })
    }
}

struct Request<'a> {
    body: &'a Value,
    token: &'a str,
    options: NotificationOptions<'a>,
}

impl std::fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Request(<redacted>)")
    }
}

impl Serialize for Request<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.body.serialize(s)
    }
}

impl PayloadLike for Request<'_> {
    fn get_device_token(&self) -> &str {
        self.token
    }

    fn get_options(&self) -> &NotificationOptions<'_> {
        &self.options
    }
}

impl Sender for Apns {
    fn send<'a>(
        &'a self,
        device: &'a Device,
        alert: &'a Alert,
    ) -> BoxFuture<'a, Result<(), Rejection>> {
        Box::pin(async move {
            let headers = alert.headers(&self.bundle_id);
            let collapse_id = match headers.collapse_id.as_deref().map(CollapseId::new) {
                Some(Ok(c)) => Some(c),
                Some(Err(e)) => return Err(Rejection::Other(e.to_string())),
                None => None,
            };
            let request = Request {
                body: &alert.payload,
                token: device.token.as_str(),
                options: NotificationOptions {
                    apns_push_type: Some(headers.push_type),
                    apns_priority: Some(if headers.priority == 10 {
                        Priority::High
                    } else {
                        Priority::Normal
                    }),
                    apns_expiration: headers.expiration,
                    apns_collapse_id: collapse_id,
                    apns_topic: Some(&headers.topic),
                    ..Default::default()
                },
            };
            let client = match device.environment {
                ApnsEnvironment::Sandbox => &self.sandbox,
                ApnsEnvironment::Production => &self.production,
            };
            match client.send(request).await {
                Ok(_) => Ok(()),
                Err(apns_h2::Error::ResponseError(r)) => {
                    Err(match r.error.as_ref().map(|e| &e.reason) {
                        Some(ErrorReason::Unregistered) => Rejection::Unregistered,
                        Some(ErrorReason::BadDeviceToken) => Rejection::BadDeviceToken,
                        Some(reason) => Rejection::Other(format!("{} {reason}", r.code)),
                        None => Rejection::Other(format!("HTTP {}", r.code)),
                    })
                }
                Err(e) => Err(Rejection::Other(e.to_string())),
            }
        })
    }
}

fn apple_id(s: &str) -> bool {
    s.len() == 10
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

pub fn check_ids(cfg: &ApnsConfig) -> anyhow::Result<()> {
    anyhow::ensure!(
        apple_id(&cfg.key_id),
        "key_id must be the 10-character key ID"
    );
    anyhow::ensure!(
        apple_id(&cfg.team_id),
        "team_id must be the 10-character team ID"
    );
    anyhow::ensure!(
        !cfg.bundle_id.is_empty()
            && cfg
                .bundle_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
        "bundle_id is not a bundle identifier"
    );
    Ok(())
}

pub fn load_key(cfg: &ApnsConfig) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    match &cfg.key {
        ApnsKey::File(path) => Ok(read_key(path)?.0),
        ApnsKey::Keychain => load_keychain(cfg),
        ApnsKey::SystemdCreds => load_credential(cfg),
    }
}

#[cfg(target_os = "macos")]
fn load_keychain(cfg: &ApnsConfig) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    keychain::read(None, &cfg.key_id)
        .with_context(|| keychain_item(&cfg.key_id))?
        .with_context(|| {
            format!(
                "{} not found: run `collied apns import <AuthKey.p8>`",
                keychain_item(&cfg.key_id)
            )
        })
}

#[cfg(not(target_os = "macos"))]
fn load_keychain(_: &ApnsConfig) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    anyhow::bail!("the Keychain is macOS only")
}

#[cfg(target_os = "linux")]
fn load_credential(cfg: &ApnsConfig) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    let path = credential_path(&cfg.key_id)?;
    let (credential, _) = read_key(&path).with_context(|| {
        format!(
            "{}: run `collied apns import <AuthKey.p8>`",
            credential_item(&cfg.key_id)
        )
    })?;
    crate::creds::decrypt(&crate::creds::name(&cfg.key_id), &credential)
        .with_context(|| format!("decrypt {}", path.display()))
}

#[cfg(not(target_os = "linux"))]
fn load_credential(_: &ApnsConfig) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    anyhow::bail!("systemd-creds is Linux only")
}

#[cfg(target_os = "macos")]
pub fn keychain_item(key_id: &str) -> String {
    format!("Keychain item {}/{key_id}", keychain::SERVICE)
}

#[cfg(target_os = "linux")]
pub fn credential_path(key_id: &str) -> anyhow::Result<PathBuf> {
    Ok(crate::creds::path(
        &config::data_dir()?.join(config::APNS_DIR),
        key_id,
    ))
}

#[cfg(target_os = "linux")]
pub fn credential_item(key_id: &str) -> String {
    match credential_path(key_id) {
        Ok(p) => format!("systemd credential {}", p.display()),
        Err(_) => format!("systemd credential {key_id}.cred"),
    }
}

/// Opened without following symlinks and checked on the open descriptor: a regular file
/// owned by the current user with no group or world bits.
pub fn read_key(path: &Path) -> anyhow::Result<(Zeroizing<Vec<u8>>, rustix::fs::Stat)> {
    use rustix::fs::{FileType, Mode, OFlags};
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .with_context(|| format!("open {}", path.display()))?;
    let st = rustix::fs::fstat(&fd)?;
    anyhow::ensure!(
        FileType::from_raw_mode(st.st_mode) == FileType::RegularFile
            && st.st_uid == rustix::process::geteuid().as_raw()
            && st.st_mode & 0o077 == 0,
        "{}: must be a regular file with mode 0600 owned by the current user",
        path.display()
    );
    let mut key = Zeroizing::new(Vec::new());
    std::fs::File::from(fd).read_to_end(&mut key)?;
    Ok((key, st))
}

/// The `[apns]` ids, and the key read from `path` with the `key_path` checks and checked
/// to be an APNs ES256 key named for that key ID.
fn import_source(
    config_path: &Path,
    explicit: bool,
    path: &Path,
) -> anyhow::Result<(ApnsConfig, Zeroizing<Vec<u8>>, rustix::fs::Stat)> {
    let config = config::load(config_path, explicit)?.unwrap_or_default();
    let cfg = config.apns.with_context(|| {
        format!(
            "no [apns] section in {}: add key_id, team_id and bundle_id first",
            config_path.display()
        )
    })?;
    check_ids(&cfg)?;
    if let Some(named) = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("AuthKey_")?.strip_suffix(".p8"))
    {
        anyhow::ensure!(
            named == cfg.key_id,
            "{} is key {named}, but [apns] key_id is {}",
            path.display(),
            cfg.key_id
        );
    }
    let (key, st) = read_key(path)?;
    Apns::with_key(&cfg, &key).context("not an APNs ES256 key")?;
    Ok((cfg, key, st))
}

/// Points `[apns]` at `key` unless it already is; false when that has to be done by hand.
fn switch_config(config_path: &Path, cfg: &ApnsConfig, key: &ApnsKey) -> anyhow::Result<bool> {
    if cfg.key == *key {
        return Ok(true);
    }
    let text = std::fs::read_to_string(config_path)?;
    let line = match key {
        ApnsKey::File(p) => format!("key_path = {:?}", p.display().to_string()),
        k => format!("key = \"{}\"", k.config_value().unwrap_or_default()),
    };
    match config::set_apns_key(&text, key, true) {
        Some(new) => {
            config::rewrite(config_path, &new)?;
            println!("{}: [apns] now has {line}", config_path.display());
            Ok(true)
        }
        None => {
            println!(
                "{}: could not update [apns] automatically; set {line} there yourself (and remove any other key or key_path line)",
                config_path.display()
            );
            Ok(false)
        }
    }
}

// st_dev is an i32 on macOS and a u64 on Linux.
#[allow(clippy::unnecessary_cast)]
fn dev_ino(st: &rustix::fs::Stat) -> (u64, u64) {
    (st.st_dev as u64, st.st_ino as u64)
}

/// Offers to delete the imported file, only if it is still the inode that was read.
fn offer_delete(path: &Path, st: &rustix::fs::Stat) -> anyhow::Result<()> {
    // Discard anything typed earlier so a stray "y" cannot delete the file.
    let _ = rustix::termios::tcflush(std::io::stdin(), rustix::termios::QueueSelector::IFlush);
    print!("Delete {}? [y/N] ", path.display());
    std::io::Write::flush(&mut std::io::stdout())?;
    let mut answer = String::new();
    std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut answer)?;
    if !answer.trim().eq_ignore_ascii_case("y") {
        println!("kept {}", path.display());
        return Ok(());
    }
    let now = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        (now.dev(), now.ino()) == dev_ino(st),
        "{} changed since it was read; not deleted",
        path.display()
    );
    std::fs::remove_file(path)?;
    println!("deleted {}", path.display());
    Ok(())
}

/// Moves a `.p8` into the login Keychain, readable without a prompt only by this
/// executable's signing identity, then offers to delete the file.
#[cfg(target_os = "macos")]
pub fn import(config_path: &Path, explicit: bool, path: &Path) -> anyhow::Result<bool> {
    let (cfg, key, st) = import_source(config_path, explicit, path)?;
    // The item trusts the importing binary's designated requirement, so only the
    // installed, signed collied may create it.
    crate::doctor::signed_as_collied()
        .map_err(|e| anyhow::anyhow!("{e}: run the collied installed by `just collied-install`"))?;
    let item = keychain_item(&cfg.key_id);
    match keychain::store(None, &cfg.key_id, &key).with_context(|| format!("store {item}"))? {
        keychain::Stored::Added => println!("stored {item}"),
        keychain::Stored::AlreadyPresent => println!("{item} already exists"),
    }
    let stored = keychain::read(None, &cfg.key_id)
        .with_context(|| format!("read back {item}"))?
        .with_context(|| format!("{item} not found after storing it"))?;
    anyhow::ensure!(
        stored.as_slice() == key.as_slice(),
        "{item} holds a different key; remove it with `security delete-generic-password -s {} -a {}` and import again",
        keychain::SERVICE,
        cfg.key_id
    );
    drop(stored);
    drop(key);
    println!("read back {item}: identical");

    if let ApnsKey::File(_) = cfg.key
        && !switch_config(config_path, &cfg, &ApnsKey::Keychain)?
    {
        return Ok(false);
    }
    offer_delete(path, &st)?;
    Ok(true)
}

/// Encrypts a `.p8` as a systemd user credential (host key, plus the TPM2 when one is
/// usable) in `<data dir>/apns`, or, when systemd-creds is unavailable, copies it there as
/// a 0600 file; then offers to delete the original.
#[cfg(target_os = "linux")]
pub fn import(config_path: &Path, explicit: bool, path: &Path) -> anyhow::Result<bool> {
    use crate::creds;
    let (cfg, key, st) = import_source(config_path, explicit, path)?;
    let apns_dir = config::data_dir()?.join(config::APNS_DIR);
    crate::ensure_private_dir(&config::data_dir()?)?;
    crate::ensure_private_dir(&apns_dir)?;
    let name = creds::name(&cfg.key_id);
    let cred_path = creds::path(&apns_dir, &cfg.key_id);
    let target = match creds::encrypt(&name, &key) {
        Ok(credential) => {
            let item = credential_item(&cfg.key_id);
            match read_key(&cred_path) {
                Ok((existing, _)) => {
                    let stored = creds::decrypt(&name, &existing).map_err(|e| {
                        anyhow::anyhow!(
                            "{item} no longer decrypts on this machine ({e:#}): remove it and import again"
                        )
                    })?;
                    anyhow::ensure!(
                        stored.as_slice() == key.as_slice(),
                        "{item} holds a different key; remove it and import again"
                    );
                    drop(stored);
                    let (old, new) = (creds::seal(&existing), creds::seal(&credential));
                    if new.has_tpm2() && !old.has_tpm2() {
                        let check = creds::decrypt(&name, &credential)
                            .context("decrypt the new credential")?;
                        anyhow::ensure!(
                            check.as_slice() == key.as_slice(),
                            "the new credential decrypts to a different key"
                        );
                        creds::store(&cred_path, &credential)?;
                        println!(
                            "sealed {item} again: {} instead of {}",
                            new.describe(),
                            old.describe()
                        );
                    } else {
                        println!("{item} already exists ({})", old.describe());
                    }
                }
                Err(_) if cred_path.symlink_metadata().is_err() => {
                    let check =
                        creds::decrypt(&name, &credential).context("decrypt the new credential")?;
                    anyhow::ensure!(
                        check.as_slice() == key.as_slice(),
                        "the new credential decrypts to a different key"
                    );
                    creds::store(&cred_path, &credential)?;
                    println!("stored {item} ({})", creds::seal(&credential).describe());
                }
                Err(e) => return Err(e.context(item)),
            }
            let (stored, _) = read_key(&cred_path)?;
            let back = creds::decrypt(&name, &stored).context("read back")?;
            anyhow::ensure!(
                back.as_slice() == key.as_slice(),
                "{item} decrypts to a different key"
            );
            println!("read back {item}: identical");
            ApnsKey::SystemdCreds
        }
        Err(e) => {
            // Never trade a credential that exists for a plaintext copy on a passing error.
            if cred_path.symlink_metadata().is_ok() {
                return Err(e.context(format!(
                    "{} exists but systemd-creds failed; not falling back to a plaintext file",
                    credential_item(&cfg.key_id)
                )));
            }
            let dest = apns_dir.join(format!("AuthKey_{}.p8", cfg.key_id));
            println!("systemd-creds is unavailable ({e:#})");
            println!(
                "falling back to a 0600 file, protected by its mode only: {}",
                dest.display()
            );
            let same =
                std::fs::symlink_metadata(&dest).is_ok_and(|m| (m.dev(), m.ino()) == dev_ino(&st));
            if !same {
                write_private(&dest, &key)?;
            }
            let (back, _) = read_key(&dest)?;
            anyhow::ensure!(
                back.as_slice() == key.as_slice(),
                "{} holds a different key; remove it and import again",
                dest.display()
            );
            if same {
                return switch_config(config_path, &cfg, &ApnsKey::File(dest));
            }
            ApnsKey::File(dest)
        }
    };
    drop(key);
    if !switch_config(config_path, &cfg, &target)? {
        return Ok(false);
    }
    offer_delete(path, &st)?;
    Ok(true)
}

/// A new 0600 file holding `data`; an existing file is kept and must hold the same bytes.
#[cfg(target_os = "linux")]
fn write_private(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
    {
        Ok(mut f) => {
            f.write_all(data)?;
            f.sync_all()?;
            println!("stored {}", path.display());
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            println!("{} already exists", path.display());
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("create {}", path.display())),
    }
}

pub async fn send_test(data_dir: &Path, cfg: &ApnsConfig) -> anyhow::Result<bool> {
    let apns = Apns::new(cfg)?;
    let store = peers::load(&data_dir.join(config::PEERS_FILE))?;
    let devices: Devices = peers::load_json(&data_dir.join(config::PUSH_FILE))?;
    let alert = test_alert();
    let mut ok = true;
    let mut sent = 0;
    for d in &devices.devices {
        let Some(peer) = store.get(&d.stable_id) else {
            continue;
        };
        let label = crate::printable(&peer.label);
        match apns.send(d, &alert).await {
            Ok(()) => {
                sent += 1;
                println!("sent to {label} ({:?})", d.environment);
            }
            Err(e) => {
                ok = false;
                println!("{label} ({:?}): {e}", d.environment);
            }
        }
    }
    if sent == 0 && ok {
        println!("no registered device: open the app on a paired phone first");
        return Ok(false);
    }
    Ok(ok)
}

#[cfg(test)]
pub mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use protocol::{ApprovalId, Nonce, TerminalId};

    use super::*;

    // Throwaway P-256 key generated for these tests; it belongs to no Apple account.
    pub const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgj6IYYL0j3crLL2nG
Bap7WFtnJXjk8LolPAeMAK5sGsihRANCAARdncq60MvYDS4pw5g/pzIRZGwXi4Xr
UVsdPckAuSvGZZ/iBp9pjFsmPhLMtTEWs9uKc4/mI+REKuFUluqakETu
-----END PRIVATE KEY-----
";

    fn token(c: char) -> PushToken {
        PushToken::new(c.to_string().repeat(64)).unwrap()
    }

    fn approval(options: Vec<Decision>) -> Approval {
        Approval {
            approval_id: ApprovalId::new("AAAAAAAAAAAAAAAAAAAAAA").unwrap(),
            terminal_id: TerminalId::new("term_1").unwrap(),
            agent_label: "TITLE from the terminal".into(),
            workspace_label: "api".into(),
            snippet: "rm -rf build SNIPPET".into(),
            tool: None,
            options,
            choices: vec![protocol::ApprovalChoice {
                index: 0,
                label: "CHOICE".into(),
                current: true,
                detail: None,
            }],
            accepts_input: false,
            has_text_field: false,
            supports_note: false,
            nonce: Nonce::new("N".repeat(43)).unwrap(),
            created_at_ms: 1_000_000,
            expires_at_ms: 1_600_000,
        }
    }

    #[test]
    fn approval_payload_carries_labels_only() {
        let a = approval(vec![
            Decision::Approve,
            Decision::ApproveAlways,
            Decision::Deny,
        ]);
        let alert = approval_alert(&a, "api-fixer", "nMAC", "Bash: rm -rf build CONTEXT");
        assert_eq!(
            alert.payload,
            json!({
                "aps": {
                    "alert": {"title": "api-fixer", "body": "Blocked in api"},
                    "category": "APPROVAL",
                    "thread-id": "term_1",
                    "mutable-content": 1,
                },
                "approval_id": "AAAAAAAAAAAAAAAAAAAAAA",
                "node_id": "nMAC",
            })
        );
        assert_eq!(alert.collapse_id.as_deref(), Some("term_1"));
        assert_eq!(alert.expiration, Some(1600));
        let text = alert.payload.to_string();
        assert!(
            !text.contains("SNIPPET")
                && !text.contains("NNNN")
                && !text.contains("TITLE")
                && !text.contains("CHOICE"),
            "{text}"
        );

        let sealed = alert.for_device(&device(Some(key())));
        let enc = sealed.payload["enc"].as_str().unwrap();
        let text = sealed.payload.to_string();
        assert!(
            !text.contains("CONTEXT") && !text.contains("rm -rf") && !text.contains("SNIPPET"),
            "{text}"
        );
        assert_eq!(sealed.context, None);
        assert_eq!(
            open(&key(), enc, a.approval_id.as_str()).unwrap(),
            json!({"v": 1, "body": "Bash: rm -rf build CONTEXT"})
        );
        assert!(
            open(&key(), enc, "another_approval").is_none(),
            "AAD binds the approval"
        );
        let again = alert.for_device(&device(Some(key())));
        assert_ne!(
            again.payload["enc"], sealed.payload["enc"],
            "fresh nonce per send"
        );
        let mut no_enc = sealed.payload.clone();
        no_enc.as_object_mut().unwrap().remove("enc");
        assert_eq!(no_enc, alert.payload);
        assert_eq!(alert.for_device(&device(None)).payload, alert.payload);

        let deny_only = approval_alert(&approval(vec![Decision::Deny]), "t", "nMAC", "");
        assert!(deny_only.payload["aps"].get("category").is_none());
        assert!(
            approval_alert(&approval(vec![]), "t", "nMAC", "").payload["aps"]
                .get("category")
                .is_none()
        );
    }

    #[test]
    fn approval_clear_carries_the_lookup_keys_only() {
        let clear = approval_clear("nMAC", &ApprovalId::new("A".repeat(22)).unwrap());
        assert_eq!(
            clear.payload,
            json!({
                "aps": {"content-available": 1},
                "approval_id": "A".repeat(22),
                "node_id": "nMAC",
            })
        );
        assert_eq!(
            clear.headers("dev.rbstp.collie"),
            Headers {
                push_type: PushType::Background,
                topic: "dev.rbstp.collie".into(),
                priority: 5,
                expiration: Some(0),
                collapse_id: None,
            }
        );
        assert_eq!(clear.for_device(&device(Some(key()))), clear);
    }

    #[test]
    fn done_alert_carries_labels_only_and_is_never_stored() {
        let done = done_alert(&term(1), "api-fixer", "api", "nMAC");
        assert_eq!(
            done.payload,
            json!({
                "aps": {
                    "alert": {"title": "api-fixer", "body": "Done in api"},
                    "thread-id": "term_1",
                },
                "node_id": "nMAC",
                "terminal_id": "term_1",
            })
        );
        assert_eq!(
            done.headers("dev.rbstp.collie"),
            Headers {
                push_type: PushType::Alert,
                topic: "dev.rbstp.collie".into(),
                priority: 10,
                expiration: Some(0),
                collapse_id: Some("term_1".into()),
            }
        );
        assert_eq!(done.for_device(&device(Some(key()))), done);
    }

    pub fn key() -> NotificationKey {
        NotificationKey::new(URL_SAFE_NO_PAD.encode((1..=32).collect::<Vec<u8>>())).unwrap()
    }

    fn device(notification_key: Option<NotificationKey>) -> Device {
        Device {
            stable_id: "nA".into(),
            token: token('a'),
            environment: ApnsEnvironment::Sandbox,
            notification_key,
            registered_at: 0,
            mute_done: false,
        }
    }

    /// What ColliePush does: CryptoKit `ChaChaPoly.SealedBox(combined:)`, AAD = approval_id.
    pub fn open(key: &NotificationKey, enc: &str, approval_id: &str) -> Option<Value> {
        let key = URL_SAFE_NO_PAD.decode(key.as_str()).ok()?;
        let combined = STANDARD.decode(enc).ok()?;
        let (nonce, sealed) = combined.split_at_checked(12)?;
        let plain = ChaCha20Poly1305::new_from_slice(&key)
            .ok()?
            .decrypt(
                nonce.try_into().ok()?,
                Payload {
                    msg: sealed,
                    aad: approval_id.as_bytes(),
                },
            )
            .ok()?;
        serde_json::from_slice(&plain).ok()
    }

    #[test]
    fn shared_test_vector() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/protocol/notification-vector.json"
        );
        let vector: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let nonce: [u8; 12] = std::array::from_fn(|i| 0xA0 + i as u8);
        let enc = seal(&key(), nonce, "apr_test", "Bash: echo hi").unwrap();
        assert_eq!(
            vector["key_hex"],
            "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"
        );
        assert_eq!(vector["nonce_hex"], "a0a1a2a3a4a5a6a7a8a9aaab");
        assert_eq!(vector["approval_id"], "apr_test");
        assert_eq!(vector["plaintext"], r#"{"v":1,"body":"Bash: echo hi"}"#);
        assert_eq!(vector["enc"], enc.as_str(), "{path} is stale");
        assert_eq!(
            open(&key(), &enc, "apr_test").unwrap(),
            json!({"v": 1, "body": "Bash: echo hi"})
        );
        let mut tampered = STANDARD.decode(&enc).unwrap();
        tampered[20] ^= 1;
        assert!(open(&key(), &STANDARD.encode(tampered), "apr_test").is_none());
    }

    #[test]
    fn device_debug_hides_the_key() {
        let text = format!("{:?}", device(Some(key())));
        assert!(text.contains("NotificationKey(<redacted>)"), "{text}");
        assert!(!text.contains(key().as_str()), "{text}");
    }

    struct Mock {
        dead: PushToken,
        busy: Option<PushToken>,
        sent: Mutex<Vec<(String, Alert)>>,
        calls: AtomicUsize,
    }

    impl Mock {
        fn new(dead: PushToken) -> Self {
            Self {
                dead,
                busy: None,
                sent: Mutex::new(Vec::new()),
                calls: AtomicUsize::new(0),
            }
        }

        async fn wait(&self, calls: usize) -> Vec<(String, Alert)> {
            while self.calls.load(Ordering::SeqCst) < calls {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(self.calls.load(Ordering::SeqCst), calls);
            self.sent.lock().unwrap().clone()
        }
    }

    impl Sender for Mock {
        fn send<'a>(
            &'a self,
            device: &'a Device,
            alert: &'a Alert,
        ) -> BoxFuture<'a, Result<(), Rejection>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if device.token == self.dead {
                    return Err(Rejection::Unregistered);
                }
                if self.busy.as_ref() == Some(&device.token) {
                    return Err(Rejection::Other("503 ServiceUnavailable".into()));
                }
                self.sent
                    .lock()
                    .unwrap()
                    .push((device.token.as_str().to_owned(), alert.clone()));
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn store_is_private_and_drops_dead_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("push.json");
        let mock = Arc::new(Mock::new(token('b')));
        let revoked = Arc::new(AtomicBool::new(false));
        let paired: Paired = {
            let revoked = revoked.clone();
            Arc::new(move |id| id != "nRevoked" || !revoked.load(Ordering::SeqCst))
        };
        let push = Push::open(path.clone(), Some(mock.clone()), paired).unwrap();
        push.register("nA", token('a'), ApnsEnvironment::Sandbox, key(), false)
            .unwrap();
        push.register("nB", token('b'), ApnsEnvironment::Production, key(), false)
            .unwrap();
        push.register("nA", token('c'), ApnsEnvironment::Sandbox, key(), false)
            .unwrap();
        push.register(
            "nRevoked",
            token('d'),
            ApnsEnvironment::Sandbox,
            key(),
            false,
        )
        .unwrap();
        revoked.store(true, Ordering::SeqCst);
        push.register(
            "nRevoked",
            token('e'),
            ApnsEnvironment::Sandbox,
            key(),
            false,
        )
        .unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let stored: Devices = peers::load_json(&path).unwrap();
        let tokens: Vec<&str> = stored.devices.iter().map(|d| d.token.as_str()).collect();
        assert_eq!(
            tokens,
            [
                token('b').as_str(),
                token('c').as_str(),
                token('d').as_str()
            ]
        );

        push.notify(test_alert());
        while mock.calls.load(Ordering::SeqCst) < 2 || push.devices().len() == 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let sent = mock.sent.lock().unwrap().clone();
        assert_eq!(sent, [(token('c').as_str().to_owned(), test_alert())]);
        let stored: Devices = peers::load_json(&path).unwrap();
        assert_eq!(
            mock.calls.load(Ordering::SeqCst),
            2,
            "never sent to nRevoked"
        );
        assert_eq!(stored.devices.len(), 2);
        assert_eq!(stored.devices[0].stable_id, "nA");

        push.register("nB", token('b'), ApnsEnvironment::Production, key(), true)
            .unwrap();
        push.notify_done(done_alert(&term(1), "t", "w", "nMAC"));
        let sent = mock.wait(3).await;
        assert_eq!(
            sent[1].0,
            token('c').as_str(),
            "never sent to a muted device"
        );
        assert_eq!(sent[1].1.payload["terminal_id"], "term_1");
        push.register("nA", token('c'), ApnsEnvironment::Sandbox, key(), true)
            .unwrap();
        push.notify_done(done_alert(&term(1), "t", "w", "nMAC"));
        push.notify(test_alert());
        mock.wait(5).await;

        push.forget("nA").unwrap();
        push.forget("nB").unwrap();
        push.forget("nRevoked").unwrap();
        assert!(push.devices().is_empty());
        let reopened = Push::open(path, None, Arc::new(|_| true)).unwrap();
        assert!(reopened.devices().is_empty());
        reopened.notify(test_alert());
    }

    fn aid(i: usize) -> ActivityId {
        ActivityId::new(format!("ACT-{i}")).unwrap()
    }

    fn term(i: usize) -> TerminalId {
        TerminalId::new(format!("term_{i}")).unwrap()
    }

    fn long_token(i: usize) -> PushToken {
        PushToken::new(format!("{i:0>160x}")).unwrap()
    }

    #[tokio::test]
    async fn activity_tokens_are_bound_capped_and_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("push.json");
        let mock = Arc::new(Mock::new(long_token(3)));
        let revoked = Arc::new(AtomicBool::new(false));
        let paired: Paired = {
            let revoked = revoked.clone();
            Arc::new(move |id| id != "nB" || !revoked.load(Ordering::SeqCst))
        };
        let push = Push::open(path.clone(), Some(mock.clone()), paired).unwrap();
        assert_eq!(
            push.register_activity("nA", aid(0), term(0), long_token(0), true),
            Err(ActivityError::NoDevice),
            "push.register comes first"
        );
        push.register("nA", token('a'), ApnsEnvironment::Sandbox, key(), false)
            .unwrap();
        push.register("nB", token('b'), ApnsEnvironment::Production, key(), false)
            .unwrap();
        for i in 0..MAX_ACTIVITIES {
            push.register_activity("nA", aid(i), term(i), long_token(i), true)
                .unwrap();
        }
        assert_eq!(
            push.register_activity("nA", aid(99), term(99), long_token(99), true),
            Err(ActivityError::TooMany)
        );
        let at = |i: usize| {
            push.activities()
                .into_iter()
                .find(|a| a.stable_id == "nA" && a.activity_id == aid(i))
                .map(|a| a.registered_at)
        };
        let first = at(1).unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        push.register_activity("nA", aid(1), term(1), long_token(100), true)
            .unwrap();
        assert_eq!(
            at(1),
            Some(first),
            "a new token keeps the first registration"
        );
        push.register_activity("nA", aid(50), term(6), long_token(150), true)
            .unwrap();
        assert!(at(50).unwrap() > first, "a new activity starts its own");
        push.register_activity("nB", aid(0), term(0), long_token(200), true)
            .unwrap();
        let stored: Devices = peers::load_json(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let mine: Vec<&Activity> = stored
            .activities
            .iter()
            .filter(|a| a.stable_id == "nA")
            .collect();
        assert_eq!(mine.len(), MAX_ACTIVITIES, "same id or terminal replaces");
        assert!(!mine.iter().any(|a| a.activity_id == aid(6)));
        assert!(mine.iter().any(|a| a.activity_id == aid(50)));
        assert!(
            mine.iter()
                .all(|a| a.environment == ApnsEnvironment::Sandbox)
        );
        assert_eq!(
            stored.activities.last().unwrap().environment,
            ApnsEnvironment::Production
        );
        assert!(!mine.iter().any(|a| a.token == long_token(1)));

        assert_eq!(
            push.end_activity("nB", &aid(2)).unwrap(),
            None,
            "another device cannot end it"
        );
        let ended = push.end_activity("nA", &aid(2)).unwrap().unwrap();
        assert_eq!(ended.token, long_token(2));
        assert_eq!(push.end_activity("nA", &aid(2)).unwrap(), None);
        assert_eq!(push.activities().len(), MAX_ACTIVITIES);

        let find = |id: &str, i: usize| {
            push.activities()
                .into_iter()
                .find(|a| a.stable_id == id && a.activity_id == aid(i))
                .unwrap()
        };
        let alert = crate::activity::end(None, 0);
        push.notify_activity(find("nA", 3), alert.clone(), None);
        revoked.store(true, Ordering::SeqCst);
        push.notify_activity(find("nB", 0), alert.clone(), None);
        push.notify_activity(find("nA", 4), alert.clone(), None);
        while mock.calls.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(mock.calls.load(Ordering::SeqCst), 2, "never sent to nB");
        assert_eq!(
            *mock.sent.lock().unwrap(),
            [(long_token(4).as_str().to_owned(), alert)]
        );
        assert!(
            !push.activities().iter().any(|a| a.token == long_token(3)),
            "Unregistered drops the activity token"
        );
        assert_eq!(push.devices().len(), 2);
        assert_eq!(
            push.register_activity("nB", aid(5), term(5), long_token(5), true),
            Err(ActivityError::NotPaired)
        );

        let mut store = Store::default();
        store
            .add(peers::Peer {
                stable_id: "nA".into(),
                user_id: 1,
                login: "me@example.com".into(),
                label: "phone".into(),
                paired_at: 0,
                tls_key: None,
                terminal_key: None,
            })
            .unwrap();
        push.retain_paired(&store).unwrap();
        assert!(push.activities().iter().all(|a| a.stable_id == "nA"));
        assert!(push.expire_activities(0).unwrap().is_empty());
        let left = push.activities();
        assert_eq!(
            push.expire_activities(crate::now_ms() + 1).unwrap(),
            left,
            "registered before the cutoff"
        );
        assert!(push.activities().is_empty());
        assert!(
            peers::load_json::<Devices>(&path)
                .unwrap()
                .activities
                .is_empty()
        );
        push.forget("nA").unwrap();
        let stored: Devices = peers::load_json(&path).unwrap();
        assert_eq!(stored, Devices::default());
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("activities"),
            "an empty list is not written"
        );
    }

    #[tokio::test]
    async fn approval_alerts_skip_devices_that_follow_the_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let mock = Arc::new(Mock::new(token('f')));
        let push = Push::open(
            dir.path().join("push.json"),
            Some(mock.clone()),
            Arc::new(|_| true),
        )
        .unwrap();
        for (id, c) in [("nA", 'a'), ("nB", 'b'), ("nC", 'c')] {
            push.register(id, token(c), ApnsEnvironment::Sandbox, key(), false)
                .unwrap();
        }
        push.register_activity("nA", aid(1), term(1), long_token(1), true)
            .unwrap();
        push.register_activity("nB", aid(2), term(2), long_token(2), true)
            .unwrap();
        push.register_activity("nC", aid(3), term(1), long_token(3), false)
            .unwrap();
        let a = approval(vec![Decision::Approve, Decision::Deny]);
        let alert = approval_alert(&a, "api-fixer", "nMAC", "Bash: rm -rf build");

        push.notify_approval(&a, alert.clone(), true);
        let routed = push.take_routed();
        assert_eq!(routed.len(), 1);
        assert_eq!(routed[0].device, push.devices()[0]);
        assert_eq!(routed[0].terminal_id, a.terminal_id);
        assert_eq!(routed[0].approval_id, a.approval_id);
        assert_eq!(routed[0].alert, alert);
        assert!(routed[0].due);
        assert!(push.take_routed().is_empty());
        let sent = mock.wait(2).await;
        let to: Vec<&str> = sent.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            to,
            [token('b').as_str(), token('c').as_str()],
            "an activity for another terminal, or one that does not show approvals, does not hold the alert back"
        );
        for (_, sent) in &sent {
            assert_eq!(sent.delivery, Delivery::Alert);
            assert_eq!(
                open(
                    &key(),
                    sent.payload["enc"].as_str().unwrap(),
                    a.approval_id.as_str()
                ),
                Some(json!({"v": 1, "body": "Bash: rm -rf build"}))
            );
        }

        push.notify_approval(&a, alert.clone(), false);
        let quiet = push.take_routed();
        assert_eq!(quiet.len(), 1);
        assert!(!quiet[0].due);
        mock.wait(2).await;

        let offline =
            Push::open(dir.path().join("offline.json"), None, Arc::new(|_| true)).unwrap();
        offline
            .register("nA", token('a'), ApnsEnvironment::Sandbox, key(), false)
            .unwrap();
        offline
            .register_activity("nA", aid(1), term(1), long_token(1), true)
            .unwrap();
        offline.notify_approval(&a, alert, true);
        assert!(offline.take_routed().is_empty(), "nothing to deliver with");
    }

    #[tokio::test]
    async fn a_failed_activity_alert_falls_back_to_the_approval_alert() {
        let dir = tempfile::tempdir().unwrap();
        let mock = Arc::new(Mock {
            busy: Some(long_token(2)),
            ..Mock::new(long_token(1))
        });
        let push = Push::open(
            dir.path().join("push.json"),
            Some(mock.clone()),
            Arc::new(|_| true),
        )
        .unwrap();
        push.register("nA", token('a'), ApnsEnvironment::Sandbox, key(), false)
            .unwrap();
        for i in 1..=3 {
            push.register_activity("nA", aid(i), term(i), long_token(i), true)
                .unwrap();
        }
        let find = |i: usize| {
            push.activities()
                .into_iter()
                .find(|x| x.activity_id == aid(i))
                .unwrap()
        };
        let device = push.devices()[0].clone();
        let routed = |id: char| {
            let mut a = approval(vec![Decision::Approve, Decision::Deny]);
            a.approval_id = ApprovalId::new(id.to_string().repeat(22)).unwrap();
            Routed {
                device: device.clone(),
                terminal_id: a.terminal_id.clone(),
                approval_id: a.approval_id.clone(),
                alert: approval_alert(&a, "api-fixer", "nMAC", "Bash: rm -rf build"),
                due: true,
            }
        };
        let a = routed('A');
        let live = crate::activity::end(None, 0);
        let (dead, busy, ok) = (find(1), find(2), find(3));

        push.notify_activity(dead, live.clone(), Some(a.clone()));
        let sent = mock.wait(2).await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, token('a').as_str());
        assert_eq!(sent[0].1.delivery, Delivery::Alert);
        assert_eq!(
            open(
                &key(),
                sent[0].1.payload["enc"].as_str().unwrap(),
                a.approval_id.as_str()
            ),
            Some(json!({"v": 1, "body": "Bash: rm -rf build"})),
            "sealed for the device"
        );
        assert!(
            !push.activities().iter().any(|x| x.activity_id == aid(1)),
            "a dead activity token is dropped"
        );

        push.notify_routed(a);
        mock.wait(2).await;

        push.notify_activity(busy, live.clone(), Some(routed('B')));
        let sent = mock.wait(4).await;
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].0, token('a').as_str(), "any APNs error falls back");
        assert!(push.activities().iter().any(|x| x.activity_id == aid(2)));

        push.notify_activity(ok, live.clone(), Some(routed('C')));
        let sent = mock.wait(5).await;
        assert_eq!(sent.len(), 3);
        assert_eq!(
            sent[2],
            (long_token(3).as_str().to_owned(), live),
            "no fallback"
        );
        push.notify_routed(routed('C'));
        push.notify_routed(routed('C'));
        let sent = mock.wait(6).await;
        assert_eq!(sent[3].0, token('a').as_str(), "a late fallback, once");
        assert_eq!(sent[3].1.payload["approval_id"], "C".repeat(22));
    }

    #[test]
    fn stores_without_activities_still_load() {
        let old = r#"{"devices":[{"stable_id":"nA","token":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","environment":"sandbox","registered_at":1}]}"#;
        let d: Devices = serde_json::from_str(old).unwrap();
        assert!(d.activities.is_empty());
        assert!(!d.devices[0].mute_done);
        assert!(!serde_json::to_string(&d).unwrap().contains("mute_done"));
        let a = Activity {
            stable_id: "nA".into(),
            activity_id: aid(0),
            terminal_id: term(0),
            token: long_token(0),
            environment: ApnsEnvironment::Sandbox,
            registered_at: 0,
            shows_approvals: false,
        };
        assert!(!format!("{a:?}").contains(long_token(0).as_str()));
        assert_eq!(a.target().notification_key, None);
        let stored = serde_json::to_string(&a).unwrap();
        assert!(!stored.contains("shows_approvals"), "{stored}");
        assert_eq!(serde_json::from_str::<Activity>(&stored).unwrap(), a);
    }

    fn cfg(dir: &Path, key_id: &str) -> ApnsConfig {
        ApnsConfig {
            key: ApnsKey::File(dir.join("AuthKey.p8")),
            key_id: key_id.into(),
            team_id: "RM3UT3MMSR".into(),
            bundle_id: "dev.rbstp.collie".into(),
        }
    }

    #[tokio::test]
    async fn key_checks_and_signing_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let c = cfg(dir.path(), "ABCDE12345");
        let key_path = dir.path().join("AuthKey.p8");
        assert!(Apns::new(&c).is_err(), "missing key");
        std::fs::write(&key_path, TEST_KEY).unwrap();
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let open = Apns::new(&c).err().unwrap().to_string();
        assert!(open.contains("0600"), "{open}");
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        Apns::new(&c).unwrap();

        let link_path = dir.path().join("link.p8");
        std::os::unix::fs::symlink(&key_path, &link_path).unwrap();
        let link = ApnsConfig {
            key: ApnsKey::File(link_path.clone()),
            ..cfg(dir.path(), "ABCDE12345")
        };
        assert!(Apns::new(&link).is_err());
        assert!(read_key(&link_path).is_err());

        assert!(Apns::new(&cfg(dir.path(), "abc")).is_err());
        std::fs::write(&key_path, "not a key").unwrap();
        assert!(Apns::new(&c).is_err());
    }

    // Throwaway P-384 key: valid PKCS#8 EC, but not ES256.
    const P384_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIG2AgEAMBAGByqGSM49AgEGBSuBBAAiBIGeMIGbAgEBBDB2RWjt6bEXQbKDKF/q
cHU8Ul36wIKK68CJhNr/S14zBjOLhQnb7Qoa1fS6WDi7c2mhZANiAASZnXAvFQVb
AfHxp1leBwH54XlAR82ZGMu6rWJnHsx53+w8ivHKMALwn0xoPEnfh7bSvrVmcJXz
BDSKTTpvY6ZTNJ2aRaGwPfVBlnky7dc62au34JD4PDc7wpwLOJUSinI=
-----END PRIVATE KEY-----
";

    #[test]
    fn only_es256_keys_sign() {
        let c = cfg(Path::new("/nonexistent"), "ABCDE12345");
        Apns::with_key(&c, TEST_KEY.as_bytes()).unwrap();
        assert!(Apns::with_key(&c, P384_KEY.as_bytes()).is_err());
        assert!(Apns::with_key(&c, b"").is_err());
    }

    #[test]
    fn import_refuses_links_and_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("collied.toml");
        let key_path = dir.path().join("AuthKey_ABCDE12345.p8");
        std::fs::write(
            &config_path,
            format!(
                "[apns]\nkey_path = \"{}\"\nkey_id = \"ABCDE12345\"\nteam_id = \"RM3UT3MMSR\"\nbundle_id = \"dev.rbstp.collie\"\n",
                key_path.display()
            ),
        )
        .unwrap();
        std::fs::write(&key_path, TEST_KEY).unwrap();
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let link = dir.path().join("link.p8");
        std::os::unix::fs::symlink(&key_path, &link).unwrap();
        let err = import(&config_path, true, &link).unwrap_err();
        assert!(format!("{err:#}").contains("open"), "{err:#}");

        // Owned by root: refused before anything reaches the Keychain.
        let err = import(&config_path, true, Path::new("/etc/hosts")).unwrap_err();
        assert!(
            format!("{err:#}").contains("owned by the current user"),
            "{err:#}"
        );

        let err = import(
            &config_path,
            true,
            &dir.path().join("AuthKey_ZZZZZ99999.p8"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("key_id is ABCDE12345"), "{err}");

        let not_es256 = dir.path().join("p384.p8");
        std::fs::write(&not_es256, P384_KEY).unwrap();
        std::fs::set_permissions(&not_es256, std::fs::Permissions::from_mode(0o600)).unwrap();
        let err = import(&config_path, true, &not_es256).unwrap_err();
        assert!(err.to_string().contains("not an APNs ES256 key"), "{err}");

        assert!(key_path.exists());
        assert!(
            std::fs::read_to_string(&config_path)
                .unwrap()
                .contains("key_path")
        );
    }
}
