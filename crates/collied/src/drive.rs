use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::iter::Peekable;
use std::path::PathBuf;
use std::str::Chars;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use protocol::{
    AgentKind, AgentPromptParams, AgentSendKeysParams, AgentSlashDraftParams, AgentStatus,
    AgentTypeTextParams, Cwd, ErrorCode, Key, NoticeDigit, OpId, OutputPatch, PaneCloseParams,
    ReadParams, ReadSource, Request, Response, TaskNewParams, TaskOptions, TerminalId,
    TerminalRead, TerminalRunParams, WorkspaceCloseParams, WorkspaceId, limits,
};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::draft::{self, InputBox};
use crate::herdr::{self, AgentInfo, PaneInfo};
use crate::prompt::Menu;
use crate::{approvals, flock, prompt, reflow};

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
const TYPED_NOT_SENT: &str = "the prompt did not take the text; Enter was not sent";
pub const HOSTS_AGENT: &str = "the pane now hosts an agent";
pub const LOCKED: &str = "the terminal is locked; unlock it again";
const SCROLLED: &str = "scrolled up on the machine; jump to the bottom first";
const COMMAND_SHOWN: &str = "a slash command shows in the input box; send it as a prompt";
/// Claude Code's ctrl+end, bound to `scroll:bottom`; herdr 0.9.3 has no key name for it.
const SCROLL_BOTTOM: &str = "\u{1b}[1;5F";

pub(crate) fn fail<T>(code: ErrorCode, message: impl Into<String>) -> Result<T, Fail> {
    Err((code, message.into()))
}

pub fn herdr_fail(e: herdr::Error) -> Fail {
    tracing::debug!(error = %e, "herdr call failed");
    let herdr::Error::Herdr { code, .. } = &e else {
        return (ErrorCode::HerdrUnavailable, "herdr unavailable".to_owned());
    };
    let (code, message) = match code.as_str() {
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

/// herdr's error message may quote the input, so only its code is kept.
fn quiet_fail(e: herdr::Error) -> Fail {
    match e {
        herdr::Error::Herdr { code, .. } => herdr_fail(herdr::Error::Herdr {
            code,
            message: String::new(),
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
    /// Prompts, keys and slash drafts from different sessions never interleave, so a
    /// mirrored command is never submitted by another session's write.
    box_writes: tokio::sync::Mutex<()>,
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
        })
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
    async fn find_agent(&self, terminal_id: &TerminalId) -> Result<AgentInfo, Fail> {
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

    async fn ready_agent(&self, terminal_id: &TerminalId) -> Result<AgentInfo, Fail> {
        let listed = self.find_agent(terminal_id).await?;
        let current = herdr::agent_get(&self.herdr, &listed.pane_id)
            .await
            .map_err(herdr_fail)?;
        check_ready(&listed, &current)?;
        Ok(current)
    }

    /// For a blocked agent also returns the screen, read now, of a prompt `open` lets the
    /// phone answer ([`approvals::open_to`]); any other blocked prompt is refused.
    async fn writable_agent(
        &self,
        terminal_id: &TerminalId,
        open: fn(&str, Option<&str>, &str) -> bool,
    ) -> Result<(AgentInfo, Option<String>), Fail> {
        let listed = self.find_agent(terminal_id).await?;
        let current = herdr::agent_get(&self.herdr, &listed.pane_id)
            .await
            .map_err(herdr_fail)?;
        match check_ready(&listed, &current) {
            Ok(()) => Ok((current, None)),
            Err((ErrorCode::AgentBlocked, _)) => {
                match approvals::open_to(&self.herdr, &current, open)
                    .await
                    .map_err(herdr_fail)?
                {
                    Some(screen) => Ok((current, Some(screen))),
                    None => fail(ErrorCode::AgentBlocked, BLOCKED),
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Polls the blocked agent's menu until `done` holds. The agent must stay `blocked`
    /// with the same `state_change_seq` throughout: a new prompt comes with a new one.
    async fn settle_menu(&self, a: &AgentInfo, done: impl Fn(&Menu) -> bool) -> Result<(), Fail> {
        let deadline = tokio::time::Instant::now() + SCREEN_SETTLE;
        loop {
            tokio::time::sleep(SCREEN_POLL).await;
            let now = herdr::agent_get(&self.herdr, &a.pane_id)
                .await
                .map_err(herdr_fail)?;
            if !matches!(check_ready(a, &now), Err((ErrorCode::AgentBlocked, _)))
                || now.state_change_seq != a.state_change_seq
            {
                return fail(ErrorCode::AgentNotReady, TYPED_NOT_SENT);
            }
            let text = herdr::detection_text(&self.herdr, &a.pane_id)
                .await
                .map_err(herdr_fail)?;
            if Menu::parse(&text).is_some_and(|m| done(&m)) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return fail(ErrorCode::AgentNotReady, TYPED_NOT_SENT);
            }
        }
    }

    /// `pane.read` reads agent panes only: a shell is read through `terminal.watch`, under
    /// a grant.
    pub async fn read(&self, p: ReadParams, agent: bool) -> Reply {
        let mut lines = p.lines.map(u32::from);
        let source = source_name(p.source);
        let mut claude = None;
        let read = if agent {
            let a = self.find_agent(&p.terminal_id).await?;
            if a.agent.as_deref() == Some("claude") {
                claude = Some(a.pane_id.clone());
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
        if let Some(pane_id) = claude {
            self.reflow(&pane_id, &mut read).await;
        }
        Ok(Response::Terminal(read))
    }

    /// Leaves `read` unjoined when herdr does not give the pane's width; `false` when herdr
    /// failed.
    async fn reflow(&self, pane_id: &str, read: &mut TerminalRead) -> bool {
        let cols = herdr::pane_columns(&self.herdr, pane_id).await;
        if let Ok(Some(cols)) = cols {
            (read.wraps, read.splits) = reflow::soft_wraps(&read.ansi, cols.into());
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
                    .map(|p| (p.pane_id, false)),
                None => self
                    .find_agent(&terminal_id)
                    .await
                    .map(|a| (a.pane_id, a.agent.as_deref() == Some("claude"))),
            };
            let msg = match pane {
                Err((ErrorCode::NotFound, _)) => Watched::Gone,
                Err(_) => continue,
                Ok((pane_id, claude)) => {
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
                    if claude && !self.reflow(&pane_id, &mut read).await {
                        last = None;
                    }
                    if sent.as_ref() == Some(&read) {
                        continue;
                    }
                    let patch = sent
                        .as_ref()
                        .and_then(|prev| OutputPatch::between(prev, &read));
                    sent = Some(read.clone());
                    match patch {
                        Some(patch) => Watched::Patch(patch),
                        None => Watched::Output(read),
                    }
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
        let _box = self.box_writes.lock().await;
        let a = self.ready_agent(&p.terminal_id).await?;
        // herdr pastes a prompt after whatever is in Claude Code's input box.
        if a.agent.as_deref() == Some("claude") {
            // herdr ends a prompt with Enter, which would confirm a dialog scrolled out of view.
            let screen = herdr::detection_text(&self.herdr, &a.pane_id)
                .await
                .map_err(herdr_fail)?;
            if protocol::jump_banner(&screen) {
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
        let (a, screen) = match self
            .writable_agent(&p.terminal_id, prompt::open_to_keys)
            .await
        {
            Ok(found) => found,
            Err(e) => return (Err(e), None),
        };
        if a.agent.as_deref() == Some("claude") {
            // A dialog scrolled out of view leaves the agent idle: keys would answer it unseen.
            let scrolled = match &screen {
                Some(s) => protocol::jump_banner(s),
                None => match herdr::detection_text(&self.herdr, &a.pane_id).await {
                    Ok(s) => protocol::jump_banner(&s),
                    Err(e) => return (Err(herdr_fail(e)), None),
                },
            };
            if scrolled {
                return (fail(ErrorCode::AgentNotReady, SCROLLED), None);
            }
            // Enter would run the command menu's highlighted entry.
            if screen.is_none()
                && p.keys
                    .iter()
                    .any(|k| matches!(k, Key::Enter | Key::CtrlEnter))
            {
                match self.input_box(&a.pane_id).await {
                    Ok(Some(InputBox::Draft(d))) if d.text.starts_with('/') => {
                        return (fail(ErrorCode::AgentNotReady, COMMAND_SHOWN), None);
                    }
                    Ok(_) => {}
                    Err(e) => return (Err(e), None),
                }
            }
        }
        let keys: Vec<&str> = p.keys.iter().map(|k| k.herdr_name()).collect();
        let target = screen
            .is_some()
            .then(|| format!("{} blocked keys={}", p.terminal_id.as_str(), keys.join(",")));
        let sent = async {
            authorized(auth)?;
            herdr::agent_send_keys(&self.herdr, &a.pane_id, &keys)
                .await
                .map_err(herdr_fail)?;
            Ok(Response::Ok)
        };
        (sent.await, target)
    }

    /// Answers a Claude Code question or plan through its free-text option: the cursor
    /// moves there, the text is typed, and Enter is sent only once that option reads as
    /// the text with the rest of the dialog unchanged. Typed under any other option, the
    /// text would be dropped (or read as digit shortcuts) and Enter would confirm that
    /// option. Only arrows, the text and Enter are sent: never shift+tab, which on a plan
    /// approves it with the feedback.
    pub async fn type_text(&self, p: AgentTypeTextParams, auth: &Authorized) -> Reply {
        let (a, screen) = self
            .writable_agent(&p.terminal_id, prompt::open_to_text)
            .await?;
        let Some(screen) = screen else {
            return fail(
                ErrorCode::AgentNotReady,
                "the agent is not waiting for an answer; send a prompt",
            );
        };
        let Some((menu, field)) =
            Menu::parse(&screen).and_then(|m| m.free_text().map(|field| (m, field)))
        else {
            return fail(
                ErrorCode::AgentBlocked,
                "this prompt has no text field; choose an option",
            );
        };
        let arrows = menu.arrows(field);
        if !arrows.is_empty() {
            authorized(auth)?;
            herdr::agent_send_keys(&self.herdr, &a.pane_id, &arrows)
                .await
                .map_err(herdr_fail)?;
            let on_field = menu.region_at(field);
            self.settle_menu(&a, |m| m.region() == on_field).await?;
        }
        authorized(auth)?;
        herdr::pane_send_text(&self.herdr, &a.pane_id, p.text.as_str())
            .await
            .map_err(herdr_fail)?;
        self.settle_menu(&a, |m| typed_into(&menu, field, m, p.text.as_str()))
            .await?;
        authorized(auth)?;
        herdr::agent_send_keys(&self.herdr, &a.pane_id, &["enter"])
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
        let listed = self.find_agent(terminal_id).await?;
        let current = herdr::agent_get(&self.herdr, &listed.pane_id)
            .await
            .map_err(herdr_fail)?;
        match check_ready(&listed, &current) {
            Ok(()) | Err((ErrorCode::AgentBlocked, _)) => {}
            Err(e) => return Err(e),
        }
        if current.agent.as_deref() != Some("claude") {
            return fail(ErrorCode::AgentNotReady, "not a Claude Code agent");
        }
        let screen = herdr::detection_text(&self.herdr, &current.pane_id)
            .await
            .map_err(herdr_fail)?;
        if !protocol::jump_banner(&screen) {
            return fail(ErrorCode::AgentNotReady, "already at the bottom");
        }
        authorized(auth)?;
        herdr::pane_send_text(&self.herdr, &current.pane_id, SCROLL_BOTTOM)
            .await
            .map_err(herdr_fail)?;
        Ok(Response::Ok)
    }

    /// One digit, no Enter, only while Claude Code shows a notice listing it above an empty
    /// input box. Never to a blocked agent: a digit could select a permission option. The
    /// notice and the box are checked on the same visible read, the last one before the write.
    pub async fn answer_notice(
        &self,
        terminal_id: &TerminalId,
        digit: NoticeDigit,
        auth: &Authorized,
    ) -> Reply {
        let a = self.ready_agent(terminal_id).await?;
        if a.agent.as_deref() != Some("claude") {
            return fail(ErrorCode::AgentNotReady, "not a Claude Code agent");
        }
        let screen = herdr::detection_text(&self.herdr, &a.pane_id)
            .await
            .map_err(herdr_fail)?;
        if protocol::jump_banner(&screen) {
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
        if !protocol::notice(&visible).iter().any(|o| o.digit == digit) {
            return fail(ErrorCode::AgentNotReady, "no notice with that option");
        }
        if !matches!(
            draft::parse(&visible),
            Some(InputBox::Draft(d)) if d.text.is_empty()
        ) {
            return fail(ErrorCode::AgentNotReady, "the input box is not empty");
        }
        authorized(auth)?;
        herdr::pane_send_text(&self.herdr, &a.pane_id, digit.as_str())
            .await
            .map_err(herdr_fail)?;
        Ok(Response::Ok)
    }

    /// Mirrors the phone's slash command token into Claude Code's input box as a bracketed
    /// paste, never Enter or a key, so its command menu shows. The box is cleared first
    /// through [`Self::replace_draft`]; an empty command stops there. The agent, the banner
    /// and an empty visible box are checked again just before the paste.
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
        Ok(Response::Ok)
    }

    async fn claude_at_bottom(&self, terminal_id: &TerminalId) -> Result<AgentInfo, Fail> {
        let a = self.ready_agent(terminal_id).await?;
        if a.agent.as_deref() != Some("claude") {
            return fail(ErrorCode::AgentNotReady, "not a Claude Code agent");
        }
        let screen = herdr::detection_text(&self.herdr, &a.pane_id)
            .await
            .map_err(herdr_fail)?;
        if protocol::jump_banner(&screen) {
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
        Ok(Response::TaskOptions(TaskOptions {
            agents,
            default_agent,
            recent_cwds,
        }))
    }

    /// Also returns the canonical cwd once it is resolved, for the audit line.
    pub async fn task_new(&self, p: TaskNewParams, auth: &Authorized) -> (Reply, Option<Cwd>) {
        match self.task_cwd(&p).await {
            Ok(cwd) => (self.start_task(p, &cwd, auth).await, Some(cwd)),
            Err(e) => (Err(e), None),
        }
    }

    async fn task_cwd(&self, p: &TaskNewParams) -> Result<Cwd, Fail> {
        if !self.agents.contains(&p.agent) {
            return fail(ErrorCode::InvalidParams, "agent kind is not allowed");
        }
        let roots = self.roots.clone();
        let requested = p.cwd.as_str().to_owned();
        tokio::task::spawn_blocking(move || resolve_cwd(&requested, &roots))
            .await
            .map_err(|_| (ErrorCode::Internal, "cwd check failed".to_owned()))?
            .map_err(|m| (ErrorCode::InvalidParams, m.to_owned()))
    }

    /// On a failure after `workspace.create` nothing is closed: the error names the
    /// workspace that was left open.
    async fn start_task(&self, p: TaskNewParams, cwd: &Cwd, auth: &Authorized) -> Reply {
        authorized(auth)?;
        let pane = herdr::workspace_create(
            &self.herdr,
            cwd.as_str(),
            p.label.as_ref().map(|l| l.as_str()),
        )
        .await
        .map_err(herdr_fail)?
        .root_pane;
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
                    "workspace {} was created and left open: {message}",
                    workspace_id.as_str()
                ),
            )
        };
        let name = self
            .start_agent(&pane, &p.agent, auth)
            .await
            .map_err(left_open)?;
        let current = herdr::agent_get(&self.herdr, &name)
            .await
            .map_err(|e| left_open(herdr_fail(e)))?;
        self.started_check(&current, &pane, &name, &p.agent)
            .await
            .map_err(left_open)?;
        // herdr may not rule another kind's startup prompt `blocked`, and the prompt's Enter
        // would answer it.
        if p.agent.as_str() != "claude"
            && herdr::detection_text(&self.herdr, &pane.pane_id)
                .await
                .map_or(true, |t| prompt::shows_dialog(&t))
        {
            return Err(left_open((
                ErrorCode::AgentBlocked,
                "the agent is blocked on a startup prompt; answer it in the terminal".to_owned(),
            )));
        }
        authorized(auth).map_err(left_open)?;
        herdr::agent_prompt(&self.herdr, &name, p.prompt.as_str())
            .await
            .map_err(|e| left_open(herdr_fail(e)))?;
        Ok(Response::TaskStarted {
            workspace_id,
            terminal_id,
        })
    }

    // Readiness rules follow herdr's own `herdr agent start` CLI (0.9.3). The busy retry
    // gets 30 s instead of the CLI's 2 s and, unlike the CLI, does not require the shell
    // in the foreground: the CLI targets any pane, this one was just created.
    async fn start_agent(
        &self,
        pane: &PaneInfo,
        kind: &AgentKind,
        auth: &Authorized,
    ) -> Result<String, Fail> {
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        let name = agent_name()?;
        let started = loop {
            authorized(auth)?;
            match herdr::agent_start(&self.herdr, &name, kind.as_str(), &pane.pane_id).await {
                Ok(a) => break a,
                // Retried while the pane still holds the terminal created for this task: what
                // runs there is the shell's startup (rc files often run helpers in the
                // foreground), as nothing else has used the pane yet.
                Err(herdr::Error::Herdr { code, .. })
                    if code == "agent_pane_busy"
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
        let message = if kind.as_str() != "claude" {
            "the agent is blocked on a startup prompt; answer it in the terminal"
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
            .is_ok_and(|now| now.terminal_id == pane.terminal_id)
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
}

/// Whitespace is ignored because a long answer wraps onto continuation rows.
fn typed_into(before: &Menu, field: usize, now: &Menu, text: &str) -> bool {
    before.only_changed(field, now) && now.reads(field, text) && !before.reads(field, text)
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
            let mut peers = self.peers.lock().unwrap_or_else(|e| e.into_inner());
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
    fn the_watch_slows_down_after_a_quiet_spell_or_in_low_data_mode() {
        assert_eq!(watch_period(false, Duration::ZERO), WATCH_EVERY);
        assert_eq!(
            watch_period(false, WATCH_QUIET - Duration::from_millis(1)),
            WATCH_EVERY
        );
        assert_eq!(watch_period(false, WATCH_QUIET), WATCH_IDLE);
        assert_eq!(watch_period(true, Duration::ZERO), WATCH_IDLE);
        assert_eq!(watch_period(true, Duration::MAX), WATCH_IDLE);
        assert_eq!(
            (WATCH_EVERY, WATCH_IDLE),
            (Duration::from_millis(250), Duration::from_secs(1))
        );
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
}
