use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::*;
use crate::limits;

pub type RequestId = u32;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ClientFrame {
    pub id: RequestId,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum Request {
    #[serde(rename = "hello")]
    Hello(HelloParams),
    #[serde(rename = "pair.complete")]
    PairComplete(PairCompleteParams),

    #[serde(rename = "flock.snapshot")]
    FlockSnapshot(Empty),
    #[serde(rename = "workspace.list")]
    WorkspaceList(Empty),
    #[serde(rename = "agent.read")]
    AgentRead(ReadParams),
    #[serde(rename = "pane.read")]
    PaneRead(ReadParams),
    #[serde(rename = "agent.watch")]
    AgentWatch(AgentWatchParams),
    #[serde(rename = "task.options")]
    TaskOptions(Empty),

    #[serde(rename = "agent.prompt")]
    AgentPrompt(AgentPromptParams),
    #[serde(rename = "agent.send_keys")]
    AgentSendKeys(AgentSendKeysParams),
    #[serde(rename = "agent.focus")]
    AgentFocus(AgentTarget),
    #[serde(rename = "task.new")]
    TaskNew(TaskNewParams),
    #[serde(rename = "workspace.close")]
    WorkspaceClose(WorkspaceCloseParams),
    #[serde(rename = "pane.close")]
    PaneClose(PaneCloseParams),

    #[serde(rename = "approval.list")]
    ApprovalList(Empty),
    #[serde(rename = "approval.decide")]
    ApprovalDecide(ApprovalDecideParams),

    #[serde(rename = "push.register")]
    PushRegister(PushRegisterParams),
    #[serde(rename = "push.activity_token")]
    PushActivityToken(PushActivityTokenParams),

    #[serde(rename = "attachment.begin")]
    AttachmentBegin(AttachmentBeginParams),
    #[serde(rename = "attachment.chunk")]
    AttachmentChunk(AttachmentChunkParams),
    #[serde(rename = "attachment.commit")]
    AttachmentCommit(AttachmentCommitParams),
    #[serde(rename = "attachment.abort")]
    AttachmentAbort(AttachmentAbortParams),
}

impl Request {
    pub const METHODS: &[&str] = &[
        "hello",
        "pair.complete",
        "flock.snapshot",
        "workspace.list",
        "agent.read",
        "pane.read",
        "agent.watch",
        "task.options",
        "agent.prompt",
        "agent.send_keys",
        "agent.focus",
        "task.new",
        "workspace.close",
        "pane.close",
        "approval.list",
        "approval.decide",
        "push.register",
        "push.activity_token",
        "attachment.begin",
        "attachment.chunk",
        "attachment.commit",
        "attachment.abort",
    ];

    pub fn method(&self) -> &'static str {
        match self {
            Self::Hello(_) => "hello",
            Self::PairComplete(_) => "pair.complete",
            Self::FlockSnapshot(_) => "flock.snapshot",
            Self::WorkspaceList(_) => "workspace.list",
            Self::AgentRead(_) => "agent.read",
            Self::PaneRead(_) => "pane.read",
            Self::AgentWatch(_) => "agent.watch",
            Self::TaskOptions(_) => "task.options",
            Self::AgentPrompt(_) => "agent.prompt",
            Self::AgentSendKeys(_) => "agent.send_keys",
            Self::AgentFocus(_) => "agent.focus",
            Self::TaskNew(_) => "task.new",
            Self::WorkspaceClose(_) => "workspace.close",
            Self::PaneClose(_) => "pane.close",
            Self::ApprovalList(_) => "approval.list",
            Self::ApprovalDecide(_) => "approval.decide",
            Self::PushRegister(_) => "push.register",
            Self::PushActivityToken(_) => "push.activity_token",
            Self::AttachmentBegin(_) => "attachment.begin",
            Self::AttachmentChunk(_) => "attachment.chunk",
            Self::AttachmentCommit(_) => "attachment.commit",
            Self::AttachmentAbort(_) => "attachment.abort",
        }
    }

    pub fn class(&self) -> MethodClass {
        match self {
            Self::Hello(_) | Self::PairComplete(_) => MethodClass::Session,
            Self::FlockSnapshot(_)
            | Self::WorkspaceList(_)
            | Self::AgentRead(_)
            | Self::PaneRead(_)
            | Self::AgentWatch(_)
            | Self::TaskOptions(_)
            | Self::ApprovalList(_) => MethodClass::Read,
            Self::AgentPrompt(_)
            | Self::AgentSendKeys(_)
            | Self::AgentFocus(_)
            | Self::TaskNew(_)
            | Self::WorkspaceClose(_)
            | Self::PaneClose(_)
            | Self::AttachmentBegin(_)
            | Self::AttachmentChunk(_)
            | Self::AttachmentCommit(_)
            | Self::AttachmentAbort(_) => MethodClass::Drive,
            Self::ApprovalDecide(_) => MethodClass::Approval,
            Self::PushRegister(_) | Self::PushActivityToken(_) => MethodClass::Push,
        }
    }
}

/// Drive and Approval calls are written to the audit log and rate limited per peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodClass {
    Session,
    Read,
    Drive,
    Approval,
    Push,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HelloParams {
    pub protocol_version: u32,
    pub app_version: Label,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PairCompleteParams {
    pub pairing_code: PairingCode,
    pub device_label: Label,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadSource {
    Visible,
    Recent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadParams {
    pub terminal_id: TerminalId,
    pub source: ReadSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 1000))]
    pub lines: Option<u16>,
}

impl ReadParams {
    pub fn is_valid(&self) -> bool {
        self.lines
            .is_none_or(|n| (1..=limits::MAX_READ_LINES).contains(&n))
    }
}

/// One watched agent per connection; `None` stops `agent.output` events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentWatchParams {
    pub terminal_id: Option<TerminalId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTarget {
    pub terminal_id: TerminalId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPromptParams {
    pub op_id: OpId,
    pub terminal_id: TerminalId,
    pub text: PromptText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum Key {
    #[serde(rename = "enter")]
    Enter,
    #[serde(rename = "esc")]
    Esc,
    #[serde(rename = "up")]
    Up,
    #[serde(rename = "down")]
    Down,
    #[serde(rename = "tab")]
    Tab,
    #[serde(rename = "shift+tab")]
    ShiftTab,
    #[serde(rename = "ctrl+c")]
    CtrlC,
    #[serde(rename = "y")]
    Y,
    #[serde(rename = "n")]
    N,
}

impl Key {
    pub fn herdr_name(self) -> &'static str {
        match self {
            Self::Enter => "enter",
            Self::Esc => "esc",
            Self::Up => "up",
            Self::Down => "down",
            Self::Tab => "tab",
            Self::ShiftTab => "shift+tab",
            Self::CtrlC => "ctrl+c",
            Self::Y => "y",
            Self::N => "n",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSendKeysParams {
    pub op_id: OpId,
    pub terminal_id: TerminalId,
    #[schemars(length(min = 1, max = 16))]
    pub keys: Vec<Key>,
}

impl AgentSendKeysParams {
    pub fn is_valid(&self) -> bool {
        (1..=limits::MAX_KEYS_PER_CALL).contains(&self.keys.len())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskNewParams {
    pub op_id: OpId,
    pub cwd: Cwd,
    pub agent: AgentKind,
    pub prompt: PromptText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Label>,
}

/// `confirm` must be `true`; anything else yields `confirm_required`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCloseParams {
    pub workspace_id: WorkspaceId,
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaneCloseParams {
    pub terminal_id: TerminalId,
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approve,
    ApproveAlways,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApprovalDecideParams {
    pub approval_id: ApprovalId,
    pub decision: Decision,
    pub nonce: Nonce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApnsEnvironment {
    Sandbox,
    Production,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PushRegisterParams {
    pub apns_token: PushToken,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_activity_push_to_start_token: Option<PushToken>,
    pub environment: ApnsEnvironment,
    pub notification_key: NotificationKey,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PushActivityTokenParams {
    pub activity_id: ActivityId,
    pub terminal_id: TerminalId,
    pub token: PushToken,
}

/// `sha256` covers the whole file; collied checks it and the size before keeping it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachmentBeginParams {
    pub op_id: OpId,
    pub name: AttachmentName,
    #[schemars(range(min = 1, max = 20971520))]
    pub size: u64,
    pub sha256: Sha256Hex,
}

impl AttachmentBeginParams {
    pub fn is_valid(&self) -> bool {
        (1..=limits::MAX_ATTACHMENT_BYTES).contains(&self.size)
    }
}

/// Chunks arrive in order: `offset` is the number of bytes already received.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachmentChunkParams {
    pub upload_id: UploadId,
    pub offset: u64,
    pub data: ChunkData,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachmentCommitParams {
    pub op_id: OpId,
    pub upload_id: UploadId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachmentAbortParams {
    pub upload_id: UploadId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerFrame {
    Result {
        id: RequestId,
        result: Response,
    },
    Error {
        id: Option<RequestId>,
        error: ErrorBody,
    },
    /// `seq` increases by one per event on a connection; a snapshot carries the `seq`
    /// it reflects, so the client drops events with `seq <= snapshot.seq`.
    Event {
        seq: u64,
        event: Event,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Hello(HelloResult),
    Paired {
        machine: MachineInfo,
    },
    Flock(Flock),
    Workspaces {
        workspaces: Vec<Workspace>,
    },
    Terminal(TerminalRead),
    TaskOptions(TaskOptions),
    TaskStarted {
        workspace_id: WorkspaceId,
        terminal_id: TerminalId,
    },
    Approvals {
        approvals: Vec<Approval>,
    },
    ApprovalResolved {
        approval_id: ApprovalId,
        outcome: ApprovalOutcome,
    },
    AttachmentStarted {
        upload_id: UploadId,
    },
    /// Absolute path of the stored file on the Mac.
    AttachmentStored {
        path: String,
    },
    Ok,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HelloResult {
    pub protocol_version: u32,
    pub collied_version: String,
    pub machine: MachineInfo,
    pub herdr_version: Option<String>,
    pub paired: bool,
}

/// `node_id` is the Tailscale stable node ID; the phone pins it at pairing time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MachineInfo {
    pub name: String,
    pub node_id: String,
    pub herdr_session: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Workspace {
    pub workspace_id: WorkspaceId,
    pub label: String,
    pub number: u32,
    pub status: AgentStatus,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Agent {
    pub terminal_id: TerminalId,
    pub workspace_id: WorkspaceId,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: AgentStatus,
    pub status_since_ms: u64,
    pub cwd: Option<String>,
    pub last_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Flock {
    pub seq: u64,
    pub machine: MachineInfo,
    pub workspaces: Vec<Workspace>,
    pub agents: Vec<Agent>,
    pub approvals: Vec<Approval>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalRead {
    pub terminal_id: TerminalId,
    pub source: ReadSource,
    pub ansi: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskOptions {
    pub agents: Vec<AgentKind>,
    pub default_agent: AgentKind,
    pub recent_cwds: Vec<Cwd>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PendingTool {
    pub name: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Approval {
    pub approval_id: ApprovalId,
    pub terminal_id: TerminalId,
    pub agent_label: String,
    pub workspace_label: String,
    pub snippet: String,
    pub tool: Option<PendingTool>,
    pub options: Vec<Decision>,
    pub nonce: Nonce,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ApprovalOutcome {
    /// Keys were sent and the agent then left `blocked`.
    Applied {
        decision: Decision,
        by: String,
    },
    /// Keys were sent but the agent was still `blocked` when collied stopped waiting.
    Unconfirmed {
        decision: Decision,
        by: String,
    },
    Expired,
    /// The prompt changed, or was answered on the Mac, before a decision was applied.
    Superseded,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "name")]
pub enum Event {
    #[serde(rename = "agent.status")]
    AgentStatus { agent: Agent },
    #[serde(rename = "agent.output")]
    AgentOutput(TerminalRead),
    #[serde(rename = "approval.needed")]
    ApprovalNeeded { approval: Approval },
    #[serde(rename = "approval.resolved")]
    ApprovalResolved {
        approval_id: ApprovalId,
        outcome: ApprovalOutcome,
    },
    #[serde(rename = "flock.changed")]
    FlockChanged {},
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    MalformedFrame,
    FrameTooLarge,
    UnknownMethod,
    InvalidParams,
    UnsupportedProtocol,
    HelloRequired,
    NotPaired,
    PairingFailed,
    NotFound,
    ConfirmRequired,
    AgentBlocked,
    AgentNotReady,
    ApprovalNotFound,
    ApprovalExpired,
    ApprovalNonceMismatch,
    ApprovalAlreadyResolved,
    RateLimited,
    HerdrUnavailable,
    NotImplemented,
    TooLarge,
    ChecksumMismatch,
    Internal,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameError {
    pub id: Option<RequestId>,
    pub code: ErrorCode,
    pub message: String,
}

impl FrameError {
    fn new(id: Option<RequestId>, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            id,
            code,
            message: message.into(),
        }
    }

    pub fn into_frame(self) -> ServerFrame {
        ServerFrame::Error {
            id: self.id,
            error: ErrorBody {
                code: self.code,
                message: self.message,
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFrame {
    id: RequestId,
    method: String,
    params: serde_json::Value,
}

/// Fail-closed decode: unknown methods are rejected before their params are looked at,
/// and every params struct denies unknown fields.
pub fn parse_client_frame(bytes: &[u8]) -> Result<ClientFrame, FrameError> {
    if bytes.len() > limits::MAX_FRAME_BYTES {
        return Err(FrameError::new(
            None,
            ErrorCode::FrameTooLarge,
            "frame too large",
        ));
    }
    let raw: RawFrame = serde_json::from_slice(bytes)
        .map_err(|_| FrameError::new(None, ErrorCode::MalformedFrame, "malformed frame"))?;
    let id = Some(raw.id);
    if !Request::METHODS.contains(&raw.method.as_str()) {
        return Err(FrameError::new(
            id,
            ErrorCode::UnknownMethod,
            "unknown method",
        ));
    }
    let tagged = serde_json::json!({ "method": raw.method, "params": raw.params });
    let request: Request = serde_json::from_value(tagged)
        .map_err(|e| FrameError::new(id, ErrorCode::InvalidParams, e.to_string()))?;
    let valid = match &request {
        Request::AgentRead(p) | Request::PaneRead(p) => p.is_valid(),
        Request::AgentSendKeys(p) => p.is_valid(),
        Request::AttachmentBegin(p) => p.is_valid(),
        _ => true,
    };
    if !valid {
        return Err(FrameError::new(
            id,
            ErrorCode::InvalidParams,
            "params out of range",
        ));
    }
    Ok(ClientFrame {
        id: raw.id,
        request,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<ClientFrame, FrameError> {
        parse_client_frame(s.as_bytes())
    }

    #[test]
    fn methods_table_matches_enum() {
        for method in Request::METHODS {
            let err = parse(&format!(
                r#"{{"id":1,"method":"{method}","params":{{"__probe":0}}}}"#
            ))
            .unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidParams, "{method}");
        }
    }

    #[test]
    fn herdr_privileged_methods_are_unknown() {
        for method in [
            "server.stop",
            "server.reload_config",
            "server.live_handoff",
            "plugin.enable",
            "integration.install",
            "layout.apply",
            "pane.send_text",
            "pane.send_input",
            "pane.process_info",
            "agent.explain",
            "agent.start",
            "events.subscribe",
        ] {
            let err =
                parse(&format!(r#"{{"id":7,"method":"{method}","params":{{}}}}"#)).unwrap_err();
            assert_eq!(err.code, ErrorCode::UnknownMethod, "{method}");
            assert_eq!(err.id, Some(7));
        }
    }

    #[test]
    fn round_trip() {
        let frame = ClientFrame {
            id: 3,
            request: Request::AgentSendKeys(AgentSendKeysParams {
                op_id: OpId::new("A".repeat(22)).unwrap(),
                terminal_id: TerminalId::new("term_1").unwrap(),
                keys: vec![Key::ShiftTab, Key::Enter],
            }),
        };
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(
            json,
            r#"{"id":3,"method":"agent.send_keys","params":{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","terminal_id":"term_1","keys":["shift+tab","enter"]}}"#
        );
        assert_eq!(parse(&json).unwrap(), frame);
    }

    #[test]
    fn rejects_unlisted_keys_and_extra_fields() {
        let bad_key = r#"{"id":1,"method":"agent.send_keys","params":{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","terminal_id":"t","keys":["ctrl+d"]}}"#;
        assert_eq!(parse(bad_key).unwrap_err().code, ErrorCode::InvalidParams);
        let extra =
            r#"{"id":1,"method":"agent.focus","params":{"terminal_id":"t","pane_id":"w1:p1"}}"#;
        assert_eq!(parse(extra).unwrap_err().code, ErrorCode::InvalidParams);
        let top = r#"{"id":1,"method":"agent.focus","params":{"terminal_id":"t"},"x":1}"#;
        assert_eq!(parse(top).unwrap_err().code, ErrorCode::MalformedFrame);
        let no_keys = r#"{"id":1,"method":"agent.send_keys","params":{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","terminal_id":"t","keys":[]}}"#;
        assert_eq!(parse(no_keys).unwrap_err().code, ErrorCode::InvalidParams);
    }

    #[test]
    fn rejects_oversized_frames() {
        let big = format!(
            r#"{{"id":1,"method":"hello","params":{{"x":"{}"}}}}"#,
            "a".repeat(limits::MAX_FRAME_BYTES)
        );
        assert_eq!(parse(&big).unwrap_err().code, ErrorCode::FrameTooLarge);
    }

    #[test]
    fn attachment_begin_size_is_bounded() {
        let begin = |size: u64| {
            format!(
                r#"{{"id":1,"method":"attachment.begin","params":{{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","name":"a.png","size":{size},"sha256":"{}"}}}}"#,
                "0".repeat(64)
            )
        };
        assert!(parse(&begin(1)).is_ok());
        assert!(parse(&begin(limits::MAX_ATTACHMENT_BYTES)).is_ok());
        for size in [0, limits::MAX_ATTACHMENT_BYTES + 1] {
            assert_eq!(
                parse(&begin(size)).unwrap_err().code,
                ErrorCode::InvalidParams
            );
        }
    }

    #[test]
    fn largest_chunk_fits_a_frame() {
        let frame = ClientFrame {
            id: RequestId::MAX,
            request: Request::AttachmentChunk(AttachmentChunkParams {
                upload_id: UploadId::new("f".repeat(32)).unwrap(),
                offset: limits::MAX_ATTACHMENT_BYTES,
                data: ChunkData::new("A".repeat(43691) + "=").unwrap(),
            }),
        };
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.len() < limits::MAX_FRAME_BYTES, "{}", json.len());
        assert_eq!(parse(&json).unwrap(), frame);
        assert!(!format!("{frame:?}").contains("AAAA"));
    }

    #[test]
    fn destructive_confirm_defaults_false() {
        let f =
            parse(r#"{"id":1,"method":"workspace.close","params":{"workspace_id":"w6"}}"#).unwrap();
        assert!(matches!(
            f.request,
            Request::WorkspaceClose(WorkspaceCloseParams { confirm: false, .. })
        ));
    }

    #[test]
    fn older_clients_tolerate_additive_server_changes() {
        let ev: ServerFrame = serde_json::from_str(
            r#"{"kind":"event","seq":2,"event":{"name":"agent.renamed","x":1}}"#,
        )
        .unwrap();
        assert_eq!(
            ev,
            ServerFrame::Event {
                seq: 2,
                event: Event::Unrecognized
            }
        );
        let err: ServerFrame = serde_json::from_str(
            r#"{"kind":"error","id":4,"error":{"code":"quota_exceeded","message":"m"}}"#,
        )
        .unwrap();
        assert!(matches!(
            err,
            ServerFrame::Error {
                error: ErrorBody {
                    code: ErrorCode::Unrecognized,
                    ..
                },
                ..
            }
        ));
        let status: AgentStatus = serde_json::from_str(r#""sleeping""#).unwrap();
        assert_eq!(status, AgentStatus::Unknown);
    }

    #[test]
    fn server_frames_serialize() {
        let ev = ServerFrame::Event {
            seq: 1,
            event: Event::FlockChanged {},
        };
        assert_eq!(
            serde_json::to_string(&ev).unwrap(),
            r#"{"kind":"event","seq":1,"event":{"name":"flock.changed"}}"#
        );
        let ok = ServerFrame::Result {
            id: 9,
            result: Response::Ok,
        };
        assert_eq!(
            serde_json::to_string(&ok).unwrap(),
            r#"{"kind":"result","id":9,"result":{"type":"ok"}}"#
        );
        let back: ServerFrame = serde_json::from_str(&serde_json::to_string(&ok).unwrap()).unwrap();
        assert_eq!(back, ok);
    }
}
