use std::ffi::c_void;
use std::os::raw::c_int;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use ghostty_vt_sys as vt;
use serde::Deserialize;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use super::client::{Method, TIMEOUT, ws_error};
use super::screen::DETECTION;
use super::{RETRY_FIRST, Rex, discovery};
use crate::runtime::Error;
use crate::runtime::screen::Screen;

const PROTOCOL: &str = "rex.data.terminal.v1";
/// Rex sends at most 64 KiB per message.
const MAX_MESSAGE: usize = 64 << 10;
const HISTORY: usize = protocol::limits::MAX_READ_LINES as usize;
/// The line limit prunes whole pages and can keep fewer lines than asked (measured in #739).
const KEEP: usize = HISTORY * 2 + 1024;
/// A `rex1` message longer than this is not one of Rex's: its bytes are written as they are.
const MAX_APC: usize = 4096;

pub(super) type Slot = Arc<Mutex<Option<Ghost>>>;

/// A watched terminal followed as a stream; the stream ends with the last watch.
pub(super) struct Hold {
    pub(super) screen: Slot,
    pub(super) cut: Arc<tokio::sync::Notify>,
    task: tokio::task::AbortHandle,
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub struct Watch {
    _hold: Arc<Hold>,
}

impl Watch {
    pub(super) fn new(hold: Arc<Hold>) -> Self {
        Self { _hold: hold }
    }
}

pub(super) fn hold(rex: Arc<Rex>, id: String) -> Arc<Hold> {
    let screen = Slot::default();
    let cut = Arc::new(tokio::sync::Notify::new());
    let task = tokio::spawn(follow(rex, id, screen.clone(), cut.clone()));
    Arc::new(Hold {
        screen,
        cut,
        task: task.abort_handle(),
    })
}

enum Ended {
    /// Reads poll `format` for the rest of the watch.
    Unsupported(String),
    /// Reads poll `format` until a new stream is restored.
    Dropped(Error),
}

async fn follow(rex: Arc<Rex>, id: String, slot: Slot, cut: Arc<tokio::sync::Notify>) {
    loop {
        let ended = once(&rex, &id, &slot, &cut).await;
        *crate::lock(&slot) = None;
        match ended {
            Ended::Unsupported(why) => {
                tracing::warn!(id, "Rex stream unavailable, polling instead: {why}");
                return;
            }
            Ended::Dropped(e) if rex.target(&id).is_err() => {
                tracing::debug!(id, error = %e, "Rex stream ended with its block");
                return;
            }
            Ended::Dropped(e) => {
                tracing::debug!(id, error = %e, "Rex stream dropped, following again");
                tokio::time::sleep(RETRY_FIRST).await;
            }
        }
    }
}

#[derive(Deserialize)]
struct Ticket {
    ticket: String,
}

async fn once(rex: &Rex, id: &str, slot: &Slot, cut: &tokio::sync::Notify) -> Ended {
    let dropped = Ended::Dropped;
    let (client, session, block, _) = match rex.target(id) {
        Ok(t) => t,
        Err(e) => return dropped(e),
    };
    let args = json!({
        "session_id": session, "block_id": block, "data_ref_id": "terminal", "subprotocol": PROTOCOL,
    });
    let ticket = match client
        .call::<Ticket>(Method::DataConnect, args, TIMEOUT)
        .await
    {
        Ok((_, t)) => t.ticket,
        Err(e) => return dropped(e),
    };
    let Ok(ticket) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(ticket) else {
        return Ended::Unsupported("the data ticket is not base64url".to_owned());
    };
    let stream = match discovery::connect(&rex.socket).await {
        Ok(s) => s,
        Err(e) => return dropped(e),
    };
    let opened = tokio::time::timeout(TIMEOUT, async {
        let mut request = "ws://localhost/api/data"
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
        let mut hello = vec![0x01];
        hello.extend_from_slice(&ticket);
        ws.send(Message::binary(hello)).await.map_err(ws_error)?;
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => return Ok((ws, b)),
                Some(Ok(Message::Close(_))) | None => return Err(Error::Closed),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(ws_error(e)),
            }
        }
    })
    .await;
    let (mut ws, welcome) = match opened {
        Ok(Ok(opened)) => opened,
        Ok(Err(e)) => return dropped(e),
        Err(_) => return dropped(Error::Timeout),
    };
    if welcome.len() != 32 || welcome[0] != 0x81 {
        return Ended::Unsupported(format!(
            "the data connection was refused (reason {})",
            welcome.get(1).copied().unwrap_or(0)
        ));
    }
    let nonce = u64::from_le_bytes(welcome[24..32].try_into().unwrap_or_default());

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let mut restore = tokio::task::spawn_blocking(move || Ghost::restore(rx));
    let restored = loop {
        tokio::select! {
            done = &mut restore => break done,
            next = ws.next() => match next {
                Some(Ok(Message::Binary(b))) => { let _ = tx.send(b.to_vec()); }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    drop(tx);
                    return match restore.await {
                        Ok(Ok(_)) | Err(_) => dropped(Error::Closed),
                        Ok(Err(why)) => dropped(Error::Unsupported(why)),
                    };
                }
                Some(Ok(_)) => {}
            },
        }
    };
    let (ghost, mut pending, rx) = match restored {
        Ok(Ok(restored)) => restored,
        Ok(Err(why)) => return Ended::Unsupported(why),
        Err(e) => return dropped(Error::Unsupported(e.to_string())),
    };
    drop(tx);
    pending.extend(rx.try_iter().flatten());
    let mut live = Live {
        nonce: nonce.to_string(),
        carry: Vec::new(),
    };
    *crate::lock(slot) = Some(ghost);
    apply(slot, &mut live, &pending);
    drop(pending);
    loop {
        let next = tokio::select! {
            next = ws.next() => next,
            () = cut.notified() => return dropped(Error::Closed),
        };
        match next {
            Some(Ok(Message::Binary(b))) => apply(slot, &mut live, &b),
            Some(Ok(Message::Close(frame))) => {
                let why = frame.map_or_else(String::new, |f| crate::printable(&f.reason));
                return dropped(Error::Rex {
                    code: "closed".to_owned(),
                    message: why,
                });
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => return dropped(ws_error(e)),
            None => return dropped(Error::Closed),
        }
    }
}

/// Where the stream is between Rex's own messages: `ESC _ rex1;<nonce>;<json> ESC \`.
struct Live {
    nonce: String,
    carry: Vec<u8>,
}

#[derive(Deserialize)]
struct RexMessage {
    #[serde(rename = "type")]
    kind: String,
    cols: Option<u16>,
    rows: Option<u16>,
}

fn apply(slot: &Slot, live: &mut Live, bytes: &[u8]) {
    let mut data = std::mem::take(&mut live.carry);
    data.extend_from_slice(bytes);
    let mut guard = crate::lock(slot);
    let Some(ghost) = guard.as_mut() else {
        return;
    };
    let mut rest = &data[..];
    while let Some(at) = find(rest, b"\x1b_rex1;") {
        let Some(len) = find(&rest[at..], b"\x1b\\") else {
            if rest.len() - at <= MAX_APC {
                ghost.write(&rest[..at]);
                live.carry = rest[at..].to_vec();
                return;
            }
            break;
        };
        let end = at + len + 2;
        ghost.write(&rest[..end]);
        let body = &rest[at + 7..at + len];
        if let Some(json) =
            (body.strip_prefix(live.nonce.as_bytes())).and_then(|b| b.strip_prefix(b";"))
            && let Ok(m) = serde_json::from_slice::<RexMessage>(json)
        {
            match (m.kind.as_str(), m.cols, m.rows) {
                ("resize", Some(cols), Some(rows)) => ghost.resize(cols, rows),
                ("child_exited", ..) => tracing::debug!("Rex stream: the program exited"),
                _ => {}
            }
        }
        rest = &rest[end..];
    }
    // A message cut at the end of this piece waits for the next one.
    let tail = rest.len().min(7);
    match (1..=tail)
        .rev()
        .find(|&n| b"\x1b_rex1;".starts_with(&rest[rest.len() - n..]))
    {
        Some(n) => {
            ghost.write(&rest[..rest.len() - n]);
            live.carry = rest[rest.len() - n..].to_vec();
        }
        None => ghost.write(rest),
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// A Rex terminal restored from its snapshot by libghostty-vt.
pub(super) struct Ghost {
    terminal: vt::GhosttyTerminal,
}

// A terminal is used by one thread at a time, behind the slot's mutex.
unsafe impl Send for Ghost {}

impl Drop for Ghost {
    fn drop(&mut self) {
        unsafe { vt::ghostty_terminal_free(self.terminal) }
    }
}

struct Source {
    rx: mpsc::Receiver<Vec<u8>>,
    seen: Vec<u8>,
    given: usize,
}

unsafe extern "C" fn read_source(
    userdata: *mut c_void,
    buffer: *mut u8,
    capacity: usize,
    out: *mut usize,
) -> bool {
    let source = unsafe { &mut *userdata.cast::<Source>() };
    while source.given == source.seen.len() {
        match source.rx.recv() {
            Ok(bytes) => source.seen.extend_from_slice(&bytes),
            Err(_) => {
                unsafe { *out = 0 };
                return true;
            }
        }
    }
    let n = capacity.min(source.seen.len() - source.given);
    unsafe { std::ptr::copy_nonoverlapping(source.seen.as_ptr().add(source.given), buffer, n) };
    source.given += n;
    unsafe { *out = n };
    true
}

type Restored = (Ghost, Vec<u8>, mpsc::Receiver<Vec<u8>>);

impl Ghost {
    /// Blocks on `rx` until the snapshot is read; returns the terminal, the bytes read after
    /// it, and `rx` for the bytes still queued.
    fn restore(rx: mpsc::Receiver<Vec<u8>>) -> Result<Restored, String> {
        let mut source = Box::new(Source {
            rx,
            seen: Vec::new(),
            given: 0,
        });
        let reader = vt::GhosttyReader {
            read: read_source,
            userdata: std::ptr::from_mut::<Source>(&mut source).cast(),
        };
        let mut decoder = std::ptr::null_mut();
        if unsafe { vt::ghostty_snapshot_decoder_new(std::ptr::null(), &mut decoder, reader) }
            != vt::GHOSTTY_SUCCESS
        {
            return Err("the snapshot decoder could not start".to_owned());
        }
        let mut terminal = std::ptr::null_mut();
        let decoded = unsafe { vt::ghostty_snapshot_decoder_decode(decoder, &mut terminal) };
        let mut offset = 0usize;
        unsafe {
            vt::ghostty_snapshot_decoder_get(
                decoder,
                vt::GHOSTTY_SNAPSHOT_DECODER_DATA_SOURCE_OFFSET,
                std::ptr::from_mut(&mut offset).cast(),
            );
            vt::ghostty_snapshot_decoder_free(decoder);
        }
        if decoded != vt::GHOSTTY_SUCCESS || terminal.is_null() {
            return Err(format!(
                "libghostty-vt could not read the snapshot (result {decoded}): an envelope version it does not know, or a cut stream"
            ));
        }
        let ghost = Ghost { terminal };
        unsafe {
            vt::ghostty_terminal_set(
                terminal,
                vt::GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES,
                std::ptr::from_ref(&KEEP).cast(),
            );
        }
        let Source { rx, seen, .. } = *source;
        let after = seen.get(offset..).unwrap_or_default().to_vec();
        Ok((ghost, after, rx))
    }

    fn write(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            unsafe { vt::ghostty_terminal_vt_write(self.terminal, bytes.as_ptr(), bytes.len()) }
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        unsafe { vt::ghostty_terminal_resize(self.terminal, cols, rows, 1, 1) };
    }

    fn get<T: Default>(&self, data: c_int) -> T {
        let mut v = T::default();
        unsafe { vt::ghostty_terminal_get(self.terminal, data, std::ptr::from_mut(&mut v).cast()) };
        v
    }

    /// As a poll reads it: the VT of the rows a read needs, as Rex's `format` writes it,
    /// emulated by collied's `Screen`.
    pub(super) fn read(&self, source: &str, lines: Option<u32>) -> String {
        let cols: u16 = self.get(vt::GHOSTTY_TERMINAL_DATA_COLS);
        let rows: u16 = self.get(vt::GHOSTTY_TERMINAL_DATA_ROWS);
        let total: usize = self.get(vt::GHOSTTY_TERMINAL_DATA_TOTAL_ROWS);
        let screen_only = matches!(source, "visible" | DETECTION);
        let history = match screen_only {
            true => 0,
            false => lines.map_or(HISTORY, |n| n as usize).min(HISTORY),
        };
        // Twice the rows, so wrapped lines still give `history` lines once unwrapped.
        let want = match screen_only {
            true => usize::from(rows),
            false => (usize::from(rows) + history) * 2,
        };
        let vt = self.format(total.saturating_sub(want), total, cols, !screen_only);
        let mut screen = Screen::new(cols, rows, history);
        screen.feed(&vt);
        match source {
            DETECTION => screen.text(),
            _ => screen.read(source, lines),
        }
    }

    fn format(&self, first: usize, end: usize, cols: u16, unwrap: bool) -> Vec<u8> {
        let point = |x: u16, y: usize| vt::GhosttyPoint {
            tag: vt::GHOSTTY_POINT_TAG_SCREEN,
            value: vt::GhosttyPointValue {
                coordinate: vt::GhosttyPointCoordinate { x, y: y as u32 },
            },
        };
        let empty = vt::GhosttyGridRef {
            node: std::ptr::null_mut(),
            x: 0,
            y: 0,
        };
        let mut selection = vt::GhosttySelection {
            start: empty,
            end: empty,
            rectangle: false,
        };
        let mut out = Vec::new();
        unsafe {
            if end == 0
                || vt::ghostty_terminal_grid_ref(
                    self.terminal,
                    point(0, first),
                    &mut selection.start,
                ) != vt::GHOSTTY_SUCCESS
                || vt::ghostty_terminal_grid_ref(
                    self.terminal,
                    point(cols.saturating_sub(1), end - 1),
                    &mut selection.end,
                ) != vt::GHOSTTY_SUCCESS
            {
                return out;
            }
            let mut f = std::ptr::null_mut();
            if vt::ghostty_formatter_terminal_new(std::ptr::null(), &mut f, self.terminal)
                != vt::GHOSTTY_SUCCESS
            {
                return out;
            }
            let extras: u32 = 0;
            vt::ghostty_formatter_set(
                f,
                vt::GHOSTTY_FORMATTER_OPT_EMIT,
                std::ptr::from_ref(&vt::GHOSTTY_FORMATTER_FORMAT_VT).cast(),
            );
            vt::ghostty_formatter_set(
                f,
                vt::GHOSTTY_FORMATTER_OPT_UNWRAP,
                std::ptr::from_ref(&unwrap).cast(),
            );
            vt::ghostty_formatter_set(
                f,
                vt::GHOSTTY_FORMATTER_OPT_EXTRAS,
                std::ptr::from_ref(&extras).cast(),
            );
            vt::ghostty_formatter_set(
                f,
                vt::GHOSTTY_FORMATTER_OPT_SELECTION,
                std::ptr::from_ref(&selection).cast(),
            );
            let mut ptr = std::ptr::null_mut();
            let mut len = 0usize;
            if vt::ghostty_formatter_format_alloc(f, std::ptr::null(), &mut ptr, &mut len)
                == vt::GHOSTTY_SUCCESS
            {
                out.extend_from_slice(std::slice::from_raw_parts(ptr, len));
                vt::ghostty_free(std::ptr::null(), ptr, len);
            }
            vt::ghostty_formatter_free(f);
        }
        out
    }
}
