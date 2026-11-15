use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize)]
pub(super) struct Hello {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub api_revision: u32,
    pub api_min_revision: u32,
    pub client: ClientInfo,
}

#[derive(Serialize)]
pub(super) struct ClientInfo {
    pub kind: &'static str,
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Serialize)]
pub(super) struct Request<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub id: &'a str,
    pub method: &'static str,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub payload: Value,
}

/// What the server sends; a newer server may add types, which are skipped.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Incoming {
    Welcome(Welcome),
    Response(Response),
    Event(Event),
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
pub(super) struct Welcome {
    pub protocol: String,
    #[serde(default)]
    pub api_revision: u32,
    #[serde(default)]
    pub api_server_revision: u32,
}

#[derive(Deserialize)]
pub(super) struct Response {
    pub id: String,
    pub seq: u64,
    #[serde(default)]
    pub payload: Option<Value>,
    #[serde(default)]
    pub error: Option<ErrorBody>,
}

#[derive(Deserialize)]
pub(super) struct ErrorBody {
    pub code: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Deserialize)]
pub(super) struct Event {
    pub seq: u64,
    pub payload: Tagged,
}

/// An event, a session event or a block event: its name and its fields.
#[derive(Deserialize)]
pub(super) struct Tagged {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Deserialize)]
pub(super) struct SessionEvent {
    pub session_id: String,
    pub event: Tagged,
}

#[derive(Deserialize)]
pub(super) struct BlockEvent {
    pub block_id: String,
    pub name: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SessionRef {
    pub session_id: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Deserialize)]
pub(super) struct Sessions {
    #[serde(default)]
    pub sessions: Option<Vec<SessionRef>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct BlockRef {
    #[serde(default)]
    pub block_id: Option<String>,
    #[serde(default)]
    pub creator_name: Option<String>,
    #[serde(default)]
    pub host_id: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Blocks {
    #[serde(default)]
    pub blocks: Option<Vec<BlockRef>>,
}

/// A process as Rex describes it; Rex never returns the arguments after `argv[0]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(super) struct Proc {
    pub pid: u32,
    pub name: String,
    #[serde(default)]
    pub argv0: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct Process {
    #[serde(default)]
    pub child: Option<Proc>,
    #[serde(default)]
    pub foreground: Option<Proc>,
    /// Set once the terminal's program exited.
    #[serde(default)]
    pub last_exit: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(super) struct ProcessRef {
    pub pid: u32,
    pub name: String,
}

/// An OSC 7501 status record; `msg` is the program's own text and is never read.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(super) struct Record {
    #[serde(default)]
    pub id: Option<String>,
    pub state: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub app: Option<String>,
    pub updated_at: String,
    #[serde(default)]
    pub owner: Option<ProcessRef>,
}

#[derive(Deserialize)]
pub(super) struct Status {
    #[serde(default)]
    pub records: Vec<Record>,
}

#[derive(Deserialize)]
pub(super) struct StatusChanged {
    pub id: String,
    pub record: Record,
}

#[derive(Deserialize)]
pub(super) struct StatusRemoved {
    pub ids: Vec<String>,
}

#[derive(Deserialize)]
pub(super) struct ProcessChanged {
    #[serde(default)]
    pub foreground: Option<Proc>,
}

#[derive(Deserialize)]
pub(super) struct Title {
    #[serde(default)]
    pub title: String,
}

#[derive(Deserialize)]
pub(super) struct Attached {
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub view: Option<View>,
}

#[derive(Deserialize)]
pub(super) struct View {
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Label {
    #[serde(default)]
    pub label: String,
}

#[derive(Deserialize)]
pub(super) struct BlockId {
    pub block_id: String,
}

#[derive(Deserialize)]
pub(super) struct SessionId {
    pub session_id: String,
}

#[derive(Deserialize)]
pub(super) struct SessionCreated {
    pub session: SessionRef,
}

#[derive(Deserialize)]
pub(super) struct Created {
    pub session_id: String,
    #[serde(default)]
    pub initial_windows: Option<Vec<Window>>,
}

#[derive(Deserialize)]
pub(super) struct Window {
    #[serde(default)]
    pub block_ids: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub(super) struct Owners {
    #[serde(default)]
    pub blocks: Option<Vec<Owned>>,
}

#[derive(Deserialize)]
pub(super) struct Owned {
    #[serde(default)]
    pub block_id: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Named {
    pub name: String,
}

#[derive(Deserialize)]
pub(super) struct Help {
    #[serde(default)]
    pub methods: Vec<Named>,
}

#[derive(Deserialize)]
pub(super) struct Creators {
    #[serde(default)]
    pub creators: Option<Vec<Creator>>,
}

#[derive(Deserialize)]
pub(super) struct Creator {
    pub name: String,
    #[serde(default)]
    pub api: Api,
}

#[derive(Default, Deserialize)]
pub(super) struct Api {
    #[serde(default)]
    pub methods: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
pub(super) struct Version {
    #[serde(default)]
    pub rex_version: String,
}

#[derive(Deserialize)]
pub(super) struct ServerStatus {
    pub pid: u32,
    #[serde(default)]
    pub host_id: Option<String>,
    #[serde(default)]
    pub tailscale: Option<Tailscale>,
    #[serde(default)]
    pub listeners: Option<Vec<Listener>>,
}

#[derive(Deserialize)]
pub(super) struct Tailscale {
    #[serde(default)]
    pub state: String,
}

#[derive(Deserialize)]
pub(super) struct Listener {
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub transport: String,
    #[serde(default)]
    pub admission: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Size {
    pub columns: u32,
    pub rows: u32,
}

/// `format` with `max_bytes`: the first part, and the result's id while more remains.
#[derive(Deserialize)]
pub(super) struct Formatted {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub total: u64,
}

#[derive(Deserialize)]
pub(super) struct Piece {
    pub data: String,
    pub eof: bool,
}
