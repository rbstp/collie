mod conn;
mod pin;
mod session;
mod store;

use std::collections::HashMap;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use protocol::{Empty, Label, PairCompleteParams, PairingInvite, Request};
use tailnet::{BackendState, Config, Node};
use zeroize::Zeroizing;

use conn::{Conn, ConnectError, LinkPhase, NodeSlot, RequestError, blocking};
use session::{CALL_TIMEOUT, SessionError, expect_flock, expect_paired, lock};
pub use store::Machine;
use store::{MachineStore, random_id};

uniffi::setup_scaffolding!();

const HOSTNAME: &str = "collie-phone";
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(15);
/// Longer than the dial budget, the longest any caller holds a node.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(30);
/// collied holds `pair.complete` until the user confirms on the Mac (up to 65 s).
const PAIR_CONFIRM_TIMEOUT: Duration = Duration::from_secs(75);

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
    #[error("the Mac rejected the request: {message}")]
    Rejected { message: String },
    #[error("stopped retrying: {message}. Pair this Mac again.")]
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
            SessionError::Server { .. } => Self::Rejected { message },
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

#[derive(uniffi::Object)]
pub struct CollieCore {
    runtime: tokio::runtime::Runtime,
    inner: Arc<Inner>,
}

struct Inner {
    state_dir: PathBuf,
    control_url: Option<String>,
    node: NodeSlot,
    starting: Mutex<()>,
    store: MachineStore,
    machines: Mutex<Vec<Machine>>,
    conns: Mutex<HashMap<String, Arc<Conn>>>,
    cold_start: Mutex<Option<ColdStartReport>>,
    measured: AtomicBool,
    login_name: Mutex<Option<String>>,
}

#[uniffi::export]
impl CollieCore {
    #[uniffi::constructor]
    pub fn new(state_dir: String) -> Result<Arc<Self>, CoreError> {
        Self::build(PathBuf::from(state_dir), None)
    }

    pub fn tailnet_configured(&self) -> bool {
        self.inner.state_dir.join("tsnet/tailscaled.state").exists()
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

    pub fn remove_machine(&self, id: String) -> Result<(), CoreError> {
        self.inner.update_machines(|m| m.id == id, None)
    }

    /// Answers from the cache while the link is down so a refresh never waits out
    /// CALL_TIMEOUT on an unreachable Mac.
    pub async fn flock(&self, machine_id: String) -> Result<MachineFlock, CoreError> {
        let conn = self.conn(&machine_id)?;
        if lock(&conn.shared.link).phase != LinkPhase::Connected {
            return Ok(view(&conn));
        }
        self.run(async move {
            let response = conn
                .request(Request::FlockSnapshot(Empty {}), CALL_TIMEOUT)
                .await
                .map_err(|e| request_error(&conn, e))?;
            expect_flock(response).map_err(CoreError::from)?;
            Ok(view(&conn))
        })
        .await
    }

    pub fn cached_flock(&self, machine_id: String) -> Option<MachineFlock> {
        lock(&self.inner.conns).get(&machine_id).map(|c| view(c))
    }

    pub fn resume(&self, background_secs: u64) {
        for conn in lock(&self.inner.conns).values() {
            conn.resume(Duration::from_secs(background_secs));
        }
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
                starting: Mutex::default(),
                store,
                machines: Mutex::new(machines),
                conns: Mutex::default(),
                cold_start: Mutex::default(),
                measured: Default::default(),
                login_name: Mutex::default(),
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

    /// Holds the machines lock until the conn is inserted so a concurrent removal
    /// cannot leave a supervisor for a removed machine.
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
                ))
            })
            .clone())
    }
}

impl Inner {
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
        let node = Node::new(&Config {
            state_dir: self.state_dir.join("tsnet"),
            hostname: HOSTNAME.into(),
            auth_key: key,
            control_url: self.control_url.clone(),
            advertise_tags: Vec::new(),
            // Silences libtailscale's backend logger only. tsnet's UserLogf is still unset
            // in tailscale-sys, so tsnet prints the login URL to stderr via log.Printf.
            log_to_stderr: false,
        })?;
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
        let (mut session, _) = conn::open(node, &invite.host, invite.port, &invite.node_id).await?;
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
        for m in gone {
            conns.remove(&m.id);
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
            message: last.unwrap_or_else(|| "timed out waiting for the Mac".into()),
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
            .map(|a| AgentSummary {
                terminal_id: a.terminal_id.as_str().into(),
                workspace_id: a.workspace_id.as_str().into(),
                kind: a.kind.clone(),
                name: a.name.clone(),
                title: a.title.clone(),
                status: a.status.into(),
                status_since_ms: a.status_since_ms,
                cwd: a.cwd.clone(),
                last_line: a.last_line.clone(),
            })
            .collect(),
        approvals_count: flock.map_or(0, |f| f.approvals.len() as u32),
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
mod tailnet_tests {
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};

    use futures_util::{SinkExt, StreamExt};
    use protocol::{
        Agent, AgentStatus, DEFAULT_PORT, Flock, HelloResult, MachineInfo, PROTOCOL_VERSION,
        PairingCode, Response, ServerFrame, TerminalId, WS_SUBPROTOCOL, WorkspaceId,
        parse_client_frame,
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
        fn start(auth_key: &str, dir: &Path) -> Self {
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

    /// Stand-in for collied: whois-checks the peer, then answers hello, pair.complete
    /// and flock.snapshot. The handshake callback's error type is fixed by tungstenite.
    #[allow(clippy::result_large_err)]
    async fn serve(mac: Node, phone_id: String, mac_id: String) {
        let listener = mac.listen("tcp", &format!(":{DEFAULT_PORT}")).unwrap();
        loop {
            let accepted = listener.accept().await.unwrap();
            let who = mac.whois(&accepted.peer.to_string()).unwrap();
            assert_eq!(who.node.stable_id, phone_id);
            let mac_id = mac_id.clone();
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_hdr_async(
                    accepted.stream,
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
                while let Some(Ok(Message::Text(text))) = ws.next().await {
                    let frame = parse_client_frame(text.as_bytes()).unwrap();
                    let machine = MachineInfo {
                        node_id: mac_id.clone(),
                        ..info()
                    };
                    let result = match frame.request {
                        Request::Hello(_) => Response::Hello(HelloResult {
                            protocol_version: PROTOCOL_VERSION,
                            collied_version: "test".into(),
                            machine,
                            herdr_version: None,
                            paired: true,
                        }),
                        Request::PairComplete(p) => {
                            assert_eq!(p.pairing_code.as_str(), CODE);
                            Response::Paired { machine }
                        }
                        Request::FlockSnapshot(_) => Response::Flock(Flock { machine, ..flock() }),
                        other => panic!("unexpected {}", other.method()),
                    };
                    let reply = ServerFrame::Result {
                        id: frame.id,
                        result,
                    };
                    let text = serde_json::to_string(&reply).unwrap();
                    ws.send(Message::text(text)).await.unwrap();
                }
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

    #[test]
    fn tailnet_end_to_end() {
        // libtailscale reads TS_* knobs once at load, so they need a fresh process.
        if KNOBS
            .iter()
            .any(|(k, v)| std::env::var(k).as_deref() != Ok(*v))
        {
            let out = Command::new(std::env::current_exe().unwrap())
                .args([
                    "tailnet_tests::tailnet_end_to_end",
                    "--exact",
                    "--nocapture",
                ])
                .envs(KNOBS)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            print!("{stdout}");
            eprint!("{}", String::from_utf8_lossy(&out.stderr));
            assert!(out.status.success(), "child run failed: {}", out.status);
            assert!(stdout.contains("1 passed"));
            return;
        }
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(180));
            eprintln!("watchdog: tailnet_end_to_end still running");
            std::process::exit(101);
        });

        let key = format!("tskey-auth-collie-core{}", std::process::id());
        let root = tempfile::tempdir().unwrap();
        let control = TestControl::start(&key, root.path());
        let phone_dir = root.path().join("phone");
        std::fs::create_dir(&phone_dir).unwrap();
        std::fs::set_permissions(&phone_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        let core = CollieCore::with_control_url(phone_dir.clone(), control.1.clone()).unwrap();
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

        core.runtime
            .spawn(serve(mac.clone(), phone_id, mac_self.stable_id.clone()));

        let host = mac_self.dns_name.trim_end_matches('.').to_owned();
        let mac_ip = mac_self.tailscale_ips.clone().unwrap()[0];
        let session = rt.block_on(core.runtime.spawn({
            let (node, host) = (phone_node.clone(), host.clone());
            async move {
                let stream = conn::dial(&node, std::net::SocketAddr::new(mac_ip, DEFAULT_PORT))
                    .await
                    .unwrap();
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
                code: PairingCode::new(CODE).unwrap(),
            }
            .to_uri()
        };
        let err = rt
            .block_on(core.pair(invite("nSOMEONEELSE"), "iPhone".into()))
            .unwrap_err();
        assert!(matches!(err, CoreError::PinViolation { .. }), "{err:?}");

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

        let again = rt
            .block_on(core.pair(invite(&mac_self.stable_id), "iPhone".into()))
            .unwrap();
        assert_ne!(again.id, machine.id);
        assert_eq!(core.machines(), vec![again.clone()]);
        assert!(core.cached_flock(machine.id).is_none());
        core.remove_machine(again.id).unwrap();
        assert!(core.machines().is_empty());
        drop(control);
    }
}
