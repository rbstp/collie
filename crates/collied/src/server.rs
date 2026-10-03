use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, SocketAddr};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use protocol::{
    ErrorBody, ErrorCode, Event, HelloResult, MachineInfo, PairCompleteParams, PairingCode,
    PairingInvite, Request, Response, ServerFrame, TerminalId,
};
use tailnet::{Accepted, BackendState, Node, WhoIs};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast, mpsc, oneshot, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request as HttpRequest, Response as HttpResponse,
};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode, header};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Bytes, Error as WsError, Message};

use crate::approvals::{self, Approvals};
use crate::audit::Audit;
use crate::control::{self, Candidate, PairAttempt, StatusInfo};
use crate::drive::{self, Authorized, Driver, Origin, Reply, Watched, Watcher};
use crate::flock::{self, Baseline, StatusTracker};
use crate::gate::{self, Decision};
use crate::pairing::{Attempt, Pairing};
use crate::peers::{self, Peer, Store};
use crate::push::{self, Push};
use crate::{config, herdr};

const MAX_CONNECTIONS: usize = 64;
const MAX_SESSIONS_PER_NODE: usize = 4;
const WHOIS_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const PING_EVERY: Duration = Duration::from_secs(15);
const SILENCE_LIMIT: Duration = Duration::from_secs(45);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const RECONCILE_EVERY: Duration = Duration::from_secs(1);
const RECONCILE_MAX_BACKOFF: Duration = Duration::from_secs(30);
const RATE_PER_SEC: f64 = 20.0;
const RATE_BURST: f64 = 40.0;
const REJECT_AUDIT_PER_SEC: f64 = 1.0;
const REJECT_AUDIT_BURST: f64 = 10.0;

pub struct ServerConfig {
    pub data_dir: PathBuf,
    pub port: u16,
    pub owner_user_id: Option<i64>,
    pub herdr_session: String,
    pub machine_name: String,
    pub approval_ttl: Duration,
}

pub struct ServerHandle {
    state: Arc<State>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    listener_dead: watch::Receiver<bool>,
    _peers_lock: OwnedFd,
}

impl ServerHandle {
    pub fn control_path(&self) -> PathBuf {
        self.state.cfg.data_dir.join(config::CONTROL_SOCKET)
    }

    /// Resolves if the tailnet listener fails for good; the daemon must then exit
    /// so launchd restarts it instead of serving nothing.
    pub async fn listener_failed(&mut self) {
        let _ = self.listener_dead.wait_for(|dead| *dead).await;
    }

    pub async fn shutdown(self) {
        let control = self.control_path();
        let _ = self.shutdown.send(true);
        for live in self.state.lock_sessions().live.values() {
            let _ = live.kill.send(true);
        }
        for task in self.tasks {
            let _ = task.await;
        }
        let _ = std::fs::remove_file(control);
    }
}

struct Live {
    stable_id: String,
    pairing_window: Option<u64>,
    kill: watch::Sender<bool>,
}

struct Sessions {
    next: u64,
    live: HashMap<u64, Live>,
}

struct TokenBucket {
    tokens: f64,
    at: Instant,
    limited: bool,
}

#[derive(PartialEq)]
enum Rate {
    Ok,
    FirstLimited,
    Limited,
}

fn take_token<K: Hash + Eq>(
    buckets: &Mutex<HashMap<K, TokenBucket>>,
    key: K,
    per_sec: f64,
    burst: f64,
) -> Rate {
    let now = Instant::now();
    let mut buckets = lock(buckets);
    let b = buckets.entry(key).or_insert(TokenBucket {
        tokens: burst,
        at: now,
        limited: false,
    });
    b.tokens = (b.tokens + now.duration_since(b.at).as_secs_f64() * per_sec).min(burst);
    b.at = now;
    if b.tokens >= 1.0 {
        b.tokens -= 1.0;
        b.limited = false;
        Rate::Ok
    } else if b.limited {
        Rate::Limited
    } else {
        b.limited = true;
        Rate::FirstLimited
    }
}

pub struct State {
    node: Node,
    cfg: ServerConfig,
    machine: MachineInfo,
    dns_name: String,
    herdr: PathBuf,
    peers: Arc<Mutex<Store>>,
    pairing: Mutex<Pairing<mpsc::Sender<PairAttempt>>>,
    sessions: Mutex<Sessions>,
    buckets: Mutex<HashMap<String, TokenBucket>>,
    reject_buckets: Mutex<HashMap<IpAddr, TokenBucket>>,
    tracker: Mutex<StatusTracker>,
    events: broadcast::Sender<Event>,
    drive: Arc<Driver>,
    approvals: Arc<Approvals>,
    push: Arc<Push>,
    pub(crate) audit: Arc<Audit>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl State {
    pub(crate) fn lock_pairing(&self) -> MutexGuard<'_, Pairing<mpsc::Sender<PairAttempt>>> {
        lock(&self.pairing)
    }

    fn lock_sessions(&self) -> MutexGuard<'_, Sessions> {
        lock(&self.sessions)
    }

    pub(crate) fn peers(&self) -> Store {
        lock(&self.peers).clone()
    }

    pub(crate) fn invite(&self, code: PairingCode) -> PairingInvite {
        PairingInvite {
            host: self.dns_name.clone(),
            port: self.cfg.port,
            node_id: self.machine.node_id.clone(),
            code,
        }
    }

    fn peers_path(&self) -> PathBuf {
        self.cfg.data_dir.join(config::PEERS_FILE)
    }

    pub(crate) fn add_peer(&self, peer: Peer) -> anyhow::Result<()> {
        let mut store = lock(&self.peers);
        if let Some(owner) = self.cfg.owner_user_id {
            anyhow::ensure!(
                peer.user_id == owner,
                "user {} is not the configured owner",
                peer.user_id
            );
        }
        let mut next = store.clone();
        next.add(peer)?;
        peers::save(&self.peers_path(), &next)?;
        *store = next;
        Ok(())
    }

    /// Persists first, then closes every live session of that node. The peers lock is held
    /// across both so a connection cannot pass the gate between them.
    pub(crate) fn revoke(&self, target: &str) -> anyhow::Result<(Peer, usize)> {
        let mut store = lock(&self.peers);
        let mut next = store.clone();
        let peer = next.remove(target)?;
        peers::save(&self.peers_path(), &next)?;
        *store = next;
        let mut closed = 0;
        for live in self.lock_sessions().live.values() {
            if live.stable_id == peer.stable_id {
                let _ = live.kill.send(true);
                closed += 1;
            }
        }
        drop(store);
        if let Err(e) = self.push.forget(&peer.stable_id) {
            tracing::error!(error = %e, "could not drop the revoked phone's APNs token");
        }
        self.audit.log(
            &peer.label,
            "peers.revoke",
            Some(&peer.stable_id),
            &format!("revoked, {closed} session(s) closed"),
        );
        Ok((peer, closed))
    }

    pub(crate) async fn status_info(&self) -> StatusInfo {
        let node = self.node.clone();
        let backend_state = match tokio::task::spawn_blocking(move || node.status()).await {
            Ok(Ok(st)) => format!("{:?}", st.backend_state),
            Ok(Err(e)) => format!("error: {e}"),
            Err(e) => format!("error: {e}"),
        };
        let herdr_version = herdr::ping(&self.herdr).await.ok().map(|p| p.version);
        let sessions = self.lock_sessions().live.len();
        let peers = lock(&self.peers).peers.len();
        StatusInfo {
            pid: std::process::id(),
            backend_state,
            dns_name: self.dns_name.clone(),
            node_id: self.machine.node_id.clone(),
            port: self.cfg.port,
            sessions,
            peers,
            herdr_version,
        }
    }

    /// Closes the window and every pairing-only session it admitted, so a session cannot
    /// outlive its window or carry over to the next one. Lock order: pairing, then sessions.
    pub(crate) fn end_window(&self, window: u64) {
        let mut pairing = self.lock_pairing();
        pairing.close(window);
        for live in self.lock_sessions().live.values() {
            if live.pairing_window == Some(window) {
                let _ = live.kill.send(true);
            }
        }
        drop(pairing);
    }

    fn peer_authorized(&self, stable_id: &str, user: i64) -> bool {
        lock(&self.peers)
            .get(stable_id)
            .is_some_and(|p| p.user_id == user)
    }

    fn rate(&self, key: &str) -> Rate {
        take_token(&self.buckets, key.to_owned(), RATE_PER_SEC, RATE_BURST)
    }

    /// Rejections happen before any rate limit tied to an identity, so their audit lines
    /// are limited per source address to keep a flooding peer from growing the log
    /// without bound.
    fn audit_reject(&self, addr: SocketAddr, peer: &str, reason: &str) {
        let addr_text = addr.to_string();
        let result = format!("rejected: {reason}");
        match take_token(
            &self.reject_buckets,
            addr.ip(),
            REJECT_AUDIT_PER_SEC,
            REJECT_AUDIT_BURST,
        ) {
            Rate::Ok => self.audit.log(peer, "connect", Some(&addr_text), &result),
            Rate::FirstLimited => self.audit.log(
                peer,
                "connect",
                Some(&addr_text),
                &format!("{result}; further rejections from this address suppressed"),
            ),
            Rate::Limited => {}
        }
    }
}

pub async fn start(
    node: Node,
    cfg: ServerConfig,
    herdr_socket: PathBuf,
) -> anyhow::Result<ServerHandle> {
    start_with_tasks(node, cfg, herdr_socket, &config::TasksConfig::default()).await
}

/// Fails closed: the node must already be Running, and the only TCP listener is the
/// tailnet one.
pub async fn start_with_tasks(
    node: Node,
    cfg: ServerConfig,
    herdr_socket: PathBuf,
    tasks: &config::TasksConfig,
) -> anyhow::Result<ServerHandle> {
    start_with(node, cfg, herdr_socket, tasks, None).await
}

pub async fn start_with(
    node: Node,
    cfg: ServerConfig,
    herdr_socket: PathBuf,
    tasks: &config::TasksConfig,
    apns: Option<Arc<dyn push::Sender>>,
) -> anyhow::Result<ServerHandle> {
    crate::ensure_private_dir(&cfg.data_dir)?;
    let roots = match &tasks.roots {
        Some(roots) => roots.clone(),
        None => vec![config::home_dir()?],
    };
    let drive = Arc::new(Driver::new(
        herdr_socket.clone(),
        tasks.agents.clone(),
        &roots,
    )?);
    let st = {
        let node = node.clone();
        tokio::task::spawn_blocking(move || node.status()).await??
    };
    anyhow::ensure!(
        st.backend_state == BackendState::Running,
        "tailnet node is {:?}, not Running",
        st.backend_state
    );
    let me = st
        .self_node
        .ok_or_else(|| anyhow::anyhow!("tailnet status has no self node"))?;
    let dns_name = me.dns_name.trim_end_matches('.').to_owned();
    anyhow::ensure!(
        !dns_name.is_empty() && !me.stable_id.is_empty(),
        "tailnet node has no MagicDNS name or stable id yet"
    );
    let peers_lock = peers::lock(&cfg.data_dir.join(config::PEERS_LOCK))?;
    let store = peers::load(&cfg.data_dir.join(config::PEERS_FILE))?;
    if let (Some(configured), Some(stored)) = (cfg.owner_user_id, store.owner_user_id) {
        anyhow::ensure!(
            configured == stored,
            "owner_user_id {configured} in the config differs from {stored} in peers.json"
        );
    }
    let peers = Arc::new(Mutex::new(store));
    let audit = Arc::new(Audit::open(&cfg.data_dir.join(config::AUDIT_FILE))?);
    let push = {
        let peers = peers.clone();
        Push::open(
            cfg.data_dir.join(config::PUSH_FILE),
            apns,
            Arc::new(move |id| lock(&peers).get(id).is_some()),
        )?
    };
    push.retain_paired(&lock(&peers))?;
    let events = broadcast::channel(256).0;
    let approvals = Arc::new(
        Approvals::new(
            herdr_socket.clone(),
            me.stable_id.clone(),
            events.clone(),
            audit.clone(),
            Some(push.clone()),
        )
        .with_timing(cfg.approval_ttl, approvals::SETTLE),
    );
    let control = control::bind(&cfg.data_dir.join(config::CONTROL_SOCKET)).await?;
    let listener = node.listen("tcp", &format!(":{}", cfg.port))?;

    let state = Arc::new(State {
        machine: MachineInfo {
            name: cfg.machine_name.clone(),
            node_id: me.stable_id,
            herdr_session: cfg.herdr_session.clone(),
        },
        node,
        cfg,
        dns_name,
        herdr: herdr_socket,
        peers,
        pairing: Mutex::new(Pairing::default()),
        sessions: Mutex::new(Sessions {
            next: 1,
            live: HashMap::new(),
        }),
        buckets: Mutex::new(HashMap::new()),
        reject_buckets: Mutex::new(HashMap::new()),
        tracker: Mutex::new(StatusTracker::default()),
        events,
        drive,
        approvals,
        push,
        audit,
    });
    let (shutdown, rx) = watch::channel(false);
    let (dead_tx, listener_dead) = watch::channel(false);
    let tasks = vec![
        tokio::spawn(accept_loop(listener, state.clone(), rx.clone(), dead_tx)),
        tokio::spawn(control::serve(control, state.clone(), rx.clone())),
        tokio::spawn(reconcile(state.clone(), rx)),
    ];
    tracing::info!(host = %state.dns_name, port = state.cfg.port, "listening on the tailnet");
    Ok(ServerHandle {
        state,
        shutdown,
        tasks,
        listener_dead,
        _peers_lock: peers_lock,
    })
}

const FORBIDDEN: &[u8] =
    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const MAX_ACCEPT_FAILURES: u32 = 50;

async fn accept_loop(
    listener: tailnet::Listener,
    state: Arc<State>,
    mut shutdown: watch::Receiver<bool>,
    dead: watch::Sender<bool>,
) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let mut failures = 0u32;
    loop {
        tokio::select! {
            res = listener.accept() => match res {
                Ok(Accepted { stream, peer }) => { failures = 0; match slots.clone().try_acquire_owned() {
                    Ok(permit) => {
                        tokio::spawn(connection(state.clone(), stream, peer, permit));
                    }
                    Err(_) => {
                        drop(stream);
                        state.audit_reject(peer, &peer.to_string(), "too many connections");
                    }
                }},
                Err(e) => {
                    failures += 1;
                    if failures >= MAX_ACCEPT_FAILURES {
                        tracing::error!(error = %e, "tailnet listener keeps failing; giving up");
                        let _ = dead.send(true);
                        return;
                    }
                    tracing::warn!(error = %e, "tailnet accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            _ = shutdown.changed() => return,
        }
    }
}

struct Registration {
    state: Arc<State>,
    id: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.state.lock_sessions().live.remove(&self.id);
    }
}

struct Remote {
    who: WhoIs,
    name: String,
    pairing_window: Option<u64>,
}

impl Remote {
    fn paired(&self) -> bool {
        self.pairing_window.is_none()
    }
}

async fn connection(
    state: Arc<State>,
    stream: UnixStream,
    addr: SocketAddr,
    permit: OwnedSemaphorePermit,
) {
    let addr_text = addr.to_string();
    // The blocking whois keeps the slot until it really returns, so timed-out lookups
    // still count against MAX_CONNECTIONS.
    let permit = Arc::new(permit);
    let who = {
        let node = state.node.clone();
        let addr = addr_text.clone();
        let held = permit.clone();
        let lookup = tokio::task::spawn_blocking(move || {
            let _held = held;
            node.whois(&addr)
        });
        tokio::time::timeout(WHOIS_TIMEOUT, lookup).await
    };
    let who = match who {
        Ok(Ok(Ok(who))) => who,
        Ok(Ok(Err(e))) => {
            tracing::debug!(error = %e, peer = %addr, "whois failed");
            state.audit_reject(addr, &addr_text, "whois failed");
            return;
        }
        Ok(Err(_)) => return,
        Err(_) => {
            state.audit_reject(addr, &addr_text, "whois timed out");
            return;
        }
    };
    let (decision, kill, registration, evicted) = {
        // Lock order: peers, pairing, sessions. Holding all three makes the decision, the
        // per-node cap and the registration atomic with respect to revoke and end_window.
        let store = lock(&state.peers);
        let pairing = state.lock_pairing();
        let mut sessions = state.lock_sessions();
        let window = pairing.current(Instant::now());
        let decision = gate::decide(&who.node, state.cfg.owner_user_id, &store, window);
        // A phone that reconnects after iOS killed its sockets leaves dead sessions
        // until SILENCE_LIMIT. Evicting the node's oldest session keeps one node from
        // holding every slot without locking a reconnecting phone out.
        let mut evicted = None;
        if !matches!(decision, Decision::Reject(_)) {
            let mine: Vec<u64> = sessions
                .live
                .iter()
                .filter(|(_, l)| l.stable_id == who.node.stable_id)
                .map(|(id, _)| *id)
                .collect();
            if mine.len() >= MAX_SESSIONS_PER_NODE
                && let Some(oldest) = mine.iter().min()
                && let Some(live) = sessions.live.get(oldest)
            {
                let _ = live.kill.send(true);
                evicted = Some(*oldest);
            }
        }
        if let Decision::Reject(reason) = decision {
            drop(sessions);
            drop(pairing);
            drop(store);
            state.audit_reject(addr, &who.node.stable_id, reason);
            // Nothing from the peer is read. The fixed 403 lets a revoked phone stop
            // retrying instead of treating the close as a transport error.
            let mut stream = stream;
            tokio::spawn(async move {
                let _ = tokio::time::timeout(SEND_TIMEOUT, stream.write_all(FORBIDDEN)).await;
            });
            return;
        }
        let pairing_window = match decision {
            Decision::PairingOnly { window } => Some(window),
            _ => None,
        };
        let (tx, rx) = watch::channel(false);
        let id = sessions.next;
        sessions.next += 1;
        sessions.live.insert(
            id,
            Live {
                stable_id: who.node.stable_id.clone(),
                pairing_window,
                kill: tx,
            },
        );
        drop(sessions);
        drop(pairing);
        (
            decision,
            rx,
            Registration {
                state: state.clone(),
                id,
            },
            evicted,
        )
    };
    if let Some(old) = evicted {
        state.audit.log(
            &who.node.stable_id,
            "connect",
            Some(&addr_text),
            &format!("evicted session {old}: per-node limit"),
        );
    }
    let (name, pairing_window) = match decision {
        Decision::Full { label } => (label, None),
        Decision::PairingOnly { window } => (who.node.stable_id.clone(), Some(window)),
        Decision::Reject(_) => return,
    };
    let handshake = tokio_tungstenite::accept_hdr_async_with_config(
        stream,
        check_upgrade,
        Some(
            WebSocketConfig::default()
                .max_message_size(Some(protocol::limits::MAX_FRAME_BYTES))
                .max_frame_size(Some(protocol::limits::MAX_FRAME_BYTES)),
        ),
    );
    let ws = match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            state.audit_reject(addr, &name, &format!("websocket handshake: {e}"));
            return;
        }
        Err(_) => {
            state.audit_reject(addr, &name, "websocket handshake timed out");
            return;
        }
    };
    let peer = Remote {
        who,
        name,
        pairing_window,
    };
    Session {
        state: &state,
        id: registration.id,
        ws,
        peer: &peer,
        seq: 0,
        watch: None,
        tasks: JoinSet::new(),
        starting: None,
    }
    .run(kill)
    .await;
    drop(registration);
    drop(permit);
}

#[allow(clippy::result_large_err)]
fn check_upgrade(req: &HttpRequest, mut resp: HttpResponse) -> Result<HttpResponse, ErrorResponse> {
    let reject = |status: StatusCode| {
        let mut r = ErrorResponse::new(None);
        *r.status_mut() = status;
        r
    };
    if req.uri().path() != protocol::WS_PATH || req.uri().query().is_some() {
        return Err(reject(StatusCode::NOT_FOUND));
    }
    let offered = req
        .headers()
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|p| p.trim() == protocol::WS_SUBPROTOCOL);
    if !offered {
        return Err(reject(StatusCode::BAD_REQUEST));
    }
    resp.headers_mut().insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static(protocol::WS_SUBPROTOCOL),
    );
    Ok(resp)
}

struct Session<'a> {
    state: &'a Arc<State>,
    id: u64,
    ws: WebSocketStream<UnixStream>,
    peer: &'a Remote,
    seq: u64,
    watch: Option<Watcher>,
    // Dropping the set only stops waiting: the operations run detached in the op cache
    // and audit their own outcome.
    tasks: JoinSet<Finished>,
    starting: Option<(u32, Option<String>)>,
}

struct Finished {
    id: u32,
    target: Option<String>,
    reply: Reply,
    origin: Option<Origin>,
}

enum Flow {
    Continue,
    Close,
    Abort,
}

enum Step {
    HelloTimeout,
    Ping { idle: bool },
    Event(Result<Event, broadcast::error::RecvError>),
    Watch(Option<Watched>),
    Finished(Option<Result<Finished, tokio::task::JoinError>>),
    Message(Option<Result<Message, WsError>>),
}

fn err(code: ErrorCode, message: &str) -> Reply {
    Err((code, message.to_owned()))
}

impl Session<'_> {
    async fn run(mut self, mut kill: watch::Receiver<bool>) {
        let mut greeted = false;
        let hello_deadline = tokio::time::sleep(HELLO_TIMEOUT);
        tokio::pin!(hello_deadline);
        let mut ping =
            tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
        let mut last_seen = Instant::now();
        let mut events: Option<broadcast::Receiver<Event>> = None;
        loop {
            let step = tokio::select! {
                biased;
                _ = kill.changed() => return,
                _ = &mut hello_deadline, if !greeted => Step::HelloTimeout,
                _ = ping.tick() => Step::Ping {
                    idle: last_seen.elapsed() > SILENCE_LIMIT,
                },
                ev = next_event(&mut events) => Step::Event(ev),
                out = next_watch(&mut self.watch) => Step::Watch(out),
                done = self.tasks.join_next(), if !self.tasks.is_empty() => Step::Finished(done),
                msg = self.ws.next() => {
                    last_seen = Instant::now();
                    Step::Message(msg)
                }
            };
            // A killed session is dropped without a close handshake: the handshake would
            // flush a frame still buffered from a send the kill interrupted.
            let flow = tokio::select! {
                biased;
                _ = kill.changed() => return,
                flow = self.step(step, &mut greeted, &mut events) => flow,
            };
            match flow {
                Flow::Continue => {}
                Flow::Close => break,
                Flow::Abort => return,
            }
        }
        tokio::select! {
            biased;
            _ = kill.changed() => {}
            _ = tokio::time::timeout(SEND_TIMEOUT, self.ws.close(None)) => {}
        }
    }

    async fn step(
        &mut self,
        step: Step,
        greeted: &mut bool,
        events: &mut Option<broadcast::Receiver<Event>>,
    ) -> Flow {
        match step {
            Step::HelloTimeout => {
                self.send_error(None, ErrorCode::HelloRequired, "hello timed out")
                    .await;
                Flow::Close
            }
            Step::Ping { idle: true } => Flow::Close,
            Step::Ping { idle: false } => self.send(Message::Ping(Bytes::new())).await,
            Step::Event(Ok(event)) => self.push(event).await,
            Step::Event(Err(broadcast::error::RecvError::Lagged(_))) => {
                self.push(Event::FlockChanged {}).await
            }
            Step::Event(Err(broadcast::error::RecvError::Closed)) => Flow::Close,
            Step::Watch(Some(Watched::Output(read))) => self.push(Event::AgentOutput(read)).await,
            Step::Watch(Some(Watched::Gone) | None) => {
                self.watch = None;
                self.push(Event::FlockChanged {}).await
            }
            Step::Finished(Some(Ok(done))) => {
                self.starting = None;
                self.finish("task.new", done).await
            }
            Step::Finished(_) => self.task_lost().await,
            Step::Message(Some(Ok(Message::Text(text)))) => {
                self.frame(text.as_bytes(), greeted, events).await
            }
            Step::Message(Some(Ok(Message::Binary(_)))) => {
                self.send_error(None, ErrorCode::MalformedFrame, "text frames only")
                    .await;
                Flow::Close
            }
            Step::Message(Some(Ok(Message::Close(_))) | Some(Err(_)) | None) => Flow::Close,
            Step::Message(Some(Ok(_))) => Flow::Continue,
        }
    }

    async fn frame(
        &mut self,
        bytes: &[u8],
        greeted: &mut bool,
        events: &mut Option<broadcast::Receiver<Event>>,
    ) -> Flow {
        // Every frame, parseable or not, takes a token first so that neither replies nor
        // audit lines can outrun the per-peer limit.
        let rate = self.state.rate(&self.peer.who.node.stable_id);
        let parsed = protocol::parse_client_frame(bytes);
        let keep_open = *greeted && self.peer.pairing_window.is_none();
        if rate != Rate::Ok {
            let (id, method) = match &parsed {
                Ok(f) => (Some(f.id), f.request.method()),
                Err(e) => (e.id, "invalid_frame"),
            };
            if rate == Rate::FirstLimited {
                self.audit(method, "rate_limited");
            }
            self.send_error(id, ErrorCode::RateLimited, "slow down")
                .await;
            return if keep_open {
                Flow::Continue
            } else {
                Flow::Close
            };
        }
        let frame = match parsed {
            Ok(f) => f,
            Err(e) => {
                self.audit("invalid_frame", code_name(e.code).as_str());
                let flow = self.send_frame(&e.into_frame()).await;
                return if keep_open { flow } else { Flow::Close };
            }
        };
        let id = frame.id;
        let method = frame.request.method();
        if !*greeted {
            return match frame.request {
                Request::Hello(p) if p.protocol_version == protocol::PROTOCOL_VERSION => {
                    *greeted = true;
                    if self.peer.paired() {
                        *events = Some(self.state.events.subscribe());
                    }
                    let hello = self.hello().await;
                    self.reply(id, Ok(hello)).await
                }
                Request::Hello(_) => {
                    self.audit(method, "unsupported_protocol");
                    self.send_error(
                        Some(id),
                        ErrorCode::UnsupportedProtocol,
                        "unsupported protocol version",
                    )
                    .await;
                    Flow::Close
                }
                _ => {
                    self.audit(method, "hello_required");
                    self.send_error(Some(id), ErrorCode::HelloRequired, "hello first")
                        .await;
                    Flow::Close
                }
            };
        }
        if let Some(window) = self.peer.pairing_window {
            let reply = match frame.request {
                Request::PairComplete(p) => self.pair_complete(window, p).await,
                _ => {
                    self.audit(method, "not_paired");
                    err(ErrorCode::NotPaired, "pairing only")
                }
            };
            self.reply(id, reply).await;
            return Flow::Close;
        }
        let target = audit_target(&frame.request);
        let fingerprint = drive::fingerprint(&frame.request);
        let drive = self.state.drive.clone();
        let peer = self.peer.who.node.stable_id.clone();
        let auth = self.authorizer();
        let (reply, origin) = match frame.request {
            Request::Hello(_) => (
                err(ErrorCode::InvalidParams, "hello already received"),
                None,
            ),
            Request::PairComplete(_) => (err(ErrorCode::PairingFailed, "already paired"), None),
            Request::FlockSnapshot(_) => (self.flock().await, None),
            Request::WorkspaceList(_) => (self.workspaces().await, None),
            Request::AgentRead(p) => (drive.read(p, true).await, None),
            Request::PaneRead(p) => (drive.read(p, false).await, None),
            Request::AgentWatch(p) => (self.watch(p.terminal_id).await, None),
            Request::TaskOptions(_) => (drive.task_options().await, None),
            Request::AgentPrompt(p) => {
                let (op_id, d) = (p.op_id.clone(), drive.clone());
                let op = self.audited(method, target.clone(), async move {
                    (d.prompt(p, &auth).await, None)
                });
                let (reply, origin) = drive.once(&peer, &op_id, fingerprint, op).await;
                (reply, Some(origin))
            }
            Request::AgentSendKeys(p) => {
                let (op_id, d) = (p.op_id.clone(), drive.clone());
                let op = self.audited(method, target.clone(), async move {
                    (d.send_keys(p, &auth).await, None)
                });
                let (reply, origin) = drive.once(&peer, &op_id, fingerprint, op).await;
                (reply, Some(origin))
            }
            Request::AgentFocus(p) => (drive.focus(&p.terminal_id, &auth).await, None),
            // Starting an agent takes up to 30 s; the session keeps serving meanwhile.
            Request::TaskNew(_) if !self.tasks.is_empty() => (
                err(ErrorCode::RateLimited, "a task is already starting"),
                None,
            ),
            Request::TaskNew(p) => {
                let (op_id, d) = (p.op_id.clone(), drive.clone());
                let op = self.audited(method, target.clone(), async move {
                    let (reply, cwd) = d.task_new(p, &auth).await;
                    (reply, cwd.map(|c| c.as_str().to_owned()))
                });
                self.starting = Some((id, target.clone()));
                self.tasks.spawn(async move {
                    let (reply, origin) = drive.once(&peer, &op_id, fingerprint, op).await;
                    Finished {
                        id,
                        target,
                        reply,
                        origin: Some(origin),
                    }
                });
                return Flow::Continue;
            }
            Request::WorkspaceClose(p) => (drive.workspace_close(p, &auth).await, None),
            Request::PaneClose(p) => (drive.pane_close(p, &auth).await, None),
            Request::ApprovalList(_) => (
                Ok(Response::Approvals {
                    approvals: self.state.approvals.pending(),
                }),
                None,
            ),
            // Detached so a session dropped mid-decision still resolves the approval; it
            // audits every attempt itself.
            Request::ApprovalDecide(p) => {
                let (approvals, name) = (self.state.approvals.clone(), self.peer.name.clone());
                let reply =
                    tokio::spawn(async move { approvals.decide(&name, &peer, p, &auth).await })
                        .await
                        .unwrap_or_else(|_| err(ErrorCode::Internal, "decision failed"));
                (reply, Some(Origin::Ran))
            }
            Request::PushRegister(_) if !auth() => (
                err(ErrorCode::NotPaired, "peer is no longer authorized"),
                None,
            ),
            Request::PushRegister(p) => (
                self.state
                    .push
                    .register(&peer, p.apns_token, p.environment, p.notification_key)
                    .map(|()| Response::Ok)
                    .map_err(|e| {
                        tracing::error!(error = %e, "push.register");
                        (ErrorCode::Internal, "could not store the token".to_owned())
                    }),
                None,
            ),
            Request::PushActivityToken(_) => {
                (err(ErrorCode::NotImplemented, "not implemented"), None)
            }
        };
        self.finish(
            method,
            Finished {
                id,
                target,
                reply,
                origin,
            },
        )
        .await
    }

    /// A mutation that ran in this call was already audited by the detached operation.
    async fn finish(&mut self, method: &str, done: Finished) -> Flow {
        let quiet = matches!(
            method,
            "hello" | "flock.snapshot" | "workspace.list" | "approval.list"
        );
        if !quiet && done.origin != Some(Origin::Ran) {
            let mut result = outcome(&done.reply);
            if done.origin == Some(Origin::Replayed) {
                result.push_str(" (replayed)");
            }
            self.state
                .audit
                .log(&self.peer.name, method, done.target.as_deref(), &result);
        }
        self.reply(done.id, done.reply).await
    }

    async fn task_lost(&mut self) -> Flow {
        let Some((id, target)) = self.starting.take() else {
            return Flow::Continue;
        };
        self.finish(
            "task.new",
            Finished {
                id,
                target,
                reply: err(ErrorCode::Internal, "task.new failed"),
                origin: None,
            },
        )
        .await
    }

    async fn watch(&mut self, terminal_id: Option<TerminalId>) -> Reply {
        self.watch = None;
        if let Some(t) = terminal_id {
            self.watch = Some(self.state.drive.watch(t).await?);
        }
        Ok(Response::Ok)
    }

    fn audited<F>(
        &self,
        method: &'static str,
        target: Option<String>,
        op: F,
    ) -> impl Future<Output = Reply> + Send + 'static
    where
        F: Future<Output = (Reply, Option<String>)> + Send + 'static,
    {
        audited(
            self.state.audit.clone(),
            self.peer.name.clone(),
            method,
            target,
            op,
        )
    }

    fn authorizer(&self) -> Authorized {
        let state = self.state.clone();
        let (stable_id, user) = (
            self.peer.who.node.stable_id.clone(),
            self.peer.who.node.user,
        );
        Arc::new(move || state.peer_authorized(&stable_id, user))
    }

    fn audit(&self, method: &str, result: &str) {
        self.state.audit.log(&self.peer.name, method, None, result);
    }

    async fn hello(&self) -> Response {
        let paired = self.peer.paired();
        // An unpaired peer learns nothing about herdr and cannot make collied call it.
        let herdr_version = if paired {
            herdr::ping(&self.state.herdr).await.ok().map(|p| p.version)
        } else {
            None
        };
        Response::Hello(HelloResult {
            protocol_version: protocol::PROTOCOL_VERSION,
            collied_version: env!("CARGO_PKG_VERSION").to_owned(),
            machine: self.state.machine.clone(),
            herdr_version,
            paired,
        })
    }

    async fn flock(&self) -> Reply {
        let snap = herdr_result(herdr::session_snapshot(&self.state.herdr).await)?;
        let mut tracker = lock(&self.state.tracker);
        let mut flock = flock::map_flock(
            &snap,
            &mut tracker,
            crate::now_ms(),
            self.state.machine.clone(),
            self.seq,
        );
        flock.approvals = self.state.approvals.pending();
        Ok(Response::Flock(flock))
    }

    async fn workspaces(&self) -> Reply {
        let snap = herdr_result(herdr::session_snapshot(&self.state.herdr).await)?;
        Ok(Response::Workspaces {
            workspaces: flock::map_workspaces(&snap),
        })
    }

    async fn pair_complete(&self, window: u64, p: PairCompleteParams) -> Reply {
        let who = &self.peer.who;
        let attempt = {
            let mut pairing = self.state.lock_pairing();
            let attempt = pairing.attempt(window, &p.pairing_code, Instant::now());
            // The attempt spends the window; detaching this session from it lets the
            // outcome reach the phone after the control side calls end_window.
            if !matches!(attempt, Attempt::NoWindow)
                && let Some(live) = self.state.lock_sessions().live.get_mut(&self.id)
            {
                live.pairing_window = None;
            }
            attempt
        };
        let candidate = Candidate {
            device_label: p.device_label.as_str().to_owned(),
            node_name: who.node.name.trim_end_matches('.').to_owned(),
            stable_id: who.node.stable_id.clone(),
            login: who
                .user_profile
                .as_ref()
                .map(|u| u.login_name.clone())
                .unwrap_or_default(),
            user_id: who.node.user,
        };
        let (tx, code_ok) = match attempt {
            Attempt::NoWindow => {
                self.audit_pair("no open window");
                return err(ErrorCode::PairingFailed, "pairing failed");
            }
            Attempt::WrongCode(tx) => (tx, false),
            Attempt::Accepted(tx) => (tx, true),
        };
        let (reply, rx) = oneshot::channel();
        let sent = tx
            .send(PairAttempt {
                candidate,
                code_ok,
                reply,
            })
            .await
            .is_ok();
        let paired = sent
            && matches!(
                tokio::time::timeout(control::CONFIRM_TIMEOUT + Duration::from_secs(5), rx).await,
                Ok(Ok(true))
            );
        self.audit_pair(match (code_ok, paired) {
            (false, _) => "wrong code",
            (true, false) => "refused",
            (true, true) => "paired",
        });
        if paired {
            Ok(Response::Paired {
                machine: self.state.machine.clone(),
            })
        } else {
            err(ErrorCode::PairingFailed, "pairing failed")
        }
    }

    fn audit_pair(&self, result: &str) {
        self.state.audit.log(
            &self.peer.name,
            "pair.complete",
            Some(&self.peer.who.node.stable_id),
            result,
        );
    }

    async fn push(&mut self, event: Event) -> Flow {
        self.seq += 1;
        let frame = ServerFrame::Event {
            seq: self.seq,
            event,
        };
        self.send_frame(&frame).await
    }

    async fn reply(&mut self, id: u32, reply: Reply) -> Flow {
        let frame = match reply {
            Ok(result) => ServerFrame::Result { id, result },
            Err((code, message)) => ServerFrame::Error {
                id: Some(id),
                error: ErrorBody { code, message },
            },
        };
        self.send_frame(&frame).await
    }

    async fn send_error(&mut self, id: Option<u32>, code: ErrorCode, message: &str) {
        let frame = ServerFrame::Error {
            id,
            error: ErrorBody {
                code,
                message: message.to_owned(),
            },
        };
        self.send_frame(&frame).await;
    }

    async fn send_frame(&mut self, frame: &ServerFrame) -> Flow {
        if !self.authorized() {
            return Flow::Abort;
        }
        match serde_json::to_string(frame) {
            Ok(text) => self.send(Message::text(text)).await,
            Err(_) => Flow::Close,
        }
    }

    fn authorized(&self) -> bool {
        if !self.peer.paired() {
            return true;
        }
        let node = &self.peer.who.node;
        self.state.peer_authorized(&node.stable_id, node.user)
    }

    async fn send(&mut self, msg: Message) -> Flow {
        match tokio::time::timeout(SEND_TIMEOUT, self.ws.send(msg)).await {
            Ok(Ok(())) => Flow::Continue,
            _ => Flow::Close,
        }
    }
}

async fn next_event(
    rx: &mut Option<broadcast::Receiver<Event>>,
) -> Result<Event, broadcast::error::RecvError> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

async fn next_watch(watch: &mut Option<Watcher>) -> Option<Watched> {
    match watch {
        Some(w) => w.recv().await,
        None => std::future::pending().await,
    }
}

fn audit_target(request: &Request) -> Option<String> {
    let target = match request {
        Request::AgentRead(p) | Request::PaneRead(p) => p.terminal_id.as_str(),
        Request::AgentWatch(p) => p.terminal_id.as_ref().map_or("none", |t| t.as_str()),
        Request::AgentPrompt(p) => p.terminal_id.as_str(),
        Request::AgentSendKeys(p) => p.terminal_id.as_str(),
        Request::AgentFocus(p) => p.terminal_id.as_str(),
        Request::PaneClose(p) => p.terminal_id.as_str(),
        Request::WorkspaceClose(p) => p.workspace_id.as_str(),
        Request::TaskNew(p) => p.cwd.as_str(),
        _ => return None,
    };
    Some(target.to_owned())
}

/// Runs a cached mutation and writes its audit line itself, so the outcome is audited even
/// when the session that asked for it has ended. The op may name a more precise target.
async fn audited<F>(
    audit: Arc<Audit>,
    peer: String,
    method: &'static str,
    target: Option<String>,
    op: F,
) -> Reply
where
    F: Future<Output = (Reply, Option<String>)> + Send + 'static,
{
    let (reply, resolved) = tokio::spawn(op)
        .await
        .unwrap_or_else(|_| (err(ErrorCode::Internal, "operation failed"), None));
    audit.log(
        &peer,
        method,
        resolved.or(target).as_deref(),
        &outcome(&reply),
    );
    reply
}

fn outcome(reply: &Reply) -> String {
    match reply {
        Ok(Response::TaskStarted {
            workspace_id,
            terminal_id,
        }) => format!(
            "ok workspace={} terminal={}",
            workspace_id.as_str(),
            terminal_id.as_str()
        ),
        Ok(_) => "ok".to_owned(),
        Err((code, message)) => format!("{}: {message}", code_name(*code)),
    }
}

fn code_name(code: ErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn herdr_result<T>(res: Result<T, herdr::Error>) -> Result<T, (ErrorCode, String)> {
    res.map_err(|e| {
        tracing::warn!(error = %e, "herdr call failed");
        (ErrorCode::HerdrUnavailable, "herdr unavailable".to_owned())
    })
}

async fn reconcile(state: Arc<State>, mut shutdown: watch::Receiver<bool>) {
    let mut base: Option<Baseline> = None;
    let mut outage = false;
    let mut delay = RECONCILE_EVERY;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = shutdown.changed() => return,
        }
        let polled = tokio::try_join!(
            herdr::agent_list(&state.herdr),
            herdr::workspace_list(&state.herdr)
        );
        let (agents, workspaces) = match polled {
            Ok(v) => v,
            Err(e) => {
                if !outage {
                    tracing::warn!(error = %e, "herdr unavailable, retrying with backoff");
                }
                outage = true;
                base = None;
                delay = (delay * 2).min(RECONCILE_MAX_BACKOFF);
                continue;
            }
        };
        delay = RECONCILE_EVERY;
        let next = Baseline::new(&agents, &workspaces);
        match &base {
            None if outage => {
                tracing::info!("herdr reachable again");
                let _ = state.events.send(Event::FlockChanged {});
            }
            None => {}
            Some(prev) => {
                let (changed, shape) = prev.diff(&next);
                if !changed.is_empty() {
                    let mut tracker = lock(&state.tracker);
                    let now = crate::now_ms();
                    for a in agents.iter().filter(|a| changed.contains(&a.terminal_id)) {
                        if let Some(agent) = flock::map_agent(a, &mut tracker, now) {
                            let _ = state.events.send(Event::AgentStatus { agent });
                        }
                    }
                }
                if shape {
                    let _ = state.events.send(Event::FlockChanged {});
                }
            }
        }
        base = Some(next);
        outage = false;
        state.approvals.observe(&agents, &workspaces).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn detached_mutations_audit_themselves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let audit = Arc::new(Audit::open(&path).unwrap());
        let lines = || -> Vec<serde_json::Value> {
            std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        };

        let op = audited(
            audit.clone(),
            "phone".into(),
            "task.new",
            Some("/req/../cwd".into()),
            async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                (
                    Ok(Response::TaskStarted {
                        workspace_id: protocol::WorkspaceId::new("w9").unwrap(),
                        terminal_id: TerminalId::new("term_new").unwrap(),
                    }),
                    Some("/cwd".to_owned()),
                )
            },
        );
        // Nobody waits for the outcome, as when the phone disconnects mid-operation.
        drop(tokio::spawn(op));
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(lines().is_empty());
        tokio::time::sleep(Duration::from_millis(100)).await;
        let first = &lines()[0];
        assert_eq!(first["method"], "task.new");
        assert_eq!(first["target"], "/cwd");
        assert_eq!(first["result"], "ok workspace=w9 terminal=term_new");

        let panicked = audited(
            audit.clone(),
            "phone".into(),
            "agent.prompt",
            Some("term_1".into()),
            async { panic!("boom") },
        )
        .await;
        assert_eq!(panicked.unwrap_err().0, ErrorCode::Internal);
        let second = &lines()[1];
        assert_eq!(second["target"], "term_1");
        assert_eq!(second["result"], "internal: operation failed");
    }
}
