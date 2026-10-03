use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use protocol::{Empty, HelloResult, Request};
use tailnet::{BackendState, Node};
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::pin::{self, PinError};
use crate::session::{FlockState, Reply, Session, SessionError, lock};
use crate::store::Machine;

const BACKOFF: [Duration; 4] = [
    Duration::from_secs(3),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];
const RESET_AFTER: Duration = Duration::from_secs(30);
const OFFLINE_POLL: Duration = Duration::from_secs(3);
const DIAL_ATTEMPT: Duration = Duration::from_secs(5);
const DIAL_BUDGET: Duration = Duration::from_secs(20);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const FOREGROUND_RECONNECT: Duration = Duration::from_secs(10);
const QUEUE: usize = 8;

/// Every holder clones the outer Arc, never the inner Node, so the strong count says
/// whether a replaced node is still open on the shared tsnet state dir.
pub type NodeSlot = Arc<Mutex<Option<Arc<Node>>>>;

#[derive(Debug, Clone, thiserror::Error)]
pub enum ConnectError {
    #[error("tailnet is not running")]
    Offline,
    #[error(transparent)]
    Pin(#[from] PinError),
    #[error("could not reach the Mac: {0}")]
    Dial(String),
    #[error(transparent)]
    Session(#[from] SessionError),
}

impl ConnectError {
    pub fn is_auth(&self) -> bool {
        match self {
            Self::Pin(e) => e.is_violation(),
            Self::Session(e) => e.is_auth(),
            _ => false,
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
pub async fn open(
    node: Arc<Node>,
    host: &str,
    port: u16,
    node_id: &str,
) -> Result<(Session<UnixStream>, HelloResult), ConnectError> {
    let n = node.clone();
    let status = blocking(move || n.status())
        .await
        .map_err(|e| ConnectError::Dial(e.to_string()))?
        .map_err(|e| ConnectError::Dial(e.to_string()))?;
    if status.backend_state != BackendState::Running {
        return Err(ConnectError::Offline);
    }
    let ip = pin::resolve(&status, host, node_id)?;
    let n = node.clone();
    let who = blocking(move || n.whois(&ip.to_string()))
        .await
        .map_err(|e| ConnectError::Dial(e.to_string()))?
        .map_err(|e| ConnectError::Dial(e.to_string()))?;
    pin::verify_whois(&who.node, host, node_id)?;
    let stream = dial(&node, SocketAddr::new(ip, port)).await?;
    drop(node);
    let mut session = Session::connect(stream, host, port).await?;
    let hello = session.hello().await?;
    pin::verify_node(&hello.machine.node_id, host, node_id)?;
    Ok((session, hello))
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
    Offline,
    Stopped,
}

pub struct Link {
    pub phase: LinkPhase,
    pub last_error: Option<String>,
}

pub struct Shared {
    pub flock: Mutex<FlockState>,
    pub link: Mutex<Link>,
}

impl Shared {
    fn set(&self, phase: LinkPhase, error: Option<String>) {
        let mut link = lock(&self.link);
        link.phase = phase;
        if error.is_some() || phase == LinkPhase::Connected {
            link.last_error = error;
        }
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
    pub fn spawn(runtime: &tokio::runtime::Handle, machine: Machine, node: NodeSlot) -> Self {
        let shared = Arc::new(Shared {
            flock: Mutex::default(),
            link: Mutex::new(Link {
                phase: LinkPhase::Connecting,
                last_error: None,
            }),
        });
        let (requests, rx) = mpsc::channel(QUEUE);
        let (reconnect, reconnect_rx) = watch::channel(0);
        let wake = Arc::new(Notify::new());
        let task = runtime.spawn(supervise(
            machine.clone(),
            node,
            shared.clone(),
            rx,
            reconnect_rx,
            wake.clone(),
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
    ) -> Result<protocol::Response, RequestError> {
        let (tx, rx) = oneshot::channel();
        self.requests.try_send((request, tx)).map_err(|e| match e {
            mpsc::error::TrySendError::Closed(_) => RequestError::Stopped,
            mpsc::error::TrySendError::Full(_) => RequestError::Busy,
        })?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(response))) => Ok(response),
            Ok(Ok(Err(e))) => Err(RequestError::Failed(e)),
            Ok(Err(_)) => Err(RequestError::Stopped),
            Err(_) => Err(RequestError::Timeout),
        }
    }

    pub fn reconnect_now(&self) {
        self.reconnect.send_modify(|n| *n += 1);
        self.wake.notify_one();
    }

    /// iOS suspends sockets without closing them: after a long background period the
    /// connection is assumed dead, after a short one it is probed.
    pub fn resume(self: &Arc<Self>, background: Duration) {
        if background >= FOREGROUND_RECONNECT {
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

async fn supervise(
    machine: Machine,
    node: NodeSlot,
    shared: Arc<Shared>,
    mut requests: mpsc::Receiver<(Request, Reply)>,
    mut reconnect: watch::Receiver<u64>,
    wake: Arc<Notify>,
) {
    let mut attempt = 0;
    loop {
        let running = lock(&node).clone();
        let Some(node) = running else {
            shared.set(LinkPhase::Offline, None);
            wait(&wake, OFFLINE_POLL).await;
            continue;
        };
        shared.set(LinkPhase::Connecting, None);
        let err = match open(node, &machine.host, machine.port, &machine.node_id).await {
            Ok((_, hello)) if !hello.paired => ConnectError::Session(SessionError::NotPaired),
            Ok((session, _)) => {
                lock(&shared.flock).new_connection();
                shared.set(LinkPhase::Connected, None);
                let since = Instant::now();
                reconnect.borrow_and_update();
                let end = session
                    .run(&mut requests, &shared.flock, async {
                        let _ = reconnect.changed().await;
                    })
                    .await;
                if since.elapsed() >= RESET_AFTER {
                    attempt = 0;
                }
                ConnectError::Session(end)
            }
            Err(ConnectError::Offline) => {
                shared.set(LinkPhase::Offline, None);
                wait(&wake, OFFLINE_POLL).await;
                continue;
            }
            Err(e) => e,
        };
        if err.is_auth() {
            shared.set(LinkPhase::Stopped, Some(err.to_string()));
            return;
        }
        shared.set(LinkPhase::Waiting, Some(err.to_string()));
        wait(&wake, BACKOFF[attempt.min(BACKOFF.len() - 1)]).await;
        attempt += 1;
    }
}

async fn wait(wake: &Notify, delay: Duration) {
    tokio::select! {
        () = tokio::time::sleep(delay) => {}
        () = wake.notified() => {}
    }
}
