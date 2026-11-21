use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use protocol::{
    AgentWatchParams, ClientFrame, Empty, ErrorBody, ErrorCode, Event, Flock, HelloParams,
    HelloResult, Label, Nonce, PROTOCOL_VERSION, ReadSource, Request, RequestId, Response,
    ServerFrame, TerminalId, TerminalRead, TerminalWatchParams, WS_PATH, WS_SUBPROTOCOL, is_format,
    limits,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{self, Message};

type StdInstant = std::time::Instant;

pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// collied pings every 15 s: three missed pings mean the machine is gone without a FIN.
const SILENCE_LIMIT: Duration = Duration::from_secs(45);
const MAX_MESSAGE_BYTES: usize = 4 << 20;
const MAX_REPLAY_EVENTS: usize = 1024;
/// Events are applied only while the queue takes them, so a machine can make the phone hold a
/// flock of one snapshot frame and this much in events, however many frames it sends.
const MAX_REPLAY_BYTES: usize = MAX_MESSAGE_BYTES;
const MAX_ERROR_CHARS: usize = 200;

#[derive(Debug, Clone, thiserror::Error)]
pub enum SessionError {
    #[error("connection closed")]
    Closed,
    #[error("timed out waiting for the machine")]
    Timeout,
    #[error("websocket: {0}")]
    WebSocket(String),
    #[error("the machine refused the connection (HTTP {0})")]
    Refused(u16),
    #[error("{message}")]
    Server { code: ErrorCode, message: String },
    #[error("the agent's input box has unsent text")]
    DraftChanged { current: String },
    #[error("unexpected reply from the machine: {0}")]
    Protocol(String),
    #[error("this phone is not paired with this machine")]
    NotPaired,
    #[error(
        "the machine does not accept this phone's key: run collied peers revoke for this phone there, then pair again"
    )]
    KeyRefused,
    #[error("{}", version_mismatch(*.collied, "the machine"))]
    VersionMismatch { collied: Option<u32> },
}

pub fn version_mismatch(collied: Option<u32>, machine: &str) -> String {
    match collied.map(|v| v.cmp(&PROTOCOL_VERSION)) {
        Some(std::cmp::Ordering::Less) => {
            format!(
                "Update collied on {machine}: install the latest release from github.com/rbstp/collie/releases."
            )
        }
        Some(std::cmp::Ordering::Greater) => "Update the app.".to_owned(),
        _ => format!("This app and collied on {machine} use different versions. Update both."),
    }
}

impl SessionError {
    /// Errors that a reconnect cannot fix: the supervisor stops instead of retrying.
    pub fn is_auth(&self) -> bool {
        match self {
            Self::NotPaired | Self::KeyRefused => true,
            Self::Refused(code) => matches!(code, 401 | 403),
            Self::Server { code, .. } => *code == ErrorCode::NotPaired,
            _ => false,
        }
    }

    /// The connection failed before collied answered: the request may or may not have run.
    pub fn is_transport(&self) -> bool {
        matches!(self, Self::Closed | Self::Timeout | Self::WebSocket(_))
    }
}

impl From<tungstenite::Error> for SessionError {
    fn from(e: tungstenite::Error) -> Self {
        match e {
            tungstenite::Error::Http(resp) => Self::Refused(resp.status().as_u16()),
            // TLS 1.3: collied checks the phone's key after the phone's side of the
            // handshake is done, so its refusal arrives on the first read.
            tungstenite::Error::Io(e) if key_refused(&e) => Self::KeyRefused,
            tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
                Self::Closed
            }
            other => Self::WebSocket(other.to_string()),
        }
    }
}

fn key_refused(e: &std::io::Error) -> bool {
    use collie_tls::rustls::{AlertDescription, Error};
    matches!(
        e.get_ref().and_then(|e| e.downcast_ref::<Error>()),
        Some(Error::AlertReceived(
            AlertDescription::AccessDenied | AlertDescription::BadCertificate
        ))
    )
}

impl From<ErrorBody> for SessionError {
    fn from(e: ErrorBody) -> Self {
        match (e.code, e.draft) {
            (ErrorCode::DraftChanged, Some(current)) => Self::DraftChanged { current },
            (ErrorCode::UnsupportedProtocol, _) => Self::VersionMismatch {
                collied: e.protocol_version,
            },
            _ => Self::Server {
                code: e.code,
                message: e.message,
            },
        }
    }
}

/// What the phone keeps of an error for a link's state or a background decision's outcome, both
/// of which the app writes to the device log: the text can be a machine's own, as long as a frame.
pub fn brief(error: &str) -> String {
    (error.chars())
        .filter(|c| !c.is_control() && !is_format(*c))
        .take(MAX_ERROR_CHARS)
        .collect()
}

pub type Reply = oneshot::Sender<Result<Response, SessionError>>;
pub type Seed = Vec<(Request, Option<Reply>)>;

pub struct Session<S> {
    ws: WebSocketStream<S>,
    next_id: RequestId,
    last_frame: Instant,
    protocol: Option<protocol::NegotiatedProtocol>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Session<S> {
    /// `host` only fills the Host header; the stream is already connected to the
    /// pinned node's address.
    pub async fn connect(stream: S, host: &str, port: u16) -> Result<Self, SessionError> {
        let mut request = format!("ws://{host}:{port}{WS_PATH}")
            .into_client_request()
            .map_err(SessionError::from)?;
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            HeaderValue::from_static(WS_SUBPROTOCOL),
        );
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_MESSAGE_BYTES));
        let (ws, _) = tokio::time::timeout(
            CALL_TIMEOUT,
            tokio_tungstenite::client_async_with_config(request, stream, Some(config)),
        )
        .await
        .map_err(|_| SessionError::Timeout)??;
        Ok(Self {
            ws,
            next_id: 1,
            last_frame: Instant::now(),
            protocol: None,
        })
    }

    pub async fn hello(&mut self) -> Result<HelloResult, SessionError> {
        let app_version = Label::new(env!("CARGO_PKG_VERSION")).expect("crate version is a label");
        match self
            .call(
                Request::Hello(HelloParams::current(app_version)),
                CALL_TIMEOUT,
            )
            .await?
        {
            Response::Hello(hello) => {
                let negotiated = protocol::NegotiatedProtocol::from_result(&hello)
                    .ok_or_else(|| SessionError::Protocol("invalid protocol negotiation".into()))?;
                self.protocol = Some(negotiated);
                Ok(hello)
            }
            other => Err(unexpected(&other)),
        }
    }

    /// Sequential call used before the session loop starts: events received while
    /// waiting are ignored because no snapshot has been taken yet.
    pub async fn call(
        &mut self,
        request: Request,
        timeout: Duration,
    ) -> Result<Response, SessionError> {
        tokio::time::timeout(timeout, async {
            let id = self.send(request).await?;
            loop {
                match self.recv().await?.0 {
                    ServerFrame::Result { id: rid, result } if rid == id => return Ok(result),
                    ServerFrame::Error {
                        id: Some(rid),
                        error,
                    } if rid == id => return Err(error.into()),
                    ServerFrame::Error { id: None, error } => return Err(error.into()),
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| SessionError::Timeout)?
    }

    async fn send(&mut self, request: Request) -> Result<RequestId, SessionError> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let frame = ClientFrame { id, request };
        let text = if let Some(protocol) = &self.protocol {
            protocol
                .client_frame(frame)
                .map_err(|message| SessionError::Server {
                    code: ErrorCode::NotImplemented,
                    message: message.to_owned(),
                })?
        } else if matches!(frame.request, Request::Hello(_)) {
            serde_json::to_string(&frame).map_err(|e| SessionError::Protocol(e.to_string()))?
        } else {
            return Err(SessionError::Protocol("hello first".into()));
        };
        self.ws.send(Message::text(text)).await?;
        Ok(id)
    }

    async fn recv(&mut self) -> Result<(ServerFrame, usize), SessionError> {
        loop {
            let message = self.ws.next().await.ok_or(SessionError::Closed)??;
            self.last_frame = Instant::now();
            match message {
                Message::Text(text) => {
                    // serde's own message quotes the frame, which the machine chose.
                    return serde_json::from_str(text.as_str())
                        .map(|frame| (frame, text.len()))
                        .map_err(|e| {
                            SessionError::Protocol(format!(
                                "{:?} error at line {} column {}",
                                e.classify(),
                                e.line(),
                                e.column()
                            ))
                        });
                }
                Message::Close(_) => return Err(SessionError::Closed),
                Message::Binary(_) => {
                    return Err(SessionError::Protocol("binary frame".into()));
                }
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }

    pub async fn close(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.ws.close(None)).await;
    }

    /// `push` is re-registered on every session so collied always has the current tokens;
    /// a request's reply, when it has one, gets collied's answer. The watch's first
    /// `agent.output` is the full screen, so no read follows it. When `stop` yields true the
    /// session sends a WebSocket close before dropping the socket.
    pub async fn run(
        mut self,
        requests: &mut tokio::sync::mpsc::Receiver<(Request, Reply)>,
        state: &Mutex<FlockState>,
        push: Seed,
        stop: impl Future<Output = bool>,
    ) -> SessionError {
        let mut pending: HashMap<RequestId, Pending> = HashMap::new();
        let mut seed = vec![(Request::FlockSnapshot(Empty {}), None)];
        seed.extend(push);
        // A shell's watch needs a grant, which no new session has.
        let watch = {
            let s = lock(state);
            s.watched
                .clone()
                .filter(|_| !s.watch_shell)
                .map(|terminal_id| (terminal_id, s.watch_lines, s.low_data))
        };
        if let Some((terminal_id, lines, low_data)) = watch {
            seed.push((
                Request::AgentWatch(AgentWatchParams {
                    terminal_id: Some(terminal_id),
                    lines,
                    low_data,
                }),
                None,
            ));
        }
        for (request, reply) in seed {
            let read_mark = read_mark(state, &request);
            match self.send(request).await {
                Ok(id) => pending.insert(id, (reply, read_mark)),
                Err(e) => return e,
            };
        }
        let silence = tokio::time::sleep_until(self.last_frame + SILENCE_LIMIT);
        tokio::pin!(stop, silence);
        let mut close = false;
        let end = loop {
            tokio::select! {
                c = &mut stop => {
                    close = c;
                    break SessionError::Closed;
                }
                () = &mut silence => {
                    let deadline = self.last_frame + SILENCE_LIMIT;
                    if Instant::now() >= deadline {
                        break SessionError::Timeout;
                    }
                    silence.as_mut().reset(deadline);
                }
                req = requests.recv() => {
                    let Some((request, reply)) = req else { break SessionError::Closed };
                    if reply.is_closed() {
                        continue;
                    }
                    let read_mark = read_mark(state, &request);
                    match self.send(request).await {
                        Ok(id) => { pending.insert(id, (Some(reply), read_mark)); }
                        Err(e) => {
                            let unsupported = matches!(e, SessionError::Server { code: ErrorCode::NotImplemented, .. });
                            let _ = reply.send(Err(e.clone()));
                            if !unsupported { break e; }
                        }
                    }
                }
                frame = self.recv() => {
                    let (frame, bytes) = match frame {
                        Ok(frame) => frame,
                        Err(e) => break e,
                    };
                    match frame {
                        ServerFrame::Result { id, result } => {
                            let (reply, read_mark) = pending.remove(&id).unwrap_or_default();
                            let behind = match &result {
                                Response::Flock(flock) => lock(state).apply_snapshot(flock.clone()),
                                Response::Terminal(read) if read_mark.is_some() => {
                                    lock(state).apply_read(read.clone(), read_mark);
                                    false
                                }
                                _ => false,
                            };
                            if let Some(reply) = reply {
                                let _ = reply.send(Ok(result));
                            }
                            if behind {
                                match self.send(Request::FlockSnapshot(Empty {})).await {
                                    Ok(id) => { pending.insert(id, (None, None)); }
                                    Err(e) => break e,
                                }
                            }
                        }
                        ServerFrame::Error { id: Some(id), error } => {
                            if error.code == ErrorCode::HerdrUnavailable {
                                lock(state).herdr_down = true;
                            }
                            let err = SessionError::from(error);
                            if err.is_auth() {
                                break err;
                            }
                            if let Some((Some(reply), _)) = pending.remove(&id) {
                                let _ = reply.send(Err(err));
                            }
                        }
                        ServerFrame::Error { id: None, error } => break error.into(),
                        ServerFrame::Event { seq, event } => {
                            let outcome = lock(state).apply_event(seq, event, bytes);
                            let request = match outcome {
                                EventOutcome::NeedsSnapshot => Request::FlockSnapshot(Empty {}),
                                EventOutcome::NeedsWatch(terminal_id) => {
                                    let state = lock(state);
                                    if state.watch_shell {
                                        Request::TerminalWatch(TerminalWatchParams {
                                            terminal_id,
                                            lines: state.watch_lines,
                                            low_data: state.low_data,
                                        })
                                    } else {
                                        Request::AgentWatch(AgentWatchParams {
                                            terminal_id: Some(terminal_id),
                                            lines: state.watch_lines,
                                            low_data: state.low_data,
                                        })
                                    }
                                }
                                EventOutcome::Dropped | EventOutcome::Applied => continue,
                            };
                            match self.send(request).await {
                                Ok(id) => { pending.insert(id, (None, None)); }
                                Err(e) => break e,
                            }
                        }
                    }
                }
            }
        };
        for reply in pending.into_values().filter_map(|(reply, _)| reply) {
            let _ = reply.send(Err(end.clone()));
        }
        if close {
            self.close().await;
        }
        end
    }
}

type Pending = (Option<Reply>, Option<u64>);

/// `None` for a read of another depth than the screen's watch: a preview's few lines must
/// not replace the watched agent's history.
fn read_mark(state: &Mutex<FlockState>, request: &Request) -> Option<u64> {
    let Request::AgentRead(p) = request else {
        return None;
    };
    let state = lock(state);
    let depth = state.watch_lines.unwrap_or(limits::DEFAULT_WATCH_LINES);
    (p.lines == Some(depth)).then_some(state.output_events)
}

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn unexpected(response: &Response) -> SessionError {
    let kind = serde_json::to_value(response)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
        .unwrap_or_default();
    SessionError::Protocol(kind)
}

pub fn expect_flock(response: Response) -> Result<Flock, SessionError> {
    match response {
        Response::Flock(flock) => Ok(flock),
        other => Err(unexpected(&other)),
    }
}

/// The name becomes the machine's label, which the app shows and logs.
pub fn expect_paired(response: Response) -> Result<protocol::MachineInfo, SessionError> {
    match response {
        Response::Paired { machine } => match Label::new(machine.name.as_str()) {
            Ok(_) => Ok(machine),
            Err(_) => Err(SessionError::Protocol("invalid machine name".into())),
        },
        other => Err(unexpected(&other)),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum EventOutcome {
    Dropped,
    Applied,
    NeedsSnapshot,
    /// A patch did not apply: a new watch starts with a full `agent.output`.
    NeedsWatch(TerminalId),
}

/// Flock as last seen, kept current by events. `seq` is per connection, so
/// [`FlockState::new_connection`] resets the cursor but keeps the last flock and an agent's
/// output for display. `watched` outlives connections so a new session re-issues the watch.
#[derive(Default)]
pub struct FlockState {
    pub flock: Option<Flock>,
    pub watched: Option<TerminalId>,
    /// `watched` is a shell pane, watched with `terminal.watch`.
    pub watch_shell: bool,
    pub watch_lines: Option<u16>,
    /// The phone is in Low Data Mode; every watch issued says so.
    pub low_data: bool,
    /// The challenge collied gave this session, for the grant that answers it.
    pub challenge: Option<(TerminalId, Nonce)>,
    pub unlocked: HashMap<TerminalId, StdInstant>,
    /// Read through [`FlockState::screen`], which serves a shell's only under its grant.
    output: Option<TerminalRead>,
    pub output_revision: u64,
    /// A request on this session found herdr not running, and no snapshot has worked since.
    pub herdr_down: bool,
    /// The text the watch sent last, the base of its next patch. A read reply can replace
    /// `output` but not this.
    watched_output: Option<TerminalRead>,
    output_events: u64,
    last_seq: u64,
    since_snapshot: VecDeque<(u64, usize, Event)>,
    replay_bytes: usize,
    /// The newest event the full queue left out: only a snapshot at or past it is current.
    skipped: u64,
}

impl FlockState {
    /// Grants are per session on the machine, so a new one starts locked.
    pub fn new_connection(&mut self) {
        self.last_seq = 0;
        self.since_snapshot.clear();
        self.replay_bytes = 0;
        self.skipped = 0;
        self.herdr_down = false;
        self.watched_output = None;
        self.lock_terminals();
    }

    pub fn lock_terminals(&mut self) {
        self.challenge = None;
        self.end_grants();
    }

    /// Also for a grant collied no longer holds, which leaves the challenge of an unlock in
    /// progress alone.
    pub fn end_grants(&mut self) {
        self.unlocked.clear();
        self.drop_ungranted(StdInstant::now());
    }

    /// A shell's screen is read under a grant, so it is not kept past it.
    fn drop_ungranted(&mut self, now: StdInstant) {
        let granted = self
            .watched
            .as_ref()
            .is_some_and(|t| self.unlocked(t.as_str(), now));
        if self.watch_shell && !granted {
            self.output = None;
            self.watched_output = None;
        }
    }

    /// The watched screen of `terminal_id`. A grant that ran out drops its shell's screen
    /// here, as a lock does, so no view gets one without a live grant.
    pub fn screen(&mut self, terminal_id: &str, now: StdInstant) -> Option<&TerminalRead> {
        self.drop_ungranted(now);
        self.output
            .as_ref()
            .filter(|o| o.terminal_id.as_str() == terminal_id)
    }

    pub fn unlocked(&self, terminal_id: &str, now: StdInstant) -> bool {
        self.unlocked
            .iter()
            .any(|(t, until)| t.as_str() == terminal_id && now < *until)
    }

    pub fn watch(&mut self, terminal_id: Option<TerminalId>) {
        // A shell's screen never becomes an agent's: a screen opened on a locked shell watches
        // the same terminal as an agent.
        if self.watched != terminal_id || self.watch_shell {
            self.output = None;
            self.watched_output = None;
        }
        self.watched = terminal_id;
        self.watch_shell = false;
    }

    pub fn watch_terminal(&mut self, terminal_id: TerminalId) {
        self.watch(Some(terminal_id));
        self.watch_shell = true;
    }

    /// Only `recent` reads of the watched agent are kept: that is what `agent.output`
    /// carries, so a `visible` read would flicker against the stream. A shell's are kept only
    /// under its grant: one collied sent before a lock can arrive after it.
    fn apply_output(&mut self, read: TerminalRead) -> bool {
        let watched = read.source == ReadSource::Recent
            && self.watched.as_ref() == Some(&read.terminal_id)
            && (!self.watch_shell || self.unlocked(read.terminal_id.as_str(), StdInstant::now()));
        if watched {
            self.output = Some(read);
            self.output_revision += 1;
        }
        watched
    }

    fn apply_watched(&mut self, read: TerminalRead) {
        if self.apply_output(read.clone()) {
            self.watched_output = Some(read);
            self.output_events += 1;
        }
    }

    /// A read reply is not ordered against `agent.output` events: collied may have read
    /// herdr before an event it sent first. `mark` is the event count when the read was
    /// sent; if an event was applied since, the reply is older than the screen.
    pub fn apply_read(&mut self, read: TerminalRead, mark: Option<u64>) {
        if mark.is_none_or(|m| m == self.output_events) {
            self.apply_output(read);
        }
    }

    /// `bytes` is the size of the event's frame.
    pub fn apply_event(&mut self, seq: u64, event: Event, bytes: usize) -> EventOutcome {
        if seq <= self.last_seq {
            return EventOutcome::Dropped;
        }
        self.last_seq = seq;
        match event {
            Event::AgentOutput(read) => self.apply_watched(read),
            Event::AgentOutputPatch(patch) => {
                if self.watched.as_ref() != Some(&patch.terminal_id) {
                    return EventOutcome::Applied;
                }
                let read = self
                    .watched_output
                    .as_ref()
                    .filter(|o| o.terminal_id == patch.terminal_id)
                    .and_then(|o| patch.apply(o));
                match read {
                    Some(read) => self.apply_watched(read),
                    None => {
                        self.watched_output = None;
                        return EventOutcome::NeedsWatch(patch.terminal_id);
                    }
                }
            }
            Event::FlockChanged {} => return EventOutcome::NeedsSnapshot,
            Event::Unrecognized => {}
            event => {
                if self.since_snapshot.len() == MAX_REPLAY_EVENTS
                    || self.replay_bytes + bytes > MAX_REPLAY_BYTES
                {
                    self.skipped = seq;
                    return EventOutcome::NeedsSnapshot;
                }
                if let Some(flock) = &mut self.flock {
                    apply(flock, &event);
                }
                self.replay_bytes += bytes;
                self.since_snapshot.push_back((seq, bytes, event));
            }
        }
        EventOutcome::Applied
    }

    /// A snapshot answered after newer events were applied must not erase them, so
    /// events past `snapshot.seq` are replayed onto it. True when the full queue left out an
    /// event newer than the snapshot, which then needs another.
    pub fn apply_snapshot(&mut self, mut snapshot: Flock) -> bool {
        // The app keys its lists on the terminal id, and events update the first match.
        let mut seen = HashSet::new();
        snapshot
            .agents
            .retain(|a| seen.insert(a.terminal_id.clone()));
        self.since_snapshot
            .retain(|(seq, _, _)| *seq > snapshot.seq);
        self.replay_bytes = self.since_snapshot.iter().map(|(_, bytes, _)| bytes).sum();
        for (_, _, event) in &self.since_snapshot {
            apply(&mut snapshot, event);
        }
        self.last_seq = self.last_seq.max(snapshot.seq);
        let behind = self.skipped > snapshot.seq;
        if !behind {
            self.skipped = 0;
        }
        self.flock = Some(snapshot);
        self.herdr_down = false;
        behind
    }

    /// From a one-shot listing while this connection is closed, so the app does not
    /// publish the older plan it froze with before its next snapshot.
    pub fn listed_plan(&mut self, plan: &protocol::PlanUsage) {
        if let Some(flock) = &mut self.flock {
            let Some(current) = &mut flock.plan_usage else {
                flock.plan_usage = Some(plan.clone());
                return;
            };
            if current.recorded_ms < plan.recorded_ms {
                current.five_hour = plan.five_hour.clone();
                current.seven_day = plan.seven_day.clone();
                current.recorded_ms = plan.recorded_ms;
            }
            if let Some(codex) = &plan.codex
                && current
                    .codex
                    .as_ref()
                    .is_none_or(|old| old.recorded_ms < codex.recorded_ms)
            {
                current.codex = Some(codex.clone());
            }
            if let Some(copilot) = &plan.copilot
                && current
                    .copilot
                    .as_ref()
                    .is_none_or(|old| old.recorded_ms < copilot.recorded_ms)
            {
                current.copilot = Some(copilot.clone());
            }
            if let Some(agy) = &plan.agy
                && current
                    .agy
                    .as_ref()
                    .is_none_or(|old| old.recorded_ms < agy.recorded_ms)
            {
                current.agy = Some(agy.clone());
            }
        }
    }
}

fn apply(flock: &mut Flock, event: &Event) {
    match event {
        Event::AgentStatus { agent } => {
            match flock
                .agents
                .iter_mut()
                .find(|a| a.terminal_id == agent.terminal_id)
            {
                Some(slot) => *slot = agent.clone(),
                None => flock.agents.push(agent.clone()),
            }
        }
        Event::ApprovalNeeded { approval } => {
            flock
                .approvals
                .retain(|a| a.approval_id != approval.approval_id);
            flock.approvals.push(approval.clone());
        }
        Event::ApprovalResolved { approval_id, .. } => {
            flock.approvals.retain(|a| a.approval_id != *approval_id);
        }
        Event::PlanUsage { plan_usage } => flock.plan_usage = Some(plan_usage.clone()),
        Event::AgentOutput(_)
        | Event::AgentOutputPatch(_)
        | Event::FlockChanged {}
        | Event::Unrecognized => {}
    }
}

#[cfg(test)]
mod tests {
    use protocol::{
        Agent, AgentStatus, ApprovalId, ClientFrame, MachineInfo, OutputPatch, TerminalId,
        WorkspaceId, parse_client_frame,
    };
    use tokio::net::UnixStream;
    use tokio::sync::mpsc;
    use tokio_tungstenite::tungstenite::handshake::server::{
        Request as HsRequest, Response as HsResponse,
    };

    use super::*;

    fn agent(id: &str, status: AgentStatus) -> Agent {
        Agent {
            terminal_id: TerminalId::new(id).unwrap(),
            workspace_id: WorkspaceId::new("w1").unwrap(),
            kind: Some("claude".into()),
            name: None,
            title: Some(format!("{id} title")),
            status,
            status_since_ms: 1,
            cwd: None,
            last_line: None,
            context_left: None,
            last_prompt: None,
            last_prompt_origin: None,
            last_activity_ms: None,
            subagents: Vec::new(),
            pull_request: None,
            pr_watch: None,
            runtime: None,
        }
    }

    fn machine() -> MachineInfo {
        MachineInfo {
            name: "mac".into(),
            node_id: "nMAC".into(),
            herdr_session: "default".into(),
        }
    }

    fn flock(seq: u64, agents: Vec<Agent>) -> Flock {
        Flock {
            seq,
            machine: machine(),
            workspaces: Vec::new(),
            agents,
            approvals: Vec::new(),
            terminals: Vec::new(),
            terminals_enabled: false,
            plan_usage: None,
            starred: Vec::new(),
        }
    }

    fn status_of(state: &FlockState, id: &str) -> Option<AgentStatus> {
        state
            .flock
            .as_ref()?
            .agents
            .iter()
            .find(|a| a.terminal_id.as_str() == id)
            .map(|a| a.status)
    }

    fn status_event(id: &str, status: AgentStatus) -> Event {
        Event::AgentStatus {
            agent: agent(id, status),
        }
    }

    #[test]
    fn events_at_or_below_cursor_are_dropped() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(5, vec![agent("t1", AgentStatus::Idle)]));
        assert_eq!(
            s.apply_event(5, status_event("t1", AgentStatus::Blocked), 0),
            EventOutcome::Dropped
        );
        assert_eq!(
            s.apply_event(3, status_event("t1", AgentStatus::Blocked), 0),
            EventOutcome::Dropped
        );
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Idle));
        assert_eq!(
            s.apply_event(6, status_event("t1", AgentStatus::Working), 0),
            EventOutcome::Applied
        );
        assert_eq!(
            s.apply_event(6, status_event("t1", AgentStatus::Done), 0),
            EventOutcome::Dropped
        );
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Working));
        assert_eq!(
            s.apply_event(7, status_event("t2", AgentStatus::Blocked), 0),
            EventOutcome::Applied
        );
        assert_eq!(status_of(&s, "t2"), Some(AgentStatus::Blocked));
    }

    #[test]
    fn unknown_events_advance_the_cursor() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(1, vec![]));
        assert_eq!(
            s.apply_event(2, Event::Unrecognized, 0),
            EventOutcome::Applied
        );
        assert_eq!(
            s.apply_event(2, status_event("t1", AgentStatus::Idle), 0),
            EventOutcome::Dropped
        );
        assert_eq!(
            s.apply_event(3, Event::FlockChanged {}, 0),
            EventOutcome::NeedsSnapshot
        );
    }

    #[test]
    fn a_snapshot_lists_a_terminal_id_once() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(
            1,
            vec![
                agent("t1", AgentStatus::Idle),
                agent("t2", AgentStatus::Idle),
                agent("t1", AgentStatus::Blocked),
            ],
        ));
        let ids: Vec<_> = (s.flock.iter().flat_map(|f| &f.agents))
            .map(|a| a.terminal_id.as_str())
            .collect();
        assert_eq!(ids, ["t1", "t2"]);
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Idle));
    }

    #[test]
    fn events_past_the_replay_queue_wait_for_a_snapshot() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(0, vec![]));
        let status = |seq: u64| status_event(&format!("t{seq}"), AgentStatus::Idle);
        let agents = |s: &FlockState| s.flock.as_ref().unwrap().agents.len();
        for seq in 1..=2000 {
            let asks = s.apply_event(seq, status(seq), 0) == EventOutcome::NeedsSnapshot;
            assert_eq!(asks, seq > MAX_REPLAY_EVENTS as u64, "{seq}");
        }
        assert_eq!(agents(&s), MAX_REPLAY_EVENTS);
        assert!(s.apply_snapshot(flock(1999, vec![])), "event 2000 is newer");
        assert!(!s.apply_snapshot(flock(2000, vec![agent("t2000", AgentStatus::Idle)])));
        assert_eq!(agents(&s), 1);
        assert_eq!(s.apply_event(2001, status(2001), 0), EventOutcome::Applied);
    }

    #[test]
    fn the_replay_queue_is_bounded_in_bytes() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(0, vec![]));
        let status = |seq: u64| status_event(&format!("t{seq}"), AgentStatus::Idle);
        let half = MAX_REPLAY_BYTES / 2;
        assert_eq!(s.apply_event(1, status(1), half), EventOutcome::Applied);
        assert_eq!(s.apply_event(2, status(2), half), EventOutcome::Applied);
        assert_eq!(s.apply_event(3, status(3), 1), EventOutcome::NeedsSnapshot);
        assert_eq!(status_of(&s, "t3"), None);
        assert!(!s.apply_snapshot(flock(3, vec![])));
        assert_eq!(
            s.apply_event(4, status(4), MAX_REPLAY_BYTES),
            EventOutcome::Applied
        );
    }

    #[test]
    fn a_paired_reply_needs_a_name_that_is_a_label() {
        let paired = |name: &str| {
            expect_paired(Response::Paired {
                machine: MachineInfo {
                    name: name.into(),
                    ..machine()
                },
            })
        };
        assert_eq!(paired("Mac mini").unwrap().name, "Mac mini");
        for name in ["", "mac\u{202e}", "mac\n", &"m".repeat(65)] {
            assert!(matches!(paired(name), Err(SessionError::Protocol(_))));
        }
    }

    #[test]
    fn late_snapshot_keeps_newer_events() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(1, vec![agent("t1", AgentStatus::Idle)]));
        s.apply_event(2, status_event("t1", AgentStatus::Working), 0);
        s.apply_event(3, status_event("t1", AgentStatus::Blocked), 0);
        s.apply_snapshot(flock(2, vec![agent("t1", AgentStatus::Working)]));
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Blocked));
        s.apply_snapshot(flock(9, vec![agent("t1", AgentStatus::Done)]));
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Done));
        assert_eq!(
            s.apply_event(9, status_event("t1", AgentStatus::Idle), 0),
            EventOutcome::Dropped
        );
    }

    #[test]
    fn plan_usage_event_survives_a_late_snapshot() {
        let plan = |used_percent| protocol::PlanUsage {
            five_hour: Some(protocol::UsageWindow {
                used_percent,
                resets_at_ms: 9,
            }),
            seven_day: None,
            recorded_ms: 1,
            codex: None,
            copilot: None,
            agy: None,
        };
        let with = |seq, used| Flock {
            plan_usage: Some(plan(used)),
            ..flock(seq, vec![])
        };
        let used = |s: &FlockState| s.flock.as_ref()?.plan_usage.clone();
        let mut s = FlockState::default();
        s.apply_snapshot(flock(1, vec![]));
        let event = Event::PlanUsage {
            plan_usage: plan(30),
        };
        assert_eq!(s.apply_event(2, event, 0), EventOutcome::Applied);
        s.apply_snapshot(with(1, 10));
        assert_eq!(used(&s), Some(plan(30)));
        s.apply_snapshot(with(5, 40));
        assert_eq!(used(&s), Some(plan(40)));
    }

    #[test]
    fn listed_plan_replaces_only_an_older_one() {
        let plan = |used_percent, recorded_ms| protocol::PlanUsage {
            five_hour: Some(protocol::UsageWindow {
                used_percent,
                resets_at_ms: 9,
            }),
            seven_day: None,
            recorded_ms,
            codex: None,
            copilot: None,
            agy: None,
        };
        let used = |s: &FlockState| s.flock.as_ref()?.plan_usage.clone();
        let mut s = FlockState::default();
        s.listed_plan(&plan(10, 1));
        assert!(s.flock.is_none());
        s.apply_snapshot(flock(1, vec![]));
        s.listed_plan(&plan(30, 2));
        assert_eq!(used(&s), Some(plan(30, 2)));
        s.listed_plan(&plan(20, 1));
        assert_eq!(used(&s), Some(plan(30, 2)));
    }

    #[test]
    fn listed_plan_merges_codex_and_claude_by_source() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(1, vec![]));
        let mut codex = protocol::PlanUsage {
            five_hour: None,
            seven_day: None,
            recorded_ms: 0,
            codex: Some(protocol::CodexUsage {
                used: 100,
                limit: 1_000,
                resets_at_ms: 10_000,
                recorded_ms: 20,
            }),
            copilot: None,
            agy: None,
        };
        s.listed_plan(&codex);
        let mut claude = codex.clone();
        claude.codex = None;
        claude.five_hour = Some(protocol::UsageWindow {
            used_percent: 30,
            resets_at_ms: 10_000,
        });
        claude.recorded_ms = 10;
        s.listed_plan(&claude);
        let plan = s.flock.as_ref().unwrap().plan_usage.as_ref().unwrap();
        assert_eq!(plan.five_hour, claude.five_hour);
        assert_eq!(plan.codex, codex.codex);
        codex.codex.as_mut().unwrap().used = 200;
        codex.codex.as_mut().unwrap().recorded_ms = 30;
        s.listed_plan(&codex);
        let plan = s.flock.as_ref().unwrap().plan_usage.as_ref().unwrap();
        assert_eq!(plan.five_hour, claude.five_hour);
        assert_eq!(plan.codex, codex.codex);
        let mut copilot = claude.clone();
        copilot.copilot = Some(protocol::CopilotUsage {
            used: 100,
            limit: 300,
            unlimited: false,
            token_based_billing: false,
            resets_at_ms: 10_000,
            recorded_ms: 40,
        });
        s.listed_plan(&copilot);
        let plan = s.flock.as_ref().unwrap().plan_usage.as_ref().unwrap();
        assert_eq!(plan.codex, codex.codex);
        assert_eq!(plan.copilot, copilot.copilot);
    }

    #[test]
    fn codex_plan_removal_reaches_the_next_snapshot() {
        let mut s = FlockState::default();
        let mut with_codex = flock(1, vec![]);
        with_codex.plan_usage = Some(protocol::PlanUsage {
            five_hour: None,
            seven_day: None,
            recorded_ms: 0,
            codex: Some(protocol::CodexUsage {
                used: 100,
                limit: 1_000,
                resets_at_ms: 10_000,
                recorded_ms: 20,
            }),
            copilot: None,
            agy: None,
        });
        s.apply_snapshot(with_codex);
        assert_eq!(
            s.apply_event(2, Event::FlockChanged {}, 0),
            EventOutcome::NeedsSnapshot
        );
        s.apply_snapshot(flock(2, vec![]));
        assert!(s.flock.as_ref().unwrap().plan_usage.is_none());
    }

    #[test]
    fn new_connection_resets_cursor_but_keeps_flock() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(40, vec![agent("t1", AgentStatus::Idle)]));
        s.new_connection();
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Idle));
        assert_eq!(
            s.apply_event(1, status_event("t1", AgentStatus::Working), 0),
            EventOutcome::Applied
        );
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Working));
    }

    #[test]
    fn approval_events() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(0, vec![]));
        let approval: protocol::Approval = serde_json::from_value(serde_json::json!({
            "approval_id": "a1", "terminal_id": "t1", "agent_label": "claude",
            "workspace_label": "w", "snippet": "run?", "tool": null,
            "options": ["approve", "deny"], "nonce": "A".repeat(43),
            "created_at_ms": 1, "expires_at_ms": 2
        }))
        .unwrap();
        s.apply_event(
            1,
            Event::ApprovalNeeded {
                approval: approval.clone(),
            },
            0,
        );
        s.apply_event(2, Event::ApprovalNeeded { approval }, 0);
        assert_eq!(s.flock.as_ref().unwrap().approvals.len(), 1);
        s.apply_event(
            3,
            Event::ApprovalResolved {
                approval_id: ApprovalId::new("a1").unwrap(),
                outcome: protocol::ApprovalOutcome::Expired,
            },
            0,
        );
        assert!(s.flock.as_ref().unwrap().approvals.is_empty());
    }

    #[test]
    fn read_reply_older_than_an_applied_event_is_dropped() {
        let mut s = FlockState::default();
        s.watch(Some(TerminalId::new("t1").unwrap()));
        let ev = |ansi| Event::AgentOutput(output("t1", ansi, ReadSource::Recent));
        let mark = s.output_events;
        s.apply_event(1, ev("newer"), 0);
        s.apply_read(output("t1", "older", ReadSource::Recent), Some(mark));
        assert_eq!((shown(&s), s.output_revision), (Some("newer"), 1));
        s.apply_read(
            output("t1", "fresh", ReadSource::Recent),
            Some(s.output_events),
        );
        assert_eq!((shown(&s), s.output_revision), (Some("fresh"), 2));
        let mark = s.output_events;
        s.apply_event(
            2,
            Event::AgentOutput(output("t2", "x", ReadSource::Recent)),
            0,
        );
        s.apply_read(output("t1", "still fresh", ReadSource::Recent), Some(mark));
        assert_eq!(
            shown(&s),
            Some("still fresh"),
            "an unwatched agent's event does not count"
        );
    }

    #[test]
    fn patches_build_on_the_last_watched_output() {
        let mut s = FlockState::default();
        let t1 = TerminalId::new("t1").unwrap();
        s.watch(Some(t1.clone()));
        let history = "\u{1b}[1mline\u{1b}[0m\r\n".repeat(50);
        let text = |end: &str| output("t1", &format!("{history}{end}"), ReadSource::Recent);
        let patch = |prev: &TerminalRead, next: &TerminalRead| {
            Event::AgentOutputPatch(OutputPatch::between(prev, next).unwrap())
        };
        let b = TerminalRead {
            wraps: vec![50],
            ..text("b")
        };
        let (a, c) = (text("a"), text("c"));

        assert_eq!(
            s.apply_event(1, patch(&a, &b), 0),
            EventOutcome::NeedsWatch(t1.clone())
        );
        assert_eq!(shown(&s), None);
        s.apply_event(2, Event::AgentOutput(a.clone()), 0);
        let mark = s.output_events;
        assert_eq!(s.apply_event(3, patch(&a, &b), 0), EventOutcome::Applied);
        assert_eq!((shown(&s), s.output_revision), (Some(b.ansi.as_str()), 2));
        assert_eq!(s.output.as_ref().unwrap().wraps, [50]);
        assert_eq!(s.output_events, mark + 1, "a patch is an output event");

        s.apply_read(output("t1", "read", ReadSource::Recent), None);
        assert_eq!(s.apply_event(4, patch(&b, &c), 0), EventOutcome::Applied);
        assert_eq!(
            shown(&s),
            Some(c.ansi.as_str()),
            "a read reply is not the base"
        );

        assert_eq!(
            s.apply_event(5, patch(&a, &b), 0),
            EventOutcome::NeedsWatch(t1.clone())
        );
        assert_eq!((shown(&s), s.output_revision), (Some(c.ansi.as_str()), 4));
        assert_eq!(
            s.apply_event(6, patch(&c, &b), 0),
            EventOutcome::NeedsWatch(t1.clone()),
            "nothing to patch until the next full output"
        );
        s.apply_event(7, Event::AgentOutput(c.clone()), 0);
        assert_eq!(s.apply_event(8, patch(&c, &a), 0), EventOutcome::Applied);
        assert_eq!(shown(&s), Some(a.ansi.as_str()));

        let other = OutputPatch {
            terminal_id: TerminalId::new("t2").unwrap(),
            ..OutputPatch::between(&a, &b).unwrap()
        };
        assert_eq!(
            s.apply_event(9, Event::AgentOutputPatch(other), 0),
            EventOutcome::Applied
        );
        assert_eq!(shown(&s), Some(a.ansi.as_str()));

        s.new_connection();
        assert_eq!(
            s.apply_event(1, patch(&a, &b), 0),
            EventOutcome::NeedsWatch(t1)
        );
        assert_eq!(
            shown(&s),
            Some(a.ansi.as_str()),
            "kept on screen while reconnecting"
        );
    }

    #[test]
    fn locking_drops_a_shell_screen_and_keeps_an_agent_screen() {
        let t1 = TerminalId::new("t1").unwrap();
        let history = "line\r\n".repeat(50);
        let text = |end: &str| output("t1", &format!("{history}{end}"), ReadSource::Recent);
        let (a, b) = (text("a"), text("b"));
        let patch = Event::AgentOutputPatch(OutputPatch::between(&a, &b).unwrap());
        let until = StdInstant::now() + Duration::from_secs(60);

        let mut s = FlockState::default();
        s.watch_terminal(t1.clone());
        s.unlocked.insert(t1.clone(), until);
        s.apply_event(1, Event::AgentOutput(a.clone()), 0);
        assert_eq!(shown(&s), Some(a.ansi.as_str()));
        s.lock_terminals();
        assert_eq!(shown(&s), None, "a shell's screen is behind its grant");
        s.apply_event(2, patch.clone(), 0);
        s.apply_event(3, Event::AgentOutput(b.clone()), 0);
        assert_eq!(
            shown(&s),
            None,
            "output sent before the lock arrives after it"
        );

        s.unlocked.insert(t1.clone(), until);
        s.apply_event(4, Event::AgentOutput(a.clone()), 0);
        assert_eq!(shown(&s), Some(a.ansi.as_str()));
        s.new_connection();
        assert_eq!(shown(&s), None, "a new session starts locked");

        s.watch(Some(t1));
        s.apply_event(1, Event::AgentOutput(a.clone()), 0);
        s.lock_terminals();
        assert_eq!(shown(&s), Some(a.ansi.as_str()), "an agent needs no grant");
        assert_eq!(s.apply_event(2, patch, 0), EventOutcome::Applied);
        assert_eq!(shown(&s), Some(b.ansi.as_str()));
    }

    #[test]
    fn a_shell_screen_is_served_only_under_a_live_grant() {
        let t1 = TerminalId::new("t1").unwrap();
        let now = StdInstant::now();
        let end = now + Duration::from_secs(300);
        let granted = || {
            let mut s = FlockState::default();
            s.watch_terminal(t1.clone());
            s.unlocked.insert(t1.clone(), end);
            let read = output("t1", "$ cat .env", ReadSource::Recent);
            s.apply_event(1, Event::AgentOutput(read), 0);
            s
        };
        let screen = |s: &mut FlockState, at| s.screen("t1", at).map(|o| o.ansi.clone());

        let mut s = granted();
        assert_eq!(screen(&mut s, now).as_deref(), Some("$ cat .env"));
        assert_eq!(screen(&mut s, end), None, "the grant ran out");
        assert_eq!(screen(&mut s, now), None, "dropped, not hidden");

        let mut s = granted();
        s.end_grants();
        assert!(
            s.output.is_none() && s.watched_output.is_none(),
            "collied dropped the grant: not kept until the next view"
        );

        let mut s = granted();
        s.watch(Some(t1.clone()));
        assert_eq!(
            screen(&mut s, now),
            None,
            "a screen opened on the shell watches it as an agent"
        );

        let mut s = FlockState::default();
        s.watch(Some(t1));
        s.apply_event(
            1,
            Event::AgentOutput(output("t1", "agent", ReadSource::Recent)),
            0,
        );
        s.end_grants();
        assert_eq!(
            screen(&mut s, end).as_deref(),
            Some("agent"),
            "an agent needs no grant"
        );
    }

    fn output(id: &str, ansi: &str, source: ReadSource) -> TerminalRead {
        TerminalRead {
            terminal_id: TerminalId::new(id).unwrap(),
            source,
            ansi: ansi.into(),
            truncated: false,
            wraps: Vec::new(),
            splits: Vec::new(),
            copilot_scrollbar: false,
            suggestion: None,
        }
    }

    fn shown(s: &FlockState) -> Option<&str> {
        s.output.as_ref().map(|o| o.ansi.as_str())
    }

    #[test]
    fn output_events_follow_seq_and_the_watched_agent() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(1, vec![agent("t1", AgentStatus::Working)]));
        let ev = |id, ansi| Event::AgentOutput(output(id, ansi, ReadSource::Recent));
        assert_eq!(
            s.apply_event(2, ev("t1", "unwatched"), 0),
            EventOutcome::Applied
        );
        assert_eq!(shown(&s), None);

        s.watch(Some(TerminalId::new("t1").unwrap()));
        s.apply_event(3, ev("t1", "a"), 0);
        assert_eq!((shown(&s), s.output_revision), (Some("a"), 1));
        assert_eq!(s.apply_event(3, ev("t1", "dup"), 0), EventOutcome::Dropped);
        assert_eq!(s.apply_event(2, ev("t1", "old"), 0), EventOutcome::Dropped);
        s.apply_event(4, ev("t2", "other agent"), 0);
        s.apply_read(output("t1", "visible", ReadSource::Visible), None);
        assert_eq!((shown(&s), s.output_revision), (Some("a"), 1));
        s.apply_event(5, ev("t1", "b"), 0);
        assert_eq!((shown(&s), s.output_revision), (Some("b"), 2));
        assert!(
            s.since_snapshot
                .iter()
                .all(|(_, _, e)| !matches!(e, Event::AgentOutput(_)))
        );

        s.new_connection();
        assert_eq!(shown(&s), Some("b"), "kept on screen while reconnecting");
        s.apply_event(1, ev("t1", "c"), 0);
        assert_eq!((shown(&s), s.output_revision), (Some("c"), 3));

        s.watch(Some(TerminalId::new("t1").unwrap()));
        assert_eq!(shown(&s), Some("c"));
        s.watch(Some(TerminalId::new("t2").unwrap()));
        assert_eq!(shown(&s), None);
        s.apply_event(2, ev("t2", "d"), 0);
        assert_eq!((shown(&s), s.output_revision), (Some("d"), 4));
        s.watch(None);
        assert_eq!(shown(&s), None);
    }

    type ServerWs = WebSocketStream<UnixStream>;

    // The handshake callback's error type is fixed by tungstenite.
    #[allow(clippy::result_large_err)]
    async fn pair_of(
        subprotocol: Option<&'static str>,
    ) -> (Result<Session<UnixStream>, SessionError>, Option<ServerWs>) {
        let (client, server) = UnixStream::pair().unwrap();
        let server = tokio::spawn(async move {
            tokio_tungstenite::accept_hdr_async(
                server,
                move |req: &HsRequest, mut resp: HsResponse| {
                    assert_eq!(req.uri().path(), WS_PATH);
                    assert_eq!(req.headers()["Host"], "mac.tail1234.ts.net:8457");
                    assert_eq!(req.headers()["Sec-WebSocket-Protocol"], WS_SUBPROTOCOL);
                    if let Some(p) = subprotocol {
                        resp.headers_mut()
                            .insert("Sec-WebSocket-Protocol", HeaderValue::from_static(p));
                    }
                    Ok(resp)
                },
            )
            .await
            .ok()
        });
        let session = Session::connect(client, "mac.tail1234.ts.net", 8457)
            .await
            .map(|mut session| {
                session.protocol = protocol::NegotiatedProtocol::negotiate(&HelloParams::current(
                    Label::new("test").unwrap(),
                ));
                session
            });
        (session, server.await.unwrap())
    }

    async fn read(ws: &mut ServerWs) -> ClientFrame {
        loop {
            match ws.next().await.unwrap().unwrap() {
                Message::Text(t) => return parse_client_frame(t.as_bytes()).unwrap(),
                Message::Close(_) => panic!("closed"),
                _ => {}
            }
        }
    }

    async fn write(ws: &mut ServerWs, frame: ServerFrame) {
        ws.send(Message::text(serde_json::to_string(&frame).unwrap()))
            .await
            .unwrap();
    }

    fn hello_result(paired: bool) -> Response {
        Response::Hello(HelloResult {
            push_mode: None,
            protocol_version: PROTOCOL_VERSION,
            protocol_min_version: protocol::PROTOCOL_MIN_VERSION,
            protocol_max_version: PROTOCOL_VERSION,
            capabilities: HelloParams::current(Label::new("test").unwrap()).capabilities,
            collied_version: "0.1.0".into(),
            machine: machine(),
            herdr_version: None,
            paired,
        })
    }

    #[tokio::test]
    async fn handshake_requires_the_subprotocol() {
        let (session, _server) = pair_of(None).await;
        assert!(matches!(session, Err(SessionError::WebSocket(_))));
        let (session, _server) = pair_of(Some("collie.v2")).await;
        assert!(matches!(session, Err(SessionError::WebSocket(_))));
    }

    #[tokio::test]
    async fn negotiated_terminal_opt_out_keeps_ordinary_calls_usable() {
        for maximum in [26, 27, 28, 29, 30, 31, 32, 33] {
            let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
            let (mut session, mut ws) = (session.unwrap(), server.unwrap());
            let server = tokio::spawn(async move {
                let hello = read(&mut ws).await;
                let Response::Hello(mut result) = hello_result(true) else {
                    unreachable!()
                };
                result.protocol_version = maximum.min(32);
                result.protocol_max_version = maximum;
                result.capabilities.clear();
                write(
                    &mut ws,
                    ServerFrame::Result {
                        id: hello.id,
                        result: Response::Hello(result),
                    },
                )
                .await;
                let ordinary = read(&mut ws).await;
                assert!(matches!(ordinary.request, Request::FlockSnapshot(_)));
                write(
                    &mut ws,
                    ServerFrame::Result {
                        id: ordinary.id,
                        result: Response::Ok,
                    },
                )
                .await;
            });
            session.hello().await.unwrap();
            let error = session
                .call(
                    Request::TerminalChallenge(protocol::AgentTarget {
                        terminal_id: TerminalId::new("t1").unwrap(),
                    }),
                    CALL_TIMEOUT,
                )
                .await
                .unwrap_err();
            assert!(matches!(
                error,
                SessionError::Server {
                    code: ErrorCode::NotImplemented,
                    ..
                }
            ));
            if maximum < 28 {
                let costs = Request::UsageCosts(protocol::UsageCostsParams {
                    range: protocol::CostRange::Today,
                    utc_offset_minutes: 0,
                });
                assert!(matches!(
                    session.call(costs, CALL_TIMEOUT).await,
                    Err(SessionError::Server {
                        code: ErrorCode::NotImplemented,
                        ..
                    })
                ));
            }
            if maximum < 29 {
                let open = Request::FileOpen(protocol::FileOpenParams {
                    terminal_id: protocol::TerminalId::new("term_1").unwrap(),
                    path: "a.png".into(),
                });
                assert!(matches!(
                    session.call(open, CALL_TIMEOUT).await,
                    Err(SessionError::Server {
                        code: ErrorCode::NotImplemented,
                        ..
                    })
                ));
            }
            assert_eq!(
                session
                    .call(Request::FlockSnapshot(Empty {}), CALL_TIMEOUT)
                    .await
                    .unwrap(),
                Response::Ok
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn hello_refuses_a_selection_without_an_implemented_codec() {
        let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
        let (mut session, mut ws) = (session.unwrap(), server.unwrap());
        let server = tokio::spawn(async move {
            let hello = read(&mut ws).await;
            let Response::Hello(mut result) = hello_result(true) else {
                unreachable!()
            };
            result.protocol_version = 33;
            result.protocol_max_version = 34;
            write(
                &mut ws,
                ServerFrame::Result {
                    id: hello.id,
                    result: Response::Hello(result),
                },
            )
            .await;
        });
        assert!(
            matches!(session.hello().await, Err(SessionError::Protocol(message)) if message == "invalid protocol negotiation")
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pairing_calls_are_valid_frames() {
        let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
        let (mut session, mut ws) = (session.unwrap(), server.unwrap());
        let server = tokio::spawn(async move {
            let hello = read(&mut ws).await;
            assert!(matches!(hello.request, Request::Hello(_)));
            write(
                &mut ws,
                ServerFrame::Event {
                    seq: 1,
                    event: Event::FlockChanged {},
                },
            )
            .await;
            write(
                &mut ws,
                ServerFrame::Result {
                    id: hello.id,
                    result: hello_result(false),
                },
            )
            .await;
            let pair = read(&mut ws).await;
            let Request::PairComplete(p) = pair.request else {
                panic!()
            };
            assert_eq!(p.pairing_code.as_str(), "Zm9vYmFyYmF6cXV4cXV1dQ");
            assert_eq!(p.device_label.as_str(), "iPhone");
            write(
                &mut ws,
                ServerFrame::Result {
                    id: pair.id,
                    result: Response::Paired { machine: machine() },
                },
            )
            .await;
            let again = read(&mut ws).await;
            write(
                &mut ws,
                ServerFrame::Error {
                    id: Some(again.id),
                    error: ErrorBody {
                        code: ErrorCode::PairingFailed,
                        message: "pairing window closed".into(),
                        draft: None,
                        protocol_version: None,
                    },
                },
            )
            .await;
        });
        assert!(!session.hello().await.unwrap().paired);
        let pair = || {
            Request::PairComplete(protocol::PairCompleteParams {
                pairing_code: protocol::PairingCode::new("Zm9vYmFyYmF6cXV4cXV1dQ").unwrap(),
                device_label: Label::new("iPhone").unwrap(),
                terminal_key: None,
            })
        };
        assert_eq!(
            expect_paired(session.call(pair(), CALL_TIMEOUT).await.unwrap()).unwrap(),
            machine()
        );
        let err = session.call(pair(), CALL_TIMEOUT).await.unwrap_err();
        assert!(matches!(
            err,
            SessionError::Server {
                code: ErrorCode::PairingFailed,
                ..
            }
        ));
        assert!(!err.is_auth());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn version_mismatch_says_which_side_to_update() {
        for (collied, message) in [
            (
                Some(PROTOCOL_VERSION - 1),
                "Update collied on Mac mini: install the latest release from github.com/rbstp/collie/releases.",
            ),
            (Some(PROTOCOL_VERSION + 1), "Update the app."),
            (
                None,
                "This app and collied on Mac mini use different versions. Update both.",
            ),
        ] {
            let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
            let (mut session, mut ws) = (session.unwrap(), server.unwrap());
            let server = tokio::spawn(async move {
                let hello = read(&mut ws).await;
                write(
                    &mut ws,
                    ServerFrame::Error {
                        id: Some(hello.id),
                        error: ErrorBody {
                            code: ErrorCode::UnsupportedProtocol,
                            message: "unsupported protocol version".into(),
                            draft: None,
                            protocol_version: collied,
                        },
                    },
                )
                .await;
            });
            let err = session.hello().await.unwrap_err();
            let SessionError::VersionMismatch { collied: got } = err else {
                panic!("{err:?}")
            };
            assert_eq!(got, collied);
            assert!(!err.is_auth(), "an update fixes it, not pairing again");
            assert_eq!(version_mismatch(got, "Mac mini"), message);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_frame_that_does_not_parse_is_reported_without_its_text() {
        let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
        let (mut session, mut ws) = (session.unwrap(), server.unwrap());
        let server = tokio::spawn(async move {
            read(&mut ws).await;
            ws.send(Message::text(r#"{"kind":"text the machine chose"}"#))
                .await
                .unwrap();
            ws
        });
        let err = session.hello().await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "unexpected reply from the machine: Data error at line 1 column 32"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn run_correlates_replies_and_applies_events() {
        let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
        let (session, mut ws) = (session.unwrap(), server.unwrap());
        let state = Mutex::new(FlockState::default());
        let (tx, mut rx) = mpsc::channel(8);
        let server = tokio::spawn(async move {
            let seed = read(&mut ws).await;
            assert!(matches!(seed.request, Request::FlockSnapshot(_)));
            write(
                &mut ws,
                ServerFrame::Result {
                    id: seed.id,
                    result: Response::Flock(flock(1, vec![agent("t1", AgentStatus::Idle)])),
                },
            )
            .await;
            let a = read(&mut ws).await;
            let b = read(&mut ws).await;
            write(
                &mut ws,
                ServerFrame::Event {
                    seq: 2,
                    event: status_event("t1", AgentStatus::Blocked),
                },
            )
            .await;
            write(
                &mut ws,
                ServerFrame::Event {
                    seq: 2,
                    event: status_event("t1", AgentStatus::Done),
                },
            )
            .await;
            write(
                &mut ws,
                ServerFrame::Error {
                    id: Some(b.id),
                    error: ErrorBody {
                        code: ErrorCode::NotFound,
                        message: "no such agent".into(),
                        draft: None,
                        protocol_version: None,
                    },
                },
            )
            .await;
            write(
                &mut ws,
                ServerFrame::Result {
                    id: a.id,
                    result: Response::Ok,
                },
            )
            .await;
            write(
                &mut ws,
                ServerFrame::Event {
                    seq: 3,
                    event: Event::FlockChanged {},
                },
            )
            .await;
            let refetch = read(&mut ws).await;
            assert!(matches!(refetch.request, Request::FlockSnapshot(_)));
            write(
                &mut ws,
                ServerFrame::Result {
                    id: refetch.id,
                    result: Response::Flock(flock(
                        3,
                        vec![
                            agent("t1", AgentStatus::Blocked),
                            agent("t2", AgentStatus::Working),
                        ],
                    )),
                },
            )
            .await;
            let c = read(&mut ws).await;
            write(
                &mut ws,
                ServerFrame::Error {
                    id: Some(c.id),
                    error: ErrorBody {
                        code: ErrorCode::NotPaired,
                        message: "not paired".into(),
                        draft: None,
                        protocol_version: None,
                    },
                },
            )
            .await;
            ws
        });
        let (a_tx, a_rx) = oneshot::channel::<Result<Response, SessionError>>();
        let (b_tx, b_rx) = oneshot::channel::<Result<Response, SessionError>>();
        let (c_tx, c_rx) = oneshot::channel::<Result<Response, SessionError>>();
        let focus = |id: &str| {
            Request::AgentFocus(protocol::AgentTarget {
                terminal_id: TerminalId::new(id).unwrap(),
            })
        };
        tx.send((focus("t1"), a_tx)).await.unwrap();
        tx.send((focus("t9"), b_tx)).await.unwrap();
        let client = async {
            assert_eq!(a_rx.await.unwrap().unwrap(), Response::Ok);
            assert!(matches!(
                b_rx.await.unwrap(),
                Err(SessionError::Server {
                    code: ErrorCode::NotFound,
                    ..
                })
            ));
            while lock(&state)
                .flock
                .as_ref()
                .is_none_or(|f| f.agents.len() < 2)
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tx.send((Request::FlockSnapshot(Empty {}), c_tx))
                .await
                .unwrap();
        };
        let (end, ()) = tokio::join!(
            session.run(&mut rx, &state, Vec::new(), std::future::pending()),
            client
        );
        assert!(end.is_auth(), "{end:?}");
        assert!(matches!(
            c_rx.await.unwrap(),
            Err(SessionError::Server {
                code: ErrorCode::NotPaired,
                ..
            })
        ));
        assert_eq!(status_of(&lock(&state), "t1"), Some(AgentStatus::Blocked));
        assert_eq!(status_of(&lock(&state), "t2"), Some(AgentStatus::Working));
        drop(server.await.unwrap());
    }

    #[tokio::test]
    async fn run_asks_again_while_a_snapshot_is_older_than_a_skipped_event() {
        let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
        let (session, mut ws) = (session.unwrap(), server.unwrap());
        let state = Mutex::new(FlockState::default());
        let (_tx, mut rx) = mpsc::channel(8);
        let server = tokio::spawn(async move {
            let snapshot = |id, seq, agents| ServerFrame::Result {
                id,
                result: Response::Flock(flock(seq, agents)),
            };
            let seed = read(&mut ws).await;
            write(&mut ws, snapshot(seed.id, 0, vec![])).await;
            for seq in 1..=2 {
                let mut agent = agent("t1", AgentStatus::Idle);
                agent.title = Some("x".repeat(MAX_REPLAY_BYTES / 2));
                let event = Event::AgentStatus { agent };
                write(&mut ws, ServerFrame::Event { seq, event }).await;
            }
            for (seq, agents) in [(1, vec![]), (2, vec![agent("t2", AgentStatus::Done)])] {
                let asked = read(&mut ws).await;
                assert!(matches!(asked.request, Request::FlockSnapshot(_)));
                write(&mut ws, snapshot(asked.id, seq, agents)).await;
            }
            let more = tokio::time::timeout(Duration::from_millis(200), read(&mut ws)).await;
            assert!(more.is_err(), "the snapshot is current: {more:?}");
        });
        let end = session
            .run(&mut rx, &state, Vec::new(), std::future::pending())
            .await;
        assert!(!end.is_auth());
        server.await.unwrap();
        let s = lock(&state);
        assert_eq!(status_of(&s, "t2"), Some(AgentStatus::Done));
        assert_eq!(status_of(&s, "t1"), None);
    }

    #[tokio::test]
    async fn a_failed_seed_snapshot_marks_herdr_down_until_a_snapshot_works() {
        let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
        let (session, mut ws) = (session.unwrap(), server.unwrap());
        let state = Mutex::new(FlockState::default());
        let (_tx, mut rx) = mpsc::channel(8);
        let (down_tx, down_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let seed = read(&mut ws).await;
            write(
                &mut ws,
                ServerFrame::Error {
                    id: Some(seed.id),
                    error: ErrorBody {
                        code: ErrorCode::HerdrUnavailable,
                        message: "herdr unavailable".into(),
                        draft: None,
                        protocol_version: None,
                    },
                },
            )
            .await;
            down_rx.await.unwrap();
            write(
                &mut ws,
                ServerFrame::Event {
                    seq: 1,
                    event: Event::FlockChanged {},
                },
            )
            .await;
            let refetch = read(&mut ws).await;
            write(
                &mut ws,
                ServerFrame::Result {
                    id: refetch.id,
                    result: Response::Flock(flock(1, vec![agent("t1", AgentStatus::Idle)])),
                },
            )
            .await;
            write(
                &mut ws,
                ServerFrame::Error {
                    id: None,
                    error: ErrorBody {
                        code: ErrorCode::NotPaired,
                        message: "not paired".into(),
                        draft: None,
                        protocol_version: None,
                    },
                },
            )
            .await;
            ws
        });
        let client = async {
            while !lock(&state).herdr_down {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            down_tx.send(()).unwrap();
        };
        let (end, ()) = tokio::join!(
            session.run(&mut rx, &state, Vec::new(), std::future::pending()),
            client
        );
        assert!(end.is_auth(), "{end:?}");
        assert!(!lock(&state).herdr_down);
        assert_eq!(status_of(&lock(&state), "t1"), Some(AgentStatus::Idle));
        drop(server.await.unwrap());
    }

    #[tokio::test]
    async fn new_session_reissues_the_watch() {
        let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
        let (session, mut ws) = (session.unwrap(), server.unwrap());
        let state = Mutex::new(FlockState::default());
        lock(&state).watch(Some(TerminalId::new("t1").unwrap()));
        lock(&state).watch_lines = Some(300);
        lock(&state).low_data = true;
        let (_tx, mut rx) = mpsc::channel(8);
        let server = tokio::spawn(async move {
            assert!(matches!(
                read(&mut ws).await.request,
                Request::FlockSnapshot(_)
            ));
            let watch = read(&mut ws).await;
            let Request::AgentWatch(p) = watch.request else {
                panic!("{watch:?}")
            };
            assert_eq!(p.lines, Some(300));
            assert!(p.low_data);
            assert_eq!(p.terminal_id.unwrap().as_str(), "t1");
            write(
                &mut ws,
                ServerFrame::Error {
                    id: Some(watch.id),
                    error: ErrorBody {
                        code: ErrorCode::NotImplemented,
                        message: "not implemented".into(),
                        draft: None,
                        protocol_version: None,
                    },
                },
            )
            .await;
            write(
                &mut ws,
                ServerFrame::Event {
                    seq: 1,
                    event: Event::AgentOutput(output("t1", "hello world", ReadSource::Recent)),
                },
            )
            .await;
            let stale = OutputPatch {
                terminal_id: TerminalId::new("t1").unwrap(),
                base: 0,
                head: Vec::new(),
                skip: 0,
                keep: 1,
                tail: vec!["x".into()],
                trail: 0,
                truncated: false,
                wraps: None,
                splits: None,
                copilot_scrollbar: None,
                suggestion: None,
            };
            write(
                &mut ws,
                ServerFrame::Event {
                    seq: 2,
                    event: Event::AgentOutputPatch(stale),
                },
            )
            .await;
            let rewatch = read(&mut ws).await;
            let Request::AgentWatch(p) = rewatch.request else {
                panic!("no agent.read after the watch: {rewatch:?}")
            };
            assert_eq!(p.lines, Some(300));
            assert!(p.low_data);
            assert_eq!(p.terminal_id.unwrap().as_str(), "t1");
        });
        let end = session
            .run(&mut rx, &state, Vec::new(), std::future::pending())
            .await;
        assert!(
            !end.is_auth(),
            "an internal request error does not end the session"
        );
        server.await.unwrap();
        let s = lock(&state);
        assert_eq!((shown(&s), s.output_revision), (Some("hello world"), 1));
    }

    #[tokio::test]
    async fn a_new_session_never_reissues_a_terminal_watch() {
        let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
        let (session, mut ws) = (session.unwrap(), server.unwrap());
        let state = Mutex::new(FlockState::default());
        {
            let mut s = lock(&state);
            s.watch_terminal(TerminalId::new("t1").unwrap());
            s.watch_lines = Some(300);
            s.unlocked.insert(
                TerminalId::new("t1").unwrap(),
                StdInstant::now() + Duration::from_secs(60),
            );
            s.new_connection();
            assert!(
                !s.unlocked("t1", StdInstant::now()),
                "a new session is locked"
            );
        }
        let (_tx, mut rx) = mpsc::channel(8);
        let server = tokio::spawn(async move {
            assert!(matches!(
                read(&mut ws).await.request,
                Request::FlockSnapshot(_)
            ));
            let seeded = tokio::time::timeout(Duration::from_millis(200), read(&mut ws)).await;
            assert!(seeded.is_err(), "the snapshot is the only seed: {seeded:?}");
            write(
                &mut ws,
                ServerFrame::Event {
                    seq: 1,
                    event: Event::AgentOutput(output("t1", "$ ls", ReadSource::Recent)),
                },
            )
            .await;
            let stale = OutputPatch {
                terminal_id: TerminalId::new("t1").unwrap(),
                base: 0,
                head: Vec::new(),
                skip: 0,
                keep: 1,
                tail: vec!["x".into()],
                trail: 0,
                truncated: false,
                wraps: None,
                splits: None,
                copilot_scrollbar: None,
                suggestion: None,
            };
            write(
                &mut ws,
                ServerFrame::Event {
                    seq: 2,
                    event: Event::AgentOutputPatch(stale),
                },
            )
            .await;
            let rewatch = read(&mut ws).await;
            let Request::TerminalWatch(p) = rewatch.request else {
                panic!("a broken patch watches the shell again: {rewatch:?}")
            };
            assert_eq!((p.terminal_id.as_str(), p.lines), ("t1", Some(300)));
        });
        let end = session
            .run(&mut rx, &state, Vec::new(), std::future::pending())
            .await;
        assert!(!end.is_auth());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_stop_that_yields_true_closes_the_websocket() {
        for close in [true, false] {
            let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
            let (session, mut ws) = (session.unwrap(), server.unwrap());
            let state = Mutex::new(FlockState::default());
            let (_tx, mut rx) = mpsc::channel(8);
            let (stop_tx, stop_rx) = oneshot::channel();
            let server = tokio::spawn(async move {
                assert!(matches!(
                    read(&mut ws).await.request,
                    Request::FlockSnapshot(_)
                ));
                stop_tx.send(close).unwrap();
                loop {
                    match ws.next().await {
                        Some(Ok(Message::Close(_))) => return true,
                        Some(Ok(_)) => {}
                        _ => return false,
                    }
                }
            });
            let end = session
                .run(&mut rx, &state, Vec::new(), async {
                    stop_rx.await.unwrap()
                })
                .await;
            assert!(matches!(end, SessionError::Closed), "{end:?}");
            assert_eq!(server.await.unwrap(), close);
        }
    }

    #[tokio::test]
    async fn run_fails_pending_requests_when_the_connection_drops() {
        let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
        let (session, mut ws) = (session.unwrap(), server.unwrap());
        let state = Mutex::new(FlockState::default());
        let (tx, mut rx) = mpsc::channel(8);
        let (a_tx, a_rx) = oneshot::channel();
        tx.send((Request::FlockSnapshot(Empty {}), a_tx))
            .await
            .unwrap();
        let server = tokio::spawn(async move {
            read(&mut ws).await;
            read(&mut ws).await;
            drop(ws);
        });
        let end = session
            .run(&mut rx, &state, Vec::new(), std::future::pending())
            .await;
        assert!(!end.is_auth());
        assert!(a_rx.await.unwrap().is_err());
        server.await.unwrap();
    }

    // An in-memory stream: the paused clock auto-advances past socket readiness. The
    // handshake callback's error type is fixed by tungstenite.
    #[allow(clippy::result_large_err)]
    #[tokio::test(start_paused = true)]
    async fn run_ends_a_session_once_the_machine_goes_silent() {
        let (client, server) = tokio::io::duplex(1 << 16);
        let server = tokio::spawn(async move {
            let mut ws = tokio_tungstenite::accept_hdr_async(
                server,
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
            for _ in 0..4 {
                tokio::time::sleep(Duration::from_secs(30)).await;
                ws.send(Message::Ping(Default::default())).await.unwrap();
            }
            (ws, Instant::now())
        });
        let mut session = Session::connect(client, "mac.tail1234.ts.net", 8457)
            .await
            .unwrap();
        session.protocol = protocol::NegotiatedProtocol::negotiate(&HelloParams::current(
            Label::new("test").unwrap(),
        ));
        let state = Mutex::new(FlockState::default());
        let (_tx, mut rx) = mpsc::channel(8);
        let end = session
            .run(&mut rx, &state, Vec::new(), std::future::pending())
            .await;
        let ended = Instant::now();
        let (_ws, last_ping) = server.await.unwrap();
        assert!(matches!(end, SessionError::Timeout), "{end:?}");
        assert_eq!(
            ended - last_ping,
            SILENCE_LIMIT,
            "pings keep the session alive"
        );
    }
}
