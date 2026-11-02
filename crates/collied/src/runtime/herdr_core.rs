use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::watch;

use super::herdr::{Explain, MatchedRule};
use super::screen::Screen;
use super::{AgentInfo, AgentSession, Error, PaneInfo, PaneRead, WorkspaceInfo};

pub const GENERATION: u32 = 1;
pub const MIN_VERSION: &str = "0.1.16";
/// Every method collied calls. The API also runs programs and writes files on other machines,
/// stores documents and stops the core; none of that is ever called.
pub const METHODS: [&str; 11] = [
    "hello",
    "server.status",
    "events.subscribe",
    "terminal.attach",
    "terminal.kill",
    "terminal.rm",
    "terminal.get",
    "terminal.read",
    "terminal.send",
    "terminal.keys",
    "terminal.run",
];
/// herdr's and Claude Code's session variables, which a core passes from whatever started
/// it to every terminal: herdr's hooks would report a herdr-core agent's session against the
/// starter's herdr pane, and Claude Code with `CLAUDE_CODE_CHILD_SESSION` saves no transcript.
/// Named, not matched by pattern, so variables the user chose (`CLAUDE_CONFIG_DIR`) stay.
pub const STRIPPED: [&str; 16] = [
    "HERDR_ENV",
    "HERDR_PANE_ID",
    "HERDR_SOCKET_PATH",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
    "HERDR_BIN_PATH",
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_EFFORT",
    "CLAUDE_PID",
];
/// The kinds a task starts in herdr-core: those that report OSC 7501, without which nothing
/// says the agent is ready or what dialog it shows.
pub const TASK_KINDS: [&str; 2] = ["claude", "codex"];
/// Set on the terminals collied starts as tasks, to the agent kind.
const TASK_LABEL: &str = "collie.kind";
/// Set on a worktree task's terminals, to the checkout collied created or opened for it:
/// only that checkout is ever removed by an archive.
const WORKTREE_LABEL: &str = "collie.worktree";

/// A terminal the core holds, shell or agent, with the folders it works in.
pub struct Occupant {
    pub id: String,
    /// Started by collied as a task.
    pub task: bool,
    /// The checkout collied recorded for the worktree task it started there.
    pub worktree: Option<String>,
    pub cwd: String,
    pub foreground_cwd: Option<String>,
}
pub const GROUP_LABEL: &str = "Herdr 2";
const PREFIX: &str = "hc:";
const AGENT_CLIS: [&str; 3] = ["claude", "codex", "copilot"];
const SHELLS: [&str; 9] = [
    "sh", "bash", "zsh", "fish", "dash", "ksh", "tcsh", "csh", "nu",
];
const TIMEOUT: Duration = Duration::from_secs(5);
const KILL_TIMEOUT: Duration = Duration::from_secs(15);
/// A send or keys call first waits until nobody has typed for about 3 s, and input through
/// a typing attach counts as typing.
const INPUT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long after the last typing the core holds a send or keys call: measured 2.8 to 3.3 s
/// on herdr-core 0.1.16.
pub const TYPING_HOLD: Duration = Duration::from_millis(3500);
/// The core sends a heartbeat every 15 s.
const SILENCE: Duration = Duration::from_secs(45);
const MAX_LINE: u64 = 16 << 20;
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);
/// The history a watch keeps: the most the phone reads (`MAX_READ_LINES`).
const HISTORY: u32 = protocol::limits::MAX_READ_LINES as u32;
const DATA: u8 = 0;
const CREDIT: u8 = 1;
const CLOSE: u8 = 2;
const MAX_FRAME: usize = 16 << 20;
/// The core paints the screen in one burst right after its attach answer.
const PAINT_QUIET: Duration = Duration::from_millis(20);
const PAINT_MAX: Duration = Duration::from_secs(1);
/// A watch-only attach never sizes the terminal, and nothing on the stream says when a
/// desktop client did, so a watch checks the size the core's events report.
const SIZE_CHECK: Duration = Duration::from_millis(500);

pub fn owns(id: &str) -> bool {
    id.starts_with(PREFIX)
}

/// A task kind's CLI by absolute path: `terminal.run` starts a template for a bare name that
/// matches one. A path holding `=` would be read as a variable by `env`.
pub fn cli(kind: &str) -> Option<PathBuf> {
    if !TASK_KINDS.contains(&kind) {
        return None;
    }
    crate::usage::program(kind)
        .filter(|p| p.is_absolute() && p.to_str().is_some_and(|s| !s.contains('=')))
}

/// The argv a task agent runs as: `env` drops each of `STRIPPED` first, since `terminal.run`'s
/// `env` is set on top of the core's environment and cannot remove a variable.
fn task_argv(cli: &Path, args: &[&str]) -> Option<Vec<String>> {
    let mut argv = vec!["/usr/bin/env".to_owned()];
    for name in STRIPPED {
        argv.extend(["-u".to_owned(), name.to_owned()]);
    }
    argv.push(cli.to_str()?.to_owned());
    argv.extend(args.iter().map(|a| (*a).to_owned()));
    Some(argv)
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
    #[serde(default)]
    cols: u16,
    #[serde(default)]
    rows: u16,
    #[serde(default)]
    labels: BTreeMap<String, String>,
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
    pid: Option<u32>,
    cli: Option<String>,
    /// A login shell's `argv[0]` starts with `-`.
    shell: bool,
    cwd: String,
}

#[derive(Deserialize)]
struct RawForeground {
    #[serde(default)]
    pid: Option<u32>,
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
            pid: raw.pid,
            cli: name.filter(|n| AGENT_CLIS.contains(n)).map(str::to_owned),
            shell: name.is_some_and(|n| SHELLS.contains(&n.trim_start_matches('-'))),
            cwd: raw.cwd,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Status {
    state: String,
    #[serde(default)]
    app: Option<String>,
    /// `permission`, `question` or `auth`, on a blocked report.
    #[serde(default)]
    kind: Option<String>,
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
                    "size" => {
                        let side = |k: &str| e.data[k].as_u64().and_then(|n| u16::try_from(n).ok());
                        if let (Some(cols), Some(rows)) = (side("cols"), side("rows")) {
                            (t.cols, t.rows) = (cols, rows);
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

    fn agents(&self, session: impl Fn(&str, &str, u32) -> Option<String>) -> Vec<AgentInfo> {
        self.terminals
            .values()
            .filter_map(|h| agent(&self.group, h, &session))
            .collect()
    }
}

fn not_found() -> Error {
    Error::Core {
        kind: "not_found".to_owned(),
        message: "no such terminal".to_owned(),
    }
}

/// Terminals on other machines (`box/t7`) are skipped, so the phone never reaches another
/// machine through the core.
fn local(address: &str) -> bool {
    !address.is_empty() && address.len() <= 28 && address.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A terminal is an agent once it reports OSC 7501 or runs an agent CLI; plain shells are not.
/// `session` gives the session of an agent kind, terminal address and foreground pid.
fn agent(
    group: &str,
    h: &Held,
    session: &dyn Fn(&str, &str, u32) -> Option<String>,
) -> Option<AgentInfo> {
    let t = &h.terminal;
    if t.state != "live" {
        return None;
    }
    // A shell leads the terminal, so input would reach it: the status the core still holds
    // is that of a program that exited or is suspended (Ctrl+Z), which herdr-core 0.1.16
    // keeps, or of one a shell wrapper runs without handing it the terminal.
    if t.foreground.as_ref().is_some_and(|f| f.shell) {
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
    let pid = t.foreground.as_ref().and_then(|f| f.pid);
    let session = kind
        .as_deref()
        .zip(pid)
        .and_then(|(kind, pid)| session(kind, &t.address, pid));
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
        agent_session: session.map(|value| AgentSession { value }),
        interactive_ready: false,
        launch_pending: false,
        read_only: t.status.is_none(),
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
    /// Codex sessions by terminal address, as collied's SessionStart hook reported them,
    /// with the foreground pid they were reported under: a session holds only while that
    /// process leads the terminal.
    codex: BTreeMap<String, (u32, String)>,
}

/// One core, followed through `events.subscribe`. collied never starts a core: an absent
/// socket means none runs.
pub struct Core {
    socket: PathBuf,
    claude_sessions: Option<PathBuf>,
    inner: Mutex<Inner>,
    settled: watch::Sender<bool>,
    herdr: AtomicU8,
    held: Mutex<HashMap<String, Weak<Hold>>>,
}

type Slot = Arc<Mutex<Option<Screen>>>;

/// A terminal kept attached while a phone watches it; the attach ends with the last watch.
struct Hold {
    screen: Slot,
    task: tokio::task::AbortHandle,
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub struct Watch {
    _hold: Arc<Hold>,
}

impl Core {
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            claude_sessions: None,
            inner: Mutex::default(),
            settled: watch::channel(false).0,
            herdr: AtomicU8::new(0),
            held: Mutex::default(),
        }
    }

    /// Claude Code's `sessions` folder, where each process keeps `<pid>.json`.
    pub fn with_claude_sessions(mut self, dir: Option<PathBuf>) -> Self {
        self.claude_sessions = dir;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        crate::lock(&self.inner)
    }

    pub fn following(&self) -> bool {
        self.lock().live.is_some()
    }

    /// herdr's hooks exit without `HERDR_PANE_ID`, so herdr-core agents get their session
    /// here: Claude Code's from the file its foreground process keeps, Codex's from the hook.
    pub fn agents(&self) -> Option<Vec<AgentInfo>> {
        let inner = self.lock();
        let live = inner.live.as_ref()?;
        Some(live.agents(|kind, address, pid| self.session(&inner, kind, address, pid)))
    }

    fn session(&self, inner: &Inner, kind: &str, address: &str, pid: u32) -> Option<String> {
        match kind {
            "claude" => crate::transcript::claude_session(self.claude_sessions.as_deref()?, pid),
            "codex" => inner
                .codex
                .get(address)
                .filter(|(bound, _)| *bound == pid)
                .map(|(_, session)| session.clone()),
            _ => None,
        }
    }

    /// A Codex session reported by collied's SessionStart hook from `reporter` (the hook's
    /// pid, from the control socket) for the terminal named by its `HC_TERMINAL_ID`. Kept
    /// only when the hook runs under the Codex process that leads that terminal.
    pub fn bind(&self, address: &str, session: &str, reporter: u32) -> bool {
        if !local(address) || !crate::transcript::is_session_id(session) {
            return false;
        }
        let mut inner = self.lock();
        let Some(live) = inner.live.as_ref() else {
            return false;
        };
        let Some(pid) = live
            .terminals
            .get(address)
            .and_then(|h| h.terminal.foreground.as_ref()?.pid)
        else {
            return false;
        };
        if !reported_by(pid, reporter, process) {
            return false;
        }
        let held: Vec<String> = live.terminals.keys().cloned().collect();
        inner.codex.retain(|a, _| held.contains(a));
        inner
            .codex
            .insert(address.to_owned(), (pid, session.to_owned()));
        true
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

    /// The address and size of a terminal collied lists; nothing else is ever reached.
    fn terminal(&self, id: &str) -> Result<(String, u16, u16), Error> {
        let inner = self.lock();
        inner
            .live
            .as_ref()
            .and_then(|live| {
                let address = id.strip_prefix(live.group.as_str())?.strip_prefix(':')?;
                let t = &live.terminals.get(address)?.terminal;
                Some((address.to_owned(), t.cols, t.rows))
            })
            .ok_or_else(not_found)
    }

    /// The agent as the core has it right now, for the ready check before a write.
    pub async fn get(&self, id: &str) -> Result<AgentInfo, Error> {
        let (address, ..) = self.terminal(id)?;
        let mut conn = Conn::open(&self.socket).await?;
        let t: Terminal = conn
            .call("terminal.get", json!({ "terminal": address }), TIMEOUT)
            .await?;
        if t.address != address {
            return Err(not_found());
        }
        let inner = self.lock();
        let live = inner.live.as_ref().ok_or(Error::Closed)?;
        let seq = live.terminals.get(&address).map_or(live.seq, |h| h.seq);
        agent(
            &live.group,
            &Held { terminal: t, seq },
            &|kind, address, pid| self.session(&inner, kind, address, pid),
        )
        .ok_or_else(not_found)
    }

    /// A blocked report's kind (`permission`, `question`) stands in for herdr's rule id.
    pub fn explain(&self, id: &str) -> Result<Explain, Error> {
        let (address, ..) = self.terminal(id)?;
        let inner = self.lock();
        let kind = inner
            .live
            .as_ref()
            .and_then(|live| live.terminals.get(&address)?.terminal.status.clone())
            .filter(|s| s.state == "blocked")
            .and_then(|s| s.kind);
        Ok(Explain {
            matched_rule: kind.map(|id| MatchedRule { id }),
        })
    }

    /// The screen as text, without an attach: what herdr's `detection` source gives.
    pub async fn screen_text(&self, id: &str) -> Result<String, Error> {
        #[derive(Deserialize)]
        struct Read {
            text: String,
        }
        let (address, ..) = self.terminal(id)?;
        let mut conn = Conn::open(&self.socket).await?;
        let read: Read = conn
            .call("terminal.read", json!({ "terminal": address }), TIMEOUT)
            .await?;
        Ok(read.text)
    }

    /// Pasted, then Enter. `seen` is the status collied's ready check saw: `if` takes only
    /// `idle` or `working`, so a `done` or `error` agent is sent to without it and only the
    /// core's own refusal of `blocked` applies after the check. `force` types into a blocked
    /// agent, for an answer to its dialog.
    pub async fn send(&self, id: &str, text: &str, seen: &str, force: bool) -> Result<(), Error> {
        let (address, ..) = self.terminal(id)?;
        let mut params = json!({ "terminal": address, "text": text });
        if force {
            params["force"] = json!(true);
        } else if matches!(seen, "idle" | "working") {
            params["if"] = json!(seen);
        }
        let mut conn = Conn::open(&self.socket).await?;
        conn.call::<Value>("terminal.send", params, INPUT_TIMEOUT)
            .await?;
        Ok(())
    }

    /// Named keys, which the core encodes for the program's keyboard modes.
    pub async fn keys(&self, id: &str, keys: &[&str]) -> Result<(), Error> {
        let (address, ..) = self.terminal(id)?;
        let mut conn = Conn::open(&self.socket).await?;
        conn.call::<Value>(
            "terminal.keys",
            json!({ "terminal": address, "keys": keys }),
            INPUT_TIMEOUT,
        )
        .await?;
        Ok(())
    }

    /// Raw input without Enter, which no RPC takes, through a typing attach closed after the
    /// write. A typing attach sizes the terminal to what it asks, so it asks for the size the
    /// terminal has, which a desktop client may have chosen.
    pub async fn type_raw(&self, id: &str, text: &str) -> Result<(), Error> {
        let (address, cols, rows) = self.terminal(id)?;
        let (mut attach, _) = Attach::open(&self.socket, &address, cols, rows, 0, false).await?;
        let channel = attach.channel;
        let sent = attach.send(channel, DATA, text.as_bytes()).await;
        attach.close().await;
        sent
    }

    fn size(&self, address: &str) -> Option<(u16, u16)> {
        let inner = self.lock();
        let t = &inner.live.as_ref()?.terminals.get(address)?.terminal;
        Some((t.cols, t.rows))
    }

    pub fn columns(&self, id: &str) -> Result<Option<u16>, Error> {
        Ok(Some(self.terminal(id)?.1))
    }

    /// From the watched screen while a watch holds one, else through an attach of its own
    /// that ends with the read.
    pub async fn read(
        &self,
        id: &str,
        source: &str,
        lines: Option<u32>,
    ) -> Result<PaneRead, Error> {
        let (address, cols, rows) = self.terminal(id)?;
        let slot = crate::lock(&self.held)
            .get(&address)
            .and_then(Weak::upgrade)
            .map(|h| h.screen.clone());
        if let Some(text) =
            slot.and_then(|s| crate::lock(&s).as_mut().map(|s| s.read(source, lines)))
        {
            return Ok(PaneRead {
                text,
                truncated: false,
            });
        }
        let history = match source {
            "visible" => 0,
            _ => lines.unwrap_or(HISTORY).min(HISTORY),
        };
        let (mut attach, mut screen) =
            Attach::open(&self.socket, &address, cols, rows, history, true).await?;
        let painted = attach.paint(&mut screen).await;
        attach.close().await;
        painted?;
        Ok(PaneRead {
            text: screen.read(source, lines),
            truncated: false,
        })
    }

    pub fn watch(self: &Arc<Self>, id: &str) -> Result<Watch, Error> {
        let (address, ..) = self.terminal(id)?;
        let mut held = crate::lock(&self.held);
        held.retain(|_, h| h.strong_count() > 0);
        if let Some(h) = held.get(&address).and_then(Weak::upgrade) {
            return Ok(Watch { _hold: h });
        }
        let screen = Slot::default();
        let task = tokio::spawn(self.clone().stream(address.clone(), screen.clone()));
        let h = Arc::new(Hold {
            screen,
            task: task.abort_handle(),
        });
        held.insert(address, Arc::downgrade(&h));
        Ok(Watch { _hold: h })
    }

    /// Keeps `slot` on the terminal's screen until aborted with the last watch, attaching
    /// again when its size changes or the stream ends.
    async fn stream(self: Arc<Self>, address: String, slot: Slot) {
        loop {
            let attached = async {
                let (cols, rows) = self.size(&address)?;
                let (mut attach, mut screen) =
                    Attach::open(&self.socket, &address, cols, rows, HISTORY, true)
                        .await
                        .ok()?;
                attach.paint(&mut screen).await.ok()?;
                *crate::lock(&slot) = Some(screen);
                Some(attach)
            }
            .await;
            let Some(mut attach) = attached else {
                tokio::time::sleep(RETRY_FIRST).await;
                continue;
            };
            let mut check = tokio::time::interval(SIZE_CHECK);
            let resized = loop {
                tokio::select! {
                    next = attach.next() => match next {
                        Ok(Some(bytes)) => {
                            if let Some(s) = crate::lock(&slot).as_mut() {
                                s.feed(&bytes);
                            }
                            if attach.credit(bytes.len()).await.is_err() {
                                break false;
                            }
                        }
                        Ok(None) | Err(_) => break false,
                    },
                    _ = check.tick() => {
                        if self.size(&address) != Some(attach.size) {
                            break true;
                        }
                    }
                }
            };
            *crate::lock(&slot) = None;
            // A finished terminal stays listed until it is removed, and answers each attach
            // with a paint and a close.
            if !resized {
                tokio::time::sleep(RETRY_FIRST).await;
            }
        }
    }

    /// A task agent in a terminal of its own, with no parent to notify; returns its id. The
    /// terminal identity goes in `env`, as the core asks its clients to pass it.
    pub async fn start(
        &self,
        cwd: &str,
        name: &str,
        kind: &str,
        worktree: Option<&str>,
        cli: &Path,
        args: &[&str],
    ) -> Result<String, Error> {
        #[derive(Deserialize)]
        struct Run {
            address: String,
        }
        let group = (self.lock().live.as_ref())
            .map(|live| live.group.clone())
            .ok_or(Error::Closed)?;
        let argv = task_argv(cli, args)
            .ok_or_else(|| Error::Unsupported("the agent's path is not UTF-8".to_owned()))?;
        let mut labels = serde_json::Map::new();
        labels.insert(TASK_LABEL.to_owned(), json!(kind));
        if let Some(checkout) = worktree {
            labels.insert(WORKTREE_LABEL.to_owned(), json!(checkout));
        }
        let mut conn = Conn::open(&self.socket).await?;
        let run: Run = conn
            .call(
                "terminal.run",
                json!({
                    "argv": argv, "cwd": cwd, "name": name, "labels": labels,
                    "env": {"TERM": "xterm-256color", "COLORTERM": "truecolor"},
                    "no_parent": true, "no_notify": true,
                }),
                TIMEOUT,
            )
            .await?;
        if !local(&run.address) {
            return Err(Error::Unsupported(format!(
                "herdr-core started the agent as {:?}, on another machine; it was left running",
                run.address
            )));
        }
        Ok(format!("{group}:{}", run.address))
    }

    /// The checkout collied created or opened for the worktree task it started in `id`.
    pub fn worktree(&self, id: &str) -> Option<String> {
        let (address, ..) = self.terminal(id).ok()?;
        let inner = self.lock();
        let t = &inner.live.as_ref()?.terminals.get(&address)?.terminal;
        t.labels
            .contains_key(TASK_LABEL)
            .then(|| t.labels.get(WORKTREE_LABEL).cloned())
            .flatten()
    }

    /// Whether `id` held a terminal that has finished: its program exited.
    pub fn finished(&self, id: &str) -> bool {
        self.terminal(id).is_ok_and(|(address, ..)| {
            let inner = self.lock();
            inner.live.as_ref().is_some_and(|live| {
                live.terminals
                    .get(&address)
                    .is_some_and(|h| h.terminal.state == "finished")
            })
        })
    }

    /// Every live terminal on this machine, listed as an agent or not, shells included;
    /// `None` while the core is not followed, when nothing is known about them.
    pub fn occupants(&self) -> Option<Vec<Occupant>> {
        let inner = self.lock();
        let live = inner.live.as_ref()?;
        Some(
            (live.terminals.values())
                .map(|h| &h.terminal)
                .filter(|t| t.state == "live")
                .map(|t| Occupant {
                    id: format!("{}:{}", live.group, t.address),
                    task: t.labels.contains_key(TASK_LABEL),
                    worktree: (t.labels.get(WORKTREE_LABEL).cloned())
                        .filter(|_| t.labels.contains_key(TASK_LABEL)),
                    cwd: t.cwd.clone(),
                    foreground_cwd: t.foreground.as_ref().map(|f| f.cwd.clone()),
                })
                .collect(),
        )
    }

    /// Only a terminal collied lists.
    pub async fn close(&self, id: &str) -> Result<(), Error> {
        let (address, ..) = self.terminal(id)?;
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

/// The hook runs under the Codex process that started the session, which must lead the
/// terminal. Codex without `--no-daemon` runs it under its shared app-server, a second
/// `codex` whose environment names whichever terminal started it, so that report is refused.
fn reported_by(
    foreground: u32,
    reporter: u32,
    process: impl Fn(u32) -> Option<(u32, String)>,
) -> bool {
    let mut pid = reporter;
    let mut codex = 0;
    for _ in 0..16 {
        let Some((parent, name)) = process(pid) else {
            return false;
        };
        codex += usize::from(name == "codex");
        if pid == foreground {
            return codex == 1;
        }
        if parent <= 1 {
            return false;
        }
        pid = parent;
    }
    false
}

/// A process's parent and short name.
#[cfg(target_os = "macos")]
fn process(pid: u32) -> Option<(u32, String)> {
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    // SAFETY: `info` is a writable `proc_bsdinfo` of `size` bytes, what PROC_PIDTBSDINFO fills.
    let n = unsafe {
        libc::proc_pidinfo(
            i32::try_from(pid).ok()?,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            i32::try_from(size).ok()?,
        )
    };
    if usize::try_from(n).ok()? != size {
        return None;
    }
    // SAFETY: zeroed, then filled whole by the call above.
    let info = unsafe { info.assume_init() };
    let comm: Vec<u8> = info
        .pbi_comm
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    Some((info.pbi_ppid, String::from_utf8_lossy(&comm).into_owned()))
}

#[cfg(target_os = "linux")]
fn process(pid: u32) -> Option<(u32, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (head, rest) = stat.rsplit_once(')')?;
    let name = head.split_once('(')?.1.to_owned();
    let parent = rest.split_whitespace().nth(1)?.parse().ok()?;
    Some((parent, name))
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

async fn connect(socket: &Path) -> Result<UnixStream, Error> {
    let stream = tokio::time::timeout(TIMEOUT, UnixStream::connect(socket))
        .await
        .map_err(|_| Error::Timeout)??;
    // The socket is 0755: its folder alone keeps other users from serving one there.
    if stream.peer_cred()?.uid() != rustix::process::geteuid().as_raw() {
        return Err(Error::Unsupported("is served by another user".to_owned()));
    }
    Ok(stream)
}

fn request(id: u64, method: &str, params: Value) -> Result<Vec<u8>, Error> {
    if !METHODS.contains(&method) {
        return Err(Error::Unsupported(format!(
            "{method} is not a method collied calls"
        )));
    }
    tracing::debug!(method, id, "herdr-core request");
    Ok(serde_json::to_vec(
        &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
    )?)
}

fn answer<R: DeserializeOwned>(reply: Reply) -> Result<R, Error> {
    if let Some(e) = reply.error {
        return Err(Error::Core {
            kind: e.data["kind"].as_str().unwrap_or("error").to_owned(),
            message: e.message,
        });
    }
    Ok(serde_json::from_value(reply.result.ok_or(Error::Empty)?)?)
}

impl Conn {
    async fn open(socket: &Path) -> Result<Self, Error> {
        let (reader, writer) = connect(socket).await?.into_split();
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
        self.next += 1;
        let id = self.next;
        let mut line = request(id, method, params)?;
        line.push(b'\n');
        tokio::time::timeout(timeout, async {
            self.writer.write_all(&line).await?;
            loop {
                let reply: Reply = serde_json::from_slice(&self.line().await?)?;
                if reply.id == json!(id) {
                    return answer(reply);
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}

/// An attach on a framed connection of its own. A watch-only one (`observe`) never types or
/// sizes the terminal; a typing one sizes it to what it asks. Reads keep partial frames in
/// `buf`, so a read dropped in a `select!` loses nothing.
struct Attach {
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    buf: Vec<u8>,
    channel: u32,
    /// The terminal's size when attached, as the core answered it.
    size: (u16, u16),
}

#[derive(Deserialize)]
struct Attached {
    channel: u32,
    terminal: Size,
}

#[derive(Deserialize)]
struct Size {
    cols: u16,
    rows: u16,
}

impl Attach {
    async fn open(
        socket: &Path,
        address: &str,
        cols: u16,
        rows: u16,
        history: u32,
        observe: bool,
    ) -> Result<(Self, Screen), Error> {
        let (reader, writer) = connect(socket).await?.into_split();
        let mut attach = Self {
            reader,
            writer,
            buf: Vec::new(),
            channel: 0,
            size: (0, 0),
        };
        let attached: Attached = tokio::time::timeout(TIMEOUT, async {
            let mut hello = request(0, "hello", json!({ "generation": GENERATION, "framed": true }))?;
            hello.push(b'\n');
            attach.writer.write_all(&hello).await?;
            answer::<Value>(serde_json::from_slice(&attach.line().await?)?)?;
            let params = json!({
                "terminal": address, "cols": cols, "rows": rows, "observe": observe, "history": history,
            });
            attach.send(0, DATA, &request(1, "terminal.attach", params)?).await?;
            loop {
                let (channel, kind, payload) = attach.frame().await?;
                if channel == 0 && kind == DATA {
                    let reply: Reply = serde_json::from_slice(&payload)?;
                    if reply.id == json!(1) {
                        return answer(reply);
                    }
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout)??;
        attach.channel = attached.channel;
        let t = attached.terminal;
        attach.size = (t.cols, t.rows);
        Ok((attach, Screen::new(t.cols, t.rows, history as usize)))
    }

    async fn fill(&mut self) -> Result<(), Error> {
        self.buf.reserve(64 << 10);
        if self.reader.read_buf(&mut self.buf).await? == 0 {
            return Err(Error::Closed);
        }
        Ok(())
    }

    async fn line(&mut self) -> Result<Vec<u8>, Error> {
        loop {
            if let Some(end) = self.buf.iter().position(|b| *b == b'\n') {
                return Ok(self.buf.drain(..=end).collect());
            }
            if self.buf.len() as u64 > MAX_LINE {
                return Err(Error::LineTooLong);
            }
            self.fill().await?;
        }
    }

    /// `[channel u32 LE][kind u8][len u32 LE][payload]`.
    async fn frame(&mut self) -> Result<(u32, u8, Vec<u8>), Error> {
        loop {
            if let Some(header) = self.buf.get(..9) {
                let len = u32::from_le_bytes([header[5], header[6], header[7], header[8]]) as usize;
                if len > MAX_FRAME {
                    return Err(Error::LineTooLong);
                }
                if self.buf.len() >= 9 + len {
                    let channel = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
                    let kind = header[4];
                    let payload = self.buf[9..9 + len].to_vec();
                    self.buf.drain(..9 + len);
                    return Ok((channel, kind, payload));
                }
            }
            self.fill().await?;
        }
    }

    async fn send(&mut self, channel: u32, kind: u8, payload: &[u8]) -> Result<(), Error> {
        let mut frame = Vec::with_capacity(9 + payload.len());
        frame.extend_from_slice(&channel.to_le_bytes());
        frame.push(kind);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        self.writer.write_all(&frame).await?;
        Ok(())
    }

    /// The terminal's next output, or `None` once it ended. The core sends no more than its
    /// credit: each read is given back with [`Self::credit`].
    async fn next(&mut self) -> Result<Option<Vec<u8>>, Error> {
        loop {
            let (channel, kind, payload) = self.frame().await?;
            if channel != self.channel {
                continue;
            }
            match kind {
                DATA => return Ok(Some(payload)),
                CLOSE => return Ok(None),
                _ => {}
            }
        }
    }

    async fn credit(&mut self, bytes: usize) -> Result<(), Error> {
        let credit = (bytes as u32).to_le_bytes();
        self.send(self.channel, CREDIT, &credit).await
    }

    /// The paint follows the attach answer at once; it is over when the stream pauses, or
    /// a second after it began for a terminal whose output never does.
    async fn paint(&mut self, screen: &mut Screen) -> Result<(), Error> {
        let mut deadline = None;
        loop {
            let wait = deadline.map_or(TIMEOUT, |at: tokio::time::Instant| {
                PAINT_QUIET.min(at.saturating_duration_since(tokio::time::Instant::now()))
            });
            match tokio::time::timeout(wait, self.next()).await {
                Ok(Ok(Some(bytes))) => {
                    screen.feed(&bytes);
                    self.credit(bytes.len()).await?;
                    deadline.get_or_insert(tokio::time::Instant::now() + PAINT_MAX);
                }
                Ok(Ok(None)) | Err(_) if deadline.is_some() => return Ok(()),
                Ok(Ok(None)) => return Err(Error::Closed),
                Err(_) => return Err(Error::Timeout),
                Ok(Err(e)) => return Err(e),
            }
        }
    }

    async fn close(mut self) {
        let channel = self.channel;
        let _ = self.send(channel, CLOSE, &[]).await;
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
        let lacking = hello(1, "0.1.16", &METHODS[..7]);
        assert_eq!(
            lacking.missing(),
            [
                "terminal.read",
                "terminal.send",
                "terminal.keys",
                "terminal.run"
            ]
        );
        assert_eq!(
            lacking.problem().as_deref(),
            Some("lacks terminal.read, terminal.send, terminal.keys, terminal.run")
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

    fn none(_: &str, _: &str, _: u32) -> Option<String> {
        None
    }

    fn shown(live: &Live) -> Vec<(String, String, Option<String>, u64)> {
        live.agents(none)
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
        let a = &l.agents(none)[0];
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
        assert_eq!(
            l.agents(none)[0].title.as_deref(),
            Some("Fix the flaky test")
        );

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
        assert!(l.agents(none).is_empty());
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
        assert!(l.agents(none).is_empty());
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
        assert!(l.agents(none).is_empty() && l.terminals.len() == 1);
    }

    #[test]
    fn a_terminal_a_shell_leads_is_never_an_agent() {
        // herdr-core 0.1.16 keeps a status after its program exits, and while it is
        // suspended (Ctrl+Z), with the shell leading the terminal again.
        let codex = terminal("t1", json!({"state": "idle", "app": "codex"}), "codex");
        let mut l = live(10, json!([codex]));
        assert_eq!(shown(&l)[0].1, "idle");
        let fg = |pid: u32, argv0: &str| json!({"pid": pid, "argv": [argv0, "-i"], "cwd": "/w"});
        let shells = ["-zsh", "/bin/bash", "sh", "/opt/homebrew/bin/fish", "nu"];
        for (seq, shell) in (11..).zip(shells) {
            event(&mut l, seq, "t1", "foreground", fg(3, shell));
            assert!(l.agents(none).is_empty(), "{shell}");
        }
        event(&mut l, 20, "t1", "foreground", fg(9, "codex"));
        assert_eq!(shown(&l)[0].1, "idle", "back in the foreground");

        // A fresh follow (collied restarted, the core reconnected) sees only the snapshot.
        let mut stale = terminal("t1", json!({"state": "idle", "app": "codex"}), "codex");
        stale["foreground"] = fg(3, "-zsh");
        assert!(live(20, json!([stale])).agents(none).is_empty());
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
            l.agents(none).len(),
            2,
            "events up to the snapshot's seq are ignored"
        );
        l.notice(&line(
            json!({"jsonrpc": "2.0", "method": "missed", "params": {
                "seq": 50, "terminals": [],
            }}),
        ));
        assert!(l.agents(none).is_empty());
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

    #[derive(Default, Clone)]
    struct Seen {
        /// Each attach's params.
        attaches: Vec<Value>,
        /// The kinds of the frames each client wrote after its attach.
        kinds: Vec<u8>,
        /// What clients typed on their attaches.
        typed: Vec<u8>,
        /// Attaches that ended.
        closed: usize,
        /// Line-mode calls, as `{method, params}`.
        calls: Vec<Value>,
    }

    /// A core that answers every attach with a one-line paint, and every line-mode call with
    /// `answer` (else an error), recording what clients asked.
    async fn fake_core(socket: &Path, answer: fn(&str) -> Value) -> Arc<Mutex<Seen>> {
        let listener = tokio::net::UnixListener::bind(socket).unwrap();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let record = seen.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let record = record.clone();
                tokio::spawn(async move {
                    let (reader, mut writer) = stream.into_split();
                    let mut a = Attach {
                        reader,
                        writer: tokio::net::UnixStream::pair().unwrap().0.into_split().1,
                        buf: Vec::new(),
                        channel: 0,
                        size: (0, 0),
                    };
                    let mut line = a.line().await.unwrap();
                    loop {
                        let request: Value = serde_json::from_slice(&line).unwrap();
                        if request["method"] == "hello" {
                            break;
                        }
                        let result = answer(request["method"].as_str().unwrap());
                        crate::lock(&record).calls.push(
                            json!({"method": request["method"], "params": request["params"]}),
                        );
                        let reply = if result.is_null() {
                            json!({"jsonrpc": "2.0", "id": request["id"], "error": {
                                "code": -32000, "message": "refused", "data": {"kind": "status_mismatch"},
                            }})
                        } else {
                            json!({"jsonrpc": "2.0", "id": request["id"], "result": result})
                        };
                        let mut out = serde_json::to_vec(&reply).unwrap();
                        out.push(b'\n');
                        writer.write_all(&out).await.unwrap();
                        match a.line().await {
                            Ok(next) => line = next,
                            Err(_) => return,
                        }
                    }
                    writer
                        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"framed\":true}}\n")
                        .await
                        .unwrap();
                    let (_, _, request) = a.frame().await.unwrap();
                    let request: Value = serde_json::from_slice(&request).unwrap();
                    crate::lock(&record)
                        .attaches
                        .push(request["params"].clone());
                    let frame = |channel: u32, kind: u8, payload: &[u8]| {
                        let mut f = channel.to_le_bytes().to_vec();
                        f.push(kind);
                        f.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                        f.extend_from_slice(payload);
                        f
                    };
                    let answer = json!({"jsonrpc": "2.0", "id": 1, "result": {
                        "channel": 7, "terminal": {"cols": 80, "rows": 24},
                    }});
                    writer
                        .write_all(&frame(0, DATA, &serde_json::to_vec(&answer).unwrap()))
                        .await
                        .unwrap();
                    writer.write_all(&frame(7, DATA, b"ready")).await.unwrap();
                    while let Ok((_, kind, payload)) = a.frame().await {
                        let mut r = crate::lock(&record);
                        r.kinds.push(kind);
                        if kind == DATA {
                            r.typed.extend_from_slice(&payload);
                        }
                    }
                    crate::lock(&record).closed += 1;
                });
            }
        });
        seen
    }

    #[tokio::test]
    async fn attaches_only_watch_and_never_type_or_resize() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("core.sock");
        let seen = fake_core(&socket, |_| Value::Null).await;
        let core = followed(socket);
        let id = format!("hc:{ID}:t1");
        let read = core.read(&id, "recent_unwrapped", Some(50)).await.unwrap();
        assert_eq!(read.text, "ready");
        let watch = core.watch(&id).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(watch);
        for _ in 0..100 {
            if crate::lock(&seen).closed == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let Seen {
            attaches: params,
            kinds,
            closed,
            ..
        } = crate::lock(&seen).clone();
        assert_eq!(closed, 2, "the read and the watch both detached");
        assert_eq!(params.len(), 2);
        for p in &params {
            assert_eq!(p["observe"], json!(true), "{p}");
            assert_eq!(p["terminal"], json!("t1"));
        }
        assert_eq!(
            (params[0]["history"].clone(), params[1]["history"].clone()),
            (json!(50), json!(HISTORY))
        );
        assert!(
            kinds.contains(&CREDIT) && kinds.contains(&CLOSE),
            "{kinds:?}"
        );
        assert!(
            kinds.iter().all(|k| matches!(*k, CREDIT | CLOSE)),
            "only credit and close follow an attach: {kinds:?}"
        );
    }

    #[tokio::test]
    async fn input_goes_through_send_keys_and_a_same_size_typing_attach() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("core.sock");
        let seen = fake_core(&socket, |method| match method {
            "terminal.get" => {
                let mut t = terminal("t1", json!({"state": "idle", "app": "codex"}), "codex");
                t["turn"] = json!(3);
                t
            }
            "terminal.read" => json!({"text": "screen", "complete": true, "how": "screen"}),
            "terminal.send" | "terminal.keys" => json!({"turn": 3, "status": "idle"}),
            _ => Value::Null,
        })
        .await;
        let core = followed(socket);
        crate::lock(&core.inner)
            .live
            .as_mut()
            .unwrap()
            .terminals
            .get_mut("t1")
            .unwrap()
            .terminal
            .cols = 70;
        let id = format!("hc:{ID}:t1");
        let rt = Runtime::new(dir.path().join("herdr.sock")).with_core(Some(core.clone()));

        let a = rt.agent_get(&id).await.unwrap();
        assert_eq!(
            (
                a.terminal_id.as_str(),
                a.agent_status.as_str(),
                a.state_change_seq,
                a.read_only
            ),
            (id.as_str(), "idle", 10, false)
        );
        assert_eq!(rt.detection_text(&id).await.unwrap(), "screen");
        for seen in ["idle", "working", "done", "unknown"] {
            rt.agent_prompt(&id, "hi", seen).await.unwrap();
        }
        rt.type_text(&id, "yes", "blocked").await.unwrap();
        rt.agent_send_keys(&id, &["down", "enter"]).await.unwrap();
        rt.scroll_bottom(&id).await.unwrap();
        rt.notice_digit(&id, "2").await.unwrap();
        rt.pane_send_text(&id, "/model").await.unwrap();
        assert!(matches!(
            rt.agent_get(&format!("hc:{ID}:t9")).await,
            Err(Error::Core { kind, .. }) if kind == "not_found"
        ));
        for _ in 0..100 {
            if crate::lock(&seen).closed == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let seen = crate::lock(&seen).clone();
        assert_eq!(seen.closed, 1, "the typing attach closed");
        let calls: Vec<String> = seen.calls.iter().map(|c| c.to_string()).collect();
        let t = |rest: &str| format!(r#"{{"method":"terminal.{rest}}}"#);
        assert_eq!(
            calls,
            [
                t(r#"get","params":{"terminal":"t1"}"#),
                t(r#"read","params":{"terminal":"t1"}"#),
                t(r#"send","params":{"if":"idle","terminal":"t1","text":"hi"}"#),
                t(r#"send","params":{"if":"working","terminal":"t1","text":"hi"}"#),
                t(r#"send","params":{"terminal":"t1","text":"hi"}"#),
                t(r#"send","params":{"terminal":"t1","text":"hi"}"#),
                t(r#"send","params":{"force":true,"terminal":"t1","text":"yes"}"#),
                t(r#"keys","params":{"keys":["down","enter"],"terminal":"t1"}"#),
                t(r#"keys","params":{"keys":["ctrl+end"],"terminal":"t1"}"#),
                t(r#"keys","params":{"keys":["2"],"terminal":"t1"}"#),
            ]
        );
        assert_eq!(seen.attaches.len(), 1);
        let p = &seen.attaches[0];
        assert_eq!(
            (&p["observe"], &p["cols"], &p["rows"]),
            (&json!(false), &json!(70), &json!(24)),
            "a typing attach asks for the size the terminal has"
        );
        assert_eq!(seen.typed, b"/model");
        assert_eq!(seen.kinds, [DATA, CLOSE]);

        crate::lock(&core.inner)
            .live
            .as_mut()
            .unwrap()
            .terminals
            .get_mut("t1")
            .unwrap()
            .terminal
            .status = Some(Status {
            state: "blocked".into(),
            app: Some("claude-code".into()),
            kind: Some("permission".into()),
        });
        let rule = rt
            .agent_explain(&id)
            .await
            .unwrap()
            .matched_rule
            .map(|r| r.id);
        assert_eq!(rule.as_deref(), Some("permission"));
    }

    const SESSION: &str = "01a199ed-7993-7261-8219-300d543d764b";

    #[test]
    fn sessions_come_from_claude_codes_file_and_codexs_hook() {
        let dir = tempfile::tempdir().unwrap();
        let pid = std::process::id();
        let mut claude = terminal(
            "t1",
            json!({"state": "idle", "app": "claude-code"}),
            "claude",
        );
        claude["foreground"]["pid"] = json!(pid);
        let mut codex = terminal("t2", json!({"state": "idle", "app": "codex"}), "codex");
        codex["foreground"]["pid"] = json!(pid);
        let core = Core::new(dir.path().join("core.sock"))
            .with_claude_sessions(Some(dir.path().to_owned()));
        core.lock().live = Some(live(10, json!([claude, codex])));
        let sessions = |core: &Core| -> Vec<Option<String>> {
            core.agents()
                .unwrap()
                .into_iter()
                .map(|a| a.agent_session.map(|s| s.value))
                .collect()
        };
        assert_eq!(sessions(&core), [None, None]);

        let started = crate::now_ms();
        std::fs::write(
            dir.path().join(format!("{pid}.json")),
            json!({"pid": pid, "sessionId": SESSION, "cwd": "/w", "startedAt": started})
                .to_string(),
        )
        .unwrap();
        core.lock()
            .codex
            .insert("t2".into(), (pid + 1, SESSION.into()));
        assert_eq!(
            sessions(&core),
            [Some(SESSION.into()), None],
            "a report made under another foreground process does not hold"
        );
        core.lock().codex.insert("t2".into(), (pid, SESSION.into()));
        assert_eq!(
            sessions(&core),
            [Some(SESSION.into()), Some(SESSION.into())]
        );

        assert!(
            !core.bind("t2", SESSION, pid),
            "the test is not run by Codex"
        );
        assert!(!core.bind("t9", SESSION, pid));
        assert!(!core.bind("box/t2", SESSION, pid));
        assert!(!core.bind("t2", "../x", pid));
    }

    #[test]
    fn a_session_report_holds_only_from_the_codex_that_leads_the_terminal() {
        let table = |rows: &'static [(u32, u32, &'static str)]| {
            move |pid: u32| {
                rows.iter()
                    .find(|r| r.0 == pid)
                    .map(|r| (r.1, r.2.to_owned()))
            }
        };
        // collied hook, under sh, under the terminal's Codex (`--no-daemon`).
        let own = table(&[
            (30, 20, "collied"),
            (20, 10, "sh"),
            (10, 5, "codex"),
            (5, 1, "hc"),
        ]);
        assert!(reported_by(10, 30, own));
        // Under the shared app-server that terminal 10's Codex started.
        let shared = table(&[
            (30, 25, "collied"),
            (25, 10, "codex"),
            (10, 5, "codex"),
            (5, 1, "hc"),
        ]);
        assert!(!reported_by(10, 30, shared));
        // An npm install leads with node, which runs the native codex.
        let npm = table(&[
            (30, 15, "collied"),
            (15, 10, "codex"),
            (10, 5, "node"),
            (5, 1, "hc"),
        ]);
        assert!(reported_by(10, 30, npm));
        // From another terminal, or a vanished process.
        let other = table(&[(30, 40, "collied"), (40, 1, "codex")]);
        assert!(!reported_by(10, 30, other));
        assert!(!reported_by(10, 30, table(&[])));
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
    async fn archive_closes_a_herdr_core_agent() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("core.sock");
        let seen = fake_core(&socket, |method| match method {
            "terminal.kill" => json!({"address": "t1"}),
            "terminal.rm" => json!({"removed": "t1"}),
            _ => Value::Null,
        })
        .await;
        let drive = crate::drive::Driver::new(
            dir.path().join("herdr.sock"),
            vec![],
            &[dir.path().to_owned()],
        )
        .unwrap()
        .with_core(Some(followed(socket)));
        let yes: crate::drive::Authorized = Arc::new(|| true);
        let archive = |id: String| protocol::TaskArchiveParams {
            op_id: protocol::OpId::new("a".repeat(22)).unwrap(),
            terminal_id: protocol::TerminalId::new(id).unwrap(),
            confirm: true,
        };
        let Ok(protocol::Response::TaskArchived { message }) = drive
            .task_archive(archive(format!("hc:{ID}:t1")), &yes)
            .await
        else {
            panic!("not archived");
        };
        assert!(message.starts_with("Closed pane."), "{message}");
        let calls: Vec<Value> = crate::lock(&seen).calls.clone();
        assert_eq!(
            calls,
            [
                json!({"method": "terminal.kill", "params": {"terminal": "t1"}}),
                json!({"method": "terminal.rm", "params": {"terminal": "t1"}}),
            ]
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
    async fn a_task_agent_starts_without_the_starters_session_variables() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("core.sock");
        let seen = fake_core(&socket, |method| match method {
            "terminal.run" => json!({"address": "t2", "state": "live"}),
            _ => Value::Null,
        })
        .await;
        let core = followed(socket);
        let cli = Path::new("/opt/bin/codex");
        let args = super::super::herdr::agent_args("codex", Some(SESSION), false);
        assert_eq!(
            core.start("/w", "collie-0a1b2c3d", "codex", Some("/w"), cli, &args)
                .await
                .unwrap(),
            format!("hc:{ID}:t2")
        );
        let calls = crate::lock(&seen).calls.clone();
        let params = &calls[0]["params"];
        let argv: Vec<&str> = params["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        let mut expected = vec!["/usr/bin/env"];
        for name in STRIPPED {
            expected.extend(["-u", name]);
        }
        expected.extend(["/opt/bin/codex", "resume", SESSION, "--no-daemon"]);
        assert_eq!(argv, expected);
        assert_eq!(STRIPPED.len(), 16);
        assert_eq!(
            (
                &params["cwd"],
                &params["name"],
                &params["no_parent"],
                &params["no_notify"]
            ),
            (
                &json!("/w"),
                &json!("collie-0a1b2c3d"),
                &json!(true),
                &json!(true)
            )
        );
        assert_eq!(
            params["env"],
            json!({"TERM": "xterm-256color", "COLORTERM": "truecolor"})
        );
        assert_eq!(
            params["labels"],
            json!({"collie.kind": "codex", "collie.worktree": "/w"})
        );
        assert_eq!(
            super::super::herdr::agent_args("codex", None, false),
            ["--no-daemon"]
        );
        assert_eq!(
            super::super::herdr::agent_args("claude", Some(SESSION), true),
            ["--resume", SESSION, "--settings", r#"{"ultracode":true}"#]
        );
        assert!(super::cli("copilot").is_none());
    }

    /// A herdr that answers `ping`, `answer`'s methods, and an error for anything else.
    fn fake_herdr(socket: &Path, answer: fn(&str) -> Option<Value>) {
        let listener = tokio::net::UnixListener::bind(socket).unwrap();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (reader, mut writer) = stream.into_split();
                let mut line = String::new();
                BufReader::new(reader).read_line(&mut line).await.unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                let method = request["method"].as_str().unwrap();
                let reply = match (method, answer(method)) {
                    ("ping", _) => json!({"id": request["id"], "result": {
                        "type": "pong", "version": "0.9.5", "protocol": 1,
                    }}),
                    (_, Some(result)) => json!({"id": request["id"], "result": result}),
                    (_, None) => json!({"id": request["id"], "error": {
                        "code": "unknown_method", "message": "no",
                    }}),
                };
                writer
                    .write_all(format!("{reply}\n").as_bytes())
                    .await
                    .unwrap();
            }
        });
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    /// A repository with a linked worktree under a canonical root: the root, the main
    /// checkout and the linked one.
    fn worktree() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "start"]);
        let checkout = repo.join(".worktree/feat");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat",
                checkout.to_str().unwrap(),
            ],
        );
        (dir, root, repo, checkout)
    }

    /// An idle Codex started in `cwd` as t1, with `labels`.
    fn codex_in(cwd: &Path, labels: Value) -> Value {
        let mut t = terminal("t1", json!({"state": "idle", "app": "codex"}), "codex");
        t["cwd"] = json!(cwd);
        t["foreground"]["cwd"] = json!(cwd);
        t["labels"] = labels;
        t
    }

    /// The labels of a worktree task collied started in `checkout`.
    fn worktree_task(checkout: &Path) -> Value {
        json!({"collie.kind": "codex", "collie.worktree": checkout})
    }

    fn core_of(root: &Path, terminals: Value) -> Arc<Core> {
        let core = Core::new(root.join("core.sock"));
        core.lock().live = Some(live(10, terminals));
        Arc::new(core)
    }

    fn archive_params() -> protocol::TaskArchiveParams {
        protocol::TaskArchiveParams {
            op_id: protocol::OpId::new("a".repeat(22)).unwrap(),
            terminal_id: protocol::TerminalId::new(format!("hc:{ID}:t1")).unwrap(),
            confirm: true,
        }
    }

    async fn archive(root: &Path, core: Arc<Core>) -> crate::drive::Reply {
        archive_while(root, core, Arc::new(|| true)).await
    }

    async fn archive_while(
        root: &Path,
        core: Arc<Core>,
        auth: crate::drive::Authorized,
    ) -> crate::drive::Reply {
        let drive = crate::drive::Driver::new(root.join("herdr.sock"), vec![], &[root.to_owned()])
            .unwrap()
            .with_core(Some(core));
        drive.task_archive(archive_params(), &auth).await
    }

    fn kill_and_rm(method: &str) -> Value {
        match method {
            "terminal.kill" => json!({"address": "t1"}),
            "terminal.rm" => json!({"removed": "t1"}),
            _ => Value::Null,
        }
    }

    fn methods(seen: &Arc<Mutex<Seen>>) -> Vec<Value> {
        (crate::lock(seen).calls.iter())
            .map(|c| c["method"].clone())
            .collect()
    }

    #[tokio::test]
    async fn archive_closes_only_an_agent_collie_did_not_start_or_in_a_main_checkout() {
        // Not collied's; a plain task whose folder is a linked worktree; one in the main
        // checkout.
        for (labels, main) in [
            (json!({}), false),
            (json!({"collie.kind": "codex"}), false),
            (json!({"collie.kind": "codex"}), true),
        ] {
            let (_dir, root, repo, checkout) = worktree();
            let seen = fake_core(&root.join("core.sock"), kill_and_rm).await;
            let folder = if main { &repo } else { &checkout };
            let core = core_of(&root, json!([codex_in(folder, labels)]));
            let Ok(protocol::Response::TaskArchived { message }) = archive(&root, core).await
            else {
                panic!("not archived");
            };
            assert!(message.starts_with("Closed pane."), "{message}");
            assert!(checkout.exists(), "the checkout stays");
            assert_eq!(
                methods(&seen),
                [json!("terminal.kill"), json!("terminal.rm")]
            );
        }
    }

    #[tokio::test]
    async fn archive_leaves_a_worktree_anything_else_works_in() {
        let (_dir, root, repo, checkout) = worktree();
        let seen = fake_core(&root.join("core.sock"), kill_and_rm).await;
        // A shell opened at the repository, then moved into the checkout.
        let mut shell = terminal("t2", Value::Null, "-zsh");
        shell["cwd"] = json!(repo);
        shell["foreground"]["cwd"] = json!(checkout.join("src"));
        let core = core_of(
            &root,
            json!([codex_in(&checkout, worktree_task(&checkout)), shell]),
        );
        let (code, message) = archive(&root, core).await.unwrap_err();
        assert_eq!(code, protocol::ErrorCode::InvalidParams);
        assert!(message.contains("collie did not start"), "{message}");
        assert!(checkout.exists());
        assert!(methods(&seen).is_empty(), "no terminal was closed");
    }

    #[tokio::test]
    async fn archive_leaves_a_worktree_herdr_has_open_or_while_herdr_does_not_answer() {
        let (_dir, root, repo, checkout) = worktree();
        let seen = fake_core(&root.join("core.sock"), |_| Value::Null).await;
        let core = core_of(
            &root,
            json!([codex_in(&checkout, worktree_task(&checkout))]),
        );
        // A socket that takes the connection and closes it unanswered: herdr runs but is busy.
        let listener = tokio::net::UnixListener::bind(root.join("herdr.sock")).unwrap();
        let busy = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                drop(stream);
            }
        });
        let (_, message) = archive(&root, core.clone()).await.unwrap_err();
        assert!(message.contains("herdr did not answer"), "{message}");
        busy.abort();
        let _ = busy.await;
        std::fs::remove_file(root.join("herdr.sock")).unwrap();

        static LISTED: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
        LISTED
            .set((repo.display().to_string(), checkout.display().to_string()))
            .unwrap();
        fake_herdr(&root.join("herdr.sock"), |method| {
            let (repo, checkout) = LISTED.get().unwrap();
            match method {
                "session.snapshot" => Some(json!({"type": "session_snapshot",
                    "snapshot": {"workspaces": [], "panes": [], "agents": []}})),
                "worktree.list" => Some(
                    json!({"type": "worktree_list", "source": {"source_checkout_path": repo},
                    "worktrees": [{"path": checkout, "branch": "feat", "open_workspace_id": "w3",
                        "is_bare": false, "is_linked_worktree": true}]}),
                ),
                _ => None,
            }
        });
        let (code, message) = archive(&root, core).await.unwrap_err();
        assert_eq!(code, protocol::ErrorCode::InvalidParams);
        assert!(
            message.contains("herdr has this checkout open"),
            "{message}"
        );
        assert!(checkout.exists());
        assert!(methods(&seen).is_empty(), "no terminal was closed");
    }

    #[tokio::test]
    async fn archive_removes_a_worktree_task_checkout_after_ending_its_agents() {
        let (_dir, root, _, checkout) = worktree();
        let seen = fake_core(&root.join("core.sock"), kill_and_rm).await;
        let core = core_of(
            &root,
            json!([codex_in(&checkout, worktree_task(&checkout))]),
        );
        // Authorized while the checkout exists, so `gh poi` is skipped after the removal.
        let held = checkout.clone();
        let reply = archive_while(&root, core, Arc::new(move || held.exists())).await;
        let Ok(protocol::Response::TaskArchived { message }) = reply else {
            panic!("not archived: {reply:?}");
        };
        assert!(
            message.starts_with("Removed worktree checkout"),
            "{message}"
        );
        assert!(
            message.contains(&format!("and closed hc:{ID}:t1.")),
            "{message}"
        );
        assert!(message.contains("gh poi was skipped"), "{message}");
        assert!(!checkout.exists());
        assert_eq!(
            methods(&seen),
            [json!("terminal.kill"), json!("terminal.rm")]
        );
    }

    fn task_in(address: &str, cwd: &Path, labels: Value) -> Value {
        let mut t = codex_in(cwd, labels);
        t["address"] = json!(address);
        t
    }

    #[tokio::test]
    async fn archive_leaves_a_worktree_another_task_works_in() {
        let (_dir, root, _, checkout) = worktree();
        let seen = fake_core(&root.join("core.sock"), kill_and_rm).await;
        // A Resume started in the checkout: collied's, but not this worktree task's.
        let resume = task_in("t2", &checkout, json!({"collie.kind": "claude"}));
        let core = core_of(
            &root,
            json!([codex_in(&checkout, worktree_task(&checkout)), resume]),
        );
        let (_, message) = archive(&root, core).await.unwrap_err();
        assert!(
            message.contains("another task works in this checkout"),
            "{message}"
        );
        assert!(checkout.exists());
        assert!(methods(&seen).is_empty());
    }

    #[tokio::test]
    async fn archive_names_the_agents_it_closed_before_one_failed() {
        static KILLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let (_dir, root, _, checkout) = worktree();
        let seen = fake_core(&root.join("core.sock"), |method| match method {
            "terminal.kill" if KILLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 => {
                Value::Null
            }
            other => kill_and_rm(other),
        })
        .await;
        let both = json!([
            codex_in(&checkout, worktree_task(&checkout)),
            task_in("t2", &checkout, worktree_task(&checkout)),
        ]);
        let (_, message) = archive(&root, core_of(&root, both)).await.unwrap_err();
        assert!(
            message.contains(&format!("could not close agent hc:{ID}:t2")),
            "{message}"
        );
        assert!(
            message.contains(&format!("Closed hc:{ID}:t1 before that.")),
            "{message}"
        );
        assert!(checkout.exists(), "the checkout stays");
        assert_eq!(
            methods(&seen),
            [
                json!("terminal.kill"),
                json!("terminal.rm"),
                json!("terminal.kill")
            ]
        );
    }

    #[tokio::test]
    async fn archive_refuses_a_locked_worktree_or_a_checkout_outside_the_roots() {
        let (_dir, root, repo, checkout) = worktree();
        let seen = fake_core(&root.join("core.sock"), kill_and_rm).await;
        git(&repo, &["worktree", "lock", checkout.to_str().unwrap()]);
        let core = core_of(
            &root,
            json!([codex_in(&checkout, worktree_task(&checkout))]),
        );
        let (_, message) = archive(&root, core).await.unwrap_err();
        assert!(message.contains("locked; nothing was closed"), "{message}");
        assert!(checkout.exists());

        let outside = tempfile::tempdir().unwrap();
        let outside = std::fs::canonicalize(outside.path()).unwrap();
        let core = core_of(&root, json!([codex_in(&checkout, worktree_task(&outside))]));
        let (_, message) = archive(&root, core).await.unwrap_err();
        assert!(message.ends_with("; nothing was closed"), "{message}");
        assert!(outside.exists() && checkout.exists());
        assert!(methods(&seen).is_empty());
    }

    #[tokio::test]
    async fn archive_closes_only_when_git_no_longer_lists_the_recorded_checkout() {
        let (_dir, root, repo, checkout) = worktree();
        let seen = fake_core(&root.join("core.sock"), kill_and_rm).await;
        let plain = repo.join("plain");
        std::fs::create_dir(&plain).unwrap();
        let core = core_of(&root, json!([codex_in(&plain, worktree_task(&plain))]));
        let Ok(protocol::Response::TaskArchived { message }) = archive(&root, core).await else {
            panic!("not archived");
        };
        assert!(message.starts_with("Closed pane."), "{message}");
        assert!(plain.exists() && checkout.exists());
        assert_eq!(
            methods(&seen),
            [json!("terminal.kill"), json!("terminal.rm")]
        );
    }

    #[tokio::test]
    async fn archive_leaves_a_worktree_a_herdr_pane_works_in() {
        let (_dir, root, _, checkout) = worktree();
        let seen = fake_core(&root.join("core.sock"), kill_and_rm).await;
        let core = core_of(
            &root,
            json!([codex_in(&checkout, worktree_task(&checkout))]),
        );
        static CHECKOUT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        CHECKOUT.set(checkout.display().to_string()).unwrap();
        fake_herdr(&root.join("herdr.sock"), |method| {
            (method == "session.snapshot").then(|| {
                json!({"type": "session_snapshot", "snapshot": {"workspaces": [], "agents": [],
                    "panes": [{"pane_id": "w1:p1", "terminal_id": "term_1", "workspace_id": "w1",
                        "tab_id": "w1:t1", "cwd": "/", "foreground_cwd": CHECKOUT.get().unwrap()}]}})
            })
        });
        let (_, message) = archive(&root, core).await.unwrap_err();
        assert!(
            message.contains("a herdr pane works in this checkout"),
            "{message}"
        );
        assert!(checkout.exists());
        assert!(methods(&seen).is_empty());
    }

    #[tokio::test]
    async fn archive_closes_nothing_once_the_core_is_no_longer_followed() {
        let (_dir, root, repo, checkout) = worktree();
        let seen = fake_core(&root.join("core.sock"), kill_and_rm).await;
        let core = core_of(
            &root,
            json!([codex_in(&checkout, worktree_task(&checkout))]),
        );
        static FOLLOWED: std::sync::OnceLock<(Arc<Core>, String, String)> =
            std::sync::OnceLock::new();
        static SNAPSHOTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        assert!(
            FOLLOWED
                .set((
                    core.clone(),
                    repo.display().to_string(),
                    checkout.display().to_string(),
                ))
                .is_ok()
        );
        // The events connection drops while archive asks herdr: the core forgets its terminals.
        fake_herdr(&root.join("herdr.sock"), |method| {
            let (core, repo, checkout) = FOLLOWED.get().unwrap();
            match method {
                "session.snapshot" => {
                    if SNAPSHOTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                        core.lock().live = None;
                    }
                    Some(json!({"type": "session_snapshot",
                        "snapshot": {"workspaces": [], "panes": [], "agents": []}}))
                }
                "worktree.list" => Some(
                    json!({"type": "worktree_list", "source": {"source_checkout_path": repo},
                    "worktrees": [{"path": checkout, "branch": "feat", "open_workspace_id": null,
                        "is_bare": false, "is_linked_worktree": true}]}),
                ),
                _ => None,
            }
        });
        let (_, message) = archive(&root, core).await.unwrap_err();
        assert!(message.contains("not followed"), "{message}");
        assert!(message.contains("nothing was closed"), "{message}");
        assert!(checkout.exists());
        assert!(methods(&seen).is_empty());
    }

    #[tokio::test]
    async fn tasks_start_in_herdr_while_it_answers() {
        let dir = tempfile::tempdir().unwrap();
        let herdr = dir.path().join("herdr.sock");
        let rt =
            Runtime::new(herdr.clone()).with_core(Some(followed(dir.path().join("core.sock"))));
        assert!(
            rt.task_core().await.unwrap().is_some(),
            "herdr is not running"
        );
        let unfollowed = Runtime::new(herdr.clone())
            .with_core(Some(Arc::new(Core::new(dir.path().join("core.sock")))));
        assert!(unfollowed.task_core().await.unwrap().is_none());

        // herdr runs but takes the connection and closes it unanswered.
        let listener = tokio::net::UnixListener::bind(&herdr).unwrap();
        let busy = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                drop(stream);
            }
        });
        assert!(rt.task_core().await.is_err(), "herdr does not answer");
        busy.abort();
        let _ = busy.await;
        std::fs::remove_file(&herdr).unwrap();

        fake_herdr(&herdr, |_| None);
        assert!(rt.task_core().await.unwrap().is_none(), "herdr answers");
    }

    #[tokio::test]
    async fn a_task_herdr_core_cannot_start_creates_no_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let drive = crate::drive::Driver::new(
            root.join("herdr.sock"),
            vec![protocol::AgentKind::new("copilot").unwrap()],
            std::slice::from_ref(&root),
        )
        .unwrap()
        .with_core(Some(followed(root.join("core.sock"))));
        let yes: crate::drive::Authorized = Arc::new(|| true);
        let p = protocol::TaskNewParams {
            op_id: protocol::OpId::new("n".repeat(22)).unwrap(),
            cwd: protocol::Cwd::new(root.to_str().unwrap()).unwrap(),
            agent: protocol::AgentKind::new("copilot").unwrap(),
            prompt: Some(protocol::PromptText::new("hi".to_owned()).unwrap()),
            label: None,
            new_folder: Some(protocol::FolderName::new("made").unwrap()),
            worktree: None,
            clone: None,
            resume: None,
            ultracode: false,
        };
        let (reply, cwd) = drive.task_new(p, &yes).await;
        assert_eq!(reply.unwrap_err().0, protocol::ErrorCode::InvalidParams);
        assert!(cwd.is_none());
        assert!(!root.join("made").exists());
    }

    #[tokio::test]
    async fn herdr_core_ids_never_reach_herdr() {
        let dir = tempfile::tempdir().unwrap();
        let rt = Runtime::new(dir.path().join("herdr.sock"))
            .with_core(Some(followed(dir.path().join("core.sock"))));
        let id = format!("hc:{ID}:t1");
        assert!(matches!(
            rt.agent_focus(&id).await,
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            rt.pane_processes(&id).await,
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
