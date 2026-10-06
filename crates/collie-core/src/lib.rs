mod approvals;
mod attachments;
mod conn;
mod identity;
mod pin;
mod reach;
mod session;
mod store;

use std::collections::HashMap;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use protocol::{
    ActivityId, AgentKind, AgentPromptParams, AgentSendKeysParams, AgentTarget,
    AgentTypeTextParams, AgentWatchParams, ApprovalId, Cwd, DraftText, Empty, ErrorCode, Key,
    Label, NotificationKey, OpId, PairCompleteParams, PairingInvite, PaneCloseParams, PromptText,
    PushActivityEndParams, PushActivityTokenParams, PushRegisterParams, PushToken, ReadParams,
    ReadSource, Request, Response, TaskNewParams, TerminalId, TerminalRead, WorkspaceCloseParams,
    WorkspaceId, limits,
};
use tailnet::{BackendState, Config, Node};
use tokio::sync::watch;
use zeroize::Zeroizing;

pub use approvals::{
    ApprovalChoice, ApprovalDecision, ApprovalEvent, ApprovalFeed, BackgroundDecideReport,
    BackgroundOutcome, DecideStage, DecisionOutcome, PendingApproval,
};
pub use attachments::UploadProgress;
use conn::{
    Conn, ConnectError, FOREGROUND_RECONNECT, IdentitySlot, LinkPhase, NodeSlot, PushSlot,
    RequestError, blocking,
};
pub use identity::IdentitySigner;
use reach::Reachability;
use session::{CALL_TIMEOUT, SessionError, expect_flock, expect_paired, lock, unexpected};
pub use store::{Machine, MachineKind};
use store::{MachineStore, random_id};

uniffi::setup_scaffolding!();

const HOSTNAME: &str = "collie-phone";
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(15);
/// Longer than the dial budget and the lock-screen decide's budget, the longest any caller
/// holds a node.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(30);
/// collied holds `pair.complete` until the user confirms on the Mac (up to 65 s).
const PAIR_CONFIRM_TIMEOUT: Duration = Duration::from_secs(75);
/// Covers a reconnect (backoff up to 16 s plus the dial) before the retried send.
const DRIVE_TIMEOUT: Duration = Duration::from_secs(30);
/// herdr's `agent.start` waits up to 30 s for the agent before collied prompts it.
const TASK_NEW_TIMEOUT: Duration = Duration::from_secs(90);
/// An unanswered mutation's `op_id` is reused for an identical retry within this window.
/// Longer than any mutation timeout, shorter than collied's 600 s outcome cache.
const OP_REUSE_WINDOW: Duration = Duration::from_secs(180);
/// collied waits for the agent to leave `blocked` before it answers `approval.decide`.
const DECIDE_TIMEOUT: Duration = Duration::from_secs(20);
/// Of the roughly 25 s iOS gives a background action, leaving time to post a fallback.
const BACKGROUND_BUDGET: Duration = Duration::from_secs(20);
/// How long a suspend lets prompts, uploads and decisions in flight finish.
const SUSPEND_GRACE: Duration = Duration::from_secs(5);
/// While an upload runs: the rest of the roughly 25 s background task, after the app's
/// wait for Live Activity tokens (3 s at most) and before `SUSPEND_CLOSE`.
const SUSPEND_UPLOAD_GRACE: Duration = Duration::from_secs(18);
/// Longer than the 2 s a session takes at most to send its WebSocket close.
const SUSPEND_CLOSE: Duration = Duration::from_secs(3);
/// A wedged node must not hold up the diagnostics or the recovery after a resume.
const KICK_TIMEOUT: Duration = Duration::from_secs(3);
/// Longer than a healthy reconnect after a resume. A rebind is not done sooner: a dial in
/// flight during one can stall for its whole attempt.
const REBIND_AFTER: Duration = Duration::from_secs(3);
/// A full dial past the resume grace: a rebound node has reached DERP again long before.
const RESTART_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum CoreError {
    #[error("state dir must be a 0700 directory owned by the current user")]
    InsecureStateDir,
    #[error("Tailscale is not connected yet")]
    NotRunning,
    #[error("this is not a collie pairing code")]
    InvalidInvite,
    #[error("device name must be 1 to 64 characters without control characters")]
    InvalidLabel,
    #[error("unknown machine")]
    MachineNotFound,
    #[error("refusing to connect: {message}")]
    PinViolation { message: String },
    #[error("{message}")]
    Unreachable { message: String },
    #[error("the machine rejected the request: {message}")]
    Rejected { message: String },
    #[error("{message}")]
    InvalidInput {
        field: Option<String>,
        message: String,
    },
    #[error("the agent is waiting for an approval")]
    AgentBlocked,
    #[error("the agent is not ready for input")]
    AgentNotReady,
    #[error("this needs confirmation")]
    ConfirmRequired,
    #[error("the agent or workspace no longer exists")]
    NotFound,
    #[error("herdr is not running on the machine")]
    HerdrUnavailable,
    #[error("too many requests, try again in a moment")]
    RateLimited,
    #[error("this approval is no longer pending")]
    ApprovalNotFound,
    #[error("this approval expired")]
    ApprovalExpired,
    #[error("this approval was already answered")]
    ApprovalAlreadyResolved,
    #[error("the machine does not support this yet, update collied")]
    NotImplemented,
    #[error("{message}")]
    TooLarge { message: String },
    #[error("the file did not arrive intact, try again")]
    ChecksumMismatch,
    #[error("upload cancelled")]
    Cancelled,
    #[error("The agent's input box has unsent text.")]
    DraftChanged { current: String },
    #[error("Could not clear the agent's input box; nothing was sent.")]
    DraftNotCleared,
    #[error("stopped retrying: {message}. Pair this machine again.")]
    Unauthorized { message: String },
    #[error("tailnet: {message}")]
    Tailnet { message: String },
    #[error("storage: {message}")]
    Storage { message: String },
    #[error("internal: {message}")]
    Internal { message: String },
}

impl From<tailnet::Error> for CoreError {
    fn from(e: tailnet::Error) -> Self {
        match e {
            tailnet::Error::InsecureStateDir(_) => Self::InsecureStateDir,
            other => Self::Tailnet {
                message: other.to_string(),
            },
        }
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Storage {
            message: e.to_string(),
        }
    }
}

impl From<ConnectError> for CoreError {
    fn from(e: ConnectError) -> Self {
        let message = e.to_string();
        match e {
            ConnectError::Offline => Self::NotRunning,
            ConnectError::Pin(p) if p.is_violation() => Self::PinViolation { message },
            ConnectError::Session(s) => s.into(),
            _ => Self::Unreachable { message },
        }
    }
}

impl From<SessionError> for CoreError {
    fn from(e: SessionError) -> Self {
        let message = e.to_string();
        match e {
            _ if e.is_auth() => Self::Unauthorized { message },
            SessionError::Server { code, .. } => match code {
                ErrorCode::InvalidParams => Self::InvalidInput {
                    field: invalid_field(&message),
                    message,
                },
                ErrorCode::ApprovalNotFound => Self::ApprovalNotFound,
                ErrorCode::ApprovalExpired => Self::ApprovalExpired,
                ErrorCode::ApprovalAlreadyResolved => Self::ApprovalAlreadyResolved,
                ErrorCode::AgentBlocked => Self::AgentBlocked,
                ErrorCode::AgentNotReady => Self::AgentNotReady,
                ErrorCode::ConfirmRequired => Self::ConfirmRequired,
                ErrorCode::NotFound => Self::NotFound,
                ErrorCode::HerdrUnavailable => Self::HerdrUnavailable,
                ErrorCode::RateLimited => Self::RateLimited,
                ErrorCode::NotImplemented | ErrorCode::UnknownMethod => Self::NotImplemented,
                ErrorCode::TooLarge => Self::TooLarge { message },
                ErrorCode::ChecksumMismatch => Self::ChecksumMismatch,
                ErrorCode::DraftNotCleared => Self::DraftNotCleared,
                _ => Self::Rejected { message },
            },
            SessionError::DraftChanged { current } => Self::DraftChanged { current },
            _ => Self::Unreachable { message },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TailnetState {
    NotStarted,
    NoState,
    NeedsLogin,
    NeedsMachineAuth,
    Stopped,
    Starting,
    Running,
    Unknown,
}

impl From<BackendState> for TailnetState {
    fn from(s: BackendState) -> Self {
        match s {
            BackendState::NoState => Self::NoState,
            BackendState::NeedsLogin => Self::NeedsLogin,
            BackendState::NeedsMachineAuth => Self::NeedsMachineAuth,
            BackendState::Stopped => Self::Stopped,
            BackendState::Starting => Self::Starting,
            BackendState::Running => Self::Running,
            BackendState::Other => Self::Unknown,
        }
    }
}

/// No Debug: `auth_url` is a login capability for this node until it is used.
#[derive(Clone, uniffi::Record)]
pub struct NodeState {
    pub backend_state: TailnetState,
    pub auth_url: Option<String>,
    pub self_dns_name: Option<String>,
    pub login_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AgentState {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl From<protocol::AgentStatus> for AgentState {
    fn from(s: protocol::AgentStatus) -> Self {
        match s {
            protocol::AgentStatus::Idle => Self::Idle,
            protocol::AgentStatus::Working => Self::Working,
            protocol::AgentStatus::Blocked => Self::Blocked,
            protocol::AgentStatus::Done => Self::Done,
            protocol::AgentStatus::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MachineDetails {
    pub name: String,
    pub node_id: String,
    pub herdr_session: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WorkspaceSummary {
    pub workspace_id: String,
    pub label: String,
    pub number: u32,
    pub status: AgentState,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentSummary {
    pub terminal_id: String,
    pub workspace_id: String,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: AgentState,
    pub status_since_ms: u64,
    pub cwd: Option<String>,
    pub last_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MachineFlock {
    pub machine: Machine,
    pub link: LinkPhase,
    pub last_error: Option<String>,
    pub details: Option<MachineDetails>,
    pub workspaces: Vec<WorkspaceSummary>,
    pub agents: Vec<AgentSummary>,
    pub approvals_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TerminalSource {
    Visible,
    Recent,
}

impl From<TerminalSource> for ReadSource {
    fn from(s: TerminalSource) -> Self {
        match s {
            TerminalSource::Visible => Self::Visible,
            TerminalSource::Recent => Self::Recent,
        }
    }
}

impl From<ReadSource> for TerminalSource {
    fn from(s: ReadSource) -> Self {
        match s {
            ReadSource::Visible => Self::Visible,
            ReadSource::Recent => Self::Recent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TerminalSnapshot {
    pub terminal_id: String,
    pub source: TerminalSource,
    pub ansi: String,
    pub truncated: bool,
}

impl From<TerminalRead> for TerminalSnapshot {
    fn from(r: TerminalRead) -> Self {
        Self {
            terminal_id: r.terminal_id.into(),
            source: r.source.into(),
            ansi: r.ansi,
            truncated: r.truncated,
        }
    }
}

/// `output` is set only when it is newer than the caller's `after_revision`;
/// `output_revision` never decreases, across reconnects and watch changes.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentView {
    pub link: LinkPhase,
    pub last_error: Option<String>,
    pub agent: Option<AgentSummary>,
    pub output: Option<TerminalSnapshot>,
    pub output_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AgentKey {
    Enter,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Tab,
    ShiftTab,
    CtrlC,
    CtrlEnter,
    Y,
    N,
}

impl From<AgentKey> for Key {
    fn from(k: AgentKey) -> Self {
        match k {
            AgentKey::Enter => Self::Enter,
            AgentKey::Esc => Self::Esc,
            AgentKey::Up => Self::Up,
            AgentKey::Down => Self::Down,
            AgentKey::Left => Self::Left,
            AgentKey::Right => Self::Right,
            AgentKey::Tab => Self::Tab,
            AgentKey::ShiftTab => Self::ShiftTab,
            AgentKey::CtrlC => Self::CtrlC,
            AgentKey::CtrlEnter => Self::CtrlEnter,
            AgentKey::Y => Self::Y,
            AgentKey::N => Self::N,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TaskOptions {
    pub agents: Vec<String>,
    pub default_agent: String,
    pub recent_cwds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TaskStarted {
    pub workspace_id: String,
    pub terminal_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PushEnvironment {
    Sandbox,
    Production,
}

impl From<PushEnvironment> for protocol::ApnsEnvironment {
    fn from(e: PushEnvironment) -> Self {
        match e {
            PushEnvironment::Sandbox => Self::Sandbox,
            PushEnvironment::Production => Self::Production,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BuildInfo {
    pub core_version: String,
    pub rustc: String,
    pub go: String,
    pub target: String,
    pub release: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ColdStartReport {
    pub node_created_ms: u64,
    pub started_ms: u64,
    pub settled_ms: Option<u64>,
    pub backend_state: TailnetState,
    pub auth_url_present: bool,
    pub status_polls: u32,
    pub build: BuildInfo,
}

#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").into()
}

#[uniffi::export]
pub fn protocol_version() -> u32 {
    protocol::PROTOCOL_VERSION
}

#[uniffi::export]
pub fn build_info() -> BuildInfo {
    BuildInfo {
        core_version: core_version(),
        rustc: env!("COLLIE_RUSTC_VERSION").into(),
        go: env!("COLLIE_GO_VERSION").into(),
        target: env!("COLLIE_TARGET").into(),
        release: !cfg!(debug_assertions),
    }
}

/// Diagnostics for the device log. Called from a collie-core thread; never carries keys,
/// tokens or nonces.
#[uniffi::export(callback_interface)]
pub trait CoreLog: Send + Sync {
    fn log(&self, message: String);
}

#[derive(uniffi::Object)]
pub struct CollieCore {
    runtime: tokio::runtime::Runtime,
    inner: Arc<Inner>,
}

struct Inner {
    state_dir: PathBuf,
    control_url: Option<String>,
    node: NodeSlot,
    identity: IdentitySlot,
    starting: Mutex<()>,
    store: MachineStore,
    machines: Mutex<Vec<Machine>>,
    conns: Mutex<HashMap<String, Arc<Conn>>>,
    cold_start: Mutex<Option<ColdStartReport>>,
    measured: AtomicBool,
    login_name: Mutex<Option<String>>,
    push: PushSlot,
    reach: Arc<Reachability>,
    ops: Mutex<HashMap<String, (OpId, Instant)>>,
    uploads: Mutex<HashMap<u64, (String, watch::Sender<bool>)>>,
    next_upload: AtomicU64,
    suspended: watch::Sender<bool>,
    resumes: AtomicU64,
    busy: watch::Sender<usize>,
    log: Mutex<Option<Box<dyn CoreLog>>>,
}

/// A prompt, upload or decision in flight, which a suspend lets finish.
struct Busy(Arc<Inner>);

impl Busy {
    fn new(inner: &Arc<Inner>) -> Self {
        inner.busy.send_modify(|n| *n += 1);
        Self(inner.clone())
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.busy.send_modify(|n| *n -= 1);
    }
}

#[uniffi::export]
impl CollieCore {
    #[uniffi::constructor]
    pub fn new(state_dir: String) -> Result<Arc<Self>, CoreError> {
        Self::build(PathBuf::from(state_dir), None)
    }

    pub fn set_identity(
        &self,
        public_key: Vec<u8>,
        signer: Box<dyn IdentitySigner>,
    ) -> Result<(), CoreError> {
        let key = identity::certified(public_key, signer).map_err(|e| CoreError::InvalidInput {
            field: Some("public_key".into()),
            message: e.to_string(),
        })?;
        *lock(&self.inner.identity) = Some(key);
        Ok(())
    }

    pub fn set_log(&self, log: Box<dyn CoreLog>) {
        *lock(&self.inner.log) = Some(log);
    }

    pub fn tailnet_configured(&self) -> bool {
        self.inner.tailnet_configured()
    }

    /// The App Group container, where `reachability.json` is kept current for the
    /// Notification Service Extension.
    pub fn set_app_group_dir(&self, path: String) -> Result<(), CoreError> {
        Ok(self.inner.reach.set_dir(PathBuf::from(path))?)
    }

    /// The key is used once to build the node and never stored by collie-core.
    pub async fn node_start(&self, auth_key: Option<String>) -> Result<(), CoreError> {
        let key = auth_key
            .map(Zeroizing::new)
            .map(|k| Zeroizing::new(k.trim().to_owned()))
            .filter(|k| !k.is_empty());
        let inner = self.inner.clone();
        self.run(async move { blocking(move || inner.node_start(key)).await? })
            .await
    }

    pub async fn node_state(&self) -> Result<NodeState, CoreError> {
        let inner = self.inner.clone();
        self.run(async move { blocking(move || inner.node_state()).await? })
            .await
    }

    pub fn cold_start_report(&self) -> Option<ColdStartReport> {
        lock(&self.inner.cold_start).clone()
    }

    pub async fn pair(
        &self,
        invite_uri: String,
        device_label: String,
    ) -> Result<Machine, CoreError> {
        let inner = self.inner.clone();
        self.run(async move { inner.pair(&invite_uri, &device_label).await })
            .await
    }

    pub fn machines(&self) -> Vec<Machine> {
        lock(&self.inner.machines).clone()
    }

    /// Asks a connected machine to revoke this phone first, so it stops sending pushes, and
    /// returns whether it did. The machine is forgotten either way.
    pub async fn remove_machine(&self, id: String) -> Result<bool, CoreError> {
        let conn = lock(&self.inner.conns).get(&id).cloned();
        let unpaired = match conn {
            Some(conn) if lock(&conn.shared.link).phase == LinkPhase::Connected => self
                .run(async move {
                    let reply = conn.request(Request::Unpair(Empty {}), CALL_TIMEOUT).await;
                    Ok(matches!(reply, Ok(Response::Ok)))
                })
                .await
                .unwrap_or(false),
            _ => false,
        };
        self.inner.update_machines(|m| m.id == id, None)?;
        Ok(unpaired)
    }

    /// Answers from the cache while the link is down so a refresh never waits out
    /// CALL_TIMEOUT on an unreachable Mac.
    pub async fn flock(&self, machine_id: String) -> Result<MachineFlock, CoreError> {
        let conn = self.conn(&machine_id)?;
        if lock(&conn.shared.link).phase != LinkPhase::Connected {
            return Ok(view(&conn));
        }
        self.run(async move {
            match conn
                .request(Request::FlockSnapshot(Empty {}), CALL_TIMEOUT)
                .await
            {
                Ok(response) => {
                    expect_flock(response).map_err(CoreError::from)?;
                }
                // The session ended under the request (a resume reconnect, or iOS killed
                // the socket): the link phase reports it, and the supervisor reconnects.
                Err(RequestError::Failed(e)) if e.is_transport() => {}
                Err(e) => return Err(request_error(&conn, e)),
            }
            Ok(view(&conn))
        })
        .await
    }

    pub fn cached_flock(&self, machine_id: String) -> Option<MachineFlock> {
        lock(&self.inner.conns).get(&machine_id).map(|c| view(c))
    }

    /// After a long background (or a suspend), when no machine is connected again
    /// [`REBIND_AFTER`] later the node is rebound, and when dials still fail
    /// [`RESTART_AFTER`] after the resume it is restarted.
    pub fn resume(&self, background_secs: u64) {
        // Bumped before the send: a suspend checking it under the watch lock either sees
        // it or is undone by the send.
        let epoch = self.inner.resumes.fetch_add(1, Ordering::SeqCst) + 1;
        let suspended = self.inner.suspended.send_replace(false);
        let background = Duration::from_secs(background_secs);
        let conns: Vec<_> = lock(&self.inner.conns).values().cloned().collect();
        let links: Vec<_> = conns
            .iter()
            .map(|c| {
                let link = lock(&c.shared.link);
                let line = format!(
                    "{}: {:?}, last dial {} s ago, last error {:?}",
                    c.machine.label,
                    link.phase,
                    link.last_dial.elapsed().as_secs(),
                    link.last_error
                );
                (c.machine.node_id.clone(), line)
            })
            .collect();
        for conn in &conns {
            conn.resume(background, suspended);
        }
        let inner = self.inner.clone();
        self.runtime
            .spawn(inner.resumed(background, suspended, epoch, links));
    }

    /// Pull to refresh or a tap on the machine: a live session is probed, any other link
    /// dials at once, even when Tailscale reports the machine offline. When the node runs
    /// and the machine is online but dials keep failing, the node is restarted instead, and
    /// when there is no node (a restart that could not start one) it is started.
    pub fn reconnect(&self, machine_id: String) -> Result<(), CoreError> {
        let conn = self.conn(&machine_id)?;
        let inner = self.inner.clone();
        self.runtime.spawn(async move {
            inner.start_missing_node("reconnect").await;
            if let Some(suspect) = inner.node_suspect(std::slice::from_ref(&conn)).await {
                inner.restart("reconnect", suspect).await;
            } else if lock(&conn.shared.link).phase == LinkPhase::Connected {
                conn.resume(Duration::ZERO, false);
            } else {
                conn.reconnect_now();
            }
        });
        Ok(())
    }

    /// Called as the app leaves the foreground, before any await: the epoch to pass to
    /// [`Self::suspend`].
    pub fn begin_suspend(&self) -> u64 {
        self.inner.resumes.load(Ordering::SeqCst)
    }

    /// For the app leaving the foreground, inside a background task. Once the work in
    /// flight is done (5 s at most), every session is closed, so collied drops it and its
    /// watch at once, and no supervisor dials again until [`Self::resume`], even when iOS
    /// wakes the app for a lock-screen decide, which opens its own connection. Does nothing
    /// when a resume came after the [`Self::begin_suspend`] that returned `epoch`.
    pub async fn suspend(&self, epoch: u64) {
        let inner = self.inner.clone();
        let _ = self
            .runtime
            .spawn(async move {
                let mut busy = inner.busy.subscribe();
                let grace = if lock(&inner.uploads).is_empty() {
                    SUSPEND_GRACE
                } else {
                    SUSPEND_UPLOAD_GRACE
                };
                let _ = tokio::time::timeout(grace, busy.wait_for(|n| *n == 0)).await;
                let suspending = inner.suspended.send_if_modified(|s| {
                    let now = !*s && inner.resumes.load(Ordering::SeqCst) == epoch;
                    *s |= now;
                    now
                });
                if !suspending {
                    return;
                }
                let conns: Vec<_> = lock(&inner.conns).values().cloned().collect();
                let deadline = Instant::now() + SUSPEND_CLOSE;
                while Instant::now() < deadline
                    && conns
                        .iter()
                        .any(|c| lock(&c.shared.link).phase == LinkPhase::Connected)
                {
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            })
            .await;
    }

    pub async fn agent_read(
        &self,
        machine_id: String,
        terminal_id: String,
        source: TerminalSource,
        lines: Option<u16>,
    ) -> Result<TerminalSnapshot, CoreError> {
        let source = ReadSource::from(source);
        let request = Request::AgentRead(ReadParams {
            terminal_id: terminal(terminal_id)?,
            source,
            lines: lines.or((source == ReadSource::Recent).then_some(limits::DEFAULT_WATCH_LINES)),
        });
        match self.call(&machine_id, request, CALL_TIMEOUT).await? {
            Response::Terminal(read) => Ok(read.into()),
            other => Err(unexpected(&other).into()),
        }
    }

    /// One watched agent per machine; `None` stops the watch. The choice outlives the
    /// connection: every new session re-issues `agent.watch`, so while the link is down
    /// this only records it. Poll [`Self::agent_view`] for output, which starts with the
    /// watch's first full `agent.output`.
    pub async fn watch_agent(
        &self,
        machine_id: String,
        terminal_id: Option<String>,
        lines: u16,
    ) -> Result<(), CoreError> {
        let terminal_id = terminal_id.map(terminal).transpose()?;
        let conn = self.conn(&machine_id)?;
        {
            let mut state = lock(&conn.shared.flock);
            state.watch(terminal_id.clone());
            state.watch_lines = Some(lines);
        }
        if lock(&conn.shared.link).phase != LinkPhase::Connected {
            return Ok(());
        }
        self.run(async move {
            let watch = Request::AgentWatch(AgentWatchParams {
                terminal_id,
                lines: Some(lines),
            });
            let response = conn
                .request(watch, CALL_TIMEOUT)
                .await
                .map_err(|e| request_error(&conn, e))?;
            expect_ok(response)
        })
        .await
    }

    /// Local and cheap, meant to be polled while the agent screen is visible. `None`
    /// when the machine has no connection yet.
    pub fn agent_view(
        &self,
        machine_id: String,
        terminal_id: String,
        after_revision: u64,
    ) -> Option<AgentView> {
        let conn = lock(&self.inner.conns).get(&machine_id).cloned()?;
        let (link, last_error) = {
            let link = lock(&conn.shared.link);
            (link.phase, link.last_error.clone())
        };
        let state = lock(&conn.shared.flock);
        Some(AgentView {
            link,
            last_error,
            agent: state
                .flock
                .iter()
                .flat_map(|f| &f.agents)
                .find(|a| a.terminal_id.as_str() == terminal_id)
                .map(agent_summary),
            output: state
                .output
                .as_ref()
                .filter(|o| {
                    o.terminal_id.as_str() == terminal_id && state.output_revision > after_revision
                })
                .map(|o| o.clone().into()),
            output_revision: state.output_revision,
        })
    }

    /// The unsent text in a Claude Code agent's input box on the Mac: `None` when it
    /// cannot be read (another agent kind, a dialog, or an older collied).
    pub async fn agent_draft(
        &self,
        machine_id: String,
        terminal_id: String,
    ) -> Result<Option<String>, CoreError> {
        let request = Request::AgentDraft(AgentTarget {
            terminal_id: terminal(terminal_id)?,
        });
        match self.call(&machine_id, request, CALL_TIMEOUT).await {
            Ok(Response::Draft { text }) => Ok(text),
            Ok(other) => Err(unexpected(&other).into()),
            Err(CoreError::NotImplemented) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `expected_draft` is what the phone last saw in the Mac's input box: collied
    /// replaces that draft, and refuses with `DraftChanged` if the box holds anything else.
    pub async fn prompt(
        &self,
        machine_id: String,
        terminal_id: String,
        text: String,
        expected_draft: Option<String>,
    ) -> Result<(), CoreError> {
        let request = Request::AgentPrompt(AgentPromptParams {
            op_id: new_op_id(),
            terminal_id: terminal(terminal_id)?,
            text: prompt_text(text)?,
            expected_draft: expected_draft
                .map(DraftText::new)
                .transpose()
                .map_err(|_| invalid("expected_draft", "invalid draft"))?,
        });
        expect_ok(self.mutate(&machine_id, request, DRIVE_TIMEOUT).await?)
    }

    pub async fn send_keys(
        &self,
        machine_id: String,
        terminal_id: String,
        keys: Vec<AgentKey>,
    ) -> Result<(), CoreError> {
        if !(1..=limits::MAX_KEYS_PER_CALL).contains(&keys.len()) {
            return Err(invalid("keys", "send 1 to 16 keys at a time"));
        }
        let request = Request::AgentSendKeys(AgentSendKeysParams {
            op_id: new_op_id(),
            terminal_id: terminal(terminal_id)?,
            keys: keys.into_iter().map(Key::from).collect(),
        });
        expect_ok(self.mutate(&machine_id, request, DRIVE_TIMEOUT).await?)
    }

    /// Answers a blocked prompt that offers no approval decision through its free-text
    /// field, a question's "Type something." or a plan's "Tell Claude what to change":
    /// collied types `text`, then Enter. One line. Refused with `AgentBlocked` on a
    /// permission prompt, `AgentNotReady` when the agent is not blocked.
    pub async fn type_text(
        &self,
        machine_id: String,
        terminal_id: String,
        text: String,
    ) -> Result<(), CoreError> {
        let text = PromptText::new(text)
            .ok()
            .filter(|t| !t.as_str().contains(['\n', '\t']))
            .ok_or_else(|| {
                invalid(
                    "text",
                    "an answer must be one non-empty line of at most 32 KiB, without control characters",
                )
            })?;
        let request = Request::AgentTypeText(AgentTypeTextParams {
            op_id: new_op_id(),
            terminal_id: terminal(terminal_id)?,
            text,
        });
        expect_ok(self.mutate(&machine_id, request, DRIVE_TIMEOUT).await?)
    }

    /// Brings the agent's pane to the front in herdr on the Mac.
    pub async fn focus(&self, machine_id: String, terminal_id: String) -> Result<(), CoreError> {
        let request = Request::AgentFocus(AgentTarget {
            terminal_id: terminal(terminal_id)?,
        });
        expect_ok(self.call(&machine_id, request, CALL_TIMEOUT).await?)
    }

    pub async fn task_options(&self, machine_id: String) -> Result<TaskOptions, CoreError> {
        match self
            .call(&machine_id, Request::TaskOptions(Empty {}), CALL_TIMEOUT)
            .await?
        {
            Response::TaskOptions(o) => Ok(TaskOptions {
                agents: o.agents.into_iter().map(String::from).collect(),
                default_agent: o.default_agent.into(),
                recent_cwds: o.recent_cwds.into_iter().map(String::from).collect(),
            }),
            other => Err(unexpected(&other).into()),
        }
    }

    pub async fn task_new(
        &self,
        machine_id: String,
        cwd: String,
        agent: String,
        prompt: String,
        label: Option<String>,
    ) -> Result<TaskStarted, CoreError> {
        let label = label
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .map(|l| {
                Label::new(l).map_err(|_| {
                    invalid(
                        "label",
                        "label must be 1 to 64 characters without control characters",
                    )
                })
            })
            .transpose()?;
        let request = Request::TaskNew(TaskNewParams {
            op_id: new_op_id(),
            cwd: Cwd::new(cwd).map_err(|_| {
                invalid(
                    "cwd",
                    "folder must be an absolute path of at most 1024 bytes",
                )
            })?,
            agent: AgentKind::new(agent).map_err(|_| invalid("agent", "unknown agent kind"))?,
            prompt: prompt_text(prompt)?,
            label,
        });
        match self.mutate(&machine_id, request, TASK_NEW_TIMEOUT).await? {
            Response::TaskStarted {
                workspace_id,
                terminal_id,
            } => Ok(TaskStarted {
                workspace_id: workspace_id.into(),
                terminal_id: terminal_id.into(),
            }),
            other => Err(unexpected(&other).into()),
        }
    }

    /// Fails with `ConfirmRequired` unless `confirm` is true.
    pub async fn close_workspace(
        &self,
        machine_id: String,
        workspace_id: String,
        confirm: bool,
    ) -> Result<(), CoreError> {
        let request = Request::WorkspaceClose(WorkspaceCloseParams {
            workspace_id: WorkspaceId::new(workspace_id)
                .map_err(|_| invalid("workspace_id", "invalid workspace id"))?,
            confirm,
        });
        expect_ok(self.call(&machine_id, request, CALL_TIMEOUT).await?)
    }

    pub async fn approvals(&self, machine_id: String) -> Result<Vec<PendingApproval>, CoreError> {
        match self
            .call(&machine_id, Request::ApprovalList(Empty {}), CALL_TIMEOUT)
            .await?
        {
            Response::Approvals { approvals } => Ok(approvals.iter().map(Into::into).collect()),
            other => Err(unexpected(&other).into()),
        }
    }

    /// Uses the nonce collie-core holds from the flock and `approval.needed`, fetching
    /// `approval.list` when it has none. Not retried: the nonce is single use. `note`, one
    /// line, goes with Approve or Deny on an approval with `supports_note`.
    pub async fn decide(
        &self,
        machine_id: String,
        approval_id: String,
        decision: ApprovalDecision,
        note: Option<String>,
    ) -> Result<DecisionOutcome, CoreError> {
        let approval_id = ApprovalId::new(approval_id)
            .map_err(|_| invalid("approval_id", "invalid approval id"))?;
        let note = match note {
            None => None,
            Some(note) => Some(
                PromptText::new(note)
                    .ok()
                    .filter(|n| !n.as_str().contains(['\n', '\t']))
                    .filter(|n| n.as_str().chars().count() <= limits::MAX_NOTE_CHARS)
                    .filter(|_| {
                        matches!(decision, ApprovalDecision::Approve | ApprovalDecision::Deny)
                    })
                    .ok_or_else(|| {
                        invalid(
                            "note",
                            "a note must be one non-empty line of at most 200 characters, without control characters, with Approve or Deny",
                        )
                    })?,
            ),
        };
        let conn = self.conn(&machine_id)?;
        let _busy = Busy::new(&self.inner);
        self.run(async move {
            let cached = approvals::cached_nonce(&lock(&conn.shared.flock), &approval_id);
            let nonce = match cached {
                Some(nonce) => nonce,
                None => {
                    let list = conn
                        .request(Request::ApprovalList(Empty {}), CALL_TIMEOUT)
                        .await
                        .map_err(|e| request_error(&conn, e))?;
                    approvals::listed_nonce(list, &approval_id)?
                        .ok_or(CoreError::ApprovalNotFound)?
                }
            };
            let request = Request::ApprovalDecide(approvals::decide_params(
                approval_id,
                decision,
                nonce,
                note,
            ));
            let response = conn
                .request(request, DECIDE_TIMEOUT)
                .await
                .map_err(|e| request_error(&conn, e))?;
            Ok(approvals::expect_resolved(response)?.into())
        })
        .await
    }

    /// Local and cheap, meant to be polled. `None` when the machine has no connection yet.
    pub fn approval_feed(&self, machine_id: String, after_revision: u64) -> Option<ApprovalFeed> {
        let conn = lock(&self.inner.conns).get(&machine_id).cloned()?;
        let link = lock(&conn.shared.link).phase;
        Some(approvals::feed(
            link,
            &lock(&conn.shared.flock),
            after_revision,
        ))
    }

    /// Lock-screen Approve/Deny. Starts the tailnet from cached state if needed (never
    /// a login), dials the Mac pinned to `machine_node_id`, fetches the nonce and
    /// decides, all within `budget_ms` (default and at most 20 s, 0 is refused). Never
    /// throws: the report carries the outcome and step timings.
    pub async fn decide_from_notification(
        &self,
        machine_node_id: String,
        approval_id: String,
        decision: ApprovalDecision,
        budget_ms: Option<u64>,
    ) -> BackgroundDecideReport {
        let budget = match budget_ms {
            None => BACKGROUND_BUDGET,
            Some(0) => return BackgroundDecideReport::failed("budget_ms must be positive".into()),
            Some(ms) => Duration::from_millis(ms).min(BACKGROUND_BUDGET),
        };
        let inner = self.inner.clone();
        let fut =
            approvals::decide_in_background(inner, machine_node_id, approval_id, decision, budget);
        match self.runtime.spawn(fut).await {
            Ok(report) => report,
            Err(e) => BackgroundDecideReport::failed(e.to_string()),
        }
    }

    /// Sends `push.register` to that machine if it is connected; every later connection
    /// of this process registers it again. Kept in memory only: the notification key's
    /// one store is the iOS Keychain, so the app registers each machine on launch.
    pub fn register_push(
        &self,
        machine_id: String,
        apns_token_hex: String,
        environment: PushEnvironment,
        notification_key: Vec<u8>,
    ) -> Result<(), CoreError> {
        let notification_key = Zeroizing::new(notification_key);
        if notification_key.len() != 32 {
            return Err(invalid(
                "notification_key",
                "notification key must be 32 bytes",
            ));
        }
        let push = PushRegisterParams {
            apns_token: PushToken::new(apns_token_hex.trim()).map_err(|_| {
                invalid("apns_token", "APNs token must be 64 to 256 hex characters")
            })?,
            live_activity_push_to_start_token: None,
            environment: environment.into(),
            notification_key: NotificationKey::new(URL_SAFE_NO_PAD.encode(&*notification_key))
                .expect("32 bytes encode to a canonical 43-char base64url key"),
        };
        {
            let machines = lock(&self.inner.machines);
            if !machines.iter().any(|m| m.id == machine_id) {
                return Err(CoreError::MachineNotFound);
            }
            lock(&self.inner.push)
                .entry(machine_id.clone())
                .or_default()
                .push = Some(push.clone());
        }
        self.send_if_connected(&machine_id, Request::PushRegister(push), None)
    }

    /// Sends a Live Activity's update token to the Mac that runs `terminal_id`, now if
    /// connected and again on every later connection of this process. collied takes the
    /// APNs environment from `push.register`, so call [`Self::register_push`] first.
    /// Kept in memory only.
    pub fn register_activity_token(
        &self,
        machine_id: String,
        activity_id: String,
        terminal_id: String,
        token_hex: String,
    ) -> Result<(), CoreError> {
        let params = PushActivityTokenParams {
            activity_id: activity(activity_id)?,
            terminal_id: terminal(terminal_id)?,
            token: PushToken::new(token_hex.trim())
                .map_err(|_| invalid("token", "activity token must be 64 to 256 hex characters"))?,
            // The app restarts activities an older build started before it registers any.
            shows_approvals: true,
        };
        {
            let machines = lock(&self.inner.machines);
            if !machines.iter().any(|m| m.id == machine_id) {
                return Err(CoreError::MachineNotFound);
            }
            let mut push = lock(&self.inner.push);
            let reg = push.entry(machine_id.clone()).or_default();
            reg.unsent_ends.retain(|a| *a != params.activity_id);
            reg.activities
                .insert(params.activity_id.as_str().to_owned(), params.clone());
        }
        self.send_if_connected(&machine_id, Request::PushActivityToken(params), None)
    }

    /// Stops the Mac pushing to that activity. Sent now if connected, else with the next
    /// connection.
    pub fn end_activity(&self, machine_id: String, activity_id: String) -> Result<(), CoreError> {
        let activity_id = activity(activity_id)?;
        {
            let machines = lock(&self.inner.machines);
            if !machines.iter().any(|m| m.id == machine_id) {
                return Err(CoreError::MachineNotFound);
            }
            lock(&self.inner.push)
                .entry(machine_id.clone())
                .or_default()
                .end(&activity_id);
        }
        let request = Request::PushActivityEnd(PushActivityEndParams {
            activity_id: activity_id.clone(),
        });
        self.send_if_connected(&machine_id, request, Some(activity_id))
    }

    pub fn max_attachment_bytes(&self) -> u64 {
        limits::MAX_ATTACHMENT_BYTES
    }

    /// Stores `data` in collied's private attachments dir and returns its absolute path
    /// on the Mac. `progress` is called after each acknowledged chunk. A dropped
    /// connection fails the upload; nothing resumes. UniFFI does not forward Swift task
    /// cancellation to Rust, so cancel with [`Self::cancel_uploads`].
    pub async fn upload_attachment(
        &self,
        machine_id: String,
        name: String,
        data: Vec<u8>,
        progress: Box<dyn UploadProgress>,
    ) -> Result<String, CoreError> {
        let name = attachments::check(&name, &data)?;
        let conn = self.conn(&machine_id)?;
        let _busy = Busy::new(&self.inner);
        let (cancel, mut cancelled) = watch::channel(false);
        let key = self.inner.next_upload.fetch_add(1, Ordering::Relaxed);
        lock(&self.inner.uploads).insert(key, (machine_id, cancel));
        let result = self
            .run(async move {
                let call = |request: Request| {
                    let conn = conn.clone();
                    async move {
                        let result = if matches!(request, Request::AttachmentCommit(_)) {
                            conn.mutate(request, DRIVE_TIMEOUT).await
                        } else {
                            conn.request(request, CALL_TIMEOUT).await
                        };
                        result.map_err(|e| request_error(&conn, e))
                    }
                };
                let cancel = async move {
                    if cancelled.wait_for(|c| *c).await.is_err() {
                        std::future::pending::<()>().await;
                    }
                };
                attachments::upload(call, name, data, progress, cancel).await
            })
            .await;
        lock(&self.inner.uploads).remove(&key);
        result
    }

    /// Cancels every upload in progress to that machine; each fails with `Cancelled`
    /// and is aborted on the Mac.
    pub fn cancel_uploads(&self, machine_id: String) {
        for (machine, cancel) in lock(&self.inner.uploads).values() {
            if *machine == machine_id {
                let _ = cancel.send(true);
            }
        }
    }

    /// Fails with `ConfirmRequired` unless `confirm` is true.
    pub async fn close_pane(
        &self,
        machine_id: String,
        terminal_id: String,
        confirm: bool,
    ) -> Result<(), CoreError> {
        let request = Request::PaneClose(PaneCloseParams {
            terminal_id: terminal(terminal_id)?,
            confirm,
        });
        expect_ok(self.call(&machine_id, request, CALL_TIMEOUT).await?)
    }
}

impl CollieCore {
    /// Rust-only entry point for tests against a local control server.
    pub fn with_control_url(
        state_dir: PathBuf,
        control_url: String,
    ) -> Result<Arc<Self>, CoreError> {
        Self::build(state_dir, Some(control_url))
    }

    fn build(state_dir: PathBuf, control_url: Option<String>) -> Result<Arc<Self>, CoreError> {
        ensure_private_dir(&state_dir)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("collie-core")
            .enable_all()
            .build()
            .map_err(|e| CoreError::Internal {
                message: e.to_string(),
            })?;
        let store = MachineStore::new(state_dir.clone());
        let machines = store.load()?;
        Ok(Arc::new(Self {
            runtime,
            inner: Arc::new(Inner {
                state_dir,
                control_url,
                node: NodeSlot::default(),
                identity: IdentitySlot::default(),
                starting: Mutex::default(),
                store,
                machines: Mutex::new(machines),
                conns: Mutex::default(),
                cold_start: Mutex::default(),
                measured: Default::default(),
                login_name: Mutex::default(),
                push: Arc::default(),
                reach: Arc::default(),
                ops: Mutex::default(),
                uploads: Mutex::default(),
                next_upload: AtomicU64::default(),
                suspended: watch::Sender::new(false),
                resumes: AtomicU64::default(),
                busy: watch::Sender::new(0),
                log: Mutex::default(),
            }),
        }))
    }

    /// Exported futures are polled by the foreign executor, outside any tokio context.
    async fn run<T: Send + 'static>(
        &self,
        fut: impl Future<Output = Result<T, CoreError>> + Send + 'static,
    ) -> Result<T, CoreError> {
        self.runtime
            .spawn(fut)
            .await
            .map_err(|e| CoreError::Internal {
                message: e.to_string(),
            })?
    }

    async fn call(
        &self,
        machine_id: &str,
        request: Request,
        timeout: Duration,
    ) -> Result<Response, CoreError> {
        let conn = self.conn(machine_id)?;
        let _busy = Busy::new(&self.inner);
        self.run(async move {
            conn.request(request, timeout)
                .await
                .map_err(|e| request_error(&conn, e))
        })
        .await
    }

    /// For requests carrying an `op_id`: the same request, op_id included, is resent
    /// after a dropped connection. Until collied answers, an identical request (a re-tap
    /// after a local timeout) reuses that op_id, so collied replays the first outcome
    /// instead of running it twice.
    async fn mutate(
        &self,
        machine_id: &str,
        mut request: Request,
        timeout: Duration,
    ) -> Result<Response, CoreError> {
        let conn = self.conn(machine_id)?;
        let _busy = Busy::new(&self.inner);
        let key = self.inner.claim_op(machine_id, &mut request);
        let inner = self.inner.clone();
        self.run(async move {
            let result = conn.mutate(request, timeout).await;
            let answered = match &result {
                Ok(_) => true,
                Err(RequestError::Failed(e)) => !e.is_transport(),
                Err(_) => false,
            };
            if answered && let Some(key) = key {
                lock(&inner.ops).remove(&key);
            }
            result.map_err(|e| request_error(&conn, e))
        })
        .await
    }

    /// Holds the machines lock until the conn is inserted so a concurrent removal
    /// cannot leave a supervisor for a removed machine.
    /// An end the Mac acknowledged is no longer sent with the next connection.
    fn send_if_connected(
        &self,
        machine_id: &str,
        request: Request,
        ends: Option<ActivityId>,
    ) -> Result<(), CoreError> {
        let conn = self.conn(machine_id)?;
        if lock(&conn.shared.link).phase != LinkPhase::Connected {
            return Ok(());
        }
        let push = self.inner.push.clone();
        let machine_id = machine_id.to_owned();
        let sent = conn.request_in_order(request, CALL_TIMEOUT);
        let busy = Busy::new(&self.inner);
        self.runtime.spawn(async move {
            let sent = sent.await;
            drop(busy);
            if let (Ok(_), Some(ended)) = (sent, ends)
                && let Some(reg) = lock(&push).get_mut(&machine_id)
            {
                reg.unsent_ends.retain(|a| *a != ended);
            }
        });
        Ok(())
    }

    fn conn(&self, machine_id: &str) -> Result<Arc<Conn>, CoreError> {
        let machines = lock(&self.inner.machines);
        let machine = machines
            .iter()
            .find(|m| m.id == machine_id)
            .cloned()
            .ok_or(CoreError::MachineNotFound)?;
        let mut conns = lock(&self.inner.conns);
        Ok(conns
            .entry(machine.id.clone())
            .or_insert_with(|| {
                Arc::new(Conn::spawn(
                    self.runtime.handle(),
                    machine,
                    self.inner.node.clone(),
                    self.inner.identity.clone(),
                    self.inner.push.clone(),
                    self.inner.reach.clone(),
                    self.inner.suspended.subscribe(),
                ))
            })
            .clone())
    }
}

impl Inner {
    fn tailnet_configured(&self) -> bool {
        self.state_dir.join("tsnet/tailscaled.state").exists()
    }

    fn claim_op(&self, machine_id: &str, request: &mut Request) -> Option<String> {
        let mut probe = request.clone();
        *op_id_mut(&mut probe)? = OpId::new("A".repeat(22)).expect("valid op_id");
        let key = format!(
            "{machine_id}\n{}",
            serde_json::to_string(&probe).expect("requests serialize")
        );
        let mut ops = lock(&self.ops);
        let now = Instant::now();
        ops.retain(|_, (_, at)| now.duration_since(*at) < OP_REUSE_WINDOW);
        let op_id = op_id_mut(request)?;
        match ops.get(&key) {
            Some((reused, _)) => op_id.clone_from(reused),
            None => {
                ops.insert(key.clone(), (op_id.clone(), now));
            }
        }
        Some(key)
    }

    fn config(&self, auth_key: Option<Zeroizing<String>>) -> Config {
        Config {
            state_dir: self.state_dir.join("tsnet"),
            hostname: HOSTNAME.into(),
            auth_key,
            control_url: self.control_url.clone(),
            advertise_tags: Vec::new(),
            // Silences libtailscale's backend logger only. tsnet's UserLogf is still unset
            // in tailscale-sys, so tsnet prints the login URL to stderr via log.Printf.
            log_to_stderr: false,
        }
    }

    fn node_start(self: &Arc<Self>, key: Option<Zeroizing<String>>) -> Result<(), CoreError> {
        let _starting = lock(&self.starting);
        let current = lock(&self.node).clone();
        if let Some(node) = current {
            if key.is_none() || node.status()?.backend_state == BackendState::Running {
                return Ok(node.start()?);
            }
            drop(node);
            if let Some(old) = lock(&self.node).take() {
                self.release(old)?;
            }
        }
        let t0 = Instant::now();
        let node = Node::new(&self.config(key))?;
        let created = t0.elapsed();
        node.start()?;
        let started = t0.elapsed();
        *lock(&self.node) = Some(Arc::new(node));
        if !self.measured.swap(true, Ordering::SeqCst) {
            let inner = self.clone();
            std::thread::spawn(move || {
                let report = measure(&inner.node, t0, created, started);
                *lock(&inner.cold_start) = Some(report);
            });
        }
        Ok(())
    }

    /// Two tsnet servers must never share the state dir, so the replacement is only
    /// created once every other holder has let go of the old node.
    fn release(&self, old: Arc<Node>) -> Result<(), CoreError> {
        let deadline = Instant::now() + RELEASE_TIMEOUT;
        while Arc::strong_count(&old) > 1 {
            if Instant::now() >= deadline {
                *lock(&self.node) = Some(old);
                return Err(CoreError::Tailnet {
                    message: "the previous Tailscale session is still closing, try again".into(),
                });
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Ok(())
    }

    /// Replaces a node that runs but reaches no machine with a fresh one on the same state,
    /// as a relaunch would, unless another restart already replaced `suspect`. The old node
    /// is closed once nothing holds it (the lock-screen decide holds it until it is
    /// answered). It is kept when a machine connects meanwhile, so no session is cut, or when
    /// the app is suspended, so a lock-screen decide waiting for the node gets it at once.
    /// Requests queued while no machine is connected wait for the new node.
    fn restart_node(&self, suspect: &Weak<Node>) -> Result<&'static str, CoreError> {
        let _starting = lock(&self.starting);
        let Some(old) = lock(&self.node).take_if(|n| Arc::as_ptr(n) == suspect.as_ptr()) else {
            return Ok("already replaced");
        };
        let deadline = Instant::now() + RELEASE_TIMEOUT;
        loop {
            let released = Arc::strong_count(&old) == 1;
            let kept = if self.connected() {
                Ok("kept, a machine connected")
            } else if *self.suspended.borrow() {
                Ok("kept, suspended")
            } else if released {
                break;
            } else if Instant::now() >= deadline {
                Err(CoreError::Tailnet {
                    message: "the Tailscale session is still in use, try again".into(),
                })
            } else {
                std::thread::sleep(POLL_INTERVAL);
                continue;
            };
            *lock(&self.node) = Some(old);
            return kept;
        }
        drop(old);
        let node = Node::new(&self.config(None))?;
        node.start()?;
        *lock(&self.node) = Some(Arc::new(node));
        for conn in lock(&self.conns).values() {
            conn.resume(FOREGROUND_RECONNECT, false);
        }
        Ok("done")
    }

    async fn restart(self: &Arc<Self>, why: &str, suspect: Weak<Node>) {
        let t0 = Instant::now();
        let inner = self.clone();
        let result = blocking(move || inner.restart_node(&suspect)).await;
        let outcome = match result {
            Ok(Ok(outcome)) => outcome.to_owned(),
            Ok(Err(e)) => e.to_string(),
            Err(e) => e.to_string(),
        };
        self.log(format!(
            "node restart ({why}): {outcome} in {} ms",
            ms(t0.elapsed())
        ));
    }

    async fn start_missing_node(self: &Arc<Self>, why: &str) {
        if lock(&self.node).is_some() || !self.tailnet_configured() {
            return;
        }
        let inner = self.clone();
        let outcome = match blocking(move || inner.node_start(None)).await {
            Ok(Ok(())) => "done".to_owned(),
            Ok(Err(e)) => e.to_string(),
            Err(e) => e.to_string(),
        };
        self.log(format!("node start ({why}): {outcome}"));
    }

    fn connected(&self) -> bool {
        lock(&self.conns)
            .values()
            .any(|c| lock(&c.shared.link).phase == LinkPhase::Connected)
    }

    /// When Tailscale says the node runs and a failing machine is online, yet no machine is
    /// connected, the node itself is the likely fault. A node that cannot report its status
    /// is too. Returns that node.
    async fn node_suspect(&self, conns: &[Arc<Conn>]) -> Option<Weak<Node>> {
        let failing: Vec<String> = conns
            .iter()
            .filter(|c| {
                let link = lock(&c.shared.link);
                matches!(link.phase, LinkPhase::Connecting | LinkPhase::Waiting)
                    && link.last_error.is_some()
            })
            .map(|c| c.machine.node_id.clone())
            .collect();
        if self.connected() || failing.is_empty() {
            return None;
        }
        let node = lock(&self.node).clone()?;
        let suspect = Arc::downgrade(&node);
        let suspected =
            match tokio::time::timeout(KICK_TIMEOUT, blocking(move || node.status())).await {
                Ok(Ok(Ok(status))) => {
                    status.backend_state == BackendState::Running
                        && status
                            .peer
                            .iter()
                            .flat_map(|p| p.values())
                            .any(|p| p.online && failing.contains(&p.stable_id))
                }
                _ => true,
            };
        suspected.then_some(suspect)
    }

    async fn resumed(
        self: Arc<Self>,
        background: Duration,
        suspended: bool,
        epoch: u64,
        links: Vec<(String, String)>,
    ) {
        self.start_missing_node("resume").await;
        let mut line = format!(
            "resume after {} s{}:",
            background.as_secs(),
            if suspended { ", suspended" } else { "" }
        );
        let mut peers = None;
        let node = lock(&self.node).clone();
        match node {
            None => line += " no node",
            Some(node) => {
                match tokio::time::timeout(KICK_TIMEOUT, blocking(move || node.status())).await {
                    Ok(Ok(Ok(status))) => {
                        line += &format!(
                            " node {:?}, health {:?}",
                            status.backend_state,
                            status.health.unwrap_or_default()
                        );
                        peers = status.peer;
                    }
                    Ok(Ok(Err(e))) => line += &format!(" node status failed: {e}"),
                    Ok(Err(e)) => line += &format!(" node status failed: {e}"),
                    Err(_) => line += " node did not answer",
                }
            }
        }
        for (node_id, link) in links {
            let online = peers
                .iter()
                .flat_map(|p| p.values())
                .find(|p| p.stable_id == node_id)
                .map(|p| p.online);
            line += &format!("; {link}, online {online:?}");
        }
        self.log(line);
        if !suspended && background < FOREGROUND_RECONNECT {
            return;
        }
        let current = || self.resumes.load(Ordering::SeqCst) == epoch && !*self.suspended.borrow();
        tokio::time::sleep(REBIND_AFTER).await;
        if !current() || lock(&self.conns).is_empty() || self.connected() {
            return;
        }
        let node = lock(&self.node).clone();
        if let Some(node) = node {
            let rebound = tokio::time::timeout(KICK_TIMEOUT, blocking(move || node.rebind())).await;
            let outcome = match rebound {
                Ok(Ok(Ok(()))) => "done".to_owned(),
                Ok(Ok(Err(e))) => e.to_string(),
                Ok(Err(e)) => e.to_string(),
                Err(_) => "no answer".to_owned(),
            };
            self.log(format!(
                "rebind, nothing connected after the resume: {outcome}"
            ));
        }
        tokio::time::sleep(RESTART_AFTER - REBIND_AFTER).await;
        if !current() {
            return;
        }
        let conns: Vec<_> = lock(&self.conns).values().cloned().collect();
        let suspect = self.node_suspect(&conns).await;
        drop(conns);
        if let Some(suspect) = suspect {
            self.restart("resume", suspect).await;
        }
    }

    fn log(&self, message: String) {
        if let Some(log) = &*lock(&self.log) {
            log.log(message);
        }
    }

    fn node_state(&self) -> Result<NodeState, CoreError> {
        let Some(node) = lock(&self.node).clone() else {
            return Ok(NodeState {
                backend_state: TailnetState::NotStarted,
                auth_url: None,
                self_dns_name: None,
                login_name: None,
            });
        };
        let status = node.status()?;
        let running = status.backend_state == BackendState::Running;
        let self_node = status.self_node.as_ref();
        let mut login_name = lock(&self.login_name).clone();
        if running && login_name.is_none() {
            let ip = self_node.and_then(|s| s.tailscale_ips.as_ref()?.first().copied());
            login_name = ip
                .and_then(|ip| node.whois(&ip.to_string()).ok())
                .and_then(|w| w.user_profile)
                .map(|p| p.login_name);
            lock(&self.login_name).clone_from(&login_name);
        }
        Ok(NodeState {
            backend_state: status.backend_state.into(),
            auth_url: Some(status.auth_url).filter(|u| !running && !u.is_empty()),
            self_dns_name: self_node
                .map(|s| s.dns_name.trim_end_matches('.').to_owned())
                .filter(|n| !n.is_empty()),
            login_name,
        })
    }

    async fn pair(&self, invite_uri: &str, device_label: &str) -> Result<Machine, CoreError> {
        let invite = PairingInvite::parse(invite_uri).map_err(|_| CoreError::InvalidInvite)?;
        let device_label = Label::new(device_label.trim()).map_err(|_| CoreError::InvalidLabel)?;
        let node = lock(&self.node).clone().ok_or(CoreError::NotRunning)?;
        let identity = lock(&self.identity).clone();
        let (mut session, _, kind) = conn::open(
            node,
            &invite.host,
            invite.port,
            &invite.node_id,
            None,
            invite.key.as_str(),
            identity,
            true,
        )
        .await?;
        let info = expect_paired(
            session
                .call(
                    Request::PairComplete(PairCompleteParams {
                        pairing_code: invite.code,
                        device_label,
                    }),
                    PAIR_CONFIRM_TIMEOUT,
                )
                .await?,
        )?;
        session.close().await;
        let machine = Machine {
            id: random_id(),
            label: info.name,
            host: invite.host,
            port: invite.port,
            node_id: invite.node_id,
            kind,
            key: invite.key.as_str().to_owned(),
        };
        let node_id = machine.node_id.clone();
        self.update_machines(|m| m.node_id == node_id, Some(machine.clone()))?;
        Ok(machine)
    }

    fn update_machines(
        &self,
        remove: impl Fn(&Machine) -> bool,
        add: Option<Machine>,
    ) -> Result<(), CoreError> {
        let mut machines = lock(&self.machines);
        let (gone, mut kept): (Vec<Machine>, Vec<Machine>) =
            machines.iter().cloned().partition(|m| remove(m));
        if gone.is_empty() && add.is_none() {
            return Ok(());
        }
        kept.extend(add);
        self.store.save(&kept)?;
        *machines = kept;
        let mut conns = lock(&self.conns);
        let mut push = lock(&self.push);
        for m in gone {
            conns.remove(&m.id);
            push.remove(&m.id);
        }
        Ok(())
    }
}

/// Polls until a login URL is issued or the node is Running, so the report covers
/// the control-plane round trip and not just the local backend start. The slot is
/// re-read on every poll rather than holding a clone, so a node replaced by an
/// auth-key start is closed at once and never shares its state dir with the new one.
fn measure(slot: &NodeSlot, t0: Instant, created: Duration, started: Duration) -> ColdStartReport {
    let mut polls = 0;
    let mut last = None;
    let settled = loop {
        let status = lock(slot).clone().map(|n| n.status());
        if let Some(Ok(status)) = status {
            polls += 1;
            let done = status.backend_state == BackendState::Running || !status.auth_url.is_empty();
            last = Some((status.backend_state, !status.auth_url.is_empty()));
            if done {
                break Some(t0.elapsed());
            }
        }
        if t0.elapsed() >= SETTLE_TIMEOUT {
            break None;
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    let (state, auth_url_present) = last.unwrap_or((BackendState::NoState, false));
    ColdStartReport {
        node_created_ms: ms(created),
        started_ms: ms(started),
        settled_ms: settled.map(ms),
        backend_state: state.into(),
        auth_url_present,
        status_polls: polls,
        build: build_info(),
    }
}

fn request_error(conn: &Conn, e: RequestError) -> CoreError {
    let link = lock(&conn.shared.link);
    let last = link.last_error.clone();
    match e {
        RequestError::Failed(e) => e.into(),
        RequestError::Stopped if link.phase == LinkPhase::Stopped => CoreError::Unauthorized {
            message: last.unwrap_or_default(),
        },
        _ if link.phase == LinkPhase::Offline => CoreError::NotRunning,
        _ => CoreError::Unreachable {
            message: last.unwrap_or_else(|| "timed out waiting for the machine".into()),
        },
    }
}

fn view(conn: &Conn) -> MachineFlock {
    let (link, last_error) = {
        let link = lock(&conn.shared.link);
        (link.phase, link.last_error.clone())
    };
    let state = lock(&conn.shared.flock);
    let flock = state.flock.as_ref();
    MachineFlock {
        machine: conn.machine.clone(),
        link,
        last_error,
        details: flock.map(|f| MachineDetails {
            name: f.machine.name.clone(),
            node_id: f.machine.node_id.clone(),
            herdr_session: f.machine.herdr_session.clone(),
        }),
        workspaces: flock
            .into_iter()
            .flat_map(|f| &f.workspaces)
            .map(|w| WorkspaceSummary {
                workspace_id: w.workspace_id.as_str().into(),
                label: w.label.clone(),
                number: w.number,
                status: w.status.into(),
                cwd: w.cwd.clone(),
            })
            .collect(),
        agents: flock
            .into_iter()
            .flat_map(|f| &f.agents)
            .map(agent_summary)
            .collect(),
        approvals_count: flock.map_or(0, |f| f.approvals.len() as u32),
    }
}

fn agent_summary(a: &protocol::Agent) -> AgentSummary {
    AgentSummary {
        terminal_id: a.terminal_id.as_str().into(),
        workspace_id: a.workspace_id.as_str().into(),
        kind: a.kind.clone(),
        name: a.name.clone(),
        title: a.title.clone(),
        status: a.status.into(),
        status_since_ms: a.status_since_ms,
        cwd: a.cwd.clone(),
        last_line: a.last_line.clone(),
    }
}

fn op_id_mut(request: &mut Request) -> Option<&mut OpId> {
    match request {
        Request::AgentPrompt(p) => Some(&mut p.op_id),
        Request::AgentSendKeys(p) => Some(&mut p.op_id),
        Request::AgentTypeText(p) => Some(&mut p.op_id),
        Request::TaskNew(p) => Some(&mut p.op_id),
        _ => None,
    }
}

/// collied's `invalid_params` messages come from the protocol decoder: `invalid <Type>`
/// from a validated id, or serde's "missing field `x`" / "unknown field `x`".
fn invalid_field(message: &str) -> Option<String> {
    if let Some(ty) = message.strip_prefix("invalid ") {
        let field = match ty {
            "TerminalId" => "terminal_id",
            "WorkspaceId" => "workspace_id",
            "ApprovalId" => "approval_id",
            "AgentKind" => "agent",
            "Cwd" => "cwd",
            "PromptText" => "prompt",
            "Label" => "label",
            "Nonce" => "nonce",
            "OpId" => "op_id",
            "PushToken" => "apns_token",
            "NotificationKey" => "notification_key",
            "AttachmentName" => "name",
            "UploadId" => "upload_id",
            "Sha256Hex" => "sha256",
            "ChunkData" => "data",
            _ => return None,
        };
        return Some(field.into());
    }
    let (_, rest) = message.split_once("field `")?;
    let (field, _) = rest.split_once('`')?;
    Some(field.into())
}

fn new_op_id() -> OpId {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("system RNG");
    OpId::new(URL_SAFE_NO_PAD.encode(bytes)).expect("16 bytes encode to 22 base64url chars")
}

fn invalid(field: &str, message: &str) -> CoreError {
    CoreError::InvalidInput {
        field: Some(field.into()),
        message: message.into(),
    }
}

fn activity(id: String) -> Result<ActivityId, CoreError> {
    ActivityId::new(id).map_err(|_| invalid("activity_id", "invalid activity id"))
}

fn terminal(id: String) -> Result<TerminalId, CoreError> {
    TerminalId::new(id).map_err(|_| invalid("terminal_id", "invalid agent id"))
}

fn prompt_text(text: String) -> Result<PromptText, CoreError> {
    PromptText::new(text).map_err(|_| {
        invalid(
            "prompt",
            "prompt must be non-empty, at most 32 KiB, without control characters other than newline and tab",
        )
    })
}

fn expect_ok(response: Response) -> Result<(), CoreError> {
    match response {
        Response::Ok => Ok(()),
        other => Err(unexpected(&other).into()),
    }
}

fn ensure_private_dir(dir: &Path) -> Result<(), CoreError> {
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir()
        || meta.permissions().mode() & 0o077 != 0
        || meta.uid() != unsafe { libc::getuid() }
    {
        return Err(CoreError::InsecureStateDir);
    }
    Ok(())
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn server(code: ErrorCode, message: &str) -> CoreError {
        SessionError::Server {
            code,
            message: message.into(),
        }
        .into()
    }

    #[test]
    fn server_errors_map_to_readable_variants() {
        let e = server(ErrorCode::AgentBlocked, "agent is blocked");
        assert!(matches!(e, CoreError::AgentBlocked));
        assert_eq!(e.to_string(), "the agent is waiting for an approval");
        assert!(matches!(
            server(ErrorCode::AgentNotReady, ""),
            CoreError::AgentNotReady
        ));
        assert!(matches!(
            server(ErrorCode::ConfirmRequired, ""),
            CoreError::ConfirmRequired
        ));
        assert!(matches!(
            server(ErrorCode::NotImplemented, ""),
            CoreError::NotImplemented
        ));
        assert!(matches!(
            server(ErrorCode::RateLimited, ""),
            CoreError::RateLimited
        ));
        assert!(matches!(
            server(ErrorCode::NotFound, ""),
            CoreError::NotFound
        ));
        let e = server(ErrorCode::InvalidParams, "invalid Cwd");
        assert!(
            matches!(&e, CoreError::InvalidInput { field: Some(f), message } if f == "cwd" && message == "invalid Cwd")
        );
        assert!(matches!(
            server(ErrorCode::NotPaired, ""),
            CoreError::Unauthorized { .. }
        ));
        assert!(matches!(
            server(ErrorCode::Unrecognized, "quota"),
            CoreError::Rejected { .. }
        ));
        assert!(matches!(
            server(ErrorCode::UnknownMethod, "unknown method"),
            CoreError::NotImplemented
        ));
        let e = server(ErrorCode::TooLarge, "attachment storage on the Mac is full");
        assert!(matches!(&e, CoreError::TooLarge { .. }));
        assert_eq!(e.to_string(), "attachment storage on the Mac is full");
        assert!(matches!(
            server(ErrorCode::ChecksumMismatch, ""),
            CoreError::ChecksumMismatch
        ));
        assert!(matches!(
            CoreError::from(SessionError::Closed),
            CoreError::Unreachable { .. }
        ));
    }

    #[test]
    fn draft_errors_map_to_variants() {
        let changed = |draft: Option<&str>| {
            CoreError::from(SessionError::from(protocol::ErrorBody {
                code: ErrorCode::DraftChanged,
                message: "the Mac's input box has unsent text".into(),
                draft: draft.map(Into::into),
            }))
        };
        let e = changed(Some("typed on the Mac"));
        assert!(
            matches!(&e, CoreError::DraftChanged { current } if current == "typed on the Mac"),
            "{e:?}"
        );
        assert_eq!(e.to_string(), "The agent's input box has unsent text.");
        assert!(matches!(changed(None), CoreError::Rejected { .. }));
        let e = server(ErrorCode::DraftNotCleared, "m");
        assert!(matches!(e, CoreError::DraftNotCleared));
        assert_eq!(
            e.to_string(),
            "Could not clear the agent's input box; nothing was sent."
        );
    }

    #[test]
    fn invalid_params_carry_the_field() {
        let field = |m: &str| match server(ErrorCode::InvalidParams, m) {
            CoreError::InvalidInput { field, .. } => field,
            other => panic!("{other:?}"),
        };
        assert_eq!(field("invalid Cwd"), Some("cwd".into()));
        assert_eq!(field("invalid TerminalId"), Some("terminal_id".into()));
        assert_eq!(field("missing field `nonce`"), Some("nonce".into()));
        assert_eq!(
            field("unknown field `pane_id`, expected `terminal_id`"),
            Some("pane_id".into())
        );
        assert_eq!(field("params out of range"), None);
        assert_eq!(field("invalid Sheep"), None);
    }

    #[test]
    fn approval_errors_map_to_variants() {
        assert!(matches!(
            server(ErrorCode::ApprovalExpired, ""),
            CoreError::ApprovalExpired
        ));
        assert!(matches!(
            server(ErrorCode::ApprovalAlreadyResolved, ""),
            CoreError::ApprovalAlreadyResolved
        ));
        assert!(matches!(
            server(ErrorCode::ApprovalNotFound, ""),
            CoreError::ApprovalNotFound
        ));
        assert!(matches!(
            server(ErrorCode::ApprovalNonceMismatch, ""),
            CoreError::Rejected { .. }
        ));
    }

    #[test]
    fn unanswered_mutation_op_id_is_reused_by_an_identical_retry() {
        let dir = tempfile::tempdir().unwrap();
        let core = CollieCore::new(dir.path().join("s").to_string_lossy().into()).unwrap();
        let prompt = |text: &str| {
            Request::AgentPrompt(AgentPromptParams {
                op_id: new_op_id(),
                terminal_id: TerminalId::new("term_1").unwrap(),
                text: PromptText::new(text).unwrap(),
                expected_draft: None,
            })
        };
        let op = |r: &mut Request| op_id_mut(r).unwrap().clone();
        let mut first = prompt("run the tests");
        let sent = op(&mut first);
        let key = core.inner.claim_op("m1", &mut first).unwrap();
        assert_eq!(op(&mut first), sent);

        let mut retap = prompt("run the tests");
        assert_eq!(core.inner.claim_op("m1", &mut retap), Some(key.clone()));
        assert_eq!(op(&mut retap), sent, "same request reuses the op_id");

        let mut other = prompt("something else");
        let fresh = op(&mut other);
        core.inner.claim_op("m1", &mut other);
        assert_eq!(op(&mut other), fresh);
        let mut elsewhere = prompt("run the tests");
        let fresh = op(&mut elsewhere);
        core.inner.claim_op("m2", &mut elsewhere);
        assert_eq!(op(&mut elsewhere), fresh, "keyed per machine");

        lock(&core.inner.ops).remove(&key);
        let mut after = prompt("run the tests");
        let fresh = op(&mut after);
        core.inner.claim_op("m1", &mut after);
        assert_eq!(op(&mut after), fresh, "answered: a new tap is a new action");

        lock(&core.inner.ops).get_mut(&key).unwrap().1 -= OP_REUSE_WINDOW;
        let mut late = prompt("run the tests");
        let fresh = op(&mut late);
        core.inner.claim_op("m1", &mut late);
        assert_eq!(op(&mut late), fresh, "window expired");

        let mut focus = Request::AgentFocus(AgentTarget {
            terminal_id: TerminalId::new("term_1").unwrap(),
        });
        assert_eq!(core.inner.claim_op("m1", &mut focus), None);
    }

    #[test]
    fn push_registration_is_per_machine_and_kept_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("s");
        ensure_private_dir(&state).unwrap();
        let mac = |id: &str| Machine {
            id: id.into(),
            label: "mac".into(),
            host: format!("{id}.tail1234.ts.net"),
            port: 8457,
            node_id: format!("n{id}"),
            kind: MachineKind::Mac,
            key: String::new(),
        };
        MachineStore::new(state.clone())
            .save(&[mac("m1"), mac("m2")])
            .unwrap();
        let core = CollieCore::new(state.to_string_lossy().into()).unwrap();
        let key: Vec<u8> = (1..=32).collect();
        let token = "ab".repeat(32);
        assert!(matches!(
            core.register_push("m1".into(), "not hex".into(), PushEnvironment::Sandbox, key.clone()),
            Err(CoreError::InvalidInput { field: Some(f), .. }) if f == "apns_token"
        ));
        assert!(matches!(
            core.register_push("m1".into(), token.clone(), PushEnvironment::Sandbox, vec![1; 31]),
            Err(CoreError::InvalidInput { field: Some(f), .. }) if f == "notification_key"
        ));
        assert!(matches!(
            core.register_push(
                "nope".into(),
                token.clone(),
                PushEnvironment::Sandbox,
                key.clone()
            ),
            Err(CoreError::MachineNotFound)
        ));
        core.register_push(
            "m1".into(),
            format!(" {token} "),
            PushEnvironment::Production,
            key.clone(),
        )
        .unwrap();
        core.register_push(
            "m2".into(),
            token.clone(),
            PushEnvironment::Production,
            vec![9; 32],
        )
        .unwrap();
        let push: BTreeMap<String, PushRegisterParams> = lock(&core.inner.push)
            .iter()
            .map(|(m, r)| (m.clone(), r.push.clone().unwrap()))
            .collect();
        assert_eq!(push.keys().collect::<Vec<_>>(), ["m1", "m2"]);
        assert_eq!(push["m1"].apns_token.as_str(), token);
        assert_eq!(
            push["m1"].environment,
            protocol::ApnsEnvironment::Production
        );
        assert_eq!(
            URL_SAFE_NO_PAD
                .decode(push["m1"].notification_key.as_str())
                .unwrap(),
            key
        );
        assert_eq!(
            URL_SAFE_NO_PAD
                .decode(push["m2"].notification_key.as_str())
                .unwrap(),
            [9; 32]
        );
        assert!(
            !core
                .runtime
                .block_on(core.remove_machine("m2".into()))
                .unwrap(),
            "an unreachable machine is forgotten without an unpair"
        );
        assert_eq!(
            lock(&core.inner.push).keys().collect::<Vec<_>>(),
            ["m1"],
            "unpairing drops the machine's registration"
        );
        let encoded = push["m1"].notification_key.as_str().to_owned();
        drop(core);
        for entry in std::fs::read_dir(&state).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
                assert!(!text.contains(&encoded), "{}", path.display());
            }
        }
        let core = CollieCore::new(state.to_string_lossy().into()).unwrap();
        assert!(lock(&core.inner.push).is_empty());
    }

    #[test]
    fn activity_tokens_are_validated_and_kept_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("s");
        ensure_private_dir(&state).unwrap();
        MachineStore::new(state.clone())
            .save(&[Machine {
                id: "m1".into(),
                label: "mac".into(),
                host: "m1.tail1234.ts.net".into(),
                port: 8457,
                node_id: "nm1".into(),
                kind: MachineKind::Mac,
                key: String::new(),
            }])
            .unwrap();
        let core = CollieCore::new(state.to_string_lossy().into()).unwrap();
        let token = "cd".repeat(80);
        let act = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";
        let field = |r: Result<(), CoreError>| match r {
            Err(CoreError::InvalidInput { field, .. }) => field,
            other => panic!("{other:?}"),
        };
        let register = |m: &str, a: &str, t: &str, tok: &str| {
            core.register_activity_token(m.into(), a.into(), t.into(), tok.into())
        };
        assert_eq!(
            field(register("m1", "a/b", "term_1", &token)).as_deref(),
            Some("activity_id")
        );
        assert_eq!(
            field(register("m1", act, "term 1", &token)).as_deref(),
            Some("terminal_id")
        );
        assert_eq!(
            field(register("m1", act, "term_1", "abc")).as_deref(),
            Some("token")
        );
        assert!(matches!(
            register("nope", act, "term_1", &token),
            Err(CoreError::MachineNotFound)
        ));
        assert_eq!(
            field(core.end_activity("m1".into(), "".into())).as_deref(),
            Some("activity_id")
        );
        assert!(matches!(
            core.end_activity("nope".into(), act.into()),
            Err(CoreError::MachineNotFound)
        ));

        register("m1", act, "term_1", &format!(" {token}\n")).unwrap();
        register("m1", "B", "term_2", &token).unwrap();
        core.register_push(
            "m1".into(),
            "ab".repeat(32),
            PushEnvironment::Sandbox,
            vec![3; 32],
        )
        .unwrap();
        let methods = |reg: &mut conn::Registrations| -> Vec<String> {
            reg.take_requests()
                .iter()
                .map(|r| match r {
                    Request::PushActivityToken(p) => format!("token {}", p.activity_id.as_str()),
                    Request::PushActivityEnd(p) => format!("end {}", p.activity_id.as_str()),
                    other => other.method().to_owned(),
                })
                .collect()
        };
        let mut reg = lock(&core.inner.push)["m1"].clone();
        assert_eq!(
            methods(&mut reg),
            [
                "push.register",
                "token 3F2504E0-4F89-11D3-9A0C-0305E82C3301",
                "token B"
            ]
        );
        assert_eq!(
            reg.activities[act].token.as_str(),
            token,
            "trimmed, and kept for the next session"
        );

        core.end_activity("m1".into(), act.into()).unwrap();
        let mut reg = lock(&core.inner.push)["m1"].clone();
        assert_eq!(
            methods(&mut reg),
            [
                "push.register",
                "end 3F2504E0-4F89-11D3-9A0C-0305E82C3301",
                "token B"
            ]
        );
        assert_eq!(
            methods(&mut reg),
            ["push.register", "token B"],
            "ends go once"
        );
        register("m1", act, "term_1", &token).unwrap();
        assert!(lock(&core.inner.push)["m1"].unsent_ends.is_empty());
        for i in 0..40 {
            core.end_activity("m1".into(), format!("E{i}")).unwrap();
        }
        assert_eq!(lock(&core.inner.push)["m1"].unsent_ends.len(), 16);

        drop(core);
        for entry in std::fs::read_dir(&state).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
                assert!(!text.contains(&token), "{}", path.display());
            }
        }
    }

    #[test]
    fn op_ids_are_fresh_and_valid() {
        let (a, b) = (new_op_id(), new_op_id());
        assert_eq!(a.as_str().len(), 22);
        assert_ne!(a, b);
    }

    #[test]
    fn inputs_are_validated_before_anything_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let core = CollieCore::new(dir.path().join("s").to_string_lossy().into()).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let field = |r: Result<(), CoreError>| match r {
            Err(CoreError::InvalidInput { field, .. }) => field,
            other => panic!("{other:?}"),
        };
        let m = || "unknown".to_owned();
        let t = || "term_1".to_owned();
        assert_eq!(
            field(rt.block_on(core.prompt(m(), t(), "x\u{1b}[201~rm -rf ~\r".into(), None))),
            Some("prompt".into())
        );
        assert_eq!(
            field(rt.block_on(core.prompt(m(), "a b".into(), "hi".into(), None))),
            Some("terminal_id".into())
        );
        assert_eq!(
            field(rt.block_on(core.prompt(m(), t(), "hi".into(), Some("a\u{1b}[2J".into())))),
            Some("expected_draft".into())
        );
        assert_eq!(
            field(rt.block_on(core.send_keys(m(), t(), Vec::new()))),
            Some("keys".into())
        );
        assert_eq!(
            field(rt.block_on(core.send_keys(m(), t(), vec![AgentKey::Y; 17]))),
            Some("keys".into())
        );
        for bad in ["", "one\ntwo", "a\tb", "x\u{1b}[2J"] {
            assert_eq!(
                field(rt.block_on(core.type_text(m(), t(), bad.into()))),
                Some("text".into()),
                "{bad:?}"
            );
        }
        let task = |cwd: &str, agent: &str, label: Option<&str>| {
            rt.block_on(core.task_new(
                m(),
                cwd.into(),
                agent.into(),
                "go".into(),
                label.map(Into::into),
            ))
            .map(|_| ())
        };
        assert_eq!(field(task("src", "claude", None)), Some("cwd".into()));
        assert_eq!(field(task("/src", "Claude", None)), Some("agent".into()));
        assert_eq!(
            field(task("/src", "claude", Some("a\u{202E}b"))),
            Some("label".into())
        );
        assert!(matches!(
            task("/src", "claude", Some("  ")),
            Err(CoreError::MachineNotFound)
        ));
        assert!(matches!(
            rt.block_on(core.prompt(m(), t(), "fix it\nthen test".into(), None)),
            Err(CoreError::MachineNotFound)
        ));
    }
}

#[cfg(test)]
mod tailnet_tests {
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::OnceLock;

    use collie_tls::rustls::sign::{CertifiedKey, SigningKey};

    use futures_util::{SinkExt, StreamExt};
    use protocol::{
        Agent, AgentStatus, DEFAULT_PORT, Flock, HelloResult, MachineInfo, PROTOCOL_VERSION,
        PairingCode, ServerFrame, WS_SUBPROTOCOL, parse_client_frame,
    };
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::handshake::server::{
        Request as HsRequest, Response as HsResponse,
    };
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    use super::*;
    use crate::session::Session;

    const KNOBS: [(&str, &str); 1] = [("TS_DISABLE_PORTMAPPER", "1")];
    const CODE: &str = "Zm9vYmFyYmF6cXV4cXV1dQ";

    struct TestControl(Child, String);

    impl TestControl {
        fn start(auth_key: &str, dir: &Path, args: &[&str]) -> Self {
            let bin = dir.join("testcontrol");
            let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tailnet/testcontrol");
            let status = Command::new(std::env::var("GO").unwrap_or_else(|_| "go".into()))
                .current_dir(src)
                .env("GOTOOLCHAIN", "local")
                .env("GOFLAGS", "-mod=readonly")
                .args(["build", "-o"])
                .arg(&bin)
                .arg(".")
                .status()
                .expect("go build testcontrol");
            assert!(status.success());
            let mut child = Command::new(&bin)
                .args(["-authkey", auth_key])
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut url = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut url)
                .unwrap();
            assert!(url.starts_with("http://127.0.0.1:"), "{url:?}");
            Self(child, url.trim().into())
        }
    }

    impl Drop for TestControl {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn flock() -> Flock {
        Flock {
            seq: 1,
            machine: info(),
            workspaces: Vec::new(),
            agents: vec![Agent {
                terminal_id: TerminalId::new("term_1").unwrap(),
                workspace_id: WorkspaceId::new("w1").unwrap(),
                kind: Some("claude".into()),
                name: None,
                title: Some("fix the build".into()),
                status: AgentStatus::Blocked,
                status_since_ms: 1,
                cwd: None,
                last_line: None,
            }],
            approvals: Vec::new(),
        }
    }

    fn info() -> MachineInfo {
        MachineInfo {
            name: "it-mac".into(),
            node_id: String::new(),
            herdr_session: "test".into(),
        }
    }

    /// What the fake collied saw, shared across connections. `approvals` are pending
    /// on the fake and announced with `approval.needed` after every snapshot.
    #[derive(Default)]
    struct Seen {
        watches: Vec<Option<String>>,
        reads: Vec<Option<u16>>,
        lines: Vec<Option<u16>>,
        prompt_ops: Vec<String>,
        executed: HashMap<String, Response>,
        task_ops: Vec<String>,
        approvals: Vec<protocol::Approval>,
        lists: usize,
        decisions: Vec<(String, protocol::Decision)>,
        notes: Vec<Option<String>>,
        pushes: Vec<String>,
        activities: Vec<String>,
        unpaired: bool,
        connections: usize,
        closed: usize,
    }

    const NONCE: &str = "Tm9uY2VOb25jZU5vbmNlTm9uY2VOb25jZU5vbmNlTm9";
    const ACTIVITY: &str = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn approval(id: &str) -> protocol::Approval {
        protocol::Approval {
            approval_id: ApprovalId::new(id).unwrap(),
            terminal_id: TerminalId::new("term_1").unwrap(),
            agent_label: "claude".into(),
            workspace_label: "collie".into(),
            snippet: "Run cargo test?".into(),
            tool: None,
            options: vec![protocol::Decision::Approve, protocol::Decision::Deny],
            choices: Vec::new(),
            accepts_input: false,
            has_text_field: false,
            supports_note: true,
            nonce: protocol::Nonce::new(NONCE).unwrap(),
            created_at_ms: 1,
            expires_at_ms: u64::MAX,
        }
    }

    fn question(id: &str) -> protocol::Approval {
        protocol::Approval {
            options: Vec::new(),
            choices: ["SQLite", "Redis", "Type something."]
                .iter()
                .enumerate()
                .map(|(i, l)| protocol::ApprovalChoice {
                    index: i as u8,
                    label: (*l).into(),
                    current: i == 0,
                    detail: (*l == "Redis").then(|| "Shared across processes".into()),
                })
                .collect(),
            accepts_input: true,
            has_text_field: true,
            ..approval(id)
        }
    }

    fn terminal_read(ansi: &str) -> TerminalRead {
        TerminalRead {
            terminal_id: TerminalId::new("term_1").unwrap(),
            source: ReadSource::Recent,
            ansi: ansi.into(),
            truncated: false,
        }
    }

    struct Soft(Arc<dyn SigningKey>);

    impl IdentitySigner for Soft {
        fn sign(&self, message: Vec<u8>) -> Option<Vec<u8>> {
            self.0
                .choose_scheme(&[collie_tls::SCHEME])?
                .sign(&message)
                .ok()
        }
    }

    fn phone_key() -> &'static [u8] {
        static KEY: OnceLock<Vec<u8>> = OnceLock::new();
        KEY.get_or_init(|| collie_tls::generate().unwrap())
    }

    fn set_identity(core: &CollieCore) {
        let key = collie_tls::load(phone_key()).unwrap();
        let spki = key.public_key().unwrap().as_ref().to_vec();
        core.set_identity(spki, Box::new(Soft(key))).unwrap();
    }

    fn mac_key() -> Arc<CertifiedKey> {
        static KEY: OnceLock<Arc<CertifiedKey>> = OnceLock::new();
        KEY.get_or_init(|| {
            collie_tls::certified(collie_tls::load(&collie_tls::generate().unwrap()).unwrap())
                .unwrap()
        })
        .clone()
    }

    fn mac_pin() -> protocol::KeyPin {
        collie_tls::pin(mac_key().cert[0].as_ref())
    }

    /// Stand-in for collied: whois-checks the peer, then answers hello, pair.complete,
    /// flock.snapshot and the Phase 2 methods. A watch starts with the full output, as
    /// collied's first watch tick does. The first prompt of an op_id is
    /// "executed" and its connection dropped before the reply, like a phone losing its
    /// socket mid-call; a resend gets the stored outcome, as collied's op_id cache does.
    /// The handshake callback's error type is fixed by tungstenite.
    #[allow(clippy::result_large_err)]
    async fn serve(mac: Node, phone_id: String, mac_id: String, seen: Arc<Mutex<Seen>>) {
        let listener = mac.listen("tcp", &format!(":{DEFAULT_PORT}")).unwrap();
        loop {
            let accepted = listener.accept().await.unwrap();
            let who = mac.whois(&accepted.peer.to_string()).unwrap();
            assert_eq!(who.node.stable_id, phone_id);
            let mac_id = mac_id.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                let Ok((stream, _)) = collie_tls::accept(accepted.stream, mac_key(), None).await
                else {
                    return;
                };
                let mut ws = tokio_tungstenite::accept_hdr_async(
                    stream,
                    |_: &HsRequest, mut resp: HsResponse| {
                        resp.headers_mut().insert(
                            "Sec-WebSocket-Protocol",
                            HeaderValue::from_static(WS_SUBPROTOCOL),
                        );
                        Ok(resp)
                    },
                )
                .await
                .unwrap();
                lock(&seen).connections += 1;
                let mut seq = flock().seq;
                let mut announced = std::collections::HashSet::new();
                while let Some(Ok(Message::Text(text))) = ws.next().await {
                    let frame = parse_client_frame(text.as_bytes()).unwrap();
                    let machine = MachineInfo {
                        node_id: mac_id.clone(),
                        ..info()
                    };
                    let mut events = Vec::new();
                    let result = match frame.request {
                        Request::Hello(_) => Ok(Response::Hello(HelloResult {
                            protocol_version: PROTOCOL_VERSION,
                            collied_version: "test".into(),
                            machine,
                            herdr_version: None,
                            paired: true,
                        })),
                        Request::PairComplete(p) => {
                            assert_eq!(p.pairing_code.as_str(), CODE);
                            Ok(Response::Paired { machine })
                        }
                        Request::FlockSnapshot(_) => {
                            // Once per connection, as collied sends approval.needed once: a
                            // repeat on every snapshot reorders and re-counts them in the feed.
                            events = lock(&seen)
                                .approvals
                                .iter()
                                .filter(|a| announced.insert(a.approval_id.clone()))
                                .zip(seq + 1..)
                                .map(|(a, s)| {
                                    (
                                        s,
                                        protocol::Event::ApprovalNeeded {
                                            approval: a.clone(),
                                        },
                                    )
                                })
                                .collect();
                            Ok(Response::Flock(Flock { machine, ..flock() }))
                        }
                        Request::AgentWatch(p) => {
                            if p.terminal_id.is_some() {
                                events = vec![
                                    (seq + 1, protocol::Event::AgentOutput(terminal_read("live"))),
                                    (
                                        seq + 1,
                                        protocol::Event::AgentOutput(terminal_read("replayed")),
                                    ),
                                ];
                            }
                            lock(&seen).lines.push(p.lines);
                            lock(&seen).watches.push(p.terminal_id.map(String::from));
                            Ok(Response::Ok)
                        }
                        Request::AgentRead(p) => {
                            assert_eq!(p.terminal_id.as_str(), "term_1");
                            lock(&seen).lines.push(p.lines);
                            lock(&seen).reads.push(p.lines);
                            Ok(Response::Terminal(terminal_read("read")))
                        }
                        Request::AgentPrompt(p) => {
                            let op = p.op_id.as_str().to_owned();
                            let mut seen = lock(&seen);
                            seen.prompt_ops.push(op.clone());
                            match seen.executed.get(&op) {
                                Some(stored) => Ok(stored.clone()),
                                None => {
                                    seen.executed.insert(op, Response::Ok);
                                    return;
                                }
                            }
                        }
                        Request::AgentSendKeys(p) => {
                            assert_eq!(p.keys, vec![protocol::Key::ShiftTab, protocol::Key::Y]);
                            Err(ErrorCode::AgentBlocked)
                        }
                        Request::AgentTypeText(p) => {
                            assert_eq!(p.text.as_str(), "DuckDB");
                            Ok(Response::Ok)
                        }
                        Request::TaskOptions(_) => {
                            Ok(Response::TaskOptions(protocol::TaskOptions {
                                agents: vec![AgentKind::new("claude").unwrap()],
                                default_agent: AgentKind::new("claude").unwrap(),
                                recent_cwds: vec![Cwd::new("/src/collie").unwrap()],
                            }))
                        }
                        Request::TaskNew(p) => {
                            assert_eq!(
                                (p.cwd.as_str(), p.agent.as_str(), p.prompt.as_str()),
                                ("/src/collie", "claude", "add tests")
                            );
                            assert_eq!(p.label.unwrap().as_str(), "tests");
                            lock(&seen).task_ops.push(p.op_id.as_str().into());
                            Ok(Response::TaskStarted {
                                workspace_id: WorkspaceId::new("w2").unwrap(),
                                terminal_id: TerminalId::new("term_2").unwrap(),
                            })
                        }
                        Request::PaneClose(p) if !p.confirm => Err(ErrorCode::ConfirmRequired),
                        Request::ApprovalList(_) => {
                            let mut seen = lock(&seen);
                            seen.lists += 1;
                            Ok(Response::Approvals {
                                approvals: seen.approvals.clone(),
                            })
                        }
                        Request::ApprovalDecide(p) => {
                            let mut seen = lock(&seen);
                            match seen
                                .approvals
                                .iter()
                                .position(|a| a.approval_id == p.approval_id)
                            {
                                Some(i) if seen.approvals[i].nonce == p.nonce => {
                                    seen.approvals.remove(i);
                                    seen.decisions
                                        .push((p.approval_id.as_str().into(), p.decision));
                                    seen.notes.push(p.note.map(|n| n.as_str().to_owned()));
                                    let outcome = match p.choice {
                                        Some(choice) => protocol::ApprovalOutcome::Chosen {
                                            choice,
                                            by: "phone".into(),
                                        },
                                        None => protocol::ApprovalOutcome::Applied {
                                            decision: p.decision,
                                            by: "phone".into(),
                                        },
                                    };
                                    events = vec![(
                                        seq + 1,
                                        protocol::Event::ApprovalResolved {
                                            approval_id: p.approval_id.clone(),
                                            outcome: outcome.clone(),
                                        },
                                    )];
                                    Ok(Response::ApprovalResolved {
                                        approval_id: p.approval_id,
                                        outcome,
                                    })
                                }
                                Some(_) => Err(ErrorCode::ApprovalNonceMismatch),
                                None => Err(ErrorCode::ApprovalNotFound),
                            }
                        }
                        Request::PushRegister(p) => {
                            let mut seen = lock(&seen);
                            seen.pushes.push(p.apns_token.as_str().into());
                            seen.activities.push("register".into());
                            Ok(Response::Ok)
                        }
                        Request::PushActivityToken(p) => {
                            lock(&seen).activities.push(format!(
                                "token {} {} {} {}",
                                p.activity_id.as_str(),
                                p.terminal_id.as_str(),
                                p.token.as_str(),
                                p.shows_approvals
                            ));
                            Ok(Response::Ok)
                        }
                        Request::PushActivityEnd(p) => {
                            lock(&seen)
                                .activities
                                .push(format!("end {}", p.activity_id.as_str()));
                            Ok(Response::Ok)
                        }
                        Request::Unpair(_) => {
                            lock(&seen).unpaired = true;
                            Ok(Response::Ok)
                        }
                        other => panic!("unexpected {}", other.method()),
                    };
                    let reply = match result {
                        Ok(result) => ServerFrame::Result {
                            id: frame.id,
                            result,
                        },
                        Err(code) => ServerFrame::Error {
                            id: Some(frame.id),
                            error: protocol::ErrorBody {
                                code,
                                message: "fake".into(),
                                draft: None,
                            },
                        },
                    };
                    let text = serde_json::to_string(&reply).unwrap();
                    ws.send(Message::text(text)).await.unwrap();
                    for (s, event) in events {
                        seq = s;
                        let event = ServerFrame::Event { seq, event };
                        let text = serde_json::to_string(&event).unwrap();
                        ws.send(Message::text(text)).await.unwrap();
                    }
                }
                lock(&seen).closed += 1;
            });
        }
    }

    fn wait_running(node: &Node, peers: usize) -> tailnet::Status {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let st = node.status().unwrap();
            if st.backend_state == BackendState::Running
                && st.peer.as_ref().is_some_and(|p| {
                    p.len() >= peers && p.values().all(|p| p.tailscale_ips.is_some())
                })
            {
                return st;
            }
            assert!(Instant::now() < deadline, "node not ready: {st:#?}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// libtailscale reads TS_* knobs once at load, so a tailnet test runs itself again
    /// in a fresh process with them set. Returns true in the parent, after the child.
    fn ran_in_child(test: &str, env: &[(&str, &str)]) -> bool {
        if KNOBS
            .iter()
            .chain(env)
            .all(|(k, v)| std::env::var(k).as_deref() == Ok(*v))
        {
            let test = test.to_owned();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(180));
                eprintln!("watchdog: {test} still running");
                std::process::exit(101);
            });
            return false;
        }
        let out = Command::new(std::env::current_exe().unwrap())
            .args([test, "--exact", "--nocapture"])
            .envs(KNOBS)
            .envs(env.iter().copied())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        print!("{stdout}");
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "child run failed: {}", out.status);
        assert!(stdout.contains("1 passed"));
        true
    }

    fn poll<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
        (0..400)
            .find_map(|_| {
                let found = f();
                if found.is_none() {
                    std::thread::sleep(Duration::from_millis(50));
                }
                found
            })
            .unwrap_or_else(|| panic!("timed out waiting for {what}"))
    }

    fn reachability(dir: &Path, node_id: &str) -> reach::Seen {
        let map: HashMap<String, reach::Seen> =
            serde_json::from_slice(&std::fs::read(dir.join(reach::FILE)).unwrap()).unwrap();
        map[node_id]
    }

    #[test]
    fn tailnet_end_to_end() {
        if ran_in_child("tailnet_tests::tailnet_end_to_end", &[]) {
            return;
        }

        let key = format!("test-authkey-collie-core{}", std::process::id());
        let root = tempfile::tempdir().unwrap();
        let control = TestControl::start(&key, root.path(), &[]);
        let phone_dir = root.path().join("phone");
        std::fs::create_dir(&phone_dir).unwrap();
        std::fs::set_permissions(&phone_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        let core = CollieCore::with_control_url(phone_dir.clone(), control.1.clone()).unwrap();
        set_identity(&core);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(
            rt.block_on(core.node_state()).unwrap().backend_state,
            TailnetState::NotStarted
        );
        assert!(!core.tailnet_configured());
        rt.block_on(core.node_start(Some(format!(" {key}\n"))))
            .unwrap();

        let mac = Node::new(&Config {
            state_dir: root.path().join("mac"),
            hostname: "it-mac".into(),
            auth_key: Some(Zeroizing::new(key.clone())),
            control_url: Some(control.1.clone()),
            advertise_tags: vec![pin::MAC_TAG.into()],
            log_to_stderr: false,
        })
        .unwrap();
        mac.start().unwrap();
        let mac_st = wait_running(&mac, 1);
        let phone_node = lock(&core.inner.node).clone().unwrap();
        let phone_st = wait_running(&phone_node, 1);
        let mac_self = mac_st.self_node.clone().unwrap();
        let phone_id = phone_st.self_node.as_ref().unwrap().stable_id.clone();

        let state = rt.block_on(core.node_state()).unwrap();
        assert_eq!(state.backend_state, TailnetState::Running);
        assert!(state.auth_url.is_none());
        assert!(state.self_dns_name.is_some());
        assert!(state.login_name.is_some());
        assert!(core.tailnet_configured());
        let report = (0..100)
            .find_map(|_| {
                std::thread::sleep(Duration::from_millis(50));
                core.cold_start_report()
            })
            .unwrap();
        assert_eq!(report.backend_state, TailnetState::Running);
        assert!(report.settled_ms.is_some());

        let seen = Arc::new(Mutex::new(Seen::default()));
        core.runtime.spawn(serve(
            mac.clone(),
            phone_id,
            mac_self.stable_id.clone(),
            seen.clone(),
        ));

        let host = mac_self.dns_name.trim_end_matches('.').to_owned();
        let mac_ip = mac_self.tailscale_ips.clone().unwrap()[0];
        let session = rt.block_on(core.runtime.spawn({
            let (node, host) = (phone_node.clone(), host.clone());
            let identity = lock(&core.inner.identity).clone().unwrap();
            async move {
                let stream = conn::dial(&node, std::net::SocketAddr::new(mac_ip, DEFAULT_PORT))
                    .await
                    .unwrap();
                let stream = conn::tls(stream, &host, mac_pin(), identity).await.unwrap();
                let mut session = Session::connect(stream, &host, DEFAULT_PORT).await.unwrap();
                session.hello().await.unwrap()
            }
        }));
        assert!(session.unwrap().paired, "transport over the tailnet");

        let invite = |node_id: &str| {
            PairingInvite {
                host: host.clone(),
                port: DEFAULT_PORT,
                node_id: node_id.into(),
                key: mac_pin(),
                code: PairingCode::new(CODE).unwrap(),
            }
            .to_uri()
        };
        let err = rt
            .block_on(core.pair(invite("nSOMEONEELSE"), "iPhone".into()))
            .unwrap_err();
        assert!(matches!(err, CoreError::PinViolation { .. }), "{err:?}");
        let mut wrong_key = PairingInvite::parse(&invite(&mac_self.stable_id)).unwrap();
        wrong_key.key = collie_tls::pin(b"another key");
        let err = rt
            .block_on(core.pair(wrong_key.to_uri(), "iPhone".into()))
            .unwrap_err();
        assert!(
            matches!(&err, CoreError::PinViolation { message } if message.contains("key")),
            "{err:?}"
        );

        assert!(
            mac_self.tags.iter().flatten().any(|t| t == pin::MAC_TAG),
            "testcontrol must tag the Mac: {:?}",
            mac_self.tags
        );
        let machine = rt
            .block_on(core.pair(invite(&mac_self.stable_id), "iPhone".into()))
            .unwrap();
        assert_eq!(core.machines(), vec![machine.clone()]);
        let first = rt.block_on(core.flock(machine.id.clone())).unwrap();
        assert_ne!(first.link, LinkPhase::Stopped, "{:?}", first.last_error);
        let flock = (0..200)
            .find_map(|_| {
                let f = rt.block_on(core.flock(machine.id.clone())).unwrap();
                if f.link == LinkPhase::Connected && f.details.is_some() {
                    return Some(f);
                }
                std::thread::sleep(Duration::from_millis(50));
                None
            })
            .expect("connected flock");
        assert_eq!(flock.agents[0].status, AgentState::Blocked);
        assert_eq!(
            Arc::strong_count(&phone_node),
            2,
            "a live session must not hold the node"
        );

        let id = || machine.id.clone();
        let t1 = || "term_1".to_owned();
        let poll = |after: u64, want: &str| {
            (0..200)
                .find_map(|_| {
                    let view = core.agent_view(id(), t1(), after).unwrap();
                    if view.output.as_ref().is_some_and(|o| o.ansi == want) {
                        return Some(view);
                    }
                    std::thread::sleep(Duration::from_millis(25));
                    None
                })
                .unwrap_or_else(|| panic!("no output {want:?}"))
        };
        rt.block_on(core.watch_agent(id(), Some(t1()), 500))
            .unwrap();
        let view = poll(0, "live");
        assert_eq!(view.agent.unwrap().status, AgentState::Blocked);
        assert_eq!(view.output_revision, 1, "the replay is dropped");
        assert!(core.agent_view(id(), t1(), 1).unwrap().output.is_none());
        assert!(
            lock(&seen).reads.is_empty(),
            "the watch's output is not read again"
        );
        let snap = rt
            .block_on(core.agent_read(id(), t1(), TerminalSource::Recent, Some(500)))
            .unwrap();
        assert_eq!((snap.ansi.as_str(), snap.truncated), ("read", false));
        let read = poll(view.output_revision, "read").output_revision;
        let snap = rt
            .block_on(core.agent_read(id(), t1(), TerminalSource::Recent, Some(60)))
            .unwrap();
        assert_eq!(snap.ansi, "read");
        assert!(
            core.agent_view(id(), t1(), read).unwrap().output.is_none(),
            "a preview read does not replace the watched screen"
        );
        assert_eq!(lock(&seen).reads, vec![Some(500), Some(60)]);

        rt.block_on(core.prompt(id(), t1(), "fix the build".into(), None))
            .unwrap();
        {
            let seen = lock(&seen);
            assert_eq!(seen.prompt_ops.len(), 2, "sent, dropped, resent");
            assert_eq!(seen.prompt_ops[0], seen.prompt_ops[1]);
            assert_eq!(seen.executed.len(), 1);
            assert_eq!(
                seen.watches,
                vec![Some(t1()), Some(t1())],
                "the watch is re-issued on the new connection"
            );
            assert_eq!(
                seen.reads,
                vec![Some(500), Some(60)],
                "only the explicit agent_read calls"
            );
            assert_eq!(
                seen.lines,
                [Some(500), Some(500), Some(60), Some(500)],
                "watch, read, preview read, re-issued watch"
            );
        }
        poll(view.output_revision, "live");
        rt.block_on(core.prompt(id(), t1(), "again".into(), None))
            .unwrap();
        {
            let seen = lock(&seen);
            assert_eq!(seen.executed.len(), 2);
            assert_ne!(
                seen.prompt_ops[0], seen.prompt_ops[2],
                "fresh op_id per action"
            );
        }

        let err = rt
            .block_on(core.send_keys(id(), t1(), vec![AgentKey::ShiftTab, AgentKey::Y]))
            .unwrap_err();
        assert!(matches!(err, CoreError::AgentBlocked), "{err:?}");
        rt.block_on(core.type_text(id(), t1(), "DuckDB".into()))
            .unwrap();
        let options = rt.block_on(core.task_options(id())).unwrap();
        assert_eq!(options.default_agent, "claude");
        assert_eq!(options.recent_cwds, vec!["/src/collie".to_owned()]);
        let started = rt
            .block_on(core.task_new(
                id(),
                "/src/collie".into(),
                "claude".into(),
                "add tests".into(),
                Some(" tests ".into()),
            ))
            .unwrap();
        assert_eq!(
            (started.workspace_id.as_str(), started.terminal_id.as_str()),
            ("w2", "term_2")
        );
        assert_eq!(lock(&seen).task_ops.len(), 1);
        let err = rt
            .block_on(core.close_pane(id(), "term_2".into(), false))
            .unwrap_err();
        assert!(matches!(err, CoreError::ConfirmRequired), "{err:?}");
        rt.block_on(core.watch_agent(id(), None, 500)).unwrap();
        assert_eq!(lock(&seen).watches.last(), Some(&None));
        assert!(core.agent_view(id(), t1(), 0).unwrap().output.is_none());

        let again = rt
            .block_on(core.pair(invite(&mac_self.stable_id), "iPhone".into()))
            .unwrap();
        assert_ne!(again.id, machine.id);
        assert_eq!(core.machines(), vec![again.clone()]);
        assert!(core.cached_flock(machine.id).is_none());
        (0..200)
            .find(|_| {
                let f = rt.block_on(core.flock(again.id.clone())).unwrap();
                std::thread::sleep(Duration::from_millis(50));
                f.link == LinkPhase::Connected
            })
            .expect("connected again");
        assert!(rt.block_on(core.remove_machine(again.id)).unwrap());
        assert!(lock(&seen).unpaired);
        assert!(core.machines().is_empty());
        drop(control);
    }

    #[test]
    fn approvals_end_to_end() {
        // testcontrol demands the auth key on every registration, even a known node
        // key re-registering from cached state, which real control accepts. tsnet falls
        // back to TS_AUTHKEY, so the cold start below passes no key itself.
        const KEY: &str = "test-authkey-collie-approvals";
        if ran_in_child(
            "tailnet_tests::approvals_end_to_end",
            &[("TS_AUTHKEY", KEY)],
        ) {
            return;
        }
        let key = KEY.to_owned();
        let root = tempfile::tempdir().unwrap();
        let control = TestControl::start(&key, root.path(), &[]);
        let phone_dir = root.path().join("phone");
        let group = root.path().join("group");
        std::fs::create_dir(&group).unwrap();
        std::fs::create_dir(&phone_dir).unwrap();
        std::fs::set_permissions(&phone_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        let core = CollieCore::with_control_url(phone_dir.clone(), control.1.clone()).unwrap();
        set_identity(&core);
        core.set_app_group_dir(group.to_string_lossy().into())
            .unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(core.node_start(Some(key.clone()))).unwrap();
        let mac = Node::new(&Config {
            state_dir: root.path().join("mac"),
            hostname: "it-mac".into(),
            auth_key: Some(Zeroizing::new(key.clone())),
            control_url: Some(control.1.clone()),
            advertise_tags: vec![pin::MAC_TAG.into()],
            log_to_stderr: false,
        })
        .unwrap();
        mac.start().unwrap();
        let mac_self = wait_running(&mac, 1).self_node.unwrap();
        let phone_node = lock(&core.inner.node).clone().unwrap();
        let phone_id = wait_running(&phone_node, 1).self_node.unwrap().stable_id;
        drop(phone_node);
        let mac_id = mac_self.stable_id.clone();

        // The fake collied outlives the phone's runtime, which is dropped for the cold start.
        let server_rt = tokio::runtime::Runtime::new().unwrap();
        let seen = Arc::new(Mutex::new(Seen {
            approvals: vec![approval("a1"), question("q1")],
            ..Seen::default()
        }));
        server_rt.spawn(serve(mac.clone(), phone_id, mac_id.clone(), seen.clone()));
        let invite = PairingInvite {
            host: mac_self.dns_name.trim_end_matches('.').to_owned(),
            port: DEFAULT_PORT,
            node_id: mac_id.clone(),
            key: mac_pin(),
            code: PairingCode::new(CODE).unwrap(),
        };
        let machine = rt
            .block_on(core.pair(invite.to_uri(), "iPhone".into()))
            .unwrap();
        let id = || machine.id.clone();
        poll("connected", || {
            let f = rt.block_on(core.flock(id())).unwrap();
            (f.link == LinkPhase::Connected && f.details.is_some()).then_some(())
        });
        assert!(reachability(&group, &mac_id).last_ok_ms.is_some());

        let feed = poll("approval.needed", || {
            core.approval_feed(id(), 0)
                .filter(|f| f.events.len() >= 2 && f.pending.len() == 2)
        });
        let pending = PendingApproval::from(&approval("a1"));
        let asked = PendingApproval::from(&question("q1"));
        assert_eq!(
            feed.events[0],
            ApprovalEvent::Needed {
                approval: pending.clone()
            }
        );
        assert_eq!(feed.pending, vec![pending.clone(), asked.clone()]);
        assert!(asked.options.is_empty());
        assert!(asked.accepts_input && asked.has_text_field);
        assert!(!pending.accepts_input && !pending.has_text_field && pending.supports_note);
        assert_eq!(
            asked.choices[1],
            ApprovalChoice {
                index: 1,
                label: "Redis".into(),
                current: false,
                detail: Some("Shared across processes".into()),
            }
        );
        let listed = rt.block_on(core.approvals(id())).unwrap();
        assert_eq!(listed, vec![pending, asked]);
        for shown in [format!("{feed:?}"), format!("{listed:?}")] {
            assert!(
                !shown.contains(NONCE) && !shown.contains("Nonce"),
                "{shown}"
            );
        }
        assert_eq!(lock(&seen).lists, 1);
        for (decision, note) in [
            (ApprovalDecision::ApproveAlways, "x".to_owned()),
            (ApprovalDecision::Approve, "a\nb".to_owned()),
            (ApprovalDecision::Approve, "a\u{1b}[Z".to_owned()),
            (
                ApprovalDecision::Deny,
                "a".repeat(limits::MAX_NOTE_CHARS + 1),
            ),
        ] {
            let err = rt
                .block_on(core.decide(id(), "a1".into(), decision, Some(note)))
                .unwrap_err();
            assert!(matches!(err, CoreError::InvalidInput { .. }), "{err:?}");
        }
        let outcome = rt
            .block_on(core.decide(
                id(),
                "a1".into(),
                ApprovalDecision::Approve,
                Some("use a .tmp extension".into()),
            ))
            .unwrap();
        assert_eq!(
            outcome,
            DecisionOutcome::Applied {
                decision: ApprovalDecision::Approve,
                by: "phone".into()
            }
        );
        assert_eq!(
            lock(&seen).lists,
            1,
            "decided with the nonce held since approval.needed"
        );
        assert_eq!(
            lock(&seen).decisions,
            vec![("a1".to_owned(), protocol::Decision::Approve)]
        );
        assert_eq!(lock(&seen).notes, [Some("use a .tmp extension".to_owned())]);
        let resolved = poll("approval.resolved", || {
            core.approval_feed(id(), feed.revision)
                .filter(|f| !f.events.is_empty())
        });
        assert!(matches!(
            &resolved.events[0],
            ApprovalEvent::Resolved { approval_id, .. } if approval_id == "a1"
        ));
        assert_eq!(resolved.pending.len(), 1);
        let err = rt
            .block_on(core.decide(id(), "a1".into(), ApprovalDecision::Approve, None))
            .unwrap_err();
        assert!(matches!(err, CoreError::ApprovalNotFound), "{err:?}");
        let chosen = rt
            .block_on(core.decide(
                id(),
                "q1".into(),
                ApprovalDecision::Choose { choice: 1 },
                None,
            ))
            .unwrap();
        assert_eq!(
            chosen,
            DecisionOutcome::Applied {
                decision: ApprovalDecision::Choose { choice: 1 },
                by: "phone".into()
            }
        );
        assert_eq!(
            lock(&seen).decisions[1],
            ("q1".to_owned(), protocol::Decision::Choose)
        );

        core.register_push(id(), TOKEN.into(), PushEnvironment::Sandbox, vec![7; 32])
            .unwrap();
        poll("push.register", || {
            (lock(&seen).pushes.len() == 1).then_some(())
        });
        let token = "ab".repeat(80);
        core.register_activity_token(id(), ACTIVITY.into(), "term_1".into(), token.clone())
            .unwrap();
        let registered = format!("token {ACTIVITY} term_1 {token} true");
        poll("push.activity_token", || {
            (lock(&seen).activities.last() == Some(&registered)).then_some(())
        });
        core.resume(60);
        poll("push.register after reconnect", || {
            (lock(&seen).pushes.len() == 2).then_some(())
        });
        poll("activity token after reconnect", || {
            (lock(&seen).activities.len() == 4).then_some(())
        });
        assert_eq!(
            lock(&seen).activities,
            ["register", &registered, "register", &registered],
            "push.register first: collied takes the environment from it"
        );
        core.end_activity(id(), ACTIVITY.into()).unwrap();
        let ended = format!("end {ACTIVITY}");
        poll("push.activity_end", || {
            (lock(&seen).activities.last() == Some(&ended)).then_some(())
        });
        poll("acknowledged end", || {
            lock(&core.inner.push)[&id()]
                .unsent_ends
                .is_empty()
                .then_some(())
        });
        core.resume(60);
        poll("push.register after the end", || {
            (lock(&seen).pushes.len() == 3).then_some(())
        });
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            lock(&seen).activities[5..],
            ["register"],
            "an acknowledged end and an ended activity are not sent again"
        );

        // Cold start: a new process on the same state dir, node not started, no conns.
        lock(&seen).approvals.push(approval("a2"));
        let node = Arc::downgrade(&lock(&core.inner.node).clone().unwrap());
        drop(core);
        assert!(node.upgrade().is_none(), "the old node is closed");
        let core = CollieCore::with_control_url(phone_dir.clone(), control.1.clone()).unwrap();
        set_identity(&core);
        core.set_app_group_dir(group.to_string_lossy().into())
            .unwrap();
        assert!(lock(&core.inner.node).is_none());
        let report = rt.block_on(core.decide_from_notification(
            mac_id.clone(),
            "a2".into(),
            ApprovalDecision::Deny,
            Some(15_000),
        ));
        println!("cold background decide: {report:?}");
        assert_eq!(
            report.outcome,
            BackgroundOutcome::Applied {
                decision: ApprovalDecision::Deny
            }
        );
        assert!(!report.node_was_running);
        assert!(report.total_ms <= 15_000);
        let steps = [
            report.node_up_ms,
            report.connect_ms,
            report.lookup_ms,
            report.decide_ms,
        ];
        assert!(steps.iter().all(Option::is_some), "{report:?}");
        assert_eq!(lock(&seen).lists, 3, "the cold path fetched approval.list");
        assert_eq!(
            lock(&seen).decisions.last(),
            Some(&("a2".to_owned(), protocol::Decision::Deny))
        );

        let again = rt.block_on(core.decide_from_notification(
            mac_id.clone(),
            "a2".into(),
            ApprovalDecision::Deny,
            Some(u64::MAX),
        ));
        assert_eq!(again.outcome, BackgroundOutcome::NotFound);
        assert!(again.node_was_running);
        let unknown = rt.block_on(core.decide_from_notification(
            "nNOPE".into(),
            "a2".into(),
            ApprovalDecision::Approve,
            None,
        ));
        assert_eq!(unknown.outcome, BackgroundOutcome::UnknownMachine);
        let zero = rt.block_on(core.decide_from_notification(
            mac_id.clone(),
            "a2".into(),
            ApprovalDecision::Approve,
            Some(0),
        ));
        assert!(
            matches!(zero.outcome, BackgroundOutcome::Failed { .. }),
            "{zero:?}"
        );
        assert_eq!(reachability(&group, &mac_id).last_fail_ms, None);
        let late = rt.block_on(core.decide_from_notification(
            mac_id.clone(),
            "a2".into(),
            ApprovalDecision::Approve,
            Some(1),
        ));
        assert!(
            matches!(late.outcome, BackgroundOutcome::Unreachable { .. }),
            "{late:?}"
        );
        assert!(reachability(&group, &mac_id).last_fail_ms.is_some());

        poll("connected after cold start", || {
            let f = rt.block_on(core.flock(id())).unwrap();
            (f.link == LinkPhase::Connected).then_some(())
        });
        core.register_push(id(), TOKEN.into(), PushEnvironment::Sandbox, vec![7; 32])
            .unwrap();
        poll("token registered again by the new process", || {
            (lock(&seen).pushes.len() == 4).then_some(())
        });
        assert!(lock(&seen).pushes.iter().all(|t| t == TOKEN));

        let link = || core.cached_flock(id()).unwrap().link;
        let (connections, closed) = {
            let seen = lock(&seen);
            (seen.connections, seen.closed)
        };
        core.register_activity_token(id(), ACTIVITY.into(), "term_1".into(), token.clone())
            .unwrap();
        rt.block_on(core.suspend(core.begin_suspend()));
        assert_eq!(
            lock(&seen).activities.last(),
            Some(&registered),
            "a token sent as the app leaves still reaches the Mac"
        );
        let flock = core.cached_flock(id()).unwrap();
        assert_eq!(
            (flock.link, flock.last_error),
            (LinkPhase::Connecting, None),
            "a suspend is not a failure"
        );
        poll("collied sees the session close", || {
            (lock(&seen).closed == closed + 1).then_some(())
        });
        lock(&seen).approvals.push(approval("a3"));
        let report = rt.block_on(core.decide_from_notification(
            mac_id.clone(),
            "a3".into(),
            ApprovalDecision::Approve,
            None,
        ));
        assert_eq!(
            report.outcome,
            BackgroundOutcome::Applied {
                decision: ApprovalDecision::Approve
            },
            "the lock-screen decide works while suspended"
        );
        // A wake permit cuts any backoff short: the supervisor must still stay parked.
        lock(&core.inner.conns)[&id()].reconnect_now();
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(
            lock(&seen).connections,
            connections + 1,
            "only the decide's own connection while suspended"
        );
        assert_ne!(link(), LinkPhase::Connected);
        core.resume(1);
        poll("reconnected after a short switch", || {
            (link() == LinkPhase::Connected).then_some(())
        });
        assert_eq!(lock(&seen).connections, connections + 2);

        let closed = lock(&seen).closed;
        let epoch = core.begin_suspend();
        core.resume(1);
        rt.block_on(core.suspend(epoch));
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(
            link(),
            LinkPhase::Connected,
            "a resume cancels a late suspend"
        );
        let after = {
            let seen = lock(&seen);
            (seen.connections, seen.closed)
        };
        assert_eq!(after, (connections + 2, closed), "the session stays open");
        drop(core);
        drop(server_rt);
        drop(control);
    }

    #[test]
    fn recovery_end_to_end() {
        // The node restart registers the node key again: see approvals_end_to_end.
        const KEY: &str = "test-authkey-collie-recovery";
        if ran_in_child("tailnet_tests::recovery_end_to_end", &[("TS_AUTHKEY", KEY)]) {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let control = TestControl::start(KEY, root.path(), &["-offline"]);
        let phone_dir = root.path().join("phone");
        std::fs::create_dir(&phone_dir).unwrap();
        std::fs::set_permissions(&phone_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let core = CollieCore::with_control_url(phone_dir, control.1.clone()).unwrap();
        set_identity(&core);
        struct Log(Arc<Mutex<Vec<String>>>);
        impl CoreLog for Log {
            fn log(&self, message: String) {
                lock(&self.0).push(message);
            }
        }
        let logged = Arc::new(Mutex::new(Vec::new()));
        core.set_log(Box::new(Log(logged.clone())));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(core.node_start(Some(KEY.into()))).unwrap();
        let mac = Node::new(&Config {
            state_dir: root.path().join("mac"),
            hostname: "it-mac".into(),
            auth_key: Some(Zeroizing::new(KEY.into())),
            control_url: Some(control.1.clone()),
            advertise_tags: vec![pin::MAC_TAG.into()],
            log_to_stderr: false,
        })
        .unwrap();
        mac.start().unwrap();
        let mac_self = wait_running(&mac, 1).self_node.unwrap();
        let phone_node = lock(&core.inner.node).clone().unwrap();
        let phone_st = wait_running(&phone_node, 1);
        drop(phone_node);
        assert!(
            phone_st
                .peer
                .iter()
                .flat_map(|p| p.values())
                .all(|p| !p.online),
            "the phone's netmap reports the Mac offline"
        );
        let phone_id = phone_st.self_node.unwrap().stable_id;
        let server_rt = tokio::runtime::Runtime::new().unwrap();
        let seen = Arc::new(Mutex::new(Seen::default()));
        server_rt.spawn(serve(
            mac.clone(),
            phone_id,
            mac_self.stable_id.clone(),
            seen.clone(),
        ));
        let invite = PairingInvite {
            host: mac_self.dns_name.trim_end_matches('.').to_owned(),
            port: DEFAULT_PORT,
            node_id: mac_self.stable_id.clone(),
            key: mac_pin(),
            code: PairingCode::new(CODE).unwrap(),
        };
        let machine = rt
            .block_on(core.pair(invite.to_uri(), "iPhone".into()))
            .unwrap();
        let id = || machine.id.clone();
        let link = || core.cached_flock(id()).unwrap().link;
        let connections = || lock(&seen).connections;
        assert_eq!(connections(), 1, "the pairing");

        rt.block_on(core.flock(id())).unwrap();
        poll("unavailable", || {
            (link() == LinkPhase::Unavailable).then_some(())
        });
        std::thread::sleep(Duration::from_secs(4));
        assert_eq!(connections(), 1, "a machine reported offline is not dialed");
        core.reconnect(id()).unwrap();
        poll("connected by a manual reconnect", || {
            (link() == LinkPhase::Connected).then_some(())
        });
        assert_eq!(connections(), 2);
        core.reconnect(id()).unwrap();
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(connections(), 2, "a live session is only probed");

        let node = lock(&core.inner.node).clone().unwrap();
        node.rebind().unwrap();
        drop(node);
        rt.block_on(core.agent_read(id(), "term_1".into(), TerminalSource::Recent, None))
            .unwrap();
        assert_eq!(connections(), 2, "a rebind keeps the session");

        core.resume(60);
        poll("reconnected after a resume", || {
            (connections() == 3 && link() == LinkPhase::Connected).then_some(())
        });
        std::thread::sleep(Duration::from_secs(4));
        assert_eq!(connections(), 3, "one dial past the offline flag");
        assert!(
            lock(&logged)[0]
                .starts_with("resume after 60 s: node Running, health []; it-mac: Connected"),
            "{logged:?}"
        );
        assert!(
            lock(&logged)[0].ends_with("online Some(false)"),
            "{logged:?}"
        );
        assert_eq!(lock(&logged).len(), 1, "connected again: no rebind");

        let identity = lock(&core.inner.identity).take();
        core.resume(60);
        poll("rebind with nothing connected", || {
            lock(&logged)
                .iter()
                .any(|l| l == "rebind, nothing connected after the resume: done")
                .then_some(())
        });
        let held = lock(&core.inner.node).clone().unwrap();
        let old = Arc::downgrade(&held);
        let busy = Busy::new(&core.inner);
        let restart = std::thread::spawn({
            let inner = core.inner.clone();
            let old = old.clone();
            move || {
                let t0 = Instant::now();
                inner
                    .restart_node(&old)
                    .map(|outcome| (outcome, t0.elapsed()))
            }
        });
        std::thread::sleep(Duration::from_secs(1));
        assert!(lock(&core.inner.node).is_none());
        assert!(
            !restart.is_finished(),
            "a holder (the lock-screen decide) keeps the old node"
        );
        drop(held);
        let (outcome, took) = restart.join().unwrap().unwrap();
        println!("node restart took {took:?}");
        assert_eq!(
            outcome, "done",
            "a request queued with nothing connected does not hold it up"
        );
        assert!(took < RELEASE_TIMEOUT);
        assert!(old.upgrade().is_none(), "the old node is closed");
        assert_eq!(
            core.inner.restart_node(&old).unwrap(),
            "already replaced",
            "a second restart leaves the new node alone"
        );
        drop(busy);
        *lock(&core.inner.identity) = identity;
        core.reconnect(id()).unwrap();
        poll("connected on the new node", || {
            (connections() == 4 && link() == LinkPhase::Connected).then_some(())
        });

        let node = lock(&core.inner.node).clone().unwrap();
        let current = Arc::downgrade(&node);
        drop(node);
        assert_eq!(
            core.inner.restart_node(&current).unwrap(),
            "kept, a machine connected"
        );
        rt.block_on(core.suspend(core.begin_suspend()));
        assert_eq!(
            core.inner.restart_node(&current).unwrap(),
            "kept, suspended"
        );
        assert!(
            lock(&core.inner.node)
                .as_ref()
                .is_some_and(|n| Arc::as_ptr(n) == current.as_ptr())
        );
        core.resume(1);
        poll("connected after the suspend", || {
            (connections() == 5 && link() == LinkPhase::Connected).then_some(())
        });

        let gone = Arc::downgrade(&lock(&core.inner.node).take().unwrap());
        poll("the node closed", || gone.upgrade().is_none().then_some(()));
        core.reconnect(id()).unwrap();
        poll("a reconnect starts a missing node", || {
            (connections() == 6 && link() == LinkPhase::Connected).then_some(())
        });
        assert!(
            lock(&logged)
                .iter()
                .any(|l| l == "node start (reconnect): done"),
            "{logged:?}"
        );

        let identity = lock(&core.inner.identity).take();
        core.resume(60);
        poll("the session ended", || {
            (link() != LinkPhase::Connected).then_some(())
        });
        std::thread::sleep(Duration::from_secs(1));
        *lock(&core.inner.identity) = identity;
        poll("a failure before the dial keeps the forced dial", || {
            (connections() == 7 && link() == LinkPhase::Connected).then_some(())
        });
        drop(core);
        drop(server_rt);
        drop(control);
    }
}
