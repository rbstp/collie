use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::client::{Client, Method, TIMEOUT};
use super::wire::{Formatted, Piece};
use crate::runtime::Error;
use crate::runtime::screen::Screen;

/// The tail read first; doubled while it holds fewer lines than the read needs.
const FIRST_TAIL: u64 = 64 << 10;
/// The most one `blob_read` answers, measured as a JSON string, its quotes included.
const MAX_TAIL: u64 = 983_040;
/// The longest tail one answer can hold, when nothing in it needs escaping.
const MAX_WANT: u64 = MAX_TAIL - 2;
const HISTORY: u32 = protocol::limits::MAX_READ_LINES as u32;
/// The visible screen as plain text, named as herdr names that read.
pub(super) const DETECTION: &str = "detection";
/// Rex keeps the newest four results of each block, so a read races any other client's.
const ATTEMPTS: usize = 3;

/// A block, a source and the lines asked for.
type Reader = (String, String, Option<u32>);

/// How much of a block's rendering one reader asks for, learned from its earlier reads.
#[derive(Clone, Copy)]
struct Window {
    want: u64,
    /// The shortest tail one answer could not hold, never asked for again: escapes make a
    /// tail longer as a JSON string than it is.
    limit: u64,
    /// The whole rendering was no longer than `FIRST_TAIL` last time, so `format` is asked
    /// for all of it: a `blob_read` costs Rex more than the `format` of a short terminal
    /// (measured), and escapes at most sextuple its length as a JSON string.
    whole: bool,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            want: FIRST_TAIL,
            limit: u64::MAX,
            whole: false,
        }
    }
}

/// What one reader of a block last fed the emulator and read back: the same tail at the same
/// size reads the same.
struct Fed {
    size: (u16, u16),
    window: Window,
    tail: Vec<u8>,
    text: String,
}

/// A block's last `format` with or without `unwrap`, and the ticket taken just before it was
/// asked for.
type Rendered = Arc<tokio::sync::Mutex<Option<(u64, Formatted)>>>;

/// Rex agent screens, each read from a `format` VT tail and emulated afresh when the tail
/// changed: Rex sends no event for output, so each read is the terminal's whole state.
#[derive(Default)]
pub(super) struct Screens {
    fed: Mutex<HashMap<Reader, Fed>>,
    rendered: Mutex<HashMap<(String, bool), Rendered>>,
    tickets: AtomicU64,
    #[cfg(test)]
    pub(super) emulated: std::sync::atomic::AtomicUsize,
}

impl Screens {
    pub(super) async fn read(
        &self,
        client: &Client,
        session: &str,
        block: &str,
        size: (u16, u16),
        source: &str,
        lines: Option<u32>,
    ) -> Result<String, Error> {
        let screen_only = matches!(source, "visible" | DETECTION);
        let key = (
            block.to_owned(),
            source.to_owned(),
            lines.filter(|_| !screen_only),
        );
        let history = match screen_only {
            true => 0,
            false => lines.unwrap_or(HISTORY).min(HISTORY),
        };
        let rows = u64::from(size.1);
        let ask = Ask {
            unwrap: !screen_only,
            rows,
            need: rows + u64::from(history),
        };
        let mut window =
            (crate::lock(&self.fed).get(&key)).map_or_else(Window::default, |f| f.window);
        let tail = self.tail(client, session, block, ask, &mut window).await?;
        if let Some(f) = crate::lock(&self.fed).get_mut(&key) {
            f.window = window;
            if f.size == size && f.tail == tail {
                return Ok(f.text.clone());
            }
        }
        #[cfg(test)]
        self.emulated
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut screen = Screen::new(size.0, size.1, history as usize);
        screen.feed(&tail);
        let text = match source {
            DETECTION => screen.text(),
            _ => screen.read(source, lines),
        };
        let fed = Fed {
            size,
            window,
            tail,
            text: text.clone(),
        };
        crate::lock(&self.fed).insert(key, fed);
        Ok(text)
    }

    /// Drops what blocks no longer listed were read with.
    pub(super) fn retain(&self, listed: impl Fn(&str) -> bool) {
        crate::lock(&self.fed).retain(|(block, ..), _| listed(block));
        crate::lock(&self.rendered).retain(|(block, _), _| listed(block));
    }

    /// The end of the terminal's VT rendering, from a line break on. Neither call changes the
    /// terminal's size or owner (verified).
    async fn tail(
        &self,
        client: &Client,
        session: &str,
        block: &str,
        ask: Ask,
        window: &mut Window,
    ) -> Result<Vec<u8>, Error> {
        for _ in 0..ATTEMPTS {
            let head = if window.whole { MAX_TAIL } else { 1 };
            let f = self
                .format(client, session, block, ask.unwrap, head)
                .await?;
            window.whole = f.total <= FIRST_TAIL;
            let Some(id) = f.id else {
                return Ok(f.content.into_bytes());
            };
            match last(client, session, block, id, f.total, ask, window).await {
                Err(Error::Rex { code, .. }) if code == "not_found" => {}
                read => return read,
            }
        }
        Err(Error::Rex {
            code: "not_found".to_owned(),
            message: "Rex dropped each screen before it was read".to_owned(),
        })
    }

    /// One `format` at a time per block and `unwrap`: a reader that finds one under way
    /// waits, and shares the next one with the readers that waited with it. A reader is only
    /// served a rendering asked for after it asked, so a screen read right before input sees
    /// what it would see on a `format` of its own. The wait and its own `format` end within
    /// one `TIMEOUT` of when it asked, so a `format` Rex never answers fails no reader later
    /// than its own would have.
    async fn format(
        &self,
        client: &Client,
        session: &str,
        block: &str,
        unwrap: bool,
        head: u64,
    ) -> Result<Formatted, Error> {
        let asked = self.tickets.fetch_add(1, Ordering::Relaxed);
        let rendered = (crate::lock(&self.rendered))
            .entry((block.to_owned(), unwrap))
            .or_default()
            .clone();
        let shared = async {
            let mut latest = rendered.lock().await;
            if let Some((ticket, f)) = &*latest
                && *ticket > asked
            {
                return Ok(f.clone());
            }
            let ticket = self.tickets.fetch_add(1, Ordering::Relaxed);
            let args = json!({ "format": "vt", "unwrap": unwrap, "max_bytes": head });
            let (_, f): (_, Formatted) = client
                .terminal(Method::Format, session, block, args)
                .await?;
            *latest = Some((ticket, f.clone()));
            Ok(f)
        };
        tokio::time::timeout(TIMEOUT, shared)
            .await
            .map_err(|_| Error::Timeout)?
    }
}

#[derive(Clone, Copy)]
struct Ask {
    /// Soft-wrapped rows joined, so the emulator wraps them again and a recent read can join
    /// them as herdr does. A screen-only read keeps rows as they are: one line longer than
    /// any tail would leave it nothing to show.
    unwrap: bool,
    rows: u64,
    /// Line breaks wanted: the rows and the history asked for.
    need: u64,
}

/// The shortest tail holding `ask.need` line breaks, within what one answer holds. One that
/// holds fewer than the screen's rows is refused: the line it cut would have drawn them.
async fn last(
    client: &Client,
    session: &str,
    block: &str,
    id: u64,
    total: u64,
    ask: Ask,
    window: &mut Window,
) -> Result<Vec<u8>, Error> {
    let mut kept: Option<(u64, Vec<u8>, u64)> = None;
    let (bytes, breaks) = loop {
        let offset = total.saturating_sub(window.want);
        let args = json!({ "id": id, "offset": offset, "max_bytes": MAX_TAIL });
        let (_, piece): (_, Piece) = client
            .terminal(Method::BlobRead, session, block, args)
            .await?;
        if !piece.eof {
            window.limit = window.want;
            match kept.take() {
                Some((want, bytes, breaks)) => {
                    window.want = want;
                    break (bytes, breaks);
                }
                None => {
                    window.want /= 2;
                    continue;
                }
            }
        }
        let mut bytes = piece.data.into_bytes();
        if offset == 0 {
            return Ok(bytes);
        }
        // The tail may start inside a line, or a character: Rex answers its first bytes as
        // replacement characters.
        let start = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |i| i + 1);
        bytes.drain(..start);
        let breaks = bytes.iter().filter(|b| **b == b'\n').count() as u64;
        let next = (window.want * 2).min(MAX_WANT);
        if breaks >= ask.need || next >= window.limit || next == window.want {
            break (bytes, breaks);
        }
        kept = Some((window.want, bytes, breaks));
        window.want = next;
    };
    if breaks < ask.rows {
        return Err(Error::Unsupported(
            "the screen holds a line too long to read".to_owned(),
        ));
    }
    Ok(bytes)
}
