use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::iter::Peekable;
use std::os::fd::OwnedFd;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::str::Chars;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use protocol::{
    AgentAnswerNoticeParams, AgentKind, AgentPromptParams, AgentSendKeysParams,
    AgentSlashDraftParams, AgentStatus, AgentTypeTextParams, CloneSource, Cwd, ErrorCode,
    FolderName, Key, OpId, OutputPatch, PaneCloseParams, ReadParams, ReadSource, Request, Response,
    SessionId, TaskArchiveParams, TaskFoldersParams, TaskNewParams, TaskOptions, TaskSessionInfo,
    TaskSessionsParams, TaskWorktree, TaskWorktreeInfo, TaskWorktreesParams, TerminalId,
    TerminalRead, TerminalRunParams, WorkspaceCloseParams, WorkspaceId, limits,
};
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::{process::Command, time::timeout};

use crate::draft::{self, InputBox};
use crate::herdr::{self, AgentInfo, PaneInfo};
use crate::transcript::{self, Transcripts};
use crate::{flock, git, prompt, pulls, reflow};

/// For `draft_changed` the message is the Mac's current draft: it goes to the phone in
/// `ErrorBody.draft` and is never written to the audit log.
pub type Fail = (ErrorCode, String);
pub type Reply = Result<Response, Fail>;
/// Checked again right before every herdr write, so an operation still running when its
/// peer is revoked cannot write anything more.
pub type Authorized = Arc<dyn Fn() -> bool + Send + Sync>;

const WATCH_EVERY: Duration = Duration::from_millis(250);
const WATCH_IDLE: Duration = Duration::from_secs(1);
/// A screen unchanged this long is read at `WATCH_IDLE`; the first change goes back to
/// `WATCH_EVERY`.
const WATCH_QUIET: Duration = Duration::from_secs(5);
const START_TIMEOUT: Duration = Duration::from_secs(30);
const START_POLL: Duration = Duration::from_millis(250);
pub const OP_TTL: Duration = Duration::from_secs(600);
pub const OPS_PER_PEER: usize = 256;
const MAX_SGR_PARAMS: usize = 64;
const SCREEN_SETTLE: Duration = Duration::from_secs(1);
const SCREEN_POLL: Duration = Duration::from_millis(100);
pub const DRAFT_CHANGED: &str = "the agent's input box has unsent text";
const BLOCKED: &str = "agent is blocked; answer it through an approval";
pub const HOSTS_AGENT: &str = "the pane now hosts an agent";
pub const LOCKED: &str = "the terminal is locked; unlock it again";
const SCROLLED: &str = "scrolled up on the machine; jump to the bottom first";
const COMMAND_SHOWN: &str = "a slash command shows in the input box; send it as a prompt";
const STARTUP_PROMPT: &str = "the agent is blocked on a startup prompt; answer it in the terminal";
pub const CODEX_UPDATE: &str = "Codex is asking to update. Answer it in Terminal.";
pub const TOO_LONG: &str = "The line is longer than the terminal accepts before its shell prompt shows. Wait for the prompt and try again.";
const MAX_FOLDERS: usize = 500;
const MAX_FOLDERS_SCANNED: usize = 10_000;
/// Within the phone's `TASK_CLONE_TIMEOUT`, before the agent start's own budget.
const CLONE_TIMEOUT: Duration = Duration::from_secs(75);
const MAX_CLONE_ERROR_BYTES: usize = 1000;
/// Command-line config, so it outranks every config file; a `protocol.<name>.allow`
/// in a file would otherwise override `protocol.allow`.
const CLONE_PROTOCOLS: [(&str, &str); 7] = [
    ("protocol.allow", "never"),
    ("protocol.file.allow", "never"),
    ("protocol.ext.allow", "never"),
    ("protocol.git.allow", "never"),
    ("protocol.http.allow", "never"),
    ("protocol.https.allow", "always"),
    ("protocol.ssh.allow", "always"),
];
/// `UF_HIDDEN` from `sys/stat.h`: Finder hides `~/Library` with it, not with a dot.
#[cfg(target_os = "macos")]
const UF_HIDDEN: u32 = 0x8000;
/// Claude Code's ctrl+end, bound to `scroll:bottom`; herdr 0.9.3 has no key name for it.
const SCROLL_BOTTOM: &str = "\u{1b}[1;5F";

fn startup_prompt(kind: &AgentKind, screen: Option<&str>) -> &'static str {
    if kind.as_str() == "codex" && screen.is_some_and(prompt::codex_update) {
        CODEX_UPDATE
    } else {
        STARTUP_PROMPT
    }
}

pub(crate) fn fail<T>(code: ErrorCode, message: impl Into<String>) -> Result<T, Fail> {
    Err((code, message.into()))
}

pub fn herdr_fail(e: herdr::Error) -> Fail {
    tracing::debug!(error = %e, "herdr call failed");
    let herdr::Error::Herdr { code, message } = &e else {
        return (ErrorCode::HerdrUnavailable, "herdr unavailable".to_owned());
    };
    let (code, message) = match code.as_str() {
        "agent_start_input_failed" | "pane_send_failed" if canonical_limit(message) => {
            (ErrorCode::TooLarge, TOO_LONG)
        }
        "agent_blocked" => (ErrorCode::AgentBlocked, BLOCKED),
        "agent_not_ready" | "agent_not_running" | "agent_launch_pending" => {
            (ErrorCode::AgentNotReady, "agent is not ready")
        }
        "empty_agent_prompt" => (ErrorCode::InvalidParams, "empty prompt"),
        "timeout" => (
            ErrorCode::AgentNotReady,
            "the agent did not take the input in time",
        ),
        "agent_pane_busy" => (ErrorCode::AgentNotReady, "the pane is busy"),
        "workspace_group_close_required" => (
            ErrorCode::NotImplemented,
            "the workspace has linked worktree workspaces; close the group in herdr",
        ),
        "agent_not_found" | "pane_not_found" | "workspace_not_found" | "target_pane_not_found" => {
            (ErrorCode::NotFound, "not found")
        }
        other => return (ErrorCode::Internal, format!("herdr refused: {other}")),
    };
    (code, message.to_owned())
}

fn authorized(auth: &Authorized) -> Result<(), Fail> {
    if auth() {
        Ok(())
    } else {
        fail(ErrorCode::NotPaired, "peer is no longer authorized")
    }
}

/// For terminal input the check is the grant: refusing with `not_paired` would make the
/// phone stop reconnecting.
fn granted(auth: &Authorized) -> Result<(), Fail> {
    if auth() {
        Ok(())
    } else {
        fail(ErrorCode::TerminalLocked, LOCKED)
    }
}

/// herdr 0.9.5 on macOS refuses a line longer than the terminal's canonical input limit
/// while its shell starts, instead of cutting it.
fn canonical_limit(message: &str) -> bool {
    message.contains("terminal is in canonical mode")
}

/// herdr's error message may quote the input, so only its code is kept, or the fact that
/// the line was too long.
fn quiet_fail(e: herdr::Error) -> Fail {
    match e {
        herdr::Error::Herdr { code, message } => herdr_fail(herdr::Error::Herdr {
            code,
            message: if canonical_limit(&message) {
                message
            } else {
                String::new()
            },
        }),
        other => herdr_fail(other),
    }
}

pub fn fingerprint(request: &Request) -> u64 {
    let mut h = DefaultHasher::new();
    serde_json::to_string(request)
        .unwrap_or_default()
        .hash(&mut h);
    h.finish()
}

pub struct Driver {
    herdr: PathBuf,
    agents: Vec<AgentKind>,
    roots: Vec<PathBuf>,
    ops: OpCache,
    /// Prompts, keys, slash drafts and notice answers from different sessions never
    /// interleave, so a mirrored command or a side agent's note is never submitted by
    /// another session's write.
    box_writes: tokio::sync::Mutex<()>,
    transcripts: Transcripts,
}

pub enum Watched {
    Output(TerminalRead),
    Patch(OutputPatch),
    Gone,
}

pub struct Watcher {
    rx: mpsc::Receiver<Watched>,
    task: JoinHandle<()>,
}

impl Watcher {
    pub async fn recv(&mut self) -> Option<Watched> {
        self.rx.recv().await
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Driver {
    pub fn new(herdr: PathBuf, agents: Vec<AgentKind>, roots: &[PathBuf]) -> anyhow::Result<Self> {
        let roots = roots
            .iter()
            .map(|r| {
                anyhow::ensure!(r.is_absolute(), "task root {} is not absolute", r.display());
                std::fs::canonicalize(r).with_context(|| format!("task root {}", r.display()))
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(Self {
            herdr,
            agents,
            roots,
            ops: OpCache::new(OPS_PER_PEER, OP_TTL),
            box_writes: tokio::sync::Mutex::new(()),
            transcripts: Transcripts::default(),
        })
    }

    pub fn with_transcripts(mut self, transcripts: Transcripts) -> Self {
        self.transcripts = transcripts;
        self
    }

    pub async fn once<F>(
        &self,
        peer: &str,
        op_id: &OpId,
        fingerprint: u64,
        op: F,
    ) -> (Reply, Origin)
    where
        F: Future<Output = Reply> + Send + 'static,
    {
        self.ops.once(peer, op_id, fingerprint, op).await
    }

    /// Never trusts a cached `pane_id`: it changes on cross-workspace moves.
    pub(crate) async fn find_agent(&self, terminal_id: &TerminalId) -> Result<AgentInfo, Fail> {
        herdr::agent_list(&self.herdr)
            .await
            .map_err(herdr_fail)?
            .into_iter()
            .find(|a| a.terminal_id == terminal_id.as_str())
            .ok_or_else(|| (ErrorCode::NotFound, "no such agent".to_owned()))
    }

    pub async fn find_pane(&self, terminal_id: &TerminalId) -> Result<PaneInfo, Fail> {
        Ok(self.pane_and_agent(terminal_id).await?.0)
    }

    /// Read now, never from a cache: an agent herdr is still launching counts, though its
    /// pane has no `agent` yet.
    async fn pane_and_agent(&self, terminal_id: &TerminalId) -> Result<(PaneInfo, bool), Fail> {
        let snap = herdr::session_snapshot(&self.herdr)
            .await
            .map_err(herdr_fail)?;
        let pane = snap
            .panes
            .into_iter()
            .find(|p| p.terminal_id == terminal_id.as_str())
            .ok_or_else(|| (ErrorCode::NotFound, "no such terminal".to_owned()))?;
        let agent = pane.agent.is_some()
            || snap
                .agents
                .iter()
                .any(|a| a.terminal_id == pane.terminal_id);
        Ok((pane, agent))
    }

    /// A pane with no agent at this moment. herdr targets panes by `pane_id` only, which it
    /// reuses: this is re-read right before every write.
    pub async fn shell_pane(&self, terminal_id: &TerminalId) -> Result<PaneInfo, Fail> {
        match self.pane_and_agent(terminal_id).await? {
            (_, true) => fail(ErrorCode::NotFound, HOSTS_AGENT),
            (pane, false) => Ok(pane),
        }
    }

    async fn git_checkout(&self, terminal_id: &TerminalId, auth: &Authorized) -> Result<Cwd, Fail> {
        authorized(auth)?;
        let current = self.readable_agent(terminal_id).await?;
        self.checkout(&current)
    }

    pub(crate) fn checkout(&self, agent: &AgentInfo) -> Result<Cwd, Fail> {
        let cwd = agent
            .foreground_cwd
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(agent.cwd.as_deref())
            .ok_or_else(|| invalid("agent has no checkout"))?;
        resolve_cwd(cwd, &self.roots).map_err(invalid)
    }

    pub(crate) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub async fn git_changes(&self, p: protocol::AgentChangesParams, auth: &Authorized) -> Reply {
        let cwd = self.git_checkout(&p.terminal_id, auth).await?;
        let changes = crate::git::changes(&cwd, &self.roots, p.root.as_ref(), p.scope).await?;
        authorized(auth)?;
        Ok(Response::GitChanges(changes))
    }

    pub async fn git_diff(&self, p: protocol::AgentDiffParams, auth: &Authorized) -> Reply {
        let cwd = self.git_checkout(&p.terminal_id, auth).await?;
        let diff = crate::git::diff(&cwd, &self.roots, &p).await?;
        authorized(auth)?;
        Ok(Response::GitDiff(diff))
    }

    async fn current_agent(
        &self,
        terminal_id: &TerminalId,
    ) -> Result<(AgentInfo, Result<(), Fail>), Fail> {
        let listed = self.find_agent(terminal_id).await?;
        let current = herdr::agent_get(&self.herdr, &listed.pane_id)
            .await
            .map_err(herdr_fail)?;
        let ready = check_ready(&listed, &current);
        Ok((current, ready))
    }

    async fn ready_agent(&self, terminal_id: &TerminalId) -> Result<AgentInfo, Fail> {
        let (current, ready) = self.current_agent(terminal_id).await?;
        ready.map(|()| current)
    }

    async fn readable_agent(&self, terminal_id: &TerminalId) -> Result<AgentInfo, Fail> {
        match self.current_agent(terminal_id).await? {
            (current, Ok(()) | Err((ErrorCode::AgentBlocked, _))) => Ok(current),
            (_, Err(e)) => Err(e),
        }
    }

    async fn writable_agent(&self, terminal_id: &TerminalId) -> Result<AgentInfo, Fail> {
        match self.current_agent(terminal_id).await? {
            (current, Err((ErrorCode::AgentBlocked, _))) if current.agent.is_some() => Ok(current),
            (current, ready) => ready.map(|()| current),
        }
    }

    /// Read with SGR: [`protocol::jump_banner`] finds the banner by how it is drawn.
    async fn scrolled(&self, pane_id: &str) -> Result<bool, Fail> {
        let read = herdr::pane_read(&self.herdr, pane_id, source_name(ReadSource::Visible), None)
            .await
            .map_err(herdr_fail)?;
        Ok(protocol::jump_banner(&sanitize_ansi(&read.text)))
    }

    async fn input_safeguards(&self, a: &AgentInfo, submits: bool) -> Result<(), Fail> {
        if a.agent.as_deref() != Some("claude")
            || flock::status(&a.agent_status) == AgentStatus::Blocked
        {
            return Ok(());
        }
        if self.scrolled(&a.pane_id).await? {
            return fail(ErrorCode::AgentNotReady, SCROLLED);
        }
        if submits
            && let Some(InputBox::Draft(d)) = self.input_box(&a.pane_id).await?
            && d.text.starts_with('/')
        {
            return fail(ErrorCode::AgentNotReady, COMMAND_SHOWN);
        }
        Ok(())
    }

    /// `pane.read` reads agent panes only: a shell is read through `terminal.watch`, under
    /// a grant.
    pub async fn read(&self, p: ReadParams, agent: bool) -> Reply {
        let mut lines = p.lines.map(u32::from);
        let source = source_name(p.source);
        let mut reflow = None;
        let read = if agent {
            let a = self.find_agent(&p.terminal_id).await?;
            if let Some(kind @ ("claude" | "codex" | "copilot")) = a.agent.as_deref() {
                reflow = Some((a.pane_id.clone(), kind.to_owned()));
            }
            self.agent_read(&a.pane_id, source, &mut lines).await
        } else {
            let (pane, agent) = self.pane_and_agent(&p.terminal_id).await?;
            if !agent {
                return fail(
                    ErrorCode::NotFound,
                    "no agent in this pane; open it as a terminal",
                );
            }
            herdr::pane_read(&self.herdr, &pane.pane_id, source, lines).await
        }
        .map_err(herdr_fail)?;
        let mut read = terminal_read(p.terminal_id, p.source, read);
        if let Some((pane_id, kind)) = reflow {
            self.reflow(&pane_id, &kind, &mut read).await;
        }
        Ok(Response::Terminal(read))
    }

    /// Leaves `read` unjoined when herdr does not give the pane's width; `false` when herdr
    /// failed.
    async fn reflow(&self, pane_id: &str, kind: &str, read: &mut TerminalRead) -> bool {
        let cols = herdr::pane_columns(&self.herdr, pane_id).await;
        let screen_cols = match kind {
            "copilot" => reflow::copilot_scrollbar_columns(&read.ansi),
            "claude" => reflow::claude_rule_columns(&read.ansi),
            _ => None,
        };
        if let Ok(Some(cols)) = cols {
            (read.wraps, read.splits) =
                reflow::soft_wraps_for(kind, &read.ansi, screen_cols.unwrap_or(cols.into()));
            read.copilot_scrollbar = kind == "copilot";
        }
        if kind == "copilot"
            && let Some(width) =
                screen_cols.or_else(|| cols.as_ref().ok().and_then(|cols| cols.map(usize::from)))
        {
            read.ansi = reflow::without_copilot_scrollbar(&read.ansi, width);
        }
        if kind == "claude" {
            read.ansi = reflow::without_claude_black(&read.ansi);
        }
        cols.is_ok()
    }

    /// A reply longer than herdr's line limit is refused, so a pane dense with escapes is
    /// read again with half the lines, and `lines` keeps the depth that fit.
    async fn agent_read(
        &self,
        pane_id: &str,
        source: &str,
        lines: &mut Option<u32>,
    ) -> Result<herdr::PaneRead, herdr::Error> {
        self.halving_read(pane_id, source, lines, true).await
    }

    async fn halving_read(
        &self,
        pane_id: &str,
        source: &str,
        lines: &mut Option<u32>,
        agent: bool,
    ) -> Result<herdr::PaneRead, herdr::Error> {
        loop {
            let read = if agent {
                herdr::agent_read(&self.herdr, pane_id, source, *lines).await
            } else {
                herdr::pane_read(&self.herdr, pane_id, source, *lines).await
            };
            match read {
                Err(herdr::Error::LineTooLong) if lines.is_some_and(|n| n > 1) => {
                    *lines = lines.map(|n| n / 2);
                }
                read => return read,
            }
        }
    }

    pub async fn watch(
        self: &Arc<Self>,
        terminal_id: TerminalId,
        lines: u16,
        low_data: bool,
    ) -> Result<Watcher, Fail> {
        self.find_agent(&terminal_id).await?;
        let (tx, rx) = mpsc::channel(1);
        let task = tokio::spawn(
            self.clone()
                .watch_loop(terminal_id, lines, low_data, None, tx),
        );
        Ok(Watcher { rx, task })
    }

    /// Ends with `Gone` once the pane closes or an agent starts in it, and reads nothing
    /// while `auth` (the grant) does not hold.
    pub async fn watch_terminal(
        self: &Arc<Self>,
        terminal_id: TerminalId,
        lines: u16,
        low_data: bool,
        auth: Authorized,
    ) -> Result<Watcher, Fail> {
        self.shell_pane(&terminal_id).await?;
        let (tx, rx) = mpsc::channel(1);
        let watch = self
            .clone()
            .watch_loop(terminal_id, lines, low_data, Some(auth), tx);
        let task = tokio::spawn(watch);
        Ok(Watcher { rx, task })
    }

    async fn watch_loop(
        self: Arc<Self>,
        terminal_id: TerminalId,
        lines: u16,
        low_data: bool,
        grant: Option<Authorized>,
        tx: mpsc::Sender<Watched>,
    ) {
        let shell = grant.is_some();
        let mut next = tokio::time::Instant::now();
        let mut changed = next;
        let mut last = None;
        let mut sent: Option<TerminalRead> = None;
        let mut lines = Some(u32::from(lines));
        loop {
            tokio::time::sleep_until(next).await;
            let now = tokio::time::Instant::now();
            next = now + watch_period(low_data, now - changed);
            let pane = match &grant {
                Some(auth) if !auth() => continue,
                Some(_) => self
                    .shell_pane(&terminal_id)
                    .await
                    .map(|p| (p.pane_id, None)),
                None => self
                    .find_agent(&terminal_id)
                    .await
                    .map(|a| (a.pane_id, a.agent)),
            };
            let msg = match pane {
                Err((ErrorCode::NotFound, _)) => Watched::Gone,
                Err(_) => continue,
                Ok((pane_id, kind)) => {
                    let Ok(read) = self
                        .halving_read(
                            &pane_id,
                            source_name(ReadSource::Recent),
                            &mut lines,
                            !shell,
                        )
                        .await
                    else {
                        continue;
                    };
                    let mut h = DefaultHasher::new();
                    (&read.text, read.truncated).hash(&mut h);
                    let hash = h.finish();
                    if last == Some(hash) {
                        continue;
                    }
                    last = Some(hash);
                    changed = now;
                    next = now + watch_period(low_data, Duration::ZERO);
                    let mut read = terminal_read(terminal_id.clone(), ReadSource::Recent, read);
                    if let Some(kind @ ("claude" | "codex" | "copilot")) = kind.as_deref()
                        && !self.reflow(&pane_id, kind, &mut read).await
                    {
                        last = None;
                    }
                    if sent.as_ref() == Some(&read) {
                        continue;
                    }
                    let patch = sent
                        .as_ref()
                        .and_then(|prev| OutputPatch::between(prev, &read));
                    let watched = match patch {
                        Some(patch) => Watched::Patch(patch),
                        None => Watched::Output(read.clone()),
                    };
                    sent = Some(read);
                    watched
                }
            };
            let gone = matches!(msg, Watched::Gone);
            if tx.send(msg).await.is_err() || gone {
                return;
            }
        }
    }

    pub async fn draft(&self, terminal_id: &TerminalId) -> Reply {
        let a = self.find_agent(terminal_id).await?;
        let text = match a.agent.as_deref() {
            Some("claude") => match self.input_box(&a.pane_id).await? {
                Some(InputBox::Draft(d)) => Some(d.text),
                Some(InputBox::Opaque) | None => None,
            },
            _ => None,
        };
        Ok(Response::Draft { text })
    }

    async fn input_box(&self, pane_id: &str) -> Result<Option<InputBox>, Fail> {
        let read = herdr::pane_read(&self.herdr, pane_id, source_name(ReadSource::Visible), None)
            .await
            .map_err(herdr_fail)?;
        Ok(draft::parse(&sanitize_ansi(&read.text)))
    }

    pub async fn prompt(&self, p: AgentPromptParams, auth: &Authorized) -> Reply {
        self.prompt_agent(p, auth, false).await
    }

    /// `idle_only` refuses an agent that the fresh read finds working, or that is not Claude
    /// Code, the only kind whose input box is checked before the paste.
    pub(crate) async fn prompt_agent(
        &self,
        p: AgentPromptParams,
        auth: &Authorized,
        idle_only: bool,
    ) -> Reply {
        let _box = self.box_writes.lock().await;
        let a = self.ready_agent(&p.terminal_id).await?;
        if idle_only
            && (a.agent.as_deref() != Some("claude")
                || !matches!(
                    flock::status(&a.agent_status),
                    AgentStatus::Idle | AgentStatus::Done
                ))
        {
            return fail(ErrorCode::AgentNotReady, "the agent is busy");
        }
        // herdr pastes a prompt after whatever is in Claude Code's input box.
        if a.agent.as_deref() == Some("claude") {
            // herdr ends a prompt with Enter, which would confirm a dialog scrolled out of view.
            if self.scrolled(&a.pane_id).await? {
                return fail(ErrorCode::AgentNotReady, SCROLLED);
            }
            let expected = p.expected_draft.as_ref().map(|d| d.as_str());
            self.replace_draft(&a.pane_id, expected, auth).await?;
        }
        authorized(auth)?;
        herdr::agent_prompt(&self.herdr, &a.pane_id, p.text.as_str())
            .await
            .map_err(herdr_fail)?;
        Ok(Response::Ok)
    }

    /// A draft is cleared only when it is the one the phone saw. Anything else fails
    /// closed: the agent passed `ready_agent`, so a box that cannot be read or replaced
    /// may hold text that herdr would paste the prompt after, a shell command in bash mode
    /// included.
    async fn replace_draft(
        &self,
        pane_id: &str,
        expected: Option<&str>,
        auth: &Authorized,
    ) -> Result<(), Fail> {
        let current = match self.input_box(pane_id).await? {
            Some(InputBox::Draft(d)) => d,
            Some(InputBox::Opaque) => {
                return fail(
                    ErrorCode::DraftNotCleared,
                    "the agent's input box holds a paste, an image or another mode; nothing was sent",
                );
            }
            None => {
                return fail(
                    ErrorCode::DraftNotCleared,
                    "the agent's input box could not be read; nothing was sent",
                );
            }
        };
        if current.text.is_empty() {
            return Ok(());
        }
        if expected.is_none_or(|e| draft::normalize(e) != current.text) {
            return Err((ErrorCode::DraftChanged, current.text));
        }
        for keys in draft::clear_keys(current.lines).chunks(limits::MAX_KEYS_PER_CALL) {
            authorized(auth)?;
            herdr::agent_send_keys(&self.herdr, pane_id, keys)
                .await
                .map_err(herdr_fail)?;
        }
        let deadline = tokio::time::Instant::now() + SCREEN_SETTLE;
        loop {
            tokio::time::sleep(SCREEN_POLL).await;
            if matches!(
                self.input_box(pane_id).await?,
                Some(InputBox::Draft(d)) if d.text.is_empty()
            ) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return fail(
                    ErrorCode::DraftNotCleared,
                    "could not clear the agent's input box; nothing was sent",
                );
            }
        }
    }

    /// Keys to a blocked agent also return their audit target, which names the keys.
    pub async fn send_keys(
        &self,
        p: AgentSendKeysParams,
        auth: &Authorized,
    ) -> (Reply, Option<String>) {
        let _box = self.box_writes.lock().await;
        let a = match self.writable_agent(&p.terminal_id).await {
            Ok(found) => found,
            Err(e) => return (Err(e), None),
        };
        let blocked = flock::status(&a.agent_status) == AgentStatus::Blocked;
        let submits = p
            .keys
            .iter()
            .any(|k| matches!(k, Key::Enter | Key::CtrlEnter));
        if let Err(e) = self.input_safeguards(&a, submits).await {
            return (Err(e), None);
        }
        let keys: Vec<&str> = p.keys.iter().map(|k| k.herdr_name()).collect();
        let target =
            blocked.then(|| format!("{} blocked keys={}", p.terminal_id.as_str(), keys.join(",")));
        let sent = async {
            authorized(auth)?;
            herdr::agent_send_keys(&self.herdr, &a.pane_id, &keys)
                .await
                .map_err(herdr_fail)?;
            Ok(Response::Ok)
        };
        (sent.await, target)
    }

    pub async fn type_text(&self, p: AgentTypeTextParams, auth: &Authorized) -> Reply {
        let _box = self.box_writes.lock().await;
        let a = self.writable_agent(&p.terminal_id).await?;
        self.input_safeguards(&a, true).await?;
        authorized(auth)?;
        let paste = format!("\u{1b}[200~{}\u{1b}[201~", p.text.as_str());
        herdr::pane_send_input(&self.herdr, &a.pane_id, &paste, &["enter"])
            .await
            .map_err(herdr_fail)?;
        Ok(Response::Ok)
    }

    /// One line, then Enter, into a pane that has no agent at this moment. herdr sends the
    /// text and the key in one ordered write, as `herdr pane run` does.
    pub async fn terminal_run(&self, p: TerminalRunParams, auth: &Authorized) -> Reply {
        let pane = self.shell_pane(&p.terminal_id).await?;
        granted(auth)?;
        herdr::pane_send_input(&self.herdr, &pane.pane_id, p.text.as_str(), &["enter"])
            .await
            .map_err(quiet_fail)?;
        Ok(Response::Ok)
    }

    pub async fn terminal_send_keys(&self, p: AgentSendKeysParams, auth: &Authorized) -> Reply {
        let keys: Vec<&str> = p.keys.iter().map(|k| k.herdr_name()).collect();
        let pane = self.shell_pane(&p.terminal_id).await?;
        granted(auth)?;
        herdr::pane_send_keys(&self.herdr, &pane.pane_id, &keys)
            .await
            .map_err(quiet_fail)?;
        Ok(Response::Ok)
    }

    pub async fn focus(&self, terminal_id: &TerminalId, auth: &Authorized) -> Reply {
        let a = self.find_agent(terminal_id).await?;
        authorized(auth)?;
        herdr::agent_focus(&self.herdr, &a.pane_id)
            .await
            .map_err(herdr_fail)?;
        Ok(Response::Ok)
    }

    /// Navigation only, and only while Claude Code shows its banner: Ctrl+End never answers
    /// a dialog, and brings back one scrolled out of view.
    pub async fn scroll_bottom(&self, terminal_id: &TerminalId, auth: &Authorized) -> Reply {
        let current = self.readable_agent(terminal_id).await?;
        if current.agent.as_deref() != Some("claude") {
            return fail(ErrorCode::AgentNotReady, "not a Claude Code agent");
        }
        if !self.scrolled(&current.pane_id).await? {
            return fail(ErrorCode::AgentNotReady, "already at the bottom");
        }
        authorized(auth)?;
        herdr::pane_send_text(&self.herdr, &current.pane_id, SCROLL_BOTTOM)
            .await
            .map_err(herdr_fail)?;
        Ok(Response::Ok)
    }

    /// One digit, no Enter, only while Claude Code shows a notice listing it with that label
    /// above an empty input box. Never to a blocked agent: a digit could select a permission
    /// option. The notice and the box are checked on the same visible read, the last one
    /// before the write. The digit sits in the box for 400 ms before Claude Code clears it and
    /// runs the option, which can then fill the box, only if it is still empty, so the lock is
    /// held until the box shows something other than the digit, for up to a second: a prompt
    /// pasted before the fill would be replaced by it, and herdr's Enter would submit the
    /// fill.
    pub async fn answer_notice(&self, p: &AgentAnswerNoticeParams, auth: &Authorized) -> Reply {
        let _box = self.box_writes.lock().await;
        let a = self.ready_agent(&p.terminal_id).await?;
        if a.agent.as_deref() != Some("claude") {
            return fail(ErrorCode::AgentNotReady, "not a Claude Code agent");
        }
        if self.scrolled(&a.pane_id).await? {
            return fail(ErrorCode::AgentNotReady, SCROLLED);
        }
        let read = herdr::pane_read(
            &self.herdr,
            &a.pane_id,
            source_name(ReadSource::Visible),
            None,
        )
        .await
        .map_err(herdr_fail)?;
        let visible = sanitize_ansi(&read.text);
        if !protocol::notice(&visible)
            .iter()
            .any(|o| o.digit == p.digit && o.label == p.label.as_str())
        {
            return fail(ErrorCode::AgentNotReady, "no notice with that option");
        }
        if !matches!(
            draft::parse(&visible),
            Some(InputBox::Draft(d)) if d.text.is_empty()
        ) {
            return fail(ErrorCode::AgentNotReady, "the input box is not empty");
        }
        authorized(auth)?;
        herdr::pane_send_text(&self.herdr, &a.pane_id, p.digit.as_str())
            .await
            .map_err(herdr_fail)?;
        let deadline = tokio::time::Instant::now() + SCREEN_SETTLE;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(SCREEN_POLL).await;
            match self.input_box(&a.pane_id).await? {
                Some(InputBox::Draft(d)) if d.text.is_empty() || d.text == p.digit.as_str() => {}
                None => {}
                Some(_) => break,
            }
        }
        Ok(Response::Ok)
    }

    /// Mirrors the phone's slash command token into Claude Code's input box as a bracketed
    /// paste, never Enter or a key, so its command menu shows. The box is cleared first
    /// through [`Self::replace_draft`]; an empty command stops there. The agent, the banner
    /// and an empty visible box are checked again just before the paste. It returns, and
    /// releases the lock, only once the box shows the command: the next mirror's first read
    /// would otherwise see the box empty before this paste renders, skip the clear, and
    /// paste after it.
    pub async fn slash_draft(&self, p: AgentSlashDraftParams, auth: &Authorized) -> Reply {
        let _box = self.box_writes.lock().await;
        let a = self.claude_at_bottom(&p.terminal_id).await?;
        let expected = p.expected_draft.as_ref().map(|d| d.as_str());
        self.replace_draft(&a.pane_id, expected, auth).await?;
        if p.command.as_str().is_empty() {
            return Ok(Response::Ok);
        }
        let a = self.claude_at_bottom(&p.terminal_id).await?;
        if !matches!(
            self.input_box(&a.pane_id).await?,
            Some(InputBox::Draft(d)) if d.text.is_empty()
        ) {
            return fail(ErrorCode::AgentNotReady, "the input box is not empty");
        }
        authorized(auth)?;
        let paste = format!("\u{1b}[200~{}\u{1b}[201~", p.command.as_str());
        herdr::pane_send_text(&self.herdr, &a.pane_id, &paste)
            .await
            .map_err(herdr_fail)?;
        let deadline = tokio::time::Instant::now() + SCREEN_SETTLE;
        loop {
            tokio::time::sleep(SCREEN_POLL).await;
            let shown = self.input_box(&a.pane_id).await?;
            if matches!(&shown, Some(InputBox::Draft(d)) if d.text == p.command.as_str()) {
                return Ok(Response::Ok);
            }
            if tokio::time::Instant::now() >= deadline {
                return match shown {
                    Some(InputBox::Draft(d)) if !d.text.is_empty() => {
                        Err((ErrorCode::DraftChanged, d.text))
                    }
                    _ => fail(
                        ErrorCode::AgentNotReady,
                        "the input box did not show the command",
                    ),
                };
            }
        }
    }

    async fn claude_at_bottom(&self, terminal_id: &TerminalId) -> Result<AgentInfo, Fail> {
        let a = self.ready_agent(terminal_id).await?;
        if a.agent.as_deref() != Some("claude") {
            return fail(ErrorCode::AgentNotReady, "not a Claude Code agent");
        }
        if self.scrolled(&a.pane_id).await? {
            return fail(ErrorCode::AgentNotReady, SCROLLED);
        }
        Ok(a)
    }

    pub async fn task_options(&self) -> Reply {
        let (known, snap) = tokio::try_join!(
            herdr::agent_manifests(&self.herdr),
            herdr::session_snapshot(&self.herdr)
        )
        .map_err(herdr_fail)?;
        let agents: Vec<AgentKind> = self
            .agents
            .iter()
            .filter(|a| known.iter().any(|k| k == a.as_str()))
            .cloned()
            .collect();
        let Some(default_agent) = agents.first().cloned() else {
            return fail(
                ErrorCode::NotFound,
                "no allowed agent kind is known to herdr",
            );
        };
        let cwds: Vec<String> = flock::map_workspaces(&snap)
            .into_iter()
            .filter_map(|w| w.cwd)
            .collect();
        let roots = self.roots.clone();
        let recent_cwds = tokio::task::spawn_blocking(move || {
            let mut out: Vec<Cwd> = Vec::new();
            for cwd in cwds.iter().filter_map(|c| resolve_cwd(c, &roots).ok()) {
                if !out.contains(&cwd) {
                    out.push(cwd);
                }
            }
            out
        })
        .await
        .map_err(|_| (ErrorCode::Internal, "cwd check failed".to_owned()))?;
        let roots = self
            .roots
            .iter()
            .filter_map(|r| r.to_str().and_then(|r| Cwd::new(r).ok()))
            .collect();
        Ok(Response::TaskOptions(TaskOptions {
            agents,
            default_agent,
            recent_cwds,
            roots,
        }))
    }

    pub async fn task_folders(&self, p: TaskFoldersParams, auth: &Authorized) -> Reply {
        authorized(auth)?;
        let roots = self.roots.clone();
        tokio::task::spawn_blocking(move || list_folders(p.path.as_str(), &roots))
            .await
            .map_err(|_| (ErrorCode::Internal, "folder listing failed".to_owned()))?
    }

    pub async fn task_worktrees(&self, p: TaskWorktreesParams, auth: &Authorized) -> Reply {
        authorized(auth)?;
        let requested = resolve_cwd(p.cwd.as_str(), &self.roots).map_err(invalid)?;
        let list = herdr::worktree_list(&self.herdr, requested.as_str())
            .await
            .map_err(herdr_fail)?;
        let source =
            resolve_cwd(&list.source.source_checkout_path, &self.roots).map_err(invalid)?;
        if !Path::new(requested.as_str()).starts_with(source.as_str()) {
            return fail(
                ErrorCode::InvalidParams,
                "herdr returned another source checkout",
            );
        }
        let worktrees = list
            .worktrees
            .into_iter()
            .filter_map(|w| {
                if w.is_bare || !w.is_linked_worktree {
                    return None;
                }
                let path = resolve_cwd(&w.path, &self.roots).ok()?;
                Some(TaskWorktreeInfo {
                    path,
                    branch: w.branch,
                    open: w.open_workspace_id.is_some(),
                })
            })
            .collect();
        Ok(Response::TaskWorktrees { source, worktrees })
    }

    pub async fn task_sessions(&self, p: TaskSessionsParams, auth: &Authorized) -> Reply {
        authorized(auth)?;
        let cwd = resolve_cwd(p.cwd.as_str(), &self.roots).map_err(invalid)?;
        let allowed = |kind: &str| self.agents.iter().any(|a| a.as_str() == kind);
        let (claude, codex) = (allowed("claude"), allowed("codex"));
        let open: Vec<String> = herdr::agent_list(&self.herdr)
            .await
            .map_err(herdr_fail)?
            .into_iter()
            .filter_map(|a| a.agent_session.map(|s| s.value))
            .collect();
        let reader = self.transcripts.history_reader();
        let sessions =
            tokio::task::spawn_blocking(move || reader.sessions(cwd.as_str(), claude, codex))
                .await
                .map_err(|_| (ErrorCode::Internal, "session listing failed".to_owned()))?
                .into_iter()
                .filter(|s| !open.contains(&s.id))
                .filter_map(|s| {
                    Some(TaskSessionInfo {
                        agent: AgentKind::new(s.agent).ok()?,
                        session_id: SessionId::new(s.id).ok()?,
                        first_prompt: s.first_prompt,
                        modified_ms: s.modified_ms,
                    })
                })
                .collect();
        Ok(Response::TaskSessions { sessions })
    }

    /// Also returns the canonical cwd once it is resolved or created, for the audit line.
    pub async fn task_new(&self, p: TaskNewParams, auth: &Authorized) -> (Reply, Option<Cwd>) {
        if [
            p.worktree.is_some(),
            p.new_folder.is_some(),
            p.clone.is_some(),
        ]
        .into_iter()
        .filter(|set| *set)
        .count()
            > 1
        {
            return (
                fail(
                    ErrorCode::InvalidParams,
                    "choose a folder, worktree or clone",
                ),
                None,
            );
        }
        if p.resume.is_some()
            && (p.worktree.is_some() || p.new_folder.is_some() || p.clone.is_some())
        {
            return (
                fail(
                    ErrorCode::InvalidParams,
                    "resume a session in its own folder",
                ),
                None,
            );
        }
        if p.prompt.is_none() && p.resume.is_none() {
            return (fail(ErrorCode::InvalidParams, "empty prompt"), None);
        }
        if p.ultracode && p.agent.as_str() != "claude" {
            return (
                fail(ErrorCode::InvalidParams, "ultracode needs Claude Code"),
                None,
            );
        }
        let made = if p.clone.is_some() {
            Some("cloned")
        } else {
            p.new_folder.as_ref().map(|_| "created")
        };
        match self.task_cwd(&p, auth).await {
            Ok(cwd) => {
                let mut reply = if p.worktree.is_some() {
                    self.start_worktree_task(p, &cwd, auth).await
                } else {
                    self.start_task(p, &cwd, auth).await
                };
                if let Some(made) = made {
                    reply = reply.map_err(|(code, message)| {
                        let left = format!("folder {} was {made} and left in place", cwd.as_str());
                        (code, format!("{left}: {message}"))
                    });
                }
                (reply, Some(cwd))
            }
            Err(e) => (Err(e), None),
        }
    }

    async fn task_cwd(&self, p: &TaskNewParams, auth: &Authorized) -> Result<Cwd, Fail> {
        if !self.agents.contains(&p.agent) {
            return fail(ErrorCode::InvalidParams, "agent kind is not allowed");
        }
        let roots = self.roots.clone();
        let requested = p.cwd.as_str().to_owned();
        let new_folder = p
            .new_folder
            .clone()
            .or_else(|| p.clone.as_ref().map(CloneSource::folder_name));
        let auth_check = auth.clone();
        let cwd = tokio::task::spawn_blocking(move || match new_folder {
            Some(name) => create_folder(&requested, &name, &roots, &auth_check),
            None => resolve_cwd(&requested, &roots).map_err(invalid),
        })
        .await
        .map_err(|_| (ErrorCode::Internal, "cwd check failed".to_owned()))??;
        let Some(source) = &p.clone else {
            return Ok(cwd);
        };
        let cloned = match authorized(auth) {
            Ok(()) => clone_repository(source, &cwd)
                .await
                .map_err(|message| invalid(&format!("clone failed: {message}"))),
            Err(e) => Err(e),
        };
        let Err((code, message)) = cloned else {
            return Ok(cwd);
        };
        let left = if std::fs::remove_dir(cwd.as_str()).is_ok() {
            String::new()
        } else {
            format!("; folder {} was left in place", cwd.as_str())
        };
        fail(code, format!("{message}{left}"))
    }

    /// On a failure after `workspace.create` nothing is closed: the error names the
    /// workspace that was left open.
    async fn start_task(&self, p: TaskNewParams, cwd: &Cwd, auth: &Authorized) -> Reply {
        if let Some(id) = &p.resume {
            self.resumable(&p.agent, id, cwd).await?;
        }
        authorized(auth)?;
        let pane = herdr::workspace_create(
            &self.herdr,
            cwd.as_str(),
            p.label.as_ref().map(|l| l.as_str()),
        )
        .await
        .map_err(herdr_fail)?
        .root_pane;
        self.finish_task(&p, &pane, "created and left open", true, auth)
            .await
    }

    async fn resumable(&self, agent: &AgentKind, id: &SessionId, cwd: &Cwd) -> Result<(), Fail> {
        if !transcript::is_session_id(id.as_str()) {
            return fail(ErrorCode::InvalidParams, "invalid session id");
        }
        if !matches!(agent.as_str(), "claude" | "codex") {
            return fail(
                ErrorCode::InvalidParams,
                "this agent cannot resume a session",
            );
        }
        let (reader, kind, session, folder) = (
            self.transcripts.history_reader(),
            agent.as_str().to_owned(),
            id.as_str().to_owned(),
            cwd.as_str().to_owned(),
        );
        let found =
            tokio::task::spawn_blocking(move || reader.has_session(&kind, &folder, &session))
                .await
                .map_err(|_| (ErrorCode::Internal, "session check failed".to_owned()))?;
        if !found {
            return fail(ErrorCode::InvalidParams, "no such session in this folder");
        }
        let open = herdr::agent_list(&self.herdr)
            .await
            .map_err(herdr_fail)?
            .iter()
            .any(|a| {
                a.agent_session
                    .as_ref()
                    .is_some_and(|s| s.value == id.as_str())
            });
        if open {
            return fail(
                ErrorCode::InvalidParams,
                "the session is already open in a herdr pane",
            );
        }
        Ok(())
    }

    async fn start_worktree_task(
        &self,
        p: TaskNewParams,
        source: &Cwd,
        auth: &Authorized,
    ) -> Reply {
        let Some(worktree) = &p.worktree else {
            unreachable!()
        };
        let path = match worktree {
            TaskWorktree::Create { branch } => {
                if !valid_branch(branch) {
                    return fail(ErrorCode::InvalidParams, "invalid branch name");
                }
                None
            }
            TaskWorktree::Open { path } => {
                Some(resolve_cwd(path.as_str(), &self.roots).map_err(invalid)?)
            }
            TaskWorktree::PullRequest { .. } => None,
        };
        authorized(auth)?;
        let listed = herdr::worktree_list(&self.herdr, source.as_str())
            .await
            .map_err(herdr_fail)?;
        let checkout =
            resolve_cwd(&listed.source.source_checkout_path, &self.roots).map_err(invalid)?;
        if !Path::new(source.as_str()).starts_with(checkout.as_str()) {
            return fail(
                ErrorCode::InvalidParams,
                "herdr returned another source checkout",
            );
        }
        let deadline = tokio::time::Instant::now() + CLONE_TIMEOUT;
        let pulled;
        let (worktree, head) = match worktree {
            TaskWorktree::PullRequest { number } => {
                authorized(auth)?;
                let head = within(deadline, pull_request_head(&checkout, *number)).await?;
                pulled = TaskWorktree::Create {
                    branch: head.branch.clone(),
                };
                (&pulled, Some((*number, head)))
            }
            other => (other, None),
        };
        let path = match (worktree, path) {
            (TaskWorktree::Create { branch }, None) => {
                let name = branch.replace('/', "-");
                FolderName::new(&name).map_err(|_| invalid("invalid checkout folder name"))?;
                let parent = worktree_parent(&checkout, &self.roots, auth)?;
                let target = Path::new(parent.as_str()).join(name);
                if target.symlink_metadata().is_ok() {
                    return fail(ErrorCode::InvalidParams, "checkout path already exists");
                }
                Cwd::new(target.to_str().unwrap_or(""))
                    .map_err(|_| invalid("invalid checkout path"))?
            }
            (TaskWorktree::Open { .. }, Some(path)) => path,
            _ => unreachable!(),
        };
        let expected_branch = if matches!(worktree, TaskWorktree::Open { .. }) {
            let Some(choice) = listed
                .worktrees
                .iter()
                .find(|w| w.path == path.as_str() && w.is_linked_worktree && !w.is_bare)
            else {
                return fail(
                    ErrorCode::InvalidParams,
                    "checkout is not in the selected repository",
                );
            };
            Some(choice.branch.clone())
        } else {
            None
        };
        if let Some((number, head)) = &head {
            authorized(auth)?;
            fetch_pull_request(&checkout, *number, head, deadline).await?;
        }
        authorized(auth)?;
        let result = match worktree {
            TaskWorktree::Create { branch } => {
                herdr::worktree_create(
                    &self.herdr,
                    source.as_str(),
                    branch,
                    path.as_str(),
                    p.label.as_ref().map(|l| l.as_str()),
                )
                .await
            }
            TaskWorktree::Open { .. } => {
                herdr::worktree_open(
                    &self.herdr,
                    source.as_str(),
                    path.as_str(),
                    p.label.as_ref().map(|l| l.as_str()),
                )
                .await
            }
            TaskWorktree::PullRequest { .. } => unreachable!(),
        }
        .map_err(|e| {
            let failure = herdr_fail(e);
            if matches!(worktree, TaskWorktree::Create { .. })
                && Path::new(path.as_str()).symlink_metadata().is_ok()
            {
                (
                    failure.0,
                    format!(
                        "checkout path {} now exists; herdr returned: {}",
                        path.as_str(),
                        failure.1
                    ),
                )
            } else if let TaskWorktree::Create { branch } = worktree {
                (
                    failure.0,
                    format!(
                        "worktree.create for branch {branch} at {} failed; the branch may remain: {}",
                        path.as_str(),
                        failure.1
                    ),
                )
            } else {
                failure
            }
        })?;
        let left = |(code, message): Fail| {
            (
                code,
                format!(
                    "checkout {} (branch {}) and workspace {} were left in place: {message}",
                    result.worktree.path,
                    result.worktree.branch.as_deref().unwrap_or("detached"),
                    result.workspace.workspace_id,
                ),
            )
        };
        let actual = resolve_cwd(&result.worktree.path, &self.roots)
            .map_err(invalid)
            .map_err(left)?;
        let pane_cwd = result
            .root_pane
            .cwd
            .as_deref()
            .and_then(|cwd| resolve_cwd(cwd, &self.roots).ok());
        if actual.as_str() != path.as_str()
            || pane_cwd.as_ref() != Some(&actual)
            || result.root_pane.workspace_id != result.workspace.workspace_id
            || matches!(worktree, TaskWorktree::Create { branch }
                if result.worktree.branch.as_deref() != Some(branch))
            || expected_branch
                .as_ref()
                .is_some_and(|branch| result.worktree.branch.as_deref() != branch.as_deref())
        {
            return Err(left(invalid("herdr returned a different checkout or pane")));
        }
        let pane = &result.root_pane;
        if result.already_open {
            let current = herdr::pane_get(&self.herdr, &pane.pane_id)
                .await
                .map_err(herdr_fail)
                .map_err(left)?;
            if current.terminal_id != pane.terminal_id || current.agent.is_some() {
                return Err(left(invalid("the worktree root pane is occupied")));
            }
        } else if pane.agent.is_some() {
            return Err(left(invalid("the worktree root pane is occupied")));
        }
        self.finish_task(&p, pane, "left open", !result.already_open, auth)
            .await
            .map_err(left)
    }

    async fn finish_task(
        &self,
        p: &TaskNewParams,
        pane: &PaneInfo,
        left_state: &str,
        retry_busy: bool,
        auth: &Authorized,
    ) -> Reply {
        let (Ok(workspace_id), Ok(terminal_id)) = (
            WorkspaceId::new(pane.workspace_id.clone()),
            TerminalId::new(pane.terminal_id.clone()),
        ) else {
            return fail(ErrorCode::Internal, "herdr returned an invalid id");
        };
        let left_open = |(code, message): Fail| {
            (
                code,
                format!(
                    "workspace {} was {left_state}: {message}",
                    workspace_id.as_str()
                ),
            )
        };
        let name = self
            .start_agent(
                pane,
                &p.agent,
                p.resume.as_ref(),
                p.ultracode,
                retry_busy,
                auth,
            )
            .await
            .map_err(left_open)?;
        let current = herdr::agent_get(&self.herdr, &name)
            .await
            .map_err(|e| left_open(herdr_fail(e)))?;
        self.started_check(&current, pane, &name, &p.agent)
            .await
            .map_err(left_open)?;
        if p.ultracode {
            self.ultracode_shown(&pane.pane_id)
                .await
                .map_err(left_open)?;
        }
        if let Some(text) = &p.prompt {
            // herdr may not rule another kind's startup prompt `blocked`, and the prompt's
            // Enter would answer it.
            if p.agent.as_str() != "claude" {
                let screen = herdr::detection_text(&self.herdr, &pane.pane_id).await.ok();
                if screen.as_deref().is_none_or(prompt::shows_dialog) {
                    let message = startup_prompt(&p.agent, screen.as_deref());
                    return Err(left_open((ErrorCode::AgentBlocked, message.to_owned())));
                }
            }
            authorized(auth).map_err(left_open)?;
            herdr::agent_prompt(&self.herdr, &name, text.as_str())
                .await
                .map_err(|e| left_open(herdr_fail(e)))?;
        }
        Ok(Response::TaskStarted {
            workspace_id,
            terminal_id,
        })
    }

    async fn ultracode_shown(&self, pane_id: &str) -> Result<(), Fail> {
        let deadline = tokio::time::Instant::now() + SCREEN_SETTLE;
        loop {
            let screen = herdr::detection_text(&self.herdr, pane_id)
                .await
                .map_err(herdr_fail)?;
            if prompt::ultracode_on(&screen) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return fail(
                    ErrorCode::AgentNotReady,
                    "ultracode could not be enabled; nothing was sent",
                );
            }
            tokio::time::sleep(SCREEN_POLL).await;
        }
    }

    // Readiness rules follow herdr's own `herdr agent start` CLI (0.9.5). The busy retry
    // gets 30 s instead of the CLI's 2 s and, unlike the CLI, does not require the shell
    // in the foreground: the CLI targets any pane, this one was just created.
    async fn start_agent(
        &self,
        pane: &PaneInfo,
        kind: &AgentKind,
        resume: Option<&SessionId>,
        ultracode: bool,
        retry_busy: bool,
        auth: &Authorized,
    ) -> Result<String, Fail> {
        let resume = resume.map(SessionId::as_str);
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        let name = agent_name()?;
        let started = loop {
            authorized(auth)?;
            match herdr::agent_start(
                &self.herdr,
                &name,
                kind.as_str(),
                &pane.pane_id,
                resume,
                ultracode,
            )
            .await
            {
                Ok(a) => break a,
                // Retried while the pane still holds the terminal created for this task: what
                // runs there is the shell's startup (rc files often run helpers in the
                // foreground), as nothing else has used the pane yet.
                Err(herdr::Error::Herdr { code, .. })
                    if retry_busy
                        && code == "agent_pane_busy"
                        && tokio::time::Instant::now() < deadline
                        && self.still_new(pane).await =>
                {
                    tokio::time::sleep(START_POLL).await;
                }
                Err(e) => return Err(herdr_fail(e)),
            }
        };
        if started.terminal_id != pane.terminal_id {
            return fail(
                ErrorCode::AgentNotReady,
                "the new pane no longer hosts the started agent",
            );
        }
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(START_POLL).await;
            let a = match herdr::agent_get(&self.herdr, &name).await {
                Ok(a) => a,
                Err(_) => match herdr::agent_get(&self.herdr, &pane.pane_id).await {
                    Ok(a) => a,
                    Err(_) => continue,
                },
            };
            self.started_check(&a, pane, &name, kind).await?;
            match (flock::status(&a.agent_status), a.interactive_ready) {
                (AgentStatus::Idle | AgentStatus::Done, true) => return Ok(name),
                (AgentStatus::Unknown, true) if kind.as_str() == "codex" => return Ok(name),
                (AgentStatus::Idle | AgentStatus::Done, false) if !a.launch_pending => {
                    return fail(
                        ErrorCode::AgentNotReady,
                        "agent exited before becoming interactive",
                    );
                }
                _ => {}
            }
        }
        fail(
            ErrorCode::AgentNotReady,
            "agent did not become ready within 30 s",
        )
    }

    /// The started agent must still be the one in the new pane, and is never answered on
    /// the user's behalf: a startup question such as Claude Code's folder trust prompt
    /// is left to its approval, and another kind's to the terminal.
    async fn started_check(
        &self,
        a: &AgentInfo,
        pane: &PaneInfo,
        name: &str,
        kind: &AgentKind,
    ) -> Result<(), Fail> {
        if a.terminal_id != pane.terminal_id
            || a.name.as_deref() != Some(name)
            || a.agent.as_deref().is_some_and(|k| k != kind.as_str())
        {
            return fail(
                ErrorCode::AgentNotReady,
                "the new pane no longer hosts the started agent",
            );
        }
        if flock::status(&a.agent_status) != AgentStatus::Blocked {
            return Ok(());
        }
        let message = if kind.as_str() == "codex" {
            let screen = herdr::detection_text(&self.herdr, &pane.pane_id).await.ok();
            startup_prompt(kind, screen.as_deref())
        } else if kind.as_str() != "claude" {
            STARTUP_PROMPT
        } else if herdr::detection_text(&self.herdr, &pane.pane_id)
            .await
            .is_ok_and(|t| prompt::Menu::parse(&t).is_some_and(|m| m.is_trust_prompt()))
        {
            "the agent asks whether to trust this folder; answer it through its approval"
        } else {
            "the agent is blocked on a startup prompt; answer it through its approval"
        };
        fail(ErrorCode::AgentBlocked, message)
    }

    async fn still_new(&self, pane: &PaneInfo) -> bool {
        herdr::pane_get(&self.herdr, &pane.pane_id)
            .await
            .is_ok_and(|now| now.terminal_id == pane.terminal_id && now.agent.is_none())
    }

    pub async fn workspace_close(&self, p: WorkspaceCloseParams, auth: &Authorized) -> Reply {
        if !p.confirm {
            return fail(ErrorCode::ConfirmRequired, "confirm must be true");
        }
        authorized(auth)?;
        herdr::workspace_close(&self.herdr, p.workspace_id.as_str())
            .await
            .map_err(herdr_fail)?;
        Ok(Response::Ok)
    }

    pub async fn pane_close(&self, p: PaneCloseParams, auth: &Authorized) -> Reply {
        if !p.confirm {
            return fail(ErrorCode::ConfirmRequired, "confirm must be true");
        }
        let pane = self.find_pane(&p.terminal_id).await?;
        authorized(auth)?;
        herdr::pane_close(&self.herdr, &pane.pane_id)
            .await
            .map_err(herdr_fail)?;
        Ok(Response::Ok)
    }

    pub async fn task_archive(&self, p: TaskArchiveParams, auth: &Authorized) -> Reply {
        if !p.confirm {
            return fail(ErrorCode::ConfirmRequired, "confirm must be true");
        }
        self.archive(&p.terminal_id, auth, None).await
    }

    /// The automatic archive of `agent`'s task in `root`, whose pull request `number` a fresh
    /// or kept read found merged. Each condition is read fresh, `gh pr view` again, and
    /// `archive_merged` checks them once more; `None` when one fails before the archive starts.
    pub async fn archive_if_merged(
        &self,
        gh: &str,
        agent: &AgentInfo,
        root: &Cwd,
        number: u64,
        quiet: &Quiet<'_>,
        auth: &Authorized,
    ) -> Option<Reply> {
        if !auth() {
            return None;
        }
        let terminal_id = TerminalId::new(agent.terminal_id.clone()).ok()?;
        let snapshot = herdr::session_snapshot(&self.herdr).await.ok()?;
        if !worktree_settled(&snapshot, terminal_id.as_str(), quiet)
            || !crate::git::clean(root).await.unwrap_or(false)
        {
            return None;
        }
        let (merged, head) = crate::pulls::merged_head(gh, root).await?;
        if merged != number || !crate::git::contains(root, &head).await.unwrap_or(false) {
            return None;
        }
        Some(
            self.archive_merged(&terminal_id, root, &head, quiet, auth)
                .await,
        )
    }

    /// Only a linked worktree whose panes are all quiet idle or done agents, and only while
    /// its checkout is `checkout`, clean and inside `head`, checked again right before the
    /// removal. `gh poi` is left to a manual archive: it reaches other worktrees.
    pub async fn archive_merged(
        &self,
        terminal_id: &TerminalId,
        checkout: &Cwd,
        head: &str,
        quiet: &Quiet<'_>,
        auth: &Authorized,
    ) -> Reply {
        self.archive(terminal_id, auth, Some((checkout, head, quiet)))
            .await
    }

    async fn archive(
        &self,
        terminal_id: &TerminalId,
        auth: &Authorized,
        merged: Option<(&Cwd, &str, &Quiet<'_>)>,
    ) -> Reply {
        authorized(auth)?;
        let snapshot = herdr::session_snapshot(&self.herdr)
            .await
            .map_err(herdr_fail)?;
        let pane = snapshot
            .panes
            .iter()
            .find(|pane| pane.terminal_id == terminal_id.as_str())
            .ok_or_else(|| (ErrorCode::NotFound, "no such terminal".to_owned()))?;
        let workspace = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == pane.workspace_id)
            .ok_or_else(|| (ErrorCode::NotFound, "no such workspace".to_owned()))?;
        let linked = workspace
            .worktree
            .as_ref()
            .filter(|worktree| worktree.is_linked_worktree);
        if let Some((.., quiet)) = merged
            && !worktree_settled(&snapshot, terminal_id.as_str(), quiet)
        {
            return fail(
                ErrorCode::AgentNotReady,
                "only an idle worktree task is archived automatically",
            );
        }
        let (repo, closed) = if let Some(worktree) = linked {
            let checkout = resolve_cwd(&worktree.checkout_path, &self.roots).map_err(invalid)?;
            if checkout.as_str() != worktree.checkout_path {
                return Err(invalid("worktree checkout path changed"));
            }
            let repo = resolve_cwd(&worktree.repo_root, &self.roots).map_err(invalid)?;
            let listed = herdr::worktree_list(&self.herdr, checkout.as_str())
                .await
                .map_err(herdr_fail)?;
            if !listed.worktrees.iter().any(|item| {
                item.path == checkout.as_str()
                    && item.open_workspace_id.as_deref() == Some(&pane.workspace_id)
                    && item.is_linked_worktree
                    && !item.is_bare
            }) {
                return Err(invalid("worktree is no longer linked to this workspace"));
            }
            authorized(auth)?;
            if let Some((expected, head, _)) = merged
                && !(checkout == *expected
                    && crate::git::clean(&checkout).await?
                    && crate::git::contains(&checkout, head).await?)
            {
                return Err(invalid(
                    "the checkout has changes or commits outside the pull request",
                ));
            }
            let closed = match herdr::worktree_remove(&self.herdr, &pane.workspace_id).await {
                Ok(removed) if removed.path == checkout.as_str() => {
                    format!(
                        "Removed worktree checkout {} and closed its workspace.",
                        checkout.as_str()
                    )
                }
                Ok(removed) => {
                    return Ok(Response::TaskArchived {
                        message: format!(
                            "Herdr removed a checkout but reported a different path; branch cleanup was skipped. Expected {}, got {}.",
                            checkout.as_str(),
                            removed.path
                        ),
                    });
                }
                Err(error) => {
                    if !checkout_removed(&repo, &checkout) {
                        let (code, message) = herdr_fail(error);
                        return Err((
                            code,
                            format!(
                                "Worktree removal could not be verified ({message}); gh poi was skipped."
                            ),
                        ));
                    }
                    let workspace_closed =
                        herdr::session_snapshot(&self.herdr)
                            .await
                            .is_ok_and(|snapshot| {
                                !snapshot
                                    .workspaces
                                    .iter()
                                    .any(|item| item.workspace_id == pane.workspace_id)
                            });
                    if workspace_closed {
                        format!(
                            "Removed worktree checkout {} and closed its workspace.",
                            checkout.as_str()
                        )
                    } else {
                        format!(
                            "Removed worktree checkout {}; workspace closure could not be verified.",
                            checkout.as_str()
                        )
                    }
                }
            };
            (Some(repo), closed)
        } else {
            let repo = pane
                .cwd
                .as_deref()
                .and_then(|cwd| resolve_cwd(cwd, &self.roots).ok());
            let keep_workspace = workspace.worktree.as_ref().is_some_and(|parent| {
                !parent.is_linked_worktree
                    && snapshot.workspaces.iter().any(|item| {
                        item.worktree.as_ref().is_some_and(|child| {
                            child.is_linked_worktree && child.repo_root == parent.repo_root
                        })
                    })
                    && snapshot
                        .panes
                        .iter()
                        .filter(|item| item.workspace_id == pane.workspace_id)
                        .count()
                        == 1
            });
            if keep_workspace {
                let cwd = repo.as_ref().ok_or_else(|| {
                    invalid("Archive could not preserve the worktree group: pane folder is outside task roots or unavailable.")
                })?;
                authorized(auth)?;
                herdr::tab_create(&self.herdr, &pane.workspace_id, cwd.as_str())
                    .await
                    .map_err(|error| {
                        let (code, message) = herdr_fail(error);
                        (code, format!("Archive could not create a shell to preserve the worktree group ({message}); pane closure and gh poi were skipped."))
                    })?;
            }
            authorized(auth)?;
            herdr::pane_close(&self.herdr, &pane.pane_id)
                .await
                .map_err(|error| {
                    let (code, message) = herdr_fail(error);
                    (
                        code,
                        format!(
                            "Archive could not close the pane ({message}); gh poi was skipped."
                        ),
                    )
                })?;
            let mut message = "Closed pane. Its folder was left in place.".to_owned();
            if keep_workspace {
                message.push_str(
                    " Kept a shell in its workspace to preserve linked worktree sessions.",
                );
            }
            (repo, message)
        };
        let cleanup = if merged.is_some() {
            "gh poi is left to a manual archive.".to_owned()
        } else if let Some(repo) = repo.and_then(|repo| git_repository(&repo, &self.roots)) {
            if !auth() {
                "Authorization ended; gh poi was skipped.".to_owned()
            } else {
                match run_poi(&repo).await {
                    Ok(()) => format!("gh poi completed in {}.", repo.as_str()),
                    Err(reason) => {
                        format!(
                            "gh poi did not complete ({reason}); branch cleanup may be partial."
                        )
                    }
                }
            }
        } else {
            "No repository was available for gh poi; branch cleanup was skipped.".to_owned()
        };
        Ok(Response::TaskArchived {
            message: format!("{closed} {cleanup}"),
        })
    }
}

/// Whether an agent runs nothing in the background, such as Claude Code subagents.
pub type Quiet<'a> = dyn Fn(&AgentInfo) -> bool + Send + Sync + 'a;

/// The terminal's workspace is a linked worktree whose every pane is an idle or done agent
/// that is `quiet`.
pub fn worktree_settled(
    snapshot: &herdr::SessionSnapshot,
    terminal_id: &str,
    quiet: &Quiet<'_>,
) -> bool {
    let Some(pane) = snapshot.panes.iter().find(|p| p.terminal_id == terminal_id) else {
        return false;
    };
    let linked = snapshot.workspaces.iter().any(|w| {
        w.workspace_id == pane.workspace_id
            && w.worktree.as_ref().is_some_and(|t| t.is_linked_worktree)
    });
    linked
        && snapshot
            .panes
            .iter()
            .filter(|p| p.workspace_id == pane.workspace_id)
            .all(|p| {
                snapshot.agents.iter().any(|a| {
                    a.terminal_id == p.terminal_id
                        && matches!(
                            flock::status(&a.agent_status),
                            AgentStatus::Idle | AgentStatus::Done
                        )
                        && quiet(a)
                })
            })
}

fn git_repository(cwd: &Cwd, roots: &[PathBuf]) -> Option<Cwd> {
    let output = std::process::Command::new("git")
        .args(["-C", cwd.as_str(), "rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = std::str::from_utf8(&output.stdout).ok()?.trim();
    resolve_cwd(root, roots).ok()
}

fn checkout_removed(repo: &Cwd, checkout: &Cwd) -> bool {
    if !matches!(std::fs::symlink_metadata(checkout.as_str()), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    {
        return false;
    }
    let output = std::process::Command::new("git")
        .args(["-C", repo.as_str(), "worktree", "list", "--porcelain"])
        .output();
    let Ok(output) = output else { return false };
    output.status.success()
        && std::str::from_utf8(&output.stdout).is_ok_and(|text| {
            !text
                .lines()
                .any(|line| line.strip_prefix("worktree ") == Some(checkout.as_str()))
        })
}

pub(crate) fn gh() -> Option<String> {
    ["/opt/homebrew/bin/gh", "/usr/local/bin/gh", "/usr/bin/gh"]
        .into_iter()
        .map(str::to_owned)
        .chain(crate::config::home_dir().ok().and_then(|home| {
            home.join(".local/bin/gh")
                .into_os_string()
                .into_string()
                .ok()
        }))
        .find(|path| Path::new(path).is_file())
}

pub(crate) fn tool_path() -> String {
    format!(
        "/opt/homebrew/bin:/usr/local/bin:{}",
        std::env::var("PATH").unwrap_or_default()
    )
}

/// `owner/repo` goes through `gh` when it is installed, else to GitHub over HTTPS; never
/// to git as typed, where it would be a local path. Only HTTPS and SSH transports are
/// allowed, also after a `url.<base>.insteadOf` rewrite, and nothing can prompt.
fn clone_command(source: &CloneSource, target: &Cwd, gh: Option<&str>) -> Command {
    let command = match (source.github_repo(), gh) {
        (Some(repo), Some(gh)) => {
            let mut command = Command::new(gh);
            command.args(["repo", "clone", "--", repo, target.as_str()]);
            command
        }
        (Some(repo), None) => {
            let mut command = Command::new("git");
            let url = format!("https://github.com/{repo}.git");
            command.args(["clone", "--", &url, target.as_str()]);
            command
        }
        (None, _) => {
            let mut command = Command::new("git");
            command.args(["clone", "--", source.as_str(), target.as_str()]);
            command
        }
    };
    network(command)
}

fn network(mut command: Command) -> Command {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("PATH", tool_path())
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", CLONE_PROTOCOLS.len().to_string());
    for (i, (key, value)) in CLONE_PROTOCOLS.iter().enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{i}"), key)
            .env(format!("GIT_CONFIG_VALUE_{i}"), value);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    command
}

fn valid_branch(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() <= 255
        && branch
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        && !branch.contains("..")
        && !branch.starts_with('-')
        && !branch.ends_with('/')
}

async fn pull_request_head(checkout: &Cwd, number: u64) -> Result<pulls::Head, Fail> {
    let Some(gh) = gh() else {
        return fail(
            ErrorCode::InvalidParams,
            "gh is not installed on the machine",
        );
    };
    let head = pulls::head(&gh, checkout, number)
        .await
        .map_err(|message| invalid(&message))?;
    checked_branch(checkout, &head.branch).await?;
    Ok(head)
}

/// gh's head branch name is not trusted: it must pass the rules for a typed one and git's.
async fn checked_branch(checkout: &Cwd, branch: &str) -> Result<(), Fail> {
    if !valid_branch(branch)
        || git::line(checkout, &["check-ref-format", "--branch", branch])
            .await?
            .as_deref()
            != Some(branch)
    {
        return fail(
            ErrorCode::InvalidParams,
            "the pull request's branch name is not supported",
        );
    }
    Ok(())
}

/// gh, the checks and the fetch share one budget, within the phone's `TASK_CLONE_TIMEOUT`.
async fn within<T>(
    deadline: tokio::time::Instant,
    work: impl Future<Output = Result<T, Fail>>,
) -> Result<T, Fail> {
    tokio::time::timeout_at(deadline, work)
        .await
        .unwrap_or_else(|_| {
            fail(
                ErrorCode::InvalidParams,
                format!(
                    "reading the pull request timed out after {} s",
                    CLONE_TIMEOUT.as_secs()
                ),
            )
        })
}

async fn fetch_pull_request(
    checkout: &Cwd,
    number: u64,
    head: &pulls::Head,
    deadline: tokio::time::Instant,
) -> Result<(), Fail> {
    let branch = head.branch.as_str();
    let mut command = Command::new("git");
    command.current_dir(checkout.as_str()).args([
        "fetch",
        "--no-tags",
        "--no-recurse-submodules",
        "--no-write-fetch-head",
        "origin",
        "--",
        &format!("+refs/heads/{branch}:refs/remotes/origin/{branch}"),
    ]);
    run_clone(network(command), deadline)
        .await
        .map_err(|(_, message)| {
            invalid(&format!("fetching {branch} from origin failed: {message}"))
        })?;
    within(deadline, track_pull_request(checkout, number, head)).await
}

/// `origin/<branch>` and any local branch of that name must be the pull request's head, so
/// the agent never works on other code. A missing local branch is created tracking it.
async fn track_pull_request(checkout: &Cwd, number: u64, head: &pulls::Head) -> Result<(), Fail> {
    let branch = head.branch.as_str();
    let remote = format!("refs/remotes/origin/{branch}");
    let commit = |name: String| async move {
        let args = [
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &name,
        ];
        git::line(checkout, &args).await
    };
    if commit(format!("{remote}^{{commit}}")).await?.as_deref() != Some(head.oid.as_str()) {
        return fail(
            ErrorCode::InvalidParams,
            format!("origin's {branch} is not the head of pull request #{number}; try again"),
        );
    }
    match commit(format!("refs/heads/{branch}")).await? {
        Some(local) if local == head.oid => return Ok(()),
        Some(_) => {
            return fail(
                ErrorCode::InvalidParams,
                format!(
                    "a local branch {branch} exists at another commit than pull request #{number}; open it with Open or delete it"
                ),
            );
        }
        None => {}
    }
    if git::line(checkout, &["branch", "--track", "--", branch, &remote])
        .await?
        .is_none()
    {
        return fail(
            ErrorCode::InvalidParams,
            format!("could not create branch {branch}"),
        );
    }
    Ok(())
}

async fn clone_repository(source: &CloneSource, target: &Cwd) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + CLONE_TIMEOUT;
    let gh = gh().filter(|_| source.github_repo().is_some());
    let gh = gh.as_deref();
    let mut result = run_clone(clone_command(source, target, gh), deadline).await;
    if gh.is_some() && gh_signed_out(&result, target) {
        result = run_clone(clone_command(source, target, None), deadline)
            .await
            .map_err(|(code, message)| (code, format!("{message}; gh is not logged in")));
    }
    result.map_err(|(_, message)| message)
}

/// gh exits 4 when it has no login, before running anything; git over HTTPS still clones a
/// public repository.
fn gh_signed_out(result: &Result<(), (Option<i32>, String)>, target: &Cwd) -> bool {
    matches!(result, Err((Some(4), _)))
        && std::fs::read_dir(target.as_str()).is_ok_and(|mut entries| entries.next().is_none())
}

async fn run_clone(
    mut command: Command,
    deadline: tokio::time::Instant,
) -> Result<(), (Option<i32>, String)> {
    let mut child = command
        .spawn()
        .map_err(|_| (None, "git or gh is unavailable".to_owned()))?;
    let pid = child.id();
    let mut stderr = child.stderr.take();
    let run = async {
        let mut tail = Vec::new();
        let mut chunk = [0u8; 4096];
        while let Some(pipe) = stderr.as_mut() {
            match pipe.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => tail.extend_from_slice(&chunk[..n]),
            }
            tail.drain(..tail.len().saturating_sub(MAX_CLONE_ERROR_BYTES));
        }
        (child.wait().await, tail)
    };
    let Ok((status, tail)) = tokio::time::timeout_at(deadline, run).await else {
        if let Some(group) = pid.and_then(|pid| rustix::process::Pid::from_raw(pid as i32)) {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
        return Err((
            None,
            format!("timed out after {} s", CLONE_TIMEOUT.as_secs()),
        ));
    };
    let code = match status {
        Ok(status) if status.success() => return Ok(()),
        Ok(status) => status.code(),
        Err(_) => None,
    };
    let text: String = String::from_utf8_lossy(&tail)
        .chars()
        .filter(|&c| c == '\n' || !(c.is_control() || protocol::is_format(c)))
        .collect();
    let text = text.trim();
    Err((
        code,
        if text.is_empty() {
            "git exited with an error".to_owned()
        } else {
            text.to_owned()
        },
    ))
}

async fn run_poi(repo: &Cwd) -> Result<(), &'static str> {
    let mut command = crate::pulls::gh_command(gh().as_deref().unwrap_or("gh"));
    command.arg("poi").current_dir(repo.as_str());
    let output = timeout(Duration::from_secs(60), command.output())
        .await
        .map_err(|_| "timed out")?
        .map_err(|_| "unavailable")?;
    if output.status.success() {
        Ok(())
    } else {
        Err("command failed")
    }
}

/// `listed` comes from `agent.list`, `current` from `agent.get` just before the write.
pub fn check_ready(listed: &AgentInfo, current: &AgentInfo) -> Result<(), Fail> {
    let session = |a: &AgentInfo| a.agent_session.as_ref().map(|s| s.value.clone());
    let same = current.terminal_id == listed.terminal_id
        && current.agent == listed.agent
        && (session(listed).is_none() || session(current) == session(listed));
    if !same {
        return fail(
            ErrorCode::AgentNotReady,
            "the terminal no longer hosts the same agent",
        );
    }
    if flock::status(&current.agent_status) == AgentStatus::Blocked {
        return fail(ErrorCode::AgentBlocked, BLOCKED);
    }
    if current.agent.is_none() || current.launch_pending {
        return fail(ErrorCode::AgentNotReady, "agent is not ready");
    }
    Ok(())
}

fn agent_name() -> Result<String, Fail> {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).map_err(|_| (ErrorCode::Internal, "no randomness".to_owned()))?;
    Ok(format!("collie-{:08x}", u32::from_be_bytes(b)))
}

/// Never shorter than `WATCH_EVERY`, whatever the phone asks.
fn watch_period(low_data: bool, unchanged: Duration) -> Duration {
    if low_data || unchanged >= WATCH_QUIET {
        WATCH_IDLE
    } else {
        WATCH_EVERY
    }
}

fn source_name(source: ReadSource) -> &'static str {
    match source {
        ReadSource::Visible => "visible",
        // Logical lines: a row soft-wrapped at the Mac pane width arrives whole, so the
        // phone can wrap it at its own width. `lines` still counts rendered rows.
        ReadSource::Recent => "recent_unwrapped",
    }
}

fn terminal_read(
    terminal_id: TerminalId,
    source: ReadSource,
    read: herdr::PaneRead,
) -> TerminalRead {
    TerminalRead {
        terminal_id,
        source,
        ansi: sanitize_ansi(&read.text),
        truncated: read.truncated,
        wraps: Vec::new(),
        splits: Vec::new(),
        copilot_scrollbar: false,
    }
}

/// Canonicalizing first resolves `..` and every symlink, so the root check sees the
/// directory herdr will really start in.
pub fn resolve_cwd(cwd: &str, roots: &[PathBuf]) -> Result<Cwd, &'static str> {
    let path = std::fs::canonicalize(cwd).map_err(|_| "cwd does not exist")?;
    if !path.is_dir() {
        return Err("cwd is not a directory");
    }
    if !roots.iter().any(|r| path.starts_with(r)) {
        return Err("cwd is outside the allowed roots");
    }
    path.to_str()
        .and_then(|s| Cwd::new(s).ok())
        .ok_or("cwd is not a valid path")
}

fn invalid(message: &str) -> Fail {
    (ErrorCode::InvalidParams, message.to_owned())
}

fn io_fail(e: rustix::io::Errno) -> Fail {
    (
        ErrorCode::InvalidParams,
        format!("folder cannot be opened: {}", std::io::Error::from(e)),
    )
}

fn open_under_root(cwd: &str, roots: &[PathBuf]) -> Result<(OwnedFd, Cwd), Fail> {
    let resolved = resolve_cwd(cwd, roots).map_err(invalid)?;
    let path = Path::new(resolved.as_str());
    let Some(root) = roots.iter().find(|r| path.starts_with(r)) else {
        return Err(invalid("cwd is outside the allowed roots"));
    };
    let dir = open_beneath(root, path)?;
    Ok((dir, resolved))
}

fn worktree_parent(checkout: &Cwd, roots: &[PathBuf], auth: &Authorized) -> Result<Cwd, Fail> {
    let (dir, checkout) = open_under_root(checkout.as_str(), roots)?;
    authorized(auth)?;
    match rustix::fs::mkdirat(&dir, ".worktree", Mode::from_raw_mode(0o755)) {
        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
        Err(e) => return Err(io_fail(e)),
    }
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let child = rustix::fs::openat(&dir, ".worktree", flags, Mode::empty()).map_err(io_fail)?;
    let path = Path::new(checkout.as_str()).join(".worktree");
    let path = path
        .to_str()
        .ok_or_else(|| invalid("invalid checkout path"))?;
    let resolved = resolve_cwd(path, roots).map_err(invalid)?;
    let made = rustix::fs::fstat(&child).map_err(io_fail)?;
    let now = rustix::fs::stat(resolved.as_str()).map_err(io_fail)?;
    if (made.st_dev, made.st_ino) != (now.st_dev, now.st_ino) || resolved.as_str() != path {
        return Err(invalid("worktree directory changed"));
    }
    Ok(resolved)
}

/// Opens the canonical `path` one component at a time from `root` without following a
/// symlink, so a folder swapped for a link after the check fails instead of leading
/// outside the roots. A hidden folder below the root (dot or `UF_HIDDEN`) is refused, so
/// `~/.ssh` or `~/Library` is neither listed nor created in.
fn open_beneath(root: &Path, path: &Path) -> Result<OwnedFd, Fail> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let rel = path
        .strip_prefix(root)
        .map_err(|_| io_fail(rustix::io::Errno::INVAL))?;
    let mut dir = rustix::fs::open(root, flags, Mode::empty()).map_err(io_fail)?;
    for part in rel.components() {
        let Component::Normal(part) = part else {
            return Err(io_fail(rustix::io::Errno::INVAL));
        };
        if part.as_encoded_bytes().starts_with(b".") {
            return Err(invalid("folder is hidden"));
        }
        dir = rustix::fs::openat(&dir, part, flags, Mode::empty()).map_err(io_fail)?;
        #[cfg(target_os = "macos")]
        if rustix::fs::fstat(&dir).map_err(io_fail)?.st_flags & UF_HIDDEN != 0 {
            return Err(invalid("folder is hidden"));
        }
    }
    Ok(dir)
}

/// Names only, of directories and of symlinks to a directory inside a root: no hidden
/// entry, file or other name leaves the machine, and nothing inside them is read.
fn list_folders(path: &str, roots: &[PathBuf]) -> Reply {
    let (dir, path) = open_under_root(path, roots)?;
    let mut folders = Vec::new();
    let mut truncated = false;
    for (scanned, entry) in rustix::fs::Dir::read_from(&dir)
        .map_err(io_fail)?
        .enumerate()
    {
        if scanned == MAX_FOLDERS_SCANNED {
            truncated = true;
            break;
        }
        let entry = entry.map_err(io_fail)?;
        let Some(name) = entry
            .file_name()
            .to_str()
            .ok()
            .and_then(|n| FolderName::new(n).ok())
        else {
            continue;
        };
        if is_listed_folder(&dir, &path, &name, roots) {
            folders.push(name);
        }
    }
    folders.sort_by_cached_key(|n| n.as_str().to_lowercase());
    if folders.len() > MAX_FOLDERS {
        folders.truncate(MAX_FOLDERS);
        truncated = true;
    }
    Ok(Response::TaskFolders {
        path,
        folders,
        truncated,
    })
}

fn is_listed_folder(dir: &OwnedFd, path: &Cwd, name: &FolderName, roots: &[PathBuf]) -> bool {
    let Ok(st) = rustix::fs::statat(dir, name.as_str(), AtFlags::SYMLINK_NOFOLLOW) else {
        return false;
    };
    #[cfg(target_os = "macos")]
    if st.st_flags & UF_HIDDEN != 0 {
        return false;
    }
    match FileType::from_raw_mode(st.st_mode) {
        FileType::Directory => true,
        FileType::Symlink => Path::new(path.as_str())
            .join(name.as_str())
            .to_str()
            .is_some_and(|p| resolve_cwd(p, roots).is_ok()),
        _ => false,
    }
}

/// `mkdirat` makes exactly one entry inside the opened parent and never follows a link
/// at that name: an existing one, link or not, is `EEXIST`.
fn create_folder(
    parent: &str,
    name: &FolderName,
    roots: &[PathBuf],
    auth: &Authorized,
) -> Result<Cwd, Fail> {
    let (dir, parent) = open_under_root(parent, roots)?;
    // macOS PATH_MAX (1024) counts the NUL, so a longer path could be made but never resolved.
    if parent.as_str().len() + 1 + name.as_str().len() >= limits::MAX_CWD_BYTES {
        return Err(invalid("folder path is too long"));
    }
    authorized(auth)?;
    rustix::fs::mkdirat(&dir, name.as_str(), Mode::from_raw_mode(0o755)).map_err(|e| {
        if e == rustix::io::Errno::EXIST {
            invalid("folder already exists")
        } else {
            (
                ErrorCode::InvalidParams,
                format!("folder cannot be created: {}", std::io::Error::from(e)),
            )
        }
    })?;
    let made = rustix::fs::statat(&dir, name.as_str(), AtFlags::SYMLINK_NOFOLLOW);
    let created = Path::new(parent.as_str()).join(name.as_str());
    let created = created.to_str().unwrap_or_default();
    let same = |cwd: &Cwd| {
        let (Ok(made), Ok(now)) = (&made, rustix::fs::stat(cwd.as_str())) else {
            return false;
        };
        (made.st_dev, made.st_ino) == (now.st_dev, now.st_ino)
    };
    match resolve_cwd(created, roots) {
        Ok(cwd) if same(&cwd) => Ok(cwd),
        _ => fail(
            ErrorCode::Internal,
            format!("folder {created} was created and left in place: it changed right after"),
        ),
    }
}

/// Keeps printable text, CR, LF, TAB and SGR (`ESC [ digits ; : m`) only. The phone renders
/// this in a real terminal emulator, so every other control, escape or string sequence
/// (cursor moves, OSC 52 clipboard writes, OSC 8 links, titles, DCS, device queries), in
/// 7-bit or C1 form, is dropped. SGR is re-emitted from its parsed parameters, so the
/// output never carries an ESC that does not start a plain SGR.
pub fn sanitize_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' | '\r' | '\t' => out.push(c),
            '\u{1b}' => match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    if let Some(params) = csi(&mut chars) {
                        out.push_str("\u{1b}[");
                        out.push_str(&params);
                        out.push('m');
                    }
                }
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    chars.next();
                    skip_string(&mut chars);
                }
                Some('\u{20}'..='\u{2f}') => {
                    while chars
                        .next_if(|c| matches!(c, '\u{20}'..='\u{2f}'))
                        .is_some()
                    {}
                    chars.next_if(|c| matches!(c, '\u{30}'..='\u{7e}'));
                }
                Some('\u{30}'..='\u{7e}') => {
                    chars.next();
                }
                _ => {}
            },
            '\u{9b}' => {
                csi(&mut chars);
            }
            '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => skip_string(&mut chars),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Consumes one CSI sequence and returns its parameters if it is a plain SGR. A control or
/// non-ASCII character aborts the sequence and is left for the caller.
fn csi(chars: &mut Peekable<Chars<'_>>) -> Option<String> {
    let mut params = String::new();
    let mut sgr = true;
    loop {
        let c = *chars.peek()?;
        match c {
            '0'..='9' | ';' | ':' if params.len() < MAX_SGR_PARAMS => params.push(c),
            '\u{20}'..='\u{3f}' => sgr = false,
            '\u{40}'..='\u{7e}' => {
                chars.next();
                return (sgr && c == 'm').then_some(params);
            }
            _ => return None,
        }
        chars.next();
    }
}

fn skip_string(chars: &mut Peekable<Chars<'_>>) {
    while let Some(c) = chars.next() {
        match c {
            '\u{7}' | '\u{9c}' => return,
            '\u{1b}' if chars.next_if_eq(&'\\').is_some() => return,
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// This call ran the operation.
    Ran,
    /// The outcome of an earlier call with the same `op_id`.
    Replayed,
    /// Nothing ran: the `op_id` was already used for a different request.
    Refused,
}

struct OpEntry {
    op_id: OpId,
    fingerprint: u64,
    at: Instant,
    outcome: watch::Receiver<Option<Reply>>,
}

/// Outcomes of `op_id` mutations per peer `StableID`.
pub struct OpCache {
    cap: usize,
    ttl: Duration,
    peers: Mutex<HashMap<String, VecDeque<OpEntry>>>,
}

impl OpCache {
    pub fn new(cap: usize, ttl: Duration) -> Self {
        Self {
            cap,
            ttl,
            peers: Mutex::new(HashMap::new()),
        }
    }

    /// The operation runs detached, so a retry after a dropped connection waits for, or
    /// replays, the first attempt's outcome instead of running it again.
    pub async fn once<F>(
        &self,
        peer: &str,
        op_id: &OpId,
        fingerprint: u64,
        op: F,
    ) -> (Reply, Origin)
    where
        F: Future<Output = Reply> + Send + 'static,
    {
        let (mut rx, origin) = {
            let mut peers = crate::lock(&self.peers);
            let ops = peers.entry(peer.to_owned()).or_default();
            let now = Instant::now();
            ops.retain(|e| e.outcome.borrow().is_none() || now.duration_since(e.at) < self.ttl);
            match ops.iter().find(|e| e.op_id == *op_id) {
                Some(e) if e.fingerprint != fingerprint => {
                    return (
                        fail(
                            ErrorCode::InvalidParams,
                            "op_id was already used for a different request",
                        ),
                        Origin::Refused,
                    );
                }
                Some(e) => (e.outcome.clone(), Origin::Replayed),
                None => {
                    if ops.len() >= self.cap {
                        // An in-flight entry is never evicted: its retry must replay it.
                        let Some(done) = ops.iter().position(|e| e.outcome.borrow().is_some())
                        else {
                            return (
                                fail(ErrorCode::RateLimited, "too many operations in flight"),
                                Origin::Refused,
                            );
                        };
                        ops.remove(done);
                    }
                    let (tx, rx) = watch::channel(None);
                    ops.push_back(OpEntry {
                        op_id: op_id.clone(),
                        fingerprint,
                        at: now,
                        outcome: rx.clone(),
                    });
                    tokio::spawn(async move {
                        let _ = tx.send(Some(op.await));
                    });
                    (rx, Origin::Ran)
                }
            }
        };
        let reply = match rx.wait_for(Option::is_some).await {
            Ok(r) => r
                .clone()
                .unwrap_or_else(|| fail(ErrorCode::Internal, "operation outcome unknown")),
            Err(_) => fail(ErrorCode::Internal, "operation outcome unknown"),
        };
        (reply, origin)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn only_a_linked_worktree_of_idle_or_done_agents_is_settled() {
        let snapshot = |linked: bool, statuses: &[&str], shell: bool| {
            let mut panes: Vec<_> = (0..statuses.len())
                .map(|i| serde_json::json!({"pane_id": format!("w1:p{i}"), "terminal_id": format!("t{i}"), "workspace_id": "w1", "tab_id": "w1:t1"}))
                .collect();
            if shell {
                panes.push(serde_json::json!({"pane_id": "w1:p9", "terminal_id": "t9", "workspace_id": "w1", "tab_id": "w1:t1"}));
            }
            panes.push(serde_json::json!({"pane_id": "w2:p1", "terminal_id": "other", "workspace_id": "w2", "tab_id": "w2:t1"}));
            let agents: Vec<_> = statuses
                .iter()
                .enumerate()
                .map(|(i, status)| serde_json::json!({"terminal_id": format!("t{i}"), "workspace_id": "w1", "pane_id": format!("w1:p{i}"), "agent": "claude", "agent_status": status}))
                .chain([serde_json::json!({"terminal_id": "other", "workspace_id": "w2", "pane_id": "w2:p1", "agent": "claude", "agent_status": "working"})])
                .collect();
            serde_json::from_value::<herdr::SessionSnapshot>(serde_json::json!({
                "workspaces": [
                    {"workspace_id": "w1", "number": 1, "label": "a", "agent_status": "idle",
                     "worktree": {"repo_root": "/r", "checkout_path": "/r/.worktree/x", "is_linked_worktree": linked}},
                    {"workspace_id": "w2", "number": 2, "label": "b", "agent_status": "working"},
                ],
                "panes": panes,
                "agents": agents,
            }))
            .unwrap()
        };
        let quiet = |_: &AgentInfo| true;
        let settled = |s: &herdr::SessionSnapshot, t: &str| worktree_settled(s, t, &quiet);
        assert!(settled(&snapshot(true, &["idle"], false), "t0"));
        assert!(settled(&snapshot(true, &["done", "idle"], false), "t0"));
        assert!(!settled(&snapshot(false, &["idle"], false), "t0"));
        assert!(!settled(&snapshot(true, &["idle"], true), "t0"));
        for busy in ["working", "blocked", "unknown"] {
            assert!(
                !settled(&snapshot(true, &["idle", busy], false), "t0"),
                "{busy}"
            );
        }
        assert!(!settled(&snapshot(true, &["idle"], false), "missing"));
        assert!(!settled(&snapshot(true, &["idle"], false), "other"));
        let background = |a: &AgentInfo| a.terminal_id != "t1";
        let two = snapshot(true, &["idle", "done"], false);
        assert!(!worktree_settled(&two, "t0", &background));
        assert!(worktree_settled(
            &snapshot(true, &["idle"], false),
            "t0",
            &background
        ));
    }

    #[test]
    fn the_watch_slows_down_after_a_quiet_spell_or_in_low_data_mode() {
        assert_eq!(watch_period(false, Duration::ZERO), WATCH_EVERY);
        assert_eq!(
            watch_period(false, WATCH_QUIET - Duration::from_millis(1)),
            WATCH_EVERY
        );
        assert_eq!(watch_period(false, WATCH_QUIET), WATCH_IDLE);
        assert_eq!(watch_period(true, Duration::ZERO), WATCH_IDLE);
        assert_eq!(watch_period(true, Duration::MAX), WATCH_IDLE);
    }

    fn is_sgr_only(s: &str) -> bool {
        let mut rest = s;
        while let Some(i) = rest.find('\u{1b}') {
            let tail = &rest[i + 1..];
            let Some(params) = tail.strip_prefix('[') else {
                return false;
            };
            let end = params
                .find(|c: char| !matches!(c, '0'..='9' | ';' | ':'))
                .unwrap_or(params.len());
            if !params[end..].starts_with('m') {
                return false;
            }
            rest = &params[end + 1..];
        }
        !s.chars()
            .any(|c| c.is_control() && !matches!(c, '\u{1b}' | '\r' | '\n' | '\t'))
    }

    #[test]
    fn recent_reads_logical_lines() {
        assert_eq!(source_name(ReadSource::Recent), "recent_unwrapped");
        assert_eq!(source_name(ReadSource::Visible), "visible");
    }

    #[test]
    fn sgr_and_text_survive() {
        let input = "\u{1b}[0m\u{1b}[38;2;153;153;153mWrote \u{1b}[1m4\u{1b}[0m \u{1b}[38:5:4mx\u{1b}[m\r\n\tok ❯\u{a0}";
        assert_eq!(sanitize_ansi(input), input);
    }

    #[test]
    fn strips_everything_but_sgr() {
        let cases = [
            ("a\u{1b}[2Jb", "ab"),
            ("a\u{1b}[10;5Hb", "ab"),
            ("a\u{1b}[?25lb", "ab"),
            ("a\u{1b}[?1049hb", "ab"),
            ("a\u{1b}[>4;1mb", "ab"),
            ("a\u{1b}[1 mb", "ab"),
            ("a\u{1b}[6nb", "ab"),
            ("a\u{1b}]52;c;cm0gLXJmIH4=\u{7}b", "ab"),
            ("a\u{1b}]52;c;cm0gLXJmIH4=\u{1b}\\b", "ab"),
            (
                "a\u{1b}]8;;https://evil.example\u{1b}\\link\u{1b}]8;;\u{1b}\\b",
                "alinkb",
            ),
            ("a\u{1b}]0;title\u{7}b", "ab"),
            ("a\u{1b}P1$qm\u{1b}\\b", "ab"),
            ("a\u{1b}_apc\u{1b}\\b", "ab"),
            ("a\u{1b}^pm\u{1b}\\b", "ab"),
            ("a\u{1b}Xsos\u{1b}\\b", "ab"),
            ("a\u{1b}cb", "ab"),
            ("a\u{1b}7\u{1b}8b", "ab"),
            ("a\u{1b}(0b", "ab"),
            ("a\u{1b}#8b", "ab"),
            ("a\u{9b}31mb", "ab"),
            ("a\u{9b}2Jb", "ab"),
            ("a\u{9d}52;c;eA==\u{9c}b", "ab"),
            ("a\u{90}q\u{9c}b", "ab"),
            ("a\u{7}\u{8}\u{b}\u{c}\u{e}\u{f}\u{7f}\u{85}b", "ab"),
            ("a\u{1b}]52;c;unterminated", "a"),
            ("a\u{1b}[31", "a"),
            ("a\u{1b}", "a"),
            ("a\u{1b}[31\nb", "a\nb"),
            ("a\u{1b}\nb", "a\nb"),
        ];
        for (input, want) in cases {
            let got = sanitize_ansi(input);
            assert_eq!(got, want, "{input:?}");
            assert!(is_sgr_only(&got), "{input:?}");
        }
        let long = format!("a\u{1b}[{}mb", "1;".repeat(100));
        assert_eq!(sanitize_ansi(&long), "ab");
    }

    #[test]
    fn output_is_always_sgr_only() {
        let alphabet = [
            "\u{1b}", "[", "]", "P", "\\", "\u{7}", "\u{9b}", "\u{9c}", "\u{9d}", "m", "H", "?",
            ">", "1", ";", ":", " ", "x", "\r", "\n", "\u{90}", "_", "(",
        ];
        let mut state = 0x2545f4914f6cdd1du64;
        for _ in 0..20_000 {
            let mut s = String::new();
            for _ in 0..12 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                s.push_str(alphabet[(state % alphabet.len() as u64) as usize]);
            }
            let out = sanitize_ansi(&s);
            assert!(is_sgr_only(&out), "{s:?} -> {out:?}");
        }
    }

    fn tree() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        for d in ["root/proj/sub", "root2", "outside"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        std::fs::write(base.join("root/file"), "x").unwrap();
        std::os::unix::fs::symlink(base.join("outside"), base.join("root/escape")).unwrap();
        std::os::unix::fs::symlink(base.join("root/proj"), base.join("root/alias")).unwrap();
        (dir, base)
    }

    #[test]
    fn cwd_must_stay_inside_roots() {
        let (_dir, base) = tree();
        let roots = vec![base.join("root")];
        let s = |p: &str| base.join(p).to_str().unwrap().to_owned();
        assert_eq!(
            resolve_cwd(&s("root/proj/sub"), &roots).unwrap().as_str(),
            s("root/proj/sub")
        );
        assert_eq!(
            resolve_cwd(&s("root/alias/sub/.."), &roots)
                .unwrap()
                .as_str(),
            s("root/proj")
        );
        assert_eq!(resolve_cwd(&s("root"), &roots).unwrap().as_str(), s("root"));
        assert_eq!(
            resolve_cwd(&s("root/escape"), &roots),
            Err("cwd is outside the allowed roots")
        );
        assert_eq!(
            resolve_cwd(&s("root/proj/../../outside"), &roots),
            Err("cwd is outside the allowed roots")
        );
        assert_eq!(
            resolve_cwd(&s("root2"), &roots),
            Err("cwd is outside the allowed roots")
        );
        assert_eq!(
            resolve_cwd(&s("root/missing"), &roots),
            Err("cwd does not exist")
        );
        assert_eq!(
            resolve_cwd(&s("root/file"), &roots),
            Err("cwd is not a directory")
        );
    }

    fn listed(path: &str, roots: &[PathBuf]) -> (String, Vec<String>, bool) {
        let Ok(Response::TaskFolders {
            path,
            folders,
            truncated,
        }) = list_folders(path, roots)
        else {
            panic!("no listing for {path}");
        };
        let names = folders.iter().map(|f| f.as_str().to_owned()).collect();
        (path.as_str().to_owned(), names, truncated)
    }

    #[test]
    fn folders_lists_visible_directory_names_inside_roots() {
        let (_dir, base) = tree();
        let roots = vec![base.join("root")];
        let s = |p: &str| base.join(p).to_str().unwrap().to_owned();
        for d in ["root/Beta", "root/.git", "root/Icon\r", "root/x\u{202E}y"] {
            std::fs::create_dir(base.join(d)).unwrap();
        }
        std::fs::write(base.join("root/notes.txt"), "x").unwrap();
        std::os::unix::fs::symlink(base.join("root/file"), base.join("root/to-file")).unwrap();
        std::os::unix::fs::symlink("/nonexistent", base.join("root/dangling")).unwrap();
        #[cfg(target_os = "macos")]
        {
            std::fs::create_dir(base.join("root/Library")).unwrap();
            let status = std::process::Command::new("chflags")
                .arg("hidden")
                .arg(base.join("root/Library"))
                .status()
                .unwrap();
            assert!(status.success());
        }
        assert_eq!(
            listed(&s("root/alias/.."), &roots),
            (
                s("root"),
                vec!["alias".into(), "Beta".into(), "proj".into()],
                false
            )
        );
        assert_eq!(
            listed(&s("root/alias"), &roots),
            (s("root/proj"), vec!["sub".into()], false)
        );
        for outside in ["root2", "root/escape", "root/proj/../../outside"] {
            assert_eq!(
                list_folders(&s(outside), &roots),
                Err(invalid("cwd is outside the allowed roots")),
                "{outside}"
            );
        }
        assert_eq!(
            list_folders(&s("root/file"), &roots),
            Err(invalid("cwd is not a directory"))
        );
        std::fs::create_dir(base.join("root/.git/hooks")).unwrap();
        let yes: Authorized = Arc::new(|| true);
        let mut hidden = vec!["root/.git", "root/.git/hooks"];
        if cfg!(target_os = "macos") {
            std::fs::create_dir(base.join("root/Library/Mail")).unwrap();
            hidden.extend(["root/Library", "root/Library/Mail"]);
        }
        for path in hidden {
            assert_eq!(
                list_folders(&s(path), &roots),
                Err(invalid("folder is hidden")),
                "{path}"
            );
            assert_eq!(
                create_folder(&s(path), &FolderName::new("x").unwrap(), &roots, &yes),
                Err(invalid("folder is hidden")),
                "{path}"
            );
        }
    }

    #[test]
    fn folders_are_capped() {
        let (_dir, base) = tree();
        let many = base.join("root/many");
        for i in 0..=MAX_FOLDERS {
            std::fs::create_dir_all(many.join(format!("d{i:04}"))).unwrap();
        }
        let (_, names, truncated) = listed(many.to_str().unwrap(), &[base.join("root")]);
        assert!(truncated);
        assert_eq!(names.len(), MAX_FOLDERS);
        assert_eq!(names[0], "d0000");
    }

    #[test]
    fn create_folder_makes_one_empty_folder_inside_roots() {
        use std::os::unix::fs::MetadataExt;
        let (_dir, base) = tree();
        let roots = vec![base.join("root")];
        let s = |p: &str| base.join(p).to_str().unwrap().to_owned();
        let name = |n: &str| FolderName::new(n).unwrap();
        let yes: Authorized = Arc::new(|| true);
        let created = create_folder(&s("root/alias"), &name("new"), &roots, &yes).unwrap();
        assert_eq!(created.as_str(), s("root/proj/new"));
        let meta = std::fs::symlink_metadata(created.as_str()).unwrap();
        assert!(meta.is_dir());
        assert_eq!(meta.mode() & 0o7777 & !0o755, 0);
        assert_eq!(meta.mode() & 0o700, 0o700);
        assert_eq!(meta.uid(), rustix::process::geteuid().as_raw());
        assert_eq!(std::fs::read_dir(created.as_str()).unwrap().count(), 0);

        std::os::unix::fs::symlink("/nonexistent", base.join("root/dangling")).unwrap();
        for taken in ["proj", "alias", "escape", "file", "dangling"] {
            assert_eq!(
                create_folder(&s("root"), &name(taken), &roots, &yes),
                Err(invalid("folder already exists")),
                "{taken}"
            );
        }
        assert!(!Path::new("/nonexistent").exists());
        for parent in ["outside", "root/escape", "root2"] {
            assert_eq!(
                create_folder(&s(parent), &name("x"), &roots, &yes),
                Err(invalid("cwd is outside the allowed roots")),
                "{parent}"
            );
        }
        let no: Authorized = Arc::new(|| false);
        assert_eq!(
            create_folder(&s("root"), &name("late"), &roots, &no)
                .unwrap_err()
                .0,
            ErrorCode::NotPaired
        );
        assert!(!base.join("root/late").exists());
        let deep = base
            .join("root")
            .join("d".repeat(250))
            .join("d".repeat(250));
        let deep = deep.join("d".repeat(250));
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(
            create_folder(
                deep.to_str().unwrap(),
                &name(&"n".repeat(255)),
                &roots,
                &yes
            ),
            Err(invalid("folder path is too long"))
        );
        assert_eq!(std::fs::read_dir(&deep).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(base.join("outside")).unwrap().count(), 0);
    }

    #[test]
    fn open_beneath_never_follows_a_link() {
        let (_dir, base) = tree();
        let root = base.join("root");
        assert!(open_beneath(&root, &root.join("proj/sub")).is_ok());
        assert!(open_beneath(&root, &root).is_ok());
        for swapped in ["alias", "alias/sub", "escape"] {
            assert!(
                open_beneath(&root, &root.join(swapped)).is_err(),
                "{swapped}"
            );
        }
        assert!(open_beneath(&root, &root.join("proj/..")).is_err());
        assert!(open_beneath(&root, &base.join("outside")).is_err());
    }

    #[test]
    fn driver_canonicalizes_roots() {
        let (_dir, base) = tree();
        assert!(Driver::new(PathBuf::new(), vec![], &[base.join("root/alias")]).is_ok());
        assert!(Driver::new(PathBuf::new(), vec![], &[base.join("nope")]).is_err());
        assert!(Driver::new(PathBuf::new(), vec![], &[PathBuf::from("rel")]).is_err());
    }

    fn op(n: u8) -> OpId {
        OpId::new(format!("{}", n as char).repeat(22)).unwrap()
    }

    #[tokio::test]
    async fn op_cache_replays_without_rerunning() {
        let cache = OpCache::new(2, Duration::from_millis(300));
        let runs = Arc::new(AtomicUsize::new(0));
        let run = |peer: &'static str, id: OpId, fp: u64| {
            let runs = runs.clone();
            let cache = &cache;
            async move {
                cache
                    .once(peer, &id, fp, async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        Ok(Response::Ok)
                    })
                    .await
            }
        };
        let (first, second) = tokio::join!(run("p", op(b'A'), 1), run("p", op(b'A'), 1));
        assert_eq!(first, (Ok(Response::Ok), Origin::Ran));
        assert_eq!(second, (Ok(Response::Ok), Origin::Replayed));
        assert_eq!(runs.load(Ordering::SeqCst), 1);

        let reused = run("p", op(b'A'), 2).await;
        assert_eq!(reused.0.unwrap_err().0, ErrorCode::InvalidParams);
        assert_eq!(reused.1, Origin::Refused);
        assert!(run("other", op(b'A'), 1).await.0.is_ok());
        assert_eq!(runs.load(Ordering::SeqCst), 2);

        assert!(run("p", op(b'B'), 1).await.0.is_ok());
        assert!(run("p", op(b'C'), 1).await.0.is_ok());
        assert_eq!(runs.load(Ordering::SeqCst), 4);
        assert_eq!(
            run("p", op(b'A'), 1).await.1,
            Origin::Ran,
            "evicted beyond the cap"
        );
        assert_eq!(run("p", op(b'C'), 1).await.1, Origin::Replayed);

        tokio::time::sleep(Duration::from_millis(350)).await;
        assert_eq!(run("p", op(b'C'), 1).await.1, Origin::Ran, "expired");
    }

    #[tokio::test]
    async fn op_cache_never_evicts_in_flight() {
        let cache = Arc::new(OpCache::new(1, OP_TTL));
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let slow = tokio::spawn({
            let cache = cache.clone();
            async move {
                cache
                    .once("p", &op(b'A'), 1, async move {
                        let _ = held.await;
                        Ok(Response::Ok)
                    })
                    .await
            }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let busy = cache
            .once("p", &op(b'B'), 1, async { Ok(Response::Ok) })
            .await;
        assert_eq!(busy.0.unwrap_err().0, ErrorCode::RateLimited);
        assert_eq!(busy.1, Origin::Refused);
        assert_eq!(
            cache
                .once("other", &op(b'B'), 1, async { Ok(Response::Ok) })
                .await
                .1,
            Origin::Ran
        );
        release.send(()).unwrap();
        assert_eq!(slow.await.unwrap(), (Ok(Response::Ok), Origin::Ran));
        let ran = cache
            .once("p", &op(b'B'), 1, async { Ok(Response::Ok) })
            .await;
        assert_eq!(ran, (Ok(Response::Ok), Origin::Ran));
        let evicted = cache
            .once("p", &op(b'A'), 1, async { Ok(Response::Ok) })
            .await;
        assert_eq!(evicted.1, Origin::Ran, "completed entries are evicted");
    }

    #[tokio::test]
    async fn op_cache_survives_a_dropped_caller() {
        let cache = OpCache::new(8, OP_TTL);
        let runs = Arc::new(AtomicUsize::new(0));
        let r = runs.clone();
        let id = op(b'A');
        let first = cache.once("p", &id, 1, async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            r.fetch_add(1, Ordering::SeqCst);
            Ok(Response::Ok)
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(10), first)
                .await
                .is_err()
        );
        let r = runs.clone();
        let (reply, replayed) = cache
            .once("p", &op(b'A'), 1, async move {
                r.fetch_add(1, Ordering::SeqCst);
                Ok(Response::Ok)
            })
            .await;
        assert_eq!((reply, replayed), (Ok(Response::Ok), Origin::Replayed));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn keys_map_to_herdr_names() {
        let all = [
            (Key::Esc, "esc"),
            (Key::Enter, "enter"),
            (Key::Up, "up"),
            (Key::Down, "down"),
            (Key::Left, "left"),
            (Key::Right, "right"),
            (Key::Tab, "tab"),
            (Key::ShiftTab, "shift+tab"),
            (Key::CtrlC, "ctrl+c"),
            (Key::CtrlEnter, "ctrl+enter"),
            (Key::Backspace, "backspace"),
            (Key::CtrlA, "ctrl+a"),
            (Key::CtrlB, "ctrl+b"),
            (Key::CtrlE, "ctrl+e"),
            (Key::CtrlK, "ctrl+k"),
            (Key::CtrlL, "ctrl+l"),
            (Key::CtrlU, "ctrl+u"),
            (Key::CtrlW, "ctrl+w"),
        ];
        for (key, name) in all {
            assert_eq!(key.herdr_name(), name);
        }
    }

    fn agent(status: &str) -> AgentInfo {
        serde_json::from_value(serde_json::json!({
            "terminal_id": "term_1", "workspace_id": "w1", "pane_id": "w1:p1",
            "agent": "claude", "agent_status": status,
            "agent_session": {"source": "herdr:claude", "agent": "claude", "kind": "id", "value": "s1"},
        }))
        .unwrap()
    }

    #[test]
    fn readiness() {
        let listed = agent("idle");
        assert!(check_ready(&listed, &agent("working")).is_ok());
        assert_eq!(
            check_ready(&listed, &agent("blocked")).unwrap_err().0,
            ErrorCode::AgentBlocked
        );
        let mut pending = agent("idle");
        pending.launch_pending = true;
        assert_eq!(
            check_ready(&listed, &pending).unwrap_err().0,
            ErrorCode::AgentNotReady
        );
        let mut shell = agent("idle");
        shell.agent = None;
        assert_eq!(
            check_ready(&listed, &shell).unwrap_err().0,
            ErrorCode::AgentNotReady
        );
        let mut moved = agent("idle");
        moved.terminal_id = "term_2".into();
        assert_eq!(
            check_ready(&listed, &moved).unwrap_err().0,
            ErrorCode::AgentNotReady
        );
        let mut replaced = agent("idle");
        replaced.agent_session.as_mut().unwrap().value = "s2".into();
        assert_eq!(
            check_ready(&listed, &replaced).unwrap_err().0,
            ErrorCode::AgentNotReady
        );
        let mut other_kind = agent("idle");
        other_kind.agent = Some("codex".into());
        assert_eq!(
            check_ready(&listed, &other_kind).unwrap_err().0,
            ErrorCode::AgentNotReady
        );
    }

    #[test]
    fn herdr_errors_map_to_protocol_codes() {
        let e = |code: &str| {
            herdr_fail(herdr::Error::Herdr {
                code: code.into(),
                message: "m".into(),
            })
            .0
        };
        assert_eq!(e("agent_blocked"), ErrorCode::AgentBlocked);
        assert_eq!(e("agent_not_ready"), ErrorCode::AgentNotReady);
        assert_eq!(e("empty_agent_prompt"), ErrorCode::InvalidParams);
        assert_eq!(e("timeout"), ErrorCode::AgentNotReady);
        assert_eq!(e("agent_pane_busy"), ErrorCode::AgentNotReady);
        assert_eq!(e("pane_not_found"), ErrorCode::NotFound);
        assert_eq!(
            e("workspace_group_close_required"),
            ErrorCode::NotImplemented
        );
        assert_eq!(e("agent_prompt_failed"), ErrorCode::Internal);
        assert_eq!(
            herdr_fail(herdr::Error::Timeout).0,
            ErrorCode::HerdrUnavailable
        );
    }

    #[tokio::test]
    async fn clone_runs_git_or_gh_with_the_source_after_double_dash_and_https_or_ssh_only() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let target = base.join("app");
        let target = Cwd::new(target.to_str().unwrap()).unwrap();
        let source = |s: &str| CloneSource::new(s).unwrap();
        let args = |s: &str, gh: Option<&str>| {
            let command = clone_command(&source(s), &target, gh);
            let command = command.as_std();
            let mut args = vec![command.get_program().to_str().unwrap().to_owned()];
            args.extend(command.get_args().map(|a| a.to_str().unwrap().to_owned()));
            args
        };
        let t = target.as_str();
        let gh = Some("/opt/homebrew/bin/gh");
        assert_eq!(
            args("owner/app", gh),
            [
                "/opt/homebrew/bin/gh",
                "repo",
                "clone",
                "--",
                "owner/app",
                t
            ]
        );
        assert_eq!(
            args("owner/app.git", None),
            ["git", "clone", "--", "https://github.com/owner/app.git", t]
        );
        for url in [
            "https://git.example.com/group/app.git",
            "git@git.example.com:group/app.git",
        ] {
            assert_eq!(args(url, gh), ["git", "clone", "--", url, t]);
        }
        let command = clone_command(&source("owner/app"), &target, None);
        let envs: HashMap<_, _> = command
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_str().unwrap().to_owned(),
                    v.map(|v| v.to_str().unwrap().to_owned()),
                )
            })
            .collect();
        assert_eq!(envs["GIT_TERMINAL_PROMPT"].as_deref(), Some("0"));
        assert_eq!(envs["GH_PROMPT_DISABLED"].as_deref(), Some("1"));
        assert_eq!(envs["GH_TOKEN"], None);
        assert_eq!(envs["GITHUB_TOKEN"], None);

        let repo = base.join("repos/group/app");
        std::fs::create_dir_all(&repo).unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "-q", "--bare"])
            .arg(&repo)
            .status()
            .unwrap();
        assert!(init.success());
        let home = base.join("home");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            format!(
                "[url \"file://{}/repos/\"]\n\tinsteadOf = https://example.invalid/\n[protocol \"file\"]\n\tallow = always\n",
                base.display()
            ),
        )
        .unwrap();
        let rewritten = |allowlist: bool| {
            let mut command =
                clone_command(&source("https://example.invalid/group/app"), &target, None);
            command.env("HOME", &home).env_remove("XDG_CONFIG_HOME");
            if !allowlist {
                command.env("GIT_CONFIG_COUNT", "0");
            }
            command.output()
        };
        let refused = rewritten(true).await.unwrap();
        assert!(!refused.status.success());
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(stderr.contains("transport 'file' not allowed"), "{stderr}");
        assert!(!Path::new(t).exists());
        let control = rewritten(false).await.unwrap();
        assert!(
            control.status.success(),
            "{}",
            String::from_utf8_lossy(&control.stderr)
        );
    }

    #[tokio::test]
    async fn a_pull_request_branch_is_checked_fetched_over_https_or_ssh_and_tracked_when_new() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .current_dir(&root)
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "{args:?}");
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["commit", "-q", "--allow-empty", "-m", "one"]);
        let first = git(&["rev-parse", "HEAD"]);
        git(&["commit", "-q", "--allow-empty", "-m", "two"]);
        let second = git(&["rev-parse", "HEAD"]);
        let checkout = Cwd::new(root.to_str().unwrap()).unwrap();
        for good in ["feature", "feat/282-pr-prompts", "v1.2_x"] {
            assert!(checked_branch(&checkout, good).await.is_ok(), "{good}");
        }
        for bad in [
            "", "-x", "a..b", "a/", "x.lock", "a//b", ".hidden", "a/.b", "a b", "a~1", "@{-1}",
            "é", "../../x",
        ] {
            assert!(checked_branch(&checkout, bad).await.is_err(), "{bad}");
        }

        let head = |branch: &str, oid: &str| pulls::Head {
            branch: branch.into(),
            oid: oid.into(),
        };
        git(&["remote", "add", "origin", root.to_str().unwrap()]);
        let deadline = tokio::time::Instant::now() + CLONE_TIMEOUT;
        let refused = fetch_pull_request(&checkout, 7, &head("main", &second), deadline)
            .await
            .unwrap_err();
        assert!(
            refused.1.contains("transport 'file' not allowed"),
            "{}",
            refused.1
        );

        let track = |branch: &str, oid: &str| {
            let head = head(branch, oid);
            let checkout = checkout.clone();
            async move { track_pull_request(&checkout, 7, &head).await }
        };
        let missing = track("feature", &second).await.unwrap_err();
        assert!(missing.1.contains("is not the head of pull request #7"));
        git(&["update-ref", "refs/remotes/origin/feature", &second]);
        assert!(track("feature", &first).await.is_err(), "the head moved");
        assert_eq!(git(&["branch", "--list", "feature"]), "");
        track("feature", &second).await.unwrap();
        assert_eq!(git(&["rev-parse", "refs/heads/feature"]), second);
        assert_eq!(
            git(&["rev-parse", "--abbrev-ref", "feature@{upstream}"]),
            "origin/feature"
        );
        track("feature", &second).await.unwrap();
        git(&["update-ref", "refs/heads/feature", &first]);
        let stale = track("feature", &second).await.unwrap_err();
        assert!(
            stale
                .1
                .contains("a local branch feature exists at another commit"),
            "{}",
            stale.1
        );
        assert_eq!(
            git(&["rev-parse", "refs/heads/feature"]),
            first,
            "never moved"
        );
    }

    #[tokio::test]
    async fn owner_repo_falls_back_to_git_only_when_gh_is_signed_out_and_left_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let target = Cwd::new(base.join("app").to_str().unwrap()).unwrap();
        std::fs::create_dir(target.as_str()).unwrap();
        let fake = |name: &str, code: u8| {
            let path = base.join(name);
            std::fs::write(
                &path,
                format!("#!/bin/sh\necho \"gh says $1\" >&2\nexit {code}\n"),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path.to_str().unwrap().to_owned()
        };
        let source = CloneSource::new("owner/app").unwrap();
        let deadline = tokio::time::Instant::now() + CLONE_TIMEOUT;
        let run = |gh: String| run_clone(clone_command(&source, &target, Some(&gh)), deadline);
        let signed_out = run(fake("out", 4)).await;
        assert_eq!(signed_out, Err((Some(4), "gh says repo".to_owned())));
        assert!(gh_signed_out(&signed_out, &target));
        let failed = run(fake("failed", 1)).await;
        assert_eq!(failed, Err((Some(1), "gh says repo".to_owned())));
        assert!(!gh_signed_out(&failed, &target));
        assert!(!gh_signed_out(&Err((None, "timed out".into())), &target));
        std::fs::write(base.join("app/partial"), "").unwrap();
        assert!(!gh_signed_out(&signed_out, &target));
    }
}
