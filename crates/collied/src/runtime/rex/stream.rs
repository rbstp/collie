use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::net::UnixStream;
use tokio::sync::oneshot;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use super::client::{Method, TIMEOUT, ws_error};
use super::screen::{emulate, history, screen_only};
use super::wire::Ticket;
use super::{RETRY_FIRST, RETRY_MAX, Rex, discovery};
use crate::runtime::Error;
use crate::runtime::ghostty::{Refused, Terminal};

const PROTOCOL: &str = "rex.data.terminal.v1";
/// Rex sends at most 64 KiB per message.
const MAX_MESSAGE: usize = 64 << 10;
const HISTORY: usize = protocol::limits::MAX_READ_LINES as usize;
/// History kept once restored: a read formats up to twice its rows and history, and the line
/// limit prunes whole pages, which can keep fewer lines than asked (measured in #739).
const KEEP: usize = HISTORY * 2 + 1024;
/// Rex's own messages are a few hundred bytes; a longer one is written through, not held.
const MAX_REX_MESSAGE: usize = 64 << 10;

type Ws = WebSocketStream<UnixStream>;

/// Keeps a watched terminal followed as a stream; the stream ends with the last watch.
pub struct Watch {
    _hold: Arc<Hold>,
}

struct Hold {
    screen: Slot,
    /// Dropped with the last watch, which closes the stream.
    _stop: oneshot::Sender<()>,
}

/// The restored terminal while the stream is live, and none while it is not: reads then
/// poll `format`, which covers any gap.
type Slot = Arc<Mutex<Option<Streamed>>>;

/// Watched terminals, by agent id.
#[derive(Default)]
pub(super) struct Streams {
    held: Mutex<HashMap<String, Weak<Hold>>>,
}

impl Streams {
    /// One stream per terminal, shared by every watch of it.
    pub(super) fn watch(&self, rex: &Arc<Rex>, id: &str) -> Watch {
        let mut held = crate::lock(&self.held);
        held.retain(|_, h| h.strong_count() > 0);
        if let Some(h) = held.get(id).and_then(Weak::upgrade) {
            return Watch { _hold: h };
        }
        let (stop, stopped) = oneshot::channel();
        let screen = Slot::default();
        tokio::spawn(follow(rex.clone(), id.to_owned(), screen.clone(), stopped));
        let h = Arc::new(Hold {
            screen,
            _stop: stop,
        });
        held.insert(id.to_owned(), Arc::downgrade(&h));
        Watch { _hold: h }
    }

    /// From the stream while a watch holds it live.
    pub(super) fn read(&self, id: &str, source: &str, lines: Option<u32>) -> Option<String> {
        let hold = crate::lock(&self.held).get(id).and_then(Weak::upgrade)?;
        crate::lock(&hold.screen)
            .as_mut()
            .map(|s| s.read(source, lines))
    }
}

/// A terminal restored from Rex's snapshot and kept current by the stream.
pub(super) struct Streamed {
    terminal: Terminal,
    /// Moves with every piece of the stream: a read formats only once it moved.
    changed: u64,
    reads: HashMap<(String, Option<u32>), (u64, String)>,
    exited: bool,
    #[cfg(test)]
    formatted: usize,
}

#[derive(Deserialize)]
struct RexMessage {
    #[serde(rename = "type")]
    kind: String,
    cols: Option<u16>,
    rows: Option<u16>,
}

impl Streamed {
    pub(super) fn new(mut terminal: Terminal) -> Self {
        terminal.keep_lines(KEEP);
        Self {
            terminal,
            changed: 0,
            reads: HashMap::new(),
            exited: false,
            #[cfg(test)]
            formatted: 0,
        }
    }

    pub(super) fn apply(&mut self, messages: &mut Messages, bytes: &[u8]) {
        messages.split(bytes, |piece| match piece {
            Piece::Output(b) => self.terminal.write(b),
            Piece::Message(json) => match serde_json::from_slice::<RexMessage>(json) {
                Ok(RexMessage {
                    kind,
                    cols: Some(cols),
                    rows: Some(rows),
                }) if kind == "resize" => self.terminal.resize(cols, rows),
                Ok(m) if m.kind == "child_exited" => self.exited = true,
                // `theme`, and types this build does not know.
                _ => {}
            },
        });
        self.changed += 1;
    }

    /// As a poll reads it: the rows a read needs, as VT the way Rex's `format` writes it,
    /// through the same emulation.
    pub(super) fn read(&mut self, source: &str, lines: Option<u32>) -> String {
        let screen_only = screen_only(source);
        let key = (source.to_owned(), lines.filter(|_| !screen_only));
        if let Some((at, text)) = self.reads.get(&key)
            && *at == self.changed
        {
            return text.clone();
        }
        let (cols, rows) = self.terminal.size();
        let want = match screen_only {
            true => usize::from(rows),
            // Twice the rows, so wrapped lines still fill the history once unwrapped.
            false => (usize::from(rows) + history(source, lines) as usize) * 2,
        };
        #[cfg(test)]
        {
            self.formatted += 1;
        }
        let first = self.terminal.total_rows().saturating_sub(want);
        let vt = self.terminal.vt(first, !screen_only);
        let text = emulate(&vt, (cols, rows), source, lines);
        let changed = self.changed;
        self.reads.retain(|_, (at, _)| *at == changed);
        self.reads.insert(key, (changed, text.clone()));
        text
    }
}

/// Rex's own messages in the stream, `ESC _ rex1;<nonce>;<json> ESC \`. A program that
/// prints a look-alike cannot know the channel's nonce, so its bytes reach the terminal as
/// output, as they reached Rex's.
pub(super) struct Messages {
    prefix: Vec<u8>,
    carry: Vec<u8>,
}

pub(super) enum Piece<'a> {
    Output(&'a [u8]),
    Message(&'a [u8]),
}

impl Messages {
    pub(super) fn new(nonce: u64) -> Self {
        Self {
            prefix: format!("\x1b_rex1;{nonce};").into_bytes(),
            carry: Vec::new(),
        }
    }

    /// Hands `each` the output and the messages of `bytes` in order. A message, or the start
    /// of one, cut at the end of `bytes` waits for the next piece.
    pub(super) fn split(&mut self, bytes: &[u8], mut each: impl FnMut(Piece)) {
        let joined;
        let mut rest = if self.carry.is_empty() {
            bytes
        } else {
            joined = [std::mem::take(&mut self.carry).as_slice(), bytes].concat();
            &joined[..]
        };
        loop {
            let Some(at) = find(rest, &self.prefix) else {
                let cut = (1..self.prefix.len())
                    .rev()
                    .find(|&n| rest.ends_with(&self.prefix[..n]))
                    .unwrap_or(0);
                output(&mut each, &rest[..rest.len() - cut]);
                self.carry = rest[rest.len() - cut..].to_vec();
                return;
            };
            let start = at + self.prefix.len();
            let body = &rest[start..];
            match body.iter().position(|&b| b == 0x1b) {
                Some(esc) if body.get(esc + 1) == Some(&b'\\') => {
                    output(&mut each, &rest[..at]);
                    each(Piece::Message(&body[..esc]));
                    rest = &body[esc + 2..];
                }
                Some(esc) if esc + 1 < body.len() => {
                    output(&mut each, &rest[..start + esc]);
                    rest = &body[esc..];
                }
                _ if body.len() > MAX_REX_MESSAGE => {
                    output(&mut each, &rest[..start]);
                    rest = body;
                }
                _ => {
                    output(&mut each, &rest[..at]);
                    self.carry = rest[at..].to_vec();
                    return;
                }
            }
        }
    }
}

fn output(each: &mut impl FnMut(Piece), bytes: &[u8]) {
    if !bytes.is_empty() {
        each(Piece::Output(bytes));
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Why a stream ended.
enum End {
    /// The last watch ended.
    Stopped,
    /// Reads poll `format` for the rest of the watch.
    Poll(String),
    /// Nothing more comes: the block closed, or its program exited.
    Over,
    /// A new stream, with a new ticket.
    Restart(Error),
    /// The connection dropped: the stream resumes on a new one.
    Dropped(Error),
}

/// A live data connection and what resuming its stream takes.
struct Channel {
    ws: Ws,
    token: [u8; 16],
    nonce: u64,
    /// Every stream byte received, across the stream's connections: a resume names it.
    received: u64,
}

enum Answer {
    Welcome { token: [u8; 16], nonce: u64 },
    Reject(u8),
}

/// WELCOME: `0x81`, the reconnect token, 3 zero bytes, the resume window, the nonce.
/// REJECT: `0x82` and a reason. Little-endian.
fn answer(b: &[u8]) -> Option<Answer> {
    match b {
        [0x81, ..] if b.len() == 32 => Some(Answer::Welcome {
            token: b[1..17].try_into().ok()?,
            nonce: u64::from_le_bytes(b[24..32].try_into().ok()?),
        }),
        [0x82, reason] => Some(Answer::Reject(*reason)),
        _ => None,
    }
}

/// Opens a data connection and sends `first`, a HELLO or a RESUME.
async fn handshake(socket: &Path, first: Vec<u8>) -> Result<(Ws, Answer), Error> {
    let stream = discovery::connect(socket).await?;
    tokio::time::timeout(TIMEOUT, async {
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
        ws.send(Message::binary(first)).await.map_err(ws_error)?;
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => {
                    let answer = answer(&b).ok_or_else(|| {
                        Error::Unsupported(
                            "answered a data handshake in an unknown form".to_owned(),
                        )
                    })?;
                    return Ok((ws, answer));
                }
                Some(Ok(Message::Close(_))) | None => return Err(Error::Closed),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(ws_error(e)),
            }
        }
    })
    .await
    .map_err(|_| Error::Timeout)?
}

/// A new stream: a ticket, a HELLO, and the snapshot restored while the bytes after it queue.
async fn start(rex: &Rex, id: &str) -> Result<(Channel, Streamed, Messages), End> {
    let (client, session, block, _) = rex.target(id).map_err(End::Restart)?;
    let args = json!({
        "session_id": session, "block_id": block, "data_ref_id": "terminal", "subprotocol": PROTOCOL,
    });
    let (_, Ticket { ticket }) = client
        .call(Method::DataConnect, args, TIMEOUT)
        .await
        .map_err(End::Restart)?;
    let ticket = (base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(ticket)
        .ok())
    .and_then(|t| <[u8; 16]>::try_from(t).ok())
    .ok_or_else(|| End::Poll("Rex gave a data ticket of an unknown form".to_owned()))?;
    let mut hello = vec![0x01];
    hello.extend_from_slice(&ticket);
    let (mut ws, answer) = handshake(&rex.socket, hello).await.map_err(End::Restart)?;
    let (token, nonce) = match answer {
        Answer::Welcome { token, nonce } => (token, nonce),
        // 0x02: the previous connection of this client to the block is still closing. 0x04:
        // a limit, try later.
        Answer::Reject(r @ (0x02 | 0x04)) => {
            return Err(End::Restart(Error::Unsupported(format!(
                "refused the data connection for now (reason {r})"
            ))));
        }
        // The block closed.
        Answer::Reject(0x03) => return Err(End::Over),
        Answer::Reject(r) => {
            return Err(End::Poll(format!(
                "refused the data connection (reason {r})"
            )));
        }
    };

    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let mut restore = tokio::task::spawn_blocking(move || {
        let mut next = || rx.recv().ok();
        let restored = Terminal::restore(&mut next);
        (restored, rx)
    });
    let mut tx = Some(tx);
    let mut received = 0u64;
    let done = loop {
        tokio::select! {
            done = &mut restore => break done,
            next = ws.next(), if tx.is_some() => match next {
                Some(Ok(Message::Binary(b))) => {
                    received += b.len() as u64;
                    if let Some(tx) = &tx {
                        let _ = tx.send(b.to_vec());
                    }
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => tx = None,
                Some(Ok(_)) => {}
            },
        }
    };
    let (restored, rx) = done.map_err(|e| End::Restart(Error::Io(std::io::Error::other(e))))?;
    if tx.take().is_none() {
        return Err(End::Restart(Error::Closed));
    }
    let restored = restored.map_err(|refused| match refused {
        Refused::Ended => End::Restart(Error::Closed),
        Refused::Envelope(why) => End::Poll(why),
        Refused::Decoder(code) => End::Poll(format!(
            "libghostty-vt could not read Rex's snapshot (result {code})"
        )),
    })?;
    let mut streamed = Streamed::new(restored.terminal);
    let mut messages = Messages::new(nonce);
    streamed.apply(&mut messages, &restored.rest);
    for piece in rx.try_iter() {
        streamed.apply(&mut messages, &piece);
    }
    let channel = Channel {
        ws,
        token,
        nonce,
        received,
    };
    Ok((channel, streamed, messages))
}

/// Applies the stream until it ends.
async fn live(
    ch: &mut Channel,
    slot: &Slot,
    messages: &mut Messages,
    stop: &mut oneshot::Receiver<()>,
) -> End {
    loop {
        let next = tokio::select! {
            _ = &mut *stop => return End::Stopped,
            next = ch.ws.next() => next,
        };
        match next {
            Some(Ok(Message::Binary(b))) => {
                ch.received += b.len() as u64;
                if let Some(s) = crate::lock(slot).as_mut() {
                    s.apply(messages, &b);
                }
            }
            Some(Ok(Message::Close(frame))) => return closed(frame),
            Some(Ok(_)) => {}
            Some(Err(e)) => return End::Dropped(ws_error(e)),
            None => return End::Dropped(Error::Closed),
        }
    }
}

fn closed(frame: Option<CloseFrame>) -> End {
    let why = |f: &CloseFrame| Error::Rex {
        code: u16::from(f.code).to_string(),
        message: crate::printable(&f.reason),
    };
    match frame.as_ref().map(|f| u16::from(f.code)) {
        // The stream ended, or its block closed (4000).
        Some(1000 | 4000) => End::Over,
        Some(1008) => End::Poll("Rex has no terminal stream for this block".to_owned()),
        // The block failed, or Rex asks for a new stream.
        Some(1011 | 1013) => End::Restart(frame.as_ref().map_or(Error::Closed, why)),
        _ => End::Dropped(frame.as_ref().map_or(Error::Closed, why)),
    }
}

/// RESUME with the latest reconnect token and every byte received: Rex continues the stream
/// from the next byte, the same nonce, and a new token.
async fn resume(socket: &Path, ch: &mut Channel) -> Result<(), Error> {
    let mut first = vec![0x02];
    first.extend_from_slice(&ch.token);
    first.extend_from_slice(&ch.received.to_le_bytes());
    match handshake(socket, first).await? {
        (ws, Answer::Welcome { token, nonce }) if nonce == ch.nonce => {
            (ch.ws, ch.token) = (ws, token);
            Ok(())
        }
        (_, Answer::Welcome { .. }) => Err(Error::Unsupported(
            "resumed the stream with another nonce".to_owned(),
        )),
        (_, Answer::Reject(r)) => Err(Error::Unsupported(format!(
            "refused to resume the stream (reason {r})"
        ))),
    }
}

/// Follows the terminal until the last watch ends: restored, resumed after a drop, started
/// again when Rex asks for a new stream, and given up for polling when it cannot be read.
async fn follow(rex: Arc<Rex>, id: String, slot: Slot, mut stop: oneshot::Receiver<()>) {
    let mut delay = RETRY_FIRST;
    loop {
        let started = tokio::select! {
            _ = &mut stop => return,
            started = start(&rex, &id) => started,
        };
        let (mut ch, streamed, mut messages) = match started {
            Ok(started) => started,
            Err(end) => match again(end, &id, &mut stop, &mut delay).await {
                true => continue,
                false => return,
            },
        };
        delay = RETRY_FIRST;
        *crate::lock(&slot) = Some(streamed);
        let end = loop {
            let end = live(&mut ch, &slot, &mut messages, &mut stop).await;
            let exited = crate::lock(&slot).as_ref().is_some_and(|s| s.exited);
            match end {
                End::Stopped => {
                    // Status 1000 ends the stream; any other close keeps it for a resume.
                    let normal = CloseFrame {
                        code: CloseCode::Normal,
                        reason: "".into(),
                    };
                    let _ = tokio::time::timeout(TIMEOUT, ch.ws.close(Some(normal))).await;
                    return;
                }
                _ if exited => break End::Over,
                End::Dropped(e) => {
                    tracing::debug!(id, error = %e, "Rex stream dropped, resuming it");
                    let kept = crate::lock(&slot).take();
                    let resumed = tokio::select! {
                        _ = &mut stop => return,
                        resumed = resume(&rex.socket, &mut ch) => resumed,
                    };
                    match resumed {
                        Ok(()) => *crate::lock(&slot) = kept,
                        Err(e) => break End::Restart(e),
                    }
                }
                end => break end,
            }
        };
        *crate::lock(&slot) = None;
        if !again(end, &id, &mut stop, &mut delay).await {
            return;
        }
    }
}

/// Whether to start a new stream, once the delay has passed.
async fn again(end: End, id: &str, stop: &mut oneshot::Receiver<()>, delay: &mut Duration) -> bool {
    match end {
        End::Stopped => false,
        End::Over => {
            tracing::debug!(
                id,
                "Rex stream over: its block closed or its program exited"
            );
            false
        }
        End::Poll(why) => {
            tracing::warn!(
                id,
                "Rex terminal not streamed, its reads poll instead: {why}"
            );
            false
        }
        End::Restart(e) | End::Dropped(e) => {
            tracing::debug!(id, error = %e, "Rex stream ended, starting a new one");
            tokio::select! {
                _ = &mut *stop => return false,
                () = tokio::time::sleep(*delay) => {}
            }
            *delay = (*delay * 2).min(RETRY_MAX);
            true
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::runtime::rex::screen::DETECTION;

    /// A scratch Rex 0.1.0-preview.20 server's stream of a stand-in (Perl, `--shell=none`):
    /// 300 numbered lines in eight colors, a wide line, a wrapped one, then, live, truecolor,
    /// a `resize` look-alike with another nonce, Rex's own resize to 100x30 (the capture took
    /// the block and resized it), a line at the new width and `child_exited`. The two
    /// renderings are Rex's own `format`, with and without `unwrap`, taken before the exit.
    pub(in crate::runtime::rex) const STREAM: &[u8] =
        include_bytes!("fixtures/rex-0.1.0-preview.20/stream.bin");
    pub(in crate::runtime::rex) const FORMAT: &[u8] =
        include_bytes!("fixtures/rex-0.1.0-preview.20/format.vt");
    pub(in crate::runtime::rex) const ROWS: &[u8] =
        include_bytes!("fixtures/rex-0.1.0-preview.20/format-rows.vt");

    /// `STREAM` restored and applied, in pieces of `size` bytes.
    fn streamed(stream: &[u8], size: usize) -> Streamed {
        let mut pieces = stream.chunks(size).map(<[u8]>::to_vec);
        let mut next = || pieces.next();
        let restored = Terminal::restore(&mut next).unwrap();
        let mut s = Streamed::new(restored.terminal);
        let mut messages = Messages::new(crate::runtime::rex::fake::NONCE);
        s.apply(&mut messages, &restored.rest);
        for piece in pieces {
            s.apply(&mut messages, &piece);
        }
        s
    }

    #[test]
    fn a_streamed_terminal_reads_as_rex_formats_it() {
        for size in [1, 1000, STREAM.len()] {
            let mut s = streamed(STREAM, size);
            // Rex's resize, not the program's look-alike.
            assert_eq!(s.terminal.size(), (100, 30), "pieces of {size}");
            assert!(s.exited);
            for (source, lines, vt) in [
                ("recent_unwrapped", Some(200), FORMAT),
                ("recent_unwrapped", None, FORMAT),
                ("visible", None, ROWS),
                (DETECTION, None, ROWS),
            ] {
                let polled = emulate(vt, (100, 30), source, lines);
                assert_eq!(
                    s.read(source, lines),
                    polled,
                    "{source} in pieces of {size}"
                );
            }
            let text = s.read(DETECTION, None);
            assert!(text.contains("after a resize with another nonce"), "{text}");
        }
    }

    #[test]
    fn a_read_formats_again_only_once_the_stream_brought_something() {
        let mut s = streamed(STREAM, 1000);
        let mut messages = Messages::new(crate::runtime::rex::fake::NONCE);
        let first = s.read("recent_unwrapped", Some(200));
        assert_eq!(s.read("recent_unwrapped", Some(200)), first);
        assert_eq!(s.read(DETECTION, None), s.read(DETECTION, None));
        assert_eq!(s.formatted, 2);
        s.apply(&mut messages, b"one more\r\n");
        assert!(s.read("recent_unwrapped", Some(200)).contains("one more"));
        assert_eq!(s.formatted, 3);
    }

    #[test]
    fn the_restored_history_keeps_what_a_read_needs_and_no_more() {
        let mut s = streamed(STREAM, 1000);
        let mut messages = Messages::new(crate::runtime::rex::fake::NONCE);
        let more: String = (1..=10_000).map(|i| format!("more {i}\r\n")).collect();
        s.apply(&mut messages, more.as_bytes());
        let total = s.terminal.total_rows();
        assert!((2 * (30 + HISTORY)..10_000).contains(&total), "{total}");
        let read = s.read("recent_unwrapped", None);
        assert!(read.matches("more ").count() > HISTORY, "{total}");
    }

    #[test]
    fn rex_messages_need_the_channel_nonce_and_a_known_type() {
        let mut s = Streamed::new(Terminal::new(80, 24));
        let mut messages = Messages::new(7);
        let sent = concat!(
            "\x1b_rex1;8;{\"type\":\"resize\",\"cols\":40,\"rows\":10}\x1b\\",
            "\x1b_rex1;8;{\"type\":\"child_exited\",\"exit_code\":0}\x1b\\",
            "\x1b_rex1;7;{\"type\":\"theme\",\"background\":\"000000\"}\x1b\\",
            "\x1b_rex1;7;{\"type\":\"later\",\"cols\":40,\"rows\":10}\x1b\\",
            "\x1b_rex1;7;{\"type\":\"resize\",\"cols\":40}\x1b\\",
        );
        s.apply(&mut messages, sent.as_bytes());
        assert_eq!(s.terminal.size(), (80, 24));
        assert!(!s.exited);
        s.apply(
            &mut messages,
            b"\x1b_rex1;7;{\"type\":\"resize\",\"cols\":100,\"rows\":30}\x1b\\",
        );
        assert_eq!(s.terminal.size(), (100, 30));
        s.apply(
            &mut messages,
            b"\x1b_rex1;7;{\"type\":\"child_exited\",\"exit_code\":1}\x1b\\",
        );
        assert!(s.exited);
    }

    /// Output and messages of `stream` cut after each of `cuts`, outputs joined.
    fn split(stream: &[u8], cuts: &[usize]) -> Vec<(bool, Vec<u8>)> {
        let mut messages = Messages::new(7);
        let mut out: Vec<(bool, Vec<u8>)> = Vec::new();
        let mut from = 0;
        for &to in cuts.iter().chain([&stream.len()]) {
            messages.split(&stream[from..to], |piece| match piece {
                Piece::Output(b) => match out.last_mut() {
                    Some((false, last)) => last.extend_from_slice(b),
                    _ => out.push((false, b.to_vec())),
                },
                Piece::Message(m) => out.push((true, m.to_vec())),
            });
            from = to;
        }
        out
    }

    #[test]
    fn a_message_is_found_wherever_the_stream_is_cut() {
        let stream =
            b"a\x1b_rex1;7;{\"x\":1}\x1b\\b\x1b_rex1;8;{}\x1b\\c\x1b_rex1;7;y\x1b[0md\x1b_rex1;7";
        let want = vec![
            (false, b"a".to_vec()),
            (true, b"{\"x\":1}".to_vec()),
            (
                false,
                b"b\x1b_rex1;8;{}\x1b\\c\x1b_rex1;7;y\x1b[0md".to_vec(),
            ),
        ];
        assert_eq!(split(stream, &[]), want);
        for cut in 0..stream.len() {
            assert_eq!(split(stream, &[cut]), want, "cut at {cut}");
        }
        let each: Vec<usize> = (1..stream.len()).collect();
        assert_eq!(split(stream, &each), want);
    }
}
