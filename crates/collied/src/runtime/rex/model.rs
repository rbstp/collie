use std::collections::{BTreeMap, HashSet};

use sha2::{Digest, Sha256};

use super::client::TERMINAL;
use super::wire::{
    BlockEvent, BlockId, BlockRef, Label, Proc, Process, ProcessChanged, Record, SessionCreated,
    SessionEvent, SessionId, SessionRef, StatusChanged, StatusRemoved, Tagged, Title,
};
use crate::runtime::{AgentInfo, WorkspaceInfo, agents};

pub(super) const PREFIX: &str = "rex:";

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
    child: Option<Proc>,
    title: Stamped<String>,
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
}

/// Rex's ids are `session:<uuid>` and `block:<uuid>`; anything else is skipped, so every id
/// collied makes fits the phone's terminal ids.
fn valid(id: &str, kind: &str) -> bool {
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
                self.gone.insert(id);
            }
            "session_label_changed" => {
                let label = serde_json::from_value::<Label>(e.payload).ok()?.label;
                session.label.event(seq, label);
            }
            "block_event" => {
                let be: BlockEvent = serde_json::from_value(e.payload).ok()?;
                session.blocks.get_mut(&be.block_id)?.event(seq, be);
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
        }
    }

    pub(super) fn process(&mut self, session: &str, block: &str, seq: u64, p: Process) {
        if let Some(b) = self.block(session, block) {
            b.child = p.child;
            b.foreground.read(seq, p.foreground);
        }
    }

    pub(super) fn title(&mut self, session: &str, block: &str, seq: u64, title: String) {
        if let Some(b) = self.block(session, block) {
            b.title.read(seq, title);
        }
    }

    /// No read is in flight: what closed is gone from every answer to come.
    pub(super) fn settle(&mut self) {
        self.gone.clear();
    }

    pub(super) fn listing(&self) -> Listing {
        let mut listing = Listing::default();
        for (session_id, s) in &self.sessions {
            let agents: Vec<AgentInfo> = (s.blocks.iter())
                .filter_map(|(block_id, b)| b.agent(session_id, block_id))
                .collect();
            if !agents.is_empty() {
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
            }
        }
        listing
    }

    /// The session and block of a listed agent; nothing else is ever reached.
    pub(super) fn target(&self, id: &str) -> Option<(String, String)> {
        let block = id.strip_prefix(PREFIX)?;
        self.sessions.iter().find_map(|(session, s)| {
            let b = s.blocks.get(block)?;
            b.agent(session, block)?;
            Some((session.clone(), block.to_owned()))
        })
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
                }
            }
            "title_changed" => {
                if let Ok(t) = serde_json::from_value::<Title>(e.payload) {
                    self.title.event(seq, t.title);
                }
            }
            _ => {}
        }
    }

    /// A block is an agent once its foreground process reports OSC 7501 or runs an agent
    /// CLI. Every Rex agent is read-only until input is built (#706).
    fn agent(&self, session_id: &str, block_id: &str) -> Option<AgentInfo> {
        let fg = self.foreground.value.as_ref()?;
        let argv0 = fg.argv0.as_deref().unwrap_or(&fg.name);
        // A shell leads the terminal, so input would reach it: a status Rex still holds is
        // that of a program that exited, or of one a shell wrapper runs without handing it
        // the terminal.
        if agents::shell(argv0) {
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
            agent_session: None,
            interactive_ready: false,
            launch_pending: false,
            read_only: true,
        })
    }
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

/// What a followed Rex lists: its agents, and one group per session that holds one.
#[derive(Default)]
pub struct Listing {
    pub(crate) agents: Vec<AgentInfo>,
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
