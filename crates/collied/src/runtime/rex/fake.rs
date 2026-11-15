//! A `rex.control.v1` server of the tests' own, in process, on a Unix socket in a temporary
//! folder: CI has no Rex, and no test reaches the user's.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use super::client::{Method, TERMINAL};

pub(super) const HOST: &str = "1f4bd4b7-e04f-46e2-ad3b-0e2b89b5447c";

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
}

pub(super) struct State {
    /// Every method called, in order, `hello` first.
    pub calls: Vec<String>,
    pub hello: Value,
    pub refuse_revision: bool,
    pub methods: Vec<&'static str>,
    pub terminal: Vec<&'static str>,
    pub listeners: Value,
    pub tailscale: &'static str,
    pub sessions: Vec<(String, String)>,
    pub blocks: Vec<Block>,
    hooks: Vec<(&'static str, Option<&'static str>, bool, Hook)>,
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
        });
    }
}

enum Push {
    Events(Hook),
    Drop,
}

pub(super) struct Fake {
    _dir: tempfile::TempDir,
    pub socket: PathBuf,
    state: Arc<Mutex<State>>,
    push: Arc<Mutex<Option<mpsc::UnboundedSender<Push>>>>,
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
            refuse_revision: false,
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
        }));
        let push = Arc::new(Mutex::new(None));
        let task = tokio::spawn({
            let (state, push) = (state.clone(), push.clone());
            async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let (tx, rx) = mpsc::unbounded_channel();
                    *crate::lock(&push) = Some(tx);
                    tokio::spawn(serve(state.clone(), stream, rx));
                }
            }
        });
        Self {
            _dir: dir,
            socket,
            state,
            push,
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
}

#[allow(clippy::result_large_err)]
fn protocol(_: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
    response.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static("rex.control.v1"),
    );
    Ok(response)
}

async fn serve(
    state: Arc<Mutex<State>>,
    stream: tokio::net::UnixStream,
    mut pushed: mpsc::UnboundedReceiver<Push>,
) {
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, protocol).await else {
        return;
    };
    let (mut sink, mut source) = ws.split();
    let Some(Ok(Message::Text(hello))) = source.next().await else {
        return;
    };
    let refuse = {
        let mut s = crate::lock(&state);
        s.calls.push("hello".to_owned());
        s.hello = serde_json::from_str(&hello).unwrap();
        s.refuse_revision
    };
    if refuse {
        let frame = CloseFrame {
            code: CloseCode::Policy,
            reason: "no common API revision: the client needs 13 or newer, this server speaks 0 through 12".into(),
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
                answer(&mut s, &request, &mut seq, &mut out);
            }
            pushed = pushed.recv() => {
                let Some(Push::Events(f)) = pushed else { return };
                let events = f(&mut crate::lock(&state));
                for e in events {
                    seq += 1;
                    out.push(json!({ "type": "event", "seq": seq, "payload": e }));
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

fn answer(s: &mut State, request: &Value, seq: &mut u64, out: &mut Vec<Value>) {
    let method = request["method"].as_str().unwrap().to_owned();
    let payload = &request["payload"];
    s.calls.push(method.clone());
    let event = |seq: &mut u64, out: &mut Vec<Value>, e: Value| {
        *seq += 1;
        out.push(json!({ "type": "event", "seq": *seq, "payload": e }));
    };
    let session = payload["session_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let block = payload["block_id"].as_str().unwrap_or_default().to_owned();
    for e in hooked(s, &method, &block, false) {
        event(seq, out, e);
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
            "pid": std::process::id(), "instance_id": "i", "host_id": HOST,
            "started_at": "2026-11-12T16:47:01.264128Z", "exit_empty": false,
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
            Ok(json!({ "client_id": "client:1", "view": {"revision": 1, "session_id": session} }))
        }
        ("session.list_blocks", _) if has_session => {
            let blocks: Vec<Value> = (s.blocks.iter())
                .filter(|b| b.session == session)
                .map(|b| {
                    json!({
                        "block_id": b.id, "label": "shell", "creator_name": b.creator,
                        "host_id": b.host, "placement": "placed",
                    })
                })
                .collect();
            Ok(json!({ "blocks": blocks }))
        }
        ("block.close", _) => match found {
            Some(i) => {
                closed = Some(s.blocks.remove(i));
                Ok(json!({ "block_id": block, "revision": 2 }))
            }
            None => Err("not_found"),
        },
        (_, Some(m)) if s.terminal.contains(&m) => match found.map(|i| &s.blocks[i]) {
            Some(b) => match m {
                "process" => Ok(b.process.clone()),
                "program_status" => Ok(json!({ "records": b.records })),
                "title" => Ok(json!({ "title": b.title })),
                _ => Err("not_found"),
            },
            None => Err("not_found"),
        },
        (m, _) if m.starts_with("session.") && !has_session => Err("session_not_found"),
        _ => Err("method_not_found"),
    };
    for e in hooked(s, &method, &block, true) {
        event(seq, out, e);
    }
    let id = request["id"].clone();
    out.push(match result {
        Ok(payload) => json!({ "type": "response", "id": id, "seq": answered, "payload": payload }),
        Err(code) => json!({
            "type": "response", "id": id, "seq": answered,
            "error": { "code": code, "message": code, "status": 404 },
        }),
    });
    if let Some(b) = closed {
        event(seq, out, block_closed(&b.session, &b.id));
    }
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

/// A terminal whose foreground process is `pid`, started as `argv0` and named `name`.
pub(super) fn process(pid: u32, argv0: &str, name: &str) -> Value {
    json!({
        "last_exit": null,
        "child": { "pid": 1, "user_id": 501, "name": "login", "argv0": "login", "cwd": "/work" },
        "foreground": { "pid": pid, "user_id": 501, "name": name, "argv0": argv0, "cwd": "/work/app" },
    })
}
