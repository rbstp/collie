//! The libghostty-vt 1.0.0 functions collied calls, declared by hand from its headers.
#![allow(non_camel_case_types)]

use std::ffi::c_void;
use std::os::raw::c_int;

pub type GhosttyResult = c_int;
pub type GhosttyTerminal = *mut c_void;
pub type GhosttySnapshotDecoder = *mut c_void;
pub type GhosttyFormatter = *mut c_void;

pub const GHOSTTY_SUCCESS: GhosttyResult = 0;

pub const GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES: c_int = 29;
pub const GHOSTTY_TERMINAL_DATA_COLS: c_int = 1;
pub const GHOSTTY_TERMINAL_DATA_ROWS: c_int = 2;
pub const GHOSTTY_TERMINAL_DATA_TOTAL_ROWS: c_int = 14;
pub const GHOSTTY_POINT_TAG_SCREEN: c_int = 2;
pub const GHOSTTY_SNAPSHOT_DECODER_DATA_SOURCE_OFFSET: c_int = 2;
pub const GHOSTTY_FORMATTER_OPT_EMIT: c_int = 1;
pub const GHOSTTY_FORMATTER_OPT_UNWRAP: c_int = 2;
pub const GHOSTTY_FORMATTER_OPT_SELECTION: c_int = 4;
pub const GHOSTTY_FORMATTER_OPT_EXTRAS: c_int = 5;
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

unsafe extern "C" {
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
