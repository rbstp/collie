use std::borrow::Cow;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::*;
use crate::limits;
use crate::output::OutputPatch;

pub type RequestId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum GitSection {
    Committed,
    Staged,
    Unstaged,
    Untracked,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum GitScope {
    #[default]
    Uncommitted,
    Branch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AgentChangesParams {
    pub terminal_id: TerminalId,
    pub root: Option<Cwd>,
    #[serde(default)]
    pub scope: GitScope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AgentDiffParams {
    pub terminal_id: TerminalId,
    pub root: Cwd,
    pub path: String,
    pub section: GitSection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct GitChanges {
    pub root: Option<Cwd>,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub files: Vec<GitFile>,
    pub truncated: bool,
    pub worktrees: Vec<GitWorktree>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct GitWorktree {
    pub root: Cwd,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct GitFile {
    pub path: String,
    pub old_path: Option<String>,
    pub section: GitSection,
    pub status: String,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct GitDiff {
    pub patch: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct HistoryParams {
    pub terminal_id: TerminalId,
    pub before: Option<String>,
    pub after: Option<String>,
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub latest: bool,
}

impl HistoryParams {
    pub fn is_valid(&self) -> bool {
        !(self.before.is_some() && self.after.is_some())
            && !(self.latest
                && (self.before.is_some() || self.after.is_some() || self.query.is_some()))
            && self
                .before
                .iter()
                .chain(self.after.iter())
                .all(|s| s.len() <= 256)
            && self.query.as_ref().is_none_or(|s| s.len() <= 256)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct HistoryMessage {
    pub id: String,
    pub role: String,
    pub text: String,
    pub tool: Option<String>,
    pub call_id: Option<String>,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<MessageOrigin>,
}

/// A user message collied typed itself, matched exactly against the text it remembers typing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct MessageOrigin {
    /// `pr_watch`: a pull request watch woke the agent.
    pub kind: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct HistoryPage {
    pub session: Option<String>,
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub reset: bool,
    pub messages: Vec<HistoryMessage>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub more: bool,
    pub omitted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub enum CostRange {
    #[serde(rename = "today")]
    Today,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
}

impl CostRange {
    pub fn days(self) -> u32 {
        match self {
            Self::Today => 1,
            Self::Week => 7,
            Self::Month => 30,
        }
    }
}

/// `utc_offset_minutes` is the phone's, so days end at the phone's midnight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct UsageCostsParams {
    pub range: CostRange,
    #[cfg_attr(feature = "schema", schemars(range(min = -1080, max = 1080)))]
    pub utc_offset_minutes: i16,
}

impl UsageCostsParams {
    pub fn is_valid(&self) -> bool {
        (-1080..=1080).contains(&self.utc_offset_minutes)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct CostTokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

/// Estimated at API list prices from transcripts; `None` when the model has no known price.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ModelCost {
    pub agent: String,
    pub model: String,
    pub tokens: CostTokens,
    pub cost_usd: Option<f64>,
    pub cache_savings_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct DayCost {
    /// `YYYY-MM-DD` at the requested offset.
    pub date: String,
    pub cost_usd: f64,
}

/// Totals cover every model, including those beyond the listed ones and those without a price.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct UsageCosts {
    /// A scan is still reading transcripts; the figures so far are partial.
    pub scanning: bool,
    pub tokens: CostTokens,
    pub cost_usd: f64,
    pub cache_savings_usd: f64,
    pub days: Vec<DayCost>,
    pub models: Vec<ModelCost>,
}

/// `path` is absolute or relative to the agent's working folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct FileOpenParams {
    pub terminal_id: TerminalId,
    #[cfg_attr(feature = "schema", schemars(length(min = 1, max = limits::MAX_CWD_BYTES)))]
    pub path: String,
}

impl FileOpenParams {
    pub fn is_valid(&self) -> bool {
        (1..=limits::MAX_CWD_BYTES).contains(&self.path.len())
            && !self.path.chars().any(char::is_control)
    }
}

/// `offset` is where the chunk starts; collied answers at most 32 KiB from there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct FileChunkParams {
    pub download_id: DownloadId,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct FileCloseParams {
    pub download_id: DownloadId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Image,
    Pdf,
    Text,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ClientFrame {
    pub id: RequestId,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
    #[serde(rename = "agent.history")]
    AgentHistory(HistoryParams),
    #[serde(rename = "pane.read")]
    PaneRead(ReadParams),
    #[serde(rename = "agent.watch")]
    AgentWatch(AgentWatchParams),
    #[serde(rename = "task.options")]
    TaskOptions(Empty),
    #[serde(rename = "agent.changes")]
    AgentChanges(AgentChangesParams),
    #[serde(rename = "agent.diff")]
    AgentDiff(AgentDiffParams),
    #[serde(rename = "agent.draft")]
    AgentDraft(AgentTarget),
    #[serde(rename = "usage.costs")]
    UsageCosts(UsageCostsParams),
    #[serde(rename = "file.open")]
    FileOpen(FileOpenParams),
    #[serde(rename = "file.chunk")]
    FileChunk(FileChunkParams),
    #[serde(rename = "file.close")]
    FileClose(FileCloseParams),

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
    #[serde(rename = "agent.watch_pull_request")]
    AgentWatchPullRequest(AgentWatchPullRequestParams),
    #[serde(rename = "task.new")]
    TaskNew(TaskNewParams),
    #[serde(rename = "task.folders")]
    TaskFolders(TaskFoldersParams),
    #[serde(rename = "task.worktrees")]
    TaskWorktrees(TaskWorktreesParams),
    #[serde(rename = "task.sessions")]
    TaskSessions(TaskSessionsParams),
    #[serde(rename = "task.archive")]
    TaskArchive(TaskArchiveParams),
    #[serde(rename = "task.archive_on_merge")]
    TaskArchiveOnMerge(TaskArchiveOnMergeParams),
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
        "agent.history",
        "pane.read",
        "agent.watch",
        "task.options",
        "agent.changes",
        "agent.diff",
        "agent.draft",
        "usage.costs",
        "file.open",
        "file.chunk",
        "file.close",
        "agent.prompt",
        "agent.send_keys",
        "agent.type_text",
        "agent.focus",
        "agent.scroll_bottom",
        "agent.answer_notice",
        "agent.slash_draft",
        "agent.star",
        "agent.watch_pull_request",
        "task.new",
        "task.folders",
        "task.worktrees",
        "task.sessions",
        "task.archive",
        "task.archive_on_merge",
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
            Self::AgentHistory(_) => "agent.history",
            Self::PaneRead(_) => "pane.read",
            Self::AgentWatch(_) => "agent.watch",
            Self::TaskOptions(_) => "task.options",
            Self::AgentChanges(_) => "agent.changes",
            Self::AgentDiff(_) => "agent.diff",
            Self::AgentDraft(_) => "agent.draft",
            Self::UsageCosts(_) => "usage.costs",
            Self::FileOpen(_) => "file.open",
            Self::FileChunk(_) => "file.chunk",
            Self::FileClose(_) => "file.close",
            Self::AgentPrompt(_) => "agent.prompt",
            Self::AgentSendKeys(_) => "agent.send_keys",
            Self::AgentTypeText(_) => "agent.type_text",
            Self::AgentFocus(_) => "agent.focus",
            Self::AgentScrollBottom(_) => "agent.scroll_bottom",
            Self::AgentAnswerNotice(_) => "agent.answer_notice",
            Self::AgentSlashDraft(_) => "agent.slash_draft",
            Self::AgentStar(_) => "agent.star",
            Self::AgentWatchPullRequest(_) => "agent.watch_pull_request",
            Self::TaskNew(_) => "task.new",
            Self::TaskFolders(_) => "task.folders",
            Self::TaskWorktrees(_) => "task.worktrees",
            Self::TaskSessions(_) => "task.sessions",
            Self::TaskArchive(_) => "task.archive",
            Self::TaskArchiveOnMerge(_) => "task.archive_on_merge",
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
            | Self::AgentHistory(_)
            | Self::PaneRead(_)
            | Self::AgentWatch(_)
            | Self::TaskOptions(_)
            | Self::AgentChanges(_)
            | Self::AgentDiff(_)
            | Self::AgentDraft(_)
            | Self::UsageCosts(_)
            | Self::FileOpen(_)
            | Self::FileChunk(_)
            | Self::FileClose(_)
            | Self::ApprovalList(_) => MethodClass::Read,
            Self::AgentPrompt(_)
            | Self::AgentSendKeys(_)
            | Self::AgentTypeText(_)
            | Self::AgentFocus(_)
            | Self::AgentScrollBottom(_)
            | Self::AgentAnswerNotice(_)
            | Self::AgentSlashDraft(_)
            | Self::AgentStar(_)
            | Self::AgentWatchPullRequest(_)
            | Self::TaskNew(_)
            | Self::TaskFolders(_)
            | Self::TaskWorktrees(_)
            | Self::TaskSessions(_)
            | Self::TaskArchive(_)
            | Self::TaskArchiveOnMerge(_)
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct HelloParams {
    pub protocol_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_min_version: Option<u32>,
    #[serde(
        default = "crate::baseline_capabilities",
        skip_serializing_if = "crate::has_baseline_capabilities"
    )]
    pub capabilities: Vec<Label>,
    pub app_version: Label,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PairCompleteParams {
    pub pairing_code: PairingCode,
    pub device_label: Label,
    /// The phone's terminal key, recorded with the pairing: it signs terminal grants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_key: Option<TerminalKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReadSource {
    Visible,
    Recent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ReadParams {
    pub terminal_id: TerminalId,
    pub source: ReadSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(range(min = 1, max = limits::MAX_READ_LINES)))]
    pub lines: Option<u16>,
}

impl ReadParams {
    pub fn is_valid(&self) -> bool {
        self.lines
            .is_none_or(|n| (1..=limits::MAX_READ_LINES).contains(&n))
    }
}

/// One watched agent per connection; `None` stops `agent.output` events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AgentWatchParams {
    pub terminal_id: Option<TerminalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(range(min = 1, max = limits::MAX_READ_LINES)))]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AgentTarget {
    pub terminal_id: TerminalId,
}

/// A key Claude Code reads as an answer to a notice above its input box; never Enter or text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AgentSlashDraftParams {
    pub terminal_id: TerminalId,
    pub command: SlashCommand,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_draft: Option<DraftText>,
}

/// Stars are the machine's, shared by every paired phone. Unstarring needs no live pane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AgentStarParams {
    pub terminal_id: TerminalId,
    pub starred: bool,
}

/// Watching needs a Claude Code agent's linked pull request, open or a draft, and
/// `every_minutes` of 2, 5, 15 or 30; stopping takes no `every_minutes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AgentWatchPullRequestParams {
    pub terminal_id: TerminalId,
    pub watching: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_minutes: Option<u32>,
}

/// `signature` is over [`terminal_grant_message`] for the challenge this session was given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TerminalGrantParams {
    pub terminal_id: TerminalId,
    pub challenge: Nonce,
    pub signature: Signature,
}

/// Shares the session's single watch with `agent.watch`; `agent.watch {terminal_id: null}`
/// stops either.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TerminalWatchParams {
    pub terminal_id: TerminalId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(range(min = 1, max = limits::MAX_READ_LINES)))]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AgentPromptParams {
    pub op_id: OpId,
    pub terminal_id: TerminalId,
    pub text: PromptText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_draft: Option<DraftText>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
    #[serde(rename = "backspace")]
    Backspace,
    #[serde(rename = "ctrl+c")]
    CtrlC,
    #[serde(rename = "ctrl+enter")]
    CtrlEnter,
    #[serde(rename = "ctrl+a")]
    CtrlA,
    #[serde(rename = "ctrl+b")]
    CtrlB,
    #[serde(rename = "ctrl+e")]
    CtrlE,
    #[serde(rename = "ctrl+k")]
    CtrlK,
    #[serde(rename = "ctrl+l")]
    CtrlL,
    #[serde(rename = "ctrl+u")]
    CtrlU,
    #[serde(rename = "ctrl+w")]
    CtrlW,
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
            Self::Backspace => "backspace",
            Self::CtrlC => "ctrl+c",
            Self::CtrlEnter => "ctrl+enter",
            Self::CtrlA => "ctrl+a",
            Self::CtrlB => "ctrl+b",
            Self::CtrlE => "ctrl+e",
            Self::CtrlK => "ctrl+k",
            Self::CtrlL => "ctrl+l",
            Self::CtrlU => "ctrl+u",
            Self::CtrlW => "ctrl+w",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AgentSendKeysParams {
    pub op_id: OpId,
    pub terminal_id: TerminalId,
    #[cfg_attr(feature = "schema", schemars(length(min = 1, max = limits::MAX_KEYS_PER_CALL)))]
    pub keys: Vec<Key>,
}

impl AgentSendKeysParams {
    pub fn is_valid(&self) -> bool {
        (1..=limits::MAX_KEYS_PER_CALL).contains(&self.keys.len())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AgentTypeTextParams {
    pub op_id: OpId,
    pub terminal_id: TerminalId,
    pub text: PromptText,
}

impl AgentTypeTextParams {
    pub fn is_valid(&self) -> bool {
        !self.text.as_str().chars().any(is_format)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TaskNewParams {
    pub op_id: OpId,
    pub cwd: Cwd,
    pub agent: AgentKind,
    /// Required unless `resume` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<PromptText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Label>,
    /// When set, collied creates this empty folder directly inside `cwd` and starts there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_folder: Option<FolderName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<TaskWorktree>,
    /// When set, collied clones this repository into a new folder directly inside `cwd`,
    /// named by `CloneSource::folder_name`, and starts there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clone: Option<CloneSource>,
    /// When set, collied starts `agent` resuming this session of `cwd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<SessionId>,
    /// Claude Code only: start it with ultracode on, checked on screen before the prompt.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ultracode: bool,
}

impl TaskNewParams {
    pub fn is_valid(&self) -> bool {
        (self.prompt.is_some() || self.resume.is_some())
            && !matches!(&self.worktree, Some(TaskWorktree::PullRequest { number })
                if !(1..=u64::from(u32::MAX)).contains(number))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskWorktree {
    Create {
        branch: String,
    },
    Open {
        path: Cwd,
    },
    /// collied checks out the head branch of this pull request of the repository.
    PullRequest {
        number: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TaskWorktreesParams {
    pub cwd: Cwd,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TaskSessionsParams {
    pub cwd: Cwd,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TaskArchiveParams {
    pub op_id: OpId,
    pub terminal_id: TerminalId,
    #[serde(default)]
    pub confirm: bool,
}

/// The phone's "Archive when the PR merges" setting, sent on every connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TaskArchiveOnMergeParams {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TaskFoldersParams {
    pub path: Cwd,
}

/// `confirm` must be `true`; anything else yields `confirm_required`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCloseParams {
    pub workspace_id: WorkspaceId,
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PaneCloseParams {
    pub terminal_id: TerminalId,
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ApnsEnvironment {
    Sandbox,
    Production,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RelayGrant {
    pub token: RelaySendToken,
    pub route_id: RelayRouteId,
    pub generation: u32,
    pub expires_at: u64,
}

impl RelayGrant {
    pub fn is_valid(&self, now: u64) -> bool {
        self.generation > 0
            && self.expires_at > now
            && self.expires_at <= now.saturating_add(30 * 86400)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PushMode {
    None,
    Direct,
    Relay,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PushRegisterParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_grant: Option<RelayGrant>,
    pub apns_token: PushToken,
    pub environment: ApnsEnvironment,
    pub notification_key: NotificationKey,
    /// No alert when an agent finishes a turn.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mute_done: bool,
}

/// A Live Activity's update token. collied pushes the followed terminal's status to it,
/// in the APNs environment of the device's own `push.register`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PushActivityTokenParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_grant: Option<RelayGrant>,
    pub activity_id: ActivityId,
    pub terminal_id: TerminalId,
    pub token: PushToken,
    /// The activity shows an approval's command and its Approve and Deny buttons. Without
    /// it collied keeps sending that device the approval alert.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shows_approvals: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_binding: Option<ActivityDisplayBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ActivityDisplayBinding {
    pub node_id: Label,
    pub binding_id: ActivityId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PushActivityEndParams {
    pub activity_id: ActivityId,
}

/// `sha256` covers the whole file; collied checks it and the size before keeping it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AttachmentBeginParams {
    pub op_id: OpId,
    pub name: AttachmentName,
    #[cfg_attr(feature = "schema", schemars(range(min = 1, max = limits::MAX_ATTACHMENT_BYTES)))]
    pub size: u64,
    pub sha256: Sha256Hex,
}

impl AttachmentBeginParams {
    pub fn is_valid(&self) -> bool {
        (1..=limits::MAX_ATTACHMENT_BYTES).contains(&self.size)
    }
}

/// Chunks arrive in order: `offset` is the number of bytes already received.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AttachmentChunkParams {
    pub upload_id: UploadId,
    pub offset: u64,
    pub data: ChunkData,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AttachmentCommitParams {
    pub op_id: OpId,
    pub upload_id: UploadId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AttachmentAbortParams {
    pub upload_id: UploadId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Hello(HelloResult),
    Paired {
        machine: MachineInfo,
    },
    Flock(Flock),
    Terminal(TerminalRead),
    TaskOptions(TaskOptions),
    GitChanges(GitChanges),
    GitDiff(GitDiff),
    History(HistoryPage),
    UsageCosts(UsageCosts),
    /// `sha256` covers the whole file as collied read it at open.
    FileOpened {
        download_id: DownloadId,
        name: String,
        size: u64,
        kind: FileKind,
        sha256: Sha256Hex,
    },
    FileChunk {
        data: ChunkData,
    },
    TaskWorktrees {
        source: Cwd,
        worktrees: Vec<TaskWorktreeInfo>,
    },
    /// The most recent sessions of `cwd`, newest first.
    TaskSessions {
        sessions: Vec<TaskSessionInfo>,
    },
    TaskStarted {
        workspace_id: WorkspaceId,
        terminal_id: TerminalId,
    },
    TaskArchived {
        message: String,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct HelloResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_mode: Option<PushMode>,
    pub protocol_version: u32,
    pub protocol_min_version: u32,
    pub protocol_max_version: u32,
    pub capabilities: Vec<Label>,
    pub collied_version: String,
    pub machine: MachineInfo,
    pub herdr_version: Option<String>,
    pub paired: bool,
}

/// `node_id` is the Tailscale stable node ID; the phone pins it at pairing time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct MachineInfo {
    pub name: String,
    pub node_id: String,
    pub herdr_session: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Workspace {
    pub workspace_id: WorkspaceId,
    pub label: String,
    pub number: u32,
    pub status: AgentStatus,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
    #[cfg_attr(feature = "schema", schemars(range(max = 100)))]
    pub context_left: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_origin: Option<MessageOrigin>,
    /// When the transcript last changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_ms: Option<u64>,
    /// Claude Code subagents running now, from their own transcripts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<Subagent>,
    /// The pull request of the agent's branch, as `gh` on the machine last read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_watch: Option<PrWatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct PullRequest {
    pub number: u64,
    pub state: PullState,
    pub checks: PullChecks,
    pub url: String,
    /// At most 10 names, each without control or format characters or quotes and at most
    /// 100 characters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failing_checks: Vec<String>,
    /// Reviews plus comments, never their text.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub remarks: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflicting: bool,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// The pull request collied watches for this terminal, whether or not it is still the
/// agent's linked one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct PrWatch {
    pub number: u64,
    pub url: String,
    pub every_minutes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PullState {
    Open,
    Draft,
    Merged,
    Closed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PullChecks {
    Passing,
    Failing,
    Pending,
    None,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Subagent {
    pub agent_type: String,
    /// The description it was started with, else its type.
    #[serde(default)]
    pub title: String,
    pub activity: String,
    pub started_at_ms: u64,
    pub tokens: u64,
    /// Claude Code's panel shows the tokens with ↓ once the subagent has called a tool, ↑ before.
    pub called_tools: bool,
}

/// From Claude Code's status line input, as `collied statusline` last recorded it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct PlanUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<UsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<UsageWindow>,
    pub recorded_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot: Option<CopilotUsage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct CodexUsage {
    pub used: u64,
    pub limit: u64,
    pub resets_at_ms: u64,
    pub recorded_ms: u64,
}

/// Copilot's monthly premium quota. `used` can pass `limit` when the seat permits overage;
/// both are 0 when `unlimited`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct CopilotUsage {
    pub used: u64,
    pub limit: u64,
    pub unlimited: bool,
    pub token_based_billing: bool,
    pub resets_at_ms: u64,
    pub recorded_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct UsageWindow {
    #[cfg_attr(feature = "schema", schemars(range(max = 100)))]
    pub used_percent: u8,
    pub resets_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Terminal {
    pub terminal_id: TerminalId,
    pub workspace_id: WorkspaceId,
    pub label: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
    /// there are none. A space joining shaded rows keeps their shade, and for Copilot a sent
    /// prompt's timestamp follows the joined prompt.
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
        let mut clock = String::new();
        for (i, row) in rows.iter().enumerate() {
            let joined = sep.get(i + 1).is_some_and(Option::is_some);
            let mut row = if self.copilot_scrollbar && (sep[i].is_some() || joined) {
                Cow::Owned(trim_copilot_line_end(row))
            } else {
                Cow::Borrowed(*row)
            };
            let mut moved = None;
            if self.copilot_scrollbar
                && sep[i].is_none()
                && joined
                && let Some((text, stamp)) = split_copilot_clock(&row)
            {
                moved = Some(stamp);
                row = Cow::Owned(text);
            }
            match sep[i] {
                Some(s) if i > 0 => {
                    let line = out.rfind('\n').map_or(0, |n| n + 1);
                    let mut kept = trim_line_end(&out[line..]);
                    let bare = before_sgr(&kept).len();
                    let at = if kept[bare..].contains("\u{1b}[48;") {
                        bare
                    } else {
                        kept.len()
                    };
                    kept.insert_str(at, s);
                    out.truncate(line);
                    out.push_str(&kept);
                    out.push_str(&trim_row_start(&row));
                }
                _ => {
                    if i > 0 {
                        out.push_str(&std::mem::take(&mut clock));
                        if rows[i - 1].ends_with('\r') && !out.ends_with('\r') {
                            out.push('\r');
                        }
                        out.push('\n');
                    }
                    out.push_str(&row);
                }
            }
            if let Some(stamp) = moved {
                clock = stamp;
            }
        }
        out.push_str(&clock);
        Some(out)
    }
}

/// A sent Copilot prompt's first row, `❯ ` and its text, without the padding and `HH:MM`
/// timestamp styled apart at its end, and those moved to follow the joined prompt: the
/// timestamp after one space in the padding's style.
fn split_copilot_clock(row: &str) -> Option<(String, String)> {
    let stamped = before_sgr(row);
    let (rest, time) = stamped.split_at_checked(stamped.len().checked_sub(5)?)?;
    let digits = time.as_bytes();
    if digits[2] != b':' || ![0, 1, 3, 4].iter().all(|&i| digits[i].is_ascii_digit()) {
        return None;
    }
    let padded = before_sgr(rest);
    let trimmed = padded.trim_end_matches(' ');
    let text = before_sgr(trimmed);
    if padded.len() == rest.len() || trimmed.len() == padded.len() {
        return None;
    }
    let mut chars = text.chars();
    let first = loop {
        match chars.next()? {
            ' ' => {}
            '\u{1b}' => {
                chars.find(|&c| c == 'm');
            }
            c => break c,
        }
    };
    let (padding, style, after) = (
        &trimmed[text.len()..],
        &rest[padded.len()..],
        &row[stamped.len()..],
    );
    (first == '❯').then(|| (text.to_owned(), format!("{padding} {style}{time}{after}")))
}

fn before_sgr(mut s: &str) -> &str {
    while let Some(start) = sgr_suffix(s) {
        s = &s[..start];
    }
    s
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct TaskOptions {
    pub agents: Vec<AgentKind>,
    pub default_agent: AgentKind,
    pub recent_cwds: Vec<Cwd>,
    pub roots: Vec<Cwd>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct TaskWorktreeInfo {
    pub path: Cwd,
    pub branch: Option<String>,
    pub open: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct TaskSessionInfo {
    pub agent: AgentKind,
    pub session_id: SessionId,
    /// The first line of the session's first prompt, at most 160 characters; empty when it
    /// is not in the first 256 KiB of the transcript.
    pub first_prompt: String,
    pub modified_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct PendingTool {
    pub name: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Approval {
    pub approval_id: ApprovalId,
    pub terminal_id: TerminalId,
    pub agent_label: String,
    pub workspace_label: String,
    pub snippet: String,
    pub tool: Option<PendingTool>,
    /// Why Claude Code asked, as its dialog says: one line of at most 200 characters with
    /// no control, bidi or invisible format character. Absent when it gives none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
    InputUnfocused,
    Internal,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    /// With `draft_changed`: the text now in the Mac's input box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
    /// With `unsupported_protocol`: collied's protocol version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u32>,
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
                protocol_version: None,
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
        Request::Hello(p) => p.is_valid(),
        Request::AgentRead(p) | Request::PaneRead(p) => p.is_valid(),
        Request::AgentHistory(p) => p.is_valid(),
        Request::UsageCosts(p) => p.is_valid(),
        Request::FileOpen(p) => p.is_valid(),
        Request::AgentSendKeys(p) => p.is_valid(),
        Request::AgentTypeText(p) => p.is_valid(),
        Request::TerminalRun(p) => p.is_valid(),
        Request::TerminalSendKeys(p) => p.is_valid(),
        Request::ApprovalDecide(p) => p.is_valid(),
        Request::AttachmentBegin(p) => p.is_valid(),
        Request::TaskNew(p) => p.is_valid(),
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
    fn history_is_bounded_and_read_only() {
        let request = |params: serde_json::Value| {
            serde_json::to_vec(&serde_json::json!({
                "id": 1, "method": "agent.history", "params": params,
            }))
            .unwrap()
        };
        let valid = parse_client_frame(&request(serde_json::json!({"terminal_id":"t1"}))).unwrap();
        assert_eq!(valid.request.class(), MethodClass::Read);
        parse_client_frame(&request(
            serde_json::json!({"terminal_id":"t1","latest":true}),
        ))
        .unwrap();
        for extra in [
            serde_json::json!({"before":"a","after":"b"}),
            serde_json::json!({"latest":true,"after":"b"}),
            serde_json::json!({"latest":true,"query":"x"}),
            serde_json::json!({"query":"x".repeat(257)}),
            serde_json::json!({"before":"x".repeat(257)}),
            serde_json::json!({"path":"/private/file"}),
        ] {
            let mut params = serde_json::json!({"terminal_id":"t1"});
            params
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(parse_client_frame(&request(params)).is_err());
        }
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
        assert_eq!(crate::PROTOCOL_VERSION, 29);
    }

    #[test]
    fn file_reads_are_bounded() {
        let open = |params: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"file.open","params":{params}}}"#
            ))
        };
        let frame = open(r#"{"terminal_id":"term_1","path":"docs/a b.png"}"#).unwrap();
        assert_eq!(frame.request.class(), MethodClass::Read);
        let long = format!(r#"{{"terminal_id":"t","path":"{}"}}"#, "a".repeat(1025));
        for params in [
            r#"{"terminal_id":"t","path":""}"#,
            r#"{"terminal_id":"t","path":"a\u0007b"}"#,
            r#"{"terminal_id":"t"}"#,
            r#"{"terminal_id":"t","path":"a","offset":0}"#,
            &long,
        ] {
            assert_eq!(
                open(params).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{params}"
            );
        }
        let chunk = parse(
            r#"{"id":2,"method":"file.chunk","params":{"download_id":"0123456789abcdef0123456789abcdef","offset":32768}}"#,
        )
        .unwrap();
        assert_eq!(chunk.request.class(), MethodClass::Read);
        let reply = ServerFrame::Result {
            id: 2,
            result: Response::FileChunk {
                data: ChunkData::new(format!("{}=", "A".repeat(43691))).unwrap(),
            },
        };
        let json = serde_json::to_string(&reply).unwrap();
        assert!(json.len() < limits::MAX_FRAME_BYTES, "{}", json.len());
        let unknown: Response = serde_json::from_value(serde_json::json!({
            "type": "file_opened", "download_id": "0123456789abcdef0123456789abcdef",
            "name": "a.bin", "size": 1, "kind": "video", "sha256": "0".repeat(64)
        }))
        .unwrap();
        assert!(matches!(
            unknown,
            Response::FileOpened {
                kind: FileKind::Other,
                ..
            }
        ));
    }

    #[test]
    fn usage_costs_are_a_bounded_read() {
        let costs = |params: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"usage.costs","params":{params}}}"#
            ))
        };
        let frame = costs(r#"{"range":"30d","utc_offset_minutes":330}"#).unwrap();
        assert_eq!(frame.request.class(), MethodClass::Read);
        assert_eq!(
            frame.request,
            Request::UsageCosts(UsageCostsParams {
                range: CostRange::Month,
                utc_offset_minutes: 330
            })
        );
        for params in [
            r#"{"range":"today","utc_offset_minutes":1081}"#,
            r#"{"range":"1y","utc_offset_minutes":0}"#,
            r#"{"range":"today"}"#,
            r#"{"range":"today","utc_offset_minutes":0,"path":"/etc"}"#,
        ] {
            assert_eq!(
                costs(params).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{params}"
            );
        }
    }

    #[test]
    fn agent_changes_root_is_optional() {
        let changes = |params: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"agent.changes","params":{params}}}"#
            ))
        };
        let Request::AgentChanges(plain) = changes(r#"{"terminal_id":"t"}"#).unwrap().request
        else {
            panic!("not agent.changes");
        };
        assert_eq!(plain.root, None);
        assert_eq!(plain.scope, GitScope::Uncommitted);
        let frame = changes(r#"{"terminal_id":"t","root":"/a","scope":"branch"}"#).unwrap();
        assert_eq!(frame.request.class(), MethodClass::Read);
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(parse(&json).unwrap(), frame);
        for params in [
            r#"{"terminal_id":"t","root":"a"}"#,
            r#"{"terminal_id":"t","path":"/a"}"#,
            r#"{"terminal_id":"t","scope":"main"}"#,
        ] {
            assert_eq!(
                changes(params).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{params}"
            );
        }
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
        let frame = task(r#","clone":"owner/app""#).unwrap();
        let Request::TaskNew(p) = &frame.request else {
            panic!("not a task");
        };
        assert_eq!(p.clone.as_ref().unwrap().folder_name().as_str(), "app");
        assert_eq!(
            parse(&serde_json::to_string(&frame).unwrap()).unwrap(),
            frame
        );
        for bad in [
            r#","new_folder":"../x""#,
            r#","new_folder":".x""#,
            r#","new_folder":"a/b""#,
            r#","clone":"file:///tmp/x""#,
            r#","clone":"-u/x""#,
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
            r#"{"mode":"pull_request","number":42}"#,
            r#"{"mode":"pull_request","number":4294967295}"#,
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
            r#"{"mode":"pull_request","number":0}"#,
            r#"{"mode":"pull_request","number":-1}"#,
            r#"{"mode":"pull_request","number":"42"}"#,
            r#"{"mode":"pull_request","number":4294967296}"#,
            r#"{"mode":"pull_request","number":42,"branch":"main"}"#,
            r#"{"mode":"pull_request"}"#,
        ] {
            assert_eq!(task(choice).unwrap_err().code, ErrorCode::InvalidParams);
        }
    }

    #[test]
    fn sessions_and_resume_need_a_session_id_and_only_resume_skips_the_prompt() {
        let list = parse(r#"{"id":1,"method":"task.sessions","params":{"cwd":"/repo"}}"#).unwrap();
        assert_eq!(list.request.class(), MethodClass::Drive);
        assert_eq!(list.request.method(), "task.sessions");
        for params in [r#"{"cwd":"repo"}"#, r#"{"cwd":"/repo","agent":"claude"}"#] {
            let bad = format!(r#"{{"id":1,"method":"task.sessions","params":{params}}}"#);
            assert_eq!(parse(&bad).unwrap_err().code, ErrorCode::InvalidParams);
        }
        let task = |extra: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"task.new","params":{{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","cwd":"/repo","agent":"claude"{extra}}}}}"#
            ))
        };
        for extra in [
            r#","resume":"3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d""#,
            r#","resume":"3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d","prompt":"go on""#,
            r#","prompt":"go""#,
        ] {
            let frame = task(extra).unwrap();
            assert_eq!(
                parse(&serde_json::to_string(&frame).unwrap()).unwrap(),
                frame
            );
        }
        for bad in [
            "",
            r#","prompt":"""#,
            r#","resume":"3F1B2C4D-5E6F-4A7B-8C9D-0E1F2A3B4C5D""#,
            r#","resume":"--fork-session""#,
            r#","resume":"","prompt":"go""#,
        ] {
            assert_eq!(
                task(bad).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{bad}"
            );
        }
    }

    #[test]
    fn archive_requires_explicit_confirmation_and_replays_as_one_request() {
        let frame = parse(r#"{"id":1,"method":"task.archive","params":{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","terminal_id":"term","confirm":true}}"#).unwrap();
        assert_eq!(frame.request.class(), MethodClass::Drive);
        assert_eq!(frame.request.method(), "task.archive");
        assert_eq!(
            parse(&serde_json::to_string(&frame).unwrap()).unwrap(),
            frame
        );
        for params in [
            r#"{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","terminal_id":"term","confirm":true,"force":false}"#,
            r#"{"terminal_id":"term","confirm":true}"#,
        ] {
            let bad = format!(r#"{{"id":1,"method":"task.archive","params":{params}}}"#);
            assert_eq!(parse(&bad).unwrap_err().code, ErrorCode::InvalidParams);
        }
        let omitted = parse(r#"{"id":1,"method":"task.archive","params":{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","terminal_id":"term"}}"#).unwrap();
        let Request::TaskArchive(params) = omitted.request else {
            panic!("not archive")
        };
        assert!(!params.confirm);
    }

    #[test]
    fn archive_on_merge_takes_one_flag() {
        let json = r#"{"id":1,"method":"task.archive_on_merge","params":{"enabled":true}}"#;
        let frame = parse(json).unwrap();
        assert_eq!(
            frame.request,
            Request::TaskArchiveOnMerge(TaskArchiveOnMergeParams { enabled: true })
        );
        assert_eq!(frame.request.class(), MethodClass::Drive);
        assert_eq!(serde_json::to_string(&frame).unwrap(), json);
        for params in ["{}", r#"{"enabled":"yes"}"#, r#"{"enabled":true,"x":1}"#] {
            let bad = format!(r#"{{"id":1,"method":"task.archive_on_merge","params":{params}}}"#);
            assert_eq!(
                parse(&bad).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{params}"
            );
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
            subagents: vec![Subagent {
                agent_type: "general-purpose".into(),
                title: "Check the pairing code".into(),
                activity: "Reading lib.rs".into(),
                started_at_ms: 5,
                tokens: 500,
                called_tools: true,
            }],
            ..a
        };
        let json = serde_json::to_string(&full).unwrap();
        assert!(json.ends_with(
            r#""last_line":"Done.","context_left":100,"last_prompt":"go","last_activity_ms":7,"subagents":[{"agent_type":"general-purpose","title":"Check the pairing code","activity":"Reading lib.rs","started_at_ms":5,"tokens":500,"called_tools":true}]}"#
        ));
        let linked = Agent {
            pull_request: Some(PullRequest {
                number: 12,
                state: PullState::Draft,
                checks: PullChecks::None,
                url: "https://github.com/o/r/pull/12".into(),
                failing_checks: Vec::new(),
                remarks: 0,
                conflicting: false,
            }),
            ..full.clone()
        };
        let json = serde_json::to_string(&linked).unwrap();
        assert!(json.ends_with(
            r#""pull_request":{"number":12,"state":"draft","checks":"none","url":"https://github.com/o/r/pull/12"}}"#
        ));
        let watched = Agent {
            pr_watch: Some(PrWatch {
                number: 11,
                url: "https://github.com/o/r/pull/11".into(),
                every_minutes: 15,
            }),
            ..linked.clone()
        };
        assert!(serde_json::to_string(&watched).unwrap().ends_with(
            r#""pr_watch":{"number":11,"url":"https://github.com/o/r/pull/11","every_minutes":15}}"#
        ));
        let newer = json.replace(r#""draft","checks":"none""#, r#""queued","checks":"stale""#);
        let parsed = serde_json::from_str::<Agent>(&newer)
            .unwrap()
            .pull_request
            .unwrap();
        assert_eq!(
            (parsed.state, parsed.checks),
            (PullState::Unknown, PullChecks::Unknown)
        );
        let busy = PullRequest {
            failing_checks: vec!["lint".into()],
            remarks: 3,
            conflicting: true,
            ..linked.pull_request.clone().unwrap()
        };
        let json = serde_json::to_string(&busy).unwrap();
        assert!(json.ends_with(r#""failing_checks":["lint"],"remarks":3,"conflicting":true}"#));
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
            copilot: None,
        };
        let with = Flock {
            plan_usage: Some(usage.clone()),
            ..f
        };
        let json = serde_json::to_string(&with).unwrap();
        assert!(json.ends_with(
            r#""plan_usage":{"five_hour":{"used_percent":24,"resets_at_ms":1738425600000},"recorded_ms":1738420000000}}"#
        ));
    }

    #[test]
    fn watch_pull_request_takes_a_terminal_and_a_flag() {
        let json = r#"{"id":2,"method":"agent.watch_pull_request","params":{"terminal_id":"term_1","watching":true,"every_minutes":15}}"#;
        let frame = parse(json).unwrap();
        assert_eq!(
            frame.request,
            Request::AgentWatchPullRequest(AgentWatchPullRequestParams {
                terminal_id: TerminalId::new("term_1").unwrap(),
                watching: true,
                every_minutes: Some(15),
            })
        );
        assert_eq!(frame.request.class(), MethodClass::Drive);
        assert_eq!(serde_json::to_string(&frame).unwrap(), json);
        for params in [
            r#"{"terminal_id":"t"}"#,
            r#"{"terminal_id":"t","watching":true,"url":"https://example.com"}"#,
        ] {
            let json =
                format!(r#"{{"id":2,"method":"agent.watch_pull_request","params":{params}}}"#);
            assert_eq!(
                parse(&json).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{params}"
            );
        }
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
    }

    #[test]
    fn draft_changed_carries_the_current_draft() {
        let frame = ServerFrame::Error {
            id: Some(4),
            error: ErrorBody {
                code: ErrorCode::DraftChanged,
                message: "m".into(),
                draft: Some("typed on the Mac".into()),
                protocol_version: None,
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
    fn typed_text_accepts_lines_but_rejects_unsafe_characters() {
        let typed = |text: &str| {
            parse(&format!(
                r#"{{"id":1,"method":"agent.type_text","params":{{"op_id":"AAAAAAAAAAAAAAAAAAAAAA","terminal_id":"t","text":{}}}}}"#,
                serde_json::to_string(text).unwrap()
            ))
        };
        assert!(typed("oui\u{202F}! c’est partagé 👍🏽").is_ok());
        assert!(typed("a\nb").is_ok());
        assert!(typed("a\tb").is_ok());
        let ok = typed("use Redis, it is shared").unwrap();
        assert_eq!(ok.request.class(), MethodClass::Drive);
        assert_eq!(ok.request.method(), "agent.type_text");
        for bad in [
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
        assert_eq!(approval.reason, None);
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
        assert!(keys(r#"["backspace","ctrl+a","ctrl+e","ctrl+u","ctrl+w"]"#).is_ok());
        for bad in [
            r#"["ctrl+d"]"#.to_owned(),
            r#"["ctrl+z"]"#.to_owned(),
            r#"["ctrl+r"]"#.to_owned(),
            r#"["ctrl+o"]"#.to_owned(),
            r#"["ctrl+t"]"#.to_owned(),
            r#"["ctrl+j"]"#.to_owned(),
            r#"["ctrl+m"]"#.to_owned(),
            r#"["shift+enter"]"#.to_owned(),
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
    fn copilot_sent_prompt_timestamp_follows_the_joined_prompt_in_its_shade() {
        let shade = "\u{1b}[48;2;20;27;34m";
        let text = format!("\u{1b}[38;2;240;246;252m{shade}");
        let clock = format!("\u{1b}[0m\u{1b}[38;2;145;152;161m{shade}10:19\u{1b}[0m{shade}");
        let ansi = format!(
            " \u{1b}[0m{shade} \u{1b}[0m{text}❯ Reply with \u{1b}[0m{shade}   {clock}  \u{1b}[0m\r\n \u{1b}[0m{shade}   \u{1b}[0m{text}just\u{1b}[0m{shade}        \u{1b}[0m\r\n \u{1b}[0m{shade}   \u{1b}[0m{text}OK.\u{1b}[0m{shade}         \u{1b}[0m\r\n ● Done"
        );
        let read = TerminalRead {
            copilot_scrollbar: true,
            ..read_with(&ansi, &[1, 2], &[])
        };
        assert_eq!(
            read.reflowed().unwrap(),
            format!(
                " \u{1b}[0m{shade} \u{1b}[0m{text}❯ Reply with \u{1b}[0m{shade}\u{1b}[0m{text}just \u{1b}[0m{shade}\u{1b}[0m\u{1b}[0m{shade}\u{1b}[0m{text}OK.\u{1b}[0m{shade}\u{1b}[0m\u{1b}[0m{shade} {clock}\u{1b}[0m\r\n ● Done"
            )
        );
        let unmoved = read_with(&ansi, &[1, 2], &[]).reflowed().unwrap();
        assert!(unmoved.starts_with(&format!(
            "{} \u{1b}[0m{shade}\u{1b}[0m",
            &ansi[..ansi.find("10:19").unwrap() + 5]
        )));
        let prose = format!(" ● The meeting is at {clock}\r\n   noon.");
        let read = TerminalRead {
            copilot_scrollbar: true,
            ..read_with(&prose, &[1], &[])
        };
        let joined = read.reflowed().unwrap();
        assert!(joined.find("10:19").unwrap() < joined.find("noon.").unwrap());
        for end in ["\u{a0}", "\u{3000}", "界", "🐕", "\u{1b}["] {
            let row = format!(" ❯ hi \u{1b}[0m{clock}{end}\r\n   there");
            let read = TerminalRead {
                copilot_scrollbar: true,
                ..read_with(&row, &[1], &[])
            };
            assert!(read.reflowed().unwrap().ends_with("there"));
        }
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
    }

    #[test]
    fn history_parent_call_id_is_additive() {
        let older = r#"{"id":"u1:0","role":"tool","text":"ok","tool":"Read","call_id":"c1","truncated":false}"#;
        let m: HistoryMessage = serde_json::from_str(older).unwrap();
        assert_eq!(m.parent_call_id, None);
        assert_eq!(serde_json::to_string(&m).unwrap(), older);
        let nested = HistoryMessage {
            parent_call_id: Some("task1".into()),
            ..m
        };
        let json = serde_json::to_string(&nested).unwrap();
        assert!(json.ends_with(r#""parent_call_id":"task1"}"#), "{json}");
    }
}
