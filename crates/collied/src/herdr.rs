use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LINE: u64 = 1 << 20;
const MAX_SESSION_NAME_LEN: usize = 64;
const DEFAULT_SESSION_NAME: &str = "default";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("timed out")]
    Timeout,
    #[error("response line exceeds 1 MiB")]
    LineTooLong,
    #[error("connection closed without a response")]
    Closed,
    #[error("invalid response: {0}")]
    Json(#[from] serde_json::Error),
    #[error("response id {got:?} does not match request id {want:?}")]
    IdMismatch { want: String, got: String },
    #[error("herdr error {code}: {message}")]
    Herdr { code: String, message: String },
    #[error("response has neither result nor error")]
    Empty,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid herdr session name {name:?}: {reason}")]
pub struct InvalidSession {
    name: String,
    reason: &'static str,
}

#[derive(Debug, Default)]
pub struct SocketEnv {
    pub socket_path: Option<String>,
    pub session: Option<String>,
    pub xdg_config_home: Option<String>,
    pub home: Option<String>,
}

impl SocketEnv {
    pub fn from_process() -> Self {
        let var = |k| std::env::var(k).ok();
        Self {
            socket_path: var("HERDR_SOCKET_PATH"),
            session: var("HERDR_SESSION"),
            xdg_config_home: var("XDG_CONFIG_HOME"),
            home: var("HOME"),
        }
    }
}

// Mirrors herdr 0.9.3 session::configure_from_args + active_api_socket_path and
// config::io::config_dir (release build, app dir "herdr").
pub fn resolve_socket_path(
    session: Option<&str>,
    env: &SocketEnv,
) -> Result<PathBuf, InvalidSession> {
    let name = match (session, &env.socket_path, &env.session) {
        (Some(s), _, _) => normalize_name(s)?,
        (None, Some(path), _) => return Ok(PathBuf::from(path)),
        (None, None, Some(s)) => normalize_name(s)?,
        (None, None, None) => None,
    };
    let config_dir = match (&env.xdg_config_home, &env.home) {
        (Some(xdg), _) => PathBuf::from(xdg).join("herdr"),
        (None, Some(home)) => PathBuf::from(home).join(".config/herdr"),
        (None, None) => std::env::temp_dir().join("herdr"),
    };
    let dir = match name {
        Some(name) => config_dir.join("sessions").join(name),
        None => config_dir,
    };
    Ok(dir.join("herdr.sock"))
}

pub fn session_label(session: Option<&str>, env: &SocketEnv) -> String {
    session
        .or(env.session.as_deref())
        .and_then(|s| normalize_name(s).ok().flatten())
        .unwrap_or(DEFAULT_SESSION_NAME)
        .to_owned()
}

fn normalize_name(name: &str) -> Result<Option<&str>, InvalidSession> {
    if name == DEFAULT_SESSION_NAME {
        return Ok(None);
    }
    let reason = if name.is_empty() {
        "cannot be empty"
    } else if name.len() > MAX_SESSION_NAME_LEN {
        "cannot be longer than 64 bytes"
    } else if name == "." || name == ".." {
        "cannot be . or .."
    } else if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        "may only contain ASCII letters, numbers, '.', '_' and '-'"
    } else {
        return Ok(Some(name));
    };
    Err(InvalidSession {
        name: name.to_string(),
        reason,
    })
}

#[derive(Debug, Deserialize)]
pub struct Pong {
    pub version: String,
    pub protocol: u32,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PingResult {
    Pong(Pong),
}

#[derive(Deserialize)]
struct Response<R> {
    id: String,
    result: Option<R>,
    error: Option<ErrorBody>,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    pub number: u32,
    pub label: String,
    pub active_tab_id: Option<String>,
    pub agent_status: String,
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

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SnapshotResult {
    SessionSnapshot { snapshot: SessionSnapshot },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentListResult {
    AgentList { agents: Vec<AgentInfo> },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WorkspaceListResult {
    WorkspaceList { workspaces: Vec<WorkspaceInfo> },
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaneRead {
    pub text: String,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PaneReadResult {
    PaneRead { read: PaneRead },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentInfoResult {
    AgentInfo { agent: AgentInfo },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentPromptedResult {
    AgentPrompted {},
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentStartedResult {
    AgentStarted { agent: AgentInfo },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OkResult {
    Ok {},
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceCreated {
    pub workspace: WorkspaceInfo,
    pub root_pane: PaneInfo,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WorkspaceCreatedResult {
    WorkspaceCreated(WorkspaceCreated),
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: Option<String>,
    pub open_workspace_id: Option<String>,
    pub is_bare: bool,
    pub is_linked_worktree: bool,
}

#[derive(Debug, Deserialize)]
pub struct WorktreeList {
    pub source: WorktreeSource,
    pub worktrees: Vec<WorktreeInfo>,
}

#[derive(Debug, Deserialize)]
pub struct WorktreeSource {
    pub source_checkout_path: String,
}

#[derive(Debug, Deserialize)]
pub struct WorktreeWorkspace {
    pub workspace: WorkspaceInfo,
    pub root_pane: PaneInfo,
    pub worktree: WorktreeInfo,
    #[serde(default)]
    pub already_open: bool,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WorktreeListResult {
    WorktreeList(WorktreeList),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WorktreeCreateResult {
    WorktreeCreated(WorktreeWorkspace),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WorktreeOpenResult {
    WorktreeOpened(WorktreeWorkspace),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PaneInfoResult {
    PaneInfo { pane: PaneInfo },
}

#[derive(Deserialize)]
struct LayoutRect {
    width: u16,
}

#[derive(Deserialize)]
struct LayoutPane {
    pane_id: String,
    rect: LayoutRect,
}

#[derive(Deserialize)]
struct PaneLayout {
    panes: Vec<LayoutPane>,
    zoomed: bool,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PaneLayoutResult {
    PaneLayout { layout: PaneLayout },
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MatchedRule {
    pub id: String,
}

/// Only these fields are kept: the rest of `agent.explain` carries raw pane text.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Explain {
    pub matched_rule: Option<MatchedRule>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ExplainResult {
    AgentExplain { explain: Explain },
}

#[derive(Deserialize)]
struct AgentManifest {
    agent: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentManifestsResult {
    AgentManifestStatus { manifests: Vec<AgentManifest> },
}

pub async fn ping(socket: &Path) -> Result<Pong, Error> {
    let PingResult::Pong(pong) = call(socket, "ping", json!({})).await?;
    Ok(pong)
}

pub async fn session_snapshot(socket: &Path) -> Result<SessionSnapshot, Error> {
    let SnapshotResult::SessionSnapshot { snapshot } =
        call(socket, "session.snapshot", json!({})).await?;
    Ok(snapshot)
}

pub async fn agent_list(socket: &Path) -> Result<Vec<AgentInfo>, Error> {
    let AgentListResult::AgentList { agents } = call(socket, "agent.list", json!({})).await?;
    Ok(agents)
}

pub async fn workspace_list(socket: &Path) -> Result<Vec<WorkspaceInfo>, Error> {
    let WorkspaceListResult::WorkspaceList { workspaces } =
        call(socket, "workspace.list", json!({})).await?;
    Ok(workspaces)
}

pub async fn agent_manifests(socket: &Path) -> Result<Vec<String>, Error> {
    let AgentManifestsResult::AgentManifestStatus { manifests } =
        call(socket, "server.agent_manifests", json!({})).await?;
    Ok(manifests.into_iter().map(|m| m.agent).collect())
}

/// herdr agent targets are a pane id or an agent name, never a terminal id.
pub async fn agent_get(socket: &Path, target: &str) -> Result<AgentInfo, Error> {
    let AgentInfoResult::AgentInfo { agent } =
        call(socket, "agent.get", json!({ "target": target })).await?;
    Ok(agent)
}

pub async fn pane_get(socket: &Path, pane_id: &str) -> Result<PaneInfo, Error> {
    let PaneInfoResult::PaneInfo { pane } =
        call(socket, "pane.get", json!({ "pane_id": pane_id })).await?;
    Ok(pane)
}

/// The pane's width in columns. herdr answers with the focused tab's layout when it does not
/// know `pane_id`, so the pane is matched by id. `None` in a zoomed tab, where `rect` is not
/// known to be the width the pane is drawn at.
pub async fn pane_columns(socket: &Path, pane_id: &str) -> Result<Option<u16>, Error> {
    let PaneLayoutResult::PaneLayout { layout } =
        call(socket, "pane.layout", json!({ "pane_id": pane_id })).await?;
    if layout.zoomed {
        return Ok(None);
    }
    Ok(layout
        .panes
        .into_iter()
        .find(|p| p.pane_id == pane_id)
        .map(|p| p.rect.width))
}

pub async fn agent_read(
    socket: &Path,
    pane_id: &str,
    source: &str,
    lines: Option<u32>,
) -> Result<PaneRead, Error> {
    let PaneReadResult::PaneRead { read } = call(
        socket,
        "agent.read",
        json!({ "target": pane_id, "source": source, "lines": lines, "format": "ansi" }),
    )
    .await?;
    Ok(read)
}

pub async fn pane_read(
    socket: &Path,
    pane_id: &str,
    source: &str,
    lines: Option<u32>,
) -> Result<PaneRead, Error> {
    let PaneReadResult::PaneRead { read } = call(
        socket,
        "pane.read",
        json!({ "pane_id": pane_id, "source": source, "lines": lines, "format": "ansi" }),
    )
    .await?;
    Ok(read)
}

pub async fn agent_explain(socket: &Path, target: &str) -> Result<Explain, Error> {
    let ExplainResult::AgentExplain { explain } =
        call(socket, "agent.explain", json!({ "target": target })).await?;
    Ok(explain)
}

pub async fn detection_text(socket: &Path, pane_id: &str) -> Result<String, Error> {
    let PaneReadResult::PaneRead { read } = call(
        socket,
        "pane.read",
        json!({ "pane_id": pane_id, "source": "detection", "format": "text" }),
    )
    .await?;
    Ok(read.text)
}

pub async fn agent_prompt(socket: &Path, pane_id: &str, text: &str) -> Result<(), Error> {
    let AgentPromptedResult::AgentPrompted {} = call(
        socket,
        "agent.prompt",
        json!({ "target": pane_id, "text": text }),
    )
    .await?;
    Ok(())
}

pub async fn agent_send_keys(socket: &Path, pane_id: &str, keys: &[&str]) -> Result<(), Error> {
    let OkResult::Ok {} = call(
        socket,
        "agent.send_keys",
        json!({ "target": pane_id, "keys": keys }),
    )
    .await?;
    Ok(())
}

/// Typed as is, without Enter and without bracketed paste (verified on herdr 0.9.3).
pub async fn pane_send_text(socket: &Path, pane_id: &str, text: &str) -> Result<(), Error> {
    let OkResult::Ok {} = call(
        socket,
        "pane.send_text",
        json!({ "pane_id": pane_id, "text": text }),
    )
    .await?;
    Ok(())
}

pub async fn pane_send_keys(socket: &Path, pane_id: &str, keys: &[&str]) -> Result<(), Error> {
    let OkResult::Ok {} = call(
        socket,
        "pane.send_keys",
        json!({ "pane_id": pane_id, "keys": keys }),
    )
    .await?;
    Ok(())
}

/// The text, then the keys, in one ordered write (what `herdr pane run` sends).
pub async fn pane_send_input(
    socket: &Path,
    pane_id: &str,
    text: &str,
    keys: &[&str],
) -> Result<(), Error> {
    let OkResult::Ok {} = call(
        socket,
        "pane.send_input",
        json!({ "pane_id": pane_id, "text": text, "keys": keys }),
    )
    .await?;
    Ok(())
}

pub async fn agent_focus(socket: &Path, pane_id: &str) -> Result<(), Error> {
    let AgentInfoResult::AgentInfo { .. } =
        call(socket, "agent.focus", json!({ "target": pane_id })).await?;
    Ok(())
}

pub async fn agent_start(
    socket: &Path,
    name: &str,
    kind: &str,
    pane_id: &str,
) -> Result<AgentInfo, Error> {
    let AgentStartedResult::AgentStarted { agent } = call(
        socket,
        "agent.start",
        json!({ "name": name, "kind": kind, "pane_id": pane_id }),
    )
    .await?;
    Ok(agent)
}

pub async fn workspace_create(
    socket: &Path,
    cwd: &str,
    label: Option<&str>,
) -> Result<WorkspaceCreated, Error> {
    let WorkspaceCreatedResult::WorkspaceCreated(created) = call(
        socket,
        "workspace.create",
        json!({ "cwd": cwd, "label": label, "focus": false }),
    )
    .await?;
    Ok(created)
}

pub async fn worktree_list(socket: &Path, cwd: &str) -> Result<WorktreeList, Error> {
    let WorktreeListResult::WorktreeList(list) =
        call(socket, "worktree.list", json!({ "cwd": cwd })).await?;
    Ok(list)
}

pub async fn worktree_create(
    socket: &Path,
    cwd: &str,
    branch: &str,
    path: &str,
    label: Option<&str>,
) -> Result<WorktreeWorkspace, Error> {
    let WorktreeCreateResult::WorktreeCreated(created) = call(
        socket,
        "worktree.create",
        json!({ "cwd": cwd, "branch": branch, "path": path, "label": label, "focus": false }),
    )
    .await?;
    Ok(created)
}

pub async fn worktree_open(
    socket: &Path,
    cwd: &str,
    path: &str,
    label: Option<&str>,
) -> Result<WorktreeWorkspace, Error> {
    let WorktreeOpenResult::WorktreeOpened(opened) = call(
        socket,
        "worktree.open",
        json!({ "cwd": cwd, "path": path, "label": label, "focus": false }),
    )
    .await?;
    Ok(opened)
}

pub async fn workspace_close(socket: &Path, workspace_id: &str) -> Result<(), Error> {
    let OkResult::Ok {} = call(
        socket,
        "workspace.close",
        json!({ "workspace_id": workspace_id }),
    )
    .await?;
    Ok(())
}

pub async fn pane_close(socket: &Path, pane_id: &str) -> Result<(), Error> {
    let OkResult::Ok {} = call(socket, "pane.close", json!({ "pane_id": pane_id })).await?;
    Ok(())
}

fn without_nulls(params: Value) -> Value {
    match params {
        Value::Object(map) => {
            Value::Object(map.into_iter().filter(|(_, v)| !v.is_null()).collect())
        }
        other => other,
    }
}

async fn call<R: DeserializeOwned>(socket: &Path, method: &str, params: Value) -> Result<R, Error> {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let id = format!("collied-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
    let mut line = serde_json::to_vec(&json!({
        "id": id,
        "method": method,
        "params": without_nulls(params),
    }))?;
    line.push(b'\n');
    tracing::debug!(socket = %socket.display(), method, id, "herdr request");

    let raw = tokio::time::timeout(TIMEOUT, async {
        let mut stream = UnixStream::connect(socket).await?;
        stream.write_all(&line).await?;
        let mut buf = Vec::new();
        BufReader::new(stream)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut buf)
            .await?;
        Ok::<_, Error>(buf)
    })
    .await
    .map_err(|_| Error::Timeout)??;

    if raw.last() != Some(&b'\n') {
        return Err(if raw.len() as u64 > MAX_LINE {
            Error::LineTooLong
        } else {
            Error::Closed
        });
    }
    let resp: Response<R> = serde_json::from_slice(&raw)?;
    if let Some(e) = resp.error {
        return Err(Error::Herdr {
            code: e.code,
            message: e.message,
        });
    }
    if resp.id != id {
        return Err(Error::IdMismatch {
            want: id,
            got: resp.id,
        });
    }
    resp.result.ok_or(Error::Empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(socket: Option<&str>, session: Option<&str>, xdg: Option<&str>) -> SocketEnv {
        SocketEnv {
            socket_path: socket.map(Into::into),
            session: session.map(Into::into),
            xdg_config_home: xdg.map(Into::into),
            home: Some("/Users/u".into()),
        }
    }

    fn resolve(session: Option<&str>, env: &SocketEnv) -> PathBuf {
        resolve_socket_path(session, env).unwrap()
    }

    #[test]
    fn default_path() {
        let e = env(None, None, None);
        assert_eq!(
            resolve(None, &e),
            PathBuf::from("/Users/u/.config/herdr/herdr.sock")
        );
    }

    #[test]
    fn xdg_config_home() {
        let e = env(None, None, Some("/x"));
        assert_eq!(resolve(None, &e), PathBuf::from("/x/herdr/herdr.sock"));
        assert_eq!(
            resolve(Some("w"), &e),
            PathBuf::from("/x/herdr/sessions/w/herdr.sock")
        );
    }

    #[test]
    fn precedence() {
        let e = env(Some("/s.sock"), Some("envsess"), None);
        assert_eq!(
            resolve(Some("flag"), &e),
            PathBuf::from("/Users/u/.config/herdr/sessions/flag/herdr.sock")
        );
        assert_eq!(resolve(None, &e), PathBuf::from("/s.sock"));
        let e = env(None, Some("envsess"), None);
        assert_eq!(
            resolve(None, &e),
            PathBuf::from("/Users/u/.config/herdr/sessions/envsess/herdr.sock")
        );
    }

    #[test]
    fn session_label_follows_socket_precedence() {
        let e = env(Some("/s.sock"), Some("envsess"), None);
        assert_eq!(session_label(Some("flag"), &e), "flag");
        assert_eq!(session_label(None, &e), "envsess");
        assert_eq!(session_label(None, &env(None, None, None)), "default");
        assert_eq!(
            session_label(None, &env(None, Some("default"), None)),
            "default"
        );
        assert_eq!(
            session_label(None, &env(Some("/s.sock"), Some("bad/name"), None)),
            "default"
        );
    }

    #[test]
    fn socket_env_skips_session_env_validation() {
        let e = env(Some("/s.sock"), Some("bad/name"), None);
        assert_eq!(resolve(None, &e), PathBuf::from("/s.sock"));
    }

    #[test]
    fn default_name_is_unnamed_session() {
        let e = env(Some("/s.sock"), Some("default"), None);
        assert_eq!(
            resolve(Some("default"), &e),
            PathBuf::from("/Users/u/.config/herdr/herdr.sock")
        );
        let e = env(None, Some("default"), None);
        assert_eq!(
            resolve(None, &e),
            PathBuf::from("/Users/u/.config/herdr/herdr.sock")
        );
    }

    #[test]
    fn name_validation() {
        let e = env(None, None, None);
        for bad in ["", ".", "..", "a/b", "a b", "é", &"a".repeat(65)] {
            assert!(resolve_socket_path(Some(bad), &e).is_err(), "{bad:?}");
        }
        assert!(resolve_socket_path(None, &env(None, Some("../x"), None)).is_err());
        for good in ["a", "A.b_c-9", "...", &"a".repeat(64)] {
            assert!(resolve_socket_path(Some(good), &e).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn decodes_live_samples() {
        let snap: Response<SnapshotResult> =
            serde_json::from_str(include_str!("../tests/fixtures/session.snapshot.json")).unwrap();
        let SnapshotResult::SessionSnapshot { snapshot } = snap.result.unwrap();
        assert_eq!(snapshot.agents.len(), 2);
        assert_eq!(snapshot.agents[0].state_change_seq, 5);
        let list: Response<AgentListResult> = serde_json::from_str(
            r#"{"id":"x","result":{"type":"agent_list","agents":[{"terminal_id":"t","workspace_id":"w1","agent_status":"idle","pane_id":"w1:p1","future":true}]}}"#,
        )
        .unwrap();
        let AgentListResult::AgentList { agents } = list.result.unwrap();
        assert_eq!(agents[0].state_change_seq, 0);
    }
}
