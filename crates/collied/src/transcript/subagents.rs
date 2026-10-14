use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, SystemTime};

use protocol::Subagent;
use serde_json::Value;

use super::history::{agent_id, subagent_dir};
use super::open;
use crate::prompt;

const TAIL_BYTES: u64 = 64 * 1024;
const MAX_FILES: usize = 8;
/// A subagent of a Claude Code process that ended never writes its end.
const STALE: Duration = Duration::from_secs(30 * 60);
const ACTIVITY_CHARS: usize = 120;
const TYPE_CHARS: usize = 64;

#[derive(Clone, Default)]
struct Seen {
    len: u64,
    modified: Option<SystemTime>,
    agent_type: String,
    description: String,
    finished: bool,
    activity: Option<String>,
    started_ms: Option<u64>,
    tokens: u64,
    called_tools: bool,
}

/// By session id, then by agent id.
#[derive(Default)]
pub struct Live(HashMap<String, HashMap<String, Seen>>);

impl Live {
    pub fn retain(&mut self, live: impl Fn(&String) -> bool) {
        self.0.retain(|id, _| live(id));
    }

    /// Only the most recently written files are opened, each read again only when its size
    /// or mtime moved, and then only its tail.
    pub fn running(&mut self, transcript: &Path, session: &str) -> Vec<Subagent> {
        let Some(dir) = subagent_dir(transcript, session) else {
            self.0.remove(session);
            return Vec::new();
        };
        let now = SystemTime::now();
        let mut recent = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let agent = name.strip_prefix("agent-")?.strip_suffix(".jsonl")?;
                agent_id(agent).then_some(())?;
                let modified = e.metadata().ok().filter(|m| m.is_file())?.modified().ok()?;
                (now.duration_since(modified).unwrap_or_default() < STALE)
                    .then(|| (modified, agent.to_owned()))
            })
            .collect::<Vec<_>>();
        recent.sort_by_key(|a| std::cmp::Reverse(a.0));
        recent.truncate(MAX_FILES);
        let old = self.0.remove(session).unwrap_or_default();
        let seen = recent
            .into_iter()
            .filter_map(|(_, agent)| {
                Some((agent.clone(), read(&dir, session, &agent, old.get(&agent))?))
            })
            .collect::<HashMap<_, _>>();
        let mut out = seen
            .values()
            .filter(|s| !s.finished)
            .filter_map(|s| {
                Some(Subagent {
                    agent_type: s.agent_type.clone(),
                    activity: s.activity.clone().unwrap_or_else(|| s.description.clone()),
                    started_at_ms: s.started_ms?,
                    tokens: s.tokens,
                    called_tools: s.called_tools,
                })
            })
            .collect::<Vec<_>>();
        out.sort_by(|a, b| (a.started_at_ms, &a.activity).cmp(&(b.started_at_ms, &b.activity)));
        self.0.insert(session.to_owned(), seen);
        out
    }
}

fn read(dir: &Path, session: &str, agent: &str, prev: Option<&Seen>) -> Option<Seen> {
    let (mut file, len, modified) = open(&dir.join(format!("agent-{agent}.jsonl")))?;
    if let Some(p) = prev.filter(|p| p.len == len && p.modified == Some(modified)) {
        return Some(p.clone());
    }
    let mut s = match prev {
        Some(p) => p.clone(),
        None => meta(dir, agent)?,
    };
    // One byte early, so a window that starts on a record keeps it.
    let start = len.saturating_sub(TAIL_BYTES).saturating_sub(1);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    (&mut file).take(len - start).read_to_end(&mut bytes).ok()?;
    let mut lines = bytes.split(|b| *b == b'\n');
    if start > 0 {
        lines.next();
    }
    for line in lines {
        if let Ok(v) = serde_json::from_slice::<Value>(line) {
            s.apply(&v, session);
        }
    }
    if s.started_ms.is_none() {
        let mut head = Vec::new();
        file.seek(SeekFrom::Start(0)).ok()?;
        file.take(TAIL_BYTES).read_to_end(&mut head).ok()?;
        let first = head.split(|b| *b == b'\n').next()?;
        let v = serde_json::from_slice::<Value>(first).ok()?;
        s.started_ms = v["timestamp"].as_str().and_then(timestamp_ms);
    }
    s.len = len;
    s.modified = Some(modified);
    Some(s)
}

fn meta(dir: &Path, agent: &str) -> Option<Seen> {
    let (file, _, _) = open(&dir.join(format!("agent-{agent}.meta.json")))?;
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
            Some("assistant") if m["model"] != "<synthetic>" => {
                let u = &m["usage"];
                if u["input_tokens"].is_u64() {
                    self.tokens = [
                        "input_tokens",
                        "cache_creation_input_tokens",
                        "cache_read_input_tokens",
                        "output_tokens",
                    ]
                    .iter()
                    .map(|k| u[k].as_u64().unwrap_or(0))
                    .sum();
                }
                self.finished = m["stop_reason"] == "end_turn";
                for b in m["content"].as_array().into_iter().flatten() {
                    if b["type"] != "tool_use" {
                        continue;
                    }
                    let name = b["name"].as_str().unwrap_or_default();
                    if name == "SubagentHandback" {
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
        let path = s("file_path").or_else(|| s("notebook_path"))?;
        Some(path.rsplit('/').next().unwrap_or(&path).to_owned())
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

    fn setup(records: &str) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join(format!("{SESSION}.jsonl"));
        std::fs::write(&transcript, "").unwrap();
        let sub = dir.path().join(SESSION).join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(
            sub.join(format!("agent-{AGENT}.meta.json")),
            include_str!("../../tests/fixtures/transcripts/claude-live-agent.meta.json"),
        )
        .unwrap();
        std::fs::write(sub.join(format!("agent-{AGENT}.jsonl")), records).unwrap();
        (dir, transcript, sub)
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

    #[test]
    fn a_running_subagent_reports_its_activity_start_and_tokens() {
        let lines = fixture();
        let (_dir, transcript, sub) = setup(lines[0]);
        let mut live = Live::default();
        let started = live.running(&transcript, SESSION);
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
        let file = sub.join(format!("agent-{AGENT}.jsonl"));
        append(&file, &lines[1..5].concat());
        let working = live.running(&transcript, SESSION);
        assert_eq!(working.len(), 1);
        assert_eq!(working[0].activity, "Reading pairing.rs");
        assert_eq!(working[0].tokens, 2 + 24_616 + 17_458 + 8);
        assert!(working[0].called_tools);
        assert_eq!(working[0].started_at_ms, started[0].started_at_ms);
        append(&file, &lines[5..7].concat());
        let running = live.running(&transcript, SESSION);
        assert_eq!(running[0].activity, "Running Check for unwraps");
        assert_eq!(running[0].tokens, 2 + 1_297 + 43_123 + 16);
        append(&file, &lines[7..].concat());
        assert!(live.running(&transcript, SESSION).is_empty());
    }

    #[test]
    fn finished_interrupted_and_resumed_subagents() {
        let lines = fixture();
        let end = r#"{"type":"assistant","sessionId":"3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d","timestamp":"2026-10-13T21:50:00.000Z","message":{"model":"claude-opus-5-5","stop_reason":"end_turn","content":[{"type":"text","text":"Done."}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        let (_dir, transcript, sub) =
            setup(&[lines[..5].concat(), end.into(), "\n".into()].concat());
        let mut live = Live::default();
        assert!(live.running(&transcript, SESSION).is_empty());
        let file = sub.join(format!("agent-{AGENT}.jsonl"));
        let follow_up = r#"{"type":"user","isMeta":true,"origin":{"kind":"coordinator"},"sessionId":"3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d","timestamp":"2026-10-13T22:00:00.000Z","message":{"role":"user","content":"Also check the retry loop."}}"#;
        append(&file, &format!("{follow_up}\n"));
        let resumed = live.running(&transcript, SESSION);
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].started_at_ms, 1_791_928_800_000);
        assert_eq!(resumed[0].activity, "Check the pairing code");
        assert!(!resumed[0].called_tools);
        let reminder = r#"{"type":"user","isMeta":true,"sessionId":"3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d","timestamp":"2026-10-13T22:01:00.000Z","message":{"role":"user","content":"<system-reminder>x</system-reminder>"}}"#;
        append(&file, &format!("{reminder}\n"));
        assert_eq!(live.running(&transcript, SESSION), resumed);
        let interrupted = r#"{"type":"user","sessionId":"3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d","timestamp":"2026-10-13T22:02:00.000Z","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
        append(&file, &format!("{interrupted}\n"));
        assert!(live.running(&transcript, SESSION).is_empty());
    }

    #[test]
    fn reads_are_bounded_and_refuse_symlinks_and_stale_files() {
        let lines = fixture();
        let (dir, transcript, sub) = setup(&lines[..5].concat());
        let file = sub.join(format!("agent-{AGENT}.jsonl"));
        let filler = format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"content\":\"{}\"}}]}}}}\n",
            "x".repeat(TAIL_BYTES as usize)
        );
        let long = format!(
            r#"{{"type":"assistant","message":{{"model":"m","content":[{{"type":"tool_use","name":"Bash","input":{{"description":"{}\u001b[2J"}}}}],"usage":{{"input_tokens":5}}}}}}"#,
            "é".repeat(500)
        );
        append(&file, &format!("{filler}{long}\n"));
        let mut live = Live::default();
        let running = live.running(&transcript, SESSION);
        assert_eq!(running[0].started_at_ms, 1_791_927_845_000);
        assert_eq!(running[0].tokens, 5);
        assert_eq!(running[0].activity.chars().count(), ACTIVITY_CHARS);
        assert!(
            running[0].activity.starts_with("Running éé") && running[0].activity.ends_with('…')
        );
        assert!(!running[0].activity.contains('\u{1b}'));

        for n in 0..MAX_FILES + 2 {
            let agent = format!("b{n}");
            std::fs::copy(
                sub.join(format!("agent-{AGENT}.meta.json")),
                sub.join(format!("agent-{agent}.meta.json")),
            )
            .unwrap();
            std::fs::write(sub.join(format!("agent-{agent}.jsonl")), lines[0]).unwrap();
        }
        assert_eq!(live.running(&transcript, SESSION).len(), MAX_FILES);

        let stale = SystemTime::now() - STALE - Duration::from_secs(1);
        for e in std::fs::read_dir(&sub).unwrap().flatten() {
            if e.file_name().to_str().unwrap().ends_with(".jsonl")
                && e.file_name() != *format!("agent-{AGENT}.jsonl")
            {
                std::fs::File::options()
                    .write(true)
                    .open(e.path())
                    .unwrap()
                    .set_modified(stale)
                    .unwrap();
            }
        }
        assert_eq!(live.running(&transcript, SESSION).len(), 1);

        let target = dir.path().join("elsewhere.jsonl");
        std::fs::rename(&file, &target).unwrap();
        std::os::unix::fs::symlink(&target, &file).unwrap();
        assert!(live.running(&transcript, SESSION).is_empty());

        let session = dir.path().join(SESSION);
        let outside = dir.path().join("outside");
        std::fs::rename(&session, &outside).unwrap();
        std::fs::rename(
            &target,
            outside
                .join("subagents")
                .join(format!("agent-{AGENT}.jsonl")),
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, &session).unwrap();
        assert!(live.running(&transcript, SESSION).is_empty());
        assert!(live.0.is_empty());
    }

    #[test]
    fn activity_lines() {
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
