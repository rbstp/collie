use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::json;

use super::client::{Client, Method};
use super::wire::{Formatted, Piece};
use crate::runtime::Error;
use crate::runtime::screen::Screen;

/// The tail read first; doubled while it holds fewer lines than the read needs.
const FIRST_TAIL: u64 = 64 << 10;
/// The most one `blob_read` answers, measured as a JSON string: a tail that does not fit in
/// one is halved.
const MAX_TAIL: u64 = 983_040;
const HISTORY: u32 = protocol::limits::MAX_READ_LINES as u32;
/// The visible screen as plain text, named as herdr names that read.
pub(super) const DETECTION: &str = "detection";
/// Rex keeps the newest four results of each block, so a read races any other client's.
const ATTEMPTS: usize = 3;

/// A block, a source and the lines asked for.
type Reader = (String, String, Option<u32>);

/// What one reader of a block last fed the emulator and read back: the same tail at the same
/// size reads the same.
struct Fed {
    size: (u16, u16),
    want: u64,
    /// The whole rendering was no longer than `FIRST_TAIL` last time, so `format` is asked
    /// for all of it: a `blob_read` costs Rex more than the `format` of a short terminal
    /// (measured), and escapes at most sextuple its length as a JSON string.
    whole: bool,
    tail: Vec<u8>,
    text: String,
}

/// Rex agent screens, each read from a `format` VT tail and emulated afresh when the tail
/// changed: Rex sends no event for output, so each read is the terminal's whole state.
#[derive(Default)]
pub(super) struct Screens {
    fed: Mutex<HashMap<Reader, Fed>>,
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
        let key = (block.to_owned(), source.to_owned(), lines);
        let history = match source {
            "visible" | DETECTION => 0,
            _ => lines.unwrap_or(HISTORY).min(HISTORY),
        };
        let need = u64::from(size.1) + u64::from(history);
        let (mut want, mut whole) =
            (crate::lock(&self.fed).get(&key)).map_or((FIRST_TAIL, false), |f| (f.want, f.whole));
        let tail = tail(client, session, block, need, &mut want, &mut whole).await?;
        if let Some(f) = crate::lock(&self.fed).get_mut(&key) {
            (f.want, f.whole) = (want, whole);
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
            want,
            whole,
            tail,
            text: text.clone(),
        };
        crate::lock(&self.fed).insert(key, fed);
        Ok(text)
    }

    /// Drops what blocks no longer listed were read with.
    pub(super) fn retain(&self, listed: impl Fn(&str) -> bool) {
        crate::lock(&self.fed).retain(|(block, ..), _| listed(block));
    }
}

/// The end of the terminal's VT rendering, from a line break on, holding `need` lines when
/// the result has them. Neither call changes the terminal's size or owner (verified).
async fn tail(
    client: &Client,
    session: &str,
    block: &str,
    need: u64,
    want: &mut u64,
    whole: &mut bool,
) -> Result<Vec<u8>, Error> {
    for _ in 0..ATTEMPTS {
        // Soft-wrapped rows come joined, so the emulator wraps them again and a recent read
        // can join them as herdr does.
        let head = if *whole { MAX_TAIL } else { 1 };
        let args = json!({ "format": "vt", "unwrap": true, "max_bytes": head });
        let (_, f): (_, Formatted) = client
            .terminal(Method::Format, session, block, args)
            .await?;
        *whole = f.total <= FIRST_TAIL;
        let Some(id) = f.id else {
            return Ok(f.content.into_bytes());
        };
        match last(client, session, block, id, f.total, need, want).await {
            Err(Error::Rex { code, .. }) if code == "not_found" => {}
            read => return read,
        }
    }
    Err(Error::Rex {
        code: "not_found".to_owned(),
        message: "Rex dropped each screen before it was read".to_owned(),
    })
}

async fn last(
    client: &Client,
    session: &str,
    block: &str,
    id: u64,
    total: u64,
    need: u64,
    want: &mut u64,
) -> Result<Vec<u8>, Error> {
    let mut shrunk = false;
    loop {
        let offset = total.saturating_sub(*want);
        let args = json!({ "id": id, "offset": offset, "max_bytes": MAX_TAIL });
        let (_, piece): (_, Piece) = client
            .terminal(Method::BlobRead, session, block, args)
            .await?;
        if !piece.eof {
            *want /= 2;
            shrunk = true;
            continue;
        }
        let mut bytes = piece.data.into_bytes();
        // The tail may start inside a line, or a character: Rex answers its first bytes as
        // replacement characters.
        if offset > 0 {
            let start = bytes
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |i| i + 1);
            bytes.drain(..start);
        }
        let enough = bytes.iter().filter(|b| **b == b'\n').count() as u64 >= need;
        if offset == 0 || enough || shrunk || *want >= MAX_TAIL {
            return Ok(bytes);
        }
        *want = (*want * 2).min(MAX_TAIL);
    }
}
