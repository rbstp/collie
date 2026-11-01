use std::path::PathBuf;

use serde::Deserialize;

pub mod herdr;

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

#[derive(Debug, Clone, Deserialize)]
pub struct SessionSnapshot {
    pub workspaces: Vec<WorkspaceInfo>,
    pub panes: Vec<PaneInfo>,
    pub agents: Vec<AgentInfo>,
}

pub struct PaneProcesses {
    pub pids: Vec<u32>,
    pub attach: Option<String>,
}

#[derive(Clone)]
pub struct Runtime {
    herdr: PathBuf,
}

pub fn herdr_socket(session: Option<&str>) -> Result<PathBuf, herdr::InvalidSession> {
    herdr::resolve_socket_path(session, &herdr::SocketEnv::from_process())
}

pub fn herdr_session_label(session: Option<&str>) -> String {
    herdr::session_label(session, &herdr::SocketEnv::from_process())
}

impl Runtime {
    pub fn new(herdr: PathBuf) -> Self {
        Self { herdr }
    }

    pub async fn ping(&self) -> Result<herdr::Pong, Error> {
        herdr::ping(&self.herdr).await
    }

    pub async fn session_snapshot(&self) -> Result<SessionSnapshot, Error> {
        herdr::session_snapshot(&self.herdr).await
    }

    pub async fn agent_list(&self) -> Result<Vec<AgentInfo>, Error> {
        herdr::agent_list(&self.herdr).await
    }

    pub async fn workspace_list(&self) -> Result<Vec<WorkspaceInfo>, Error> {
        herdr::workspace_list(&self.herdr).await
    }

    pub async fn agent_manifests(&self) -> Result<Vec<String>, Error> {
        herdr::agent_manifests(&self.herdr).await
    }

    pub async fn agent_get(&self, target: &str) -> Result<AgentInfo, Error> {
        herdr::agent_get(&self.herdr, target).await
    }

    pub async fn pane_get(&self, pane_id: &str) -> Result<PaneInfo, Error> {
        herdr::pane_get(&self.herdr, pane_id).await
    }

    pub async fn pane_columns(&self, pane_id: &str) -> Result<Option<u16>, Error> {
        herdr::pane_columns(&self.herdr, pane_id).await
    }

    pub async fn agent_read(
        &self,
        pane_id: &str,
        source: &str,
        lines: Option<u32>,
    ) -> Result<PaneRead, Error> {
        herdr::agent_read(&self.herdr, pane_id, source, lines).await
    }

    pub async fn pane_read(
        &self,
        pane_id: &str,
        source: &str,
        lines: Option<u32>,
    ) -> Result<PaneRead, Error> {
        herdr::pane_read(&self.herdr, pane_id, source, lines).await
    }

    pub async fn agent_explain(&self, target: &str) -> Result<herdr::Explain, Error> {
        herdr::agent_explain(&self.herdr, target).await
    }

    pub async fn detection_text(&self, pane_id: &str) -> Result<String, Error> {
        herdr::detection_text(&self.herdr, pane_id).await
    }

    pub async fn agent_prompt(&self, pane_id: &str, text: &str) -> Result<(), Error> {
        herdr::agent_prompt(&self.herdr, pane_id, text).await
    }

    pub async fn agent_send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<(), Error> {
        herdr::agent_send_keys(&self.herdr, pane_id, keys).await
    }

    pub async fn pane_send_text(&self, pane_id: &str, text: &str) -> Result<(), Error> {
        herdr::pane_send_text(&self.herdr, pane_id, text).await
    }

    pub async fn pane_send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<(), Error> {
        herdr::pane_send_keys(&self.herdr, pane_id, keys).await
    }

    pub async fn pane_send_input(
        &self,
        pane_id: &str,
        text: &str,
        keys: &[&str],
    ) -> Result<(), Error> {
        herdr::pane_send_input(&self.herdr, pane_id, text, keys).await
    }

    pub async fn agent_focus(&self, pane_id: &str) -> Result<(), Error> {
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
        herdr::worktree_remove(&self.herdr, workspace_id).await
    }

    pub async fn workspace_close(&self, workspace_id: &str) -> Result<(), Error> {
        herdr::workspace_close(&self.herdr, workspace_id).await
    }

    pub async fn pane_processes(&self, pane_id: &str) -> Result<PaneProcesses, Error> {
        herdr::pane_processes(&self.herdr, pane_id).await
    }

    pub async fn pane_close(&self, pane_id: &str) -> Result<(), Error> {
        herdr::pane_close(&self.herdr, pane_id).await
    }
}
