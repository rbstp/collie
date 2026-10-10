use std::borrow::Cow;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::*;
use crate::limits;
use crate::output::OutputPatch;

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
    #[serde(rename = "unpair")]
    Unpair(Empty),

    #[serde(rename = "flock.snapshot")]
    FlockSnapshot(Empty),
    #[serde(rename = "agent.read")]
    AgentRead(ReadParams),
    #[serde(rename = "pane.read")]
    PaneRead(ReadParams),
    #[serde(rename = "agent.watch")]
    AgentWatch(AgentWatchParams),
    #[serde(rename = "task.options")]
    TaskOptions(Empty),
    #[serde(rename = "agent.draft")]
    AgentDraft(AgentTarget),

    #[serde(rename = "agent.prompt")]
    AgentPrompt(AgentPromptParams),
    #[serde(rename = "agent.send_keys")]
    AgentSendKeys(AgentSendKeysParams),
    #[serde(rename = "agent.type_text")]
    AgentTypeText(AgentTypeTextParams),
    #[serde(rename = "agent.focus")]
    AgentFocus(AgentTarget),
    #[serde(rename = "agent.scroll_bottom")]
    AgentScrollBottom(AgentTarget),
    #[serde(rename = "agent.answer_notice")]
    AgentAnswerNotice(AgentAnswerNoticeParams),
    #[serde(rename = "agent.slash_draft")]
    AgentSlashDraft(AgentSlashDraftParams),
    #[serde(rename = "agent.star")]
    AgentStar(AgentStarParams),
    #[serde(rename = "task.new")]
    TaskNew(TaskNewParams),
    #[serde(rename = "task.folders")]
    TaskFolders(TaskFoldersParams),
    #[serde(rename = "task.worktrees")]
    TaskWorktrees(TaskWorktreesParams),
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
    #[serde(rename = "push.activity_end")]
    PushActivityEnd(PushActivityEndParams),

    #[serde(rename = "attachment.begin")]
    AttachmentBegin(AttachmentBeginParams),
    #[serde(rename = "attachment.chunk")]
    AttachmentChunk(AttachmentChunkParams),
    #[serde(rename = "attachment.commit")]
    AttachmentCommit(AttachmentCommitParams),
    #[serde(rename = "attachment.abort")]
    AttachmentAbort(AttachmentAbortParams),

    #[serde(rename = "terminal.challenge")]
    TerminalChallenge(AgentTarget),
    #[serde(rename = "terminal.grant")]
    TerminalGrant(TerminalGrantParams),
    #[serde(rename = "terminal.watch")]
    TerminalWatch(TerminalWatchParams),
    #[serde(rename = "terminal.run")]
    TerminalRun(TerminalRunParams),
    #[serde(rename = "terminal.send_keys")]
    TerminalSendKeys(AgentSendKeysParams),
    #[serde(rename = "terminal.lock")]
    TerminalLock(Empty),
}

impl Request {
    pub const METHODS: &[&str] = &[
        "hello",
        "pair.complete",
        "unpair",
        "flock.snapshot",
        "agent.read",
        "pane.read",
        "agent.watch",
        "task.options",
        "agent.draft",
        "agent.prompt",
        "agent.send_keys",
        "agent.type_text",
        "agent.focus",
        "agent.scroll_bottom",
        "agent.answer_notice",
        "agent.slash_draft",
        "agent.star",
        "task.new",
        "task.folders",
        "task.worktrees",
        "workspace.close",
        "pane.close",
        "approval.list",
        "approval.decide",
        "push.register",
        "push.activity_token",
        "push.activity_end",
        "attachment.begin",
        "attachment.chunk",
        "attachment.commit",
        "attachment.abort",
        "terminal.challenge",
        "terminal.grant",
        "terminal.watch",
        "terminal.run",
        "terminal.send_keys",
        "terminal.lock",
    ];

    pub fn method(&self) -> &'static str {
        match self {
            Self::Hello(_) => "hello",
            Self::PairComplete(_) => "pair.complete",
            Self::Unpair(_) => "unpair",
            Self::FlockSnapshot(_) => "flock.snapshot",
            Self::AgentRead(_) => "agent.read",
            Self::PaneRead(_) => "pane.read",
            Self::AgentWatch(_) => "agent.watch",
            Self::TaskOptions(_) => "task.options",
            Self::AgentDraft(_) => "agent.draft",
            Self::AgentPrompt(_) => "agent.prompt",
            Self::AgentSendKeys(_) => "agent.send_keys",
            Self::AgentTypeText(_) => "agent.type_text",
            Self::AgentFocus(_) => "agent.focus",
            Self::AgentScrollBottom(_) => "agent.scroll_bottom",
            Self::AgentAnswerNotice(_) => "agent.answer_notice",
            Self::AgentSlashDraft(_) => "agent.slash_draft",
            Self::AgentStar(_) => "agent.star",
            Self::TaskNew(_) => "task.new",
            Self::TaskFolders(_) => "task.folders",
            Self::TaskWorktrees(_) => "task.worktrees",
            Self::WorkspaceClose(_) => "workspace.close",
            Self::PaneClose(_) => "pane.close",
            Self::ApprovalList(_) => "approval.list",
            Self::ApprovalDecide(_) => "approval.decide",
            Self::PushRegister(_) => "push.register",
            Self::PushActivityToken(_) => "push.activity_token",
            Self::PushActivityEnd(_) => "push.activity_end",
            Self::AttachmentBegin(_) => "attachment.begin",
            Self::AttachmentChunk(_) => "attachment.chunk",
            Self::AttachmentCommit(_) => "attachment.commit",
            Self::AttachmentAbort(_) => "attachment.abort",
            Self::TerminalChallenge(_) => "terminal.challenge",
            Self::TerminalGrant(_) => "terminal.grant",
            Self::TerminalWatch(_) => "terminal.watch",
            Self::TerminalRun(_) => "terminal.run",
            Self::TerminalSendKeys(_) => "terminal.send_keys",
            Self::TerminalLock(_) => "terminal.lock",
        }
    }

    pub fn class(&self) -> MethodClass {
        match self {
            Self::Hello(_) | Self::PairComplete(_) | Self::Unpair(_) => MethodClass::Session,
            Self::FlockSnapshot(_)
            | Self::AgentRead(_)
            | Self::PaneRead(_)
            | Self::AgentWatch(_)
            | Self::TaskOptions(_)
            | Self::AgentDraft(_)
            | Self::ApprovalList(_) => MethodClass::Read,
            Self::AgentPrompt(_)
            | Self::AgentSendKeys(_)
            | Self::AgentTypeText(_)
            | Self::AgentFocus(_)
            | Self::AgentScrollBottom(_)
            | Self::AgentAnswerNotice(_)
            | Self::AgentSlashDraft(_)
            | Self::AgentStar(_)
            | Self::TaskNew(_)
            | Self::TaskFolders(_)
            | Self::TaskWorktrees(_)
            | Self::WorkspaceClose(_)
            | Self::PaneClose(_)
            | Self::AttachmentBegin(_)
            | Self::AttachmentChunk(_)
            | Self::AttachmentCommit(_)
            | Self::AttachmentAbort(_) => MethodClass::Drive,
            Self::ApprovalDecide(_) => MethodClass::Approval,
            Self::PushRegister(_) | Self::PushActivityToken(_) | Self::PushActivityEnd(_) => {
                MethodClass::Push
            }
            Self::TerminalChallenge(_)
            | Self::TerminalGrant(_)
            | Self::TerminalWatch(_)
            | Self::TerminalRun(_)
            | Self::TerminalSendKeys(_)
            | Self::TerminalLock(_) => MethodClass::Terminal,
        }
    }
}

/// Terminal calls reach plain shells: collied refuses them unless the machine enables them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodClass {
    Session,
    Read,
    Drive,
    Approval,
    Push,
    Terminal,
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
    /// The phone's terminal key, recorded with the pairing: it signs terminal grants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_key: Option<TerminalKey>,
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
    #[schemars(range(min = 1, max = limits::MAX_READ_LINES))]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = limits::MAX_READ_LINES))]
    pub lines: Option<u16>,
    /// The phone is in Low Data Mode: collied reads the screen once a second.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub low_data: bool,
}

impl AgentWatchParams {
    pub fn lines(&self) -> u16 {
        watch_lines(self.lines)
    }
}

fn watch_lines(lines: Option<u16>) -> u16 {
    lines.map_or(limits::DEFAULT_WATCH_LINES, |n| {
        n.clamp(1, limits::MAX_READ_LINES)
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTarget {
    pub terminal_id: TerminalId,
}

/// A key Claude Code reads as an answer to a notice above its input box; never Enter or text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum NoticeDigit {
    #[serde(rename = "0")]
    Zero,
    #[serde(rename = "1")]
    One,
    #[serde(rename = "2")]
    Two,
    #[serde(rename = "3")]
    Three,
    #[serde(rename = "4")]
    Four,
}

impl NoticeDigit {
    pub fn new(digit: u8) -> Option<Self> {
        match digit {
            0 => Some(Self::Zero),
            1 => Some(Self::One),
            2 => Some(Self::Two),
            3 => Some(Self::Three),
            4 => Some(Self::Four),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "0",
            Self::One => "1",
            Self::Two => "2",
            Self::Three => "3",
            Self::Four => "4",
        }
    }

    pub fn value(self) -> u8 {
        match self {
            Self::Zero => 0,
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
            Self::Four => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentAnswerNoticeParams {
    pub terminal_id: TerminalId,
    pub digit: NoticeDigit,
    /// The option's label as the phone showed it: a follow-up reuses a digit with another
    /// meaning, so collied sends the digit only while the screen lists this exact option.
    pub label: Label,
}

/// The phone's slash command token, mirrored into Claude Code's input box so its command
/// menu shows; empty clears the box. The box is replaced only when it holds
/// `expected_draft`, as for `agent.prompt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSlashDraftParams {
    pub terminal_id: TerminalId,
    pub command: SlashCommand,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_draft: Option<DraftText>,
}

/// Stars are the machine's, shared by every paired phone. Unstarring needs no live pane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentStarParams {
    pub terminal_id: TerminalId,
    pub starred: bool,
}

/// `signature` is over [`terminal_grant_message`] for the challenge this session was given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TerminalGrantParams {
    pub terminal_id: TerminalId,
    pub challenge: Nonce,
    pub signature: Signature,
}

/// Shares the session's single watch with `agent.watch`; `agent.watch {terminal_id: null}`
/// stops either.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TerminalWatchParams {
    pub terminal_id: TerminalId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = limits::MAX_READ_LINES))]
    pub lines: Option<u16>,
    /// As in `agent.watch`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub low_data: bool,
}

impl TerminalWatchParams {
    pub fn lines(&self) -> u16 {
        watch_lines(self.lines)
    }
}

/// One line typed into a shell pane, then Enter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TerminalRunParams {
    pub op_id: OpId,
    pub terminal_id: TerminalId,
    pub text: PromptText,
}

impl TerminalRunParams {
    pub fn is_valid(&self) -> bool {
        is_one_line(self.text.as_str())
    }
}

/// Text collied types into a pane: Enter is what submits it, and bidi and invisible
/// characters would let it read as something else.
pub fn is_one_line(text: &str) -> bool {
    !text.chars().any(|c| c == '\n' || c == '\t' || is_format(c))
}

/// What the phone's terminal key signs to unlock one terminal on one machine: each part
/// length-prefixed (u32 big-endian), so no two inputs give the same bytes.
pub fn terminal_grant_message(
    node_id: &str,
    terminal_id: &TerminalId,
    challenge: &Nonce,
) -> Vec<u8> {
    let mut out = Vec::new();
    for part in [
        "collie terminal grant v1",
        node_id,
        terminal_id.as_str(),
        challenge.as_str(),
    ] {
        out.extend_from_slice(&(part.len() as u32).to_be_bytes());
        out.extend_from_slice(part.as_bytes());
    }
    out
}

/// For a Claude Code agent, collied replaces an unsent draft in the Mac's input box only
/// when it equals `expected_draft`; any other draft fails with `draft_changed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPromptParams {
    pub op_id: OpId,
    pub terminal_id: TerminalId,
    pub text: PromptText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_draft: Option<DraftText>,
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
    #[serde(rename = "left")]
    Left,
    #[serde(rename = "shift+left")]
    ShiftLeft,
    #[serde(rename = "right")]
    Right,
    #[serde(rename = "tab")]
    Tab,
    #[serde(rename = "shift+tab")]
    ShiftTab,
    #[serde(rename = "ctrl+c")]
    CtrlC,
    #[serde(rename = "ctrl+enter")]
    CtrlEnter,
}

impl Key {
    pub fn herdr_name(self) -> &'static str {
        match self {
            Self::Enter => "enter",
            Self::Esc => "esc",
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::ShiftLeft => "shift+left",
            Self::Right => "right",
            Self::Tab => "tab",
            Self::ShiftTab => "shift+tab",
            Self::CtrlC => "ctrl+c",
            Self::CtrlEnter => "ctrl+enter",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSendKeysParams {
    pub op_id: OpId,
    pub terminal_id: TerminalId,
    #[schemars(length(min = 1, max = limits::MAX_KEYS_PER_CALL))]
    pub keys: Vec<Key>,
}

impl AgentSendKeysParams {
    pub fn is_valid(&self) -> bool {
        (1..=limits::MAX_KEYS_PER_CALL).contains(&self.keys.len())
    }
}

/// Typed into the free-text field of a blocked Claude Code question or plan, then Enter.
/// One line without bidi or invisible format characters: Enter is what submits it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTypeTextParams {
    pub op_id: OpId,
    pub terminal_id: TerminalId,
    pub text: PromptText,
}

impl AgentTypeTextParams {
    pub fn is_valid(&self) -> bool {
        is_one_line(self.text.as_str())
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
    /// When set, collied creates this empty folder directly inside `cwd` and starts there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_folder: Option<FolderName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<TaskWorktree>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskWorktree {
    Create { branch: String },
    Open { path: Cwd },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskWorktreesParams {
    pub cwd: Cwd,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskFoldersParams {
    pub path: Cwd,
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
    /// Pick `Approval.choices[choice]`; only on an approval that offers no other decision.
    Choose,
}

/// `choice` is set exactly when `decision` is `choose`. `note`, one line of at most 200
/// characters without bidi or invisible format characters, goes with `approve` or `deny`
/// on an approval with `supports_note`: collied types it into the option's amend field
/// before Enter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApprovalDecideParams {
    pub approval_id: ApprovalId,
    pub decision: Decision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<u8>,
    pub nonce: Nonce,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<PromptText>,
}

impl ApprovalDecideParams {
    pub fn is_valid(&self) -> bool {
        (self.decision == Decision::Choose) == self.choice.is_some()
            && self.note.as_ref().is_none_or(|n| {
                matches!(self.decision, Decision::Approve | Decision::Deny)
                    && is_one_line(n.as_str())
                    && n.as_str().chars().count() <= limits::MAX_NOTE_CHARS
            })
    }
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
    pub environment: ApnsEnvironment,
    pub notification_key: NotificationKey,
    /// No alert when an agent finishes a turn.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mute_done: bool,
}

/// A Live Activity's update token. collied pushes the followed terminal's status to it,
/// in the APNs environment of the device's own `push.register`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PushActivityTokenParams {
    pub activity_id: ActivityId,
    pub terminal_id: TerminalId,
    pub token: PushToken,
    /// The activity shows an approval's command and its Approve and Deny buttons. Without
    /// it collied keeps sending that device the approval alert.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shows_approvals: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PushActivityEndParams {
    pub activity_id: ActivityId,
}

/// `sha256` covers the whole file; collied checks it and the size before keeping it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachmentBeginParams {
    pub op_id: OpId,
    pub name: AttachmentName,
    #[schemars(range(min = 1, max = limits::MAX_ATTACHMENT_BYTES))]
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
    Terminal(TerminalRead),
    TaskOptions(TaskOptions),
    TaskWorktrees {
        source: Cwd,
        worktrees: Vec<TaskWorktreeInfo>,
    },
    TaskStarted {
        workspace_id: WorkspaceId,
        terminal_id: TerminalId,
    },
    /// The folders directly inside `path`, its canonical form, sorted; `truncated` when
    /// some were left out.
    TaskFolders {
        path: Cwd,
        folders: Vec<FolderName>,
        truncated: bool,
    },
    Approvals {
        approvals: Vec<Approval>,
        /// For a watch refresh while the phone's sessions are closed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan_usage: Option<PlanUsage>,
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
    /// `None` when there is no input box to read: not a Claude Code agent, or a dialog.
    Draft {
        text: Option<String>,
    },
    TerminalChallenge {
        terminal_id: TerminalId,
        challenge: Nonce,
        ttl_ms: u64,
    },
    TerminalGranted {
        terminal_id: TerminalId,
        ttl_ms: u64,
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
    /// The latest line of the agent's reply, from its transcript on the machine.
    pub last_line: Option<String>,
    /// Percent of the context window left, from the same transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(max = 100))]
    pub context_left: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt: Option<String>,
    /// When the transcript last changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_ms: Option<u64>,
}

/// From Claude Code's status line input, as `collied statusline` last recorded it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlanUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<UsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<UsageWindow>,
    pub recorded_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexUsage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CodexUsage {
    pub used: u64,
    pub limit: u64,
    pub resets_at_ms: u64,
    pub recorded_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UsageWindow {
    #[schemars(range(max = 100))]
    pub used_percent: u8,
    pub resets_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Flock {
    pub seq: u64,
    pub machine: MachineInfo,
    pub workspaces: Vec<Workspace>,
    pub agents: Vec<Agent>,
    pub approvals: Vec<Approval>,
    /// Shell panes: only when the machine enables terminals.
    #[serde(default)]
    pub terminals: Vec<Terminal>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub terminals_enabled: bool,
    /// The machine's plan usage, with or without a Claude Code agent open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_usage: Option<PlanUsage>,
    /// Starred panes, which collied follows across a herdr restart until they close.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starred: Vec<TerminalId>,
}

/// A pane with no agent. `label` is the name given in herdr, never a title the pane's
/// program sets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Terminal {
    pub terminal_id: TerminalId,
    pub workspace_id: WorkspaceId,
    pub label: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalRead {
    pub terminal_id: TerminalId,
    pub source: ReadSource,
    pub ansi: String,
    pub truncated: bool,
    /// Rows of `ansi` (split on `\n`) that continue the row above as one paragraph the agent
    /// wrapped at the Mac pane's width: after a space (`wraps`) or inside a word (`splits`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wraps: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub splits: Vec<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub copilot_scrollbar: bool,
}

impl TerminalRead {
    /// `ansi` with the rows in `wraps` and `splits` joined onto the row above; `None` when
    /// there are none.
    pub fn reflowed(&self) -> Option<String> {
        if self.wraps.is_empty() && self.splits.is_empty() {
            return None;
        }
        let rows: Vec<&str> = self.ansi.split('\n').collect();
        let mut sep = vec![None; rows.len()];
        for (list, s) in [(&self.splits, ""), (&self.wraps, " ")] {
            for &i in list {
                if let Some(slot) = sep.get_mut(i as usize) {
                    *slot = Some(s);
                }
            }
        }
        let mut out = String::with_capacity(self.ansi.len());
        for (i, row) in rows.iter().enumerate() {
            let row = if self.copilot_scrollbar
                && (sep[i].is_some() || sep.get(i + 1).is_some_and(Option::is_some))
            {
                Cow::Owned(trim_copilot_line_end(row))
            } else {
                Cow::Borrowed(*row)
            };
            match sep[i] {
                Some(s) if i > 0 => {
                    let line = out.rfind('\n').map_or(0, |n| n + 1);
                    let kept = trim_line_end(&out[line..]);
                    out.truncate(line);
                    out.push_str(&kept);
                    out.push_str(s);
                    out.push_str(&trim_row_start(&row));
                }
                _ => {
                    if i > 0 {
                        out.push('\n');
                    }
                    out.push_str(&row);
                }
            }
        }
        Some(out)
    }
}

/// The SGR escape `s` ends with, if any: collied sends no other escapes.
fn sgr_suffix(s: &str) -> Option<usize> {
    let start = s.strip_suffix('m')?.rfind('\u{1b}')?;
    s[start + 1..s.len() - 1]
        .strip_prefix('[')?
        .bytes()
        .all(|b| b.is_ascii_digit() || b == b';' || b == b':')
        .then_some(start)
}

/// `line` without its trailing spaces and `\r`, keeping the SGR escapes among them.
fn trim_line_end(mut line: &str) -> String {
    let mut sgr = Vec::new();
    loop {
        if let Some(rest) = line.strip_suffix([' ', '\r']) {
            line = rest;
        } else if let Some(start) = sgr_suffix(line) {
            sgr.push(&line[start..]);
            line = &line[..start];
        } else {
            break;
        }
    }
    sgr.into_iter().rev().fold(line.to_owned(), |s, e| s + e)
}

fn trim_copilot_line_end(line: &str) -> String {
    let trimmed = trim_line_end(line);
    let mut bare = trimmed.as_str();
    while let Some(start) = sgr_suffix(bare) {
        bare = &bare[..start];
    }
    let Some(mut bare) = bare.strip_suffix('┃') else {
        return trimmed;
    };
    while let Some(start) = sgr_suffix(bare) {
        bare = &bare[..start];
    }
    trim_line_end(bare)
}

/// `row` without its leading spaces, keeping the SGR escapes among them.
fn trim_row_start(mut row: &str) -> String {
    let mut out = String::new();
    loop {
        if let Some(rest) = row.strip_prefix(' ') {
            row = rest;
        } else if let Some(end) = row.strip_prefix("\u{1b}[").and_then(|r| r.find('m')) {
            out.push_str(&row[..end + 3]);
            row = &row[end + 3..];
        } else {
            break;
        }
    }
    out + row
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskOptions {
    pub agents: Vec<AgentKind>,
    pub default_agent: AgentKind,
    pub recent_cwds: Vec<Cwd>,
    pub roots: Vec<Cwd>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskWorktreeInfo {
    pub path: Cwd,
    pub branch: Option<String>,
    pub open: bool,
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
    /// The menu on screen, in order, whenever collied could read one.
    #[serde(default)]
    pub choices: Vec<ApprovalChoice>,
    /// Whether collied takes `agent.send_keys` and `agent.type_text` on this prompt: false for
    /// any prompt that grants a permission, and from a collied that predates the field.
    #[serde(default)]
    pub accepts_input: bool,
    /// The menu has a free-text option that `agent.type_text` fills: a question's "Type
    /// something." (with `accepts_input`), or a plan's "Tell Claude what to change", which
    /// takes text but no keys (`accepts_input` false).
    #[serde(default)]
    pub has_text_field: bool,
    /// `approval.decide` takes a `note` with `approve` and `deny`: the prompt shows "Tab
    /// to amend".
    #[serde(default)]
    pub supports_note: bool,
    pub nonce: Nonce,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

/// `index` is 0-based; `current` marks the option under the menu cursor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalChoice {
    pub index: u8,
    pub label: String,
    pub current: bool,
    /// The lines under the option's first line: a question option's description, or the
    /// rest of a label that wrapped. Absent from a collied that predates the field, which
    /// joins them into `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
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
    /// `applied` for a `choose` decision. Its own outcome, so an older app decodes it as
    /// `other` instead of failing on an unknown decision.
    Chosen {
        choice: u8,
        by: String,
    },
    /// `unconfirmed` for a `choose` decision.
    ChosenUnconfirmed {
        choice: u8,
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
    #[serde(rename = "agent.output_patch")]
    AgentOutputPatch(OutputPatch),
    #[serde(rename = "approval.needed")]
    ApprovalNeeded { approval: Approval },
    #[serde(rename = "approval.resolved")]
    ApprovalResolved {
        approval_id: ApprovalId,
        outcome: ApprovalOutcome,
    },
    #[serde(rename = "flock.changed")]
    FlockChanged {},
    #[serde(rename = "plan.usage")]
    PlanUsage { plan_usage: PlanUsage },
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
    DraftChanged,
    DraftNotCleared,
    TerminalsDisabled,
    TerminalLocked,
    TerminalKeyMissing,
    Internal,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    /// With `draft_changed`: the text now in the Mac's input box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
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
                draft: None,
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
        Request::AgentTypeText(p) => p.is_valid(),
        Request::TerminalRun(p) => p.is_valid(),
        Request::TerminalSendKeys(p) => p.is_valid(),
        Request::ApprovalDecide(p) => p.is_valid(),
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
    fn watch_lines_are_optional_and_clamped() {
        let watch = |params: &str| {
            let frame = parse(&format!(
                r#"{{"id":1,"method":"agent.watch","params":{params}}}"#
            ))
            .unwrap();
            let Request::AgentWatch(p) = frame.request.clone() else {
                panic!("not a watch");
            };
            let json = serde_json::to_string(&frame).unwrap();
            assert_eq!(parse(&json).unwrap(), frame);
            (p.lines(), json)
        };
        let (lines, json) = watch(r#"{"terminal_id":"t"}"#);
        assert_eq!(lines, 200);
        assert!(!json.contains("lines"));
        let (lines, json) = watch(r#"{"terminal_id":"t","lines":500}"#);
        assert_eq!(lines, 500);
        assert!(json.contains(r#""lines":500"#));
        assert_eq!(watch(r#"{"terminal_id":"t","lines":0}"#).0, 1);
        assert_eq!(watch(r#"{"terminal_id":"t","lines":65535}"#).0, 1000);
        assert_eq!(watch(r#"{"terminal_id":null,"lines":1}"#).0, 1);
    }

    #[test]
    fn low_data_is_an_optional_bool() {
        for method in ["agent.watch", "terminal.watch"] {
            let watch = |extra: &str| {
                parse(&format!(
                    r#"{{"id":1,"method":"{method}","params":{{"terminal_id":"t"{extra}}}}}"#
                ))
            };
            let low_data = |frame: &ClientFrame| match &frame.request {
                Request::AgentWatch(p) => p.low_data,
                Request::TerminalWatch(p) => p.low_data,
                _ => panic!("not a watch"),
            };
            let off = watch("").unwrap();
            assert!(!low_data(&off));
            assert!(!serde_json::to_string(&off).unwrap().contains("low_data"));
            assert!(!low_data(&watch(r#","low_data":false"#).unwrap()));
            let on = watch(r#","low_data":true"#).unwrap();
            assert!(low_data(&on));
            assert_eq!(parse(&serde_json::to_string(&on).unwrap()).unwrap(), on);
            for bad in [
                r#","low_data":"true""#,
                r#","low_data":1"#,
                r#","low_data":null"#,
            ] {
                assert_eq!(watch(bad).unwrap_err().code, ErrorCode::InvalidParams);
            }
        }
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
    fn scroll_bottom_names_only_the_agent() {
        let frame =
            parse(r#"{"id":1,"method":"agent.scroll_bottom","params":{"terminal_id":"term_1"}}"#)
                .unwrap();
        assert_eq!(frame.request.class(), MethodClass::Drive);
        assert_eq!(frame.request.method(), "agent.scroll_bottom");
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(parse(&json).unwrap(), frame);
        for params in [
            r#"{"terminal_id":"t","keys":["ctrl+end"]}"#,
            r#"{"terminal_id":"t","text":"x"}"#,
            "{}",
        ] {
            let bad = format!(r#"{{"id":1,"method":"agent.scroll_bottom","params":{params}}}"#);
            assert_eq!(parse(&bad).unwrap_err().code, ErrorCode::InvalidParams);
        }
    }

    #[test]
    fn answer_notice_carries_one_digit() {
        let answer = |params: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"agent.answer_notice","params":{params}}}"#
            ))
        };
        for (digit, value) in [("0", 0), ("1", 1), ("2", 2), ("3", 3), ("4", 4)] {
            let frame = answer(&format!(
                r#"{{"terminal_id":"term_1","digit":"{digit}","label":"Chat in main session"}}"#
            ))
            .unwrap();
            assert_eq!(frame.request.class(), MethodClass::Drive);
            assert_eq!(frame.request.method(), "agent.answer_notice");
            let Request::AgentAnswerNotice(p) = &frame.request else {
                panic!("not an answer");
            };
            assert_eq!(p.digit.as_str(), digit);
            assert_eq!(NoticeDigit::new(value), Some(p.digit));
            assert_eq!(p.digit.value(), value);
            assert_eq!(p.label.as_str(), "Chat in main session");
            let json = serde_json::to_string(&frame).unwrap();
            assert_eq!(parse(&json).unwrap(), frame);
        }
        assert_eq!(NoticeDigit::new(5), None);
        for params in [
            r#"{"terminal_id":"t","digit":"5","label":"Bad"}"#,
            r#"{"terminal_id":"t","digit":"1\r","label":"Bad"}"#,
            r#"{"terminal_id":"t","digit":"11","label":"Bad"}"#,
            r#"{"terminal_id":"t","digit":1,"label":"Bad"}"#,
            r#"{"terminal_id":"t","digit":"yes","label":"Bad"}"#,
            r#"{"terminal_id":"t","digit":"1","label":"Bad","text":"x"}"#,
            r#"{"terminal_id":"t","digit":"1","label":"Bad","keys":["enter"]}"#,
            r#"{"terminal_id":"t","digit":"1"}"#,
            r#"{"terminal_id":"t","digit":"1","label":""}"#,
            r#"{"terminal_id":"t","digit":"1","label":"Bad\r"}"#,
            r#"{"terminal_id":"t"}"#,
        ] {
            assert_eq!(
                answer(params).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{params}"
            );
        }
    }

    #[test]
    fn slash_draft_carries_one_command_token() {
        let slash = |params: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"agent.slash_draft","params":{params}}}"#
            ))
        };
        let frame =
            slash(r#"{"terminal_id":"term_1","command":"/sk","expected_draft":"/s"}"#).unwrap();
        assert_eq!(frame.request.class(), MethodClass::Drive);
        assert_eq!(frame.request.method(), "agent.slash_draft");
        let Request::AgentSlashDraft(p) = &frame.request else {
            panic!("not a slash draft");
        };
        assert_eq!(p.command.as_str(), "/sk");
        assert_eq!(p.expected_draft.as_ref().unwrap().as_str(), "/s");
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(parse(&json).unwrap(), frame);
        let Request::AgentSlashDraft(clear) = slash(r#"{"terminal_id":"t","command":""}"#)
            .unwrap()
            .request
        else {
            panic!("not a slash draft");
        };
        assert_eq!((clear.command.as_str(), clear.expected_draft), ("", None));
        for params in [
            r#"{"terminal_id":"t","command":"s"}"#,
            r#"{"terminal_id":"t","command":"/s x"}"#,
            r#"{"terminal_id":"t","command":"/s\n"}"#,
            r#"{"terminal_id":"t","command":"/s\r"}"#,
            r#"{"terminal_id":"t","command":"/s\u001b[201~"}"#,
            r#"{"terminal_id":"t","command":"/s","expected_draft":"\u001b[2J"}"#,
            r#"{"terminal_id":"t","command":"/s","keys":["enter"]}"#,
            r#"{"terminal_id":"t","command":"/s","text":"x"}"#,
            r#"{"terminal_id":"t"}"#,
        ] {
            assert_eq!(
                slash(params).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{params}"
            );
        }
        assert_eq!(crate::PROTOCOL_VERSION, 15);
    }

    #[test]
    fn task_folders_and_new_folder() {
        let frame =
            parse(r#"{"id":1,"method":"task.folders","params":{"path":"/Users/me/git"}}"#).unwrap();
        assert_eq!(frame.request.class(), MethodClass::Drive);
        assert_eq!(frame.request.method(), "task.folders");
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(parse(&json).unwrap(), frame);
        for params in [
            r#"{"path":"/a","recursive":true}"#,
            r#"{"path":"git"}"#,
            "{}",
        ] {
            let bad = format!(r#"{{"id":1,"method":"task.folders","params":{params}}}"#);
            assert_eq!(parse(&bad).unwrap_err().code, ErrorCode::InvalidParams);
        }
        let task = |extra: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"task.new","params":{{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","cwd":"/a","agent":"claude","prompt":"go"{extra}}}}}"#
            ))
        };
        let Request::TaskNew(plain) = task("").unwrap().request else {
            panic!("not a task");
        };
        assert_eq!(plain.new_folder, None);
        let frame = task(r#","new_folder":"app""#).unwrap();
        let Request::TaskNew(p) = &frame.request else {
            panic!("not a task");
        };
        assert_eq!(p.new_folder.as_ref().unwrap().as_str(), "app");
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(parse(&json).unwrap(), frame);
        for bad in [
            r#","new_folder":"../x""#,
            r#","new_folder":".x""#,
            r#","new_folder":"a/b""#,
        ] {
            assert_eq!(
                task(bad).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{bad}"
            );
        }
    }

    #[test]
    fn worktree_requests_are_explicit_and_bounded() {
        let list = parse(r#"{"id":1,"method":"task.worktrees","params":{"cwd":"/repo"}}"#).unwrap();
        assert_eq!(list.request.class(), MethodClass::Drive);
        assert_eq!(list.request.method(), "task.worktrees");
        let task = |worktree: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"task.new","params":{{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","cwd":"/repo","agent":"claude","prompt":"go","worktree":{worktree}}}}}"#
            ))
        };
        for choice in [
            r#"{"mode":"create","branch":"feature"}"#,
            r#"{"mode":"open","path":"/repo/feature"}"#,
        ] {
            let frame = task(choice).unwrap();
            assert_eq!(
                parse(&serde_json::to_string(&frame).unwrap()).unwrap(),
                frame
            );
        }
        for choice in [
            r#"{"mode":"create","branch":"feature","path":"/repo/feature"}"#,
            r#"{"mode":"open","path":"/repo/feature","extra":true}"#,
            r#"{"mode":"remove","path":"/repo/feature"}"#,
        ] {
            assert_eq!(task(choice).unwrap_err().code, ErrorCode::InvalidParams);
        }
    }

    #[test]
    fn prompt_expected_draft_is_optional() {
        let base = r#"{"id":1,"method":"agent.prompt","params":{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","terminal_id":"t","text":"hi""#;
        let Request::AgentPrompt(p) = parse(&format!("{base}}}}}")).unwrap().request else {
            panic!("not a prompt");
        };
        assert_eq!(p.expected_draft, None);
        assert!(
            !serde_json::to_string(&p)
                .unwrap()
                .contains("expected_draft")
        );
        let Request::AgentPrompt(p) =
            parse(&format!(r#"{base},"expected_draft":"one\n  two"}}}}"#))
                .unwrap()
                .request
        else {
            panic!("not a prompt");
        };
        assert_eq!(p.expected_draft.unwrap().as_str(), "one\n  two");
        let hostile = format!(r#"{base},"expected_draft":"\u001b[2J"}}}}"#);
        assert_eq!(parse(&hostile).unwrap_err().code, ErrorCode::InvalidParams);
        let draft =
            parse(r#"{"id":2,"method":"agent.draft","params":{"terminal_id":"t"}}"#).unwrap();
        assert_eq!(draft.request.class(), MethodClass::Read);
    }

    #[test]
    fn agent_transcript_fields_are_optional() {
        let older = r#"{"terminal_id":"term_1","workspace_id":"w1","kind":null,"name":null,"title":null,"status":"idle","status_since_ms":1,"cwd":null,"last_line":null}"#;
        let a: Agent = serde_json::from_str(older).unwrap();
        assert_eq!(
            (a.context_left, &a.last_prompt, a.last_activity_ms),
            (None, &None, None)
        );
        assert_eq!(serde_json::to_string(&a).unwrap(), older);
        let full = Agent {
            last_line: Some("Done.".into()),
            context_left: Some(100),
            last_prompt: Some("go".into()),
            last_activity_ms: Some(7),
            ..a
        };
        let json = serde_json::to_string(&full).unwrap();
        assert!(json.ends_with(
            r#""last_line":"Done.","context_left":100,"last_prompt":"go","last_activity_ms":7}"#
        ));
        assert_eq!(serde_json::from_str::<Agent>(&json).unwrap(), full);
        let event = ServerFrame::Event {
            seq: 2,
            event: Event::AgentStatus { agent: full },
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<ServerFrame>(&json).unwrap(), event);
    }

    #[test]
    fn plan_usage_is_additive() {
        let older = r#"{"seq":1,"machine":{"name":"m","node_id":"n","herdr_session":"default"},"workspaces":[],"agents":[],"approvals":[],"terminals":[]}"#;
        let f: Flock = serde_json::from_str(older).unwrap();
        assert_eq!(f.plan_usage, None);
        assert_eq!(serde_json::to_string(&f).unwrap(), older);
        let usage = PlanUsage {
            five_hour: Some(UsageWindow {
                used_percent: 24,
                resets_at_ms: 1_738_425_600_000,
            }),
            seven_day: None,
            recorded_ms: 1_738_420_000_000,
            codex: None,
        };
        let with = Flock {
            plan_usage: Some(usage.clone()),
            ..f
        };
        let json = serde_json::to_string(&with).unwrap();
        assert!(json.ends_with(
            r#""plan_usage":{"five_hour":{"used_percent":24,"resets_at_ms":1738425600000},"recorded_ms":1738420000000}}"#
        ));
        assert_eq!(serde_json::from_str::<Flock>(&json).unwrap(), with);
        let event = ServerFrame::Event {
            seq: 2,
            event: Event::PlanUsage { plan_usage: usage },
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""name":"plan.usage""#), "{json}");
        assert_eq!(serde_json::from_str::<ServerFrame>(&json).unwrap(), event);
    }

    #[test]
    fn stars_are_additive() {
        let json =
            r#"{"id":2,"method":"agent.star","params":{"terminal_id":"term_1","starred":true}}"#;
        let frame = parse(json).unwrap();
        assert_eq!(
            frame.request,
            Request::AgentStar(AgentStarParams {
                terminal_id: TerminalId::new("term_1").unwrap(),
                starred: true,
            })
        );
        assert_eq!(frame.request.class(), MethodClass::Drive);
        assert_eq!(serde_json::to_string(&frame).unwrap(), json);
        let extra = r#"{"id":2,"method":"agent.star","params":{"terminal_id":"t","starred":false,"pane_id":"w1:p1"}}"#;
        assert_eq!(parse(extra).unwrap_err().code, ErrorCode::InvalidParams);
        let missing = r#"{"id":2,"method":"agent.star","params":{"terminal_id":"t"}}"#;
        assert_eq!(parse(missing).unwrap_err().code, ErrorCode::InvalidParams);

        let older = r#"{"seq":1,"machine":{"name":"m","node_id":"n","herdr_session":"default"},"workspaces":[],"agents":[],"approvals":[],"terminals":[]}"#;
        let f: Flock = serde_json::from_str(older).unwrap();
        assert!(f.starred.is_empty());
        assert_eq!(serde_json::to_string(&f).unwrap(), older);
        let with = Flock {
            starred: vec![TerminalId::new("term_1").unwrap()],
            ..f
        };
        let json = serde_json::to_string(&with).unwrap();
        assert!(json.ends_with(r#""starred":["term_1"]}"#), "{json}");
        assert_eq!(serde_json::from_str::<Flock>(&json).unwrap(), with);
    }

    #[test]
    fn draft_changed_carries_the_current_draft() {
        let frame = ServerFrame::Error {
            id: Some(4),
            error: ErrorBody {
                code: ErrorCode::DraftChanged,
                message: "m".into(),
                draft: Some("typed on the Mac".into()),
            },
        };
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"error","id":4,"error":{"code":"draft_changed","message":"m","draft":"typed on the Mac"}}"#
        );
        assert_eq!(serde_json::from_str::<ServerFrame>(&json).unwrap(), frame);
        let older: ErrorBody =
            serde_json::from_str(r#"{"code":"not_found","message":"m"}"#).unwrap();
        assert_eq!(older.draft, None);
    }

    #[test]
    fn push_register_mute_done_is_additive() {
        let frame = |extra: &str| {
            format!(
                r#"{{"id":1,"method":"push.register","params":{{"apns_token":"{}","environment":"sandbox","notification_key":"{}"{extra}}}}}"#,
                "ab".repeat(32),
                "A".repeat(43)
            )
        };
        for (extra, muted) in [
            ("", false),
            (r#","mute_done":false"#, false),
            (r#","mute_done":true"#, true),
        ] {
            let Request::PushRegister(p) = parse(&frame(extra)).unwrap().request else {
                panic!("not a push.register");
            };
            assert_eq!(p.mute_done, muted, "{extra}");
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(json.contains("mute_done"), muted, "{json}");
        }
        assert_eq!(
            parse(&frame(r#","mute_done":"yes""#)).unwrap_err().code,
            ErrorCode::InvalidParams
        );
    }

    #[test]
    fn activity_token_and_end() {
        let activity = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";
        let token = "ab".repeat(40);
        let register = parse(&format!(
            r#"{{"id":1,"method":"push.activity_token","params":{{"activity_id":"{activity}","terminal_id":"term_1","token":"{token}"}}}}"#
        ))
        .unwrap();
        assert_eq!(register.request.class(), MethodClass::Push);
        assert!(!format!("{register:?}").contains(&token));
        let end = parse(&format!(
            r#"{{"id":2,"method":"push.activity_end","params":{{"activity_id":"{activity}"}}}}"#
        ))
        .unwrap();
        assert_eq!(end.request.class(), MethodClass::Push);
        assert_eq!(end.request.method(), "push.activity_end");
        let Request::PushActivityEnd(p) = end.request else {
            panic!("not an activity end");
        };
        assert_eq!(p.activity_id.as_str(), activity);
        for bad in [
            r#"{"activity_id":"a/b"}"#,
            r#"{"activity_id":""}"#,
            r#"{"activity_id":"a","terminal_id":"t"}"#,
        ] {
            let frame = format!(r#"{{"id":3,"method":"push.activity_end","params":{bad}}}"#);
            assert_eq!(parse(&frame).unwrap_err().code, ErrorCode::InvalidParams);
        }
    }

    #[test]
    fn choose_carries_exactly_one_choice() {
        let decide = |rest: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"approval.decide","params":{{"approval_id":"a1","nonce":"{}"{rest}}}}}"#,
                "A".repeat(43)
            ))
        };
        let Request::ApprovalDecide(p) = decide(r#","decision":"choose","choice":2"#)
            .unwrap()
            .request
        else {
            panic!("not a decision");
        };
        assert_eq!((p.decision, p.choice), (Decision::Choose, Some(2)));
        let Request::ApprovalDecide(p) = decide(r#","decision":"deny""#).unwrap().request else {
            panic!("not a decision");
        };
        assert!(!serde_json::to_string(&p).unwrap().contains("choice"));
        for bad in [
            r#","decision":"choose""#,
            r#","decision":"approve","choice":0"#,
            r#","decision":"choose","choice":256"#,
        ] {
            assert_eq!(
                decide(bad).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{bad}"
            );
        }
    }

    #[test]
    fn a_note_is_one_line_with_approve_or_deny() {
        let decide = |rest: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"approval.decide","params":{{"approval_id":"a1","nonce":"{}"{rest}}}}}"#,
                "A".repeat(43)
            ))
        };
        for ok in [
            r#","decision":"approve","note":"use a .tmp extension""#,
            r#","decision":"deny","note":"run the tests first""#,
            &format!(
                r#","decision":"deny","note":"{}""#,
                "é".repeat(limits::MAX_NOTE_CHARS)
            ),
        ] {
            let Request::ApprovalDecide(p) = decide(ok).unwrap().request else {
                panic!("not a decision");
            };
            assert!(p.note.is_some(), "{ok}");
        }
        for bad in [
            r#","decision":"approve_always","note":"x""#,
            r#","decision":"choose","choice":0,"note":"x""#,
            r#","decision":"approve","note":"a\nb""#,
            r#","decision":"approve","note":"a\tb""#,
            r#","decision":"approve","note":"a\u001b[Z""#,
            r#","decision":"approve","note":"ok \u202efi.exe""#,
            r#","decision":"deny","note":"a\u200bb""#,
            r#","decision":"deny","note":"use Redis\udb40\udc41""#,
            r#","decision":"deny","note":"  ""#,
            &format!(
                r#","decision":"approve","note":"{}""#,
                "a".repeat(limits::MAX_NOTE_CHARS + 1)
            ),
        ] {
            assert_eq!(
                decide(bad).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{bad}"
            );
        }
    }

    #[test]
    fn typed_text_is_one_line() {
        let typed = |text: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"agent.type_text","params":{{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","terminal_id":"t","text":{}}}}}"#,
                serde_json::to_string(text).unwrap()
            ))
        };
        assert!(typed("oui\u{202F}! c’est partagé 👍🏽").is_ok());
        let ok = typed("use Redis, it is shared").unwrap();
        assert_eq!(ok.request.class(), MethodClass::Drive);
        assert_eq!(ok.request.method(), "agent.type_text");
        for bad in [
            "a\nb",
            "a\tb",
            "a\u{1b}[2J",
            "  ",
            "use \u{202E}sideR",
            "use\u{2066} Redis",
            "\u{FEFF}yes",
            "use Redis\u{E0041}\u{E007F}",
            "\u{1F600}\u{E0100}",
        ] {
            assert_eq!(
                typed(bad).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn older_apps_tolerate_choices() {
        let chosen: ApprovalOutcome =
            serde_json::from_str(r#"{"outcome":"chosen","choice":1,"by":"phone"}"#).unwrap();
        assert_eq!(
            chosen,
            ApprovalOutcome::Chosen {
                choice: 1,
                by: "phone".into()
            }
        );
        #[derive(Deserialize)]
        #[serde(tag = "outcome", rename_all = "snake_case")]
        enum Older {
            Applied {
                #[allow(dead_code)]
                decision: Decision,
            },
            #[serde(other)]
            Other,
        }
        for outcome in [
            ApprovalOutcome::Chosen {
                choice: 1,
                by: "phone".into(),
            },
            ApprovalOutcome::ChosenUnconfirmed {
                choice: 1,
                by: "phone".into(),
            },
        ] {
            let json = serde_json::to_string(&outcome).unwrap();
            assert!(matches!(
                serde_json::from_str::<Older>(&json).unwrap(),
                Older::Other
            ));
        }
        let approval: Approval = serde_json::from_value(serde_json::json!({
            "approval_id": "a1", "terminal_id": "t", "agent_label": "a", "workspace_label": "w",
            "snippet": "", "tool": null, "options": [], "nonce": "A".repeat(43),
            "created_at_ms": 1, "expires_at_ms": 2,
        }))
        .unwrap();
        assert!(
            approval.choices.is_empty(),
            "an older collied sends no choices"
        );
        assert!(
            !approval.accepts_input && !approval.has_text_field && !approval.supports_note,
            "an older collied takes no input the phone can count on"
        );
    }

    #[test]
    fn terminal_methods_are_their_own_class() {
        let op = "AAAAAAAAAAAAAAAAAAAAAA";
        let nonce = "A".repeat(43);
        for (method, params) in [
            (
                "terminal.challenge",
                r#"{"terminal_id":"term_1"}"#.to_owned(),
            ),
            (
                "terminal.grant",
                format!(
                    r#"{{"terminal_id":"term_1","challenge":"{nonce}","signature":"MEUCIQDxyzAB"}}"#
                ),
            ),
            (
                "terminal.watch",
                r#"{"terminal_id":"term_1","lines":300}"#.to_owned(),
            ),
            (
                "terminal.run",
                format!(r#"{{"op_id":"{op}","terminal_id":"term_1","text":"git pull"}}"#),
            ),
            (
                "terminal.send_keys",
                format!(r#"{{"op_id":"{op}","terminal_id":"term_1","keys":["ctrl+c","enter"]}}"#),
            ),
            ("terminal.lock", "{}".to_owned()),
        ] {
            let frame = parse(&format!(
                r#"{{"id":1,"method":"{method}","params":{params}}}"#
            ))
            .unwrap();
            assert_eq!(frame.request.class(), MethodClass::Terminal, "{method}");
            assert_eq!(frame.request.method(), method);
            let json = serde_json::to_string(&frame).unwrap();
            assert_eq!(parse(&json).unwrap(), frame, "{method}");
        }
        let watch =
            parse(r#"{"id":1,"method":"terminal.watch","params":{"terminal_id":"t"}}"#).unwrap();
        let Request::TerminalWatch(p) = watch.request else {
            panic!("not a terminal watch");
        };
        assert_eq!(p.lines(), limits::DEFAULT_WATCH_LINES);
        let null = r#"{"id":1,"method":"terminal.watch","params":{"terminal_id":null}}"#;
        assert_eq!(parse(null).unwrap_err().code, ErrorCode::InvalidParams);
    }

    #[test]
    fn terminal_input_is_one_line_and_listed_keys() {
        let run = |text: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"terminal.run","params":{{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","terminal_id":"t","text":{}}}}}"#,
                serde_json::to_string(text).unwrap()
            ))
        };
        assert!(run("claude --resume 1234 && git pull").is_ok());
        for bad in [
            "a\nb",
            "a\tb",
            "a\u{1b}[201~",
            "  ",
            "a\rb",
            "echo ok \u{202E}fr- mr",
            "rm\u{200B} -rf",
        ] {
            assert_eq!(
                run(bad).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{bad:?}"
            );
        }
        let keys = |keys: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"terminal.send_keys","params":{{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","terminal_id":"t","keys":{keys}}}}}"#
            ))
        };
        assert!(keys(r#"["ctrl+c"]"#).is_ok());
        for bad in [
            r#"["ctrl+d"]"#.to_owned(),
            r#"["ctrl+z"]"#.to_owned(),
            "[]".to_owned(),
            serde_json::to_string(&vec!["enter"; 17]).unwrap(),
        ] {
            assert_eq!(
                keys(&bad).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{bad}"
            );
        }
    }

    #[test]
    fn pairing_carries_an_optional_terminal_key() {
        let key = format!("{}Q", "M".repeat(121));
        let base = r#"{"id":2,"method":"pair.complete","params":{"pairing_code":"Zm9vYmFyYmF6cXV4cXV1dQ","device_label":"iPhone""#;
        let Request::PairComplete(p) = parse(&format!("{base}}}}}")).unwrap().request else {
            panic!("not a pairing");
        };
        assert_eq!(p.terminal_key, None);
        assert!(!serde_json::to_string(&p).unwrap().contains("terminal_key"));
        let Request::PairComplete(p) = parse(&format!(r#"{base},"terminal_key":"{key}"}}}}"#))
            .unwrap()
            .request
        else {
            panic!("not a pairing");
        };
        assert_eq!(p.terminal_key.unwrap().as_str(), key);
        let short = format!(r#"{base},"terminal_key":"{}"}}}}"#, &key[1..]);
        assert_eq!(parse(&short).unwrap_err().code, ErrorCode::InvalidParams);
    }

    #[test]
    fn flock_terminals_are_additive() {
        let older = r#"{"seq":1,"machine":{"name":"m","node_id":"n","herdr_session":"default"},"workspaces":[],"agents":[],"approvals":[]}"#;
        let f: Flock = serde_json::from_str(older).unwrap();
        assert!(f.terminals.is_empty() && !f.terminals_enabled);
        let json = serde_json::to_string(&f).unwrap();
        assert!(
            json.ends_with(r#""approvals":[],"terminals":[]}"#),
            "{json}"
        );
        let on = Flock {
            terminals_enabled: true,
            terminals: vec![Terminal {
                terminal_id: TerminalId::new("term_1").unwrap(),
                workspace_id: WorkspaceId::new("w1").unwrap(),
                label: None,
                cwd: Some("/src".into()),
            }],
            ..f
        };
        let json = serde_json::to_string(&on).unwrap();
        assert!(json.contains(r#""terminals_enabled":true"#));
        assert_eq!(serde_json::from_str::<Flock>(&json).unwrap(), on);
        let granted = ServerFrame::Result {
            id: 8,
            result: Response::TerminalGranted {
                terminal_id: TerminalId::new("term_1").unwrap(),
                ttl_ms: 300_000,
            },
        };
        assert_eq!(
            serde_json::to_string(&granted).unwrap(),
            r#"{"kind":"result","id":8,"result":{"type":"terminal_granted","terminal_id":"term_1","ttl_ms":300000}}"#
        );
    }

    #[test]
    fn grant_message_vector() {
        let message = terminal_grant_message(
            "nMAC",
            &TerminalId::new("term_1").unwrap(),
            &Nonce::new("A".repeat(43)).unwrap(),
        );
        let mut want = Vec::new();
        want.extend_from_slice(b"\0\0\0\x18collie terminal grant v1");
        want.extend_from_slice(b"\0\0\0\x04nMAC");
        want.extend_from_slice(b"\0\0\0\x06term_1");
        want.extend_from_slice(b"\0\0\0\x2b");
        want.extend_from_slice("A".repeat(43).as_bytes());
        assert_eq!(message, want);
        let other = terminal_grant_message(
            "nMACt",
            &TerminalId::new("erm_1").unwrap(),
            &Nonce::new("A".repeat(43)).unwrap(),
        );
        assert_ne!(message, other, "length prefixes keep the parts apart");
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

    fn read_with(ansi: &str, wraps: &[u32], splits: &[u32]) -> TerminalRead {
        TerminalRead {
            terminal_id: TerminalId::new("t1").unwrap(),
            source: ReadSource::Recent,
            ansi: ansi.to_owned(),
            truncated: false,
            wraps: wraps.to_vec(),
            splits: splits.to_vec(),
            copilot_scrollbar: false,
        }
    }

    #[test]
    fn reflowed_joins_the_listed_rows() {
        let ansi = "\u{1b}[1m\u{1b}[0m\u{1b}[3mNext is \u{1b}[0m\r\n  \u{1b}[0m\u{1b}[3mwatching CI.\u{1b}[0m\r\n  https://exa\r\n  mple.com\r\n\r\n  end";
        assert_eq!(read_with(ansi, &[], &[]).reflowed(), None);
        assert_eq!(
            read_with(ansi, &[1], &[3]).reflowed().unwrap(),
            "\u{1b}[1m\u{1b}[0m\u{1b}[3mNext is\u{1b}[0m \u{1b}[0m\u{1b}[3mwatching CI.\u{1b}[0m\r\n  https://example.com\r\n\r\n  end"
        );
        assert_eq!(
            read_with("a \nb\nc", &[2, 0, 1, 1, 9], &[u32::MAX])
                .reflowed()
                .unwrap(),
            "a b c"
        );
    }

    #[test]
    fn copilot_scrollbar_is_removed_only_for_copilot_reflow() {
        let ansi = " ● The quick brown fox jumps over the  \u{1b}[0m\u{1b}[38;2;145;152;161m┃\u{1b}[0m\r\n   lazy dog keeps running.┃";
        let read = read_with(ansi, &[1], &[]);
        assert!(read.reflowed().unwrap().contains('┃'));
        let copilot = TerminalRead {
            copilot_scrollbar: true,
            ..read
        };
        assert_eq!(
            copilot.reflowed().as_deref(),
            Some(" ● The quick brown fox jumps over the lazy dog keeps running.")
        );
        assert_eq!(copilot.ansi, ansi);
    }

    #[test]
    fn wraps_are_additive() {
        let older = r#"{"terminal_id":"t1","source":"recent","ansi":"a","truncated":false}"#;
        let r: TerminalRead = serde_json::from_str(older).unwrap();
        assert_eq!(r, read_with("a", &[], &[]));
        assert_eq!(serde_json::to_string(&r).unwrap(), older);
        let with = read_with("a\nb\nc", &[1], &[2]);
        let json = serde_json::to_string(&with).unwrap();
        assert!(json.ends_with(r#""wraps":[1],"splits":[2]}"#), "{json}");
        assert_eq!(serde_json::from_str::<TerminalRead>(&json).unwrap(), with);
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
