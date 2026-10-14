mod history;
mod live;
mod subagents;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::herdr::AgentInfo;
use crate::prompt;
use crate::usage::Usage;

/// Only the end is read: transcripts reach tens of MB, and what is shown sits near the end.
const TAIL_BYTES: u64 = 1 << 20;
pub const MAX_TEXT_CHARS: usize = 160;
const CLAUDE_WINDOW: u64 = 200_000;
const CLAUDE_LONG_WINDOW: u64 = 1_000_000;
/// codex-rs `BASELINE_TOKENS`, so the percentage matches Codex's own "context left".
const CODEX_BASELINE: u64 = 12_000;
const MAX_SESSIONS: usize = 20;
/// Codex writes its instructions first (tens of KB); the first prompt follows.
const HEAD_BYTES: u64 = 256 * 1024;
const META_BYTES: u64 = 64 * 1024;
const MAX_LISTED: usize = 10_000;
const MAX_CANDIDATES: usize = 400;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Derived {
    pub context_left: Option<u8>,
    pub last_line: Option<String>,
    pub last_prompt: Option<String>,
    pub last_activity_ms: Option<u64>,
    pub subagents: Vec<protocol::Subagent>,
    /// The subagent scan left some out, so more may be running.
    pub subagents_truncated: bool,
}

#[derive(Clone, Copy)]
enum Kind {
    Claude,
    Codex,
}

struct Cached {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
    window: Option<u64>,
    derived: Derived,
}

#[derive(Default)]
pub struct Transcripts {
    claude_projects: Option<PathBuf>,
    codex_sessions: Option<PathBuf>,
    cache: HashMap<String, Cached>,
    missed: HashSet<String>,
    usage: Usage,
    live: subagents::Live,
    panes: HashMap<String, live::Pane>,
    resolved: HashMap<String, live::Resolved>,
    /// The live session last logged per terminal, so a mismatch with herdr is logged once.
    noted: HashMap<String, String>,
}

impl Transcripts {
    pub fn new(claude_projects: Option<PathBuf>, codex_sessions: Option<PathBuf>) -> Self {
        Self {
            claude_projects,
            codex_sessions,
            cache: HashMap::new(),
            missed: HashSet::new(),
            usage: Usage::default(),
            live: subagents::Live::default(),
            panes: HashMap::new(),
            resolved: HashMap::new(),
            noted: HashMap::new(),
        }
    }

    pub fn with_usage(mut self, path: PathBuf) -> Self {
        self.usage = Usage::new(path);
        self
    }

    /// For the reconcile tick: one fstat, the recorded plan when it changed since it last said so.
    pub fn plan_moved(&mut self) -> Option<Option<protocol::PlanUsage>> {
        self.usage.plan_moved().then(|| self.usage.plan())
    }

    pub fn plan(&mut self) -> Option<protocol::PlanUsage> {
        self.usage.refresh();
        self.usage.plan()
    }

    pub fn from_env() -> Self {
        let codex = match std::env::var_os("CODEX_HOME") {
            Some(dir) => Some(PathBuf::from(dir)),
            None => crate::config::home_dir().ok().map(|h| h.join(".codex")),
        };
        Self::new(
            crate::hooks::claude_dir().ok().map(|d| d.join("projects")),
            codex.map(|d| d.join("sessions")),
        )
    }

    pub fn retain(&mut self, agents: &[AgentInfo]) {
        let ids: Vec<String> = agents
            .iter()
            .filter_map(|a| self.resolve(a, false))
            .collect();
        let live = |id: &String| ids.contains(id);
        self.cache.retain(|id, _| live(id));
        self.missed.retain(&live);
        self.live.retain(live);
        let open = |t: &String| agents.iter().any(|a| &a.terminal_id == t);
        self.panes.retain(|t, _| open(t));
        self.resolved.retain(|t, _| open(t));
        self.noted.retain(|t, _| open(t));
    }

    /// No subagent running; a Claude Code agent whose transcript cannot be read, or whose
    /// subagent scan left some out, counts as busy.
    pub fn quiet(&mut self, a: &AgentInfo) -> bool {
        a.agent.as_deref() != Some("claude")
            || self
                .derive(a, true)
                .is_some_and(|d| d.subagents.is_empty() && !d.subagents_truncated)
    }

    /// Cached by size and mtime, so an unchanged transcript costs one open and fstat. A
    /// transcript not found is looked for again only when `relocate`.
    pub fn derive(&mut self, a: &AgentInfo, relocate: bool) -> Option<Derived> {
        let kind = match a.agent.as_deref()? {
            "claude" => Kind::Claude,
            "codex" => Kind::Codex,
            _ => return None,
        };
        let id = self.resolve(a, relocate)?;
        if session_id(a) != Some(id.as_str())
            && self
                .noted
                .insert(a.terminal_id.clone(), id.clone())
                .as_ref()
                != Some(&id)
        {
            tracing::info!(
                terminal_id = %a.terminal_id,
                herdr = session_id(a).unwrap_or("none"),
                live = %id,
                "following the live Claude Code session"
            );
        }
        let id = id.as_str();
        let path = match self.cache.get(id) {
            Some(c) => c.path.clone(),
            None if !relocate && self.missed.contains(id) => return None,
            None => match self.locate(kind, id, a) {
                Some(path) => path,
                None => {
                    self.missed.insert(id.to_owned());
                    return None;
                }
            },
        };
        let Some((file, len, modified)) = open(&path) else {
            self.cache.remove(id);
            self.missed.insert(id.to_owned());
            return None;
        };
        self.missed.remove(id);
        let (window, subagents) = match kind {
            Kind::Claude => {
                self.usage.refresh();
                let sessions = self.sessions_dir();
                (
                    self.usage.window(id),
                    self.live.running(sessions.as_deref(), &path, id),
                )
            }
            Kind::Codex => (None, Vec::new()),
        };
        let subagents_truncated = self.live.truncated(id);
        if let Some(c) = self
            .cache
            .get(id)
            .filter(|c| c.len == len && c.modified == modified && c.window == window)
        {
            return Some(Derived {
                subagents,
                subagents_truncated,
                ..c.derived.clone()
            });
        }
        let text = tail(file, len).ok()?;
        let mut derived = match kind {
            Kind::Claude => claude(&text, window),
            Kind::Codex => codex(&text),
        };
        // A line longer than the tail (a large tool result) can hide the older values.
        if let Some(prev) = self.cache.get(id).map(|c| &c.derived) {
            derived.context_left = derived.context_left.or(prev.context_left);
            derived.last_line = derived.last_line.or_else(|| prev.last_line.clone());
            derived.last_prompt = derived.last_prompt.or_else(|| prev.last_prompt.clone());
        }
        derived.subagents = subagents;
        derived.subagents_truncated = subagents_truncated;
        derived.last_activity_ms = modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as u64);
        self.cache.insert(
            id.to_owned(),
            Cached {
                path,
                len,
                modified,
                window,
                derived: derived.clone(),
            },
        );
        Some(derived)
    }

    fn locate(&self, kind: Kind, id: &str, a: &AgentInfo) -> Option<PathBuf> {
        match kind {
            Kind::Claude => {
                let root = self.claude_projects.as_ref()?;
                let name = format!("{id}.jsonl");
                let by_cwd = [&a.foreground_cwd, &a.cwd]
                    .into_iter()
                    .flatten()
                    .map(|cwd| root.join(project_dir(cwd)).join(&name));
                // The session may have started in another directory than the pane is in now.
                let any = std::fs::read_dir(root)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path().join(&name));
                by_cwd.chain(any).find(|p| is_file(p))
            }
            Kind::Codex => self.codex_rollout(id),
        }
    }

    fn codex_rollout(&self, id: &str) -> Option<PathBuf> {
        let root = self.codex_sessions.as_ref()?;
        let day = (uuid_v7_ms(id)? / 86_400_000) as i64;
        let suffix = format!("-{id}.jsonl");
        // Rollouts sit under the local date the session started; the id has it in UTC.
        (day - 1..=day + 1)
            .flat_map(|d| {
                let (y, m, d) = civil(d);
                std::fs::read_dir(root.join(format!("{y:04}/{m:02}/{d:02}")))
                    .into_iter()
                    .flatten()
                    .flatten()
            })
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(&suffix))
                    && is_file(p)
            })
    }

    /// The most recent sessions started in `cwd` (canonical) that have a prompt, newest first.
    pub fn sessions(&self, cwd: &str, claude: bool, codex: bool) -> Vec<Session> {
        let mut out = Vec::new();
        if claude {
            out.extend(self.claude_sessions(cwd));
        }
        if codex {
            out.extend(self.codex_sessions(cwd));
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.modified_ms));
        out.truncate(MAX_SESSIONS);
        out
    }

    /// Whether `id` names a transcript of this kind that belongs to `cwd`.
    pub fn has_session(&self, agent: &str, cwd: &str, id: &str) -> bool {
        if !is_session_id(id) {
            return false;
        }
        match agent {
            "claude" => self.claude_projects.as_ref().is_some_and(|root| {
                open(&root.join(project_dir(cwd)).join(format!("{id}.jsonl"))).is_some()
            }),
            "codex" => self
                .codex_rollout(id)
                .or_else(|| {
                    self.codex_files()
                        .into_iter()
                        .find_map(|(path, i)| (i == id).then_some(path))
                })
                .and_then(|path| open(&path))
                .is_some_and(|(mut file, _, _)| {
                    head(&mut file, META_BYTES)
                        .and_then(|h| codex_meta(&h))
                        .is_some_and(|(i, c, _)| i == id && c == cwd)
                }),
            _ => false,
        }
    }

    fn claude_sessions(&self, cwd: &str) -> Vec<Session> {
        let Some(root) = &self.claude_projects else {
            return Vec::new();
        };
        let files = std::fs::read_dir(root.join(project_dir(cwd)))
            .into_iter()
            .flatten()
            .flatten()
            .take(MAX_LISTED)
            .filter_map(|e| {
                let name = e.file_name();
                let id = name.to_str()?.strip_suffix(".jsonl")?;
                is_session_id(id).then(|| (e.path(), id.to_owned()))
            })
            .collect();
        recent("claude", files, |file, _| {
            let text = head(file, HEAD_BYTES)?;
            Some(claude_first_prompt(&String::from_utf8_lossy(&text)))
        })
    }

    /// Rollouts in the newest dated folders first, at most `MAX_LISTED`.
    fn codex_files(&self) -> Vec<(PathBuf, String)> {
        let Some(root) = &self.codex_sessions else {
            return Vec::new();
        };
        let mut files = Vec::new();
        'walk: for year in dated_dirs(root) {
            for month in dated_dirs(&year) {
                for day in dated_dirs(&month) {
                    for e in std::fs::read_dir(&day).into_iter().flatten().flatten() {
                        if files.len() >= MAX_LISTED {
                            break 'walk;
                        }
                        let name = e.file_name();
                        if let Some(id) = name.to_str().and_then(rollout_id) {
                            files.push((e.path(), id.to_owned()));
                        }
                    }
                }
            }
        }
        files
    }

    fn codex_sessions(&self, cwd: &str) -> Vec<Session> {
        recent("codex", self.codex_files(), |file, id| {
            let (i, c, interactive) = codex_meta(&head(file, META_BYTES)?)?;
            if i != id || c != cwd || !interactive {
                return None;
            }
            file.seek(SeekFrom::Start(0)).ok()?;
            let text = head(file, HEAD_BYTES)?;
            Some(codex_first_prompt(&String::from_utf8_lossy(&text)))
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub agent: &'static str,
    pub id: String,
    pub first_prompt: String,
    pub modified_ms: u64,
}

/// Newest first by mtime; only the `MAX_CANDIDATES` newest are opened. A transcript longer
/// than its head without a prompt found there is listed with an empty one: the prompt may
/// sit in a line the head cuts (an inline image) or after it.
fn recent(
    agent: &'static str,
    files: Vec<(PathBuf, String)>,
    prompt: impl Fn(&mut File, &str) -> Option<Option<String>>,
) -> Vec<Session> {
    let mut dated: Vec<(SystemTime, PathBuf, String)> = files
        .into_iter()
        .filter_map(|(path, id)| {
            let meta = std::fs::symlink_metadata(&path)
                .ok()
                .filter(|m| m.is_file())?;
            Some((meta.modified().ok()?, path, id))
        })
        .collect();
    dated.sort_by_key(|d| std::cmp::Reverse(d.0));
    dated
        .into_iter()
        .take(MAX_CANDIDATES)
        .filter_map(|(_, path, id)| {
            let (mut file, len, modified) = open(&path)?;
            let first_prompt =
                prompt(&mut file, &id)?.or_else(|| (len > HEAD_BYTES).then(String::new))?;
            Some(Session {
                agent,
                modified_ms: modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64,
                id,
                first_prompt,
            })
        })
        .take(MAX_SESSIONS)
        .collect()
}

fn head(file: &mut File, limit: u64) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    file.by_ref().take(limit).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Numbered subfolders (`YYYY`, `MM`, `DD`), newest first, never through a symlink.
fn dated_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                && e.file_type().is_ok_and(|t| t.is_dir())
        })
        .map(|e| e.path())
        .collect();
    dirs.sort_by(|a, b| b.cmp(a));
    dirs
}

fn rollout_id(name: &str) -> Option<&str> {
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    is_session_id(id).then_some(id)
}

/// The rollout's id and cwd from its first line, and whether a person started it (the TUI
/// or the app, not `codex exec` or a subagent).
fn codex_meta(head: &[u8]) -> Option<(String, String, bool)> {
    let first = head.split(|b| *b == b'\n').next()?;
    let v: Value = serde_json::from_slice(first).ok()?;
    if v["type"] != "session_meta" {
        return None;
    }
    let p = &v["payload"];
    Some((
        p["id"].as_str()?.to_owned(),
        p["cwd"].as_str()?.to_owned(),
        matches!(p["source"].as_str(), Some("cli" | "vscode")),
    ))
}

fn first_block_line(content: &Value, kind: &str) -> Option<String> {
    content
        .as_array()?
        .iter()
        .filter(|b| b["type"] == kind)
        .find_map(|b| first_line(b["text"].as_str()?))
}

fn claude_first_prompt(text: &str) -> Option<String> {
    text.lines()
        .filter(|l| l.contains("human"))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find_map(|v| {
            if v["type"] != "user" || v["isSidechain"] == true || v["origin"]["kind"] != "human" {
                return None;
            }
            let content = &v["message"]["content"];
            let local = history::local(&v);
            if local.is_some_and(|s| s.starts_with("<local-command-stdout>")) {
                return None;
            }
            match content.as_str() {
                Some(s) => local.and_then(history::command).or_else(|| first_line(s)),
                None => first_block_line(content, "text"),
            }
        })
}

fn codex_first_prompt(text: &str) -> Option<String> {
    text.lines()
        .filter(|l| l.contains("UserMessage") || l.contains("user_message"))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find_map(|v| {
            let p = &v["payload"];
            if v["type"] != "event_msg" {
                return None;
            }
            match p["type"].as_str()? {
                "item_completed" if p["item"]["type"] == "UserMessage" => {
                    first_block_line(&p["item"]["content"], "text")
                }
                "user_message" => p["message"].as_str().and_then(first_line),
                _ => None,
            }
        })
}

/// The id becomes a file name, so it must be a plain lowercase UUID: no separator, no `..`.
fn session_id(a: &AgentInfo) -> Option<&str> {
    let id = a.agent_session.as_ref()?.value.as_str();
    is_session_id(id).then_some(id)
}

pub(crate) fn is_session_id(id: &str) -> bool {
    id.len() == 36
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
}

/// Claude Code's project directory name for a working directory.
fn project_dir(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn is_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}

/// Never follows a symlink, and never blocks on a FIFO put in the transcript's place.
pub(crate) fn open(path: &Path) -> Option<(File, u64, SystemTime)> {
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags.bits() as i32)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok().filter(|m| m.is_file())?;
    Some((file, meta.len(), meta.modified().ok()?))
}

fn tail(mut file: File, len: u64) -> std::io::Result<String> {
    let start = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut buf)?;
    let whole = if start == 0 {
        &buf[..]
    } else {
        buf.iter()
            .position(|&b| b == b'\n')
            .map_or(&[][..], |i| &buf[i + 1..])
    };
    Ok(String::from_utf8_lossy(whole).into_owned())
}

fn uuid_v7_ms(id: &str) -> Option<u64> {
    if id.as_bytes().get(14) != Some(&b'7') {
        return None;
    }
    u64::from_str_radix(&format!("{}{}", id.get(..8)?, id.get(9..13)?), 16).ok()
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(prompt::clean).find(|l| !l.is_empty())?;
    Some(if line.chars().count() > MAX_TEXT_CHARS {
        line.chars().take(MAX_TEXT_CHARS - 1).chain(['…']).collect()
    } else {
        line
    })
}

fn blocks_line(content: &Value, kind: &str) -> Option<String> {
    content
        .as_array()?
        .iter()
        .rev()
        .filter(|b| b["type"] == kind)
        .find_map(|b| first_line(b["text"].as_str()?))
}

fn done(d: &Derived) -> bool {
    d.context_left.is_some() && d.last_line.is_some() && d.last_prompt.is_some()
}

fn percent(remaining: u64, window: u64) -> u8 {
    ((remaining.min(window) * 100 + window / 2) / window) as u8
}

/// The transcript does not record the window; without the status line's, this follows
/// Claude Code's model table.
fn claude_window(model: &str, used: u64) -> u64 {
    let mut parts = model.strip_prefix("claude-").unwrap_or(model).split('-');
    let family = parts.next().unwrap_or_default();
    let mut version = parts.map(|p| p.parse::<u32>().ok().filter(|n| *n < 100));
    let major = version.next().flatten().unwrap_or(0);
    let minor = version.next().flatten().unwrap_or(0);
    let long = match family {
        "fable" => true,
        "opus" => (major, minor) >= (4, 7),
        "sonnet" => major >= 5,
        _ => false,
    };
    if long || used > CLAUDE_WINDOW {
        CLAUDE_LONG_WINDOW
    } else {
        CLAUDE_WINDOW
    }
}

fn claude(text: &str, known_window: Option<u64>) -> Derived {
    let mut d = Derived::default();
    let mut compacted = None;
    let lines = text
        .lines()
        .rev()
        .filter(|l| {
            l.contains("assistant")
                || l.contains("human")
                || l.contains("last-prompt")
                || l.contains("compact_boundary")
        })
        .filter_map(|l| serde_json::from_str::<Value>(l).ok());
    for v in lines {
        if done(&d) {
            break;
        }
        let main = v["isSidechain"] != true;
        match v["type"].as_str() {
            // No assistant usage follows a compaction until the next reply: its size is here.
            Some("system")
                if main
                    && v["subtype"] == "compact_boundary"
                    && d.context_left.is_none()
                    && compacted.is_none() =>
            {
                compacted = Some(v["compactMetadata"]["postTokens"].as_u64().unwrap_or(0));
            }
            Some("assistant") if main && v["message"]["model"] != "<synthetic>" => {
                let m = &v["message"];
                let u = &m["usage"];
                if d.context_left.is_none()
                    && let Some(input) = u["input_tokens"].as_u64()
                {
                    let seen = input
                        + u["cache_creation_input_tokens"].as_u64().unwrap_or(0)
                        + u["cache_read_input_tokens"].as_u64().unwrap_or(0);
                    let window = known_window.unwrap_or_else(|| {
                        claude_window(m["model"].as_str().unwrap_or_default(), seen)
                    });
                    let used = compacted.unwrap_or(seen);
                    d.context_left = Some(percent(window.saturating_sub(used), window));
                }
                if d.last_line.is_none() {
                    d.last_line = blocks_line(&m["content"], "text");
                }
            }
            Some("user") if main && d.last_prompt.is_none() && v["origin"]["kind"] == "human" => {
                let content = &v["message"]["content"];
                let local = history::local(&v);
                if local.is_some_and(|s| s.starts_with("<local-command-stdout>")) {
                    continue;
                }
                d.last_prompt = match content.as_str() {
                    Some(s) => local.and_then(history::command).or_else(|| first_line(s)),
                    None => blocks_line(content, "text"),
                };
            }
            Some("last-prompt") if d.last_prompt.is_none() => {
                d.last_prompt = v["lastPrompt"].as_str().and_then(first_line);
            }
            _ => {}
        }
    }
    d
}

fn codex_left(used: u64, window: u64) -> u8 {
    if window <= CODEX_BASELINE {
        return 0;
    }
    let effective = window - CODEX_BASELINE;
    let used = used.saturating_sub(CODEX_BASELINE);
    percent(effective.saturating_sub(used), effective)
}

fn codex(text: &str) -> Derived {
    let mut d = Derived::default();
    let lines = text
        .lines()
        .rev()
        .filter(|l| l.contains("token_count") || l.contains("item_completed"))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok());
    for v in lines {
        if done(&d) {
            break;
        }
        let p = &v["payload"];
        if v["type"] != "event_msg" {
            continue;
        }
        match p["type"].as_str() {
            Some("token_count") if d.context_left.is_none() => {
                let info = &p["info"];
                if let (Some(used), Some(window)) = (
                    info["last_token_usage"]["total_tokens"].as_u64(),
                    info["model_context_window"].as_u64(),
                ) {
                    d.context_left = Some(codex_left(used, window));
                }
            }
            Some("item_completed") => {
                let item = &p["item"];
                match item["type"].as_str() {
                    Some("AgentMessage") if d.last_line.is_none() => {
                        d.last_line = blocks_line(&item["content"], "Text");
                    }
                    Some("UserMessage") if d.last_prompt.is_none() => {
                        d.last_prompt = blocks_line(&item["content"], "text");
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::herdr::AgentSession;

    const CLAUDE: &str = include_str!("../tests/fixtures/transcripts/claude.jsonl");
    const CODEX: &str = include_str!("../tests/fixtures/transcripts/codex.jsonl");
    const CLAUDE_ID: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    const CODEX_ID: &str = "01a10f07-b5ed-79a3-989d-fa10b96c5cab";

    fn agent(kind: &str, id: &str, cwd: &str) -> AgentInfo {
        AgentInfo {
            terminal_id: "term_1".into(),
            workspace_id: "w1".into(),
            pane_id: "w1:p1".into(),
            agent: Some(kind.into()),
            name: None,
            title: None,
            terminal_title_stripped: None,
            agent_status: "done".into(),
            state_change_seq: 1,
            cwd: Some(cwd.into()),
            foreground_cwd: None,
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

    fn roots(dir: &Path) -> Transcripts {
        Transcripts::new(
            Some(dir.join("claude/projects")),
            Some(dir.join("codex/sessions")),
        )
    }

    #[test]
    fn claude_fixture() {
        let d = claude(CLAUDE, None);
        // 2 + 429 + 54811 input tokens of a 1M window; the sidechain and synthetic lines are skipped.
        assert_eq!(d.context_left, Some(94));
        assert_eq!(d.last_line.as_deref(), Some("**Fixed** the flaky test."));
        assert_eq!(
            d.last_prompt.as_deref(),
            Some("fix the flaky approval test it fails one run in ten")
        );
        let sidechain = r#"{"isSidechain":true,"type":"assistant","message":{"model":"claude-opus-5-5","content":[{"type":"text","text":"Subagent done."}],"usage":{"input_tokens":900000}}}"#;
        let d = claude(&[CLAUDE, sidechain].join("\n"), None);
        assert_eq!(
            (d.context_left, d.last_line.as_deref()),
            (Some(94), Some("**Fixed** the flaky test."))
        );
    }

    #[test]
    fn claude_compaction_sets_the_context_until_the_next_reply() {
        let boundary = r#"{"isSidechain":false,"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":"manual","preTokens":749477,"postTokens":20000}}"#;
        let summary = r#"{"isSidechain":false,"type":"user","isCompactSummary":true,"message":{"role":"user","content":"This session is being continued from a previous conversation."}}"#;
        let d = claude(&[CLAUDE, boundary, summary].join("\n"), None);
        assert_eq!(d.context_left, Some(98));
        assert_eq!(d.last_line.as_deref(), Some("**Fixed** the flaky test."));
        let reply = r#"{"isSidechain":false,"type":"assistant","message":{"model":"claude-opus-5-5","content":[{"type":"text","text":"Back."}],"usage":{"input_tokens":100000}}}"#;
        assert_eq!(
            claude(&[CLAUDE, boundary, summary, reply].join("\n"), None).context_left,
            Some(90)
        );
    }

    #[test]
    fn claude_prompt_is_the_later_of_entry_and_last_prompt() {
        let human = r#"{"type":"user","isSidechain":false,"origin":{"kind":"human"},"message":{"role":"user","content":[{"type":"text","text":"  ship it\nnow"},{"type":"image"}]}}"#;
        let tool = r#"{"type":"user","isSidechain":false,"message":{"role":"user","content":[{"type":"tool_result","content":"human"}]}}"#;
        let notice = r#"{"type":"user","isSidechain":false,"origin":{"kind":"task-notification"},"message":{"role":"user","content":"<task-notification>"}}"#;
        let summary = r#"{"type":"last-prompt","lastPrompt":"older"}"#;
        assert_eq!(
            claude(&[summary, human, tool, notice].join("\n"), None)
                .last_prompt
                .as_deref(),
            Some("ship it")
        );
        assert_eq!(
            claude(&[human, summary].join("\n"), None)
                .last_prompt
                .as_deref(),
            Some("older")
        );
    }

    #[test]
    fn claude_slash_commands_give_their_marker_and_output_never_becomes_the_prompt() {
        let human = r#"{"type":"user","isSidechain":false,"origin":{"kind":"human"},"promptSource":"typed","message":{"role":"user","content":"ship it"}}"#;
        let skill = r#"{"type":"user","isSidechain":false,"origin":{"kind":"human"},"message":{"role":"user","content":"<command-message>review</command-message>\n<command-name>/review</command-name>\n<command-args>pr\u001b 12</command-args>"}}"#;
        let builtin = r#"{"type":"user","isSidechain":false,"message":{"role":"user","content":"<command-name>/exit</command-name>\n<command-message>exit</command-message>\n<command-args></command-args>"}}"#;
        let stdout = r#"{"type":"user","isSidechain":false,"origin":{"kind":"human"},"message":{"role":"user","content":"<local-command-stdout>human</local-command-stdout>"}}"#;
        let typed = r#"{"type":"user","isSidechain":false,"origin":{"kind":"human"},"promptSource":"typed","message":{"role":"user","content":"<command-name>/exit</command-name> typed"}}"#;
        let prompt = |lines: &[&str]| claude(&lines.join("\n"), None).last_prompt;
        assert_eq!(prompt(&[human, skill]).as_deref(), Some("/review pr 12"));
        assert_eq!(
            prompt(&[human, builtin, stdout]).as_deref(),
            Some("ship it")
        );
        assert_eq!(
            prompt(&[skill, typed]).as_deref(),
            Some("<command-name>/exit</command-name> typed")
        );
    }

    #[test]
    fn claude_first_prompt_gives_a_slash_command_marker_and_skips_its_output() {
        let builtin = r#"{"type":"user","isSidechain":false,"message":{"role":"user","content":"<command-name>/effort</command-name>\n<command-message>effort</command-message>\n<command-args>human</command-args>"}}"#;
        let stdout = r#"{"type":"user","isSidechain":false,"origin":{"kind":"human"},"message":{"role":"user","content":"<local-command-stdout>human</local-command-stdout>"}}"#;
        let skill = r#"{"type":"user","isSidechain":false,"origin":{"kind":"human"},"message":{"role":"user","content":"<command-message>review</command-message>\n<command-name>/review</command-name>\n<command-args>pr\u001b 12</command-args>"}}"#;
        let human = r#"{"type":"user","isSidechain":false,"origin":{"kind":"human"},"promptSource":"typed","message":{"role":"user","content":"ship it"}}"#;
        let typed = r#"{"type":"user","isSidechain":false,"origin":{"kind":"human"},"promptSource":"typed","message":{"role":"user","content":"<command-name>/exit</command-name> typed"}}"#;
        let first = |lines: &[&str]| claude_first_prompt(&lines.join("\n"));
        assert_eq!(
            first(&[builtin, stdout, skill, human]).as_deref(),
            Some("/review pr 12")
        );
        assert_eq!(first(&[builtin, stdout, human]).as_deref(), Some("ship it"));
        assert_eq!(
            first(&[typed, skill]).as_deref(),
            Some("<command-name>/exit</command-name> typed")
        );
    }

    #[test]
    fn claude_windows() {
        assert_eq!(claude_window("claude-opus-5-5", 1), CLAUDE_LONG_WINDOW);
        assert_eq!(claude_window("claude-opus-5", 1), CLAUDE_LONG_WINDOW);
        assert_eq!(claude_window("claude-opus-4-7", 1), CLAUDE_LONG_WINDOW);
        assert_eq!(claude_window("claude-opus-4-6", 1), CLAUDE_WINDOW);
        assert_eq!(claude_window("claude-sonnet-5-5", 1), CLAUDE_LONG_WINDOW);
        assert_eq!(
            claude_window("claude-sonnet-4-5-20250929", 1),
            CLAUDE_WINDOW
        );
        assert_eq!(claude_window("claude-fable-5-1", 1), CLAUDE_LONG_WINDOW);
        assert_eq!(claude_window("claude-haiku-4-5", 1), CLAUDE_WINDOW);
        assert_eq!(
            claude_window("claude-sonnet-4-6", 250_000),
            CLAUDE_LONG_WINDOW
        );
        let line = |used: u64| {
            format!(
                r#"{{"type":"assistant","isSidechain":false,"message":{{"model":"claude-haiku-4-5","content":[],"usage":{{"input_tokens":{used},"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}}}}"#
            )
        };
        assert_eq!(claude(&line(50_000), None).context_left, Some(75));
        assert_eq!(claude(&line(400_000), None).context_left, Some(60));
        assert_eq!(claude(&line(2_000_000), None).context_left, Some(0));
    }

    #[test]
    fn codex_fixture() {
        let d = codex(CODEX);
        assert_eq!(d.context_left, Some(82));
        assert_eq!(
            d.last_line.as_deref(),
            Some("Added three approval tests for the Codex menu.")
        );
        assert_eq!(
            d.last_prompt.as_deref(),
            Some("add approval tests for the codex menu")
        );
        // Codex counts reasoning tokens as in the window (codex-rs 0.160.1 `tokens_in_context_window`).
        let reasoning = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100000,"output_tokens":100000,"reasoning_output_tokens":100000,"total_tokens":200000},"model_context_window":258400}}}"#;
        assert_eq!(codex(reasoning).context_left, Some(24));
        assert_eq!(codex_left(12_000, 258_400), 100);
        assert_eq!(codex_left(300_000, 258_400), 0);
        assert_eq!(codex_left(1, 10_000), 0);
    }

    #[test]
    fn text_is_cleaned_and_capped() {
        assert_eq!(
            first_line("\n \x1b[2J\u{202e}hi\tthere \n"),
            Some("[2Jhi there".into())
        );
        let long = first_line(&"é".repeat(500)).unwrap();
        assert_eq!(long.chars().count(), MAX_TEXT_CHARS);
        assert!(long.ends_with('…'));
        assert_eq!(first_line("  \n\t"), None);
    }

    #[test]
    fn session_ids_are_plain_uuids() {
        assert_eq!(
            session_id(&agent("claude", CLAUDE_ID, "/")),
            Some(CLAUDE_ID)
        );
        for bad in [
            "",
            "sess-1",
            "../../../../../../../../../../etc/pa",
            "3F1B2C4D-5E6F-4A7B-8C9D-0E1F2A3B4C5D",
            "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d0",
            "3f1b2c4d/5e6f-4a7b-8c9d-0e1f2a3b4c5d",
        ] {
            assert_eq!(session_id(&agent("claude", bad, "/")), None, "{bad}");
        }
    }

    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(19_782), (2024, 2, 29));
        let ms = uuid_v7_ms(CODEX_ID).unwrap();
        assert_eq!(civil((ms / 86_400_000) as i64), (2026, 10, 6));
        assert_eq!(uuid_v7_ms(CLAUDE_ID), None);
    }

    #[test]
    fn finds_and_caches_transcripts() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = roots(dir.path());
        let project = dir.path().join("claude/projects/-Users-me-src-collie");
        write(&project.join(format!("{CLAUDE_ID}.jsonl")), CLAUDE);
        let a = agent("claude", CLAUDE_ID, "/Users/me/src/collie");
        let d = t.derive(&a, false).unwrap();
        assert_eq!(d.context_left, Some(94));
        assert!(d.last_activity_ms.unwrap() > 0);

        // Started elsewhere: found by name under any project.
        let moved = agent("claude", CLAUDE_ID, "/Users/me/src/other");
        t.cache.clear();
        assert_eq!(t.derive(&moved, false).unwrap().context_left, Some(94));

        let rollout = dir.path().join(format!(
            "codex/sessions/2026/10/05/rollout-2026-10-05T22-25-26-{CODEX_ID}.jsonl"
        ));
        write(&rollout, CODEX);
        let c = agent("codex", CODEX_ID, "/Users/me/src/collie");
        assert_eq!(t.derive(&c, false).unwrap().context_left, Some(82));

        assert_eq!(t.derive(&agent("copilot", CLAUDE_ID, "/"), false), None);
        assert_eq!(
            t.derive(
                &agent("claude", "11111111-2222-4333-8444-555555555555", "/"),
                false
            ),
            None
        );

        // Not found yet: looked for again only on a status change.
        let late_id = "22222222-3333-4444-8555-666666666666";
        let late = agent("claude", late_id, "/Users/me/src/collie");
        assert_eq!(t.derive(&late, false), None);
        write(&project.join(format!("{late_id}.jsonl")), CLAUDE);
        assert_eq!(t.derive(&late, false), None);
        assert_eq!(t.derive(&late, true).unwrap().context_left, Some(94));

        // Appending changes the size, so the cache is read again.
        let mut more = CLAUDE.to_owned();
        more.push_str(r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5-5","content":[{"type":"text","text":"Pushed."}],"usage":{"input_tokens":500000}}}"#);
        more.push('\n');
        write(&project.join(format!("{CLAUDE_ID}.jsonl")), &more);
        let d = t.derive(&a, false).unwrap();
        assert_eq!(
            (d.context_left, d.last_line.as_deref()),
            (Some(50), Some("Pushed."))
        );

        // A tool result longer than the tail hides the reply and prompt: the last ones are kept.
        more.push_str(&format!(
            r#"{{"type":"user","isSidechain":false,"message":{{"role":"user","content":[{{"type":"tool_result","content":"{}"}}]}}}}"#,
            "x".repeat(TAIL_BYTES as usize)
        ));
        more.push('\n');
        more.push_str(r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5-5","content":[{"type":"tool_use","name":"Bash"}],"usage":{"input_tokens":600000}}}"#);
        more.push('\n');
        write(&project.join(format!("{CLAUDE_ID}.jsonl")), &more);
        let d = t.derive(&a, false).unwrap();
        assert_eq!(
            (d.context_left, d.last_line.as_deref()),
            (Some(40), Some("Pushed."))
        );
        assert!(d.last_prompt.unwrap().starts_with("fix the flaky"));

        t.retain(&[c]);
        assert_eq!(t.cache.len(), 1);
        assert!(t.cache.contains_key(CODEX_ID));
        assert!(t.missed.is_empty());
    }

    #[test]
    fn subagents_refresh_while_the_main_transcript_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = roots(dir.path());
        let project = dir.path().join("claude/projects/-Users-me-src-collie");
        write(&project.join(format!("{CLAUDE_ID}.jsonl")), CLAUDE);
        let a = agent("claude", CLAUDE_ID, "/Users/me/src/collie");
        assert!(!t.quiet(&agent(
            "claude",
            "9f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d",
            "/Users/me/src/collie"
        )));
        assert!(t.quiet(&agent(
            "copilot",
            "9f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d",
            "/Users/me/src/collie"
        )));
        assert!(t.derive(&a, false).unwrap().subagents.is_empty());
        assert!(t.quiet(&a));
        let sub = project.join(CLAUDE_ID).join("subagents");
        let lines = include_str!("../tests/fixtures/transcripts/claude-live-agent.jsonl")
            .split_inclusive('\n')
            .collect::<Vec<_>>();
        write(
            &sub.join("agent-a1.meta.json"),
            include_str!("../tests/fixtures/transcripts/claude-live-agent.meta.json"),
        );
        write(&sub.join("agent-a1.jsonl"), &lines[..4].concat());
        assert!(t.derive(&a, false).unwrap().subagents.is_empty());
        let pid = std::process::id();
        write(
            &dir.path().join(format!("claude/sessions/{pid}.json")),
            &format!(
                r#"{{"pid":{pid},"sessionId":"{CLAUDE_ID}","startedAt":{}}}"#,
                crate::now_ms() - 60_000
            ),
        );
        let d = t.derive(&a, false).unwrap();
        assert_eq!(d.subagents.len(), 1);
        assert_eq!(d.subagents[0].activity, "Reading pairing.rs");
        assert!(!t.quiet(&a));
        write(&sub.join("agent-a1.jsonl"), &lines.concat());
        assert!(t.derive(&a, false).unwrap().subagents.is_empty());
        assert!(t.quiet(&a));
        for n in 0..9 {
            std::fs::create_dir_all(sub.join(format!("workflows/wf_{n}"))).unwrap();
        }
        assert!(t.derive(&a, false).unwrap().subagents.is_empty());
        assert!(!t.quiet(&a), "a truncated scan counts as busy");
    }

    #[test]
    fn status_line_window_and_plan_usage() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("collie");
        let mut t = roots(dir.path()).with_usage(data.join(crate::usage::USAGE_FILE));
        let project = dir.path().join("claude/projects/-Users-me-src-collie");
        write(&project.join(format!("{CLAUDE_ID}.jsonl")), CLAUDE);
        let a = agent("claude", CLAUDE_ID, "/Users/me/src/collie");
        assert_eq!(t.derive(&a, false).unwrap().context_left, Some(94));
        assert_eq!((t.plan(), t.plan_moved()), (None, None));

        // The status line says this session's window is 200K, not the model's 1M.
        let input = include_str!("../tests/fixtures/statusline.json").replace(
            r#""context_window_size": 1000000"#,
            r#""context_window_size": 200000"#,
        );
        crate::usage::record(&data, input.as_bytes(), 7).unwrap();
        assert_eq!(t.derive(&a, false).unwrap().context_left, Some(72));
        let plan = t.plan_moved().unwrap().unwrap();
        assert_eq!(plan.recorded_ms, 7);
        assert_eq!(plan.five_hour.as_ref().unwrap().used_percent, 24);
        assert_eq!((t.plan_moved(), t.plan()), (None, Some(plan)));

        let c = agent("codex", CODEX_ID, "/Users/me/src/collie");
        assert_eq!(t.derive(&c, false), None);
    }

    #[test]
    fn codex_only_plan_reports_its_removal() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("collie");
        let mut t = roots(dir.path()).with_usage(data.join(crate::usage::USAGE_FILE));
        crate::usage::record_codex(
            &data,
            Some(protocol::CodexUsage {
                used: 100,
                limit: 1_000,
                resets_at_ms: 1_000_000,
                recorded_ms: 10,
            }),
        )
        .unwrap();
        assert!(t.plan_moved().unwrap().is_some());
        crate::usage::record_codex(&data, None).unwrap();
        assert_eq!(t.plan_moved(), Some(None));
    }

    #[test]
    fn refuses_symlinks_and_reads_only_the_tail() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = roots(dir.path());
        let real = dir.path().join("secret.jsonl");
        write(&real, CLAUDE);
        let project = dir.path().join("claude/projects/-");
        std::fs::create_dir_all(&project).unwrap();
        std::os::unix::fs::symlink(&real, project.join(format!("{CLAUDE_ID}.jsonl"))).unwrap();
        assert_eq!(t.derive(&agent("claude", CLAUDE_ID, "/"), true), None);
        assert!(open(&project.join(format!("{CLAUDE_ID}.jsonl"))).is_none());

        let big = dir.path().join("big.jsonl");
        let filler = format!(
            "{{\"type\":\"assistant\",\"x\":\"{}\"}}\n",
            "a".repeat(1000)
        );
        let mut text = filler.repeat(1200);
        text.push_str(CLAUDE);
        write(&big, &text);
        let (file, len, _) = open(&big).unwrap();
        let tail = tail(file, len).unwrap();
        assert!(tail.len() < TAIL_BYTES as usize && tail.starts_with('{'));
        assert_eq!(claude(&tail, None).context_left, Some(94));
    }

    fn aged(path: &Path, text: &str, secs: u64) {
        write(path, text);
        let at = UNIX_EPOCH + std::time::Duration::from_secs(1_790_000_000 + secs);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    fn uuid(n: u32) -> String {
        format!("{n:08x}-1111-4222-8333-444444444444")
    }

    #[test]
    fn lists_the_folders_recent_sessions_with_their_first_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let t = roots(dir.path());
        let cwd = "/Users/me/src/collie";
        let project = dir.path().join("claude/projects/-Users-me-src-collie");
        aged(&project.join(format!("{CLAUDE_ID}.jsonl")), CLAUDE, 10);
        let no_prompt = r#"{"type":"mode","mode":"normal"}"#;
        aged(&project.join(format!("{}.jsonl", uuid(1))), no_prompt, 50);
        aged(&project.join("notes.jsonl"), CLAUDE, 50);
        aged(&dir.path().join("secret.jsonl"), CLAUDE, 50);
        std::os::unix::fs::symlink(
            dir.path().join("secret.jsonl"),
            project.join(format!("{}.jsonl", uuid(2))),
        )
        .unwrap();
        aged(
            &dir.path()
                .join("claude/projects/-Users-me-src-other")
                .join(format!("{}.jsonl", uuid(3))),
            CLAUDE,
            50,
        );

        let day = dir.path().join("codex/sessions/2026/10/05");
        let rollout = |id: &str| day.join(format!("rollout-2026-10-05T22-25-26-{id}.jsonl"));
        aged(&rollout(CODEX_ID), CODEX, 20);
        let first = CODEX.lines().next().unwrap();
        let variant = |from: &str, to: &str| CODEX.replacen(first, &first.replace(from, to), 1);
        for (n, text) in [
            (4, variant("/Users/me/src/collie", "/Users/me/src/other")),
            (5, variant(r#""source":"cli""#, r#""source":"exec""#)),
            (6, variant(r#""id":"01a10f07"#, r#""id":"01a10f08"#)),
        ] {
            let id = CODEX_ID.replacen("01a10f07", &format!("0{n}a10f07"), 1);
            aged(&rollout(&id), &text.replace(CODEX_ID, &id), 60);
        }
        let legacy = format!(
            r#"{{"type":"session_meta","payload":{{"id":"{}","cwd":"{cwd}","source":"vscode"}}}}
{{"type":"event_msg","payload":{{"type":"user_message","message":"  ship the\nrelease"}}}}"#,
            uuid(7)
        );
        aged(
            &dir.path().join(format!(
                "codex/sessions/2025/01/31/rollout-x-{}.jsonl",
                uuid(7)
            )),
            &legacy,
            5,
        );

        let listed = |claude, codex| -> Vec<(&str, String, String, u64)> {
            t.sessions(cwd, claude, codex)
                .into_iter()
                .map(|s| (s.agent, s.id, s.first_prompt, s.modified_ms))
                .collect()
        };
        let ms = |secs: u64| (1_790_000_000 + secs) * 1000;
        assert_eq!(
            listed(true, true),
            [
                (
                    "codex",
                    CODEX_ID.to_owned(),
                    "add approval tests for the codex menu".to_owned(),
                    ms(20)
                ),
                (
                    "claude",
                    CLAUDE_ID.to_owned(),
                    "fix the flaky approval test".to_owned(),
                    ms(10)
                ),
                ("codex", uuid(7), "ship the".to_owned(), ms(5)),
            ]
        );
        assert_eq!(listed(true, false).len(), 1);
        assert_eq!(listed(false, true).len(), 2);
        assert!(t.sessions("/Users/me/src/elsewhere", true, true).is_empty());

        for n in 10..40 {
            aged(
                &project.join(format!("{}.jsonl", uuid(n))),
                CLAUDE,
                100 + u64::from(n),
            );
        }
        let many = listed(true, true);
        assert_eq!(many.len(), MAX_SESSIONS);
        assert_eq!(many[0].1, uuid(39));
        assert!(many.windows(2).all(|w| w[0].3 >= w[1].3));

        assert!(t.has_session("claude", cwd, CLAUDE_ID));
        assert!(t.has_session("codex", cwd, CODEX_ID));
        assert!(
            t.has_session("codex", cwd, &uuid(7)),
            "not a UUIDv7: found by scanning"
        );
        assert!(!t.has_session("claude", "/Users/me/src/other", CLAUDE_ID));
        assert!(!t.has_session("codex", "/Users/me/src/other", CODEX_ID));
        assert!(!t.has_session("codex", cwd, CLAUDE_ID));
        assert!(!t.has_session("claude", cwd, &uuid(2)));
        assert!(!t.has_session("copilot", cwd, CLAUDE_ID));
        assert!(!t.has_session("claude", cwd, "../-Users-me-src-other/x"));
    }

    #[test]
    fn a_prompt_past_or_across_the_head_lists_the_session_without_its_text() {
        let dir = tempfile::tempdir().unwrap();
        let t = roots(dir.path());
        let cwd = "/Users/me/src/collie";
        let project = dir.path().join("claude/projects/-Users-me-src-collie");
        let big = "A".repeat(HEAD_BYTES as usize + 1000);
        let human = |text: &str| {
            format!(
                r#"{{"type":"user","isSidechain":false,"origin":{{"kind":"human"}},"message":{{"role":"user","content":[{{"type":"image","source":{{"data":"{big}"}}}},{{"type":"text","text":"{text}"}}]}}}}"#
            )
        };
        let late = format!(
            "{{\"type\":\"attachment\",\"data\":\"{big}\"}}\n{}",
            r#"{"type":"user","isSidechain":false,"origin":{"kind":"human"},"message":{"role":"user","content":"look later"}}"#
        );
        aged(
            &project.join(format!("{}.jsonl", uuid(1))),
            &human("look at this"),
            30,
        );
        aged(&project.join(format!("{}.jsonl", uuid(2))), &late, 20);

        let day = dir.path().join("codex/sessions/2026/10/05");
        let meta = |id: &str, source: &str| {
            format!(
                r#"{{"type":"session_meta","payload":{{"id":"{id}","cwd":"{cwd}","source":"{source}"}}}}"#
            )
        };
        let image = format!(
            r#"{{"type":"event_msg","payload":{{"type":"item_completed","item":{{"type":"UserMessage","content":[{{"type":"image","url":"data:image/png;base64,{big}"}},{{"type":"text","text":"fix it"}}]}}}}}}"#
        );
        for (n, source) in [(3, "cli"), (4, "exec")] {
            aged(
                &day.join(format!("rollout-x-{}.jsonl", uuid(n))),
                &format!("{}\n{image}\n", meta(&uuid(n), source)),
                10,
            );
        }

        let listed: Vec<(&str, String, String)> = t
            .sessions(cwd, true, true)
            .into_iter()
            .map(|s| (s.agent, s.id, s.first_prompt))
            .collect();
        assert_eq!(
            listed,
            [
                ("claude", uuid(1), String::new()),
                ("claude", uuid(2), String::new()),
                ("codex", uuid(3), String::new()),
            ]
        );
    }
}
