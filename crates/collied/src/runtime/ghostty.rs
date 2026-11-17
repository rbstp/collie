//! A safe face over the libghostty-vt calls collied makes (`ghostty-vt-sys`), for Rex's
//! streamed terminals. Every handle lives inside one call except the terminal itself: the
//! formatter borrows the terminal, and a selection's grid references hold only until the
//! terminal next changes, so both are made and freed within `vt`, under `&self`.

use std::ffi::c_void;
use std::ptr::{self, NonNull};

use ghostty_vt_sys as sys;

/// The snapshot envelope version libghostty-vt 1.0.1 reads. The format is versioned apart
/// from the library and outside its ABI promise, and Rex writes it with its own Ghostty.
pub const SNAPSHOT_VERSION: u16 = 1;
const MAGIC: &[u8] = b"GHOSTSNP";
/// The magic, then the version as a little-endian `u16`.
const ENVELOPE: usize = 10;

pub struct Terminal(NonNull<c_void>);

// A terminal is a heap object with no thread affinity: libghostty-vt keeps no thread-local or
// global state for it, so it may move between threads. It is not `Sync`: a format reads it
// through a C pointer with no locking of its own, so collied keeps it behind a mutex.
unsafe impl Send for Terminal {}

impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe { sys::ghostty_terminal_free(self.0.as_ptr()) }
    }
}

#[derive(Debug, PartialEq)]
pub enum Refused {
    /// Not a snapshot, or one of a version this build does not read.
    Envelope(String),
    /// The source ended before the snapshot did.
    Ended,
    /// libghostty-vt refused the snapshot, with its result code.
    Decoder(i32),
}

pub struct Restored {
    pub terminal: Terminal,
    /// What the source gave after the snapshot's last byte.
    pub rest: Vec<u8>,
}

/// The decoder's reader: pieces pulled from `next` one at a time, and how far it has read.
struct Source<F> {
    next: F,
    piece: Vec<u8>,
    at: usize,
    given: usize,
    ended: bool,
}

impl<F: FnMut() -> Option<Vec<u8>>> Source<F> {
    /// Whether a byte is left to give, pulling the next piece when this one is used up.
    fn fill(&mut self) -> bool {
        while self.at == self.piece.len() {
            match (self.next)() {
                Some(piece) => (self.piece, self.at) = (piece, 0),
                None => {
                    self.ended = true;
                    return false;
                }
            }
        }
        true
    }
}

unsafe extern "C" fn read<F: FnMut() -> Option<Vec<u8>>>(
    userdata: *mut c_void,
    buffer: *mut u8,
    capacity: usize,
    out: *mut usize,
) -> bool {
    let source = unsafe { &mut *userdata.cast::<Source<F>>() };
    let n = match source.fill() {
        true => capacity.min(source.piece.len() - source.at),
        false => 0,
    };
    unsafe {
        ptr::copy_nonoverlapping(source.piece.as_ptr().add(source.at), buffer, n);
        *out = n;
    }
    source.at += n;
    source.given += n;
    true
}

impl Terminal {
    /// Restores a terminal from a snapshot pulled from `next`, which blocks until a piece
    /// arrives and answers `None` once the source ends. The envelope is checked before
    /// libghostty-vt reads anything.
    pub fn restore<F: FnMut() -> Option<Vec<u8>>>(next: F) -> Result<Restored, Refused> {
        let mut source = Source {
            next,
            piece: Vec::new(),
            at: 0,
            given: 0,
            ended: false,
        };
        while source.piece.len() < ENVELOPE {
            match (source.next)() {
                Some(more) => source.piece.extend_from_slice(&more),
                None => return Err(Refused::Ended),
            }
        }
        if !source.piece.starts_with(MAGIC) {
            return Err(Refused::Envelope("not a Ghostty snapshot".to_owned()));
        }
        let version = u16::from_le_bytes([source.piece[8], source.piece[9]]);
        if version != SNAPSHOT_VERSION {
            return Err(Refused::Envelope(format!(
                "snapshot version {version}, where this build reads {SNAPSHOT_VERSION}"
            )));
        }
        let reader = sys::GhosttyReader {
            read: read::<F>,
            userdata: ptr::from_mut(&mut source).cast(),
        };
        let mut decoder = ptr::null_mut();
        let made = unsafe { sys::ghostty_snapshot_decoder_new(ptr::null(), &mut decoder, reader) };
        if made != sys::GHOSTTY_SUCCESS {
            return Err(Refused::Decoder(made));
        }
        let mut terminal = ptr::null_mut();
        let mut offset = 0usize;
        let decoded = unsafe {
            let decoded = sys::ghostty_snapshot_decoder_decode(decoder, &mut terminal);
            if decoded == sys::GHOSTTY_SUCCESS {
                sys::ghostty_snapshot_decoder_get(
                    decoder,
                    sys::GHOSTTY_SNAPSHOT_DECODER_DATA_SOURCE_OFFSET,
                    ptr::from_mut(&mut offset).cast(),
                );
            }
            sys::ghostty_snapshot_decoder_free(decoder);
            decoded
        };
        let terminal = NonNull::new(terminal).map(Terminal);
        match (decoded, terminal) {
            // The decoder reads nothing past the snapshot's end (its reader adapter takes one
            // byte at a time near a boundary), so the rest of this piece is the live stream.
            (sys::GHOSTTY_SUCCESS, Some(terminal)) if offset == source.given => Ok(Restored {
                terminal,
                rest: source.piece.split_off(source.at),
            }),
            (sys::GHOSTTY_SUCCESS, _) => Err(Refused::Decoder(sys::GHOSTTY_SUCCESS)),
            _ if source.ended => Err(Refused::Ended),
            (failed, _) => Err(Refused::Decoder(failed)),
        }
    }

    #[cfg(test)]
    pub fn new(cols: u16, rows: u16) -> Self {
        let mut terminal = ptr::null_mut();
        let made = unsafe { sys::ghostty_terminal_new(ptr::null(), &mut terminal, cols, rows) };
        assert_eq!(made, sys::GHOSTTY_SUCCESS);
        Terminal(NonNull::new(terminal).unwrap())
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            unsafe { sys::ghostty_terminal_vt_write(self.0.as_ptr(), bytes.as_ptr(), bytes.len()) }
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        unsafe { sys::ghostty_terminal_resize(self.0.as_ptr(), cols.max(1), rows.max(1), 1, 1) };
    }

    /// History limited to about `lines` rows: pruned by whole pages, so it can keep fewer
    /// (measured in #739). A restored terminal keeps the byte limit of the one that wrote the
    /// snapshot (64 MiB in Rex).
    pub fn keep_lines(&mut self, lines: usize) {
        unsafe {
            sys::ghostty_terminal_set(
                self.0.as_ptr(),
                sys::GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES,
                ptr::from_ref(&lines).cast(),
            );
        }
    }

    fn get<T: Default>(&self, data: i32) -> T {
        let mut v = T::default();
        unsafe { sys::ghostty_terminal_get(self.0.as_ptr(), data, ptr::from_mut(&mut v).cast()) };
        v
    }

    pub fn size(&self) -> (u16, u16) {
        (
            self.get(sys::GHOSTTY_TERMINAL_DATA_COLS),
            self.get(sys::GHOSTTY_TERMINAL_DATA_ROWS),
        )
    }

    /// History and screen rows.
    pub fn total_rows(&self) -> usize {
        self.get(sys::GHOSTTY_TERMINAL_DATA_TOTAL_ROWS)
    }

    /// Rows `first` to the last as VT, text and styles only, soft-wrapped rows joined when
    /// `unwrap` is set.
    pub fn vt(&self, first: usize, unwrap: bool) -> Vec<u8> {
        let (cols, _) = self.size();
        let total = self.total_rows();
        let point = |x: u16, y: usize| sys::GhosttyPoint {
            tag: sys::GHOSTTY_POINT_TAG_SCREEN,
            value: sys::GhosttyPointValue {
                coordinate: sys::GhosttyPointCoordinate {
                    x,
                    y: u32::try_from(y).unwrap_or(u32::MAX),
                },
            },
        };
        let empty = sys::GhosttyGridRef {
            node: ptr::null_mut(),
            x: 0,
            y: 0,
        };
        let mut selection = sys::GhosttySelection {
            start: empty,
            end: empty,
            rectangle: false,
        };
        let terminal = self.0.as_ptr();
        let mut out = Vec::new();
        if first >= total {
            return out;
        }
        unsafe {
            if sys::ghostty_terminal_grid_ref(terminal, point(0, first), &mut selection.start)
                != sys::GHOSTTY_SUCCESS
                || sys::ghostty_terminal_grid_ref(
                    terminal,
                    point(cols.saturating_sub(1), total - 1),
                    &mut selection.end,
                ) != sys::GHOSTTY_SUCCESS
            {
                return out;
            }
            let mut formatter = ptr::null_mut();
            if sys::ghostty_formatter_terminal_new(ptr::null(), &mut formatter, terminal)
                != sys::GHOSTTY_SUCCESS
            {
                return out;
            }
            let emit = sys::GHOSTTY_FORMATTER_FORMAT_VT;
            let extras = sys::GHOSTTY_FORMATTER_EXTRAS_NONE;
            let set = |option, value: *const c_void| {
                sys::ghostty_formatter_set(formatter, option, value) == sys::GHOSTTY_SUCCESS
            };
            let (mut text, mut len) = (ptr::null_mut(), 0usize);
            if set(sys::GHOSTTY_FORMATTER_OPT_EMIT, ptr::from_ref(&emit).cast())
                && set(
                    sys::GHOSTTY_FORMATTER_OPT_UNWRAP,
                    ptr::from_ref(&unwrap).cast(),
                )
                && set(
                    sys::GHOSTTY_FORMATTER_OPT_EXTRAS,
                    ptr::from_ref(&extras).cast(),
                )
                && set(
                    sys::GHOSTTY_FORMATTER_OPT_SELECTION,
                    ptr::from_ref(&selection).cast(),
                )
                && sys::ghostty_formatter_format_alloc(formatter, ptr::null(), &mut text, &mut len)
                    == sys::GHOSTTY_SUCCESS
            {
                // Empty output is a null pointer with length 0, which no slice may be made from.
                if !text.is_null() {
                    out.extend_from_slice(std::slice::from_raw_parts(text, len));
                }
                sys::ghostty_free(ptr::null(), text, len);
            }
            sys::ghostty_formatter_free(formatter);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(bytes: &[u8], size: usize) -> impl FnMut() -> Option<Vec<u8>> {
        let mut chunks: Vec<Vec<u8>> = bytes.chunks(size).map(<[u8]>::to_vec).collect();
        chunks.reverse();
        move || chunks.pop()
    }

    #[test]
    fn an_unknown_envelope_is_refused_before_libghostty_vt_reads_it() {
        let restore = |bytes: &[u8]| Terminal::restore(pieces(bytes, 3)).err();
        let mut v2 = b"GHOSTSNP\x02\x00".to_vec();
        v2.extend_from_slice(&[0; 64]);
        assert_eq!(
            restore(&v2),
            Some(Refused::Envelope(
                "snapshot version 2, where this build reads 1".to_owned()
            ))
        );
        assert_eq!(
            restore(b"\x1b[0mGHOSTSNP\x01\x00"),
            Some(Refused::Envelope("not a Ghostty snapshot".to_owned()))
        );
        assert_eq!(restore(b"GHOSTSN"), Some(Refused::Ended));
        assert_eq!(restore(b"GHOSTSNP\x01\x00\x01"), Some(Refused::Ended));
    }

    #[test]
    fn empty_output_formats_as_nothing() {
        let terminal = Terminal::new(10, 2);
        assert_eq!(terminal.vt(0, true), b"");
        assert_eq!(terminal.vt(5, true), b"");
    }
}
