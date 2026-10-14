mod banner;
mod ids;
mod invite;
mod messages;
mod output;
#[cfg(feature = "schema")]
mod schema;

pub use banner::{NoticeOption, jump_banner, notice};
pub use ids::*;
pub use invite::*;
pub use messages::*;
pub use output::*;
#[cfg(feature = "schema")]
pub use schema::{client_frame_schema, server_frame_schema};

pub const PROTOCOL_VERSION: u32 = 25;

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub mod limits {
    pub const MAX_FRAME_BYTES: usize = 64 * 1024;
    pub const MAX_PROMPT_BYTES: usize = 32 * 1024;
    pub const MAX_KEYS_PER_CALL: usize = 16;
    pub const MAX_READ_LINES: u16 = 1000;
    pub const DEFAULT_WATCH_LINES: u16 = 200;
    pub const MAX_SNIPPET_CHARS: usize = 200;
    pub const MAX_LABEL_CHARS: usize = 64;
    pub const MAX_CHOICE_LABEL_CHARS: usize = 120;
    /// The amend field must still parse after the note: Menu::parse takes an option
    /// wrapped onto at most 4 rows, about 300 columns at an 80 column pane.
    pub const MAX_NOTE_CHARS: usize = 200;
    pub const MAX_CWD_BYTES: usize = 1024;
    pub const MAX_FOLDER_NAME_BYTES: usize = 255;
    pub const MAX_CLONE_SOURCE_BYTES: usize = 512;
    pub const NONCE_BYTES: usize = 32;
    pub const PAIRING_CODE_BYTES: usize = 16;
    pub const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;
    pub const MAX_ATTACHMENT_CHUNK_BYTES: usize = 32 * 1024;
    pub const MAX_ATTACHMENT_NAME_CHARS: usize = 64;
}
