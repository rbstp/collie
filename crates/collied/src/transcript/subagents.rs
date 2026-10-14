use std::collections::HashMap;
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use protocol::Subagent;
use serde_json::Value;

use super::history::{agent_id, subagent_dir};
use super::open;
use crate::prompt;

const TAIL_BYTES: u64 = 64 * 1024;
/// Growth since the last read is read whole up to this, else only the tail.
const GROWTH_BYTES: u64 = 512 * 1024;
const MAX_FILES: usize = 8;
const ACTIVITY_CHARS: usize = 120;
const TYPE_CHARS: usize = 64;
/// Calls that end a subagent's run: a hand-back, or a workflow agent's result.
const ENDS: [&str; 2] = ["SubagentHandback", "StructuredOutput"];

#[derive(Clone, Default)]
struct Seen {
    len: u64,
    modified: Option<SystemTime>,
    agent_type: String,
    description: String,
    finished: bool,
    activity: Option<String>,
    started_ms: Option<u64>,
    /// The latest input, cache creation and cache read tokens.
    input: u64,
    /// This run's output tokens, counting records of one message once.
    output: u64,
    message: Option<(String, u64)>,
    called_tools: bool,
}

/// A `sessions/<pid>.json` file Claude Code keeps while that process runs.
struct Process {
    len: u64,
    modified: SystemTime,
    session: Option<(String, i32, u64)>,
}

/// Subagents by session id, then by path under `subagents/`; Claude Code processes by file name.
#[derive(Default)]
pub struct Live {
    seen: HashMap<String, HashMap<String, Seen>>,
    processes: HashMap<String, Process>,
}

impl Live {
    pub fn retain(&mut self, live: impl Fn(&String) -> bool) {
        self.seen.retain(|id, _| live(id));
    }

    /// Only for a live Claude Code process of this session, and only files written since it
    /// started. A file whose size and mtime did not move is not opened; at most `MAX_FILES`
    /// others are, newest first, and read from where the last read ended or only their tail.
    pub fn running(
        &mut self,
        sessions: Option<&Path>,
        transcript: &Path,
        session: &str,
    ) -> Vec<Subagent> {
        let started = sessions.and_then(|d| self.process_start(d, session));
        let (Some(dir), Some(started)) = (subagent_dir(transcript, session), started) else {
            self.seen.remove(session);
            return Vec::new();
        };
        let since = UNIX_EPOCH + Duration::from_millis(started);
        let mut files = agent_files(&dir, "");
        let workflows = dir.join("workflows");
        let real = std::fs::symlink_metadata(&workflows).is_ok_and(|m| m.is_dir());
        let mut flows = entries(&workflows)
            .filter(|(name, meta)| real && meta.is_dir() && workflow_id(name))
            .filter_map(|(name, meta)| Some((meta.modified().ok()?, name)))
            .collect::<Vec<_>>();
        flows.sort_by_key(|f| std::cmp::Reverse(f.0));
        for (_, flow) in flows.into_iter().take(MAX_FILES) {
            files.extend(agent_files(&workflows.join(&flow), &format!("{flow}/")));
        }
        files.retain(|f| f.1 >= since);
        files.sort_by_key(|f| std::cmp::Reverse(f.1));
        let old = self.seen.remove(session).unwrap_or_default();
        let mut seen = HashMap::new();
        let mut opened = 0;
        for (key, modified, len, path) in files {
            let prev = old.get(&key);
            if let Some(p) = prev.filter(|p| p.len == len && p.modified == Some(modified)) {
                seen.insert(key, p.clone());
                continue;
            }
            if opened == MAX_FILES {
                if let Some(p) = prev {
                    seen.insert(key, p.clone());
                }
                continue;
            }
            opened += 1;
            if let Some(s) = read(&path, session, prev) {
                seen.insert(key, s);
            }
        }
        let mut out = seen
            .values()
            .filter(|s| !s.finished)
            .filter_map(|s| {
                Some(Subagent {
                    agent_type: s.agent_type.clone(),
                    activity: s.activity.clone().unwrap_or_else(|| s.description.clone()),
                    started_at_ms: s.started_ms?,
                    tokens: (s.input + s.output + 500) / 1000 * 1000,
                    called_tools: s.called_tools,
                })
            })
            .collect::<Vec<_>>();
        out.sort_by(|a, b| (a.started_at_ms, &a.activity).cmp(&(b.started_at_ms, &b.activity)));
        self.seen.insert(session.to_owned(), seen);
        out
    }

    /// The latest start of a running Claude Code process that reports this session.
    fn process_start(&mut self, sessions: &Path, session: &str) -> Option<u64> {
        if !std::fs::symlink_metadata(sessions).ok()?.is_dir() {
            return None;
        }
        let mut next = HashMap::new();
        for (name, meta) in entries(sessions) {
            let Some(pid) = name
                .strip_suffix(".json")
                .filter(|p| p.bytes().all(|b| b.is_ascii_digit()))
            else {
                continue;
            };
            let (Ok(modified), true) = (meta.modified(), meta.is_file()) else {
                continue;
            };
            let process = match self.processes.remove(&name) {
                Some(p) if p.len == meta.len() && p.modified == modified => p,
                _ => Process {
                    len: meta.len(),
                    modified,
                    session: process(&sessions.join(&name), pid),
                },
            };
            next.insert(name, process);
        }
        self.processes = next;
        self.processes
            .values()
            .filter_map(|p| p.session.as_ref())
            .filter(|(id, pid, _)| id == session && alive(*pid))
            .map(|(_, _, started)| *started)
            .max()
    }
}

fn entries(dir: &Path) -> impl Iterator<Item = (String, std::fs::Metadata)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.metadata().ok()?)))
}

/// `(key, mtime, size, path)` of each `agent-<id>.jsonl` that is a regular file.
fn agent_files(dir: &Path, prefix: &str) -> Vec<(String, SystemTime, u64, PathBuf)> {
    entries(dir)
        .filter_map(|(name, meta)| {
            let agent = name.strip_prefix("agent-")?.strip_suffix(".jsonl")?;
            (agent_id(agent) && meta.is_file()).then_some(())?;
            Some((
                format!("{prefix}{agent}"),
                meta.modified().ok()?,
                meta.len(),
                dir.join(&name),
            ))
        })
        .collect()
}

fn workflow_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && !name.starts_with('-')
}

fn process(path: &Path, pid: &str) -> Option<(String, i32, u64)> {
    let (file, _, _) = open(path)?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes).ok()?;
    let v = serde_json::from_slice::<Value>(&bytes).ok()?;
    let pid = pid
        .parse::<i32>()
        .ok()
        .filter(|p| *p > 0 && v["pid"] == *p)?;
    Some((
        v["sessionId"].as_str()?.to_owned(),
        pid,
        v["startedAt"].as_u64()?,
    ))
}

fn alive(pid: i32) -> bool {
    rustix::process::Pid::from_raw(pid).is_some_and(|p| {
        !matches!(
            rustix::process::test_kill_process(p),
            Err(rustix::io::Errno::SRCH)
        )
    })
}

fn read(path: &Path, session: &str, prev: Option<&Seen>) -> Option<Seen> {
    let (mut file, len, modified) = open(path)?;
    if let Some(p) = prev.filter(|p| p.len == len && p.modified == Some(modified)) {
        return Some(p.clone());
    }
    let mut s = match prev {
        Some(p) => p.clone(),
        None => meta(&path.with_extension("meta.json"))?,
    };
    let grown = prev
        .map(|p| p.len)
        .filter(|from| *from <= len && len - from <= GROWTH_BYTES);
    // One byte early, so a tail window that starts on a record keeps it.
    let start = grown.unwrap_or_else(|| len.saturating_sub(TAIL_BYTES).saturating_sub(1));
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut earliest = None;
    let mut skip = grown.is_none() && start > 0;
    let end = lines(&mut file, start, len, |line| {
        if std::mem::take(&mut skip) {
            return;
        }
        if let Ok(v) = serde_json::from_slice::<Value>(line) {
            earliest = earliest.or_else(|| v["timestamp"].as_str().and_then(timestamp_ms));
            s.apply(&v, session);
        }
    })?;
    if s.started_ms.is_none() {
        let mut head = Vec::new();
        file.seek(SeekFrom::Start(0)).ok()?;
        file.take(TAIL_BYTES).read_to_end(&mut head).ok()?;
        let first = head.split(|b| *b == b'\n').next().unwrap_or_default();
        s.started_ms = serde_json::from_slice::<Value>(first)
            .ok()
            .and_then(|v| v["timestamp"].as_str().and_then(timestamp_ms))
            .or(earliest);
    }
    s.len = end;
    s.modified = Some(modified);
    Some(s)
}

/// Calls `each` with every complete line from `start` to `len`, leaving out lines longer than
/// `TAIL_BYTES` without holding them, and returns the offset after the last complete line.
fn lines(
    file: &mut std::fs::File,
    start: u64,
    len: u64,
    mut each: impl FnMut(&[u8]),
) -> Option<u64> {
    let mut reader = std::io::BufReader::new(file.take(len - start));
    let (mut line, mut over, mut at, mut end) = (Vec::new(), false, start, start);
    loop {
        let buf = reader.fill_buf().ok()?;
        if buf.is_empty() {
            return Some(end);
        }
        let newline = buf.iter().position(|b| *b == b'\n');
        let chunk = &buf[..newline.unwrap_or(buf.len())];
        over |= line.len() + chunk.len() > TAIL_BYTES as usize;
        if over {
            line.clear();
        } else {
            line.extend_from_slice(chunk);
        }
        let used = chunk.len() + usize::from(newline.is_some());
        reader.consume(used);
        at += used as u64;
        if newline.is_some() {
            if !over {
                each(&line);
            }
            line.clear();
            over = false;
            end = at;
        }
    }
}

fn meta(path: &Path) -> Option<Seen> {
    let (file, _, _) = open(path)?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes).ok()?;
    let meta = serde_json::from_slice::<Value>(&bytes).ok()?;
    Some(Seen {
        agent_type: bounded(meta["agentType"].as_str()?, TYPE_CHARS)?,
        description: meta["description"]
            .as_str()
            .and_then(|d| bounded(d, ACTIVITY_CHARS))
            .unwrap_or_default(),
        ..Seen::default()
    })
}

impl Seen {
    fn apply(&mut self, v: &Value, session: &str) {
        if v["sessionId"].as_str().is_some_and(|s| s != session) {
            return;
        }
        let m = &v["message"];
        match v["type"].as_str() {
            Some("assistant") if m["model"] == "<synthetic>" => {
                if v["isApiErrorMessage"] == true {
                    self.finished = true;
                }
            }
            Some("assistant") => {
                let u = &m["usage"];
                if u["input_tokens"].is_u64() {
                    self.input = [
                        "input_tokens",
                        "cache_creation_input_tokens",
                        "cache_read_input_tokens",
                    ]
                    .iter()
                    .map(|k| u[k].as_u64().unwrap_or(0))
                    .sum();
                    let out = u["output_tokens"].as_u64().unwrap_or(0);
                    let id = m["id"].as_str().unwrap_or_default();
                    match &mut self.message {
                        Some((last, n)) if last == id => {
                            self.output = self.output - *n + out;
                            *n = out;
                        }
                        _ => {
                            self.output += out;
                            self.message = Some((id.to_owned(), out));
                        }
                    }
                }
                self.finished = m["stop_reason"] == "end_turn";
                for b in m["content"].as_array().into_iter().flatten() {
                    if b["type"] != "tool_use" {
                        continue;
                    }
                    let name = b["name"].as_str().unwrap_or_default();
                    if ENDS.contains(&name) {
                        self.finished = true;
                        continue;
                    }
                    self.called_tools = true;
                    if let Some(a) = activity(name, &b["input"]) {
                        self.activity = Some(a);
                    }
                }
            }
            Some("user") => {
                let content = &m["content"];
                let blocks = content.as_array().map(Vec::as_slice).unwrap_or_default();
                if blocks.iter().any(|b| b["type"] == "tool_result") {
                    return;
                }
                let Some(text) = content
                    .as_str()
                    .or_else(|| blocks.iter().find(|b| b["type"] == "text")?["text"].as_str())
                else {
                    return;
                };
                if text.starts_with("[Request interrupted by user") {
                    self.finished = true;
                } else if v["isMeta"] != true
                    || matches!(
                        v["origin"]["kind"].as_str(),
                        Some("coordinator" | "peer" | "human")
                    )
                {
                    self.finished = false;
                    self.called_tools = false;
                    self.activity = None;
                    self.output = 0;
                    self.message = None;
                    self.started_ms = v["timestamp"]
                        .as_str()
                        .and_then(timestamp_ms)
                        .or(self.started_ms);
                }
            }
            _ => {}
        }
    }
}

/// Claude Code's own activity line for the tools it describes; the call's description otherwise.
fn activity(tool: &str, input: &Value) -> Option<String> {
    let s = |k: &str| input[k].as_str().and_then(|v| bounded(v, ACTIVITY_CHARS));
    let file = || {
        let path = input["file_path"]
            .as_str()
            .or(input["notebook_path"].as_str())?;
        bounded(path.rsplit('/').next().unwrap_or(path), ACTIVITY_CHARS)
    };
    let text = match tool {
        "Bash" => format!("Running {}", s("description").or_else(|| s("command"))?),
        "Read" => format!("Reading {}", file()?),
        "Edit" | "MultiEdit" => format!("Editing {}", file()?),
        "Write" => format!("Writing {}", file()?),
        "NotebookEdit" => format!("Editing notebook {}", file()?),
        "Grep" => format!("Searching for {}", s("pattern")?),
        "Glob" => format!("Finding {}", s("pattern")?),
        "WebFetch" => format!("Fetching {}", s("url")?),
        _ => s("description")?,
    };
    bounded(&text, ACTIVITY_CHARS)
}

fn bounded(text: &str, max: usize) -> Option<String> {
    let line = text.lines().map(prompt::clean).find(|l| !l.is_empty())?;
    Some(if line.chars().count() > max {
        line.chars().take(max - 1).chain(['…']).collect()
    } else {
        line
    })
}

/// `2026-10-13T21:44:05.123Z`, as Claude Code writes it.
fn timestamp_ms(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || !s.ends_with('Z') {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<u64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (h, mi, sec) = (n(11..13)?, n(14..16)?, n(17..19)?);
    let ms = if b[19] == b'.' {
        n(20..23).unwrap_or(0)
    } else {
        0
    };
    if !(1970..10000).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's `days_from_civil`.
    let y = if m <= 2 { y - 1 } else { y };
    let (era, yoe) = (y / 400, y % 400);
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(((days * 24 + h) * 60 + mi) * 60_000 + sec * 1000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    const AGENT: &str = "a3f9c2e1d4b5a6978";
    const META: &str = include_str!("../../tests/fixtures/transcripts/claude-live-agent.meta.json");

    struct Setup {
        _dir: tempfile::TempDir,
        root: PathBuf,
        transcript: PathBuf,
        sessions: PathBuf,
        sub: PathBuf,
        started: u64,
    }

    impl Setup {
        fn running(&self, live: &mut Live) -> Vec<Subagent> {
            live.running(Some(&self.sessions), &self.transcript, SESSION)
        }

        fn process(&self, pid: u32, started: u64) {
            std::fs::write(
                self.sessions.join(format!("{pid}.json")),
                format!(r#"{{"pid":{pid},"sessionId":"{SESSION}","startedAt":{started},"kind":"interactive"}}"#),
            )
            .unwrap();
        }
    }

    fn setup(records: &str) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let transcript = root.join(format!("{SESSION}.jsonl"));
        std::fs::write(&transcript, "").unwrap();
        let sub = root.join(SESSION).join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(format!("agent-{AGENT}.meta.json")), META).unwrap();
        std::fs::write(sub.join(format!("agent-{AGENT}.jsonl")), records).unwrap();
        let sessions = root.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let started = crate::now_ms() - 60_000;
        let s = Setup {
            _dir: dir,
            root,
            transcript,
            sessions,
            sub,
            started,
        };
        s.process(std::process::id(), started);
        s
    }

    fn fixture() -> Vec<&'static str> {
        include_str!("../../tests/fixtures/transcripts/claude-live-agent.jsonl")
            .split_inclusive('\n')
            .collect()
    }

    fn append(path: &Path, text: &str) {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    fn set_modified(path: &Path, ms: u64) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_millis(ms))
            .unwrap();
    }

    fn agent(sub: &Path, agent: &str, records: &str) -> PathBuf {
        std::fs::write(sub.join(format!("agent-{agent}.meta.json")), META).unwrap();
        let file = sub.join(format!("agent-{agent}.jsonl"));
        std::fs::write(&file, records).unwrap();
        file
    }

    #[test]
    fn a_running_subagent_reports_its_activity_start_and_tokens() {
        let lines = fixture();
        let s = setup(lines[0]);
        let mut live = Live::default();
        let started = s.running(&mut live);
        assert_eq!(
            started,
            [Subagent {
                agent_type: "general-purpose".into(),
                activity: "Check the pairing code".into(),
                started_at_ms: 1_791_927_845_000,
                tokens: 0,
                called_tools: false,
            }]
        );
        let file = s.sub.join(format!("agent-{AGENT}.jsonl"));
        append(&file, &lines[1..5].concat());
        let working = s.running(&mut live);
        assert_eq!(working.len(), 1);
        assert_eq!(working[0].activity, "Reading pairing.rs");
        assert_eq!(working[0].tokens, 42_000);
        assert!(working[0].called_tools);
        assert_eq!(working[0].started_at_ms, started[0].started_at_ms);
        append(&file, &lines[5..7].concat());
        let running = s.running(&mut live);
        assert_eq!(running[0].activity, "Running Check for unwraps");
        assert_eq!(running[0].tokens, 44_000);
        append(&file, &lines[7..].concat());
        assert!(s.running(&mut live).is_empty());
    }

    #[test]
    fn finished_interrupted_and_resumed_subagents() {
        let lines = fixture();
        let end = r#"{"type":"assistant","sessionId":"3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d","timestamp":"2026-10-13T21:50:00.000Z","message":{"model":"claude-opus-5-5","stop_reason":"end_turn","content":[{"type":"text","text":"Done."}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        let s = setup(&[lines[..5].concat(), end.into(), "\n".into()].concat());
        let mut live = Live::default();
        assert!(s.running(&mut live).is_empty());
        let file = s.sub.join(format!("agent-{AGENT}.jsonl"));
        let follow_up = r#"{"type":"user","isMeta":true,"origin":{"kind":"coordinator"},"sessionId":"3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d","timestamp":"2026-10-13T22:00:00.000Z","message":{"role":"user","content":"Also check the retry loop."}}"#;
        append(&file, &format!("{follow_up}\n"));
        let resumed = s.running(&mut live);
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].started_at_ms, 1_791_928_800_000);
        assert_eq!(resumed[0].activity, "Check the pairing code");
        assert!(!resumed[0].called_tools);
        let reminder = r#"{"type":"user","isMeta":true,"sessionId":"3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d","timestamp":"2026-10-13T22:01:00.000Z","message":{"role":"user","content":"<system-reminder>x</system-reminder>"}}"#;
        append(&file, &format!("{reminder}\n"));
        assert_eq!(s.running(&mut live), resumed);
        let interrupted = r#"{"type":"user","sessionId":"3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d","timestamp":"2026-10-13T22:02:00.000Z","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
        append(&file, &format!("{interrupted}\n"));
        assert!(s.running(&mut live).is_empty());
    }

    #[test]
    fn workflow_agents_are_listed_until_their_result() {
        let lines = fixture();
        let s = setup(&lines[..7].concat());
        std::fs::remove_file(s.sub.join(format!("agent-{AGENT}.jsonl"))).unwrap();
        let flow = s.sub.join("workflows").join("wf_1f732cde-9b9");
        std::fs::create_dir_all(&flow).unwrap();
        std::fs::write(
            flow.join("agent-w1.meta.json"),
            r#"{"agentType":"workflow-subagent","description":"review:correctness","workflowPhase":"Review"}"#,
        )
        .unwrap();
        let file = flow.join("agent-w1.jsonl");
        std::fs::write(&file, lines[0]).unwrap();
        let mut live = Live::default();
        let running = s.running(&mut live);
        assert_eq!(running.len(), 1);
        assert_eq!(
            (running[0].agent_type.as_str(), running[0].activity.as_str()),
            ("workflow-subagent", "review:correctness")
        );
        let result = r#"{"type":"assistant","message":{"model":"m","stop_reason":"tool_use","content":[{"type":"tool_use","name":"StructuredOutput","input":{}}],"usage":{"input_tokens":1}}}"#;
        append(&file, &format!("{result}\n"));
        assert!(s.running(&mut live).is_empty());

        std::fs::write(&file, lines[0]).unwrap();
        assert_eq!(s.running(&mut live).len(), 1);
        let elsewhere = s.root.join("flow");
        std::fs::rename(&flow, &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &flow).unwrap();
        assert!(s.running(&mut live).is_empty());
        std::fs::remove_file(&flow).unwrap();
        std::fs::rename(&elsewhere, &flow).unwrap();
        let workflows = s.sub.join("workflows");
        std::fs::rename(&workflows, s.root.join("workflows")).unwrap();
        std::os::unix::fs::symlink(s.root.join("workflows"), &workflows).unwrap();
        assert!(s.running(&mut live).is_empty());
    }

    #[test]
    fn only_while_the_claude_process_runs_and_only_records_since_it_started() {
        let lines = fixture();
        let s = setup(lines[0]);
        let mut live = Live::default();
        assert_eq!(s.running(&mut live).len(), 1);
        assert!(live.running(None, &s.transcript, SESSION).is_empty());

        let file = s.sub.join(format!("agent-{AGENT}.jsonl"));
        set_modified(&file, s.started - 1000);
        assert!(s.running(&mut live).is_empty());
        set_modified(&file, s.started + 1000);
        assert_eq!(s.running(&mut live).len(), 1);

        std::fs::remove_file(s.sessions.join(format!("{}.json", std::process::id()))).unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        s.process(dead, s.started);
        assert!(s.running(&mut live).is_empty());

        let real = s.root.join("real.json");
        std::fs::write(
            &real,
            format!(
                r#"{{"pid":{},"sessionId":"{SESSION}","startedAt":{}}}"#,
                std::process::id(),
                s.started
            ),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            &real,
            s.sessions.join(format!("{}.json", std::process::id())),
        )
        .unwrap();
        assert!(s.running(&mut live).is_empty());
        std::fs::remove_file(s.sessions.join(format!("{}.json", std::process::id()))).unwrap();
        s.process(std::process::id() + 1_000_000, s.started);
        std::fs::rename(
            s.sessions
                .join(format!("{}.json", std::process::id() + 1_000_000)),
            s.sessions.join(format!("{}.json", std::process::id())),
        )
        .unwrap();
        assert!(
            s.running(&mut live).is_empty(),
            "the pid inside must match the file name"
        );
    }

    #[test]
    fn reads_are_bounded_and_refuse_symlinks() {
        let lines = fixture();
        let s = setup(&lines[..5].concat());
        let file = s.sub.join(format!("agent-{AGENT}.jsonl"));
        let filler = format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"content\":\"{}\"}}]}}}}\n",
            "x".repeat(TAIL_BYTES as usize)
        );
        let long = format!(
            r#"{{"type":"assistant","message":{{"model":"m","content":[{{"type":"tool_use","name":"Bash","input":{{"description":"{}\u001b[2J"}}}}],"usage":{{"input_tokens":5}}}}}}"#,
            "é".repeat(500)
        );
        let search = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","name":"ToolSearch","input":{}}],"usage":{"input_tokens":5}}}"#;
        append(&file, &format!("{filler}{search}\n"));
        let mut live = Live::default();
        let first = s.running(&mut live);
        assert_eq!(
            (first[0].activity.as_str(), first[0].tokens),
            ("Check the pairing code", 0),
            "only the tail is read, not the Read call before it"
        );
        assert_eq!(first[0].started_at_ms, 1_791_927_845_000);
        append(&file, &format!("{long}\n"));
        let running = s.running(&mut live);
        assert_eq!(running[0].activity.chars().count(), ACTIVITY_CHARS);
        assert!(
            running[0].activity.starts_with("Running éé") && running[0].activity.ends_with('…')
        );
        assert!(!running[0].activity.contains('\u{1b}'));

        let big_first = format!(
            r#"{{"type":"user","message":{{"content":"{}"}},"timestamp":"2026-10-13T21:00:00.000Z"}}"#,
            "p".repeat(TAIL_BYTES as usize)
        );
        let late = r#"{"type":"attachment","timestamp":"2026-10-13T21:30:00.000Z"}"#;
        let b = agent(&s.sub, "big", &format!("{big_first}\n{late}\n"));
        let shown = s.running(&mut live);
        assert_eq!(shown.len(), 2);
        assert!(shown.iter().any(|a| a.started_at_ms == 1_791_927_000_000));
        std::fs::remove_file(b).unwrap();

        let target = s.root.join("elsewhere.jsonl");
        std::fs::rename(&file, &target).unwrap();
        std::os::unix::fs::symlink(&target, &file).unwrap();
        assert!(s.running(&mut live).is_empty());

        let session = s.root.join(SESSION);
        let outside = s.root.join("outside");
        std::fs::rename(&session, &outside).unwrap();
        std::fs::remove_file(
            outside
                .join("subagents")
                .join(format!("agent-{AGENT}.jsonl")),
        )
        .unwrap();
        std::fs::rename(
            &target,
            outside
                .join("subagents")
                .join(format!("agent-{AGENT}.jsonl")),
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, &session).unwrap();
        assert!(s.running(&mut live).is_empty());
        assert!(live.seen.is_empty());
    }

    #[test]
    fn finished_files_are_skipped_and_the_rest_capped() {
        let lines = fixture();
        let s = setup(lines[0]);
        let older = s.started + 1000;
        set_modified(&s.sub.join(format!("agent-{AGENT}.jsonl")), older);
        for n in 0..MAX_FILES {
            agent(&s.sub, &format!("f{n}"), &lines.concat());
        }
        let mut live = Live::default();
        assert!(s.running(&mut live).is_empty());
        assert_eq!(s.running(&mut live).len(), 1);

        for n in 0..MAX_FILES + 2 {
            agent(&s.sub, &format!("r{n}"), lines[0]);
        }
        assert_eq!(
            s.running(&mut live).len(),
            MAX_FILES + 1,
            "8 opened, the unchanged one kept from the last read"
        );
        assert_eq!(s.running(&mut live).len(), MAX_FILES + 3);
        assert_eq!(s.running(&mut live).len(), MAX_FILES + 3);
    }

    #[test]
    fn records_behind_a_large_result_and_api_errors() {
        let lines = fixture();
        let s = setup(&lines[..5].concat());
        let mut live = Live::default();
        assert_eq!(s.running(&mut live)[0].activity, "Reading pairing.rs");
        let file = s.sub.join(format!("agent-{AGENT}.jsonl"));
        let big = format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"content\":\"{}\"}}]}}}}\n",
            "x".repeat(2 * TAIL_BYTES as usize)
        );
        append(&file, &[lines[5], &big].concat());
        let running = s.running(&mut live);
        assert_eq!(running[0].activity, "Running Check for unwraps");
        assert_eq!(running[0].tokens, 44_000);
        append(&file, &lines[6][..20]);
        assert_eq!(s.running(&mut live), running);
        append(&file, &lines[6][20..]);
        assert_eq!(s.running(&mut live), running);

        let error = r#"{"type":"assistant","isApiErrorMessage":true,"message":{"model":"<synthetic>","stop_reason":"stop_sequence","content":[{"type":"text","text":"API Error"}]}}"#;
        append(&file, &format!("{error}\n"));
        assert!(s.running(&mut live).is_empty());
    }

    #[test]
    fn activity_lines() {
        let deep = format!("/{}/pairing.rs", "d".repeat(200));
        assert_eq!(
            activity("Read", &serde_json::json!({ "file_path": deep })).as_deref(),
            Some("Reading pairing.rs")
        );
        let input = serde_json::json!({"file_path":"/a/b/pairing.rs","pattern":"unwrap","command":"\n  cargo test\nmore","description":"Fetch"});
        assert_eq!(
            activity("Edit", &input).as_deref(),
            Some("Editing pairing.rs")
        );
        assert_eq!(
            activity("Grep", &input).as_deref(),
            Some("Searching for unwrap")
        );
        assert_eq!(activity("Agent", &input).as_deref(), Some("Fetch"));
        assert_eq!(
            activity(
                "Bash",
                &serde_json::json!({"command":"\n  cargo test\nmore"})
            )
            .as_deref(),
            Some("Running cargo test")
        );
        assert_eq!(
            activity("ToolSearch", &serde_json::json!({"query":"x"})),
            None
        );
    }

    #[test]
    fn timestamps() {
        assert_eq!(timestamp_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(
            timestamp_ms("2024-02-29T12:34:56.789Z"),
            Some(1_709_210_096_789)
        );
        assert_eq!(
            timestamp_ms("2026-10-13T21:44:05Z"),
            Some(1_791_927_845_000)
        );
        for bad in [
            "",
            "2026-10-13 21:44:05Z",
            "2026-13-13T21:44:05Z",
            "2026-10-13T21:44:05+02:00",
        ] {
            assert_eq!(timestamp_ms(bad), None, "{bad}");
        }
    }
}
