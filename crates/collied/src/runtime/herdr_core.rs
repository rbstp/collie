use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::watch;

use super::{AgentInfo, Error, PaneInfo, WorkspaceInfo};

pub const GENERATION: u32 = 1;
pub const MIN_VERSION: &str = "0.1.16";
/// Every method collied calls. The API also runs programs and writes files on other machines,
/// stores documents and stops the core; none of that is ever called.
pub const METHODS: [&str; 5] = [
    "hello",
    "server.status",
    "events.subscribe",
    "terminal.kill",
    "terminal.rm",
];
pub const GROUP_LABEL: &str = "Herdr 2";
const PREFIX: &str = "hc:";
const AGENT_CLIS: [&str; 3] = ["claude", "codex", "copilot"];
const TIMEOUT: Duration = Duration::from_secs(5);
const KILL_TIMEOUT: Duration = Duration::from_secs(15);
/// The core sends a heartbeat every 15 s.
const SILENCE: Duration = Duration::from_secs(45);
const MAX_LINE: u64 = 16 << 20;
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

pub fn owns(id: &str) -> bool {
    id.starts_with(PREFIX)
}

/// Where `hc` would find the core, from `var` (collied's own environment): `HC_SOCKET`, else
/// `socket` in herdr-core's config, else the runtime folder, else beside the session's state.
/// Mirrors herdr-core-client's `Endpoint::resolve` (penso/herdr-gpui 7bc7090).
pub fn socket_path(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
    let run_user = PathBuf::from(format!("/run/user/{}", rustix::process::getuid().as_raw()));
    resolve(&var, run_user.is_dir().then_some(run_user), &|p| {
        std::fs::read_to_string(p).ok()
    })
}

fn resolve(
    var: &dyn Fn(&str) -> Option<OsString>,
    run_user: Option<PathBuf>,
    read: &dyn Fn(&Path) -> Option<String>,
) -> Result<PathBuf, String> {
    let set = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let raw = var("HC_SESSION").map(|s| s.to_string_lossy().into_owned());
    let session = raw
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default");
    if session == "."
        || session == ".."
        || session
            .chars()
            .any(|c| c.is_control() || r#"/\:*?"<>|"#.contains(c))
    {
        return Err(format!("invalid HC_SESSION {session:?}"));
    }
    let home = set("HOME").ok_or("HOME is not set")?;
    let state = set("XDG_STATE_HOME")
        .unwrap_or_else(|| home.join(".local/state"))
        .join("herdr-core")
        .join(session);
    let config = set("XDG_CONFIG_HOME")
        .unwrap_or_else(|| home.join(".config"))
        .join("herdr-core/config.toml");
    #[derive(Deserialize)]
    struct Config {
        socket: Option<String>,
    }
    let configured = || {
        toml::from_str::<Config>(&read(&config)?)
            .ok()?
            .socket
            .map(PathBuf::from)
    };
    let name = format!("{session}.sock");
    Ok(set("HC_SOCKET").or_else(configured).unwrap_or_else(|| {
        match set("XDG_RUNTIME_DIR").or(run_user) {
            Some(run) => run.join("herdr-core").join(name),
            None => state.join(name),
        }
    }))
}

#[derive(Debug, Clone, Deserialize)]
pub struct Hello {
    pub core: CoreInfo,
    pub generation: u32,
    pub version: String,
    #[serde(default)]
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CoreInfo {
    pub id: String,
}

impl Hello {
    pub fn missing(&self) -> Vec<&'static str> {
        METHODS
            .into_iter()
            .filter(|m| !self.methods.iter().any(|have| have == m))
            .collect()
    }

    pub fn problem(&self) -> Option<String> {
        let missing = self.missing();
        if self.generation != GENERATION {
            Some(format!(
                "speaks protocol generation {}, collied {GENERATION}",
                self.generation
            ))
        } else if version(&self.version) < version(MIN_VERSION) {
            Some(format!("older than {MIN_VERSION}"))
        } else if !missing.is_empty() {
            Some(format!("lacks {}", missing.join(", ")))
        } else if self.core.id.is_empty()
            || self.core.id.len() > 32
            || !self.core.id.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            Some("has an unexpected core id".to_owned())
        } else {
            None
        }
    }
}

fn version(text: &str) -> [u64; 3] {
    let mut parts = text
        .trim()
        .split(['.', '-', '+'])
        .map(|p| p.parse::<u64>().unwrap_or(0));
    [(); 3].map(|()| parts.next().unwrap_or(0))
}

#[derive(Debug, Clone, Deserialize)]
struct Terminal {
    address: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    foreground: Option<Foreground>,
    #[serde(default)]
    status: Option<Status>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    turn: u64,
}

impl Terminal {
    fn state(&self) -> Option<&str> {
        self.status.as_ref().map(|s| s.state.as_str())
    }
}

/// Only the agent CLI the leader runs is kept: its argv can hold secrets.
#[derive(Debug, Clone, Deserialize)]
#[serde(from = "RawForeground")]
struct Foreground {
    cli: Option<String>,
    cwd: String,
}

#[derive(Deserialize)]
struct RawForeground {
    #[serde(default)]
    argv: Vec<String>,
    #[serde(default)]
    cwd: String,
}

impl From<RawForeground> for Foreground {
    fn from(raw: RawForeground) -> Self {
        let name = raw
            .argv
            .first()
            .and_then(|a| Path::new(a).file_name())
            .and_then(|n| n.to_str());
        Self {
            cli: name.filter(|n| AGENT_CLIS.contains(n)).map(str::to_owned),
            cwd: raw.cwd,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Status {
    state: String,
    #[serde(default)]
    app: Option<String>,
}

#[derive(Deserialize)]
struct Snapshot {
    seq: u64,
    #[serde(default)]
    terminals: Vec<Terminal>,
}

#[derive(Deserialize)]
struct Notice {
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
struct Event {
    seq: u64,
    terminal: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    data: Value,
}

struct Held {
    terminal: Terminal,
    /// `state_change_seq`: the seq of the event where the status's state last changed.
    seq: u64,
}

struct Live {
    group: String,
    seq: u64,
    terminals: BTreeMap<String, Held>,
}

impl Live {
    /// A terminal first seen takes the snapshot's seq, and one already held takes it only
    /// when its state or turn moved, so a dialog seen across a gap still gets a new seq.
    fn snapshot(&mut self, seq: u64, terminals: Vec<Terminal>) {
        let mut next = BTreeMap::new();
        for t in terminals.into_iter().filter(|t| local(&t.address)) {
            let kept = self
                .terminals
                .get(&t.address)
                .filter(|h| h.terminal.state() == t.state() && h.terminal.turn == t.turn)
                .map_or(seq, |h| h.seq);
            next.insert(
                t.address.clone(),
                Held {
                    terminal: t,
                    seq: kept,
                },
            );
        }
        self.terminals = next;
        self.seq = seq;
    }

    fn event(&mut self, e: Event) {
        if e.seq <= self.seq {
            return;
        }
        self.seq = e.seq;
        if !local(&e.terminal) {
            return;
        }
        match e.kind.as_str() {
            "created" | "restarted" => {
                if let Ok(t) = serde_json::from_value::<Terminal>(e.data)
                    && t.address == e.terminal
                {
                    let kept = self
                        .terminals
                        .get(&t.address)
                        .filter(|h| h.terminal.state() == t.state() && h.terminal.turn == t.turn)
                        .map_or(e.seq, |h| h.seq);
                    self.terminals.insert(
                        e.terminal,
                        Held {
                            terminal: t,
                            seq: kept,
                        },
                    );
                }
            }
            "removed" => {
                self.terminals.remove(&e.terminal);
            }
            kind => {
                let Some(h) = self.terminals.get_mut(&e.terminal) else {
                    return;
                };
                let t = &mut h.terminal;
                match kind {
                    "finished" | "lost" => {
                        t.state = "finished".to_owned();
                        t.foreground = None;
                        t.status = None;
                    }
                    "foreground" => t.foreground = serde_json::from_value(e.data).ok().flatten(),
                    "status" => {
                        #[derive(Deserialize)]
                        struct Data {
                            #[serde(default)]
                            status: Option<Status>,
                        }
                        if let Ok(Data { status }) = serde_json::from_value(e.data) {
                            if status.as_ref().map(|s| s.state.as_str()) != t.state() {
                                h.seq = e.seq;
                            }
                            t.status = status;
                        }
                    }
                    "title" => {
                        t.title = e.data["title"].as_str().unwrap_or_default().to_owned();
                    }
                    "turn" => {
                        if let Some(turn) = e.data["turn"].as_u64() {
                            t.turn = turn;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn notice(&mut self, line: &[u8]) {
        let Ok(notice) = serde_json::from_slice::<Notice>(line) else {
            return;
        };
        match notice.method.as_str() {
            "event" => {
                if let Ok(e) = serde_json::from_value(notice.params) {
                    self.event(e);
                }
            }
            "missed" => {
                if let Ok(s) = serde_json::from_value::<Snapshot>(notice.params) {
                    self.snapshot(s.seq, s.terminals);
                }
            }
            _ => {}
        }
    }

    fn agents(&self) -> Vec<AgentInfo> {
        self.terminals
            .values()
            .filter_map(|h| agent(&self.group, h))
            .collect()
    }
}

/// Terminals on other machines (`box/t7`) are skipped, so the phone never reaches another
/// machine through the core.
fn local(address: &str) -> bool {
    !address.is_empty() && address.len() <= 28 && address.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A terminal is an agent once it reports OSC 7501 or runs an agent CLI; plain shells are not.
fn agent(group: &str, h: &Held) -> Option<AgentInfo> {
    let t = &h.terminal;
    if t.state != "live" {
        return None;
    }
    let cli = t.foreground.as_ref().and_then(|f| f.cli.as_deref());
    let kind = match &t.status {
        Some(s) => match s.app.as_deref() {
            Some("claude-code") => Some("claude".to_owned()),
            Some(app) if !app.is_empty() => Some(app.to_owned()),
            _ => cli.map(str::to_owned),
        },
        None => Some(cli?.to_owned()),
    };
    let status = t
        .state()
        .filter(|s| matches!(*s, "idle" | "working" | "blocked" | "done"))
        .unwrap_or("unknown");
    let id = format!("{group}:{}", t.address);
    let some = |s: &str| (!s.is_empty()).then(|| s.to_owned());
    Some(AgentInfo {
        terminal_id: id.clone(),
        workspace_id: group.to_owned(),
        pane_id: id,
        agent: kind,
        name: t.name.as_deref().and_then(some),
        title: some(title(&t.title)),
        terminal_title_stripped: None,
        agent_status: status.to_owned(),
        state_change_seq: h.seq,
        cwd: some(&t.cwd),
        foreground_cwd: t.foreground.as_ref().and_then(|f| some(&f.cwd)),
        agent_session: None,
        interactive_ready: false,
        launch_pending: false,
    })
}

/// Without the spinner or status glyph an agent puts before its title (Codex redraws one
/// every 100 ms while it works), so a spinner tick changes nothing the phone shows.
fn title(raw: &str) -> &str {
    raw.trim_start_matches(|c: char| c.is_whitespace() || (!c.is_ascii() && !c.is_alphanumeric()))
        .trim_end()
}

/// One group per core, numbered after herdr's workspaces; none without agents.
pub fn group(agents: &[AgentInfo], herdr: &[WorkspaceInfo]) -> Option<WorkspaceInfo> {
    let first = agents.first()?;
    let rank = |s: &str| match s {
        "blocked" => 4,
        "working" => 3,
        "done" => 2,
        "idle" => 1,
        _ => 0,
    };
    let status = agents
        .iter()
        .map(|a| a.agent_status.as_str())
        .max_by_key(|s| rank(s))
        .unwrap_or("unknown");
    Some(WorkspaceInfo {
        workspace_id: first.workspace_id.clone(),
        number: herdr.iter().map(|w| w.number).max().unwrap_or(0) + 1,
        label: GROUP_LABEL.to_owned(),
        active_tab_id: None,
        agent_status: status.to_owned(),
        worktree: None,
    })
}

pub fn pane(a: &AgentInfo) -> PaneInfo {
    PaneInfo {
        pane_id: a.pane_id.clone(),
        terminal_id: a.terminal_id.clone(),
        workspace_id: a.workspace_id.clone(),
        tab_id: a.workspace_id.clone(),
        cwd: a.cwd.clone(),
        foreground_cwd: a.foreground_cwd.clone(),
        agent: a.agent.clone(),
        label: a.name.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub socket: String,
    #[serde(default)]
    pub folder_mode: Option<u32>,
    /// None while no core answers.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub generation: Option<u32>,
    #[serde(default)]
    pub missing: Vec<String>,
}

#[derive(Default)]
struct Inner {
    hello: Option<Hello>,
    live: Option<Live>,
}

/// One core, followed through `events.subscribe`. collied never starts a core: an absent
/// socket means none runs.
pub struct Core {
    socket: PathBuf,
    inner: Mutex<Inner>,
    settled: watch::Sender<bool>,
    herdr: AtomicU8,
}

impl Core {
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            inner: Mutex::default(),
            settled: watch::channel(false).0,
            herdr: AtomicU8::new(0),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        crate::lock(&self.inner)
    }

    pub fn agents(&self) -> Option<Vec<AgentInfo>> {
        self.lock().live.as_ref().map(Live::agents)
    }

    /// Returns once the first connection attempt is over, so a core that is running when
    /// collied starts is listed from the first reconcile.
    pub async fn settled(&self) {
        let _ = self.settled.subscribe().wait_for(|s| *s).await;
    }

    pub fn report(&self) -> Report {
        let hello = self.lock().hello.clone();
        Report {
            socket: self.socket.display().to_string(),
            folder_mode: self
                .socket
                .parent()
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777),
            version: hello.as_ref().map(|h| h.version.clone()),
            generation: hello.as_ref().map(|h| h.generation),
            missing: hello.map_or_else(Vec::new, |h| {
                h.missing().into_iter().map(str::to_owned).collect()
            }),
        }
    }

    /// A core that never answered stays quiet; one that stops after answering logs once.
    pub async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        let mut delay = RETRY_FIRST;
        let mut refused: Option<String> = None;
        loop {
            let ended = tokio::select! {
                e = self.follow() => e,
                _ = shutdown.changed() => return,
            };
            let followed = self.lock().live.take().is_some();
            self.settled.send_replace(true);
            match ended {
                e if followed => {
                    tracing::warn!(socket = %self.socket.display(), error = %e, "herdr-core unavailable, retrying with backoff");
                    refused = None;
                    delay = RETRY_FIRST;
                }
                Error::Unsupported(why) => {
                    if refused.as_ref() != Some(&why) {
                        tracing::warn!(socket = %self.socket.display(), "herdr-core {why}; not following it");
                        refused = Some(why);
                    }
                }
                _ => self.lock().hello = None,
            }
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = shutdown.changed() => return,
            }
            delay = (delay * 2).min(RETRY_MAX);
        }
    }

    /// herdr's reachability as calls merged with this core saw it, so that losing herdr
    /// while the core answers is logged once, and a herdr that never answered stays quiet.
    pub(super) fn herdr_seen(&self, error: Option<&Error>) {
        const UP: u8 = 1;
        const DOWN: u8 = 2;
        match error {
            None => {
                if self.herdr.swap(UP, Ordering::Relaxed) == DOWN {
                    tracing::info!("herdr reachable again");
                }
            }
            Some(e) => {
                if self
                    .herdr
                    .compare_exchange(UP, DOWN, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    tracing::warn!(error = %e, "herdr unavailable, herdr-core still answers");
                }
            }
        }
    }

    /// Follows the core until the connection ends; the error says why.
    async fn follow(&self) -> Error {
        let mut conn = match Conn::open(&self.socket).await {
            Ok(c) => c,
            Err(e) => return e,
        };
        let hello: Hello = match conn
            .call("hello", json!({ "generation": GENERATION }), TIMEOUT)
            .await
        {
            Ok(h) => h,
            Err(e) => return e,
        };
        let problem = hello.problem();
        let group = format!("{PREFIX}{}", hello.core.id);
        let version = hello.version.clone();
        self.lock().hello = Some(hello);
        if let Some(why) = problem {
            return Error::Unsupported(why);
        }
        let snap: Snapshot = match conn.call("events.subscribe", json!({}), TIMEOUT).await {
            Ok(s) => s,
            Err(e) => return e,
        };
        let mut live = Live {
            group,
            seq: 0,
            terminals: BTreeMap::new(),
        };
        live.snapshot(snap.seq, snap.terminals);
        self.lock().live = Some(live);
        self.settled.send_replace(true);
        tracing::info!(socket = %self.socket.display(), version, "following herdr-core");
        loop {
            let line = match tokio::time::timeout(SILENCE, conn.line()).await {
                Ok(Ok(line)) => line,
                Ok(Err(e)) => return e,
                Err(_) => return Error::Timeout,
            };
            let mut inner = self.lock();
            let Some(live) = inner.live.as_mut() else {
                return Error::Closed;
            };
            live.notice(&line);
        }
    }

    /// Only a terminal collied lists.
    pub async fn close(&self, id: &str) -> Result<(), Error> {
        let address = {
            let inner = self.lock();
            inner.live.as_ref().and_then(|live| {
                id.strip_prefix(live.group.as_str())?
                    .strip_prefix(':')
                    .filter(|a| live.terminals.contains_key(*a))
                    .map(str::to_owned)
            })
        }
        .ok_or_else(|| Error::Core {
            kind: "not_found".to_owned(),
            message: "no such terminal".to_owned(),
        })?;
        let mut conn = Conn::open(&self.socket).await?;
        conn.call::<Value>(
            "terminal.kill",
            json!({ "terminal": address }),
            KILL_TIMEOUT,
        )
        .await?;
        conn.call::<Value>("terminal.rm", json!({ "terminal": address }), TIMEOUT)
            .await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct Holder {
    pid: u32,
}

#[derive(Debug, Deserialize)]
struct ServerStatus {
    #[serde(default)]
    holder: Option<Holder>,
}

pub async fn probe(socket: &Path) -> Result<(Hello, Option<u32>), Error> {
    let mut conn = Conn::open(socket).await?;
    let hello: Hello = conn
        .call("hello", json!({ "generation": GENERATION }), TIMEOUT)
        .await?;
    let status: Result<ServerStatus, _> = conn.call("server.status", json!({}), TIMEOUT).await;
    Ok((hello, status.ok().and_then(|s| s.holder).map(|h| h.pid)))
}

/// JSON-RPC 2.0 in line mode; the connection keeps answering, unlike herdr's.
struct Conn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    next: u64,
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<ErrorBody>,
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Value,
}

impl Conn {
    async fn open(socket: &Path) -> Result<Self, Error> {
        let stream = tokio::time::timeout(TIMEOUT, UnixStream::connect(socket))
            .await
            .map_err(|_| Error::Timeout)??;
        // The socket is 0755: its folder alone keeps other users from serving one there.
        if stream.peer_cred()?.uid() != rustix::process::getuid().as_raw() {
            return Err(Error::Unsupported("is served by another user".to_owned()));
        }
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
            next: 0,
        })
    }

    async fn line(&mut self) -> Result<Vec<u8>, Error> {
        let mut buf = Vec::new();
        (&mut self.reader)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut buf)
            .await?;
        if buf.last() != Some(&b'\n') {
            return Err(if buf.len() as u64 > MAX_LINE {
                Error::LineTooLong
            } else {
                Error::Closed
            });
        }
        Ok(buf)
    }

    async fn call<R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<R, Error> {
        if !METHODS.contains(&method) {
            return Err(Error::Unsupported(format!(
                "{method} is not a method collied calls"
            )));
        }
        self.next += 1;
        let id = self.next;
        let mut line = serde_json::to_vec(
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )?;
        line.push(b'\n');
        tracing::debug!(method, id, "herdr-core request");
        tokio::time::timeout(timeout, async {
            self.writer.write_all(&line).await?;
            loop {
                let reply: Reply = serde_json::from_slice(&self.line().await?)?;
                if reply.id != json!(id) {
                    continue;
                }
                if let Some(e) = reply.error {
                    return Err(Error::Core {
                        kind: e.data["kind"].as_str().unwrap_or("error").to_owned(),
                        message: e.message,
                    });
                }
                return Ok(serde_json::from_value(reply.result.ok_or(Error::Empty)?)?);
            }
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Runtime;

    const ID: &str = "0123456789abcdef0123456789abcdef";

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.into())
    }

    #[test]
    fn socket_resolves_like_hc() {
        let none = |_: &Path| None;
        let path = |vars: &[(&str, &str)], run: Option<&str>| {
            resolve(&env(vars), run.map(PathBuf::from), &none).unwrap()
        };
        let home = [("HOME", "/h")];
        assert_eq!(
            path(&home, None),
            PathBuf::from("/h/.local/state/herdr-core/default/default.sock")
        );
        assert_eq!(
            path(
                &[
                    ("HOME", "/h"),
                    ("XDG_STATE_HOME", "/s"),
                    ("HC_SESSION", " w ")
                ],
                None
            ),
            PathBuf::from("/s/herdr-core/w/w.sock")
        );
        assert_eq!(
            path(&home, Some("/run/user/501")),
            PathBuf::from("/run/user/501/herdr-core/default.sock")
        );
        assert_eq!(
            path(
                &[
                    ("HOME", "/h"),
                    ("XDG_RUNTIME_DIR", "/r"),
                    ("HC_SESSION", "")
                ],
                Some("/run/user/501")
            ),
            PathBuf::from("/r/herdr-core/default.sock")
        );
        assert_eq!(
            path(
                &[
                    ("HOME", "/h"),
                    ("HC_SOCKET", "/x.sock"),
                    ("XDG_RUNTIME_DIR", "/r")
                ],
                None
            ),
            PathBuf::from("/x.sock")
        );
        let config = |p: &Path| {
            (p == Path::new("/c/herdr-core/config.toml"))
                .then(|| "socket = \"/cfg.sock\"\n".to_owned())
        };
        let vars = env(&[("HOME", "/h"), ("XDG_CONFIG_HOME", "/c")]);
        assert_eq!(
            resolve(&vars, None, &config).unwrap(),
            PathBuf::from("/cfg.sock")
        );
        let broken = |_: &Path| Some("socket = [".to_owned());
        assert_eq!(
            resolve(&env(&home), None, &broken).unwrap(),
            PathBuf::from("/h/.local/state/herdr-core/default/default.sock")
        );
        for bad in ["..", "a/b", "a:b", "a\u{7}"] {
            assert!(resolve(&env(&[("HOME", "/h"), ("HC_SESSION", bad)]), None, &none).is_err());
        }
        assert!(resolve(&env(&[]), None, &none).is_err());
    }

    fn hello(generation: u32, version: &str, methods: &[&str]) -> Hello {
        serde_json::from_value(json!({
            "core": {"id": ID, "name": "mac"}, "generation": generation, "version": version,
            "methods": methods, "framed": false, "features": [],
        }))
        .unwrap()
    }

    #[test]
    fn hello_names_what_keeps_a_core_unused() {
        assert_eq!(hello(1, "0.1.16", &METHODS).problem(), None);
        assert_eq!(hello(1, "0.2.0", &METHODS).problem(), None);
        assert!(
            hello(2, "0.1.16", &METHODS)
                .problem()
                .unwrap()
                .contains("generation 2")
        );
        assert!(
            hello(1, "0.1.15", &METHODS)
                .problem()
                .unwrap()
                .contains("older")
        );
        let lacking = hello(1, "0.1.16", &METHODS[..3]);
        assert_eq!(lacking.missing(), ["terminal.kill", "terminal.rm"]);
        assert_eq!(
            lacking.problem().as_deref(),
            Some("lacks terminal.kill, terminal.rm")
        );
        let mut odd = hello(1, "0.1.16", &METHODS);
        odd.core.id = "../x".into();
        assert!(odd.problem().is_some());
    }

    fn terminal(address: &str, status: Value, argv0: &str) -> Value {
        json!({
            "address": address, "name": null, "cwd": "/w", "state": "live", "argv": [argv0],
            "labels": {}, "created_ms": 1, "screen": "exact", "cols": 80, "rows": 24,
            "foreground": {"pid": 9, "name": "x", "argv": [argv0, "--secret"], "cwd": "/w/sub"},
            "status": status, "title": "", "turn": 0,
        })
    }

    fn live(seq: u64, terminals: Value) -> Live {
        let mut live = Live {
            group: format!("hc:{ID}"),
            seq: 0,
            terminals: BTreeMap::new(),
        };
        live.snapshot(seq, serde_json::from_value(terminals).unwrap());
        live
    }

    fn event(live: &mut Live, seq: u64, terminal: &str, kind: &str, data: Value) {
        live.event(
            serde_json::from_value(
                json!({"seq": seq, "terminal": terminal, "type": kind, "data": data}),
            )
            .unwrap(),
        );
    }

    fn shown(live: &Live) -> Vec<(String, String, Option<String>, u64)> {
        live.agents()
            .into_iter()
            .map(|a| (a.terminal_id, a.agent_status, a.agent, a.state_change_seq))
            .collect()
    }

    #[test]
    fn agents_are_terminals_that_report_or_run_an_agent_cli() {
        let l = live(
            10,
            json!([
                terminal(
                    "t1",
                    json!({"state": "idle", "app": "claude-code"}),
                    "/bin/claude"
                ),
                terminal("t2", Value::Null, "/bin/zsh"),
                terminal("t3", Value::Null, "/opt/homebrew/bin/copilot"),
                terminal("t4", json!({"state": "error", "app": "pi"}), "node"),
                terminal(
                    "box/t7",
                    json!({"state": "idle", "app": "claude-code"}),
                    "claude"
                ),
            ]),
        );
        let id = |t: &str| format!("hc:{ID}:{t}");
        assert_eq!(
            shown(&l),
            [
                (id("t1"), "idle".into(), Some("claude".into()), 10),
                (id("t3"), "unknown".into(), Some("copilot".into()), 10),
                (id("t4"), "unknown".into(), Some("pi".into()), 10),
            ]
        );
        let a = &l.agents()[0];
        assert_eq!(
            (a.pane_id.as_str(), a.workspace_id.as_str()),
            (id("t1").as_str(), format!("hc:{ID}").as_str())
        );
        assert_eq!(
            (a.cwd.as_deref(), a.foreground_cwd.as_deref()),
            (Some("/w"), Some("/w/sub"))
        );
        assert!(protocol::TerminalId::new(a.terminal_id.clone()).is_ok());
        let json = format!(
            "{:?}",
            l.terminals
                .values()
                .map(|h| &h.terminal)
                .collect::<Vec<_>>()
        );
        assert!(!json.contains("--secret"), "{json}");
    }

    #[test]
    fn state_change_seq_moves_only_when_the_state_does() {
        let t1 = format!("hc:{ID}:t1");
        let mut l = live(
            10,
            json!([terminal(
                "t1",
                json!({"state": "idle", "app": "claude-code"}),
                "claude"
            )]),
        );
        let status = |state: &str| json!({"reported": true, "records": {}, "status": {"state": state, "app": "claude-code"}});
        event(&mut l, 11, "t1", "status", status("working"));
        event(&mut l, 12, "t1", "status", status("working"));
        assert_eq!(shown(&l)[0].3, 11);
        event(&mut l, 13, "t1", "status", status("blocked"));
        event(
            &mut l,
            14,
            "t1",
            "turn",
            json!({"state": "blocked", "turn": 1}),
        );
        event(
            &mut l,
            15,
            "t1",
            "title",
            json!({"title": "⠋ Fix the flaky test"}),
        );
        assert_eq!(
            shown(&l),
            [(t1.clone(), "blocked".into(), Some("claude".into()), 13)]
        );
        assert_eq!(l.agents()[0].title.as_deref(), Some("Fix the flaky test"));

        // A snapshot after a gap keeps the seq for the same state and turn, and moves it for
        // a new dialog: blocked, idle, blocked again with a later turn.
        let mut again = terminal(
            "t1",
            json!({"state": "blocked", "app": "claude-code"}),
            "claude",
        );
        again["turn"] = json!(1);
        l.snapshot(20, serde_json::from_value(json!([again.clone()])).unwrap());
        assert_eq!(shown(&l)[0].3, 13);
        again["turn"] = json!(2);
        l.snapshot(25, serde_json::from_value(json!([again])).unwrap());
        assert_eq!(shown(&l)[0].3, 25);
        event(&mut l, 24, "t1", "status", status("idle"));
        assert_eq!(shown(&l)[0].1, "blocked");

        event(
            &mut l,
            26,
            "t1",
            "status",
            json!({"reported": true, "status": null}),
        );
        event(
            &mut l,
            27,
            "t1",
            "foreground",
            json!({"pid": 3, "name": "zsh", "argv": ["-zsh"], "cwd": "/w"}),
        );
        assert!(l.agents().is_empty());
        event(
            &mut l,
            28,
            "t1",
            "foreground",
            json!({"pid": 4, "name": "codex", "argv": ["/b/codex"], "cwd": "/w"}),
        );
        assert_eq!(
            shown(&l),
            [(t1.clone(), "unknown".into(), Some("codex".into()), 26)]
        );
        event(
            &mut l,
            29,
            "t1",
            "finished",
            json!({"exit_code": 0, "reason": "exited"}),
        );
        assert!(l.agents().is_empty());
        event(
            &mut l,
            30,
            "t2",
            "created",
            terminal("t2", Value::Null, "claude"),
        );
        event(
            &mut l,
            31,
            "box/t2",
            "created",
            terminal("box/t2", Value::Null, "claude"),
        );
        assert_eq!(shown(&l)[0].3, 30);
        event(&mut l, 32, "t2", "removed", json!({}));
        assert!(l.agents().is_empty() && l.terminals.len() == 1);
    }

    #[test]
    fn notices_from_the_wire_follow_the_core() {
        let mut l = live(10, json!([]));
        let line = |v: Value| serde_json::to_vec(&v).unwrap();
        let claude = terminal(
            "t1",
            json!({"state": "idle", "app": "claude-code"}),
            "claude",
        );
        l.notice(&line(
            json!({"jsonrpc": "2.0", "method": "event", "params": {
                "seq": 11, "terminal": "t1", "type": "created", "data": claude,
            }}),
        ));
        l.notice(&line(
            json!({"jsonrpc": "2.0", "method": "heartbeat", "params": {"seq": 11}}),
        ));
        l.notice(b"not json");
        assert_eq!(shown(&l)[0].3, 11);

        let mut blocked = terminal(
            "t1",
            json!({"state": "blocked", "app": "claude-code"}),
            "claude",
        );
        blocked["turn"] = json!(1);
        let codex = terminal("t2", json!({"state": "working", "app": "codex"}), "codex");
        l.notice(&line(
            json!({"jsonrpc": "2.0", "method": "missed", "params": {
                "seq": 40, "terminals": [blocked, codex],
            }}),
        ));
        let id = |t: &str| format!("hc:{ID}:{t}");
        assert_eq!(
            shown(&l),
            [
                (id("t1"), "blocked".into(), Some("claude".into()), 40),
                (id("t2"), "working".into(), Some("codex".into()), 40),
            ]
        );
        l.notice(&line(
            json!({"jsonrpc": "2.0", "method": "event", "params": {
                "seq": 39, "terminal": "t2", "type": "removed", "data": {},
            }}),
        ));
        assert_eq!(
            l.agents().len(),
            2,
            "events up to the snapshot's seq are ignored"
        );
        l.notice(&line(
            json!({"jsonrpc": "2.0", "method": "missed", "params": {
                "seq": 50, "terminals": [],
            }}),
        ));
        assert!(l.agents().is_empty());
    }

    fn agent(status: &str) -> AgentInfo {
        serde_json::from_value(json!({
            "terminal_id": format!("hc:{ID}:t1"), "workspace_id": format!("hc:{ID}"),
            "pane_id": format!("hc:{ID}:t1"), "agent_status": status,
        }))
        .unwrap()
    }

    #[test]
    fn one_group_numbered_after_herdr() {
        assert!(group(&[], &[]).is_none());
        let g = group(&[agent("idle"), agent("blocked"), agent("unknown")], &[]).unwrap();
        assert_eq!(
            (
                g.workspace_id.as_str(),
                g.label.as_str(),
                g.number,
                g.agent_status.as_str()
            ),
            (format!("hc:{ID}").as_str(), GROUP_LABEL, 1, "blocked")
        );
        let herdr: Vec<WorkspaceInfo> = serde_json::from_value(json!([
            {"workspace_id": "w1", "number": 1, "label": "a", "active_tab_id": null, "agent_status": "idle"},
            {"workspace_id": "w4", "number": 3, "label": "b", "active_tab_id": null, "agent_status": "idle"},
        ]))
        .unwrap();
        assert_eq!(group(&[agent("done")], &herdr).unwrap().number, 4);
    }

    fn followed(socket: PathBuf) -> Arc<Core> {
        let core = Core::new(socket);
        core.lock().live = Some(live(
            10,
            json!([terminal(
                "t1",
                json!({"state": "working", "app": "codex"}),
                "codex"
            )]),
        ));
        Arc::new(core)
    }

    #[tokio::test]
    async fn without_a_followed_core_herdr_answers_alone() {
        let dir = tempfile::tempdir().unwrap();
        let herdr = dir.path().join("herdr.sock");
        let core = Arc::new(Core::new(dir.path().join("core.sock")));
        let rt = Runtime::new(herdr.clone()).with_core(Some(core));
        assert!(matches!(rt.agent_list().await, Err(Error::Io(_))));
        assert!(matches!(rt.session_snapshot().await, Err(Error::Io(_))));

        let rt = Runtime::new(herdr).with_core(Some(followed(dir.path().join("core.sock"))));
        let agents = rt.agent_list().await.unwrap();
        assert_eq!(agents.len(), 1);
        let snap = rt.session_snapshot().await.unwrap();
        assert_eq!(
            (snap.agents.len(), snap.panes.len(), snap.workspaces.len()),
            (1, 1, 1)
        );
        assert_eq!(snap.panes[0].terminal_id, agents[0].terminal_id);
        let poll = rt.poll().await.unwrap();
        assert_eq!(poll.workspaces[0].label, GROUP_LABEL);
        assert_eq!(poll.agents.len(), 1);
        assert!(
            !poll.herdr,
            "herdr's agents are not known gone while it is down"
        );
    }

    #[tokio::test]
    async fn archive_refuses_a_herdr_core_agent() {
        let dir = tempfile::tempdir().unwrap();
        let drive = crate::drive::Driver::new(
            dir.path().join("herdr.sock"),
            vec![],
            &[dir.path().to_owned()],
        )
        .unwrap()
        .with_core(Some(followed(dir.path().join("core.sock"))));
        let yes: crate::drive::Authorized = Arc::new(|| true);
        let archive = |id: String| protocol::TaskArchiveParams {
            op_id: protocol::OpId::new("a".repeat(22)).unwrap(),
            terminal_id: protocol::TerminalId::new(id).unwrap(),
            confirm: true,
        };
        assert_eq!(
            drive
                .task_archive(archive(format!("hc:{ID}:t1")), &yes)
                .await,
            Err((
                protocol::ErrorCode::Internal,
                crate::runtime::NOT_YET.to_owned()
            ))
        );
        assert_eq!(
            drive
                .task_archive(archive("term_1".into()), &yes)
                .await
                .unwrap_err()
                .0,
            protocol::ErrorCode::HerdrUnavailable
        );
    }

    #[tokio::test]
    async fn herdr_core_ids_never_reach_herdr() {
        let dir = tempfile::tempdir().unwrap();
        let rt = Runtime::new(dir.path().join("herdr.sock"))
            .with_core(Some(followed(dir.path().join("core.sock"))));
        let id = format!("hc:{ID}:t1");
        assert!(matches!(
            rt.agent_get(&id).await,
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            rt.detection_text(&id).await,
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            rt.agent_prompt(&id, "x").await,
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            rt.workspace_close(&format!("hc:{ID}")).await,
            Err(Error::Unsupported(m)) if m.contains("cannot be closed")
        ));
        assert!(matches!(
            rt.pane_close(&format!("hc:{ID}:t9")).await,
            Err(Error::Core { kind, .. }) if kind == "not_found"
        ));
        assert!(matches!(rt.pane_close(&id).await, Err(Error::Io(_))));
        assert!(matches!(rt.agent_get("w1:p1").await, Err(Error::Io(_))));
    }
}
