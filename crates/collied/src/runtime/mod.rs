use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::Deserialize;

mod agents;
mod ghostty;
pub mod herdr;
pub mod herdr_core;
pub mod rex;
pub mod screen;

use self::herdr_core::Core;
use self::rex::Rex;

pub use agents::{STRIPPED, TASK_KINDS, pane, task_cli};
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
    /// A herdr-core or Rex agent whose status cannot tell collied a dialog is open
    /// ([`agents::read_only`]), so it takes no input from the phone.
    #[serde(skip)]
    pub read_only: bool,
    /// A herdr-core or Rex agent with no status report, whose kind is its foreground CLI's.
    #[serde(skip)]
    pub unreported: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionSnapshot {
    /// herdr's server-side focus, which follows the foreground client and stays set with no
    /// client attached.
    #[serde(default)]
    pub focused_pane_id: Option<String>,
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

/// A terminal a followed core or Rex holds, shell or agent, with the folders it works in.
pub struct Occupant {
    pub id: String,
    pub task: bool,
    /// The checkout collied recorded for the worktree task it started there.
    pub worktree: Option<String>,
    pub cwd: String,
    pub foreground_cwd: Option<String>,
}

/// Where a new task starts when herdr is not running: a followed core, else a followed Rex.
#[derive(Clone)]
pub enum TaskHost {
    Core(Arc<Core>),
    Rex(Arc<Rex>),
}

impl TaskHost {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Core(_) => herdr_core::GROUP_LABEL,
            Self::Rex(_) => rex::NAME,
        }
    }

    /// A task agent in a terminal of its own, started as `task_argv` builds it; returns its id.
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        &self,
        cwd: &str,
        name: &str,
        label: Option<&str>,
        kind: &str,
        worktree: Option<&str>,
        cli: &str,
        args: &[String],
        code_mode: Option<bool>,
    ) -> Result<String, Error> {
        match self {
            Self::Core(core) => {
                core.start(cwd, name, kind, worktree, cli, args, code_mode)
                    .await
            }
            Self::Rex(rex) => {
                let argv = agents::task_argv(cli, args, code_mode);
                rex.start_task(cwd, name, label, worktree, argv).await
            }
        }
    }

    pub fn finished(&self, id: &str) -> bool {
        match self {
            Self::Core(core) => core.finished(id),
            Self::Rex(rex) => rex.finished(id),
        }
    }

    pub fn owner(&self) -> Owner {
        match self {
            Self::Core(_) => Owner::Core,
            Self::Rex(_) => Owner::Rex,
        }
    }
}

pub struct PaneProcesses {
    pub pids: Vec<u32>,
    pub attach: Option<String>,
}

/// What keeps a watched terminal followed; dropping it ends the follow with the last watch.
pub enum Watch {
    Core(herdr_core::Watch),
    Rex(rex::Watch),
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

/// The agents of the runtimes followed beside herdr, and Rex's terminals.
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

    fn terminals(&self) -> Vec<PaneInfo> {
        self.rex.iter().flat_map(|l| l.terminals.clone()).collect()
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

pub const NOT_YET: &str = "not available for Herdr Core agents yet";

fn reconnecting(name: &str) -> String {
    format!("{name} is reconnecting; try again in a moment")
}
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

    pub fn runtime(self) -> protocol::TaskRuntime {
        match self {
            Self::Herdr => protocol::TaskRuntime::Herdr,
            Self::Core => protocol::TaskRuntime::HerdrCore,
            Self::Rex => protocol::TaskRuntime::Rex,
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

    /// Where herdr's clients attach: herdr 0.9.5 puts it beside the API socket.
    pub fn herdr_client_socket(&self) -> PathBuf {
        self.herdr.with_file_name("herdr-client.sock")
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

    /// A followed core or Rex, which can take a task the phone asks of it whatever herdr does.
    pub fn follows_another(&self) -> bool {
        self.core.as_ref().is_some_and(|c| c.following())
            || self.rex.as_ref().is_some_and(|r| r.following())
    }

    fn followed_rex(&self) -> Result<&Arc<Rex>, Error> {
        self.rex
            .as_ref()
            .ok_or_else(|| Error::Unsupported(rex::NOT_YET.to_owned()))
    }

    /// Where a new task starts: herdr while it answers, which gives the agent herdr's screen
    /// rules, focus and workspace; when herdr is not running, the followed core, else the
    /// followed Rex. A herdr that runs but does not answer is an error, not a reason to start
    /// elsewhere.
    ///
    /// A core whose connection dropped and that collied has not found gone since is the same
    /// error, not a reason to start in Rex.
    ///
    /// A `chosen` runtime is the only place the task may start: when it cannot, the error
    /// names it and says why, and nothing else is asked.
    pub async fn task_host(&self, chosen: Option<Owner>) -> Result<Option<TaskHost>, Error> {
        let core = self.core.as_ref();
        let waiting = core.is_some_and(|c| c.reconnecting());
        let unavailable = |name: &str, reconnects: bool, refused: Option<String>| {
            Error::Unavailable(match refused {
                _ if reconnects => reconnecting(name),
                Some(why) => format!("collied does not follow {name} on this computer: {why}"),
                None => format!("{name} is not running on this computer"),
            })
        };
        match chosen {
            None => {}
            Some(Owner::Herdr) => {
                return match herdr::ping(&self.herdr).await {
                    Ok(_) => Ok(None),
                    Err(e) if not_running(&e) => Err(unavailable("Herdr", false, None)),
                    Err(e) => Err(e),
                };
            }
            Some(Owner::Core) => {
                return match core {
                    Some(c) if c.following() => Ok(Some(TaskHost::Core(c.clone()))),
                    _ => Err(unavailable(
                        herdr_core::GROUP_LABEL,
                        waiting,
                        core.and_then(|c| c.refused()),
                    )),
                };
            }
            Some(Owner::Rex) => {
                let rex = self.rex.as_ref();
                return match rex {
                    Some(r) if r.following() => Ok(Some(TaskHost::Rex(r.clone()))),
                    _ => Err(unavailable(
                        rex::NAME,
                        rex.is_some_and(|r| r.reconnecting()),
                        rex.and_then(|r| r.refused()),
                    )),
                };
            }
        }
        let rex = (self.rex.as_ref()).filter(|r| r.following() && !waiting);
        let host = (core.filter(|c| c.following()))
            .map(|c| TaskHost::Core(c.clone()))
            .or_else(|| rex.map(|r| TaskHost::Rex(r.clone())));
        if host.is_none() && !waiting {
            return Ok(None);
        }
        match herdr::ping(&self.herdr).await {
            Ok(_) => Ok(None),
            Err(e) if not_running(&e) => host
                .map(Some)
                .ok_or_else(|| Error::Unsupported(reconnecting(herdr_core::GROUP_LABEL))),
            Err(e) => Err(e),
        }
    }

    /// Every terminal of the followed core and Rex. `owner`'s runtime is required (neither is
    /// for `Owner::Herdr`), and the other one counts while it is followed. Fails, naming the
    /// runtime, while a required one is not followed or a counted one does not answer, when
    /// nothing is known about what works where.
    pub async fn occupants(&self, owner: Owner) -> Result<Vec<Occupant>, String> {
        let not_followed = |name: &str| format!("{name} is not followed right now");
        let core = (self.core.as_ref()).filter(|c| owner == Owner::Core || c.following());
        let rex = (self.rex.as_ref()).filter(|r| owner == Owner::Rex || r.following());
        let mut all = match core {
            Some(core) => core.occupants().ok_or_else(|| not_followed("herdr-core"))?,
            None if owner == Owner::Core => return Err(not_followed("herdr-core")),
            None => Vec::new(),
        };
        match rex {
            Some(rex) => all.extend(rex.occupants().await.map_err(|e| match e {
                Error::Closed => not_followed(rex::NAME),
                e => format!("Rex did not list its blocks ({e})"),
            })?),
            None if owner == Owner::Rex => return Err(not_followed(rex::NAME)),
            None => {}
        }
        Ok(all)
    }

    /// The checkout recorded for the worktree task collied started in `id`.
    pub fn task_worktree(&self, id: &str) -> Option<String> {
        match Owner::of(id) {
            Owner::Herdr => None,
            Owner::Core => self.core.as_ref()?.worktree(id),
            Owner::Rex => self.rex.as_ref()?.worktree(id),
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
            snap.panes.extend(followed.terminals());
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

    /// Keeps a herdr-core terminal attached, or a Rex terminal followed as a stream, until
    /// dropped, so a watch's reads come from one screen; herdr's reads are polled.
    pub fn watch(&self, pane_id: &str) -> Result<Option<Watch>, Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => Ok(None),
            Owner::Core => Ok(Some(Watch::Core(self.followed()?.watch(pane_id)?))),
            Owner::Rex => Ok(self.followed_rex()?.watch(pane_id)?.map(Watch::Rex)),
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

    /// `seen` is the status the ready check saw, which a herdr-core or Rex send requires again.
    pub async fn agent_prompt(&self, pane_id: &str, text: &str, seen: &str) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::agent_prompt(&self.herdr, pane_id, text).await,
            Owner::Core => self.followed()?.send(pane_id, text, seen, false).await,
            Owner::Rex => {
                let seen = rex::Seen {
                    status: seen,
                    seq: None,
                };
                (self.followed_rex()?)
                    .paste(pane_id, text, true, Some(&seen))
                    .await
            }
        }
    }

    /// A typed answer: a paste and Enter, also into a blocked agent's dialog, whose
    /// `state_change_seq` was `seq` when checked.
    pub async fn type_text(
        &self,
        pane_id: &str,
        text: &str,
        seen: &str,
        seq: u64,
    ) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => {
                let paste = format!("\u{1b}[200~{text}\u{1b}[201~");
                herdr::pane_send_input(&self.herdr, pane_id, &paste, &["enter"]).await
            }
            Owner::Core => {
                let blocked = seen == "blocked";
                self.followed()?.send(pane_id, text, seen, blocked).await
            }
            Owner::Rex => {
                let seen = rex::Seen {
                    status: seen,
                    seq: (seen == "blocked").then_some(seq),
                };
                (self.followed_rex()?)
                    .paste(pane_id, text, true, Some(&seen))
                    .await
            }
        }
    }

    pub async fn agent_send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::agent_send_keys(&self.herdr, pane_id, keys).await,
            Owner::Core => self.followed()?.keys(pane_id, keys).await,
            Owner::Rex => self.followed_rex()?.keys(pane_id, keys).await,
        }
    }

    pub async fn pane_send_text(&self, pane_id: &str, text: &str) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::pane_send_text(&self.herdr, pane_id, text).await,
            Owner::Core => self.followed()?.type_raw(pane_id, text).await,
            Owner::Rex => {
                (self.followed_rex()?)
                    .paste(pane_id, text, false, None)
                    .await
            }
        }
    }

    /// Text pasted without Enter. Rex brackets a paste itself, for a program that turned
    /// bracketed paste on.
    pub async fn pane_paste(&self, pane_id: &str, text: &str) -> Result<(), Error> {
        let paste = match Owner::of(pane_id) {
            Owner::Herdr | Owner::Core => format!("\u{1b}[200~{text}\u{1b}[201~"),
            Owner::Rex => text.to_owned(),
        };
        self.pane_send_text(pane_id, &paste).await
    }

    pub async fn scroll_bottom(&self, pane_id: &str) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::pane_send_text(&self.herdr, pane_id, SCROLL_BOTTOM).await,
            Owner::Core => self.followed()?.keys(pane_id, &["ctrl+end"]).await,
            Owner::Rex => self.followed_rex()?.keys(pane_id, &["ctrl+end"]).await,
        }
    }

    /// A notice option's digit, without Enter.
    pub async fn notice_digit(&self, pane_id: &str, digit: &str) -> Result<(), Error> {
        match Owner::of(pane_id) {
            Owner::Herdr => herdr::pane_send_text(&self.herdr, pane_id, digit).await,
            Owner::Core => self.followed()?.keys(pane_id, &[digit]).await,
            Owner::Rex => self.followed_rex()?.keys(pane_id, &[digit]).await,
        }
    }

    /// The terminal calls below reach a Rex terminal, never a Rex agent; Herdr Core's shells
    /// are not listed.
    pub async fn pane_send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<(), Error> {
        if Owner::of(pane_id) == Owner::Rex {
            return self.followed_rex()?.terminal(pane_id, "", keys).await;
        }
        herdr_only(pane_id)?;
        herdr::pane_send_keys(&self.herdr, pane_id, keys).await
    }

    pub async fn pane_send_input(
        &self,
        pane_id: &str,
        text: &str,
        keys: &[&str],
    ) -> Result<(), Error> {
        if Owner::of(pane_id) == Owner::Rex {
            return self.followed_rex()?.terminal(pane_id, text, keys).await;
        }
        herdr_only(pane_id)?;
        herdr::pane_send_input(&self.herdr, pane_id, text, keys).await
    }

    /// A Rex terminal takes the text as a paste.
    pub async fn terminal_send_text(&self, pane_id: &str, text: &str) -> Result<(), Error> {
        if Owner::of(pane_id) == Owner::Rex {
            return self.followed_rex()?.terminal(pane_id, text, &[]).await;
        }
        herdr_only(pane_id)?;
        herdr::pane_send_text(&self.herdr, pane_id, text).await
    }

    pub async fn agent_focus(&self, pane_id: &str) -> Result<(), Error> {
        if Owner::of(pane_id) == Owner::Rex {
            return Err(Error::Unsupported(rex::FOCUS.to_owned()));
        }
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
        code_mode: Option<bool>,
    ) -> Result<AgentInfo, Error> {
        herdr::agent_start(
            &self.herdr,
            name,
            kind,
            pane_id,
            resume,
            ultracode,
            code_mode,
        )
        .await
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
                "a Herdr Core group cannot be closed; close its agents one at a time".to_owned(),
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

    /// Archive's close: `pane_close`, and for a Rex task the session collied created for it
    /// too, once empty; what became of that session, for the archive's answer.
    pub async fn task_close(&self, pane_id: &str) -> Result<Option<String>, Error> {
        match Owner::of(pane_id) {
            Owner::Rex => self.followed_rex()?.close_task(pane_id).await,
            _ => self.pane_close(pane_id).await.map(|()| None),
        }
    }
}
