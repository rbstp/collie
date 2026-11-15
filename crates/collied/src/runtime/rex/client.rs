use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio::sync::oneshot;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use super::wire::{ClientInfo, Hello, Incoming, Request, Response, Tagged, Welcome};
use crate::runtime::Error;

pub(super) const PROTOCOL: &str = "rex.control.v1";
/// The revision whose shapes collied reads, sent as both the newest and the oldest it
/// speaks, so an older server closes the connection rather than answer in older shapes.
pub(super) const API_REVISION: u32 = 13;
pub(super) const TERMINAL: &str = "com.superlogical.terminal";
pub(super) const TIMEOUT: Duration = Duration::from_secs(5);
/// The message limit Rex documents for this revision, and the most collied reads.
const MAX_MESSAGE: usize = 1 << 20;
/// The server pings every 120 s.
const SILENCE: Duration = Duration::from_secs(300);

/// Every method collied calls on a Rex server, and the only way to name one: the API also
/// stops the server, turns on Tailscale, edits the host list, runs Lua, moves files and
/// drives Rex.app, and none of that can be sent because nothing here names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Method {
    HelpAll,
    ServerVersion,
    ServerStatus,
    BlockCreatorList,
    SessionAttachAll,
    SessionList,
    SessionAttach,
    SessionListBlocks,
    BlockClose,
    Process,
    ProgramStatus,
    Title,
    Size,
    Format,
    BlobRead,
}

impl Method {
    pub(super) const ALL: [Self; 15] = [
        Self::HelpAll,
        Self::ServerVersion,
        Self::ServerStatus,
        Self::BlockCreatorList,
        Self::SessionAttachAll,
        Self::SessionList,
        Self::SessionAttach,
        Self::SessionListBlocks,
        Self::BlockClose,
        Self::Process,
        Self::ProgramStatus,
        Self::Title,
        Self::Size,
        Self::Format,
        Self::BlobRead,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::HelpAll => "help.all",
            Self::ServerVersion => "server.version",
            Self::ServerStatus => "server.status",
            Self::BlockCreatorList => "block_creator.list",
            Self::SessionAttachAll => "session.attach_all",
            Self::SessionList => "session.list",
            Self::SessionAttach => "session.attach",
            Self::SessionListBlocks => "session.list_blocks",
            Self::BlockClose => "block.close",
            Self::Process => "com.superlogical.terminal.process",
            Self::ProgramStatus => "com.superlogical.terminal.program_status",
            Self::Title => "com.superlogical.terminal.title",
            Self::Size => "com.superlogical.terminal.size",
            Self::Format => "com.superlogical.terminal.format",
            Self::BlobRead => "com.superlogical.terminal.blob_read",
        }
    }

    /// A terminal block's method, by the name `block_creator.list` gives it.
    pub(super) fn terminal(self) -> Option<&'static str> {
        self.name()
            .strip_prefix(TERMINAL)
            .and_then(|m| m.strip_prefix('.'))
    }
}

type Ws = WebSocketStream<UnixStream>;

/// One control connection. Calls may come from any task; the [`Pump`] matches each response
/// to its call and hands events over without waiting on anyone.
pub(super) struct Client {
    sink: tokio::sync::Mutex<SplitSink<Ws, Message>>,
    pending: Mutex<HashMap<String, oneshot::Sender<Response>>>,
    next: AtomicU64,
}

pub(super) struct Pump {
    stream: SplitStream<Ws>,
}

/// The WebSocket handshake and the hello, on a socket already checked to be a personal
/// server's.
pub(super) async fn open(stream: UnixStream) -> Result<(Client, Pump, Welcome), Error> {
    tokio::time::timeout(TIMEOUT, async {
        let mut request = "ws://localhost/api/control"
            .into_client_request()
            .map_err(ws_error)?;
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", HeaderValue::from_static(PROTOCOL));
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        let (mut ws, _) =
            tokio_tungstenite::client_async_with_config(request, stream, Some(config))
                .await
                .map_err(ws_error)?;
        let hello = Hello {
            kind: "hello",
            api_revision: API_REVISION,
            api_min_revision: API_REVISION,
            client: ClientInfo {
                kind: "other",
                name: "collied",
                version: env!("CARGO_PKG_VERSION"),
            },
        };
        ws.send(Message::text(serde_json::to_string(&hello)?))
            .await
            .map_err(ws_error)?;
        let welcome = loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => break serde_json::from_str::<Incoming>(&text)?,
                Some(Ok(Message::Close(frame))) => return Err(refused(frame)),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(ws_error(e)),
                None => return Err(Error::Closed),
            }
        };
        let Incoming::Welcome(welcome) = welcome else {
            return Err(Error::Unsupported(
                "did not open the connection with a welcome".to_owned(),
            ));
        };
        if welcome.protocol != PROTOCOL || welcome.api_revision != API_REVISION {
            return Err(Error::Unsupported(format!(
                "answered with {} at API revision {}, not {PROTOCOL} at {API_REVISION}",
                crate::printable(&welcome.protocol),
                welcome.api_revision
            )));
        }
        let (sink, stream) = ws.split();
        Ok((
            Client {
                sink: tokio::sync::Mutex::new(sink),
                pending: Mutex::default(),
                next: AtomicU64::new(1),
            },
            Pump { stream },
            welcome,
        ))
    })
    .await
    .map_err(|_| Error::Timeout)?
}

/// The plain words `collied doctor` shows: a server keeps the revision of the build it
/// started from until it restarts.
pub(super) fn revision_refused(reason: &str) -> String {
    format!(
        "does not speak API revision {API_REVISION}, which collied needs; a Rex server keeps the API revision of the build it started from, so restart the Rex server (the server said: {reason})"
    )
}

/// Before the welcome, the server closes with a policy violation only when it shares no API
/// revision with collied; its reason's wording is its own.
fn refused(frame: Option<CloseFrame>) -> Error {
    match frame {
        Some(f) if f.code == CloseCode::Policy => {
            Error::Unsupported(revision_refused(&crate::printable(&f.reason)))
        }
        _ => Error::Closed,
    }
}

fn ws_error(e: WsError) -> Error {
    match e {
        WsError::Io(io) => Error::Io(io),
        WsError::Capacity(_) => Error::LineTooLong,
        WsError::ConnectionClosed | WsError::AlreadyClosed => Error::Closed,
        other => Error::Io(std::io::Error::other(other.to_string())),
    }
}

impl Client {
    /// The response's `seq`, the last event the result already reflects, and its payload.
    pub(super) async fn call<R: DeserializeOwned>(
        &self,
        method: Method,
        payload: Value,
        wait: Duration,
    ) -> Result<(u64, R), Error> {
        let id = self.next.fetch_add(1, Ordering::Relaxed).to_string();
        let (tx, rx) = oneshot::channel();
        crate::lock(&self.pending).insert(id.clone(), tx);
        tracing::debug!(method = method.name(), id, "Rex request");
        let sent = async {
            let request = Request {
                kind: "request",
                id: &id,
                method: method.name(),
                payload,
            };
            let text = serde_json::to_string(&request)?;
            self.sink
                .lock()
                .await
                .send(Message::text(text))
                .await
                .map_err(ws_error)?;
            rx.await.map_err(|_| Error::Closed)
        };
        let reply = tokio::time::timeout(wait, sent).await;
        crate::lock(&self.pending).remove(&id);
        let reply = reply.map_err(|_| Error::Timeout)??;
        if let Some(e) = reply.error {
            return Err(Error::Rex {
                code: e.code,
                message: crate::printable(&e.message),
            });
        }
        Ok((
            reply.seq,
            serde_json::from_value(reply.payload.unwrap_or(Value::Null))?,
        ))
    }

    pub(super) async fn terminal<R: DeserializeOwned>(
        &self,
        method: Method,
        session: &str,
        block: &str,
        args: Value,
    ) -> Result<(u64, R), Error> {
        let payload = json!({ "session_id": session, "block_id": block, "args": args });
        self.call(method, payload, TIMEOUT).await
    }
}

impl Pump {
    /// Reads until the connection ends, which the error says why. Holds no lock across an
    /// await and never waits on a caller: a response goes to its call's channel, an event to
    /// `event`, which must not block.
    pub(super) async fn run(
        mut self,
        client: &Client,
        mut event: impl FnMut(u64, Tagged),
    ) -> Error {
        let ended = loop {
            let message = match tokio::time::timeout(SILENCE, self.stream.next()).await {
                Err(_) => break Error::Timeout,
                Ok(None) => break Error::Closed,
                Ok(Some(Err(e))) => break ws_error(e),
                Ok(Some(Ok(m))) => m,
            };
            let text = match message {
                Message::Text(text) => text,
                Message::Close(_) => break Error::Closed,
                _ => continue,
            };
            match serde_json::from_str::<Incoming>(&text) {
                Ok(Incoming::Response(r)) => {
                    if let Some(tx) = crate::lock(&client.pending).remove(&r.id) {
                        let _ = tx.send(r);
                    }
                }
                Ok(Incoming::Event(e)) => event(e.seq, e.payload),
                Ok(_) => {}
                Err(e) => break Error::Json(e),
            }
        };
        crate::lock(&client.pending).clear();
        ended
    }
}
