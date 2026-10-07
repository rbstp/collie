use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use collie_tls::rustls::CertificateError;
use collie_tls::rustls::sign::CertifiedKey;
use protocol::{
    ActivityId, Empty, HelloResult, KeyPin, PushActivityEndParams, PushActivityTokenParams,
    PushRegisterParams, Request, Response,
};
use tailnet::{BackendState, Node};
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::pin::{self, PinError};
use crate::reach::Reachability;
use crate::session::{FlockState, Reply, Session, SessionError, lock};
use crate::store::Machine;
use crate::store::MachineKind;

const BACKOFF: [Duration; 6] = [
    Duration::from_secs(3),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(30),
    Duration::from_secs(60),
];
const RESET_AFTER: Duration = Duration::from_secs(30);
const OFFLINE_POLL: Duration = Duration::from_secs(3);
const PEER_OFFLINE_POLL: [Duration; 4] = [
    Duration::from_secs(3),
    Duration::from_secs(6),
    Duration::from_secs(12),
    Duration::from_secs(15),
];
/// A machine Tailscale reports offline is not dialed: every 20 s dial to a dead peer runs
/// on the node all machines share. It is still tried this often, in case control is wrong,
/// and once on every reconnect request (a resume or a retry), when its netmap may be stale.
const OFFLINE_REDIAL: Duration = Duration::from_secs(15 * 60);
const DIAL_ATTEMPT: Duration = Duration::from_secs(5);
const DIAL_BUDGET: Duration = Duration::from_secs(20);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
pub(crate) const FOREGROUND_RECONNECT: Duration = Duration::from_secs(10);
const RESUME_GRACE: Duration = Duration::from_secs(10);
const GRACE_BACKOFF: [Duration; 3] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];
const QUEUE: usize = 8;
const MAX_UNSENT_ENDS: usize = 16;
const MUTATION_ATTEMPTS: usize = 3;

/// Every holder clones the outer Arc, never the inner Node, so the strong count says
/// whether a replaced node is still open on the shared tsnet state dir.
pub type NodeSlot = Arc<Mutex<Option<Arc<Node>>>>;

pub type IdentitySlot = Arc<Mutex<Option<Arc<CertifiedKey>>>>;

pub type Stream = collie_tls::client::TlsStream<collie_tls::Sniff<UnixStream>>;

/// Keyed by machine id: each Mac gets its own notification key.
pub type PushSlot = Arc<Mutex<BTreeMap<String, Registrations>>>;

/// Memory only, never written to disk. Every session of the machine sends them again.
#[derive(Debug, Clone, Default)]
pub struct Registrations {
    pub push: Option<PushRegisterParams>,
    pub activities: BTreeMap<String, PushActivityTokenParams>,
    /// Ends the Mac may not have received yet: sent once with the next session.
    pub unsent_ends: Vec<ActivityId>,
}

impl Registrations {
    pub fn end(&mut self, activity_id: &ActivityId) {
        self.activities.remove(activity_id.as_str());
        self.unsent_ends.retain(|a| a != activity_id);
        if self.unsent_ends.len() == MAX_UNSENT_ENDS {
            self.unsent_ends.remove(0);
        }
        self.unsent_ends.push(activity_id.clone());
    }

    /// `push.register` first: collied takes an activity's APNs environment from it.
    pub fn take_requests(&mut self) -> Vec<Request> {
        let ends = std::mem::take(&mut self.unsent_ends)
            .into_iter()
            .map(|activity_id| Request::PushActivityEnd(PushActivityEndParams { activity_id }));
        self.push
            .clone()
            .map(Request::PushRegister)
            .into_iter()
            .chain(ends)
            .chain(
                self.activities
                    .values()
                    .cloned()
                    .map(Request::PushActivityToken),
            )
            .collect()
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ConnectError {
    #[error("tailnet is not running")]
    Offline,
    #[error(transparent)]
    Pin(#[from] PinError),
    #[error("not reachable ({0})")]
    Dial(String),
    #[error("not reachable ({0})")]
    Status(String),
    #[error("not reachable, it may be off or asleep")]
    PeerOffline,
    #[error("this phone's key is not loaded yet")]
    NoIdentity,
    #[error("paired before mutual TLS: run collied pair on the machine and scan its code again")]
    PairAgain,

    #[error(transparent)]
    Session(#[from] SessionError),
}

impl ConnectError {
    pub fn is_auth(&self) -> bool {
        match self {
            Self::Pin(e) => e.is_violation(),
            Self::Session(e) => e.is_auth(),
            Self::PairAgain => true,
            _ => false,
        }
    }

    /// Returned before the machine was dialed: the attempt is not a dial and does not use
    /// up a forced one.
    fn before_dial(&self) -> bool {
        match self {
            Self::Offline
            | Self::PeerOffline
            | Self::NoIdentity
            | Self::PairAgain
            | Self::Status(_) => true,
            Self::Pin(e) => !e.is_violation(),
            Self::Dial(_) | Self::Session(_) => false,
        }
    }
}

pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(std::io::Error::other)
}

/// Re-checks the pin against the current netmap on every connection, then dials the
/// pinned node's tailnet IP. The node is released once dialed so a session never
/// keeps a replaced node open.
#[allow(clippy::too_many_arguments)]
pub async fn open(
    node: Arc<Node>,
    host: &str,
    port: u16,
    node_id: &str,
    kind: Option<MachineKind>,
    key: &str,
    identity: Option<Arc<CertifiedKey>>,
    dial_offline: bool,
) -> Result<(Session<Stream>, HelloResult, MachineKind), ConnectError> {
    let key = KeyPin::new(key).map_err(|_| ConnectError::PairAgain)?;
    let identity = identity.ok_or(ConnectError::NoIdentity)?;
    let n = node.clone();
    let status = blocking(move || n.status())
        .await
        .map_err(|e| ConnectError::Status(e.to_string()))?
        .map_err(|e| ConnectError::Status(e.to_string()))?;
    if status.backend_state != BackendState::Running {
        return Err(ConnectError::Offline);
    }
    let (ip, kind) = pin::resolve(&status, host, node_id, kind)?;
    let online = status
        .peer
        .iter()
        .flat_map(|peers| peers.values())
        .any(|p| p.stable_id == node_id && p.online);
    if !online && !dial_offline {
        return Err(ConnectError::PeerOffline);
    }
    let n = node.clone();
    let who = blocking(move || n.whois(&ip.to_string()))
        .await
        .map_err(|e| ConnectError::Dial(e.to_string()))?
        .map_err(|e| ConnectError::Dial(e.to_string()))?;
    pin::verify_whois(&who.node, host, node_id)?;
    let stream = dial(&node, SocketAddr::new(ip, port)).await?;
    drop(node);
    let stream = tls(stream, host, key, identity).await?;
    let mut session = Session::connect(stream, host, port).await?;
    let hello = session.hello().await?;
    pin::verify_node(&hello.machine.node_id, host, node_id)?;
    Ok((session, hello, kind))
}

pub(crate) async fn tls(
    stream: UnixStream,
    host: &str,
    key: KeyPin,
    identity: Arc<CertifiedKey>,
) -> Result<Stream, ConnectError> {
    let handshake = collie_tls::connect(stream, host, key, identity);
    match tokio::time::timeout(crate::session::CALL_TIMEOUT, handshake).await {
        Err(_) => Err(SessionError::Timeout.into()),
        Ok(Ok(s)) => Ok(s),
        Ok(Err(collie_tls::ConnectError::Forbidden)) => Err(SessionError::Refused(403).into()),
        Ok(Err(collie_tls::ConnectError::Tls(e))) => {
            use collie_tls::rustls::Error as Tls;
            Err(match e.get_ref().and_then(|e| e.downcast_ref::<Tls>()) {
                Some(Tls::InvalidCertificate(
                    CertificateError::ApplicationVerificationFailure
                    | CertificateError::BadEncoding,
                )) => PinError::KeyMismatch {
                    host: host.to_owned(),
                }
                .into(),
                _ => ConnectError::Dial(format!("tls: {e}")),
            })
        }
    }
}

pub(crate) async fn dial(node: &Arc<Node>, addr: SocketAddr) -> Result<UnixStream, ConnectError> {
    let deadline = Instant::now() + DIAL_BUDGET;
    let mut delay = Duration::from_millis(250);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let n = node.clone();
        let res = blocking(move || {
            let stream = n.dial_timeout("tcp", &addr.to_string(), DIAL_ATTEMPT.min(remaining))?;
            stream.set_nonblocking(true)?;
            Ok::<_, tailnet::Error>(stream)
        })
        .await
        .map_err(|e| ConnectError::Dial(e.to_string()))?;
        let err = match res {
            Ok(stream) => {
                return UnixStream::from_std(stream).map_err(|e| ConnectError::Dial(e.to_string()));
            }
            Err(e) => e,
        };
        if Instant::now() + delay >= deadline {
            return Err(ConnectError::Dial(err.to_string()));
        }
        tokio::time::sleep(delay).await;
        delay *= 2;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LinkPhase {
    Connecting,
    Connected,
    Waiting,
    /// Tailscale reports the machine offline; it is not dialed until it is back.
    Unavailable,
    Offline,
    Stopped,
}

pub struct Link {
    pub phase: LinkPhase,
    pub last_error: Option<String>,
    resumed: Option<Instant>,
    pub last_dial: Instant,
}

impl Link {
    fn set(&mut self, phase: LinkPhase, error: Option<String>) {
        self.phase = phase;
        if error.is_some() || phase == LinkPhase::Connected {
            self.last_error = error;
        }
    }
}

pub struct Shared {
    pub flock: Mutex<FlockState>,
    pub link: Mutex<Link>,
}

impl Shared {
    fn set(&self, phase: LinkPhase, error: Option<String>) {
        lock(&self.link).set(phase, error);
    }

    /// The session may have failed before the app saw the resume: that failure gets the
    /// grace too, so a waiting or unavailable link goes back to connecting and must be
    /// retried at once.
    fn resume(&self, now: Instant) -> bool {
        let mut link = lock(&self.link);
        link.resumed = Some(now);
        if !matches!(link.phase, LinkPhase::Waiting | LinkPhase::Unavailable) {
            return false;
        }
        link.phase = LinkPhase::Connecting;
        link.last_error = None;
        true
    }
}

pub struct Conn {
    pub machine: Machine,
    pub shared: Arc<Shared>,
    requests: mpsc::Sender<(Request, Reply)>,
    reconnect: watch::Sender<u64>,
    wake: Arc<Notify>,
    runtime: tokio::runtime::Handle,
    task: JoinHandle<()>,
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug)]
pub enum RequestError {
    Stopped,
    Busy,
    Timeout,
    Failed(SessionError),
}

impl Conn {
    pub fn spawn(
        runtime: &tokio::runtime::Handle,
        machine: Machine,
        node: NodeSlot,
        identity: IdentitySlot,
        push: PushSlot,
        reach: Arc<Reachability>,
        suspended: watch::Receiver<bool>,
    ) -> Self {
        let shared = Arc::new(Shared {
            flock: Mutex::default(),
            link: Mutex::new(Link {
                phase: LinkPhase::Connecting,
                last_error: None,
                resumed: None,
                last_dial: Instant::now(),
            }),
        });
        let (requests, rx) = mpsc::channel(QUEUE);
        let (reconnect, reconnect_rx) = watch::channel(0);
        let wake = Arc::new(Notify::new());
        let task = runtime.spawn(supervise(
            machine.clone(),
            node,
            identity,
            push,
            reach,
            shared.clone(),
            rx,
            reconnect_rx,
            wake.clone(),
            suspended,
        ));
        Self {
            machine,
            shared,
            requests,
            reconnect,
            wake,
            runtime: runtime.clone(),
            task,
        }
    }

    pub async fn request(
        &self,
        request: Request,
        timeout: Duration,
    ) -> Result<Response, RequestError> {
        send(&self.requests, request, timeout).await
    }

    /// Queued before this returns, so calls keep their order on the connection.
    pub fn request_in_order(
        &self,
        request: Request,
        timeout: Duration,
    ) -> impl Future<Output = Result<Response, RequestError>> + Send + 'static {
        let queued = enqueue(&self.requests, request);
        async move { answer(queued?, timeout).await }
    }

    pub async fn mutate(
        &self,
        request: Request,
        timeout: Duration,
    ) -> Result<Response, RequestError> {
        send_mutation(&self.requests, request, timeout).await
    }

    pub fn reconnect_now(&self) {
        self.reconnect.send_modify(|n| *n += 1);
        self.wake.notify_one();
    }

    /// Ends a wait for the node to run without forcing a dial; the attempt goes through
    /// [`open`] as any other. Only a waiter is woken: a stored permit would cut a later
    /// backoff short.
    pub fn node_running(&self) {
        if lock(&self.shared.link).phase == LinkPhase::Offline {
            self.wake.notify_waiters();
        }
    }

    /// iOS suspends sockets without closing them: after a long background period the
    /// connection is assumed dead, after a short one it is probed. A suspended core has
    /// no session left to probe.
    pub fn resume(self: &Arc<Self>, background: Duration, suspended: bool) {
        let waiting = self.shared.resume(Instant::now());
        if waiting || suspended || background >= FOREGROUND_RECONNECT {
            self.reconnect_now();
            return;
        }
        let conn = self.clone();
        self.runtime.spawn(async move {
            if conn
                .request(Request::FlockSnapshot(Empty {}), PROBE_TIMEOUT)
                .await
                .is_err()
            {
                conn.reconnect_now();
            }
        });
    }
}

async fn send(
    requests: &mpsc::Sender<(Request, Reply)>,
    request: Request,
    timeout: Duration,
) -> Result<Response, RequestError> {
    answer(enqueue(requests, request)?, timeout).await
}

type Answer = oneshot::Receiver<Result<Response, SessionError>>;

fn enqueue(
    requests: &mpsc::Sender<(Request, Reply)>,
    request: Request,
) -> Result<Answer, RequestError> {
    let (tx, rx) = oneshot::channel();
    requests.try_send((request, tx)).map_err(|e| match e {
        mpsc::error::TrySendError::Closed(_) => RequestError::Stopped,
        mpsc::error::TrySendError::Full(_) => RequestError::Busy,
    })?;
    Ok(rx)
}

async fn answer(rx: Answer, timeout: Duration) -> Result<Response, RequestError> {
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(Ok(response))) => Ok(response),
        Ok(Ok(Err(e))) => Err(RequestError::Failed(e)),
        Ok(Err(_)) => Err(RequestError::Stopped),
        Err(_) => Err(RequestError::Timeout),
    }
}

/// The request carries an `op_id`, so resending it after the connection dropped gets
/// collied's stored outcome instead of running the mutation twice. The resend waits in
/// the queue for the next session. Any answer from collied is final; a local timeout is
/// not retried because the first send may still be in flight.
async fn send_mutation(
    requests: &mpsc::Sender<(Request, Reply)>,
    request: Request,
    timeout: Duration,
) -> Result<Response, RequestError> {
    let deadline = Instant::now() + timeout;
    let mut attempt = 1;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match send(requests, request.clone(), remaining).await {
            Err(RequestError::Failed(e)) if e.is_transport() && attempt < MUTATION_ATTEMPTS => {
                attempt += 1;
            }
            other => return other,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn supervise(
    machine: Machine,
    node: NodeSlot,
    identity: IdentitySlot,
    push: PushSlot,
    reach: Arc<Reachability>,
    shared: Arc<Shared>,
    mut requests: mpsc::Receiver<(Request, Reply)>,
    mut reconnect: watch::Receiver<u64>,
    wake: Arc<Notify>,
    mut suspended: watch::Receiver<bool>,
) {
    let mut backoff = Backoff::default();
    let mut peer_offline = false;
    let mut force = false;
    loop {
        if suspended.wait_for(|s| !s).await.is_err() {
            return;
        }
        let running = lock(&node).clone();
        let Some(node) = running else {
            shared.set(LinkPhase::Offline, None);
            wait(&wake, OFFLINE_POLL).await;
            continue;
        };
        force |= reconnect.borrow_and_update().has_changed();
        if force || !peer_offline {
            shared.set(LinkPhase::Connecting, None);
        }
        let dial_offline = force || lock(&shared.link).last_dial.elapsed() >= OFFLINE_REDIAL;
        let key = lock(&identity).clone();
        let opened = tokio::select! {
            opened = open(
                node,
                &machine.host,
                machine.port,
                &machine.node_id,
                Some(machine.kind),
                &machine.key,
                key,
                dial_offline,
            ) => opened,
            _ = suspended.wait_for(|s| *s) => continue,
        };
        peer_offline = matches!(opened, Err(ConnectError::PeerOffline));
        let dialed = !opened.as_ref().is_err_and(ConnectError::before_dial);
        force &= !dialed;
        if dialed {
            lock(&shared.link).last_dial = Instant::now();
        }
        if !matches!(
            opened,
            Err(ConnectError::Offline | ConnectError::PeerOffline)
        ) {
            reach.record(
                &machine.node_id,
                matches!(&opened, Ok((_, hello, _)) if hello.paired),
            );
        }
        let err = match opened {
            Ok((_, hello, _)) if !hello.paired => ConnectError::Session(SessionError::NotPaired),
            Ok((session, _, _)) => {
                lock(&shared.flock).new_connection();
                shared.set(LinkPhase::Connected, None);
                let since = Instant::now();
                reconnect.borrow_and_update();
                // A clone, so the request that ends the session still forces the next dial.
                let mut ended = reconnect.clone();
                let push = lock(&push)
                    .get_mut(&machine.id)
                    .map(Registrations::take_requests)
                    .unwrap_or_default();
                let end = session
                    .run(&mut requests, &shared.flock, push, async {
                        tokio::select! {
                            _ = ended.changed() => false,
                            _ = suspended.wait_for(|s| *s) => true,
                        }
                    })
                    .await;
                if since.elapsed() >= RESET_AFTER {
                    backoff.attempt = 0;
                }
                if *suspended.borrow() {
                    shared.set(LinkPhase::Connecting, None);
                    continue;
                }
                ConnectError::Session(end)
            }
            Err(e) => e,
        };
        let Some(delay) = backoff.failed(&shared, &err, Instant::now()) else {
            return;
        };
        wait(&wake, delay).await;
    }
}

/// After a foreground resume the node needs a moment to rebuild its paths, and the old
/// session ends (or is ended) as `Closed`. Failures within `RESUME_GRACE` retry quickly
/// and stay `Connecting` without an error; auth failures still stop at once. A resume
/// also restarts the normal backoff, and until [`crate::RESTART_AFTER`] a machine
/// Tailscale reports offline is polled at the quickest rate, while the node recovers.
#[derive(Default)]
struct Backoff {
    attempt: usize,
    resumed: Option<Instant>,
    grace_attempt: usize,
    offline_attempt: usize,
}

impl Backoff {
    /// Publishes the failure on `shared` and returns the delay before the next attempt,
    /// or `None` when the supervisor must stop.
    fn failed(&mut self, shared: &Shared, err: &ConnectError, now: Instant) -> Option<Duration> {
        let mut link = lock(&shared.link);
        if err.is_auth() {
            link.set(LinkPhase::Stopped, Some(err.to_string()));
            return None;
        }
        if link.resumed != self.resumed {
            self.resumed = link.resumed;
            self.attempt = 0;
            self.grace_attempt = 0;
        }
        if matches!(err, ConnectError::PeerOffline) {
            link.set(LinkPhase::Unavailable, Some(err.to_string()));
            if link
                .resumed
                .is_some_and(|at| now.saturating_duration_since(at) < crate::RESTART_AFTER)
            {
                self.offline_attempt = 0;
            }
            let delay = PEER_OFFLINE_POLL[self.offline_attempt.min(PEER_OFFLINE_POLL.len() - 1)];
            self.offline_attempt += 1;
            return Some(delay);
        }
        self.offline_attempt = 0;
        if link
            .resumed
            .is_some_and(|at| now.saturating_duration_since(at) < RESUME_GRACE)
        {
            link.set(LinkPhase::Connecting, None);
            let delay = GRACE_BACKOFF[self.grace_attempt.min(GRACE_BACKOFF.len() - 1)];
            self.grace_attempt += 1;
            return Some(delay);
        }
        if matches!(err, ConnectError::Offline) {
            link.set(LinkPhase::Offline, None);
            return Some(OFFLINE_POLL);
        }
        link.set(LinkPhase::Waiting, Some(err.to_string()));
        let delay = BACKOFF[self.attempt.min(BACKOFF.len() - 1)];
        self.attempt += 1;
        Some(delay)
    }
}

async fn wait(wake: &Notify, delay: Duration) {
    tokio::select! {
        () = tokio::time::sleep(delay) => {}
        () = wake.notified() => {}
    }
}

#[cfg(test)]
mod tests {
    use protocol::{AgentPromptParams, ErrorCode, OpId, PromptText, TerminalId};

    use super::*;

    fn key() -> (Arc<CertifiedKey>, KeyPin) {
        let key =
            collie_tls::certified(collie_tls::load(&collie_tls::generate().unwrap()).unwrap())
                .unwrap();
        let pin = collie_tls::pin(key.cert[0].as_ref());
        (key, pin)
    }

    #[tokio::test]
    async fn tls_failures_say_which_side_refused() {
        let (machine, machine_pin) = key();
        let (phone, phone_pin) = key();
        let (other, other_pin) = key();
        let run = |server_key: Arc<CertifiedKey>, expect: KeyPin, pin: KeyPin| {
            let phone = phone.clone();
            async move {
                let (a, b) = UnixStream::pair().unwrap();
                tokio::spawn(collie_tls::accept(b, server_key, Some(expect)));
                tls(a, "mac.ts.net", pin, phone).await
            }
        };
        assert!(
            run(machine.clone(), phone_pin.clone(), machine_pin.clone())
                .await
                .is_ok()
        );
        let stream = run(machine.clone(), other_pin, machine_pin.clone())
            .await
            .unwrap();
        let err = Session::connect(stream, "mac.ts.net", 8457)
            .await
            .err()
            .unwrap();
        assert!(
            matches!(err, SessionError::KeyRefused) && err.is_auth(),
            "{err:?}"
        );
        let err = run(other, phone_pin, machine_pin.clone())
            .await
            .unwrap_err();
        assert!(
            matches!(err, ConnectError::Pin(PinError::KeyMismatch { .. })) && err.is_auth(),
            "{err:?}"
        );

        let (a, b) = UnixStream::pair().unwrap();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut b = b;
            b.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n").await
        });
        let err = tls(a, "mac.ts.net", machine_pin, phone).await.unwrap_err();
        assert!(
            matches!(err, ConnectError::Session(SessionError::Refused(403))) && err.is_auth(),
            "{err:?}"
        );
    }

    fn prompt() -> Request {
        Request::AgentPrompt(AgentPromptParams {
            op_id: OpId::new("Zm9vYmFyYmF6cXV4cXV1dQ").unwrap(),
            terminal_id: TerminalId::new("term_1").unwrap(),
            text: PromptText::new("run the tests").unwrap(),
            expected_draft: None,
        })
    }

    /// Answers each queued request with the next scripted reply, recording what it got.
    fn fake(
        replies: Vec<Result<Response, SessionError>>,
    ) -> (mpsc::Sender<(Request, Reply)>, JoinHandle<Vec<Request>>) {
        let (tx, mut rx) = mpsc::channel::<(Request, Reply)>(QUEUE);
        let task = tokio::spawn(async move {
            let mut seen = Vec::new();
            for reply in replies {
                let Some((request, tx)) = rx.recv().await else {
                    break;
                };
                seen.push(request);
                let _ = tx.send(reply);
            }
            seen
        });
        (tx, task)
    }

    #[tokio::test]
    async fn dropped_mutation_is_resent_with_the_same_op_id() {
        let (tx, task) = fake(vec![
            Err(SessionError::Closed),
            Err(SessionError::WebSocket("reset".into())),
            Ok(Response::Ok),
        ]);
        let res = send_mutation(&tx, prompt(), Duration::from_secs(5)).await;
        assert_eq!(res.unwrap(), Response::Ok);
        let seen = task.await.unwrap();
        assert_eq!(seen, vec![prompt(), prompt(), prompt()]);
    }

    #[tokio::test]
    async fn answers_from_collied_are_never_retried() {
        let (tx, task) = fake(vec![
            Err(SessionError::Server {
                code: ErrorCode::AgentBlocked,
                message: "blocked".into(),
            }),
            Ok(Response::Ok),
        ]);
        let res = send_mutation(&tx, prompt(), Duration::from_secs(5)).await;
        assert!(matches!(
            res,
            Err(RequestError::Failed(SessionError::Server {
                code: ErrorCode::AgentBlocked,
                ..
            }))
        ));
        drop(tx);
        assert_eq!(task.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn retries_are_bounded() {
        let (tx, task) = fake(vec![
            Err(SessionError::Closed),
            Err(SessionError::Closed),
            Err(SessionError::Closed),
            Ok(Response::Ok),
        ]);
        let res = send_mutation(&tx, prompt(), Duration::from_secs(5)).await;
        assert!(matches!(
            res,
            Err(RequestError::Failed(SessionError::Closed))
        ));
        drop(tx);
        assert_eq!(task.await.unwrap().len(), MUTATION_ATTEMPTS);
    }

    fn shared() -> Shared {
        Shared {
            flock: Mutex::default(),
            link: Mutex::new(Link {
                phase: LinkPhase::Connected,
                last_error: None,
                resumed: None,
                last_dial: Instant::now(),
            }),
        }
    }

    fn link(shared: &Shared) -> (LinkPhase, Option<String>) {
        let link = lock(&shared.link);
        (link.phase, link.last_error.clone())
    }

    fn closed() -> ConnectError {
        ConnectError::Session(SessionError::Closed)
    }

    #[test]
    fn resume_hides_a_transient_failure() {
        let shared = shared();
        let mut backoff = Backoff::default();
        let t0 = Instant::now();
        shared.resume(t0);
        assert_eq!(
            backoff.failed(&shared, &closed(), t0 + Duration::from_millis(100)),
            Some(GRACE_BACKOFF[0])
        );
        assert_eq!(link(&shared), (LinkPhase::Connecting, None));
        shared.set(LinkPhase::Connected, None);
        assert_eq!(link(&shared), (LinkPhase::Connected, None));
        assert_eq!(backoff.attempt, 0, "the normal backoff is untouched");
    }

    #[test]
    fn grace_retries_quickly_then_settles() {
        let shared = shared();
        let mut backoff = Backoff::default();
        let t0 = Instant::now();
        shared.resume(t0);
        let delays: Vec<_> = (0..5)
            .map(|i| backoff.failed(&shared, &closed(), t0 + Duration::from_secs(i)))
            .collect();
        let ms = |ms| Some(Duration::from_millis(ms));
        assert_eq!(
            delays,
            vec![ms(500), ms(1000), ms(2000), ms(2000), ms(2000)]
        );
        assert_eq!(link(&shared), (LinkPhase::Connecting, None));

        shared.resume(t0 + Duration::from_secs(5));
        assert_eq!(
            backoff.failed(&shared, &closed(), t0 + Duration::from_secs(6)),
            ms(500),
            "a new resume restarts the grace backoff"
        );
    }

    #[test]
    fn failures_after_the_grace_window_wait_with_the_error() {
        let shared = shared();
        let mut backoff = Backoff::default();
        let t0 = Instant::now();
        shared.resume(t0);
        backoff.failed(&shared, &closed(), t0);
        let late = t0 + RESUME_GRACE;
        assert_eq!(backoff.failed(&shared, &closed(), late), Some(BACKOFF[0]));
        assert_eq!(
            link(&shared),
            (LinkPhase::Waiting, Some("connection closed".into()))
        );
        assert_eq!(backoff.failed(&shared, &closed(), late), Some(BACKOFF[1]));
    }

    #[test]
    fn without_a_resume_failures_wait_as_before() {
        let shared = shared();
        let mut backoff = Backoff::default();
        let now = Instant::now();
        assert_eq!(backoff.failed(&shared, &closed(), now), Some(BACKOFF[0]));
        assert_eq!(
            link(&shared),
            (LinkPhase::Waiting, Some("connection closed".into()))
        );
        assert_eq!(
            backoff.failed(&shared, &ConnectError::Offline, now),
            Some(OFFLINE_POLL)
        );
        assert_eq!(link(&shared).0, LinkPhase::Offline);
    }

    #[test]
    fn failure_just_before_resume_gets_the_grace() {
        let shared = shared();
        let mut backoff = Backoff::default();
        let t0 = Instant::now();
        assert_eq!(backoff.failed(&shared, &closed(), t0), Some(BACKOFF[0]));
        assert_eq!(
            link(&shared),
            (LinkPhase::Waiting, Some("connection closed".into()))
        );
        assert!(shared.resume(t0), "a waiting link is retried at once");
        assert_eq!(link(&shared), (LinkPhase::Connecting, None));
        assert_eq!(
            backoff.failed(&shared, &closed(), t0),
            Some(GRACE_BACKOFF[0])
        );
        assert_eq!(link(&shared), (LinkPhase::Connecting, None));
        assert_eq!(
            backoff.failed(&shared, &closed(), t0 + RESUME_GRACE),
            Some(BACKOFF[0]),
            "the resume restarted the normal backoff"
        );
    }

    #[test]
    fn offline_node_during_grace_keeps_connecting() {
        let shared = shared();
        let mut backoff = Backoff::default();
        let t0 = Instant::now();
        shared.resume(t0);
        assert_eq!(
            backoff.failed(&shared, &ConnectError::Offline, t0),
            Some(GRACE_BACKOFF[0])
        );
        assert_eq!(link(&shared), (LinkPhase::Connecting, None));
    }

    #[test]
    fn a_machine_tailscale_reports_offline_waits_quietly() {
        let shared = shared();
        let mut backoff = Backoff::default();
        let t0 = Instant::now();
        assert!(!shared.resume(t0));
        let polls: Vec<_> = (0..5)
            .map(|i| {
                backoff
                    .failed(
                        &shared,
                        &ConnectError::PeerOffline,
                        t0 + crate::RESTART_AFTER + Duration::from_secs(i),
                    )
                    .unwrap()
                    .as_secs()
            })
            .collect();
        assert_eq!(
            polls,
            [3, 6, 12, 15, 15],
            "no grace retries, and a slower poll while it stays offline"
        );
        assert_eq!(
            link(&shared),
            (
                LinkPhase::Unavailable,
                Some("not reachable, it may be off or asleep".into())
            )
        );
        assert_eq!(backoff.attempt, 0);

        let t1 = t0 + Duration::from_secs(60);
        assert!(
            shared.resume(t1),
            "a resume dials it at once, past the netmap"
        );
        assert_eq!(link(&shared), (LinkPhase::Connecting, None));
        assert_eq!(
            backoff.failed(&shared, &closed(), t1),
            Some(GRACE_BACKOFF[0]),
            "that dial's failure gets the grace"
        );
        for _ in 0..3 {
            assert_eq!(
                backoff.failed(&shared, &ConnectError::PeerOffline, t1 + RESUME_GRACE),
                Some(PEER_OFFLINE_POLL[0]),
                "then it waits quietly again, polled quickly while the node recovers"
            );
        }
        assert_eq!(link(&shared).0, LinkPhase::Unavailable);
        let t2 = t1 + crate::RESTART_AFTER;
        assert_eq!(
            backoff.failed(&shared, &ConnectError::PeerOffline, t2),
            Some(PEER_OFFLINE_POLL[1]),
            "and more slowly after that"
        );
        assert_eq!(backoff.failed(&shared, &closed(), t2), Some(BACKOFF[0]));
        assert_eq!(
            backoff.failed(&shared, &ConnectError::PeerOffline, t2),
            Some(PEER_OFFLINE_POLL[0]),
            "a session or a dial in between restarts the offline poll"
        );
    }

    #[test]
    fn auth_failure_during_grace_stops_at_once() {
        let shared = shared();
        let mut backoff = Backoff::default();
        let t0 = Instant::now();
        shared.resume(t0);
        let err = ConnectError::Session(SessionError::NotPaired);
        assert_eq!(backoff.failed(&shared, &err, t0), None);
        assert_eq!(link(&shared), (LinkPhase::Stopped, Some(err.to_string())));
    }

    #[tokio::test(start_paused = true)]
    async fn node_running_wakes_only_a_wait_for_the_node() {
        let (_suspend, suspended) = watch::channel(true);
        let machine = Machine {
            id: "m".into(),
            label: "m".into(),
            host: "m".into(),
            port: 0,
            node_id: "n".into(),
            kind: MachineKind::Mac,
            key: String::new(),
            terminal_key: String::new(),
        };
        let conn = Conn::spawn(
            &tokio::runtime::Handle::current(),
            machine,
            NodeSlot::default(),
            IdentitySlot::default(),
            PushSlot::default(),
            Arc::default(),
            suspended,
        );
        let timed = |wake: Arc<Notify>| {
            tokio::spawn(async move {
                let t = tokio::time::Instant::now();
                wait(&wake, OFFLINE_POLL).await;
                t.elapsed()
            })
        };
        for phase in [
            LinkPhase::Connecting,
            LinkPhase::Connected,
            LinkPhase::Waiting,
            LinkPhase::Unavailable,
            LinkPhase::Offline,
            LinkPhase::Stopped,
        ] {
            lock(&conn.shared.link).phase = phase;
            let waiting = timed(conn.wake.clone());
            tokio::task::yield_now().await;
            conn.node_running();
            assert_eq!(
                waiting.await.unwrap() < OFFLINE_POLL,
                phase == LinkPhase::Offline,
                "{phase:?}"
            );
            conn.node_running();
            assert!(
                timed(conn.wake.clone()).await.unwrap() >= OFFLINE_POLL,
                "a later wait is not cut short ({phase:?})"
            );
        }
    }

    #[tokio::test]
    async fn local_timeout_is_not_retried() {
        let (tx, mut rx) = mpsc::channel::<(Request, Reply)>(QUEUE);
        let res = send_mutation(&tx, prompt(), Duration::from_millis(50)).await;
        assert!(matches!(res, Err(RequestError::Timeout)));
        let (_, reply) = rx.recv().await.unwrap();
        assert!(
            reply.is_closed(),
            "the session skips a request nobody waits for"
        );
        assert!(rx.try_recv().is_err());
    }
}
