use std::collections::HashMap;
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::subagents::alive;
use super::{Kind, MAX_LISTED, Transcripts, is_file, is_session_id, open, project_dir, session_id};
use crate::herdr::{AgentInfo, PaneProcesses};

const SESSION_BYTES: u64 = 64 * 1024;
const POINTER_BYTES: u64 = 64 * 1024;
const MAX_HOPS: usize = 4;
const MAX_PIDS: usize = 16;
const MAX_SESSION_FILES: usize = 256;
/// Linux gives a process start to the second.
const START_SLACK_MS: u64 = 2_000;
const PANE_EVERY: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, PartialEq, Eq)]
struct DirectoryStamp {
    device: u64,
    inode: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

#[derive(Default)]
struct CopilotDirectories {
    pids: Vec<(i32, Option<u64>)>,
    empty: HashMap<String, DirectoryStamp>,
}

#[derive(Clone)]
pub(super) struct Pane {
    pane_id: String,
    at: Instant,
    pids: Vec<i32>,
    attach: Option<String>,
    truncated: bool,
    copilot: Arc<Mutex<CopilotDirectories>>,
}

/// Rebuilt when herdr's id or the pane's pids are refreshed; followed again when the
/// starting transcript's size or mtime moves.
pub(super) struct Resolved {
    herdr: Option<String>,
    pane_at: Option<Instant>,
    start: Option<String>,
    path: Option<PathBuf>,
    stamp: Option<(u64, SystemTime)>,
    id: Option<String>,
}

struct Claim {
    session: String,
    cwd: String,
    kind: Option<String>,
    job: Option<String>,
    name: Option<String>,
}

enum Pointer {
    Continued(String),
    Attached(String),
}

impl Transcripts {
    pub fn stale_panes(&self, agents: &[AgentInfo]) -> Vec<(String, String)> {
        agents
            .iter()
            .filter(|a| matches!(a.agent.as_deref(), Some("claude" | "copilot")))
            .filter(|a| {
                self.panes
                    .get(&a.terminal_id)
                    .is_none_or(|p| p.pane_id != a.pane_id || p.at.elapsed() >= PANE_EVERY)
            })
            .map(|a| (a.terminal_id.clone(), a.pane_id.clone()))
            .collect()
    }

    /// `None` when herdr could not list them: the processes last listed for this pane stand.
    pub fn set_pane(&mut self, terminal_id: String, pane_id: String, procs: Option<PaneProcesses>) {
        let kept = self
            .panes
            .remove(&terminal_id)
            .filter(|p| p.pane_id == pane_id)
            .map(|p| (p.pids, p.attach, p.truncated));
        let (pids, attach, truncated) = match procs {
            Some(procs) => {
                let truncated = procs.pids.len() > MAX_PIDS;
                (
                    procs
                        .pids
                        .into_iter()
                        .filter_map(|p| i32::try_from(p).ok())
                        .take(MAX_PIDS)
                        .collect(),
                    procs.attach,
                    truncated,
                )
            }
            None => kept.unwrap_or_default(),
        };
        self.panes.insert(
            terminal_id,
            Pane {
                pane_id,
                at: Instant::now(),
                pids,
                attach,
                truncated,
                copilot: Arc::default(),
            },
        );
    }

    /// For Claude Code, the session a process in the pane reports, else herdr's, then where
    /// that session's own transcript says the conversation went.
    pub(super) fn live_id(&self, a: &AgentInfo) -> Option<String> {
        if a.agent.as_deref() == Some("copilot") {
            return self.copilot_id(a);
        }
        if a.agent.as_deref() != Some("claude") {
            return session_id(a).map(str::to_owned);
        }
        let (start, path) = self.start(a, true);
        self.follow(start, path.as_deref())
    }

    /// `live_id`, cached per terminal.
    pub(super) fn resolve(&mut self, a: &AgentInfo, relocate: bool) -> Option<String> {
        if a.agent.as_deref() == Some("copilot") {
            return self.copilot_id(a);
        }
        let herdr = session_id(a).map(str::to_owned);
        if a.agent.as_deref() != Some("claude") {
            return herdr;
        }
        let pane_at = self
            .panes
            .get(&a.terminal_id)
            .filter(|p| p.pane_id == a.pane_id)
            .map(|p| p.at);
        let (mut r, fresh) = match self.resolved.remove(&a.terminal_id) {
            Some(r)
                if r.herdr == herdr && r.pane_at == pane_at && (r.path.is_some() || !relocate) =>
            {
                (r, false)
            }
            _ => {
                let (start, path) = self.start(a, relocate);
                let r = Resolved {
                    herdr,
                    pane_at,
                    start,
                    path,
                    stamp: None,
                    id: None,
                };
                (r, true)
            }
        };
        let stamp = r.path.as_deref().and_then(stamp);
        if fresh || stamp != r.stamp {
            r.stamp = stamp;
            r.id = self.follow(r.start.clone(), r.path.as_deref());
        }
        let id = r.id.clone();
        self.resolved.insert(a.terminal_id.clone(), r);
        id
    }

    fn start(&self, a: &AgentInfo, relocate: bool) -> (Option<String>, Option<PathBuf>) {
        let sessions = self.sessions_dir();
        let start = sessions
            .as_deref()
            .and_then(|s| self.pane_session(s, a).or_else(|| self.attached_to(s, a)))
            .or_else(|| session_id(a).map(str::to_owned));
        let path = start
            .as_deref()
            .filter(|id| relocate || !self.missed.contains(*id))
            .and_then(|id| self.locate(Kind::Claude, id, a));
        (start, path)
    }

    fn follow(&self, start: Option<String>, path: Option<&Path>) -> Option<String> {
        match path {
            Some(path) => Some(follow(path, start?, self.sessions_dir().as_deref())),
            None => start,
        }
    }

    pub(super) fn sessions_dir(&self) -> Option<PathBuf> {
        let dir = self.claude_projects.as_deref()?.parent()?.join("sessions");
        std::fs::symlink_metadata(&dir)
            .is_ok_and(|m| m.is_dir())
            .then_some(dir)
    }

    /// Only when every trusted process in the pane reports the same session in the pane's cwd.
    fn pane_session(&self, sessions: &Path, a: &AgentInfo) -> Option<String> {
        let pane = self
            .panes
            .get(&a.terminal_id)
            .filter(|p| p.pane_id == a.pane_id)?;
        let cwds = [&a.foreground_cwd, &a.cwd];
        let mut found = pane
            .pids
            .iter()
            .filter_map(|pid| claim(sessions, *pid))
            .filter(|c| cwds.iter().any(|d| d.as_deref() == Some(c.cwd.as_str())))
            .map(|c| c.session);
        let first = found.next()?;
        found.all(|s| s == first).then_some(first)
    }

    /// Claude Code matches up to 8 hex as a job id prefix and anything else as a name; here a
    /// name must be exact. The attach client can switch to another session from its agent
    /// view without its argv changing, so a named session must also be the pane's title.
    fn attached_to(&self, sessions: &Path, a: &AgentInfo) -> Option<String> {
        let target = self
            .panes
            .get(&a.terminal_id)
            .filter(|p| p.pane_id == a.pane_id)?
            .attach
            .as_deref()
            .filter(|t| !t.is_empty())?;
        let job = target.len() <= 8
            && target
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        let found = unique(
            background(sessions)?.filter(|c| match (job, &c.job, &c.name) {
                (true, Some(j), _) => j == target || (target.len() < 8 && j.starts_with(target)),
                (false, _, Some(name)) => name == target,
                _ => false,
            }),
        )?;
        match found.name {
            Some(name) if a.terminal_title_stripped.as_deref() != Some(name.as_str()) => None,
            _ => Some(found.session),
        }
    }

    fn copilot_id(&self, a: &AgentInfo) -> Option<String> {
        self.copilot_selection(a).map(|(id, _)| id)
    }

    pub(super) fn copilot_selection(&self, a: &AgentInfo) -> Option<(String, Vec<String>)> {
        let root = self.copilot_sessions.as_deref()?;
        std::fs::symlink_metadata(root)
            .ok()?
            .is_dir()
            .then_some(())?;
        let pane = self
            .panes
            .get(&a.terminal_id)
            .filter(|p| p.pane_id == a.pane_id && !p.truncated)?;
        let title = a
            .terminal_title_stripped
            .as_deref()?
            .strip_suffix(" - GitHub Copilot")?;
        let pids: Vec<_> = pane
            .pids
            .iter()
            .copied()
            .filter(|pid| *pid > 0 && alive(*pid))
            .map(|pid| (pid, started_ms(pid)))
            .collect();
        if pids.is_empty() {
            return None;
        }
        let mut cache = crate::lock(&pane.copilot);
        if cache.pids != pids {
            cache.empty.clear();
            cache.pids.clone_from(&pids);
        }
        let mut found: Option<(String, i32, u64)> = None;
        let mut sessions = Vec::new();
        let (mut probes, mut claims, mut budget) = (0, 0, 1024 * 1024);
        for (listed, entry) in std::fs::read_dir(root).ok()?.enumerate() {
            if listed == MAX_LISTED {
                return None;
            }
            let entry = entry.ok()?;
            let name = entry.file_name();
            let Some(id) = name.to_str().filter(|id| is_session_id(id)) else {
                continue;
            };
            let dir = entry.path();
            let metadata = std::fs::symlink_metadata(&dir).ok()?;
            metadata.is_dir().then_some(())?;
            let stamp = DirectoryStamp {
                device: metadata.dev(),
                inode: metadata.ino(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
            };
            if cache.empty.get(id) == Some(&stamp) {
                continue;
            }
            cache.empty.remove(id);
            let mut owner = None;
            let mut markerless = true;
            for &(pid, started) in &pids {
                probes += 1;
                if probes > MAX_LISTED * MAX_PIDS {
                    return None;
                }
                let marker = dir.join(format!("inuse.{pid}.hold"));
                match std::fs::symlink_metadata(&marker) {
                    Ok(_) => markerless = false,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => return None,
                }
                if !super::provider_subagents::held(&marker)? {
                    continue;
                }
                let (_, _, modified) = open(&marker)?;
                let marked = modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
                let started = started?;
                if started > marked.saturating_add(START_SLACK_MS) {
                    continue;
                }
                let (file, len, _) = open(&dir.join(format!("inuse.{pid}.lock")))?;
                if len > 32 {
                    return None;
                }
                let mut bytes = Vec::new();
                file.take(33).read_to_end(&mut bytes).ok()?;
                if std::str::from_utf8(&bytes)
                    .ok()?
                    .trim()
                    .parse::<i32>()
                    .ok()?
                    != pid
                {
                    return None;
                }
                owner = Some((pid, started));
                break;
            }
            let Some((pid, started)) = owner else {
                // Unlocked existing markers can become held without changing the directory.
                if markerless && cache.empty.len() < MAX_LISTED {
                    cache.empty.insert(id.to_owned(), stamp);
                }
                continue;
            };
            claims += 1;
            if claims > MAX_SESSION_FILES {
                return None;
            }
            let (file, len, _) = open(&dir.join("workspace.yaml"))?;
            if len > SESSION_BYTES || len > budget {
                return None;
            }
            let mut bytes = Vec::new();
            file.take(len + 1).read_to_end(&mut bytes).ok()?;
            budget = budget.checked_sub(bytes.len() as u64)?;
            if bytes.len() as u64 != len {
                return None;
            }
            let workspace = std::str::from_utf8(&bytes).ok()?;
            if workspace_scalar(workspace, "id")? != id {
                return None;
            }
            let name = if workspace.lines().any(|line| line.starts_with("name:")) {
                Some(workspace_scalar(workspace, "name")?)
            } else {
                None
            };
            let cwd = workspace_scalar(workspace, "cwd")?;
            if !Path::new(&cwd).is_absolute() {
                return None;
            }
            let events = dir.join("events.jsonl");
            // Copilot 1.0.103 holds the session it started with but writes no events until a
            // prompt; after `/new` that unnamed session stays held beside the new one.
            if name.is_none()
                && std::fs::symlink_metadata(&events)
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            {
                continue;
            }
            let (file, _, _) = open(&events)?;
            let limit = budget.min(SESSION_BYTES);
            let mut reader = std::io::BufReader::with_capacity(1024, file.take(limit));
            let mut header = Vec::new();
            reader.read_until(b'\n', &mut header).ok()?;
            budget -= limit - reader.into_inner().limit();
            if header.last() != Some(&b'\n') {
                return None;
            }
            let event: Value = serde_json::from_slice(&header).ok()?;
            if event["type"] != "session.start" || event["data"]["sessionId"] != id {
                return None;
            }
            sessions.push(id.to_owned());
            let Some(name) = name else {
                continue;
            };
            // Copilot 1.0.100-1 renders the name, optional assistant intent, then its app name.
            if title != name
                && !title
                    .strip_prefix(&name)
                    .is_some_and(|rest| rest.starts_with(" - ") && rest.len() > 3)
            {
                continue;
            }
            if ![&a.foreground_cwd, &a.cwd]
                .iter()
                .any(|c| c.as_deref() == Some(cwd.as_str()))
                || found
                    .as_ref()
                    .is_some_and(|(previous, _, _)| previous != id)
            {
                return None;
            }
            found = Some((id.to_owned(), pid, started));
        }
        let (id, pid, started) = found?;
        if !alive(pid)
            || started_ms(pid)? != started
            || !super::provider_subagents::held(&root.join(&id).join(format!("inuse.{pid}.hold")))?
        {
            return None;
        }
        Some((id, sessions))
    }
}

fn workspace_scalar(text: &str, key: &str) -> Option<String> {
    if text
        .lines()
        .any(|line| line.starts_with([' ', '\t']) && !line.trim().is_empty())
    {
        return None;
    }
    let prefix = format!("{key}:");
    let mut values = text.lines().filter_map(|line| line.strip_prefix(&prefix));
    let raw = values.next()?;
    if !raw.starts_with(' ') {
        return None;
    }
    let raw = raw.trim();
    if values.next().is_some() || raw.is_empty() {
        return None;
    }
    let value = if raw.starts_with('"') {
        serde_json::from_str::<String>(raw).ok()?
    } else if let Some(quoted) = raw.strip_prefix('\'') {
        let mut chars = quoted.strip_suffix('\'')?.chars();
        let mut value = String::new();
        while let Some(c) = chars.next() {
            if c == '\'' && chars.next() != Some('\'') {
                return None;
            }
            value.push(c);
        }
        value
    } else {
        if raw.starts_with(['|', '>', '[', '{', '!', '&', '*', '#'])
            || raw.contains(" #")
            || raw.contains(": ")
            || matches!(raw, "null" | "~" | "true" | "false")
        {
            return None;
        }
        raw.to_owned()
    };
    (!value.is_empty() && !value.chars().any(char::is_control)).then_some(value)
}

fn unique(mut found: impl Iterator<Item = Claim>) -> Option<Claim> {
    let first = found.next()?;
    found.all(|c| c.session == first.session).then_some(first)
}

fn background(sessions: &Path) -> Option<impl Iterator<Item = Claim>> {
    let files = std::fs::read_dir(sessions).ok()?;
    Some(
        files
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()?
                    .strip_suffix(".json")?
                    .parse::<i32>()
                    .ok()
            })
            .take(MAX_SESSION_FILES)
            .filter_map(move |pid| claim(sessions, pid))
            .filter(|c| c.kind.as_deref() == Some("bg")),
    )
}

fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// Trusted only while that pid is alive and started before the file says it did, so a file
/// left by a process whose pid was reused is refused.
fn claim(sessions: &Path, pid: i32) -> Option<Claim> {
    if pid <= 0 {
        return None;
    }
    let (file, _, _) = open(&sessions.join(format!("{pid}.json")))?;
    let mut bytes = Vec::new();
    file.take(SESSION_BYTES).read_to_end(&mut bytes).ok()?;
    let v = serde_json::from_slice::<Value>(&bytes).ok()?;
    if v["pid"] != pid {
        return None;
    }
    let session = v["sessionId"].as_str().filter(|s| is_session_id(s))?;
    let started_at = v["startedAt"].as_u64()?;
    let claim = Claim {
        session: session.to_owned(),
        cwd: v["cwd"].as_str()?.to_owned(),
        kind: v["kind"].as_str().map(str::to_owned),
        job: v["jobId"].as_str().map(str::to_owned),
        name: v["name"].as_str().map(str::to_owned),
    };
    (alive(pid) && started_ms(pid)? <= started_at.saturating_add(START_SLACK_MS)).then_some(claim)
}

/// Bounded and within one project directory: a loop or a longer chain keeps `start`.
fn follow(path: &Path, start: String, sessions: Option<&Path>) -> String {
    let Some(dir) = path.parent() else {
        return start;
    };
    let mut chain = vec![start];
    let mut path = path.to_owned();
    loop {
        let id = &chain[chain.len() - 1];
        let Some(next) = moved_to(&path, id, dir, sessions) else {
            return chain.pop().unwrap_or_default();
        };
        if chain.contains(&next) || chain.len() > MAX_HOPS {
            return chain.swap_remove(0);
        }
        path = dir.join(format!("{next}.jsonl"));
        chain.push(next);
    }
}

/// The latest hand-off since the session's last turn that resolves: a `continued-in`
/// record whose target's transcript is in `dir`, or `/resume` opening a background session
/// that a live `bg` process of `dir` runs.
fn moved_to(path: &Path, id: &str, dir: &Path, sessions: Option<&Path>) -> Option<String> {
    let (mut file, len, _) = open(path)?;
    let start = len.saturating_sub(POINTER_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(POINTER_BYTES).read_to_end(&mut bytes).ok()?;
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    if !has(b"continued-in") && !has(b"commandRun") {
        return None;
    }
    let mut pointers = Vec::new();
    for line in bytes.split(|b| *b == b'\n').skip(usize::from(start > 0)) {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if v["sessionId"] != id {
            continue;
        }
        match v["type"].as_str() {
            Some("assistant") => pointers.clear(),
            Some("user") if v["origin"]["kind"] == "human" => pointers.clear(),
            Some("continued-in") => {
                if let Some(to) = v["continuedInSessionId"].as_str() {
                    pointers.push(Pointer::Continued(to.to_owned()));
                }
            }
            Some("system")
                if v["subtype"] == "local_command" && v["commandRun"]["command"] == "resume" =>
            {
                if let Some(job) = v["content"].as_str().and_then(background_job) {
                    pointers.push(Pointer::Attached(job.to_owned()));
                }
            }
            _ => {}
        }
    }
    pointers.into_iter().rev().find_map(|p| match p {
        Pointer::Continued(to) => {
            (is_session_id(&to) && to != id && is_file(&dir.join(format!("{to}.jsonl"))))
                .then_some(to)
        }
        Pointer::Attached(job) => attached(sessions?, dir, &job),
    })
}

/// The job id at the end of `/resume`'s output, `… running in the background (<job>)`.
fn background_job(content: &str) -> Option<&str> {
    let rest = content.strip_suffix(")</local-command-stdout>")?;
    let job = rest.get(rest.len().checked_sub(8)?..)?;
    (rest[..rest.len() - 8].ends_with("running in the background (")
        && job
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(job)
}

fn attached(sessions: &Path, dir: &Path, job: &str) -> Option<String> {
    let name = dir.file_name()?.to_str()?;
    unique(background(sessions)?.filter(|c| {
        c.job.as_deref() == Some(job)
            && c.session.starts_with(job)
            && project_dir(&c.cwd) == name
            && is_file(&dir.join(format!("{}.jsonl", c.session)))
    }))
    .map(|c| c.session)
}

#[cfg(target_os = "macos")]
fn started_ms(pid: i32) -> Option<u64> {
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    // SAFETY: `info` is a writable `proc_bsdinfo` of `size` bytes, what PROC_PIDTBSDINFO fills.
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
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
    Some(info.pbi_start_tvsec * 1000 + info.pbi_start_tvusec / 1000)
}

#[cfg(target_os = "linux")]
fn started_ms(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ticks: u64 = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()?;
    let boot: u64 = std::fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    let hz = rustix::param::clock_ticks_per_second();
    Some(boot * 1000 + ticks * 1000 / hz.max(1))
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;

    use protocol::{HistoryParams, TerminalId};

    use super::*;
    use crate::herdr::AgentSession;

    const CLAUDE: &str = include_str!("../../tests/fixtures/transcripts/claude.jsonl");
    const LIVE: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    const OLD: &str = "0a0a0a0a-0000-4000-8000-000000000001";
    const CLIENT: &str = "0c0c0c0c-0000-4000-8000-000000000002";
    const PARKED: &str = "0d0d0d0d-0000-4000-8000-000000000003";
    const CWD: &str = "/Users/me/src/collie";
    const DEAD: i32 = 0x7fff_fff0;
    const COPILOT_OLD: &str = "10000000-0000-4000-8000-000000000001";
    const COPILOT_CURRENT: &str = "20000000-0000-4000-8000-000000000002";
    const COPILOT_CHILD: &str = "30000000-0000-4000-8000-000000000003";
    const COPILOT_OLD_META: &str =
        include_str!("../../tests/fixtures/transcripts/copilot-new-old.workspace.yaml");
    const COPILOT_META: &str =
        include_str!("../../tests/fixtures/transcripts/copilot-new-current.workspace.yaml");
    const COPILOT_OLD_EVENTS: &str =
        include_str!("../../tests/fixtures/transcripts/copilot-new-old.jsonl");
    const COPILOT_EVENTS: &str =
        include_str!("../../tests/fixtures/transcripts/copilot-new-current.jsonl");

    fn setup() -> (tempfile::TempDir, Transcripts, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        let t = Transcripts::new(Some(claude.join("projects")), None);
        let project = claude.join("projects").join(project_dir(CWD));
        let sessions = claude.join("sessions");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&sessions).unwrap();
        (dir, t, project, sessions)
    }

    fn agent(id: &str) -> AgentInfo {
        AgentInfo {
            terminal_id: "term_1".into(),
            workspace_id: "w1".into(),
            pane_id: "w1:p1".into(),
            agent: Some("claude".into()),
            name: None,
            title: None,
            terminal_title_stripped: None,
            agent_status: "idle".into(),
            state_change_seq: 1,
            cwd: Some(CWD.into()),
            foreground_cwd: Some(CWD.into()),
            agent_session: Some(AgentSession { value: id.into() }),
            interactive_ready: true,
            launch_pending: false,
        }
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn transcript(project: &Path, id: &str, text: &str) {
        write(&project.join(format!("{id}.jsonl")), text);
    }

    fn reply(id: &str, text: &str) -> String {
        format!(
            r#"{{"isSidechain":false,"type":"assistant","message":{{"model":"claude-opus-5-5","role":"assistant","content":[{{"type":"text","text":"{text}"}}],"stop_reason":"end_turn","usage":{{"input_tokens":1000}}}},"sessionId":"{id}"}}"#
        ) + "\n"
    }

    fn continued(id: &str, to: &str) -> String {
        format!(r#"{{"type":"continued-in","sessionId":"{id}","continuedInSessionId":"{to}"}}"#)
            + "\n"
    }

    /// What a fresh client writes when `/resume` opens a background session.
    fn resumed(job: &str) -> String {
        [
            format!(
                r#"{{"type":"system","subtype":"local_command","content":"<command-name>/resume</command-name>","isMeta":false,"cwd":"{CWD}","sessionId":"{CLIENT}"}}"#
            ),
            format!(
                r#"{{"type":"system","subtype":"local_command","content":"<local-command-stdout>Opening \"Fix (deadbeef) things\", running in the background ({job})</local-command-stdout>","commandRun":{{"command":"resume","args":""}},"isMeta":false,"cwd":"{CWD}","sessionId":"{CLIENT}"}}"#
            ),
            continued(CLIENT, PARKED),
        ]
        .join("\n")
    }

    fn pid_file(sessions: &Path, pid: i32, id: &str, cwd: &str, started_at: u64, extra: &str) {
        write(
            &sessions.join(format!("{pid}.json")),
            &format!(
                r#"{{"pid":{pid},"sessionId":"{id}","cwd":"{cwd}","startedAt":{started_at}{extra}}}"#
            ),
        );
    }

    fn own() -> (i32, u64) {
        let pid = std::process::id() as i32;
        (pid, started_ms(pid).unwrap())
    }

    fn parent() -> (i32, u64) {
        let pid = std::os::unix::process::parent_id() as i32;
        (pid, started_ms(pid).unwrap())
    }

    fn pane(t: &mut Transcripts, pids: &[i32]) {
        attach(t, pids, None);
    }

    fn attach(t: &mut Transcripts, pids: &[i32], target: Option<&str>) {
        t.set_pane(
            "term_1".into(),
            "w1:p1".into(),
            Some(PaneProcesses {
                pids: pids.iter().map(|p| *p as u32).collect(),
                attach: target.map(str::to_owned),
            }),
        );
    }

    fn copilot_setup() -> (tempfile::TempDir, Transcripts, PathBuf, AgentInfo) {
        let (dir, mut t, _, _) = setup();
        let root = dir.path().join("copilot/session-state");
        std::fs::create_dir_all(&root).unwrap();
        t.copilot_sessions = Some(root.clone());
        let mut a = agent(COPILOT_OLD);
        a.agent = Some("copilot".into());
        a.terminal_title_stripped =
            Some("Current conversation - Waiting for background agents - GitHub Copilot".into());
        (dir, t, root, a)
    }

    fn copilot_records(fixture: &str, at: u64) -> String {
        let (y, m, d) = super::super::civil((at / 86_400_000) as i64);
        let timestamp = format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
            at / 3_600_000 % 24,
            at / 60_000 % 60,
            at / 1000 % 60,
            at % 1000
        );
        fixture
            .lines()
            .map(|line| {
                let mut v: Value = serde_json::from_str(line).unwrap();
                v["timestamp"] = timestamp.clone().into();
                format!("{v}\n")
            })
            .collect()
    }

    fn copilot_session(root: &Path, id: &str, pid: i32, current: bool) -> File {
        let (meta, records, original) = if current {
            (COPILOT_META, COPILOT_EVENTS, COPILOT_CURRENT)
        } else {
            (COPILOT_OLD_META, COPILOT_OLD_EVENTS, COPILOT_OLD)
        };
        let dir = root.join(id);
        write(
            &dir.join("workspace.yaml"),
            &meta
                .replace(original, id)
                .replace("/workspace/project", CWD),
        );
        let at = own().1;
        write(
            &dir.join("events.jsonl"),
            &copilot_records(records, at).replace(original, id),
        );
        write(&dir.join(format!("inuse.{pid}.lock")), &format!("{pid}\n"));
        let hold = File::create(dir.join(format!("inuse.{pid}.hold"))).unwrap();
        hold.set_modified(UNIX_EPOCH + Duration::from_millis(at))
            .unwrap();
        hold.lock().unwrap();
        hold
    }

    fn complete_copilot_session(root: &Path, id: &str) {
        let mut complete: Value = serde_json::from_str(
            include_str!("../../tests/fixtures/transcripts/copilot-subagents.jsonl")
                .lines()
                .find(|line| line.contains("\"subagent.completed\""))
                .unwrap(),
        )
        .unwrap();
        complete["agentId"] = COPILOT_CHILD.into();
        complete["data"]["toolCallId"] = "fixture-5".into();
        std::fs::OpenOptions::new()
            .append(true)
            .open(root.join(id).join("events.jsonl"))
            .unwrap()
            .write_all(format!("{complete}\n").as_bytes())
            .unwrap();
    }

    #[test]
    fn copilot_new_selects_the_displayed_conversation_not_the_stale_herdr_id() {
        let (_dir, mut t, root, mut a) = copilot_setup();
        let (pid, _) = own();
        let _old = copilot_session(&root, COPILOT_OLD, pid, false);
        let _current = copilot_session(&root, COPILOT_CURRENT, pid, true);
        pane(&mut t, &[pid]);
        a.terminal_title_stripped = Some("Previous conversation - GitHub Copilot".into());
        assert_eq!(t.resolve(&a, false).as_deref(), Some(COPILOT_OLD));
        assert!(t.derive(&a, false).unwrap().subagents.is_empty());
        assert!(!t.quiet(&a));
        a.terminal_title_stripped =
            Some("Current conversation - Waiting for background agents - GitHub Copilot".into());
        assert_eq!(t.resolve(&a, false).as_deref(), Some(COPILOT_CURRENT));
        assert_eq!(
            t.history_reader().live_id(&a).as_deref(),
            Some(COPILOT_CURRENT)
        );
        let d = t.derive(&a, false).unwrap();
        assert_eq!(d.subagents.len(), 1);
        assert_eq!(d.subagents[0].title, "fixture-child");
        assert_eq!(d.last_line, None);
        assert!(!t.quiet(&a));
        t.retain(std::slice::from_ref(&a));
        complete_copilot_session(&root, COPILOT_CURRENT);
        assert!(t.quiet(&a));
        a.agent_session = None;
        assert_eq!(t.live_id(&a).as_deref(), Some(COPILOT_CURRENT));
    }

    #[test]
    fn copilot_unnamed_held_sessions_do_not_hide_the_current_conversation() {
        let (_dir, mut t, root, a) = copilot_setup();
        let (pid, _) = own();
        let _unnamed = copilot_session(&root, COPILOT_OLD, pid, true);
        let _current = copilot_session(&root, COPILOT_CURRENT, pid, true);
        let metadata = root.join(COPILOT_OLD).join("workspace.yaml");
        let text = std::fs::read_to_string(&metadata).unwrap();
        write(&metadata, &text.replace("name: Current conversation\n", ""));
        pane(&mut t, &[pid]);
        assert_eq!(t.resolve(&a, false).as_deref(), Some(COPILOT_CURRENT));
        assert_eq!(t.derive(&a, false).unwrap().subagents.len(), 1);
        complete_copilot_session(&root, COPILOT_CURRENT);
        assert!(t.derive(&a, false).unwrap().subagents.is_empty());
        t.retain(std::slice::from_ref(&a));
        assert!(!t.quiet(&a));
        complete_copilot_session(&root, COPILOT_OLD);
        assert!(t.quiet(&a));
    }

    #[test]
    fn copilot_new_before_a_prompt_follows_the_new_conversation() {
        let (_dir, mut t, root, a) = copilot_setup();
        let (pid, _) = own();
        let _started = copilot_session(&root, COPILOT_OLD, pid, false);
        let _current = copilot_session(&root, COPILOT_CURRENT, pid, true);
        let started = root.join(COPILOT_OLD);
        std::fs::remove_file(started.join("events.jsonl")).unwrap();
        let metadata = started.join("workspace.yaml");
        let text = std::fs::read_to_string(&metadata).unwrap();
        write(
            &metadata,
            &text.replace("name: Previous conversation\n", ""),
        );
        pane(&mut t, &[pid]);
        assert_eq!(t.resolve(&a, false).as_deref(), Some(COPILOT_CURRENT));
        assert_eq!(
            t.history_reader().live_id(&a).as_deref(),
            Some(COPILOT_CURRENT)
        );
        assert_eq!(t.derive(&a, false).unwrap().subagents.len(), 1);
        assert!(!t.quiet(&a));
        t.retain(std::slice::from_ref(&a));
        complete_copilot_session(&root, COPILOT_CURRENT);
        assert!(t.quiet(&a));
        write(&metadata, &text);
        assert_eq!(t.resolve(&a, false), None);
    }

    #[test]
    fn copilot_rechecks_title_and_name_ambiguity_without_a_stale_selection_cache() {
        let (_dir, mut t, root, mut a) = copilot_setup();
        let (pid, _) = own();
        let _old = copilot_session(&root, COPILOT_OLD, pid, false);
        let _current = copilot_session(&root, COPILOT_CURRENT, pid, true);
        pane(&mut t, &[pid]);
        a.terminal_title_stripped = Some("Previous conversation - GitHub Copilot".into());
        assert_eq!(t.resolve(&a, false).as_deref(), Some(COPILOT_OLD));
        let meta = root.join(COPILOT_CURRENT).join("workspace.yaml");
        let text = std::fs::read_to_string(&meta).unwrap();
        write(
            &meta,
            &text.replace("Current conversation", "Previous conversation"),
        );
        assert_eq!(t.resolve(&a, false), None);
        assert_eq!(t.history_reader().live_id(&a), None);
        assert!(t.derive(&a, false).is_none());
        assert!(!t.quiet(&a));
        let old = root.join(COPILOT_OLD).join("workspace.yaml");
        let text = std::fs::read_to_string(&old).unwrap();
        write(
            &old,
            &text.replace("Previous conversation", "Another conversation"),
        );
        assert_eq!(t.resolve(&a, false).as_deref(), Some(COPILOT_CURRENT));
    }

    #[test]
    fn copilot_requires_complete_process_identity_for_this_pane() {
        let (_dir, mut t, root, mut a) = copilot_setup();
        let (pid, _) = own();
        let _current = copilot_session(&root, COPILOT_CURRENT, pid, true);
        assert_eq!(t.live_id(&a), None);
        assert_eq!(t.stale_panes(std::slice::from_ref(&a)).len(), 1);
        pane(&mut t, &[pid]);
        assert_eq!(t.stale_panes(std::slice::from_ref(&a)), []);
        assert_eq!(t.live_id(&a).as_deref(), Some(COPILOT_CURRENT));
        a.pane_id = "w1:p2".into();
        assert_eq!(t.live_id(&a), None);
        a.pane_id = "w1:p1".into();
        pane(&mut t, &[pid; MAX_PIDS + 1]);
        assert_eq!(t.live_id(&a), None);
        assert!(!t.quiet(&a));
        pane(&mut t, &[DEAD]);
        assert_eq!(t.live_id(&a), None);
    }

    #[test]
    fn copilot_rejects_foreign_and_reused_pid_claims_even_when_the_name_matches() {
        let (_dir, mut t, root, a) = copilot_setup();
        let (pid, started) = own();
        let (other, _) = parent();
        let current = copilot_session(&root, COPILOT_CURRENT, pid, true);
        let _foreign = copilot_session(&root, CLIENT, other, true);
        pane(&mut t, &[pid]);
        assert_eq!(t.live_id(&a).as_deref(), Some(COPILOT_CURRENT));
        pane(&mut t, &[pid, other]);
        assert_eq!(t.live_id(&a), None);
        pane(&mut t, &[pid]);
        current
            .set_modified(UNIX_EPOCH + Duration::from_millis(started - START_SLACK_MS - 1))
            .unwrap();
        assert_eq!(t.live_id(&a), None);
        current
            .set_modified(UNIX_EPOCH + Duration::from_millis(started))
            .unwrap();
        write(
            &root.join(COPILOT_CURRENT).join(format!("inuse.{pid}.lock")),
            &other.to_string(),
        );
        assert_eq!(t.live_id(&a), None);
    }

    #[test]
    fn copilot_does_not_follow_invalid_directories_or_mismatched_headers() {
        let (_dir, mut t, root, a) = copilot_setup();
        let (pid, _) = own();
        let _current = copilot_session(&root, COPILOT_CURRENT, pid, true);
        pane(&mut t, &[pid]);
        std::fs::create_dir(root.join("not-a-session")).unwrap();
        assert_eq!(t.live_id(&a).as_deref(), Some(COPILOT_CURRENT));
        let link = root.join(CLIENT);
        symlink(root.join(COPILOT_CURRENT), &link).unwrap();
        assert_eq!(t.live_id(&a), None);
        assert!(!t.quiet(&a));
        std::fs::remove_file(link).unwrap();
        let events = root.join(COPILOT_CURRENT).join("events.jsonl");
        let text = std::fs::read_to_string(&events).unwrap();
        write(&events, &text.replace(COPILOT_CURRENT, COPILOT_OLD));
        assert_eq!(t.live_id(&a), None);
    }

    #[test]
    fn copilot_requires_matching_cwd_and_supported_native_title() {
        let (_dir, mut t, root, mut a) = copilot_setup();
        let (pid, _) = own();
        let _current = copilot_session(&root, COPILOT_CURRENT, pid, true);
        pane(&mut t, &[pid]);
        let path = root.join(COPILOT_CURRENT).join("workspace.yaml");
        let text = std::fs::read_to_string(&path).unwrap();
        write(&path, &text.replace(CWD, "/other/project"));
        assert_eq!(t.live_id(&a), None);
        write(
            &path,
            &text.replace("Current conversation", "'Current ''quoted'' conversation'"),
        );
        a.terminal_title_stripped = Some(
            "Current 'quoted' conversation - Waiting for background shells - GitHub Copilot".into(),
        );
        assert_eq!(t.live_id(&a).as_deref(), Some(COPILOT_CURRENT));
        a.terminal_title_stripped = Some("GitHub Copilot".into());
        assert_eq!(t.live_id(&a), None);
    }

    #[test]
    fn copilot_unknown_or_oversized_workspace_metadata_never_proves_quiet() {
        let (_dir, mut t, root, a) = copilot_setup();
        let (pid, _) = own();
        let _current = copilot_session(&root, COPILOT_CURRENT, pid, true);
        pane(&mut t, &[pid]);
        let path = root.join(COPILOT_CURRENT).join("workspace.yaml");
        let text = std::fs::read_to_string(&path).unwrap();
        write(&path, &(text.clone() + "name: Duplicate\n"));
        assert_eq!(t.live_id(&a), None);
        write(&path, &(text + &"x".repeat(SESSION_BYTES as usize)));
        assert_eq!(t.live_id(&a), None);
        assert!(!t.quiet(&a));
    }

    #[test]
    fn copilot_markerless_history_cache_does_not_hide_new_or_reacquired_claims() {
        let (_dir, mut t, root, a) = copilot_setup();
        let (pid, _) = own();
        let _current = copilot_session(&root, COPILOT_CURRENT, pid, true);
        let other = "aa000000-0000-4000-8000-000000000000";
        for n in 0..=MAX_LISTED / MAX_PIDS {
            std::fs::create_dir(root.join(format!("aa000000-0000-4000-8000-{n:012x}"))).unwrap();
        }
        pane(&mut t, &[pid; MAX_PIDS]);
        assert_eq!(t.live_id(&a).as_deref(), Some(COPILOT_CURRENT));
        assert_eq!(t.live_id(&a).as_deref(), Some(COPILOT_CURRENT));
        let new = copilot_session(&root, other, pid, true);
        assert_eq!(t.live_id(&a), None);
        assert!(!t.quiet(&a));
        drop(new);
        assert_eq!(t.live_id(&a).as_deref(), Some(COPILOT_CURRENT));
        let reacquired = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.join(other).join(format!("inuse.{pid}.hold")))
            .unwrap();
        reacquired.lock().unwrap();
        assert_eq!(t.live_id(&a), None);
    }

    #[test]
    fn copilot_wrapped_native_names_cannot_match_another_sessions_short_name() {
        let (_dir, mut t, root, mut a) = copilot_setup();
        let (pid, _) = own();
        let _old = copilot_session(&root, COPILOT_OLD, pid, false);
        let _current = copilot_session(&root, COPILOT_CURRENT, pid, true);
        pane(&mut t, &[pid]);
        let old = root.join(COPILOT_OLD).join("workspace.yaml");
        let text = std::fs::read_to_string(&old).unwrap();
        write(
            &old,
            &text.replace("Previous conversation", "Current conversation"),
        );
        let current = root.join(COPILOT_CURRENT).join("workspace.yaml");
        let text = std::fs::read_to_string(&current).unwrap();
        write(
            &current,
            &text.replace(
                "Current conversation",
                "Current conversation - a long title\n  continues here",
            ),
        );
        a.terminal_title_stripped =
            Some("Current conversation - a long title continues here - GitHub Copilot".into());
        assert_eq!(t.live_id(&a), None);
        assert!(!t.quiet(&a));
    }

    #[test]
    fn the_pane_process_replaces_a_stale_herdr_id() {
        let (_dir, mut t, project, sessions) = setup();
        transcript(&project, OLD, &reply(OLD, "Old reply."));
        transcript(&project, LIVE, CLAUDE);
        let a = agent(OLD);
        let (pid, started) = own();
        pid_file(&sessions, pid, LIVE, CWD, started, "");
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD));

        pane(&mut t, &[DEAD, pid]);
        assert_eq!(t.stale_panes(std::slice::from_ref(&a)), []);
        let d = t.derive(&a, true).unwrap();
        assert_eq!(
            (d.context_left, d.last_line.as_deref()),
            (Some(94), Some("**Fixed** the flaky test."))
        );
        let params = HistoryParams {
            terminal_id: TerminalId::new("term_1").unwrap(),
            before: None,
            after: None,
            query: None,
            latest: false,
        };
        let page = t.history_reader().history(&a, &params);
        assert!(
            page.session
                .unwrap()
                .starts_with(&format!("claude:{LIVE}:"))
        );

        let mut moved = a.clone();
        moved.pane_id = "w1:p2".into();
        assert_eq!(t.live_id(&moved).as_deref(), Some(OLD));
        assert_eq!(t.stale_panes(&[moved]).len(), 1);
        let mut none = a.clone();
        none.agent_session = None;
        assert_eq!(t.live_id(&none).as_deref(), Some(LIVE));
    }

    #[test]
    fn untrusted_pid_files_keep_herdr_id() {
        let (dir, mut t, project, sessions) = setup();
        transcript(&project, OLD, &reply(OLD, "Old reply."));
        transcript(&project, LIVE, CLAUDE);
        let a = agent(OLD);
        let (pid, started) = own();

        pid_file(&sessions, pid, LIVE, "/Users/me/src/other", started, "");
        pane(&mut t, &[pid]);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "cwd mismatch");

        pid_file(&sessions, pid, LIVE, CWD, started - 60_000, "");
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "reused pid");

        pid_file(&sessions, DEAD, LIVE, CWD, started, "");
        pane(&mut t, &[DEAD]);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "dead pid");

        write(
            &sessions.join(format!("{pid}.json")),
            &format!(r#"{{"pid":1,"sessionId":"{LIVE}","cwd":"{CWD}","startedAt":{started}}}"#),
        );
        pane(&mut t, &[pid]);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "pid not its own");

        let (other, other_started) = parent();
        pid_file(&sessions, pid, LIVE, CWD, started, "");
        pid_file(&sessions, other, CLIENT, CWD, other_started, "");
        pane(&mut t, &[pid, other]);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "two sessions");

        let elsewhere = dir.path().join("elsewhere.json");
        pid_file(dir.path(), pid, LIVE, CWD, started, "");
        std::fs::rename(dir.path().join(format!("{pid}.json")), &elsewhere).unwrap();
        std::fs::remove_file(sessions.join(format!("{pid}.json"))).unwrap();
        std::os::unix::fs::symlink(&elsewhere, sessions.join(format!("{pid}.json"))).unwrap();
        pane(&mut t, &[pid]);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "symlink");
        std::fs::remove_file(sessions.join(format!("{pid}.json"))).unwrap();
        std::fs::copy(&elsewhere, sessions.join(format!("{pid}.json"))).unwrap();
        assert_eq!(
            t.live_id(&a).as_deref(),
            Some(LIVE),
            "the same file in place"
        );
    }

    #[test]
    fn resolution_is_cached_until_the_pids_or_the_transcript_move() {
        let (_dir, mut t, project, sessions) = setup();
        transcript(&project, OLD, &reply(OLD, "Old reply."));
        transcript(&project, LIVE, CLAUDE);
        let a = agent(OLD);
        let (pid, started) = own();
        pid_file(&sessions, pid, LIVE, CWD, started, "");
        pane(&mut t, &[pid]);
        assert_eq!(t.resolve(&a, false).as_deref(), Some(LIVE));

        let file = sessions.join(format!("{pid}.json"));
        std::fs::remove_file(&file).unwrap();
        assert_eq!(t.resolve(&a, false).as_deref(), Some(LIVE), "cached");

        t.set_pane("term_1".into(), "w1:p1".into(), None);
        assert_eq!(t.panes["term_1"].pids, [pid], "kept on a herdr error");
        assert_eq!(t.resolve(&a, false).as_deref(), Some(OLD), "refreshed");
        pid_file(&sessions, pid, LIVE, CWD, started, "");
        t.set_pane("term_1".into(), "w1:p1".into(), None);
        assert_eq!(t.resolve(&a, false).as_deref(), Some(LIVE));

        transcript(&project, LIVE, &(CLAUDE.to_owned() + &continued(LIVE, OLD)));
        assert_eq!(
            t.resolve(&a, false).as_deref(),
            Some(OLD),
            "transcript moved"
        );
    }

    #[test]
    fn continued_in_is_followed_within_the_project_only() {
        let (dir, t, project, _sessions) = setup();
        let a = agent(OLD);
        transcript(
            &project,
            OLD,
            &(reply(OLD, "Backgrounding.") + &continued(OLD, LIVE)),
        );
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "no target transcript");

        let elsewhere = dir.path().join("claude/projects/-Users-me-src-other");
        transcript(&elsewhere, LIVE, CLAUDE);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "other project");

        transcript(&project, LIVE, CLAUDE);
        assert_eq!(t.live_id(&a).as_deref(), Some(LIVE));

        transcript(
            &project,
            OLD,
            &(continued(OLD, LIVE) + &reply(OLD, "Back in the foreground.")),
        );
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "a later turn");

        transcript(&project, OLD, &continued(OLD, OLD));
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "self");

        transcript(&project, OLD, &continued(OLD, LIVE));
        transcript(&project, LIVE, &continued(LIVE, OLD));
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "loop");

        let ids: Vec<String> = (1..=6)
            .map(|n| format!("0b0b0b0b-0000-4000-8000-00000000000{n}"))
            .collect();
        transcript(&project, OLD, &continued(OLD, &ids[0]));
        for w in ids.windows(2) {
            transcript(&project, &w[0], &continued(&w[0], &w[1]));
        }
        transcript(&project, &ids[5], "");
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "too long");
        transcript(&project, OLD, &continued(OLD, &ids[3]));
        assert_eq!(t.live_id(&a).as_deref(), Some(ids[5].as_str()));
    }

    #[test]
    fn resume_into_a_background_session_follows_it() {
        let (_dir, mut t, project, sessions) = setup();
        let a = agent(CLIENT);
        let (pid, started) = own();
        let (bg, bg_started) = parent();
        transcript(&project, CLIENT, &resumed("3f1b2c4d"));
        transcript(&project, LIVE, CLAUDE);
        pid_file(&sessions, pid, CLIENT, CWD, started, "");
        pane(&mut t, &[pid]);
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "no live bg process");

        let job = r#","jobId":"3f1b2c4d""#;
        let interactive = format!(r#"{job},"kind":"interactive""#);
        pid_file(&sessions, bg, LIVE, CWD, bg_started, &interactive);
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "not kind bg");

        let bg_kind = format!(r#"{job},"kind":"bg""#);
        pid_file(
            &sessions,
            bg,
            LIVE,
            "/Users/me/src/other",
            bg_started,
            &bg_kind,
        );
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "other project");

        pid_file(&sessions, bg, LIVE, CWD, bg_started - 60_000, &bg_kind);
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "reused pid");

        pid_file(&sessions, bg, LIVE, CWD, bg_started, &bg_kind);
        transcript(&project, CLIENT, &resumed("0a0a0a0a"));
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "another job");

        transcript(&project, CLIENT, &resumed("3f1b2c4d"));
        assert_eq!(t.live_id(&a).as_deref(), Some(LIVE));

        let sub = project.join(LIVE).join("subagents");
        write(
            &sub.join("agent-a1.meta.json"),
            include_str!("../../tests/fixtures/transcripts/claude-live-agent.meta.json"),
        );
        let lines = include_str!("../../tests/fixtures/transcripts/claude-live-agent.jsonl")
            .split_inclusive('\n')
            .collect::<Vec<_>>();
        write(&sub.join("agent-a1.jsonl"), &lines[..4].concat());
        let d = t.derive(&a, true).unwrap();
        assert_eq!(
            (d.context_left, d.last_line.as_deref(), d.subagents.len()),
            (Some(94), Some("**Fixed** the flaky test."), 1)
        );

        transcript(
            &project,
            CLIENT,
            &(resumed("3f1b2c4d") + &reply(CLIENT, "Back in the client.")),
        );
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "a later turn");
    }

    #[test]
    fn claude_attach_follows_the_background_session() {
        let (_dir, mut t, project, sessions) = setup();
        let mut a = agent(OLD);
        a.agent_session = None;
        a.terminal_title_stripped = Some("collie app".into());
        let (pid, started) = own();
        let (bg, bg_started) = parent();
        transcript(&project, LIVE, CLAUDE);
        let named = r#","kind":"bg","jobId":"3f1b2c4d","name":"collie app""#;
        pid_file(&sessions, bg, LIVE, CWD, bg_started, named);

        attach(&mut t, &[pid], None);
        assert_eq!(t.live_id(&a), None, "not an attach");
        for unknown in ["0a0a0a0a", "3f1b2c4d5", LIVE, "collie", "Collie app", ""] {
            attach(&mut t, &[pid], Some(unknown));
            assert_eq!(t.live_id(&a), None, "{unknown:?}");
        }
        for target in ["3f1b2c4d", "3f1b", "3", "collie app"] {
            attach(&mut t, &[pid], Some(target));
            assert_eq!(t.live_id(&a).as_deref(), Some(LIVE), "{target}");
        }

        let sub = project.join(LIVE).join("subagents");
        write(
            &sub.join("agent-a1.meta.json"),
            include_str!("../../tests/fixtures/transcripts/claude-live-agent.meta.json"),
        );
        let lines = include_str!("../../tests/fixtures/transcripts/claude-live-agent.jsonl")
            .split_inclusive('\n')
            .collect::<Vec<_>>();
        write(&sub.join("agent-a1.jsonl"), &lines[..4].concat());
        attach(&mut t, &[pid], Some("3f1b2c4d"));
        let d = t.derive(&a, true).unwrap();
        assert_eq!(
            (d.context_left, d.last_line.as_deref(), d.subagents.len()),
            (Some(94), Some("**Fixed** the flaky test."), 1)
        );
        assert!(d.last_prompt.is_some());
        let params = HistoryParams {
            terminal_id: TerminalId::new("term_1").unwrap(),
            before: None,
            after: None,
            query: None,
            latest: false,
        };
        let page = t.history_reader().history(&a, &params);
        assert!(
            page.session
                .unwrap()
                .starts_with(&format!("claude:{LIVE}:"))
        );

        let mut stale = a.clone();
        stale.agent_session = agent(OLD).agent_session;
        assert_eq!(
            t.live_id(&stale).as_deref(),
            Some(LIVE),
            "over a stale herdr id"
        );
        let mut elsewhere = a.clone();
        elsewhere.terminal_title_stripped = Some("other session".into());
        assert_eq!(t.live_id(&elsewhere), None, "showing another session");
        elsewhere.terminal_title_stripped = None;
        assert_eq!(t.live_id(&elsewhere), None, "no title");

        pid_file(&sessions, pid, CLIENT, CWD, started, "");
        assert_eq!(
            t.live_id(&a).as_deref(),
            Some(CLIENT),
            "the pane's own process first"
        );

        let other = "3f1b0000-0000-4000-8000-000000000004";
        pid_file(
            &sessions,
            pid,
            other,
            CWD,
            started,
            r#","kind":"bg","jobId":"3f1b0000","name":"3f1b2c4d""#,
        );
        attach(&mut t, &[DEAD], Some("3f1b"));
        assert_eq!(t.live_id(&a), None, "two job prefixes");
        attach(&mut t, &[DEAD], Some("3f1b2c4d"));
        assert_eq!(t.live_id(&a).as_deref(), Some(LIVE), "hex is never a name");
        attach(&mut t, &[DEAD], Some("3f1b2"));
        assert_eq!(t.live_id(&a).as_deref(), Some(LIVE), "unique job prefix");
        std::fs::remove_file(sessions.join(format!("{pid}.json"))).unwrap();

        let unnamed = r#","kind":"bg","jobId":"3f1b2c4d""#;
        pid_file(&sessions, bg, LIVE, CWD, bg_started, unnamed);
        assert_eq!(t.live_id(&elsewhere).as_deref(), Some(LIVE), "unnamed");
        pid_file(
            &sessions,
            bg,
            LIVE,
            CWD,
            bg_started,
            r#","kind":"interactive","jobId":"3f1b2c4d""#,
        );
        assert_eq!(t.live_id(&a), None, "not kind bg");
        pid_file(&sessions, bg, LIVE, CWD, bg_started - 60_000, named);
        assert_eq!(t.live_id(&a), None, "reused pid");
        std::fs::remove_file(sessions.join(format!("{bg}.json"))).unwrap();
        pid_file(&sessions, DEAD, LIVE, CWD, bg_started, named);
        assert_eq!(t.live_id(&a), None, "dead pid");
    }

    #[test]
    fn background_job_is_read_from_the_end_of_the_output() {
        assert_eq!(
            background_job(
                "<local-command-stdout>Opening \"x (deadbeef)\", running in the background (1a2b3c4d)</local-command-stdout>"
            ),
            Some("1a2b3c4d")
        );
        for bad in [
            "<local-command-stdout>Opening \"x\", running in the background (1a2b3c4d)!</local-command-stdout>",
            "<local-command-stdout>running in the background (1A2B3C4D)</local-command-stdout>",
            "<local-command-stdout>running in the background (1a2b3c4)</local-command-stdout>",
            "<local-command-stdout>running in the background(1a2b3c4d)</local-command-stdout>",
            "<local-command-stdout>Opening \"x (1a2b3c4d)\"</local-command-stdout>",
            "<local-command-stdout>Resumed \"x\" (1a2b3c4d)</local-command-stdout>",
            "(1a2b3c4d)</local-command-stdout>",
        ] {
            assert_eq!(background_job(bad), None, "{bad}");
        }
    }
}
