use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;

pub mod herdr;
pub mod herdr_core;
pub mod screen;

use self::herdr_core::Core;

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
    /// False when a followed core answered in herdr's place: herdr's agents are then not
    /// known to be gone, and what collied keeps for them must stay.
    pub herdr: bool,
}

pub struct PaneProcesses {
    pub pids: Vec<u32>,
    pub attach: Option<String>,
}

/// herdr, and a herdr-core core beside it when one is followed. Calls are routed by id:
/// `hc:` ids are herdr-core's and never reach herdr.
#[derive(Clone)]
pub struct Runtime {
    herdr: PathBuf,
    core: Option<Arc<Core>>,
}

pub fn herdr_socket(session: Option<&str>) -> Result<PathBuf, herdr::InvalidSession> {
    herdr::resolve_socket_path(session, &herdr::SocketEnv::from_process())
}

pub fn herdr_session_label(session: Option<&str>) -> String {
    herdr::session_label(session, &herdr::SocketEnv::from_process())
}

pub const NOT_YET: &str = "not available for Herdr 2 agents yet";

fn herdr_only(id: &str) -> Result<(), Error> {
    if herdr_core::owns(id) {
        return Err(Error::Unsupported(NOT_YET.to_owned()));
    }
    Ok(())
}

impl Runtime {
    pub fn new(herdr: PathBuf) -> Self {
        Self { herdr, core: None }
    }

    pub fn with_core(mut self, core: Option<Arc<Core>>) -> Self {
        self.core = core;
        self
    }

    pub fn core(&self) -> Option<&Arc<Core>> {
        self.core.as_ref()
    }

    fn followed(&self) -> Result<&Arc<Core>, Error> {
        self.core
            .as_ref()
            .ok_or_else(|| Error::Unsupported(NOT_YET.to_owned()))
    }

    /// herdr's answer, plus the core's agents while it is followed. Without a followed core
    /// this is herdr's answer unchanged, its error included.
    fn merge<T: Default>(
        &self,
        herdr: Result<T, Error>,
        add: impl FnOnce(&mut T, Vec<AgentInfo>),
    ) -> Result<T, Error> {
        let Some((core, agents)) = self.core.as_ref().and_then(|c| Some((c, c.agents()?))) else {
            return herdr;
        };
        core.herdr_seen(herdr.as_ref().err());
        let mut merged = herdr.unwrap_or_default();
        add(&mut merged, agents);
        Ok(merged)
    }

    pub async fn ping(&self) -> Result<herdr::Pong, Error> {
        herdr::ping(&self.herdr).await
    }

    pub async fn session_snapshot(&self) -> Result<SessionSnapshot, Error> {
        let herdr = herdr::session_snapshot(&self.herdr).await;
        self.merge(herdr, |snap, agents| {
            snap.workspaces
                .extend(herdr_core::group(&agents, &snap.workspaces));
            snap.panes.extend(agents.iter().map(herdr_core::pane));
            snap.agents.extend(agents);
        })
    }

    pub async fn agent_list(&self) -> Result<Vec<AgentInfo>, Error> {
        let herdr = herdr::agent_list(&self.herdr).await;
        self.merge(herdr, |list, agents| list.extend(agents))
    }

    pub async fn poll(&self) -> Result<Poll, Error> {
        let herdr = tokio::try_join!(
            herdr::agent_list(&self.herdr),
            herdr::workspace_list(&self.herdr)
        );
        let answered = herdr.is_ok();
        let (agents, workspaces) = self.merge(herdr, |(list, workspaces), agents| {
            workspaces.extend(herdr_core::group(&agents, workspaces));
            list.extend(agents);
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
        if herdr_core::owns(id) {
            self.agent_list().await
        } else {
            herdr::agent_list(&self.herdr).await
        }
    }

    pub async fn snapshot_of(&self, id: &str) -> Result<SessionSnapshot, Error> {
        if herdr_core::owns(id) {
            self.session_snapshot().await
        } else {
            herdr::session_snapshot(&self.herdr).await
        }
    }

    pub async fn agent_manifests(&self) -> Result<Vec<String>, Error> {
        herdr::agent_manifests(&self.herdr).await
    }

    pub async fn agent_get(&self, target: &str) -> Result<AgentInfo, Error> {
        herdr_only(target)?;
        herdr::agent_get(&self.herdr, target).await
    }

    pub async fn pane_get(&self, pane_id: &str) -> Result<PaneInfo, Error> {
        herdr_only(pane_id)?;
        herdr::pane_get(&self.herdr, pane_id).await
    }

    /// A herdr-core terminal's own width; herdr's is its layout rect's.
    pub async fn pane_columns(&self, pane_id: &str) -> Result<Option<u16>, Error> {
        if herdr_core::owns(pane_id) {
            return self.followed()?.columns(pane_id);
        }
        herdr::pane_columns(&self.herdr, pane_id).await
    }

    pub async fn agent_read(
        &self,
        pane_id: &str,
        source: &str,
        lines: Option<u32>,
    ) -> Result<PaneRead, Error> {
        if herdr_core::owns(pane_id) {
            return self.followed()?.read(pane_id, source, lines).await;
        }
        herdr::agent_read(&self.herdr, pane_id, source, lines).await
    }

    pub async fn pane_read(
        &self,
        pane_id: &str,
        source: &str,
        lines: Option<u32>,
    ) -> Result<PaneRead, Error> {
        if herdr_core::owns(pane_id) {
            return self.followed()?.read(pane_id, source, lines).await;
        }
        herdr::pane_read(&self.herdr, pane_id, source, lines).await
    }

    /// Keeps a herdr-core terminal attached until dropped, so a watch's reads come from one
    /// emulated screen; herdr needs nothing kept.
    pub fn watch(&self, pane_id: &str) -> Result<Option<herdr_core::Watch>, Error> {
        if !herdr_core::owns(pane_id) {
            return Ok(None);
        }
        self.followed()?.watch(pane_id).map(Some)
    }

    pub async fn agent_explain(&self, target: &str) -> Result<herdr::Explain, Error> {
        herdr_only(target)?;
        herdr::agent_explain(&self.herdr, target).await
    }

    pub async fn detection_text(&self, pane_id: &str) -> Result<String, Error> {
        herdr_only(pane_id)?;
        herdr::detection_text(&self.herdr, pane_id).await
    }

    pub async fn agent_prompt(&self, pane_id: &str, text: &str) -> Result<(), Error> {
        herdr_only(pane_id)?;
        herdr::agent_prompt(&self.herdr, pane_id, text).await
    }

    pub async fn agent_send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<(), Error> {
        herdr_only(pane_id)?;
        herdr::agent_send_keys(&self.herdr, pane_id, keys).await
    }

    pub async fn pane_send_text(&self, pane_id: &str, text: &str) -> Result<(), Error> {
        herdr_only(pane_id)?;
        herdr::pane_send_text(&self.herdr, pane_id, text).await
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
        if herdr_core::owns(workspace_id) {
            return Err(Error::Unsupported(
                "a Herdr 2 group cannot be closed; close its agents one at a time".to_owned(),
            ));
        }
        herdr::workspace_close(&self.herdr, workspace_id).await
    }

    pub async fn pane_processes(&self, pane_id: &str) -> Result<PaneProcesses, Error> {
        herdr_only(pane_id)?;
        herdr::pane_processes(&self.herdr, pane_id).await
    }

    pub async fn pane_close(&self, pane_id: &str) -> Result<(), Error> {
        if herdr_core::owns(pane_id) {
            return self.followed()?.close(pane_id).await;
        }
        herdr::pane_close(&self.herdr, pane_id).await
    }
}
