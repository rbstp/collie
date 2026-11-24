use std::collections::{BTreeMap, BTreeSet, HashSet};

use sha2::{Digest, Sha256};

use super::client::TERMINAL;
use super::wire::{
    BlockEvent, BlockId, BlockRef, Label, Proc, Process, ProcessChanged, Record, SessionCreated,
    SessionEvent, SessionId, SessionRef, Size, StatusChanged, StatusRemoved, Tagged, Title,
};
use crate::runtime::{AgentInfo, AgentSession, PaneInfo, WorkspaceInfo, agents};

pub const PREFIX: &str = "rex:";

/// A value with the `seq` that set it: an event's own, or a read's response `seq`, the last
/// event the read reflects. Neither replaces what something later set.
#[derive(Default)]
struct Stamped<T> {
    value: T,
    seq: u64,
}

impl<T> Stamped<T> {
    fn event(&mut self, seq: u64, value: T) {
        if seq > self.seq {
            (self.value, self.seq) = (value, seq);
        }
    }

    fn read(&mut self, seq: u64, value: T) {
        if seq >= self.seq {
            (self.value, self.seq) = (value, seq);
        }
    }
}

struct Session {
    label: Stamped<String>,
    attached: bool,
    blocks: BTreeMap<String, Block>,
}

#[derive(Default)]
struct Block {
    /// The root record only: Claude Code and Codex report without an id.
    status: Stamped<Option<Record>>,
    foreground: Stamped<Option<Proc>>,
    /// A shell runs the foreground process: one of its ancestors is a shell.
    under_shell: Stamped<bool>,
    child: Option<Proc>,
    title: Stamped<String>,
    size: Stamped<(u16, u16)>,
    /// The agent's session, with the pid it was found for: it holds only while that process
    /// leads the terminal.
    session: Option<(u32, String)>,
    /// Its program exited; the block stays until it is closed.
    exited: bool,
}

/// One server's sessions and terminal blocks, from reads and the events that follow them.
/// Events for a block not read yet are dropped: its reads start after it is added, so they
/// reflect them.
#[derive(Default)]
pub(super) struct Model {
    host: Option<String>,
    sessions: BTreeMap<String, Session>,
    /// Closed blocks and destroyed sessions, while a read that started before could still
    /// list them. Ids are never reused.
    gone: HashSet<String>,
    /// Blocks whose reads failed: left out until a report of theirs has them read again.
    failed: HashSet<String>,
    /// Blocks whose process or status changed since `touched` last ran, by session and id.
    touched: BTreeSet<(String, String)>,
    stale_process: BTreeSet<(String, String)>,
}

/// Rex's ids are `session:<uuid>` and `block:<uuid>`; anything else is skipped, so every id
/// collied makes fits the phone's terminal ids.
pub(super) fn valid(id: &str, kind: &str) -> bool {
    id.strip_prefix(kind)
        .and_then(|u| u.strip_prefix(':'))
        .is_some_and(|u| {
            (1..=40).contains(&u.len()) && u.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        })
}

impl Model {
    pub(super) fn new(host: Option<String>) -> Self {
        Self {
            host,
            ..Self::default()
        }
    }

    /// Applies an event; returns the session whose blocks must be read, for one created or a
    /// block added.
    pub(super) fn event(&mut self, seq: u64, e: Tagged) -> Option<String> {
        match e.kind.as_str() {
            "session_created" => {
                let SessionRef { session_id, label } =
                    serde_json::from_value::<SessionCreated>(e.payload)
                        .ok()?
                        .session;
                self.add(&session_id, label, seq)?;
                Some(session_id)
            }
            "session_destroyed" => {
                let id = serde_json::from_value::<SessionId>(e.payload)
                    .ok()?
                    .session_id;
                self.sessions.remove(&id);
                self.gone.insert(id);
                None
            }
            "session_event" => {
                let se: SessionEvent = serde_json::from_value(e.payload).ok()?;
                self.session_event(seq, se)
            }
            _ => None,
        }
    }

    fn add(&mut self, id: &str, label: String, seq: u64) -> Option<()> {
        if !valid(id, "session") || self.gone.contains(id) {
            return None;
        }
        let session = self.sessions.entry(id.to_owned()).or_insert(Session {
            label: Stamped::default(),
            attached: false,
            blocks: BTreeMap::new(),
        });
        session.label.read(seq, label);
        Some(())
    }

    fn session_event(&mut self, seq: u64, se: SessionEvent) -> Option<String> {
        let session = self.sessions.get_mut(&se.session_id)?;
        let e = se.event;
        match e.kind.as_str() {
            "block_created" => return Some(se.session_id),
            "block_closed" => {
                let id = serde_json::from_value::<BlockId>(e.payload).ok()?.block_id;
                session.blocks.remove(&id);
                self.failed.remove(&id);
                self.gone.insert(id);
            }
            "session_label_changed" => {
                let label = serde_json::from_value::<Label>(e.payload).ok()?.label;
                session.label.event(seq, label);
            }
            "block_event" => {
                let be: BlockEvent = serde_json::from_value(e.payload).ok()?;
                let status_changed = be.name == "program_status_changed";
                let moved = status_changed || be.name == "process_changed";
                match session.blocks.get_mut(&be.block_id) {
                    Some(b) => {
                        let id = be.block_id.clone();
                        b.event(seq, be);
                        if status_changed && b.stale_foreground() {
                            self.stale_process
                                .insert((se.session_id.clone(), id.clone()));
                        }
                        if moved {
                            self.touched.insert((se.session_id, id));
                        }
                    }
                    None if self.failed.contains(&be.block_id) => return Some(se.session_id),
                    None => {}
                }
            }
            _ => {}
        }
        None
    }

    /// The sessions to attach and read: those listed, unless destroyed since.
    pub(super) fn sessions(&mut self, seq: u64, list: Vec<SessionRef>) -> Vec<String> {
        list.into_iter()
            .filter_map(|s| {
                self.add(&s.session_id, s.label, seq)?;
                Some(s.session_id)
            })
            .collect()
    }

    /// Whether `session` still needs a `session.attach`; marks it attached.
    pub(super) fn attach(&mut self, session: &str) -> bool {
        self.sessions
            .get_mut(session)
            .is_some_and(|s| !std::mem::replace(&mut s.attached, true))
    }

    pub(super) fn known(&self, session: &str) -> bool {
        self.sessions.contains_key(session)
    }

    /// The label `session.attach` answers: a rename that came before the session was listed
    /// found nothing to apply to.
    pub(super) fn relabel(&mut self, session: &str, seq: u64, label: String) {
        if let Some(s) = self.sessions.get_mut(session) {
            s.label.read(seq, label);
        }
    }

    /// A block that went away, or whose reads `failed` until its next report or a later read
    /// of its session adds it again; whether it newly failed.
    pub(super) fn forget(&mut self, session: &str, block: &str, failed: bool) -> bool {
        if let Some(s) = self.sessions.get_mut(session) {
            s.blocks.remove(block);
        }
        failed && self.failed.insert(block.to_owned())
    }

    pub(super) fn recovered(&mut self, block: &str) {
        self.failed.remove(block);
    }

    /// Adds the terminal blocks listed on this machine that were neither read nor closed
    /// since; returns them, for their reads. A block whose `host_id` is not the server's runs
    /// on another machine and is skipped, so the phone never reaches one through Rex.
    pub(super) fn blocks(&mut self, session: &str, list: Vec<BlockRef>) -> Vec<String> {
        let Some(s) = self.sessions.get_mut(session) else {
            return Vec::new();
        };
        let mut added = Vec::new();
        for b in list {
            let Some(id) = b.block_id else { continue };
            let here = self.host.is_some() && b.host_id == self.host;
            if here
                && b.creator_name.as_deref() == Some(TERMINAL)
                && valid(&id, "block")
                && !self.gone.contains(&id)
                && !s.blocks.contains_key(&id)
            {
                s.blocks.insert(id.clone(), Block::default());
                added.push(id);
            }
        }
        added
    }

    fn block(&mut self, session: &str, block: &str) -> Option<&mut Block> {
        self.sessions.get_mut(session)?.blocks.get_mut(block)
    }

    pub(super) fn status(&mut self, session: &str, block: &str, seq: u64, records: Vec<Record>) {
        if let Some(b) = self.block(session, block) {
            let root = records
                .into_iter()
                .find(|r| r.id.as_deref().is_none_or(str::is_empty));
            b.status.read(seq, root);
            self.touched.insert((session.to_owned(), block.to_owned()));
        }
    }

    pub(super) fn process(&mut self, session: &str, block: &str, seq: u64, p: Process) {
        if let Some(b) = self.block(session, block) {
            b.exited |= p.child.is_none() && p.last_exit.is_some();
            b.child = p.child;
            b.foreground.read(seq, p.foreground);
            b.under_shell.read(seq, p.ancestors.iter().any(shell));
            self.touched.insert((session.to_owned(), block.to_owned()));
        }
    }

    pub(super) fn title(&mut self, session: &str, block: &str, seq: u64, title: String) {
        if let Some(b) = self.block(session, block) {
            b.title.read(seq, title);
        }
    }

    pub(super) fn size(&mut self, session: &str, block: &str, seq: u64, size: Size) {
        if let Some(b) = self.block(session, block) {
            b.size.read(seq, cells(size));
        }
    }

    /// No read is in flight: what closed is gone from every answer to come.
    pub(super) fn settle(&mut self) {
        self.gone.clear();
    }

    pub(super) fn listing(&self) -> Listing {
        let mut listing = Listing::default();
        for (session_id, s) in &self.sessions {
            let mut agents = Vec::new();
            let mut terminals = Vec::new();
            for (block_id, b) in &s.blocks {
                match b.agent(session_id, block_id) {
                    Some(a) => agents.push(a),
                    None if b.runs_shell() => terminals.push(b.terminal(session_id, block_id)),
                    None => {}
                }
            }
            if agents.is_empty() && terminals.is_empty() {
                continue;
            }
            let label = Some(s.label.value.as_str()).filter(|l| !l.is_empty());
            listing.sessions.push(WorkspaceInfo {
                workspace_id: format!("{PREFIX}{session_id}"),
                number: 0,
                label: label.unwrap_or("Rex").to_owned(),
                active_tab_id: None,
                agent_status: agents::group_status(&agents).to_owned(),
                worktree: None,
            });
            listing.agents.extend(agents);
            listing.terminals.extend(terminals);
        }
        listing
    }

    /// The Claude Code agents among the blocks touched since the last call: their session,
    /// block and foreground pid.
    pub(super) fn touched(&mut self) -> Vec<(String, String, u32)> {
        let touched = std::mem::take(&mut self.touched);
        (touched.into_iter())
            .filter_map(|(session, block)| self.claude_of(session, block))
            .collect()
    }

    /// Every Claude Code agent: its session, block and foreground pid.
    pub(super) fn claude(&self) -> Vec<(String, String, u32)> {
        (self.sessions.iter())
            .flat_map(|(session, s)| s.blocks.keys().map(move |b| (session.clone(), b.clone())))
            .filter_map(|(session, block)| self.claude_of(session, block))
            .collect()
    }

    fn claude_of(&self, session: String, block: String) -> Option<(String, String, u32)> {
        let b = self.sessions.get(&session)?.blocks.get(&block)?;
        let a = b.agent(&session, &block)?;
        let pid = b.foreground.value.as_ref()?.pid;
        (a.agent.as_deref() == Some("claude")).then_some((session, block, pid))
    }

    /// The session, foreground pid and kind of a block listed as a Codex or Antigravity agent.
    pub(super) fn hook_agent(&self, block: &str) -> Option<(String, u32, &'static str)> {
        self.sessions.iter().find_map(|(session, s)| {
            let b = s.blocks.get(block)?;
            let a = b.agent(session, block)?;
            let pid = b.foreground.value.as_ref()?.pid;
            let kind = match a.agent.as_deref()? {
                "codex" => "codex",
                "agy" => "agy",
                _ => return None,
            };
            Some((session.clone(), pid, kind))
        })
    }

    /// Kept only while `pid` still leads the block: refused when another process leads it
    /// already, as when it changed while a pid file was read outside the lock.
    pub(super) fn bind(&mut self, session: &str, block: &str, pid: u32, value: String) {
        if let Some(b) = self.block(session, block)
            && b.foreground.value.as_ref().is_some_and(|f| f.pid == pid)
        {
            b.session = Some((pid, value));
        }
    }

    /// The session, block and size of a listed block, and what it is listed as; nothing else
    /// is ever reached.
    pub(super) fn target(&self, id: &str) -> Option<(String, String, (u16, u16), Listed)> {
        let block = id.strip_prefix(PREFIX)?;
        self.sessions.iter().find_map(|(session, s)| {
            let b = s.blocks.get(block)?;
            let listed = if b.agent(session, block).is_some() {
                Listed::Agent
            } else if b.shell() {
                Listed::Terminal
            } else {
                return None;
            };
            Some((session.clone(), block.to_owned(), b.size.value, listed))
        })
    }

    /// A listed agent's blocked report's kind (`permission`, `question`), standing in for
    /// herdr's rule id.
    pub(super) fn blocked_kind(&self, id: &str) -> Option<Option<String>> {
        let block = id.strip_prefix(PREFIX)?;
        self.sessions.iter().find_map(|(session, s)| {
            let b = s.blocks.get(block)?;
            let a = b.agent(session, block)?;
            let record = b
                .status
                .value
                .as_ref()
                .filter(|_| a.agent_status == "blocked");
            Some(record.and_then(|r| r.kind.clone()))
        })
    }

    pub(super) fn has_stale_process(&self) -> bool {
        !self.stale_process.is_empty()
    }

    pub(super) fn take_stale_process(&mut self) -> Option<(String, String)> {
        self.stale_process.pop_first()
    }

    pub(super) fn stale_target(&self, id: &str) -> Option<(String, String)> {
        let block = id.strip_prefix(PREFIX)?;
        self.sessions.iter().find_map(|(session, s)| {
            let b = s.blocks.get(block)?;
            b.stale_foreground()
                .then(|| (session.clone(), block.to_owned()))
        })
    }

    pub(super) fn idle_shell(&self, block: &str) -> Option<(String, String, (u16, u16))> {
        self.sessions.iter().find_map(|(session, s)| {
            let b = s.blocks.get(block).filter(|b| !b.exited)?;
            let fg = b.foreground.value.as_ref().filter(|f| shell(f))?;
            Some((session.clone(), fg.cwd.clone()?, b.size.value))
        })
    }

    pub(super) fn has(&self, block: &str) -> bool {
        self.sessions.values().any(|s| s.blocks.contains_key(block))
    }

    /// Known, or skipped until it reports again: either way still on the server.
    pub(super) fn holds(&self, block: &str) -> bool {
        self.has(block) || self.failed.contains(block)
    }

    pub(super) fn exited(&self, block: &str) -> bool {
        (self.sessions.values()).any(|s| s.blocks.get(block).is_some_and(|b| b.exited))
    }

    pub(super) fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }
}

impl Block {
    fn event(&mut self, seq: u64, e: BlockEvent) {
        match e.name.as_str() {
            "program_status_changed" => {
                if let Ok(c) = serde_json::from_value::<StatusChanged>(e.payload)
                    && c.id.is_empty()
                {
                    self.status.event(seq, Some(c.record));
                }
            }
            "program_status_removed" => {
                if let Ok(r) = serde_json::from_value::<StatusRemoved>(e.payload)
                    && r.ids.iter().any(String::is_empty)
                {
                    self.status.event(seq, None);
                }
            }
            "process_changed" => {
                if let Ok(p) = serde_json::from_value::<ProcessChanged>(e.payload) {
                    self.foreground.event(seq, p.foreground);
                    self.under_shell.event(seq, p.ancestors.iter().any(shell));
                }
            }
            "title_changed" => {
                if let Ok(t) = serde_json::from_value::<Title>(e.payload) {
                    self.title.event(seq, t.title);
                }
            }
            "child_exited" => self.exited = true,
            "size_changed" => {
                if let Ok(size) = serde_json::from_value::<Size>(e.payload) {
                    self.size.event(seq, cells(size));
                }
            }
            _ => {}
        }
    }

    fn stale_foreground(&self) -> bool {
        let Some(owner) = self.status.value.as_ref().and_then(|r| r.owner.as_ref()) else {
            return false;
        };
        !self
            .foreground
            .value
            .as_ref()
            .is_some_and(|fg| fg.pid == owner.pid && fg.name == owner.name)
    }

    /// A block that is no agent is a terminal while a shell runs in it, in front or above the
    /// program in front (`vim` started from zsh), so it stays listed while a command runs. A
    /// program Rex started with no shell is neither.
    fn shell(&self) -> bool {
        self.runs_shell() && !self.busy()
    }

    fn runs_shell(&self) -> bool {
        (self.foreground.value.as_ref()).is_some_and(|fg| shell(fg) || self.under_shell.value)
    }

    /// An agent under a shell that keeps the terminal (`sh -c 'cd x && claude'`) is no agent
    /// here, and Rex names that shell as its record's owner (checked live: an agent run from
    /// an interactive shell owns its own record, and no shell back at its prompt owned one).
    /// Shell input would reach such an agent past every check its own input takes, so the
    /// block is no terminal while a shell in front that is not a login shell owns the record,
    /// whatever it says, or while a live owner's record says working or blocked. A login shell
    /// (`-zsh`, as Rex.app starts one; `sh -c` never is) is left out of the first half, so a
    /// report ever credited to it cannot hide the user's own shell for as long as it lives. A
    /// wrapper suspended at idle or done keeps the shell listed for `fg`; a reused pid only
    /// hides a terminal.
    fn busy(&self) -> bool {
        let Some((r, owner)) =
            (self.status.value.as_ref()).and_then(|r| Some((r, r.owner.as_ref()?)))
        else {
            return false;
        };
        let front = (self.foreground.value.as_ref()).is_some_and(|f| {
            f.pid == owner.pid
                && f.name == owner.name
                && !f.argv0.as_deref().is_some_and(|a| a.starts_with('-'))
        });
        front
            || (matches!(r.state.as_str(), "working" | "blocked")
                && i32::try_from(owner.pid).is_ok_and(crate::transcript::alive))
    }

    /// A block is an agent once its foreground process reports OSC 7501 or runs an agent
    /// CLI. One with no status is read-only, since nothing says a dialog is open.
    fn agent(&self, session_id: &str, block_id: &str) -> Option<AgentInfo> {
        let fg = self.foreground.value.as_ref()?;
        let argv0 = fg.argv0.as_deref().unwrap_or(&fg.name);
        // A shell leads the terminal, so input would reach it: a status Rex still holds is
        // that of a program that exited, or of one a shell wrapper runs without handing it
        // the terminal.
        if shell(fg) {
            return None;
        }
        // Rex keeps a record after its program exits into another one; only the foreground
        // process's own report says what leads the terminal. Pid and name both, since pids
        // are reused.
        let record = (self.status.value.as_ref()).filter(|r| {
            r.owner
                .as_ref()
                .is_some_and(|o| o.pid == fg.pid && o.name == fg.name)
        });
        let cli = agents::cli(argv0);
        let kind = match record {
            Some(r) => agents::kind(r.app.as_deref(), cli),
            None => Some(cli?.to_owned()),
        };
        let reported = record.and_then(|r| agents::kind(r.app.as_deref(), None));
        let read_only = agents::read_only(reported.as_deref(), cli);
        let id = format!("{PREFIX}{block_id}");
        let some = |s: &str| (!s.is_empty()).then(|| s.to_owned());
        Some(AgentInfo {
            terminal_id: id.clone(),
            workspace_id: format!("{PREFIX}{session_id}"),
            pane_id: id,
            agent: kind,
            name: None,
            title: some(agents::title(&self.title.value)),
            terminal_title_stripped: None,
            agent_status: agents::status(record.map(|r| r.state.as_str())).to_owned(),
            state_change_seq: record.map_or(0, |r| change_seq(&r.updated_at)),
            cwd: self.child.as_ref().and_then(|c| c.cwd.clone()),
            foreground_cwd: fg.cwd.clone(),
            agent_session: (self.session.as_ref())
                .filter(|(pid, _)| *pid == fg.pid)
                .map(|(_, value)| AgentSession {
                    value: value.clone(),
                }),
            interactive_ready: false,
            launch_pending: false,
            read_only,
            unreported: record.is_none(),
            cli: cli.map(str::to_owned),
            reported,
        })
    }

    /// A program's title never leaves the machine, as for herdr's shells. A busy block is a
    /// pane hosting an agent, so the phone's terminal calls drop their grants on it, as on a
    /// herdr pane where an agent starts, and Terminals leaves it out.
    fn terminal(&self, session_id: &str, block_id: &str) -> PaneInfo {
        let id = format!("{PREFIX}{block_id}");
        let workspace_id = format!("{PREFIX}{session_id}");
        PaneInfo {
            pane_id: id.clone(),
            terminal_id: id,
            tab_id: workspace_id.clone(),
            workspace_id,
            // Rex.app's `login` has no folder; archive's cleanup runs where the shell is.
            cwd: (self.child.as_ref().and_then(|c| c.cwd.clone()))
                .or_else(|| self.foreground.value.as_ref().and_then(|f| f.cwd.clone())),
            foreground_cwd: (self.foreground.value.as_ref()).and_then(|f| f.cwd.clone()),
            agent: self.busy().then(|| {
                let app = self.status.value.as_ref().and_then(|r| r.app.as_deref());
                agents::kind(app, None).unwrap_or_else(|| "unknown".to_owned())
            }),
            label: None,
        }
    }
}

/// What a listed block is to the phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Listed {
    Agent,
    Terminal,
}

fn shell(p: &Proc) -> bool {
    agents::shell(p.argv0.as_deref().unwrap_or(&p.name))
}

fn cells(size: Size) -> (u16, u16) {
    let cap = |n: u32| u16::try_from(n).unwrap_or(u16::MAX);
    (cap(size.columns), cap(size.rows))
}

/// collied's own `state_change_seq`, since Rex's `seq` restarts with each connection: from
/// the time of the report, which each report sets anew. Two identical dialogs in a row never
/// share one, and a record unchanged across a reconnect or a restart of collied keeps its own.
fn change_seq(updated_at: &str) -> u64 {
    let digest = Sha256::digest(updated_at.as_bytes());
    let mut seq = [0; 8];
    seq.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(seq)
}

/// What a followed Rex lists: its agents, its terminals (a busy one as a pane hosting an
/// agent), and one group per session that holds either.
#[derive(Default)]
pub struct Listing {
    pub(crate) agents: Vec<AgentInfo>,
    pub(crate) terminals: Vec<PaneInfo>,
    sessions: Vec<WorkspaceInfo>,
}

impl Listing {
    /// Numbered after `after`, herdr's and herdr-core's last.
    pub fn groups(&self, after: u32) -> Vec<WorkspaceInfo> {
        (self.sessions.iter().zip(after + 1..))
            .map(|(w, number)| WorkspaceInfo {
                number,
                ..w.clone()
            })
            .collect()
    }
}
