use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use protocol::{Empty, HelloResult, PushRegisterParams, Request, Response};
use tailnet::{BackendState, Node};
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::pin::{self, PinError};
use crate::reach::Reachability;
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
const MUTATION_ATTEMPTS: usize = 3;

/// Every holder clones the outer Arc, never the inner Node, so the strong count says
/// whether a replaced node is still open on the shared tsnet state dir.
pub type NodeSlot = Arc<Mutex<Option<Arc<Node>>>>;

pub type PushSlot = Arc<Mutex<Option<PushRegisterParams>>>;

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
    pub fn spawn(
        runtime: &tokio::runtime::Handle,
        machine: Machine,
        node: NodeSlot,
        push: PushSlot,
        reach: Arc<Reachability>,
    ) -> Self {
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
            push,
            reach,
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
    ) -> Result<Response, RequestError> {
        send(&self.requests, request, timeout).await
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

async fn send(
    requests: &mpsc::Sender<(Request, Reply)>,
    request: Request,
    timeout: Duration,
) -> Result<Response, RequestError> {
    let (tx, rx) = oneshot::channel();
    requests.try_send((request, tx)).map_err(|e| match e {
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
    push: PushSlot,
    reach: Arc<Reachability>,
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
        let opened = open(node, &machine.host, machine.port, &machine.node_id).await;
        if !matches!(opened, Err(ConnectError::Offline)) {
            reach.record(
                &machine.node_id,
                matches!(&opened, Ok((_, hello)) if hello.paired),
            );
        }
        let err = match opened {
            Ok((_, hello)) if !hello.paired => ConnectError::Session(SessionError::NotPaired),
            Ok((session, _)) => {
                lock(&shared.flock).new_connection();
                shared.set(LinkPhase::Connected, None);
                let since = Instant::now();
                reconnect.borrow_and_update();
                let push = lock(&push).clone();
                let end = session
                    .run(&mut requests, &shared.flock, push, async {
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

#[cfg(test)]
mod tests {
    use protocol::{AgentPromptParams, ErrorCode, OpId, PromptText, TerminalId};

    use super::*;

    fn prompt() -> Request {
        Request::AgentPrompt(AgentPromptParams {
            op_id: OpId::new("Zm9vYmFyYmF6cXV4cXV1dQ").unwrap(),
            terminal_id: TerminalId::new("term_1").unwrap(),
            text: PromptText::new("run the tests").unwrap(),
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
