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

    pub fn resize(&mut self, cols: u16, rows: u16) -> bool {
        let resized = unsafe {
            sys::ghostty_terminal_resize(self.0.as_ptr(), cols.max(1), rows.max(1), 1, 1)
        };
        resized == sys::GHOSTTY_SUCCESS
    }

    /// History limited to about `lines` rows: pruned by whole pages, so it can keep fewer
    /// (measured in #739). A restored terminal keeps the byte limit of the one that wrote the
    /// snapshot (64 MiB in Rex).
    pub fn keep_lines(&mut self, lines: usize) -> bool {
        let set = unsafe {
            sys::ghostty_terminal_set(
                self.0.as_ptr(),
                sys::GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES,
                ptr::from_ref(&lines).cast(),
            )
        };
        set == sys::GHOSTTY_SUCCESS
    }

    fn get<T: Default>(&self, data: i32) -> Option<T> {
        let mut v = T::default();
        let got = unsafe {
            sys::ghostty_terminal_get(self.0.as_ptr(), data, ptr::from_mut(&mut v).cast())
        };
        (got == sys::GHOSTTY_SUCCESS).then_some(v)
    }

    pub fn size(&self) -> Option<(u16, u16)> {
        Some((
            self.get(sys::GHOSTTY_TERMINAL_DATA_COLS)?,
            self.get(sys::GHOSTTY_TERMINAL_DATA_ROWS)?,
        ))
    }

    /// History and screen rows.
    pub fn total_rows(&self) -> Option<usize> {
        self.get(sys::GHOSTTY_TERMINAL_DATA_TOTAL_ROWS)
    }

    /// Rows `first` to the last as VT, text, styles and links, soft-wrapped rows joined when
    /// `unwrap` is set; `None` when it fails or would pass `max` bytes. The formatter writes
    /// each cell's whole OSC 8 link again whenever the link changes, so a few rows of short
    /// text can format to megabytes: nothing past `max` is ever held.
    pub fn vt(&self, first: usize, unwrap: bool, max: usize) -> Option<Vec<u8>> {
        let (cols, _) = self.size()?;
        let total = self.total_rows()?;
        if first >= total {
            return None;
        }
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
        let mut sink = Sink {
            out: Vec::new(),
            max,
        };
        unsafe {
            if sys::ghostty_terminal_grid_ref(terminal, point(0, first), &mut selection.start)
                != sys::GHOSTTY_SUCCESS
                || sys::ghostty_terminal_grid_ref(
                    terminal,
                    point(cols.saturating_sub(1), total - 1),
                    &mut selection.end,
                ) != sys::GHOSTTY_SUCCESS
            {
                return None;
            }
            let mut formatter = ptr::null_mut();
            if sys::ghostty_formatter_terminal_new(ptr::null(), &mut formatter, terminal)
                != sys::GHOSTTY_SUCCESS
            {
                return None;
            }
            let emit = sys::GHOSTTY_FORMATTER_FORMAT_VT;
            let extras = sys::GHOSTTY_FORMATTER_EXTRAS_NONE;
            let set = |option, value: *const c_void| {
                sys::ghostty_formatter_set(formatter, option, value) == sys::GHOSTTY_SUCCESS
            };
            let writer = sys::GhosttyWriter {
                write: append,
                userdata: ptr::from_mut(&mut sink).cast(),
            };
            let formatted = set(sys::GHOSTTY_FORMATTER_OPT_EMIT, ptr::from_ref(&emit).cast())
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
                && sys::ghostty_formatter_format(formatter, writer) == sys::GHOSTTY_SUCCESS;
            sys::ghostty_formatter_free(formatter);
            formatted.then_some(sink.out)
        }
    }
}

/// Where a format writes, up to `max` bytes.
struct Sink {
    out: Vec<u8>,
    max: usize,
}

/// Refuses a write past the sink's `max`, which fails the format with `GHOSTTY_IO_ERROR`.
unsafe extern "C" fn append(userdata: *mut c_void, data: *const u8, len: usize) -> bool {
    let sink = unsafe { &mut *userdata.cast::<Sink>() };
    if len > sink.max - sink.out.len() {
        return false;
    }
    // An empty write may come with a null pointer, which no slice may be made of.
    if len == 0 {
        return true;
    }
    sink.out
        .extend_from_slice(unsafe { std::slice::from_raw_parts(data, len) });
    true
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
    fn an_empty_write_makes_no_slice_of_a_null_pointer() {
        let mut sink = Sink {
            out: Vec::new(),
            max: 4,
        };
        assert!(unsafe { append(ptr::from_mut(&mut sink).cast(), ptr::null(), 0) });
        assert!(sink.out.is_empty());
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
        assert_eq!(terminal.vt(0, true, 1), Some(Vec::new()));
        assert_eq!(terminal.vt(5, true, 1), None);
    }

    /// Each cell linked to a 2,000-byte target, the next cell to another: the formatter
    /// writes every link again, so 80x24 such cells format to about 3.4 MB.
    #[test]
    fn a_format_past_its_ceiling_fails_and_holds_no_more() {
        let mut terminal = Terminal::new(80, 24);
        let links: String = (0..80 * 24)
            .map(|i| {
                let target = format!("https://example.com/{}", "x".repeat(2000));
                format!("\x1b]8;id={i};{target}{}\x1b\\{}", i % 2, i % 10)
            })
            .collect();
        terminal.write(links.as_bytes());
        let whole = terminal.vt(0, true, usize::MAX).unwrap();
        assert!(whole.len() > 3_000_000, "{}", whole.len());
        assert_eq!(terminal.vt(0, true, 983_040), None);
        let fitting = terminal.vt(0, true, whole.len()).unwrap();
        assert_eq!(fitting, whole);
    }
}
