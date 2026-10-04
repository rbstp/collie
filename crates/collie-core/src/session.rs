use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use protocol::{
    AgentWatchParams, ClientFrame, Empty, ErrorBody, ErrorCode, Event, Flock, HelloParams,
    HelloResult, Label, PROTOCOL_VERSION, PushRegisterParams, ReadParams, ReadSource, Request,
    RequestId, Response, ServerFrame, TerminalId, TerminalRead, WS_PATH, WS_SUBPROTOCOL,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{self, Message};

pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MESSAGE_BYTES: usize = 4 << 20;
const MAX_REPLAY_EVENTS: usize = 1024;
const MAX_APPROVAL_EVENTS: usize = 64;

#[derive(Debug, Clone, thiserror::Error)]
pub enum SessionError {
    #[error("connection closed")]
    Closed,
    #[error("timed out waiting for the Mac")]
    Timeout,
    #[error("websocket: {0}")]
    WebSocket(String),
    #[error("Mac refused the connection (HTTP {0})")]
    Refused(u16),
    #[error("{message}")]
    Server { code: ErrorCode, message: String },
    #[error("the Mac's input box has unsent text")]
    DraftChanged { current: String },
    #[error("unexpected reply from the Mac: {0}")]
    Protocol(String),
    #[error("this phone is not paired with the Mac")]
    NotPaired,
}

impl SessionError {
    /// Errors that a reconnect cannot fix: the supervisor stops instead of retrying.
    pub fn is_auth(&self) -> bool {
        match self {
            Self::NotPaired => true,
            Self::Refused(code) => matches!(code, 401 | 403),
            Self::Server { code, .. } => {
                matches!(code, ErrorCode::NotPaired | ErrorCode::UnsupportedProtocol)
            }
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
            tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
                Self::Closed
            }
            other => Self::WebSocket(other.to_string()),
        }
    }
}

impl From<ErrorBody> for SessionError {
    fn from(e: ErrorBody) -> Self {
        match (e.code, e.draft) {
            (ErrorCode::DraftChanged, Some(current)) => Self::DraftChanged { current },
            _ => Self::Server {
                code: e.code,
                message: e.message,
            },
        }
    }
}

pub type Reply = oneshot::Sender<Result<Response, SessionError>>;

pub struct Session<S> {
    ws: WebSocketStream<S>,
    next_id: RequestId,
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
        Ok(Self { ws, next_id: 1 })
    }

    pub async fn hello(&mut self) -> Result<HelloResult, SessionError> {
        let app_version = Label::new(env!("CARGO_PKG_VERSION")).expect("crate version is a label");
        match self
            .call(
                Request::Hello(HelloParams {
                    protocol_version: PROTOCOL_VERSION,
                    app_version,
                }),
                CALL_TIMEOUT,
            )
            .await?
        {
            Response::Hello(hello) if hello.protocol_version == PROTOCOL_VERSION => Ok(hello),
            Response::Hello(hello) => Err(SessionError::Protocol(format!(
                "protocol version {}",
                hello.protocol_version
            ))),
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
                match self.recv().await? {
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
        let text = serde_json::to_string(&ClientFrame { id, request })
            .map_err(|e| SessionError::Protocol(e.to_string()))?;
        self.ws.send(Message::text(text)).await?;
        Ok(id)
    }

    async fn recv(&mut self) -> Result<ServerFrame, SessionError> {
        loop {
            match self.ws.next().await.ok_or(SessionError::Closed)?? {
                Message::Text(text) => {
                    return serde_json::from_str(text.as_str())
                        .map_err(|e| SessionError::Protocol(e.to_string()));
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

    /// `push` is re-registered on every session so collied always has the current token.
    pub async fn run(
        mut self,
        requests: &mut tokio::sync::mpsc::Receiver<(Request, Reply)>,
        state: &Mutex<FlockState>,
        push: Option<PushRegisterParams>,
        stop: impl Future<Output = ()>,
    ) -> SessionError {
        let mut pending: HashMap<RequestId, Pending> = HashMap::new();
        let mut seed = vec![Request::FlockSnapshot(Empty {})];
        seed.extend(push.map(Request::PushRegister));
        if let Some(terminal_id) = lock(state).watched.clone() {
            seed.push(Request::AgentWatch(AgentWatchParams {
                terminal_id: Some(terminal_id.clone()),
            }));
            seed.push(Request::AgentRead(ReadParams {
                terminal_id,
                source: ReadSource::Recent,
                lines: None,
            }));
        }
        for request in seed {
            let read_mark = read_mark(state, &request);
            match self.send(request).await {
                Ok(id) => pending.insert(id, (None, read_mark)),
                Err(e) => return e,
            };
        }
        tokio::pin!(stop);
        let end = loop {
            tokio::select! {
                () = &mut stop => break SessionError::Closed,
                req = requests.recv() => {
                    let Some((request, reply)) = req else { break SessionError::Closed };
                    if reply.is_closed() {
                        continue;
                    }
                    let read_mark = read_mark(state, &request);
                    match self.send(request).await {
                        Ok(id) => { pending.insert(id, (Some(reply), read_mark)); }
                        Err(e) => { let _ = reply.send(Err(e.clone())); break e; }
                    }
                }
                frame = self.recv() => {
                    let frame = match frame {
                        Ok(frame) => frame,
                        Err(e) => break e,
                    };
                    match frame {
                        ServerFrame::Result { id, result } => {
                            let (reply, read_mark) = pending.remove(&id).unwrap_or_default();
                            match &result {
                                Response::Flock(flock) => lock(state).apply_snapshot(flock.clone()),
                                Response::Terminal(read) => {
                                    lock(state).apply_read(read.clone(), read_mark);
                                }
                                _ => {}
                            }
                            if let Some(reply) = reply {
                                let _ = reply.send(Ok(result));
                            }
                        }
                        ServerFrame::Error { id: Some(id), error } => {
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
                            if lock(state).apply_event(seq, event) == EventOutcome::NeedsSnapshot {
                                match self.send(Request::FlockSnapshot(Empty {})).await {
                                    Ok(id) => { pending.insert(id, (None, None)); }
                                    Err(e) => break e,
                                }
                            }
                        }
                    }
                }
            }
        };
        for reply in pending.into_values().filter_map(|(reply, _)| reply) {
            let _ = reply.send(Err(end.clone()));
        }
        end
    }
}

type Pending = (Option<Reply>, Option<u64>);

fn read_mark(state: &Mutex<FlockState>, request: &Request) -> Option<u64> {
    matches!(request, Request::AgentRead(_)).then(|| lock(state).output_events)
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

pub fn expect_paired(response: Response) -> Result<protocol::MachineInfo, SessionError> {
    match response {
        Response::Paired { machine } => Ok(machine),
        other => Err(unexpected(&other)),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum EventOutcome {
    Dropped,
    Applied,
    NeedsSnapshot,
}

/// Flock as last seen, kept current by events. `seq` is per connection, so
/// [`FlockState::new_connection`] resets the cursor but keeps the last flock and output
/// for display. `watched` outlives connections so a new session re-issues the watch.
#[derive(Default)]
pub struct FlockState {
    pub flock: Option<Flock>,
    pub watched: Option<TerminalId>,
    pub output: Option<TerminalRead>,
    pub output_revision: u64,
    /// `approval.needed` and `approval.resolved` events, numbered by `approval_revision`.
    pub approval_events: VecDeque<(u64, Event)>,
    pub approval_revision: u64,
    output_events: u64,
    last_seq: u64,
    since_snapshot: Vec<(u64, Event)>,
}

impl FlockState {
    pub fn new_connection(&mut self) {
        self.last_seq = 0;
        self.since_snapshot.clear();
    }

    pub fn watch(&mut self, terminal_id: Option<TerminalId>) {
        if self.watched != terminal_id {
            self.output = None;
        }
        self.watched = terminal_id;
    }

    /// Only `recent` reads of the watched agent are kept: that is what `agent.output`
    /// carries, so a `visible` read would flicker against the stream.
    fn apply_output(&mut self, read: TerminalRead) -> bool {
        let watched =
            read.source == ReadSource::Recent && self.watched.as_ref() == Some(&read.terminal_id);
        if watched {
            self.output = Some(read);
            self.output_revision += 1;
        }
        watched
    }

    /// A read reply is not ordered against `agent.output` events: collied may have read
    /// herdr before an event it sent first. `mark` is the event count when the read was
    /// sent; if an event was applied since, the reply is older than the screen.
    pub fn apply_read(&mut self, read: TerminalRead, mark: Option<u64>) {
        if mark.is_none_or(|m| m == self.output_events) {
            self.apply_output(read);
        }
    }

    pub fn apply_event(&mut self, seq: u64, event: Event) -> EventOutcome {
        if seq <= self.last_seq {
            return EventOutcome::Dropped;
        }
        self.last_seq = seq;
        match event {
            Event::AgentOutput(read) => {
                if self.apply_output(read) {
                    self.output_events += 1;
                }
            }
            Event::FlockChanged {} => return EventOutcome::NeedsSnapshot,
            Event::Unrecognized => {}
            event => {
                if let Some(flock) = &mut self.flock {
                    apply(flock, &event);
                }
                if matches!(
                    event,
                    Event::ApprovalNeeded { .. } | Event::ApprovalResolved { .. }
                ) {
                    if self.approval_events.len() == MAX_APPROVAL_EVENTS {
                        self.approval_events.pop_front();
                    }
                    self.approval_revision += 1;
                    self.approval_events
                        .push_back((self.approval_revision, event.clone()));
                }
                if self.since_snapshot.len() == MAX_REPLAY_EVENTS {
                    self.since_snapshot.remove(0);
                }
                self.since_snapshot.push((seq, event));
            }
        }
        EventOutcome::Applied
    }

    /// A snapshot answered after newer events were applied must not erase them, so
    /// events past `snapshot.seq` are replayed onto it.
    pub fn apply_snapshot(&mut self, mut snapshot: Flock) {
        self.since_snapshot.retain(|(seq, _)| *seq > snapshot.seq);
        for (_, event) in &self.since_snapshot {
            apply(&mut snapshot, event);
        }
        self.last_seq = self.last_seq.max(snapshot.seq);
        self.flock = Some(snapshot);
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
        Event::AgentOutput(_) | Event::FlockChanged {} | Event::Unrecognized => {}
    }
}

#[cfg(test)]
mod tests {
    use protocol::{
        Agent, AgentStatus, ApprovalId, ClientFrame, MachineInfo, TerminalId, WorkspaceId,
        parse_client_frame,
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
            s.apply_event(5, status_event("t1", AgentStatus::Blocked)),
            EventOutcome::Dropped
        );
        assert_eq!(
            s.apply_event(3, status_event("t1", AgentStatus::Blocked)),
            EventOutcome::Dropped
        );
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Idle));
        assert_eq!(
            s.apply_event(6, status_event("t1", AgentStatus::Working)),
            EventOutcome::Applied
        );
        assert_eq!(
            s.apply_event(6, status_event("t1", AgentStatus::Done)),
            EventOutcome::Dropped
        );
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Working));
        assert_eq!(
            s.apply_event(7, status_event("t2", AgentStatus::Blocked)),
            EventOutcome::Applied
        );
        assert_eq!(status_of(&s, "t2"), Some(AgentStatus::Blocked));
    }

    #[test]
    fn unknown_events_advance_the_cursor() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(1, vec![]));
        assert_eq!(s.apply_event(2, Event::Unrecognized), EventOutcome::Applied);
        assert_eq!(
            s.apply_event(2, status_event("t1", AgentStatus::Idle)),
            EventOutcome::Dropped
        );
        assert_eq!(
            s.apply_event(3, Event::FlockChanged {}),
            EventOutcome::NeedsSnapshot
        );
    }

    #[test]
    fn late_snapshot_keeps_newer_events() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(1, vec![agent("t1", AgentStatus::Idle)]));
        s.apply_event(2, status_event("t1", AgentStatus::Working));
        s.apply_event(3, status_event("t1", AgentStatus::Blocked));
        s.apply_snapshot(flock(2, vec![agent("t1", AgentStatus::Working)]));
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Blocked));
        s.apply_snapshot(flock(9, vec![agent("t1", AgentStatus::Done)]));
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Done));
        assert_eq!(
            s.apply_event(9, status_event("t1", AgentStatus::Idle)),
            EventOutcome::Dropped
        );
    }

    #[test]
    fn new_connection_resets_cursor_but_keeps_flock() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(40, vec![agent("t1", AgentStatus::Idle)]));
        s.new_connection();
        assert_eq!(status_of(&s, "t1"), Some(AgentStatus::Idle));
        assert_eq!(
            s.apply_event(1, status_event("t1", AgentStatus::Working)),
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
        );
        s.apply_event(2, Event::ApprovalNeeded { approval });
        assert_eq!(s.flock.as_ref().unwrap().approvals.len(), 1);
        s.apply_event(
            3,
            Event::ApprovalResolved {
                approval_id: ApprovalId::new("a1").unwrap(),
                outcome: protocol::ApprovalOutcome::Expired,
            },
        );
        assert!(s.flock.as_ref().unwrap().approvals.is_empty());
    }

    #[test]
    fn approval_events_are_numbered_and_capped() {
        let mut s = FlockState::default();
        s.apply_snapshot(flock(0, vec![]));
        let resolved = |id: &str| Event::ApprovalResolved {
            approval_id: ApprovalId::new(id).unwrap(),
            outcome: protocol::ApprovalOutcome::Superseded,
        };
        s.apply_event(1, resolved("a1"));
        s.apply_event(1, resolved("dup"));
        s.apply_event(2, status_event("t1", AgentStatus::Idle));
        assert_eq!(s.approval_revision, 1);
        for seq in 3..(3 + MAX_APPROVAL_EVENTS as u64) {
            s.apply_event(seq, resolved("a2"));
        }
        assert_eq!(s.approval_events.len(), MAX_APPROVAL_EVENTS);
        assert_eq!(s.approval_events.front().unwrap().0, 2);
        s.new_connection();
        s.apply_event(1, resolved("a3"));
        assert_eq!(
            s.approval_revision,
            MAX_APPROVAL_EVENTS as u64 + 2,
            "revisions never go back across connections"
        );
    }

    #[test]
    fn read_reply_older_than_an_applied_event_is_dropped() {
        let mut s = FlockState::default();
        s.watch(Some(TerminalId::new("t1").unwrap()));
        let ev = |ansi| Event::AgentOutput(output("t1", ansi, ReadSource::Recent));
        let mark = s.output_events;
        s.apply_event(1, ev("newer"));
        s.apply_read(output("t1", "older", ReadSource::Recent), Some(mark));
        assert_eq!((shown(&s), s.output_revision), (Some("newer"), 1));
        s.apply_read(
            output("t1", "fresh", ReadSource::Recent),
            Some(s.output_events),
        );
        assert_eq!((shown(&s), s.output_revision), (Some("fresh"), 2));
        let mark = s.output_events;
        s.apply_event(2, Event::AgentOutput(output("t2", "x", ReadSource::Recent)));
        s.apply_read(output("t1", "still fresh", ReadSource::Recent), Some(mark));
        assert_eq!(
            shown(&s),
            Some("still fresh"),
            "an unwatched agent's event does not count"
        );
    }

    fn output(id: &str, ansi: &str, source: ReadSource) -> TerminalRead {
        TerminalRead {
            terminal_id: TerminalId::new(id).unwrap(),
            source,
            ansi: ansi.into(),
            truncated: false,
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
            s.apply_event(2, ev("t1", "unwatched")),
            EventOutcome::Applied
        );
        assert_eq!(shown(&s), None);

        s.watch(Some(TerminalId::new("t1").unwrap()));
        s.apply_event(3, ev("t1", "a"));
        assert_eq!((shown(&s), s.output_revision), (Some("a"), 1));
        assert_eq!(s.apply_event(3, ev("t1", "dup")), EventOutcome::Dropped);
        assert_eq!(s.apply_event(2, ev("t1", "old")), EventOutcome::Dropped);
        s.apply_event(4, ev("t2", "other agent"));
        s.apply_read(output("t1", "visible", ReadSource::Visible), None);
        assert_eq!((shown(&s), s.output_revision), (Some("a"), 1));
        s.apply_event(5, ev("t1", "b"));
        assert_eq!((shown(&s), s.output_revision), (Some("b"), 2));
        assert!(
            s.since_snapshot
                .iter()
                .all(|(_, e)| !matches!(e, Event::AgentOutput(_)))
        );

        s.new_connection();
        assert_eq!(shown(&s), Some("b"), "kept on screen while reconnecting");
        s.apply_event(1, ev("t1", "c"));
        assert_eq!((shown(&s), s.output_revision), (Some("c"), 3));

        s.watch(Some(TerminalId::new("t1").unwrap()));
        assert_eq!(shown(&s), Some("c"));
        s.watch(Some(TerminalId::new("t2").unwrap()));
        assert_eq!(shown(&s), None);
        s.apply_event(2, ev("t2", "d"));
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
        let session = Session::connect(client, "mac.tail1234.ts.net", 8457).await;
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
            protocol_version: PROTOCOL_VERSION,
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
            session.run(&mut rx, &state, None, std::future::pending()),
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
    async fn new_session_reissues_the_watch() {
        let (session, server) = pair_of(Some(WS_SUBPROTOCOL)).await;
        let (session, mut ws) = (session.unwrap(), server.unwrap());
        let state = Mutex::new(FlockState::default());
        lock(&state).watch(Some(TerminalId::new("t1").unwrap()));
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
            assert_eq!(p.terminal_id.unwrap().as_str(), "t1");
            let initial = read(&mut ws).await;
            let Request::AgentRead(p) = initial.request else {
                panic!("{initial:?}")
            };
            assert_eq!(
                (p.terminal_id.as_str(), p.source),
                ("t1", ReadSource::Recent)
            );
            write(
                &mut ws,
                ServerFrame::Error {
                    id: Some(watch.id),
                    error: ErrorBody {
                        code: ErrorCode::NotImplemented,
                        message: "not implemented".into(),
                        draft: None,
                    },
                },
            )
            .await;
            write(
                &mut ws,
                ServerFrame::Result {
                    id: initial.id,
                    result: Response::Terminal(output("t1", "hello", ReadSource::Recent)),
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
        });
        let end = session
            .run(&mut rx, &state, None, std::future::pending())
            .await;
        assert!(
            !end.is_auth(),
            "an internal request error does not end the session"
        );
        server.await.unwrap();
        let s = lock(&state);
        assert_eq!((shown(&s), s.output_revision), (Some("hello world"), 2));
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
            .run(&mut rx, &state, None, std::future::pending())
            .await;
        assert!(!end.is_auth());
        assert!(a_rx.await.unwrap().is_err());
        server.await.unwrap();
    }
}
