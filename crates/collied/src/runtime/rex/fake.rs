//! A `rex.control.v1` server of the tests' own, in process, on a Unix socket in a temporary
//! folder: CI has no Rex, and no test reaches the user's.

use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use super::client::{Method, TERMINAL};

pub(super) const HOST: &str = "1f4bd4b7-e04f-46e2-ad3b-0e2b89b5447c";
/// This connection's client id, as `session.attach` names it.
const CLIENT: &str = "client:1";
pub(super) const NONCE: u64 = 422_037_677_951_212_370;

/// Run once, on a request for its method (and block, when named): `after` runs once the
/// answer is read, so the events it returns carry a later seq than the answer's, yet reach
/// collied before it.
pub(super) type Hook = Box<dyn FnOnce(&mut State) -> Vec<Value> + Send>;

pub(super) struct Block {
    pub session: String,
    pub id: String,
    pub host: String,
    pub creator: String,
    pub records: Vec<Value>,
    pub process: Value,
    pub title: String,
    /// What `format` renders as VT, and the size `size` answers.
    pub screen: String,
    /// What it renders without `unwrap`, when that differs.
    pub rows: Option<String>,
    pub size: (u32, u32),
    pub events: Vec<Value>,
    /// The client that owns it: the one that created it, until it gives it up.
    pub owner: Option<String>,
    /// In no window: `session.list_blocks` lists it, the session's view does not.
    pub detached: bool,
}

pub(super) struct State {
    /// Every method called, in order, `hello` first.
    pub calls: Vec<String>,
    pub hello: Value,
    /// How the server closes the connection after the hello, instead of a welcome.
    pub close: Option<(CloseCode, &'static str)>,
    /// Sessions this connection attached to: only theirs send block events, as Rex does.
    attached: HashSet<String>,
    pub methods: Vec<&'static str>,
    pub terminal: Vec<&'static str>,
    pub listeners: Value,
    pub tailscale: &'static str,
    pub sessions: Vec<(String, String)>,
    pub blocks: Vec<Block>,
    hooks: Vec<(&'static str, Option<&'static str>, bool, Hook)>,
    /// `format` results kept for `blob_read`, newest last, at most four per block as Rex
    /// keeps them: id, block, rendering.
    pub results: Vec<(u64, String, String)>,
    next_result: u64,
    /// The bytes of screen `format` and `blob_read` answered with.
    pub moved: usize,
    /// While set, `block.event` responses are held until an `emit` finds it cleared.
    pub stall: bool,
    held: Vec<Value>,
    pub created: Vec<Value>,
    /// The status the program of a created block reports first; none for `None`.
    pub start: Option<&'static str>,
    /// While set, `session.disown_block` leaves the caller the owner.
    pub keep_owner: bool,
    /// While set, `session.attach` does not name the caller's client.
    pub anonymous: bool,
    /// The folder a created block's program runs in instead of the layout's, as Rex starts
    /// one whose folder is not a directory in the home folder.
    pub runs_in: Option<String>,
    /// A method whose request ends the connection instead of an answer.
    pub hang_up: Option<&'static str>,
    /// Sessions whose view shows a window `session.list_blocks` has no block for.
    pub bare_windows: Vec<String>,
    /// A method whose requests are never answered, the connection left open.
    pub unanswered: Option<&'static str>,
    pub instance: &'static str,
    pub started_at: String,
    pub splits: Vec<Value>,
    pub split_pid: Option<u32>,
    pub split_exits: bool,
    pub split_host: Option<String>,
    pub split_cwd: Option<String>,
    pub data: Data,
}

/// The one stream every data connection serves, as Rex serves a block's terminal.
#[derive(Default)]
pub(super) struct Data {
    /// A snapshot and what follows it.
    pub stream: Vec<u8>,
    /// The REJECT reason a HELLO gets instead of a WELCOME.
    pub reject: Option<u8>,
    /// Each HELLO (1) and RESUME (2), with the count a RESUME named.
    pub handshakes: Vec<(u8, u64)>,
    pub at: Vec<tokio::time::Instant>,
    /// Told of each handshake, with no timer: a client that loops keeps a paused clock still.
    pub shaken: Arc<tokio::sync::Notify>,
    /// How each data connection ended: the close code collied sent, `None` for none.
    pub closed: Vec<Option<u16>>,
    /// Where a connection stops sending the stream, as in the middle of a snapshot.
    pub hold_at: Option<usize>,
    /// A RESUME is refused, or welcomed and then dropped before any byte.
    pub refuse_resume: bool,
    pub drop_resumed: bool,
    /// The bytes of `stream` sent, across its connections.
    sent: usize,
    token: [u8; 16],
}

impl State {
    pub(super) fn hook(
        &mut self,
        method: Method,
        block: Option<&'static str>,
        after: bool,
        hook: Hook,
    ) {
        self.hooks.push((method.name(), block, after, hook));
    }

    pub(super) fn block(&mut self, id: &str) -> &mut Block {
        self.blocks.iter_mut().find(|b| b.id == id).unwrap()
    }

    pub(super) fn add(&mut self, session: &str, id: &str, process: Value, records: Vec<Value>) {
        self.blocks.push(Block {
            session: session.to_owned(),
            id: id.to_owned(),
            host: HOST.to_owned(),
            creator: TERMINAL.to_owned(),
            records,
            process,
            title: String::new(),
            screen: String::new(),
            rows: None,
            size: (80, 24),
            events: Vec::new(),
            owner: None,
            detached: false,
        });
    }
}

enum Push {
    Events(Hook),
    Drop,
}

enum DataPush {
    /// What `stream` gained since the last send.
    More,
    /// A close frame with this code, as Rex ends a stream.
    Close(u16),
    Drop,
}

type Pushes<T> = Arc<Mutex<Option<mpsc::UnboundedSender<T>>>>;

pub(super) struct Fake {
    _dir: tempfile::TempDir,
    pub socket: PathBuf,
    pub state: Arc<Mutex<State>>,
    push: Pushes<Push>,
    data_push: Pushes<DataPush>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fake {
    /// Every method this phase checks, a personal Unix listener, Tailscale off, no session.
    pub(super) async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("rex");
        std::fs::create_dir(&folder).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = folder.join("server.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let state = Arc::new(Mutex::new(State {
            calls: Vec::new(),
            hello: Value::Null,
            close: None,
            attached: HashSet::new(),
            methods: (Method::ALL.into_iter())
                .filter(|m| m.terminal().is_none())
                .map(Method::name)
                .collect(),
            terminal: Method::ALL
                .into_iter()
                .filter_map(Method::terminal)
                .collect(),
            listeners: json!([{
                "endpoint": format!("unix://{}", socket.display()),
                "transport": "unix", "admission": "peer_credentials", "primary": true,
            }]),
            tailscale: "disabled",
            sessions: Vec::new(),
            blocks: Vec::new(),
            hooks: Vec::new(),
            results: Vec::new(),
            next_result: 1,
            moved: 0,
            stall: false,
            held: Vec::new(),
            created: Vec::new(),
            start: Some("idle"),
            keep_owner: false,
            anonymous: false,
            runs_in: None,
            hang_up: None,
            bare_windows: Vec::new(),
            unanswered: None,
            instance: "i",
            splits: Vec::new(),
            split_pid: None,
            split_exits: false,
            split_host: None,
            split_cwd: None,
            started_at: "2026-11-12T16:47:01.264128Z".to_owned(),
            data: Data::default(),
        }));
        let push = Pushes::default();
        let data_push = Pushes::default();
        let task = tokio::spawn({
            let (state, push, data_push) = (state.clone(), push.clone(), data_push.clone());
            async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let (state, push, data_push) = (state.clone(), push.clone(), data_push.clone());
                    tokio::spawn(accept(state, push, data_push, stream));
                }
            }
        });
        Self {
            _dir: dir,
            socket,
            state,
            push,
            data_push,
            task,
        }
    }

    pub(super) fn state(&self) -> MutexGuard<'_, State> {
        crate::lock(&self.state)
    }

    /// Changes the state and sends the events `f` returns, on the latest connection.
    pub(super) fn emit(&self, f: impl FnOnce(&mut State) -> Vec<Value> + Send + 'static) {
        if let Some(tx) = crate::lock(&self.push).as_ref() {
            let _ = tx.send(Push::Events(Box::new(f)));
        }
    }

    /// Ends the latest connection without a close frame, as a server that went away.
    pub(super) fn drop_connection(&self) {
        if let Some(tx) = crate::lock(&self.push).take() {
            let _ = tx.send(Push::Drop);
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.state().calls.clone()
    }

    /// Adds `bytes` to the stream, sent at once on the open data connection, else on the
    /// next one that resumes.
    pub(super) fn data_send(&self, bytes: &[u8]) {
        self.state().data.stream.extend_from_slice(bytes);
        if let Some(tx) = crate::lock(&self.data_push).as_ref() {
            let _ = tx.send(DataPush::More);
        }
    }

    pub(super) fn data_close(&self, code: u16) {
        if let Some(tx) = crate::lock(&self.data_push).take() {
            let _ = tx.send(DataPush::Close(code));
        }
    }

    /// Ends the latest data connection without a close frame, keeping its stream to resume.
    pub(super) fn data_drop(&self) {
        if let Some(tx) = crate::lock(&self.data_push).take() {
            let _ = tx.send(DataPush::Drop);
        }
    }
}

/// Serves `/api/control` or `/api/data`, by the request's path.
async fn accept(
    state: Arc<Mutex<State>>,
    push: Pushes<Push>,
    data_push: Pushes<DataPush>,
    stream: UnixStream,
) {
    let mut data = false;
    #[allow(clippy::result_large_err)]
    let protocol = |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
        data = request.uri().path() == "/api/data";
        let protocol = match data {
            true => "rex.data.terminal.v1",
            false => "rex.control.v1",
        };
        (response.headers_mut())
            .insert("Sec-WebSocket-Protocol", HeaderValue::from_static(protocol));
        Ok(response)
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, protocol).await else {
        return;
    };
    if data {
        let (tx, rx) = mpsc::unbounded_channel();
        *crate::lock(&data_push) = Some(tx);
        serve_data(state, ws, rx).await;
    } else {
        let (tx, rx) = mpsc::unbounded_channel();
        *crate::lock(&push) = Some(tx);
        serve(state, ws, rx).await;
    }
}

/// A WELCOME with a new reconnect token, or the REJECT a HELLO is set to get; a RESUME is
/// welcomed only with the latest token and the exact count of bytes sent, as Rex requires.
fn welcome(d: &mut Data, first: &[u8]) -> Result<usize, u8> {
    let count = first
        .get(17..25)
        .and_then(|b| b.try_into().ok())
        .map_or(0, u64::from_le_bytes);
    d.handshakes.push((first[0], count));
    d.at.push(tokio::time::Instant::now());
    d.shaken.notify_one();
    let from = match first[0] {
        1 => match d.reject {
            Some(reason) => return Err(reason),
            None => 0,
        },
        2 if !d.refuse_resume && first[1..17] == d.token && count == d.sent as u64 => d.sent,
        _ => return Err(0x01),
    };
    d.token = [d.handshakes.len() as u8; 16];
    Ok(from)
}

async fn serve_data(
    state: Arc<Mutex<State>>,
    ws: WebSocketStream<UnixStream>,
    mut pushed: mpsc::UnboundedReceiver<DataPush>,
) {
    let (mut sink, mut source) = ws.split();
    let Some(Ok(Message::Binary(first))) = source.next().await else {
        return;
    };
    let welcomed = welcome(&mut crate::lock(&state).data, &first);
    let token = crate::lock(&state).data.token;
    let answer = match welcomed {
        Ok(_) => {
            let mut w = vec![0x81];
            w.extend_from_slice(&token);
            w.extend_from_slice(&[0, 0, 0]);
            w.extend_from_slice(&60_000u32.to_le_bytes());
            w.extend_from_slice(&NONCE.to_le_bytes());
            w
        }
        Err(reason) => vec![0x82, reason],
    };
    if sink.send(Message::binary(answer)).await.is_err() {
        return;
    }
    let Ok(mut sent) = welcomed else {
        let _ = sink.close().await;
        return;
    };
    if first[0] == 2 && crate::lock(&state).data.drop_resumed {
        crate::lock(&state).data.closed.push(None);
        return;
    }
    let ended = loop {
        let more = {
            let mut s = crate::lock(&state);
            let end = s
                .data
                .hold_at
                .unwrap_or(usize::MAX)
                .min(s.data.stream.len());
            let more = s.data.stream[sent.min(end)..end].to_vec();
            sent = end.max(sent);
            s.data.sent = sent;
            more
        };
        // Pieces of 1,000 bytes, so the snapshot and Rex's messages are cut across them.
        for piece in more.chunks(1000) {
            if sink.send(Message::binary(piece.to_vec())).await.is_err() {
                return;
            }
        }
        tokio::select! {
            next = source.next() => match next {
                Some(Ok(Message::Close(frame))) => break frame.map(|f| u16::from(f.code)),
                Some(Ok(_)) => {}
                _ => break None,
            },
            pushed = pushed.recv() => match pushed {
                Some(DataPush::More) => {}
                Some(DataPush::Close(code)) => {
                    let frame = CloseFrame { code: CloseCode::from(code), reason: "".into() };
                    let _ = sink.send(Message::Close(Some(frame))).await;
                    break None;
                }
                Some(DataPush::Drop) | None => break None,
            },
        }
    };
    crate::lock(&state).data.closed.push(ended);
}

async fn serve(
    state: Arc<Mutex<State>>,
    ws: WebSocketStream<UnixStream>,
    mut pushed: mpsc::UnboundedReceiver<Push>,
) {
    let (mut sink, mut source) = ws.split();
    let Some(Ok(Message::Text(hello))) = source.next().await else {
        return;
    };
    let close = {
        let mut s = crate::lock(&state);
        s.calls.push("hello".to_owned());
        s.hello = serde_json::from_str(&hello).unwrap();
        s.attached.clear();
        s.close
    };
    if let Some((code, reason)) = close {
        let frame = CloseFrame {
            code,
            reason: reason.into(),
        };
        let _ = sink.send(Message::Close(Some(frame))).await;
        return;
    }
    let welcome = json!({
        "type": "welcome", "protocol": "rex.control.v1",
        "limits": {"max_message_bytes": 1 << 20},
        "api_revision": 13, "api_server_revision": 13, "api_min_revision": 0,
    });
    if sink.send(Message::text(welcome.to_string())).await.is_err() {
        return;
    }
    let mut seq = 0u64;
    let mut out: Vec<Value> = Vec::new();
    loop {
        tokio::select! {
            next = source.next() => {
                let Some(Ok(Message::Text(text))) = next else { return };
                let request: Value = serde_json::from_str(&text).unwrap();
                let mut s = crate::lock(&state);
                if s.hang_up.is_some() && s.hang_up == request["method"].as_str() {
                    s.calls.push(request["method"].as_str().unwrap().to_owned());
                    return;
                }
                answer(&mut s, &request, &mut seq, &mut out);
            }
            pushed = pushed.recv() => {
                let Some(Push::Events(f)) = pushed else { return };
                let mut s = crate::lock(&state);
                for e in f(&mut s) {
                    deliver(&s, &mut seq, &mut out, e);
                }
                if !s.stall {
                    out.append(&mut s.held);
                }
            }
        }
        for message in out.drain(..) {
            if sink.send(Message::text(message.to_string())).await.is_err() {
                return;
            }
        }
    }
}

fn hooked(s: &mut State, method: &str, block: &str, after: bool) -> Vec<Value> {
    let Some(i) = (s.hooks.iter())
        .position(|(m, b, a, _)| *m == method && b.is_none_or(|b| b == block) && *a == after)
    else {
        return Vec::new();
    };
    let (.., hook) = s.hooks.remove(i);
    hook(s)
}

/// A block event reaches only a connection attached to its session.
fn deliver(s: &State, seq: &mut u64, out: &mut Vec<Value>, e: Value) {
    let session = e["payload"]["session_id"].as_str().unwrap_or_default();
    if e["payload"]["event"]["type"] == "block_event" && !s.attached.contains(session) {
        return;
    }
    *seq += 1;
    out.push(json!({ "type": "event", "seq": *seq, "payload": e }));
}

fn answer(s: &mut State, request: &Value, seq: &mut u64, out: &mut Vec<Value>) {
    let method = request["method"].as_str().unwrap().to_owned();
    let payload = &request["payload"];
    s.calls.push(method.clone());
    if s.unanswered == Some(method.as_str()) {
        return;
    }
    let session = payload["session_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let block = payload["block_id"].as_str().unwrap_or_default().to_owned();
    for e in hooked(s, &method, &block, false) {
        deliver(s, seq, out, e);
    }
    let answered = *seq;
    let has_session = s.sessions.iter().any(|(id, _)| *id == session);
    let found = s
        .blocks
        .iter()
        .position(|b| b.id == block && b.session == session);
    let sessions = || {
        let list: Vec<Value> = (s.sessions.iter())
            .map(|(id, label)| json!({ "session_id": id, "label": label }))
            .collect();
        json!({ "sessions": list })
    };
    let terminal = method
        .strip_prefix(TERMINAL)
        .and_then(|m| m.strip_prefix('.'));
    let mut closed = None;
    let result: Result<Value, &str> = match (method.as_str(), terminal) {
        ("help.all", _) => {
            let methods: Vec<Value> = s.methods.iter().map(|m| json!({ "name": m })).collect();
            Ok(json!({ "methods": methods, "block_types": [] }))
        }
        ("server.version", _) => Ok(json!({
            "rex_version": "0.1.0-preview.16", "platform": "darwin_arm64", "dirty": false,
            "api_revision": 13, "api_min_revision": 0,
        })),
        ("server.status", _) => Ok(json!({
            "pid": std::process::id(), "instance_id": s.instance, "host_id": HOST,
            "started_at": s.started_at, "exit_empty": false,
            "session_count": s.sessions.len(), "config": {"generation": 1, "hash": ""},
            "tailscale": {"state": s.tailscale, "allow_tailnet_members": false},
            "listeners": s.listeners.clone(),
        })),
        ("block_creator.list", _) => {
            let methods: serde_json::Map<String, Value> = (s.terminal.iter())
                .map(|m| ((*m).to_owned(), json!({})))
                .collect();
            Ok(json!({ "creators": [
                { "name": "com.superlogical.onboarding", "title": "", "description": "", "flavors": [], "api": {"methods": {"state": {}}} },
                { "name": TERMINAL, "title": "Terminal", "description": "", "flavors": [], "host_id": HOST, "api": {"methods": methods} },
            ]}))
        }
        ("session.attach_all", _) => Ok(sessions()),
        ("session.list", _) => Ok(sessions()),
        ("session.attach", _) if has_session => {
            s.attached.insert(session.clone());
            let label = s
                .sessions
                .iter()
                .find(|(id, _)| *id == session)
                .map(|(_, l)| l);
            let client = (!s.anonymous).then_some(CLIENT);
            // One window per placed block, left out when there is none, as Rex leaves it out.
            let mut windows: Vec<Value> = (s.blocks.iter())
                .filter(|b| b.session == session && !b.detached)
                .map(|b| json!({ "window_id": format!("window:{}", b.id), "label": "w" }))
                .collect();
            if s.bare_windows.contains(&session) {
                windows.push(json!({ "window_id": "window:bare", "label": "w" }));
            }
            let mut view = json!({ "revision": 1, "session_id": session, "label": label });
            if !windows.is_empty() {
                view["windows"] = json!(windows);
            }
            Ok(json!({ "client_id": client, "view": view }))
        }
        ("session.list_blocks", _) if has_session => {
            let blocks: Vec<Value> = (s.blocks.iter())
                .filter(|b| b.session == session)
                .map(|b| {
                    json!({
                        "block_id": b.id, "label": "shell", "creator_name": b.creator,
                        "host_id": b.host,
                        "placement": if b.detached { "detached" } else { "placed" },
                    })
                })
                .collect();
            Ok(json!({ "blocks": blocks }))
        }
        ("session.create", _) => Ok(create(s, payload, seq, out)),
        ("session.new_split", _) if !s.attached.contains(&session) => Err("bad_request"),
        ("session.new_split", _) => {
            let anchor = payload["anchor_id"].as_str().unwrap_or_default();
            match s
                .blocks
                .iter()
                .any(|b| b.id == anchor && b.session == session)
            {
                true => Ok(split(s, &session, payload, seq, out)),
                false => Err("not_found"),
            }
        }
        ("session.disown_block", _) if !s.attached.contains(&session) => Err("bad_request"),
        ("session.disown_block", _) => {
            let ids: Vec<String> = (payload["block_ids"].as_array().into_iter().flatten())
                .filter_map(|b| b.as_str().map(str::to_owned))
                .collect();
            let mut owners = Vec::new();
            for id in ids {
                match s
                    .blocks
                    .iter_mut()
                    .find(|b| b.id == id && b.session == session)
                {
                    Some(b) => {
                        if b.owner.as_deref() == Some(CLIENT) && !s.keep_owner {
                            b.owner = None;
                        }
                        owners.push(json!({ "block_id": id, "owner": b.owner }));
                    }
                    None => return answer_error(request, answered, out, "bad_request"),
                }
            }
            Ok(json!({ "blocks": owners }))
        }
        ("session.destroy", _) if !s.attached.contains(&session) => Err("conflict"),
        ("session.destroy", _) if has_session => {
            s.sessions.retain(|(id, _)| *id != session);
            s.blocks.retain(|b| b.session != session);
            let destroyed =
                json!({ "type": "session_destroyed", "payload": { "session_id": session } });
            deliver(s, seq, out, destroyed);
            Ok(json!({}))
        }
        ("block.close", _) => match found {
            Some(i) => {
                closed = Some(s.blocks.remove(i));
                Ok(json!({ "block_id": block, "revision": 2 }))
            }
            None => Err("not_found"),
        },
        ("data.connect", _) if !s.attached.contains(&session) => Err("bad_request"),
        ("data.connect", _) => match found {
            // 16 zero bytes, base64url without padding.
            Some(_) => Ok(json!({ "ticket": "AAAAAAAAAAAAAAAAAAAAAA", "expires_in_ms": 60000 })),
            None => Err("not_found"),
        },
        ("block.event", _) if !s.attached.contains(&session) => Err("conflict"),
        ("block.event", _) => match found {
            Some(i) => {
                s.blocks[i].events.push(payload["event"].clone());
                Ok(json!({}))
            }
            None => Err("not_found"),
        },
        (_, Some("resize")) if s.terminal.contains(&"resize") => match found {
            Some(i) => {
                let args = &payload["args"];
                let (columns, rows) = (args["columns"].as_u64(), args["rows"].as_u64());
                let b = &mut s.blocks[i];
                let owned = b.owner.as_deref().is_none_or(|o| o == CLIENT);
                if let (Some(c), Some(r), true) = (columns, rows, owned) {
                    b.size = (c as u32, r as u32);
                    let size = json!({ "columns": c, "rows": r });
                    let changed = block_event(&session, &block, "size_changed", size);
                    deliver(s, seq, out, changed);
                }
                Ok(json!({}))
            }
            None => Err("not_found"),
        },
        (_, Some(m)) if s.terminal.contains(&m) => match found.map(|i| &s.blocks[i]) {
            Some(b) => match m {
                "process" => Ok(b.process.clone()),
                "program_status" => Ok(json!({ "records": b.records })),
                "title" => Ok(json!({ "title": b.title })),
                "size" => Ok(json!({
                    "columns": b.size.0, "rows": b.size.1, "cell_width_px": 8,
                    "cell_height_px": 16, "owner": null, "desired": [],
                })),
                "format" => {
                    let rows = b
                        .rows
                        .as_ref()
                        .filter(|_| payload["args"]["unwrap"] == false);
                    let screen = rows.unwrap_or(&b.screen).clone();
                    Ok(format(s, &block, screen, &payload["args"]))
                }
                "blob_read" => blob_read(s, &block, &payload["args"]),
                _ => Err("not_found"),
            },
            None => Err("not_found"),
        },
        (m, _) if m.starts_with("session.") && !has_session => Err("session_not_found"),
        _ => Err("method_not_found"),
    };
    for e in hooked(s, &method, &block, true) {
        deliver(s, seq, out, e);
    }
    let id = request["id"].clone();
    let response = match result {
        Ok(payload) => json!({ "type": "response", "id": id, "seq": answered, "payload": payload }),
        Err(code) => json!({
            "type": "response", "id": id, "seq": answered,
            "error": { "code": code, "message": code, "status": 404 },
        }),
    };
    if s.stall && method == "block.event" {
        s.held.push(response);
    } else {
        out.push(response);
    }
    if let Some(b) = closed {
        deliver(s, seq, out, block_closed(&b.session, &b.id));
    }
}

fn answer_error(request: &Value, seq: u64, out: &mut Vec<Value>, code: &str) {
    out.push(json!({
        "type": "response", "id": request["id"].clone(), "seq": seq,
        "error": { "code": code, "message": code, "status": 400 },
    }));
}

/// A session with one terminal block running the layout's `command`, owned by its creator,
/// announced before the answer as Rex does. The block's foreground process is the program
/// after `env`'s options, in the layout's `cwd`, and reports `start` as that program's app.
fn create(s: &mut State, payload: &Value, seq: &mut u64, out: &mut Vec<Value>) -> Value {
    s.created.push(payload.clone());
    let n = s.created.len();
    let session = format!("session:019a7d2e-0000-7000-8000-0000000c{n:04x}");
    let block = format!("block:019a7d2e-0000-7000-8000-0000000d{n:04x}");
    let options = &payload["initial_windows"][0]["layout"]["block"]["options"];
    let command: Vec<&str> = (options["command"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    let mut i = 1;
    while command.get(i) == Some(&"-u") {
        i += 2;
    }
    let program = command.get(i).copied().unwrap_or_default();
    let cwd = s
        .runs_in
        .clone()
        .map_or_else(|| options["cwd"].clone(), Value::from);
    let pid = 900 + n as u32;
    let label = payload["label"].as_str().unwrap_or_default().to_owned();
    s.sessions.push((session.clone(), label.clone()));
    let proc =
        json!({ "pid": pid, "user_id": 501, "name": "2.1.322", "argv0": program, "cwd": cwd });
    let records = (s.start.into_iter())
        .map(|state| {
            let mut r = record(state, pid, "2.1.322", "2026-11-14T10:00:00.000001Z");
            if program.ends_with("/codex") {
                r["app"] = json!("codex");
            }
            r
        })
        .collect();
    s.add(
        &session,
        &block,
        json!({ "last_exit": null, "child": proc, "foreground": proc }),
        records,
    );
    s.block(&block).owner = Some(CLIENT.to_owned());
    deliver(s, seq, out, session_created(&session, &label));
    json!({ "session_id": session, "initial_windows": [{
        "window_id": "window:019a7d2e-0000-7000-8000-000000000001", "block_ids": [block], "revision": 1,
    }]})
}

fn split(
    s: &mut State,
    session: &str,
    payload: &Value,
    seq: &mut u64,
    out: &mut Vec<Value>,
) -> Value {
    s.splits.push(payload.clone());
    let n = s.splits.len();
    let block = format!("block:019a7d2e-0000-7000-8000-0000000f{n:04x}");
    let options = &payload["layout"]["block"]["options"];
    let command: Vec<&str> = (options["command"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    let mut i = 1;
    while command.get(i) == Some(&"-u") {
        i += 2;
    }
    let program = match command.get(i + 1..i + 4) {
        Some(["-l", "-i", "-c"]) => {
            (command.get(i + 4)).and_then(|c| c.rsplit("&& ").next()?.split(' ').next())
        }
        _ => command.get(i).copied(),
    }
    .unwrap_or_default();
    let pid = s.split_pid.unwrap_or(950 + n as u32);
    let cwd = s
        .split_cwd
        .clone()
        .map_or_else(|| options["cwd"].clone(), Value::from);
    let proc =
        json!({ "pid": pid, "user_id": 501, "name": "2.1.322", "argv0": program, "cwd": cwd });
    let process = match s.split_exits {
        true => json!({ "last_exit": { "exit_code": 1 }, "child": null, "foreground": null }),
        false => json!({ "last_exit": null, "child": proc, "foreground": proc }),
    };
    s.add(session, &block, process, Vec::new());
    if let Some(host) = s.split_host.clone() {
        s.block(&block).host = host;
    }
    s.block(&block).owner = Some(CLIENT.to_owned());
    let created = json!({ "type": "block_created", "payload": { "block_id": block } });
    deliver(s, seq, out, session_event(session, created));
    json!({ "anchor_id": payload["anchor_id"], "block_ids": [block], "split_ids": ["split:1"], "revision": 1 })
}

/// The first part, by `max_bytes` as revision 13 answers it: no `eof`, and an id while more
/// remains. Arguments the tests do not use are ignored.
fn format(s: &mut State, block: &str, screen: String, args: &Value) -> Value {
    let max = args["max_bytes"].as_u64().unwrap_or(0) as usize;
    let total = screen.len();
    if max == 0 || total <= max {
        s.moved += total;
        return json!({ "content": screen, "total": total });
    }
    let content = String::from_utf8_lossy(&screen.as_bytes()[..max]).into_owned();
    s.moved += content.len();
    let id = s.next_result;
    s.next_result += 1;
    s.results.push((id, block.to_owned(), screen));
    let kept: Vec<usize> = (s.results.iter().enumerate())
        .filter(|(_, (_, b, _))| b == block)
        .map(|(i, _)| i)
        .collect();
    if kept.len() > 4 {
        s.results.remove(kept[0]);
    }
    json!({ "content": content, "id": id, "total": total })
}

/// What fits in `max_bytes` once encoded as a JSON string; a start inside a character reads
/// as replacement characters, as Rex answers it.
fn blob_read(s: &mut State, block: &str, args: &Value) -> Result<Value, &'static str> {
    let id = args["id"].as_u64().unwrap_or(0);
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;
    let max = args["max_bytes"].as_u64().unwrap_or(49152) as usize;
    let (_, _, screen) = (s.results.iter())
        .find(|(i, b, _)| *i == id && b == block)
        .ok_or("not_found")?;
    let rest = String::from_utf8_lossy(&screen.as_bytes()[offset.min(screen.len())..]);
    let mut data = String::new();
    let mut size = 2;
    for c in rest.chars() {
        size += serde_json::to_string(&c).unwrap().len() - 2;
        if size > max {
            break;
        }
        data.push(c);
    }
    let eof = data.len() == rest.len();
    let total = screen.len();
    s.moved += data.len();
    Ok(json!({ "id": id, "data": data, "total": total, "eof": eof }))
}

pub(super) fn session_created(id: &str, label: &str) -> Value {
    json!({ "type": "session_created", "payload": { "session": { "session_id": id, "label": label } } })
}

pub(super) fn session_event(session: &str, event: Value) -> Value {
    json!({ "type": "session_event", "payload": { "session_id": session, "event": event } })
}

pub(super) fn block_closed(session: &str, block: &str) -> Value {
    session_event(
        session,
        json!({ "type": "block_closed", "payload": { "block_id": block } }),
    )
}

pub(super) fn block_event(session: &str, block: &str, name: &str, payload: Value) -> Value {
    session_event(
        session,
        json!({ "type": "block_event", "payload": {
            "block_id": block, "creator_name": TERMINAL, "name": name, "payload": payload,
        }}),
    )
}

pub(super) fn status_changed(session: &str, block: &str, record: Value) -> Value {
    block_event(
        session,
        block,
        "program_status_changed",
        json!({ "id": "", "record": record, "reason": "report" }),
    )
}

/// The root record a program reports, owned by the foreground process `pid` named `name`.
pub(super) fn record(state: &str, pid: u32, name: &str, at: &str) -> Value {
    json!({
        "state": state, "app": "claude-code", "updated_at": at,
        "owner": { "pid": pid, "name": name },
    })
}

/// As [`process`], run from a login zsh, as Rex.app's terminals run a command.
pub(super) fn under_shell(pid: u32, argv0: &str, name: &str) -> Value {
    let mut p = process(pid, argv0, name);
    p["ancestors"] = json!([
        { "pid": 2, "user_id": 501, "name": "zsh", "argv0": "-zsh", "cwd": "/work/app" },
        { "pid": 1, "user_id": 0, "name": "login" },
    ]);
    p
}

/// A terminal whose foreground process is `pid`, started as `argv0` and named `name`.
pub(super) fn process(pid: u32, argv0: &str, name: &str) -> Value {
    json!({
        "last_exit": null,
        "child": { "pid": 1, "user_id": 501, "name": "login", "argv0": "login", "cwd": "/work" },
        "foreground": { "pid": pid, "user_id": 501, "name": name, "argv0": argv0, "cwd": "/work/app" },
    })
}
