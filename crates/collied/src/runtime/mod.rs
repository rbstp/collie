use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::Deserialize;

mod agents;
pub mod herdr;
pub mod herdr_core;
pub mod rex;
pub mod screen;

use self::herdr_core::Core;
use self::rex::Rex;

pub use agents::pane;
pub use herdr::{Error, PaneRead};

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    pub number: u32,
    pub label: String,
    pub active_tab_id: Option<String>,
    pub agent_status: String,
    #[serde(default)]
    pub worktree: Option<WorkspaceWorktreeInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceWorktreeInfo {
    pub repo_root: String,
    pub checkout_path: String,
    pub is_linked_worktree: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaneInfo {
    pub pane_id: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub cwd: Option<String>,
    pub foreground_cwd: Option<String>,
    /// Absent on a pane with no agent (herdr 0.9.3).
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AgentSession {
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentInfo {
    pub terminal_id: String,
    pub workspace_id: String,
    pub pane_id: String,
    pub agent: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub terminal_title_stripped: Option<String>,
    pub agent_status: String,
    #[serde(default)]
    pub state_change_seq: u64,
    pub cwd: Option<String>,
    pub foreground_cwd: Option<String>,
    pub agent_session: Option<AgentSession>,
    #[serde(default)]
    pub interactive_ready: bool,
    #[serde(default)]
    pub launch_pending: bool,
    /// A herdr-core agent with no OSC 7501 status: nothing tells collied a dialog is open
    /// (Copilot's folder trust prompt reports nothing), so it takes no input from the phone.
    #[serde(skip)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionSnapshot {
    pub workspaces: Vec<WorkspaceInfo>,
    pub panes: Vec<PaneInfo>,
    pub agents: Vec<AgentInfo>,
}

pub struct Poll {
    pub agents: Vec<AgentInfo>,
    pub workspaces: Vec<WorkspaceInfo>,
    /// False when a followed core or Rex answered in herdr's place: herdr's agents are then
    /// not known to be gone, and what collied keeps for them must stay.
    pub herdr: bool,
}

pub struct PaneProcesses {
    pub pids: Vec<u32>,
    pub attach: Option<String>,
}

/// herdr, and beside it a herdr-core core and a Rex server when they are followed. Calls are
/// routed by id: `hc:` ids are herdr-core's, `rex:` ids Rex's, and neither reaches herdr.
#[derive(Clone)]
pub struct Runtime {
    herdr: PathBuf,
    core: Option<Arc<Core>>,
    rex: Option<Arc<Rex>>,
}

/// herdr's reachability as calls merged with a followed runtime saw it, so that losing herdr
/// while another runtime answers is logged once, and a herdr that never answered stays quiet.
/// One is shared by the core and Rex, whichever answers.
#[derive(Default)]
pub struct HerdrSeen(AtomicU8);

impl HerdrSeen {
    fn seen(&self, error: Option<&Error>) {
        const UP: u8 = 1;
        const DOWN: u8 = 2;
        match error {
            None => {
                if self.0.swap(UP, Ordering::Relaxed) == DOWN {
                    tracing::info!("herdr reachable again");
                }
            }
            Some(e) => {
                if (self.0)
                    .compare_exchange(UP, DOWN, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    tracing::warn!(error = %e, "herdr unavailable, herdr-core or Rex still answers");
                }
            }
        }
    }
}

/// The agents of the runtimes followed beside herdr.
struct Followed {
    core: Vec<AgentInfo>,
    rex: Option<rex::Listing>,
}

impl Followed {
    /// herdr-core's group, then one per Rex session, numbered after herdr's.
    fn groups(&self, workspaces: &mut Vec<WorkspaceInfo>) {
        workspaces.extend(herdr_core::group(&self.core, workspaces));
        if let Some(rex) = &self.rex {
            let after = workspaces.iter().map(|w| w.number).max().unwrap_or(0);
            workspaces.extend(rex.groups(after));
        }
    }

    fn agents(self) -> Vec<AgentInfo> {
        let mut agents = self.core;
        agents.extend(self.rex.into_iter().flat_map(|l| l.agents));
        agents
    }
}

pub fn herdr_socket(session: Option<&str>) -> Result<PathBuf, herdr::InvalidSession> {
    herdr::resolve_socket_path(session, &herdr::SocketEnv::from_process())
}

pub fn herdr_session_label(session: Option<&str>) -> String {
    herdr::session_label(session, &herdr::SocketEnv::from_process())
}

/// herdr is not running at all: no socket, or nothing listening on it. Any other failure
/// may be a herdr that is busy or restarting.
pub fn not_running(e: &Error) -> bool {
    matches!(e, Error::Io(io) if matches!(
        io.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    ))
}

pub const NOT_YET: &str = "not available for Herdr 2 agents yet";
/// Claude Code's ctrl+end, bound to `scroll:bottom`; herdr 0.9.3 has no key name for it.
const SCROLL_BOTTOM: &str = "\u{1b}[1;5F";

/// The runtime an id belongs to, by its prefix; herdr's ids have none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Herdr,
    Core,
    Rex,
}

impl Owner {
    pub fn of(id: &str) -> Self {
        if herdr_core::owns(id) {
            Self::Core
        } else if rex::owns(id) {
            Self::Rex
        } else {
            Self::Herdr
        }
    }
}

fn rex_not_yet<T>() -> Result<T, Error> {
    Err(Error::Unsupported(rex::NOT_YET.to_owned()))
}

fn herdr_only(id: &str) -> Result<(), Error> {
    match Owner::of(id) {
        Owner::Herdr => Ok(()),
        Owner::Core => Err(Error::Unsupported(NOT_YET.to_owned())),
        Owner::Rex => rex_not_yet(),
    }
}

impl Runtime {
    pub fn new(herdr: PathBuf) -> Self {
        Self {
            herdr,
            core: None,
            rex: None,
        }
    }

    pub fn with_core(mut self, core: Option<Arc<Core>>) -> Self {
        self.core = core;
        self
    }

    pub fn core(&self) -> Option<&Arc<Core>> {
        self.core.as_ref()
    }

    pub fn with_rex(mut self, rex: Option<Arc<Rex>>) -> Self {
        self.rex = rex;
        self
    }

    pub fn rex(&self) -> Option<&Arc<Rex>> {
        self.rex.as_ref()
    }

    fn followed_rex(&self) -> Result<&Arc<Rex>, Error> {
        self.rex
            .as_ref()
            .ok_or_else(|| Error::Unsupported(rex::NOT_YET.to_owned()))
    }

    /// Where a new task starts: herdr while it answers, which gives the agent herdr's screen
    /// rules, focus and workspace; the followed core only when herdr is not running. A herdr
    /// that runs but does not answer is an error, not a reason to start in the core.
    pub async fn task_core(&self) -> Result<Option<Arc<Core>>, Error> {
        let Some(core) = self.core.as_ref().filter(|c| c.following()) else {
            return Ok(None);
        };
        match herdr::ping(&self.herdr).await {
            Ok(_) => Ok(None),
            Err(e) if not_running(&e) => Ok(Some(core.clone())),
            Err(e) => Err(e),
        }
    }

    fn followed(&self) -> Result<&Arc<Core>, Error> {
        self.core
            .as_ref()
            .ok_or_else(|| Error::Unsupported(NOT_YET.to_owned()))
    }

    /// herdr's answer, plus the agents of the core and of Rex while they are followed, from
    /// what their events keep current. With neither followed this is herdr's answer
    /// unchanged, its error included.
    fn merge<T: Default>(
        &self,
        herdr: Result<T, Error>,
        add: impl FnOnce(&mut T, Followed),
    ) -> Result<T, Error> {
        let core = self.core.as_ref().and_then(|c| Some((c, c.agents()?)));
        let rex = self.rex.as_ref().and_then(|r| Some((r, r.listing()?)));
        let seen = match (&core, &rex) {
            (Some((c, _)), _) => &c.herdr,
            (None, Some((r, _))) => &r.herdr,
            (None, None) => return herdr,
        };
        seen.seen(herdr.as_ref().err());
        let mut merged = herdr.unwrap_or_default();
        let followed = Followed {
            core: core.map(|(_, agents)| agents).unwrap_or_default(),
            rex: rex.map(|(_, listing)| listing),
        };
        add(&mut merged, followed);
        Ok(merged)
    }

    pub async fn ping(&self) -> Result<herdr::Pong, Error> {
        herdr::ping(&self.herdr).await
    }

    pub async fn session_snapshot(&self) -> Result<SessionSnapshot, Error> {
        let herdr = herdr::session_snapshot(&self.herdr).await;
        self.merge(herdr, |snap, followed| {
            followed.groups(&mut snap.workspaces);
            let agents = followed.agents();
            snap.panes.extend(agents.iter().map(pane));
            snap.agents.extend(agents);
        })
    }

    pub async fn agent_list(&self) -> Result<Vec<AgentInfo>, Error> {
        let herdr = herdr::agent_list(&self.herdr).await;
        self.merge(herdr, |list, followed| list.extend(followed.agents()))
    }

    pub async fn poll(&self) -> Result<Poll, Error> {
        let herdr = tokio::try_join!(
            herdr::agent_list(&self.herdr),
            herdr::workspace_list(&self.herdr)
        );
        let answered = herdr.is_ok();
        let (agents, workspaces) = self.merge(herdr, |(list, workspaces), followed| {
            followed.groups(workspaces);
            list.extend(followed.agents());
        })?;
        Ok(Poll {
            agents,
            workspaces,
            herdr: answered,
        })
    }

    /// The agents of the runtime `id` belongs to, so a herdr agent's lookup still fails with
    /// herdr's error while herdr is down.
    pub async fn agents_of(&self, id: &str) -> Result<Vec<AgentInfo>, Error> {
        match Owner::of(id) {
            Owner::Herdr => herdr::agent_list(&self.herdr).await,
            Owner::Core | Owner::Rex => self.agent_list().await,
        }
    }

    pub async fn snapshot_of(&self, id: &str) -> Result<SessionSnapshot, Error> {
        match Owner::of(id) {
            Owner::Herdr => herdr::session_snapshot(&self.herdr).await,
            Owner::Core | Owner::Rex => self.session_snapshot().await,
        }
    }

    /// herdr's own snapshot, without the core's agents, its error included.
    pub async fn herdr_snapshot(&self) -> Result<SessionSnapshot, Error> {
        herdr::session_snapshot(&self.herdr).await
    }

    pub async fn agent_manifests(&self) -> Result<Vec<String>, Error> {
        herdr::agent_manifests(&self.herdr).await
    }

    pub async fn agent_get(&self, target: &str) -> Result<AgentInfo, Error> {
        match Owner::of(target) {
            Owner::Herdr => herdr::agent_get(&self.herdr, target).await,
            Owner::Core => self.followed()?.get(target).await,
            Owner::Rex => self.followed_rex()?.get(target),
        }
    }

    pub async fn pane_get(&self, pane_id: &str) -> Result<PaneInfo, Error> {
        herdr_only(pane_id)?;
        herdr::pane_get(&self.herdr, pane_id).await
    }

    /// A herdr-core terminal's own width; herdr's is its layout rect's.
    pub async fn pane_columns(&self, pane_id: &str) -> Result<Option<u16>, Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::pane_columns(&self.herdr, pane_id).await,
            Owner::Core => self.followed()?.columns(pane_id),
            Owner::Rex => self.followed_rex()?.columns(pane_id),
        }
    }

    pub async fn agent_read(
        &self,
        pane_id: &str,
        source: &str,
        lines: Option<u32>,
    ) -> Result<PaneRead, Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::agent_read(&self.herdr, pane_id, source, lines).await,
            Owner::Core => self.followed()?.read(pane_id, source, lines).await,
            Owner::Rex => self.followed_rex()?.read(pane_id, source, lines).await,
        }
    }

    pub async fn pane_read(
        &self,
        pane_id: &str,
        source: &str,
        lines: Option<u32>,
    ) -> Result<PaneRead, Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::pane_read(&self.herdr, pane_id, source, lines).await,
            Owner::Core => self.followed()?.read(pane_id, source, lines).await,
            Owner::Rex => self.followed_rex()?.read(pane_id, source, lines).await,
        }
    }

    /// Keeps a herdr-core terminal attached until dropped, so a watch's reads come from one
    /// emulated screen; herdr and Rex need nothing kept, their reads are polled.
    pub fn watch(&self, pane_id: &str) -> Result<Option<herdr_core::Watch>, Error> {
        match Owner::of(pane_id) {
            Owner::Herdr | Owner::Rex => Ok(None),
            Owner::Core => self.followed()?.watch(pane_id).map(Some),
        }
    }

    pub async fn agent_explain(&self, target: &str) -> Result<herdr::Explain, Error> {
        match Owner::of(target) {
            Owner::Herdr => herdr::agent_explain(&self.herdr, target).await,
            Owner::Core => self.followed()?.explain(target),
            Owner::Rex => self.followed_rex()?.explain(target),
        }
    }

    pub async fn detection_text(&self, pane_id: &str) -> Result<String, Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::detection_text(&self.herdr, pane_id).await,
            Owner::Core => self.followed()?.screen_text(pane_id).await,
            Owner::Rex => self.followed_rex()?.screen_text(pane_id).await,
        }
    }

    /// `seen` is the status the ready check saw, which a herdr-core send requires again.
    pub async fn agent_prompt(&self, pane_id: &str, text: &str, seen: &str) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::agent_prompt(&self.herdr, pane_id, text).await,
            Owner::Core => self.followed()?.send(pane_id, text, seen, false).await,
            Owner::Rex => rex_not_yet(),
        }
    }

    /// A typed answer: a paste and Enter, also into a blocked agent's dialog.
    pub async fn type_text(&self, pane_id: &str, text: &str, seen: &str) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => {
                let paste = format!("\u{1b}[200~{text}\u{1b}[201~");
                herdr::pane_send_input(&self.herdr, pane_id, &paste, &["enter"]).await
            }
            Owner::Core => {
                let blocked = seen == "blocked";
                self.followed()?.send(pane_id, text, seen, blocked).await
            }
            Owner::Rex => rex_not_yet(),
        }
    }

    pub async fn agent_send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::agent_send_keys(&self.herdr, pane_id, keys).await,
            Owner::Core => self.followed()?.keys(pane_id, keys).await,
            Owner::Rex => rex_not_yet(),
        }
    }

    pub async fn pane_send_text(&self, pane_id: &str, text: &str) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::pane_send_text(&self.herdr, pane_id, text).await,
            Owner::Core => self.followed()?.type_raw(pane_id, text).await,
            Owner::Rex => rex_not_yet(),
        }
    }

    pub async fn scroll_bottom(&self, pane_id: &str) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::pane_send_text(&self.herdr, pane_id, SCROLL_BOTTOM).await,
            Owner::Core => self.followed()?.keys(pane_id, &["ctrl+end"]).await,
            Owner::Rex => rex_not_yet(),
        }
    }

    /// A notice option's digit, without Enter.
    pub async fn notice_digit(&self, pane_id: &str, digit: &str) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::pane_send_text(&self.herdr, pane_id, digit).await,
            Owner::Core => self.followed()?.keys(pane_id, &[digit]).await,
            Owner::Rex => rex_not_yet(),
        }
    }

    pub async fn pane_send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<(), Error> {
        herdr_only(pane_id)?;
        herdr::pane_send_keys(&self.herdr, pane_id, keys).await
    }

    pub async fn pane_send_input(
        &self,
        pane_id: &str,
        text: &str,
        keys: &[&str],
    ) -> Result<(), Error> {
        herdr_only(pane_id)?;
        herdr::pane_send_input(&self.herdr, pane_id, text, keys).await
    }

    pub async fn agent_focus(&self, pane_id: &str) -> Result<(), Error> {
        herdr_only(pane_id)?;
        herdr::agent_focus(&self.herdr, pane_id).await
    }

    pub async fn agent_start(
        &self,
        name: &str,
        kind: &str,
        pane_id: &str,
        resume: Option<&str>,
        ultracode: bool,
    ) -> Result<AgentInfo, Error> {
        herdr::agent_start(&self.herdr, name, kind, pane_id, resume, ultracode).await
    }

    pub async fn workspace_create(
        &self,
        cwd: &str,
        label: Option<&str>,
    ) -> Result<herdr::WorkspaceCreated, Error> {
        herdr::workspace_create(&self.herdr, cwd, label).await
    }

    pub async fn tab_create(&self, workspace_id: &str, cwd: &str) -> Result<(), Error> {
        herdr_only(workspace_id)?;
        herdr::tab_create(&self.herdr, workspace_id, cwd).await
    }

    pub async fn worktree_list(&self, cwd: &str) -> Result<herdr::WorktreeList, Error> {
        herdr::worktree_list(&self.herdr, cwd).await
    }

    pub async fn worktree_create(
        &self,
        cwd: &str,
        branch: &str,
        path: &str,
        label: Option<&str>,
    ) -> Result<herdr::WorktreeWorkspace, Error> {
        herdr::worktree_create(&self.herdr, cwd, branch, path, label).await
    }

    pub async fn worktree_open(
        &self,
        cwd: &str,
        path: &str,
        label: Option<&str>,
    ) -> Result<herdr::WorktreeWorkspace, Error> {
        herdr::worktree_open(&self.herdr, cwd, path, label).await
    }

    pub async fn worktree_remove(&self, workspace_id: &str) -> Result<herdr::WorktreeInfo, Error> {
        herdr_only(workspace_id)?;
        herdr::worktree_remove(&self.herdr, workspace_id).await
    }

    /// A herdr-core group is a label, not a workspace: closing it would end every terminal in
    /// the core, a desktop app's included, so it is refused.
    pub async fn workspace_close(&self, workspace_id: &str) -> Result<(), Error> {
        match Owner::of(workspace_id) {
            Owner::Herdr => herdr::workspace_close(&self.herdr, workspace_id).await,
            Owner::Core => Err(Error::Unsupported(
                "a Herdr 2 group cannot be closed; close its agents one at a time".to_owned(),
            )),
            Owner::Rex => Err(Error::Unsupported(rex::GROUP_CLOSE.to_owned())),
        }
    }

    pub async fn pane_processes(&self, pane_id: &str) -> Result<PaneProcesses, Error> {
        herdr_only(pane_id)?;
        herdr::pane_processes(&self.herdr, pane_id).await
    }

    pub async fn pane_close(&self, pane_id: &str) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::pane_close(&self.herdr, pane_id).await,
            Owner::Core => self.followed()?.close(pane_id).await,
            Owner::Rex => self.followed_rex()?.close(pane_id).await,
        }
    }
}
