use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use protocol::{HistoryMessage, HistoryPage, HistoryParams};
use serde_json::Value;

use super::{Kind, Transcripts, open};
use crate::herdr::AgentInfo;

const SCAN_BYTES: u64 = 2 * 1024 * 1024;
const PAGE_BYTES: usize = 40 * 1024;
const RECORD_TEXT_BYTES: usize = 6000;

impl Transcripts {
    pub fn history_reader(&self) -> Self {
        Self {
            panes: self.panes.clone(),
            ..Self::new(self.claude_projects.clone(), self.codex_sessions.clone())
        }
    }

    pub fn history(&self, agent: &AgentInfo, params: &HistoryParams) -> HistoryPage {
        self.read_history(agent, params).unwrap_or(HistoryPage {
            session: None,
            available: false,
            unavailable_reason: Some(if self.live_id(agent).is_none() {
                "The agent has not reported its session ID to herdr. Chat cannot safely identify this conversation. Check the agent’s herdr SessionStart integration, then resume the session."
            } else {
                "The transcript for this session is not readable yet. Terminal remains available."
            }.into()),
            reset: false,
            messages: vec![],
            before: None,
            after: None,
            more: false,
            omitted: false,
        })
    }

    fn read_history(&self, agent: &AgentInfo, p: &HistoryParams) -> Option<HistoryPage> {
        let kind = match agent.agent.as_deref()? {
            "claude" => Kind::Claude,
            "codex" => Kind::Codex,
            _ => return None,
        };
        let id = self.live_id(agent)?;
        let id = id.as_str();
        let path = self.locate(kind, id, agent)?;
        let (mut file, len, _) = open(&path)?;
        let meta = file.metadata().ok()?;
        let mut header = vec![0; len.min(65536) as usize];
        file.read_exact(&mut header).ok()?;
        let first = header.split(|b| *b == b'\n').next().unwrap_or_default();
        if matches!(kind, Kind::Codex) && header_string(first, "id").is_some_and(|s| s != id) {
            return None;
        }
        let modern = header_string(first, "cli_version")
            .and_then(|s| s.split('.').nth(1).and_then(|s| s.parse::<u32>().ok()))
            .is_some_and(|v| v >= 160)
            || header.windows(16).any(|w| w == b"\"item_completed\"");
        let session = format!(
            "{}:{id}:{}:{}",
            agent.agent.as_deref()?,
            meta.dev(),
            meta.ino()
        );
        let supplied = p.before.as_ref().or(p.after.as_ref());
        let offset = supplied.and_then(|c| cursor_offset(&mut file, &session, c, len));
        let reset = supplied.is_some() && offset.is_none();
        let forward = p.after.is_some() && !reset;
        let edge = offset.unwrap_or(len);
        let (start, end) = if forward {
            (edge, (edge + SCAN_BYTES).min(len))
        } else {
            (edge.saturating_sub(SCAN_BYTES), edge)
        };
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut bytes = vec![0; (end - start) as usize];
        file.read_exact(&mut bytes).ok()?;
        let boundary = if start == 0 {
            true
        } else {
            file.seek(SeekFrom::Start(start - 1)).ok()?;
            let mut b = [0];
            file.read_exact(&mut b).ok()?;
            b[0] == b'\n'
        };
        let mut positions = Vec::new();
        let mut line_start = if boundary { Some(0) } else { None };
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'\n' {
                if let Some(a) = line_start {
                    positions.push((a, i + 1));
                }
                line_start = Some(i + 1);
            }
        }
        let mut omitted = false;
        let complete_end = positions.last().map_or_else(
            || line_start.map_or(start, |n| start + n as u64),
            |(_, b)| start + *b as u64,
        );
        let mut low = complete_end;
        let mut high = if forward { start } else { complete_end };
        let query = p.query.as_deref().unwrap_or("").to_lowercase();
        let mut messages = Vec::new();
        let mut size = 0;
        let empty = positions.is_empty();
        if !forward {
            positions.reverse();
        }
        let mut budget = SCAN_BYTES;
        let mut attached = HashSet::new();
        let mut calls = None;
        let mut consumed = false;
        for (a, b) in positions {
            let record = serde_json::from_slice::<Value>(&bytes[a..b]);
            let fallback = (start + a as u64).to_string();
            let mut batch = match &record {
                Ok(v) => normalize(v, kind, id, modern, fallback, &query, None),
                Err(_) => {
                    omitted = true;
                    Vec::new()
                }
            };
            if let (Some(m), Ok(v)) = (batch.first_mut(), &record)
                && m.role == "event"
                && v["origin"]["kind"] == "peer"
                && let Some(text) = v["origin"]["from"]
                    .as_str()
                    .filter(|a| agent_id(a))
                    .and_then(|a| finished(&path, id, a))
            {
                m.text = text;
            }
            let named = match (kind, &record) {
                (Kind::Claude, Ok(v)) => named(v, id, forward, &path, &mut calls),
                _ => Vec::new(),
            };
            let mut found = Vec::new();
            let mut cut = false;
            let mut deferred = false;
            for agent in named {
                if !attached.insert(agent.clone()) {
                    continue;
                }
                let Some((parent, file, sub_len)) = subagent(&path, id, &agent) else {
                    continue;
                };
                if sub_len > budget && budget < SCAN_BYTES && consumed {
                    deferred = true;
                    break;
                }
                let read = sub_len.min(budget);
                budget -= read;
                let (more, partial) = steps(file, sub_len, read, id, &agent, &parent, &query);
                found.extend(more);
                cut |= partial;
            }
            if deferred {
                break;
            }
            let main_size = serde_json::to_vec(&batch).ok()?.len();
            let any = !found.is_empty();
            let (kept, trimmed) = fit(
                found,
                40usize.saturating_sub(messages.len() + batch.len()),
                PAGE_BYTES.saturating_sub(size + main_size),
            );
            if any && kept.is_empty() && !messages.is_empty() {
                break;
            }
            batch.extend(kept);
            let batch_size = serde_json::to_vec(&batch).ok()?.len();
            if size + batch_size > PAGE_BYTES || messages.len() + batch.len() > 40 {
                break;
            }
            omitted |= trimmed || cut;
            consumed = true;
            low = low.min(start + a as u64);
            if forward {
                high = start + b as u64;
            }
            size += batch_size;
            if !forward {
                batch.reverse();
            }
            messages.extend(batch);
        }
        if forward {
            low = start;
            if empty {
                high = complete_end;
                if high == start && end < len {
                    high = end;
                    omitted = true;
                }
            }
            if !boundary && high > start {
                omitted = true;
            }
        } else {
            messages.reverse();
            if empty {
                low = start;
                omitted = start > 0;
            }
        }
        Some(HistoryPage {
            session: Some(session.clone()),
            available: true,
            unavailable_reason: None,
            reset,
            messages,
            before: (low > 0)
                .then(|| cursor(&mut file, &session, low))
                .flatten(),
            after: cursor(&mut file, &session, high),
            more: forward && (end < len || high < complete_end),
            omitted,
        })
    }
}

fn header_string(bytes: &[u8], key: &str) -> Option<String> {
    let key = format!("\"{key}\"");
    let at = bytes.windows(key.len()).position(|w| w == key.as_bytes())? + key.len();
    let colon = at + bytes[at..].iter().position(|b| !b.is_ascii_whitespace())?;
    if bytes[colon] != b':' {
        return None;
    }
    serde_json::Deserializer::from_slice(&bytes[colon + 1..])
        .into_iter::<String>()
        .next()?
        .ok()
}

fn digest(file: &mut std::fs::File, offset: u64) -> Option<u64> {
    let start = offset.saturating_sub(64);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = vec![0; (offset - start) as usize];
    file.read_exact(&mut bytes).ok()?;
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    Some(hash.finish())
}

fn cursor(file: &mut std::fs::File, session: &str, offset: u64) -> Option<String> {
    Some(format!("{session}:{offset}:{}", digest(file, offset)?))
}

fn cursor_offset(file: &mut std::fs::File, session: &str, value: &str, len: u64) -> Option<u64> {
    let rest = value.strip_prefix(session)?.strip_prefix(':')?;
    let (offset, hash) = rest.split_once(':')?;
    let offset = offset.parse::<u64>().ok().filter(|n| *n <= len)?;
    (digest(file, offset)? == hash.parse::<u64>().ok()?).then_some(offset)
}

/// Claude Code links a subagent to its Agent call in `agent-<id>.meta.json`; a hand-back names it by id.
fn named(
    v: &Value,
    session: &str,
    forward: bool,
    transcript: &Path,
    calls: &mut Option<HashMap<String, String>>,
) -> Vec<String> {
    if v["isSidechain"] == true || v["sessionId"].as_str().is_some_and(|s| s != session) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if forward
        && v["origin"]["kind"] == "peer"
        && let Some(from) = v["origin"]["from"].as_str()
    {
        out.push(from.to_owned());
    }
    for b in v["message"]["content"].as_array().into_iter().flatten() {
        if b["type"] == "tool_use"
            && matches!(b["name"].as_str(), Some("Agent" | "Task"))
            && let Some(call) = b["id"].as_str()
        {
            let calls = calls.get_or_insert_with(|| subagent_calls(transcript, session));
            out.extend(calls.get(call).cloned());
        }
    }
    out.retain(|a| agent_id(a));
    out
}

pub(super) fn agent_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Only real directories under this session's own directory.
pub(super) fn subagent_dir(transcript: &Path, session: &str) -> Option<std::path::PathBuf> {
    let mut dir = transcript.parent()?.to_path_buf();
    for part in [session, "subagents"] {
        dir.push(part);
        if !std::fs::symlink_metadata(&dir).ok()?.is_dir() {
            return None;
        }
    }
    Some(dir)
}

fn meta(dir: &Path, agent: &str) -> Option<Value> {
    let (meta, _, _) = open(&dir.join(format!("agent-{agent}.meta.json")))?;
    let mut bytes = Vec::new();
    meta.take(65536).read_to_end(&mut bytes).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()
}

fn meta_parent(dir: &Path, agent: &str) -> Option<String> {
    meta(dir, agent)?["toolUseId"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .map(str::to_owned)
}

fn finished(transcript: &Path, session: &str, agent: &str) -> Option<String> {
    let meta = meta(&subagent_dir(transcript, session)?, agent)?;
    let label = meta["description"]
        .as_str()
        .or(meta["agentType"].as_str())?
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>();
    Some(
        format!("Subagent finished: {}", label.trim())
            .chars()
            .take(120)
            .collect(),
    )
}

fn subagent_calls(transcript: &Path, session: &str) -> HashMap<String, String> {
    let Some(dir) = subagent_dir(transcript, session) else {
        return HashMap::new();
    };
    std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let agent = name.strip_prefix("agent-")?.strip_suffix(".meta.json")?;
            agent_id(agent).then_some(())?;
            Some((meta_parent(&dir, agent)?, agent.to_owned()))
        })
        .collect()
}

fn subagent(transcript: &Path, session: &str, agent: &str) -> Option<(String, std::fs::File, u64)> {
    let dir = subagent_dir(transcript, session)?;
    let parent = meta_parent(&dir, agent)?;
    let (file, len, _) = open(&dir.join(format!("agent-{agent}.jsonl")))?;
    Some((parent, file, len))
}

fn steps(
    mut file: std::fs::File,
    len: u64,
    read: u64,
    session: &str,
    agent: &str,
    parent: &str,
    query: &str,
) -> (Vec<HistoryMessage>, bool) {
    let start = (len - read).saturating_sub(1);
    let mut head = vec![0; len.min(65536) as usize];
    let mut bytes = vec![0; (len - start) as usize];
    if len > read && file.read_exact(&mut head).is_err()
        || file.seek(SeekFrom::Start(start)).is_err()
        || file.read_exact(&mut bytes).is_err()
    {
        return (Vec::new(), false);
    }
    let mut lines = Vec::new();
    if len > read {
        lines.extend(
            head.iter()
                .position(|b| *b == b'\n')
                .map(|n| (0, &head[..n])),
        );
    }
    let mut at = if len == read {
        0
    } else {
        bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |i| i + 1)
    };
    while let Some(n) = bytes[at..].iter().position(|b| *b == b'\n') {
        lines.push((start + at as u64, &bytes[at..at + n]));
        at += n + 1;
    }
    let steps = lines
        .into_iter()
        .filter_map(|(offset, line)| {
            let v = serde_json::from_slice::<Value>(line).ok()?;
            let fallback = format!("{agent}:{offset}");
            Some(normalize(
                &v,
                Kind::Claude,
                session,
                false,
                fallback,
                query,
                Some(parent),
            ))
        })
        .flatten()
        .collect();
    (steps, len > read)
}

/// A subagent larger than the room left keeps its first step and its latest ones.
fn fit(
    steps: Vec<HistoryMessage>,
    mut count: usize,
    mut bytes: usize,
) -> (Vec<HistoryMessage>, bool) {
    let mut keep = vec![false; steps.len()];
    for i in (0..steps.len().min(1)).chain((1..steps.len()).rev()) {
        let size = serde_json::to_vec(&steps[i]).map_or(usize::MAX, |v| v.len().saturating_add(1));
        if count == 0 || size > bytes {
            break;
        }
        keep[i] = true;
        count -= 1;
        bytes -= size;
    }
    let trimmed = keep.contains(&false);
    let kept = steps
        .into_iter()
        .zip(keep)
        .filter_map(|(m, k)| k.then_some(m))
        .collect();
    (kept, trimmed)
}

fn text(value: &Value) -> String {
    if let Some(s) = value.as_str() {
        return s.to_owned();
    }
    if let Some(blocks) = value.as_array() {
        return blocks
            .iter()
            .filter_map(|b| {
                b["text"].as_str().map(str::to_owned).or_else(|| {
                    matches!(
                        b["type"].as_str(),
                        Some("image" | "input_image" | "image_url" | "Image")
                    )
                    .then(|| "[Image attachment]".into())
                })
            })
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    if value.is_null() {
        String::new()
    } else {
        value.to_string()
    }
}

fn normalize(
    v: &Value,
    kind: Kind,
    session: &str,
    modern: bool,
    fallback: String,
    query: &str,
    parent: Option<&str>,
) -> Vec<HistoryMessage> {
    let mut out = Vec::new();
    let base = match kind {
        Kind::Claude => v["uuid"].as_str(),
        Kind::Codex => v["payload"]["item"]["id"]
            .as_str()
            .or(v["payload"]["id"].as_str()),
    }
    .filter(|s| s.len() <= 128)
    .map(str::to_owned)
    .unwrap_or(fallback);
    let mut add = |role: &str, body: String, tool: Option<&str>, call_id: Option<&str>| {
        if body.is_empty() && tool.is_none() {
            return;
        }
        let id = format!("{base}:{}", out.len());
        out.push(HistoryMessage {
            id,
            role: role.into(),
            text: body,
            tool: tool.map(|s| s.chars().take(80).collect()),
            call_id: call_id.map(|s| s.chars().take(128).collect()),
            truncated: false,
            parent_call_id: parent.map(str::to_owned),
        });
    };
    match kind {
        Kind::Claude => {
            if (parent.is_none() && v["isSidechain"] == true)
                || (parent.is_some() && v["isMeta"] == true && v["origin"].is_null())
                || v["sessionId"].as_str().is_some_and(|s| s != session)
                || v["message"]["model"] == "<synthetic>"
            {
                return out;
            }
            let notice = v["origin"]["kind"] == "task-notification";
            if parent.is_none()
                && v["type"] == "user"
                && (v["isMeta"] == true || v["isCompactSummary"] == true || notice)
            {
                let event = if v["isMeta"] == true && v["origin"]["kind"] == "peer" {
                    Some("Subagent finished")
                } else {
                    notice.then_some("Background task finished")
                };
                return event
                    .filter(|_| query.is_empty())
                    .map(|text| HistoryMessage {
                        id: format!("{base}:0"),
                        role: "event".into(),
                        text: text.into(),
                        tool: None,
                        call_id: None,
                        truncated: false,
                        parent_call_id: None,
                    })
                    .into_iter()
                    .collect();
            }
            let role = match v["type"].as_str() {
                Some(r @ ("user" | "assistant")) => r,
                _ => return out,
            };
            let content = &v["message"]["content"];
            if content.is_string() {
                add(role, text(content), None, None);
            }
            if let Some(blocks) = content.as_array() {
                for b in blocks {
                    match b["type"].as_str() {
                        Some("text") => add(role, text(&b["text"]), None, None),
                        Some("tool_use") => add(
                            "assistant",
                            text(&b["input"]),
                            Some(b["name"].as_str().unwrap_or("Tool")),
                            b["id"].as_str(),
                        ),
                        Some("tool_result") => add(
                            "tool",
                            text(&b["content"]),
                            Some("Result"),
                            b["tool_use_id"].as_str(),
                        ),
                        Some("image") => add(role, "[Image attachment]".into(), None, None),
                        _ => (),
                    }
                }
            }
        }
        Kind::Codex => {
            let p = &v["payload"];
            if p["thread_id"].as_str().is_some_and(|s| s != session) {
                return out;
            }
            if modern && v["type"] == "event_msg" && p["type"] == "item_completed" {
                let i = &p["item"];
                match i["type"].as_str() {
                    Some("UserMessage") => add("user", text(&i["content"]), None, None),
                    Some("AgentMessage") => add("assistant", text(&i["content"]), None, None),
                    _ => (),
                }
            } else if v["type"] == "response_item" {
                match p["type"].as_str() {
                    Some("message") if !modern => {
                        if let Some(role @ ("user" | "assistant")) = p["role"].as_str() {
                            add(role, text(&p["content"]), None, None);
                        }
                    }
                    Some("function_call" | "custom_tool_call") => add(
                        "assistant",
                        text(p.get("arguments").unwrap_or(&p["input"])),
                        Some(p["name"].as_str().unwrap_or("Tool")),
                        p["call_id"].as_str(),
                    ),
                    Some("function_call_output" | "custom_tool_call_output") => add(
                        "tool",
                        text(&p["output"]),
                        Some("Result"),
                        p["call_id"].as_str(),
                    ),
                    _ => (),
                }
            }
        }
    }
    if !query.is_empty() {
        out.retain(|m| {
            m.text.to_lowercase().contains(query)
                || m.tool
                    .as_deref()
                    .is_some_and(|s| s.to_lowercase().contains(query))
        });
    }
    let mut remaining = RECORD_TEXT_BYTES;
    let mut omitted_blocks = out.len() > 24;
    out.truncate(24);
    for m in &mut out {
        let cap = remaining.min(4000);
        if !query.is_empty()
            && let Some(at) = m.text.to_lowercase().find(query)
        {
            let mut lower_offset = 0;
            let original = m
                .text
                .char_indices()
                .find_map(|(i, c)| {
                    let begins = lower_offset;
                    lower_offset += c.to_lowercase().map(char::len_utf8).sum::<usize>();
                    (begins >= at).then_some(i)
                })
                .unwrap_or(0);
            if original > cap.saturating_sub(query.len()) {
                let mut start = original.saturating_sub(200);
                while !m.text.is_char_boundary(start) {
                    start -= 1;
                }
                m.text.drain(..start);
                m.truncated = true;
            }
        }
        if m.text.len() > cap {
            let mut end = cap;
            while !m.text.is_char_boundary(end) {
                end -= 1;
            }
            m.text.truncate(end);
            m.truncated = true;
        }
        remaining -= m.text.len();
    }
    while serde_json::to_vec(&out).is_ok_and(|v| v.len() > 32 * 1024) {
        let mut reduced = false;
        for m in &mut out {
            if !m.text.is_empty() {
                let mut end = m.text.len() / 2;
                while !m.text.is_char_boundary(end) {
                    end -= 1;
                }
                m.text.truncate(end);
                m.truncated = true;
                reduced = true;
            }
        }
        if !reduced {
            out.pop();
            omitted_blocks = true;
        }
    }
    if omitted_blocks && let Some(last) = out.last_mut() {
        last.truncated = true;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::AgentSession;
    use serde_json::json;
    use std::io::Write;
    use std::path::PathBuf;

    const CLAUDE: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    const CODEX: &str = "01a10f07-b5ed-79a3-989d-fa10b96c5cab";

    fn setup(kind: &str, content: &str) -> (tempfile::TempDir, Transcripts, AgentInfo, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let id = if kind == "claude" { CLAUDE } else { CODEX };
        let path = if kind == "claude" {
            dir.path().join(format!("claude/-test/{id}.jsonl"))
        } else {
            dir.path()
                .join(format!("codex/2026/10/06/rollout-test-{id}.jsonl"))
        };
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        let t = Transcripts::new(
            Some(dir.path().join("claude")),
            Some(dir.path().join("codex")),
        );
        let agent = AgentInfo {
            terminal_id: "t1".into(),
            workspace_id: "w1".into(),
            pane_id: "w1:p1".into(),
            agent: Some(kind.into()),
            name: None,
            title: None,
            terminal_title_stripped: None,
            agent_status: "working".into(),
            state_change_seq: 1,
            cwd: Some("/test".into()),
            foreground_cwd: None,
            agent_session: Some(AgentSession { value: id.into() }),
            interactive_ready: true,
            launch_pending: false,
        };
        (dir, t, agent, path)
    }

    fn params(before: Option<String>, after: Option<String>, query: Option<&str>) -> HistoryParams {
        HistoryParams {
            terminal_id: protocol::TerminalId::new("t1").unwrap(),
            before,
            after,
            query: query.map(str::to_owned),
        }
    }

    fn row(n: usize, text: &str) -> String {
        json!({"type":"user", "uuid":format!("u{n}"), "sessionId":CLAUDE,
            "message":{"role":"user","content":text}})
        .to_string()
            + "\n"
    }

    fn append(path: &PathBuf, content: &str) {
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
    }

    #[test]
    fn real_formats_exclude_metadata_sidechains_and_duplicate_codex_events() {
        for (kind, fixture, count) in [
            (
                "claude",
                include_str!("../../tests/fixtures/transcripts/claude.jsonl"),
                4,
            ),
            (
                "codex",
                include_str!("../../tests/fixtures/transcripts/codex.jsonl"),
                3,
            ),
        ] {
            let (_dir, t, a, _) = setup(kind, fixture);
            let page = t.history(&a, &params(None, None, None));
            assert!(page.available);
            assert_eq!(page.messages.len(), count, "{kind}");
            assert_eq!(page.messages[0].role, "user");
            assert!(page.before.is_none());
            assert!(
                !page
                    .messages
                    .iter()
                    .any(|m| m.text.contains("environment_context")
                        || m.text.contains("Subagent report"))
            );
        }
    }

    #[test]
    fn codex_pages_search_and_partial_updates_use_the_same_session() {
        let header = json!({"type":"session_meta", "payload":{"id":CODEX,"cli_version":"0.160.1"}})
            .to_string()
            + "\n";
        let (_dir, t, a, path) = setup("codex", &header);
        let first = t.history(&a, &params(None, None, None));
        let row = |n| {
            json!({"type":"event_msg", "payload":{"type":"item_completed", "thread_id":CODEX,
            "item":{"type":"AgentMessage","id":format!("a{n}"),"content":[{"type":"Text","text":format!("Reply {n}")}]}}}).to_string() + "\n"
        };
        for n in 0..85 {
            append(&path, &row(n));
        }
        let mut p = t.history(&a, &params(None, None, None));
        let mut backward = p.messages.clone();
        while p.before.is_some() {
            p = t.history(&a, &params(p.before, None, None));
            backward.splice(0..0, p.messages.clone());
        }
        assert_eq!(backward.len(), 85);
        let mut cursor = first.after;
        let mut forward = vec![];
        loop {
            p = t.history(&a, &params(None, cursor, None));
            cursor = p.after;
            forward.extend(p.messages);
            if !p.more {
                break;
            }
        }
        assert_eq!(backward, forward);
        let search = t.history(&a, &params(None, None, Some("REPLY 42")));
        assert_eq!(search.messages[0].id, "a42:0");
        append(&path, row(85).trim_end());
        assert!(
            t.history(&a, &params(None, cursor.clone(), None))
                .messages
                .is_empty()
        );
        append(&path, "\n");
        p = t.history(&a, &params(None, cursor, None));
        assert_eq!(p.messages[0].id, "a85:0");
        assert_eq!(p.messages.len(), 1);
        append(&path, &row(86).replace(CODEX, CLAUDE));
        assert!(
            t.history(&a, &params(None, p.after, None))
                .messages
                .is_empty()
        );
    }

    #[test]
    fn modern_codex_response_mirrors_do_not_duplicate_messages_or_tools() {
        let mut fixture = include_str!("../../tests/fixtures/transcripts/codex.jsonl").to_owned();
        fixture.push_str(include_str!(
            "../../tests/fixtures/transcripts/codex-history-tools.jsonl"
        ));
        let (_dir, t, a, _) = setup("codex", &fixture);
        let p = t.history(&a, &params(None, None, None));
        assert_eq!(p.messages.len(), 5);
        assert_eq!(p.messages[3].tool.as_deref(), Some("exec_command"));
        assert_eq!(p.messages[4].role, "tool");
        assert_eq!(p.messages[3].call_id, p.messages[4].call_id);
    }

    #[test]
    fn codex_legacy_messages_and_both_tool_formats() {
        let fixture = include_str!("../../tests/fixtures/transcripts/codex-history-legacy.jsonl");
        let (_dir, t, a, _) = setup("codex", fixture);
        let page = t.history(&a, &params(None, None, None));
        assert_eq!(page.messages.len(), 6);
        assert_eq!(page.messages[1].tool.as_deref(), Some("exec_command"));
        assert_eq!(page.messages[2].role, "tool");
        assert_eq!(page.messages[3].tool.as_deref(), Some("apply_patch"));
        assert_eq!(page.messages[4].call_id.as_deref(), Some("patch1"));
        assert!(page.messages[5].text.contains("```rust"));
    }

    #[test]
    fn backwards_pages_and_incremental_forward_pages_have_no_gaps() {
        let (_dir, t, a, path) = setup("claude", "");
        let initial = t.history(&a, &params(None, None, None));
        for n in 0..135 {
            append(&path, &row(n, &format!("message {n}")));
        }
        let mut p = t.history(&a, &params(None, None, None));
        let latest = p.after.clone();
        let mut back = p.messages.clone();
        while p.before.is_some() {
            let old = p.before.clone();
            p = t.history(&a, &params(p.before, None, None));
            assert_ne!(old, p.before);
            back.splice(0..0, p.messages.clone());
        }
        assert_eq!(back.len(), 135);
        assert_eq!(back[0].id, "u0:0");
        assert_eq!(back[134].id, "u134:0");
        let mut cursor = initial.after;
        let mut forward = vec![];
        loop {
            let p = t.history(&a, &params(None, cursor, None));
            cursor = p.after;
            forward.extend(p.messages);
            if !p.more {
                break;
            }
        }
        assert_eq!(back, forward);
        assert_eq!(cursor, latest);
        let unchanged = t.history(&a, &params(None, cursor, None));
        assert!(unchanged.messages.is_empty());
    }

    #[test]
    fn partial_record_waits_for_newline_and_reconnect_reuses_cursor() {
        let (_dir, t, a, path) = setup("claude", &row(1, "hello"));
        let page = t.history(&a, &params(None, None, None));
        let more = row(2, "héllo from a partial write");
        append(&path, more.trim_end());
        let partial = t.history(&a, &params(None, page.after.clone(), None));
        assert!(partial.messages.is_empty());
        assert_eq!(partial.after, page.after);
        append(&path, "\n");
        let reconnect = t
            .history_reader()
            .history(&a, &params(None, partial.after, None));
        assert_eq!(reconnect.messages.len(), 1);
        assert_eq!(reconnect.messages[0].id, "u2:0");
    }

    #[test]
    fn search_visits_older_pages_and_matches_beyond_display_limit() {
        let mut content = row(0, &format!("{}hidden NEEDLE", "é".repeat(9000)));
        for n in 1..100 {
            content.push_str(&row(n, &"x".repeat(30000)));
        }
        let (_dir, t, a, _) = setup("claude", &content);
        let mut p = t.history(&a, &params(None, None, Some("needle")));
        let mut found = p.messages.clone();
        while p.before.is_some() {
            p = t.history(&a, &params(p.before, None, Some("needle")));
            found.extend(p.messages.clone());
        }
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "u0:0");
        assert!(found[0].truncated);
        assert!(found[0].text.contains("NEEDLE"));
    }

    #[test]
    fn missing_session_identity_never_guesses_and_recovers_when_reported() {
        for (kind, fixture) in [
            ("claude", row(1, "hello")),
            (
                "codex",
                include_str!("../../tests/fixtures/transcripts/codex-history-legacy.jsonl").into(),
            ),
        ] {
            let (_dir, reader, mut agent, _path) = setup(kind, &fixture);
            let identity = agent.agent_session.take();
            let page = reader.history(&agent, &params(None, None, None));
            assert!(!page.available && page.messages.is_empty());
            assert!(page.unavailable_reason.unwrap().contains("session ID"));
            agent.agent_session = identity;
            let page = reader.history(&agent, &params(None, None, None));
            assert!(page.available && !page.messages.is_empty());
            assert!(page.unavailable_reason.is_none());
        }
    }

    #[test]
    fn unavailable_is_distinct_from_empty_and_can_recover() {
        let (_dir, t, a, path) = setup("claude", "");
        let empty = t.history(&a, &params(None, None, None));
        assert!(empty.available && empty.messages.is_empty());
        std::fs::remove_file(&path).unwrap();
        assert!(!t.history(&a, &params(None, None, None)).available);
        std::fs::write(&path, row(0, "created later")).unwrap();
        assert_eq!(t.history(&a, &params(None, None, None)).messages.len(), 1);
    }

    #[test]
    fn stale_cursors_reject_session_changes_replacement_and_truncation() {
        let (_dir, t, mut a, path) = setup("claude", &row(1, "old"));
        let p = t.history(&a, &params(None, None, None));
        let old_session = p.session.clone();
        let moved = path.with_extension("old");
        std::fs::rename(&path, moved).unwrap();
        std::fs::write(&path, row(2, "new")).unwrap();
        let replaced = t.history(&a, &params(None, p.after, None));
        assert!(replaced.reset);
        assert_ne!(replaced.session, old_session);
        std::fs::write(&path, "").unwrap();
        assert!(t.history(&a, &params(None, replaced.after, None)).reset);
        a.agent_session = Some(AgentSession {
            value: "22222222-3333-4444-8555-666666666666".into(),
        });
        assert!(!t.history(&a, &params(None, None, None)).available);
    }

    #[test]
    fn oversized_records_make_progress_in_both_directions() {
        let (_dir, t, a, path) = setup("claude", "");
        let first = t.history(&a, &params(None, None, None));
        append(&path, &row(0, "before"));
        append(&path, &row(1, &"x".repeat(SCAN_BYTES as usize * 3)));
        append(&path, &row(2, "after"));
        let mut p = t.history(&a, &params(None, None, None));
        let mut ids = p.messages.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        let mut omitted = p.omitted;
        for _ in 0..10 {
            if p.before.is_none() {
                break;
            }
            p = t.history(&a, &params(p.before, None, None));
            omitted |= p.omitted;
            ids.extend(p.messages.iter().map(|m| m.id.clone()));
        }
        assert!(p.before.is_none());
        assert!(omitted);
        assert_eq!(ids, ["u2:0", "u0:0"]);
        let mut cursor = first.after;
        let mut ids = vec![];
        for _ in 0..10 {
            p = t.history(&a, &params(None, cursor, None));
            cursor = p.after.clone();
            ids.extend(p.messages.iter().map(|m| m.id.clone()));
            if !p.more {
                break;
            }
        }
        assert!(!p.more);
        assert_eq!(ids, ["u0:0", "u2:0"]);
    }

    #[test]
    fn output_fits_frame_even_with_escaping_and_many_blocks() {
        let content = json!({"type":"assistant","uuid":"a","message":{"content":
            (0..24).map(|_| json!({"type":"tool_use","name":"\\\"".repeat(80),"id":"\\\"".repeat(128),"input":"\u{0001}".repeat(1000)})).collect::<Vec<_>>()}}).to_string() + "\n";
        let (_dir, t, a, _) = setup("claude", &content);
        let p = t.history(&a, &params(None, None, None));
        assert!(!p.messages.is_empty());
        assert!(p.messages.iter().any(|m| m.truncated));
        let frame = protocol::ServerFrame::Result {
            id: 1,
            result: protocol::Response::History(p),
        };
        assert!(serde_json::to_vec(&frame).unwrap().len() < protocol::limits::MAX_FRAME_BYTES);
    }

    #[test]
    fn omitted_blocks_and_cross_session_cursors_are_explicit() {
        let content = json!({"type":"assistant", "uuid":"many", "message":{"content":
            (0..30).map(|_| json!({"type":"text","text":"block"})).collect::<Vec<_>>()}})
        .to_string()
            + "\n";
        let (_dir, t, mut a, path) = setup("claude", &content);
        let old = t.history(&a, &params(None, None, None));
        assert_eq!(old.messages.len(), 24);
        assert!(old.messages.last().unwrap().truncated);
        let id = "22222222-3333-4444-8555-666666666666";
        a.agent_session = Some(AgentSession { value: id.into() });
        std::fs::write(
            path.with_file_name(format!("{id}.jsonl")),
            row(1, "new session").replace(CLAUDE, id),
        )
        .unwrap();
        let new = t.history(&a, &params(None, old.after, None));
        assert!(new.reset);
        assert_eq!(new.messages.len(), 1);
        assert_eq!(new.messages[0].text, "new session");
    }

    #[test]
    fn malformed_records_foreign_sessions_and_symlinks_do_not_leak() {
        let content = row(0, "valid") + "broken\n" + &row(1, "foreign").replace(CLAUDE, CODEX);
        let (_dir, t, a, path) = setup("claude", &content);
        let p = t.history(&a, &params(None, None, None));
        assert_eq!(p.messages.len(), 1);
        let target = path.with_extension("secret");
        std::fs::rename(&path, &target).unwrap();
        std::os::unix::fs::symlink(target, path).unwrap();
        assert!(!t.history(&a, &params(None, None, None)).available);
    }

    const AGENT: &str = "a3f9c2e1d4b5a6978";

    fn subagent_files(path: &Path, agent: &str, meta: &str, records: &str) -> PathBuf {
        let dir = path.with_extension("").join("subagents");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("agent-{agent}.meta.json")), meta).unwrap();
        std::fs::write(dir.join(format!("agent-{agent}.jsonl")), records).unwrap();
        dir
    }

    fn line(v: Value) -> String {
        v.to_string() + "\n"
    }

    fn agent_call(uuid: &str, call: &str) -> String {
        line(
            json!({"type":"assistant","uuid":uuid,"sessionId":CLAUDE,"message":{"content":
            [{"type":"tool_use","id":call,"name":"Agent","input":{"prompt":"go"}}]}}),
        )
    }

    fn sub_line(uuid: &str, role: &str, content: Value) -> String {
        line(
            json!({"type":role,"uuid":uuid,"isSidechain":true,"sessionId":CLAUDE,
            "message":{"role":role,"content":content}}),
        )
    }

    #[test]
    fn claude_subagent_steps_are_tagged_with_their_agent_call() {
        let main = include_str!("../../tests/fixtures/transcripts/claude-subagent.jsonl");
        let meta = include_str!("../../tests/fixtures/transcripts/claude-subagent-agent.meta.json");
        let records = include_str!("../../tests/fixtures/transcripts/claude-subagent-agent.jsonl");
        let main = main.split_inclusive('\n').collect::<Vec<_>>();
        let records = records.split_inclusive('\n').collect::<Vec<_>>();
        let (_dir, t, a, path) = setup("claude", "");
        let initial = t.history(&a, &params(None, None, None));
        append(&path, &main[..3].concat());
        let dir = subagent_files(&path, AGENT, meta, records[0]);
        let launched = t.history(&a, &params(None, initial.after, None));
        assert_eq!(launched.messages.len(), 4);
        assert_eq!(
            launched.messages[1].call_id.as_deref(),
            Some("toolu_01AgentCall")
        );
        assert_eq!(
            launched.messages[2].parent_call_id.as_deref(),
            Some("toolu_01AgentCall")
        );
        append(
            &dir.join(format!("agent-{AGENT}.jsonl")),
            &records[1..].concat(),
        );
        append(&path, &main[3..].concat());
        let handed = t.history(&a, &params(None, launched.after, None));
        assert_eq!(handed.messages.len(), 9);
        assert_eq!(handed.messages[0].role, "event");
        assert_eq!(handed.messages[0].text, "Subagent finished: Review pairing");
        assert!(
            handed.messages[1..8]
                .iter()
                .all(|m| m.parent_call_id.is_some())
        );
        let page = t.history(&a, &params(None, None, None));
        assert_eq!(page.messages.len(), 12);
        let steps = page
            .messages
            .iter()
            .filter(|m| m.parent_call_id.is_some())
            .collect::<Vec<_>>();
        assert_eq!(steps.len(), 7);
        assert_eq!(
            page.messages[1].call_id.as_deref(),
            Some("toolu_01AgentCall")
        );
        assert!(
            page.messages[2..9]
                .iter()
                .all(|m| m.parent_call_id.as_deref() == Some("toolu_01AgentCall"))
        );
        assert_eq!(steps[0].role, "user");
        assert_eq!(steps[0].text, "Read pairing.rs and list risky unwraps.");
        assert_eq!(steps[1].tool.as_deref(), Some("Read"));
        assert_eq!(steps[2].role, "tool");
        assert_eq!(steps[1].call_id, steps[2].call_id);
        assert_eq!(steps[3].tool.as_deref(), Some("Grep"));
        assert!(steps[5].text.ends_with("Also check the retry loop."));
        assert_eq!(steps[6].text, "One risky unwrap: line 42 of pairing.rs.");
        assert!(
            !page
                .messages
                .iter()
                .any(|m| m.text.contains("system-reminder") || m.text.contains("Deferred tools"))
        );
        let search = t.history(&a, &params(None, None, Some("TOKEN.UNWRAP()")));
        assert_eq!(search.messages.len(), 1);
        assert_eq!(search.messages[0].id, "s6:0");
        assert_eq!(
            search.messages[0].parent_call_id.as_deref(),
            Some("toolu_01AgentCall")
        );
        let file = dir.join(format!("agent-{AGENT}.jsonl"));
        let target = dir.join("elsewhere.jsonl");
        std::fs::rename(&file, &target).unwrap();
        std::os::unix::fs::symlink(&target, &file).unwrap();
        assert_eq!(t.history(&a, &params(None, None, None)).messages.len(), 5);
        let session = path.with_extension("");
        std::fs::remove_dir_all(&session).unwrap();
        let outside = session.with_extension("outside");
        subagent_files(&outside.join("x"), AGENT, meta, &records.concat());
        std::os::unix::fs::symlink(&outside, &session).unwrap();
        assert_eq!(t.history(&a, &params(None, None, None)).messages.len(), 5);
    }

    #[test]
    fn large_subagents_are_trimmed_to_the_page_and_every_older_page_brings_new_records() {
        let (_dir, t, a, path) = setup("claude", "");
        let initial = t.history(&a, &params(None, None, None));
        let mut records = sub_line("prompt", "user", json!("Subagent prompt"));
        for n in 0..60 {
            records.push_str(&sub_line(
                &format!("use{n}"),
                "assistant",
                json!([{"type":"tool_use",
                "id":format!("c{n}"),"name":"Read","input":{"file_path":"x".repeat(500)}}]),
            ));
            records.push_str(&sub_line(
                &format!("result{n}"),
                "user",
                json!([{"type":"tool_result",
                "tool_use_id":format!("c{n}"),"content":"y".repeat(500)}]),
            ));
        }
        records.push_str(&sub_line(
            "reply",
            "assistant",
            json!([{"type":"text","text":"Subagent reply"}]),
        ));
        subagent_files(&path, AGENT, r#"{"toolUseId":"call1"}"#, &records);
        for n in 0..50 {
            append(&path, &row(n, &format!("message {n}")));
        }
        append(&path, &agent_call("call", "call1"));
        append(
            &path,
            &line(json!({"type":"user","uuid":"launch","sessionId":CLAUDE,
            "message":{"content":[{"type":"tool_result","tool_use_id":"call1","content":"launched"}]},
            "toolUseResult":{"agentId":AGENT,"status":"async_launched"}})),
        );
        append(
            &path,
            &line(
                json!({"type":"user","uuid":"handback","isMeta":true,"sessionId":CLAUDE,
            "origin":{"kind":"peer","from":AGENT},"message":{"content":"Handed back"}}),
            ),
        );
        append(
            &path,
            &line(json!({"type":"assistant","uuid":"done","sessionId":CLAUDE,
            "message":{"content":[{"type":"text","text":"All done"}]}})),
        );
        let fits = |p: &HistoryPage| {
            p.messages.len() <= 40 && serde_json::to_vec(&p.messages).unwrap().len() <= PAGE_BYTES
        };
        let mut p = t.history(&a, &params(None, None, None));
        let texts = p
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>();
        for text in ["All done", "launched", "Subagent prompt", "Subagent reply"] {
            assert!(texts.contains(&text), "{text}");
        }
        assert!(p.omitted && fits(&p));
        let mut back = p.messages.clone();
        while p.before.is_some() {
            p = t.history(&a, &params(p.before, None, None));
            assert!(!p.messages.is_empty() && fits(&p));
            back.splice(0..0, p.messages.clone());
        }
        let ids = back.iter().map(|m| m.id.clone()).collect::<HashSet<_>>();
        assert_eq!(ids.len(), back.len());
        let main = |messages: &[HistoryMessage]| {
            messages
                .iter()
                .filter(|m| m.parent_call_id.is_none())
                .map(|m| m.id.clone())
                .collect::<Vec<_>>()
        };
        let mut expected = (0..50).map(|n| format!("u{n}:0")).collect::<Vec<_>>();
        expected.extend(["call:0", "launch:0", "handback:0", "done:0"].map(String::from));
        assert_eq!(main(&back), expected);
        let mut cursor = initial.after;
        let mut forward = vec![];
        loop {
            let p = t.history(&a, &params(None, cursor, None));
            assert!(fits(&p));
            cursor = p.after;
            forward.extend(p.messages);
            if !p.more {
                break;
            }
        }
        assert_eq!(main(&forward), expected);
    }

    #[test]
    fn subagents_beyond_the_window_keep_their_prompt_without_empty_pages() {
        let (_dir, t, a, path) = setup("claude", "");
        let records = [
            sub_line("prompt", "user", json!("Prompt")),
            sub_line(
                "big",
                "user",
                json!("z".repeat(SCAN_BYTES as usize * 5 / 4)),
            ),
            sub_line(
                "reply",
                "assistant",
                json!([{"type":"text","text":"Reply"}]),
            ),
        ]
        .concat();
        subagent_files(&path, AGENT, r#"{"toolUseId":"call1"}"#, &records);
        append(&path, &agent_call("call", "call1"));
        for n in 0..5 {
            append(
                &path,
                &line(json!({"type":"system","uuid":format!("sys{n}"),"sessionId":CLAUDE})),
            );
        }
        let p = t.history(&a, &params(None, None, None));
        let texts = p
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>();
        assert!(
            texts.contains(&"Prompt") && texts.contains(&"Reply"),
            "{texts:?}"
        );
        assert!(p.before.is_none() && p.omitted);
    }

    #[test]
    fn subagent_reads_share_one_window_and_defer_to_the_next_page() {
        let (_dir, t, a, path) = setup("claude", "");
        for n in 0..2 {
            let agent = format!("agent{n}");
            let records = [
                sub_line(&format!("prompt{n}"), "user", json!(format!("Prompt {n}"))),
                sub_line(
                    &format!("big{n}"),
                    "user",
                    json!("z".repeat(SCAN_BYTES as usize * 3 / 4)),
                ),
                sub_line(
                    &format!("reply{n}"),
                    "assistant",
                    json!([{"type":"text","text":format!("Reply {n}")}]),
                ),
            ]
            .concat();
            subagent_files(
                &path,
                &agent,
                &format!(r#"{{"toolUseId":"call{n}"}}"#),
                &records,
            );
            append(&path, &agent_call(&format!("task{n}"), &format!("call{n}")));
        }
        let first = t.history(&a, &params(None, None, None));
        let texts = |p: &HistoryPage| {
            p.messages
                .iter()
                .map(|m| m.text.clone())
                .collect::<Vec<_>>()
        };
        assert!(texts(&first).contains(&"Prompt 1".into()));
        assert!(!texts(&first).iter().any(|s| s.ends_with(" 0")));
        let older = t.history(&a, &params(first.before, None, None));
        assert!(texts(&older).contains(&"Prompt 0".into()));
        assert!(texts(&older).contains(&"Reply 0".into()));
        assert!(older.before.is_none());
    }

    #[test]
    fn a_tail_window_starting_on_a_record_keeps_that_record() {
        let (_dir, t, a, path) = setup("claude", "");
        let prompt = sub_line("prompt", "user", json!("Prompt"));
        let big = sub_line("big", "user", json!("z".repeat(SCAN_BYTES as usize)));
        let next = sub_line(
            "next",
            "assistant",
            json!([{"type":"text","text":"First in window"}]),
        );
        let reply = sub_line(
            "reply",
            "assistant",
            json!([{"type":"text","text":"Reply"}]),
        );
        let pad = SCAN_BYTES as usize - next.len() - reply.len();
        let filler = sub_line(
            "filler",
            "user",
            json!("w".repeat(pad - sub_line("filler", "user", json!("")).len())),
        );
        let tail = [next, filler, reply].concat();
        assert_eq!(tail.len(), SCAN_BYTES as usize);
        subagent_files(
            &path,
            AGENT,
            r#"{"toolUseId":"call1"}"#,
            &[prompt, big, tail].concat(),
        );
        append(&path, &agent_call("call", "call1"));
        let p = t.history(&a, &params(None, None, None));
        let texts = p
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>();
        assert!(
            texts.contains(&"Prompt") && texts.contains(&"Reply"),
            "{texts:?}"
        );
        assert!(texts.contains(&"First in window"), "{texts:?}");
    }

    #[test]
    fn harness_injected_user_records_show_only_bounded_markers() {
        let user = |uuid: &str, extra: Value, content: Value| {
            let mut v = json!({"type":"user","uuid":uuid,"sessionId":CLAUDE,
                "message":{"role":"user","content":content}});
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            line(v)
        };
        let content = [
            user(
                "typed",
                json!({"origin":{"kind":"human"},"promptSource":"typed"}),
                json!("injected? no, typed"),
            ),
            user(
                "handback",
                json!({"isMeta":true,"origin":{"kind":"peer","from":AGENT,"handback":true}}),
                json!("injected hand-back"),
            ),
            user(
                "notice",
                json!({"origin":{"kind":"task-notification"},"promptSource":"system"}),
                json!("<task-notification>injected</task-notification>"),
            ),
            user(
                "skill",
                json!({"isMeta":true}),
                json!([{"type":"text","text":"injected skill"}]),
            ),
            user(
                "summary",
                json!({"isCompactSummary":true,"isVisibleInTranscriptOnly":true}),
                json!("injected summary"),
            ),
            user(
                "interrupt",
                json!({}),
                json!([{"type":"text","text":"[Request interrupted by user]"}]),
            ),
            user("bash", json!({}), json!("<bash-input>ls</bash-input>")),
        ]
        .concat();
        let (_dir, t, a, path) = setup("claude", &content);
        let p = t.history(&a, &params(None, None, None));
        let ids = p.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>();
        assert_eq!(
            ids,
            ["typed:0", "handback:0", "notice:0", "interrupt:0", "bash:0"]
        );
        assert_eq!(p.messages[1].role, "event");
        assert_eq!(p.messages[1].text, "Subagent finished");
        assert_eq!(p.messages[2].role, "event");
        assert_eq!(p.messages[2].text, "Background task finished");
        let label = format!("Fix\u{1b}[31m the\nbug {}", "x".repeat(300));
        subagent_files(&path, AGENT, &json!({"description": label}).to_string(), "");
        let p = t.history(&a, &params(None, None, None));
        let text = &p.messages[1].text;
        assert!(
            text.starts_with("Subagent finished: Fix[31m thebug x"),
            "{text}"
        );
        assert_eq!(text.chars().count(), 120);
        std::fs::write(
            path.with_extension("")
                .join(format!("subagents/agent-{AGENT}.meta.json")),
            r#"{"agentType":"general-purpose"}"#,
        )
        .unwrap();
        let p = t.history(&a, &params(None, None, None));
        assert_eq!(p.messages[1].text, "Subagent finished: general-purpose");
        for query in ["injected", "finished"] {
            let search = t.history(&a, &params(None, None, Some(query)));
            assert_eq!(search.messages.len(), usize::from(query == "injected"));
        }
    }
}
