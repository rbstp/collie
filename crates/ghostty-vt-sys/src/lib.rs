//! libghostty-vt's C API as collied calls it, declared by hand from the headers of the release
//! `scripts/ghostty/pins.env` pins. A null allocator is libghostty-vt's default one.

use std::ffi::c_void;
use std::mem::{offset_of, size_of};
use std::os::raw::c_int;

pub type GhosttyResult = c_int;
pub type GhosttyTerminal = *mut c_void;
pub type GhosttySnapshotDecoder = *mut c_void;
pub type GhosttyFormatter = *mut c_void;

pub const GHOSTTY_SUCCESS: GhosttyResult = 0;
pub const GHOSTTY_NO_VALUE: GhosttyResult = -4;

pub const GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES: c_int = 28;
pub const GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES: c_int = 29;
pub const GHOSTTY_TERMINAL_DATA_COLS: c_int = 1;
pub const GHOSTTY_TERMINAL_DATA_ROWS: c_int = 2;
pub const GHOSTTY_TERMINAL_DATA_TOTAL_ROWS: c_int = 14;
pub const GHOSTTY_POINT_TAG_SCREEN: c_int = 2;
pub const GHOSTTY_SNAPSHOT_DECODER_DATA_SOURCE_OFFSET: c_int = 2;
pub const GHOSTTY_FORMATTER_OPT_EMIT: c_int = 1;
pub const GHOSTTY_FORMATTER_OPT_UNWRAP: c_int = 2;
pub const GHOSTTY_FORMATTER_OPT_TRIM: c_int = 3;
pub const GHOSTTY_FORMATTER_OPT_SELECTION: c_int = 4;
pub const GHOSTTY_FORMATTER_OPT_EXTRAS: c_int = 5;
pub const GHOSTTY_FORMATTER_FORMAT_PLAIN: c_int = 0;
pub const GHOSTTY_FORMATTER_FORMAT_VT: c_int = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyGridRef {
    pub node: *mut c_void,
    pub x: u16,
    pub y: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyPointCoordinate {
    pub x: u16,
    pub y: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union GhosttyPointValue {
    pub coordinate: GhosttyPointCoordinate,
    pub padding: [u64; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyPoint {
    pub tag: c_int,
    pub value: GhosttyPointValue,
}

#[repr(C)]
pub struct GhosttySelection {
    pub start: GhosttyGridRef,
    pub end: GhosttyGridRef,
    pub rectangle: bool,
}

pub type GhosttyReaderFn = unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize) -> bool;

#[repr(C)]
pub struct GhosttyReader {
    pub read: GhosttyReaderFn,
    pub userdata: *mut c_void,
}

// The headers' sizes and offsets on every 64-bit target collied builds for.
const _: () = {
    assert!(size_of::<GhosttyGridRef>() == 16);
    assert!(offset_of!(GhosttyGridRef, x) == 8);
    assert!(offset_of!(GhosttyGridRef, y) == 10);
    assert!(size_of::<GhosttyPointCoordinate>() == 8);
    assert!(offset_of!(GhosttyPointCoordinate, y) == 4);
    assert!(size_of::<GhosttyPointValue>() == 16);
    assert!(size_of::<GhosttyPoint>() == 24);
    assert!(offset_of!(GhosttyPoint, value) == 8);
    assert!(size_of::<GhosttySelection>() == 40);
    assert!(offset_of!(GhosttySelection, end) == 16);
    assert!(offset_of!(GhosttySelection, rectangle) == 32);
    assert!(size_of::<GhosttyReader>() == 16);
    assert!(offset_of!(GhosttyReader, userdata) == 8);
};

unsafe extern "C" {
    pub fn ghostty_terminal_new(
        allocator: *const c_void,
        terminal: *mut GhosttyTerminal,
        cols: u16,
        rows: u16,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_free(terminal: GhosttyTerminal);
    pub fn ghostty_terminal_vt_write(terminal: GhosttyTerminal, data: *const u8, len: usize);
    pub fn ghostty_terminal_resize(
        terminal: GhosttyTerminal,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_set(
        terminal: GhosttyTerminal,
        option: c_int,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_get(
        terminal: GhosttyTerminal,
        data: c_int,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_grid_ref(
        terminal: GhosttyTerminal,
        point: GhosttyPoint,
        out: *mut GhosttyGridRef,
    ) -> GhosttyResult;

    pub fn ghostty_snapshot_decoder_new(
        allocator: *const c_void,
        decoder: *mut GhosttySnapshotDecoder,
        reader: GhosttyReader,
    ) -> GhosttyResult;
    pub fn ghostty_snapshot_decoder_decode(
        decoder: GhosttySnapshotDecoder,
        terminal: *mut GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_snapshot_decoder_get(
        decoder: GhosttySnapshotDecoder,
        data: c_int,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_snapshot_decoder_free(decoder: GhosttySnapshotDecoder);

    pub fn ghostty_formatter_terminal_new(
        allocator: *const c_void,
        formatter: *mut GhosttyFormatter,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_formatter_set(
        formatter: GhosttyFormatter,
        option: c_int,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_formatter_format_alloc(
        formatter: GhosttyFormatter,
        allocator: *const c_void,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
    ) -> GhosttyResult;
    pub fn ghostty_formatter_free(formatter: GhosttyFormatter);
    pub fn ghostty_free(allocator: *const c_void, ptr: *mut u8, len: usize);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    // A misspelled or missing declaration fails this test's link.
    #[test]
    fn every_declared_function_links() {
        let functions = [
            ghostty_terminal_new as *const (),
            ghostty_terminal_free as *const (),
            ghostty_terminal_vt_write as *const (),
            ghostty_terminal_resize as *const (),
            ghostty_terminal_set as *const (),
            ghostty_terminal_get as *const (),
            ghostty_terminal_grid_ref as *const (),
            ghostty_snapshot_decoder_new as *const (),
            ghostty_snapshot_decoder_decode as *const (),
            ghostty_snapshot_decoder_get as *const (),
            ghostty_snapshot_decoder_free as *const (),
            ghostty_formatter_terminal_new as *const (),
            ghostty_formatter_set as *const (),
            ghostty_formatter_format_alloc as *const (),
            ghostty_formatter_free as *const (),
            ghostty_free as *const (),
        ];
        assert!(functions.iter().all(|f| !f.is_null()));
    }

    #[test]
    fn a_terminal_formats_the_bytes_it_was_fed() {
        unsafe {
            let mut terminal = ptr::null_mut();
            assert_eq!(
                ghostty_terminal_new(ptr::null(), &mut terminal, 20, 4),
                GHOSTTY_SUCCESS
            );
            let bytes = b"hello\r\n\x1b[1mworld\x1b[0m";
            ghostty_terminal_vt_write(terminal, bytes.as_ptr(), bytes.len());
            let mut cols = 0u16;
            assert_eq!(
                ghostty_terminal_get(terminal, GHOSTTY_TERMINAL_DATA_COLS, (&raw mut cols).cast()),
                GHOSTTY_SUCCESS
            );
            assert_eq!(cols, 20);

            let mut formatter = ptr::null_mut();
            assert_eq!(
                ghostty_formatter_terminal_new(ptr::null(), &mut formatter, terminal),
                GHOSTTY_SUCCESS
            );
            let plain = GHOSTTY_FORMATTER_FORMAT_PLAIN;
            assert_eq!(
                ghostty_formatter_set(
                    formatter,
                    GHOSTTY_FORMATTER_OPT_EMIT,
                    (&raw const plain).cast()
                ),
                GHOSTTY_SUCCESS
            );
            let (mut text, mut len) = (ptr::null_mut(), 0);
            assert_eq!(
                ghostty_formatter_format_alloc(formatter, ptr::null(), &mut text, &mut len),
                GHOSTTY_SUCCESS
            );
            assert_eq!(std::slice::from_raw_parts(text, len), b"hello\nworld");
            ghostty_free(ptr::null(), text, len);
            ghostty_formatter_free(formatter);
            ghostty_terminal_free(terminal);
        }
    }
}
