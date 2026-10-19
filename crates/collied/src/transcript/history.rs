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
            copilot_sessions: self.copilot_sessions.clone(),
            ..Self::new(self.claude_projects.clone(), self.codex_sessions.clone())
        }
    }

    pub fn history(&self, agent: &AgentInfo, params: &HistoryParams) -> HistoryPage {
        self.read_history(agent, params).unwrap_or(HistoryPage {
            session: None,
            available: false,
            unavailable_reason: Some(if self.live_id(agent).is_some() {
                "The transcript for this session is not readable yet. Terminal remains available."
            } else if agent.agent.as_deref() == Some("copilot") {
                "Chat cannot yet confirm which Copilot CLI conversation this pane shows, so it does not guess. It retries once the pane’s title shows the session’s name. Terminal remains available."
            } else {
                "The agent has not reported its session ID to herdr. Chat cannot safely identify this conversation. Check the agent’s herdr SessionStart integration, then resume the session."
            }.into()),
            reset: false,
            messages: vec![],
            before: None,
            after: None,
            more: false,
            omitted: false,
        })
    }

    /// Copilot's `/new` can switch conversations while herdr's ID stays, so a page is
    /// current only while the pane still selects the session it was read from.
    pub fn history_current(&self, agent: &AgentInfo, page: &HistoryPage) -> bool {
        let Some(session) = &page.session else {
            return true;
        };
        agent.agent.as_deref() != Some("copilot")
            || self.live_id(agent).is_some_and(|id| {
                session
                    .strip_prefix("copilot:")
                    .and_then(|s| s.strip_prefix(id.as_str()))
                    .is_some_and(|s| s.starts_with(':'))
            })
    }

    fn read_history(&self, agent: &AgentInfo, p: &HistoryParams) -> Option<HistoryPage> {
        let kind = match agent.agent.as_deref()? {
            "claude" => Kind::Claude,
            "codex" => Kind::Codex,
            "copilot" => Kind::Copilot,
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
        if matches!(kind, Kind::Copilot)
            && serde_json::from_slice::<Value>(first)
                .ok()
                .is_none_or(|v| v["type"] != "session.start" || v["data"]["sessionId"] != id)
        {
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
        if p.latest {
            let mut messages = Vec::new();
            let (mut users, mut replies, mut size) = (0, 0, 0);
            'records: for (a, b) in positions.into_iter().rev() {
                let Ok(v) = serde_json::from_slice::<Value>(&bytes[a..b]) else {
                    continue;
                };
                if matches!(kind, Kind::Copilot) && v["agentId"].is_string() {
                    continue;
                }
                let fallback = (start + a as u64).to_string();
                for m in normalize(&v, kind, id, modern, fallback, "", None)
                    .into_iter()
                    .rev()
                {
                    let count = match m.role.as_str() {
                        _ if m.tool.is_some() || m.text.trim().is_empty() => continue,
                        "user" => &mut users,
                        "assistant" => &mut replies,
                        _ => continue,
                    };
                    if *count == 2 {
                        continue;
                    }
                    size += serde_json::to_vec(&m).ok()?.len();
                    if size > PAGE_BYTES {
                        break 'records;
                    }
                    *count += 1;
                    messages.push(m);
                }
                if users == 2 && replies == 2 {
                    break;
                }
            }
            messages.reverse();
            return Some(HistoryPage {
                session: Some(session),
                available: true,
                unavailable_reason: None,
                reset: false,
                messages,
                before: None,
                after: None,
                more: false,
                omitted: false,
            });
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
        let agents = match kind {
            Kind::Copilot => copilot_agents(&bytes, &positions),
            _ => HashMap::new(),
        };
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
                Ok(v) if matches!(kind, Kind::Copilot) && v["agentId"].is_string() => {
                    match copilot_parent(v, &agents) {
                        Some(parent) => {
                            normalize(v, kind, id, modern, fallback, &query, Some(parent))
                        }
                        None => Vec::new(),
                    }
                }
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

/// Copilot CLI writes subagent records into the parent's file, tagged with the child's
/// `agentId`; its prompt names no call, so the call comes from `subagent.started` or a
/// sibling record in the same window. A child whose call is not found there is left out.
fn copilot_agents(bytes: &[u8], positions: &[(usize, usize)]) -> HashMap<String, String> {
    let mut agents = HashMap::new();
    for (a, b) in positions {
        let line = &bytes[*a..*b];
        if !line.windows(9).any(|w| w == b"\"agentId\"") {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if let (Some(agent), Some(call)) = (
            v["agentId"].as_str(),
            if v["type"] == "subagent.started" {
                v["data"]["toolCallId"].as_str()
            } else {
                v["data"]["parentToolCallId"].as_str()
            },
        ) {
            agents
                .entry(agent.to_owned())
                .or_insert_with(|| call.to_owned());
        }
    }
    agents
}

fn copilot_parent<'a>(v: &'a Value, agents: &'a HashMap<String, String>) -> Option<&'a str> {
    v["data"]["parentToolCallId"]
        .as_str()
        .or_else(|| agents.get(v["agentId"].as_str()?).map(String::as_str))
        .filter(|s| !s.is_empty() && s.len() <= 128)
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

pub(super) fn local(v: &Value) -> Option<&str> {
    v["message"]["content"]
        .as_str()
        .filter(|_| v["promptSource"].is_null() && v["isMeta"] != true)
}

pub(super) fn command(s: &str) -> Option<String> {
    if !s.starts_with("<command-name>") && !s.starts_with("<command-message>") {
        return None;
    }
    let tag = |t: &str| {
        s.split_once(&format!("<{t}>"))?
            .1
            .split_once(&format!("</{t}>"))
            .map(|(x, _)| x)
    };
    let name = tag("command-name").filter(|n| n.starts_with('/'))?;
    let line = format!("{name} {}", tag("command-args").unwrap_or(""))
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control())
        .take(120)
        .collect::<String>();
    Some(line.trim().to_owned())
}

/// Claude 5.1 and later write the progress update before a tool call as a second thinking block.
fn writes_updates_as_thinking(model: &str) -> bool {
    let number = |p: Option<&str>| {
        let p = p?;
        let n = p.bytes().take_while(u8::is_ascii_digit).count();
        (1..=2).contains(&n).then(|| p[..n].parse::<u32>().ok())?
    };
    let mut parts = model.split('-');
    if parts.next() != Some("claude")
        || !parts
            .next()
            .is_some_and(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_lowercase()))
    {
        return false;
    }
    let Some(major) = number(parts.next()) else {
        return false;
    };
    (major, number(parts.next()).unwrap_or(0)) >= (5, 1)
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
        Kind::Copilot => v["id"].as_str(),
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
            let local = local(v);
            let slash = local.and_then(command);
            if parent.is_none()
                && v["type"] == "user"
                && (v["isMeta"] == true
                    || v["isCompactSummary"] == true
                    || notice
                    || slash.is_some()
                    || local.is_some_and(|s| s.starts_with("<local-command-stdout>")))
            {
                let event = if v["isMeta"] == true && v["origin"]["kind"] == "peer" {
                    Some("Subagent finished".into())
                } else if notice {
                    Some("Background task finished".into())
                } else {
                    slash
                };
                return event
                    .filter(|_| query.is_empty())
                    .map(|text| HistoryMessage {
                        id: format!("{base}:0"),
                        role: "event".into(),
                        text,
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
                let first = v["apiBlockIndex"].as_u64().unwrap_or(0);
                let updates = parent.is_none()
                    && v["message"]["model"]
                        .as_str()
                        .is_some_and(writes_updates_as_thinking);
                for (i, b) in blocks.iter().enumerate() {
                    match b["type"].as_str() {
                        Some("text") => add(role, text(&b["text"]), None, None),
                        // Thinking blocks lead a message, so a later one follows another thinking block.
                        Some("thinking") if updates && first + i as u64 > 0 => {
                            add(role, text(&b["thinking"]), None, None)
                        }
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
        Kind::Copilot => {
            let d = &v["data"];
            match v["type"].as_str() {
                // Other origins are injected: autopilot, `skill-*`, and `agent-<id>` outside a child.
                Some("user.message")
                    if d["source"].is_null()
                        || d["source"] == "user"
                        || parent.is_some()
                            && d["source"]
                                .as_str()
                                .is_some_and(|s| s.starts_with("agent-")) =>
                {
                    add("user", text(&d["content"]), None, None);
                }
                Some("assistant.message") => {
                    add("assistant", text(&d["content"]), None, None);
                    for r in d["toolRequests"].as_array().into_iter().flatten() {
                        add(
                            "assistant",
                            text(&r["arguments"]),
                            Some(r["name"].as_str().unwrap_or("Tool")),
                            r["toolCallId"].as_str(),
                        );
                    }
                }
                Some("tool.execution_complete") => {
                    let result = &d["result"]["content"];
                    add(
                        "tool",
                        text(if result.is_null() {
                            &d["error"]["message"]
                        } else {
                            result
                        }),
                        Some("Result"),
                        d["toolCallId"].as_str(),
                    );
                }
                Some("system.notification") if parent.is_none() && query.is_empty() => {
                    match d["kind"]["type"].as_str() {
                        Some("agent_idle") => add("event", "Subagent finished".into(), None, None),
                        Some("shell_completed") => {
                            add("event", "Background task finished".into(), None, None)
                        }
                        _ => (),
                    }
                }
                _ => (),
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
            latest: false,
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
    fn claude_progress_updates_written_as_thinking_show_as_agent_text() {
        let fixture =
            include_str!("../../tests/fixtures/transcripts/claude-thinking-updates.jsonl");
        let update = "The timeout comes from the retry loop, so I'll read its test next.\n\n";
        let rows = |page: HistoryPage| {
            page.messages
                .into_iter()
                .map(|m| (m.role, m.tool, m.text))
                .collect::<Vec<_>>()
        };
        let (_dir, t, a, _) = setup("claude", fixture);
        let page = rows(t.history(&a, &params(None, None, None)));
        assert_eq!(page.len(), 7);
        assert_eq!(page[1], ("assistant".into(), None, update.into()));
        assert_eq!(page[2].1.as_deref(), Some("Bash"));
        assert!(!page.iter().any(|m| m.2.contains("stays hidden")));
        let latest = HistoryParams {
            latest: true,
            ..params(None, None, None)
        };
        let replies = rows(t.history(&a, &latest))
            .into_iter()
            .filter(|m| m.0 == "assistant")
            .map(|m| m.2)
            .collect::<Vec<_>>();
        assert_eq!(replies, [update, "The retry loop never resets its timer."]);

        let (_dir, t, a, _) = setup(
            "claude",
            &fixture.replace("claude-opus-5-5", "claude-opus-5"),
        );
        let page = rows(t.history(&a, &params(None, None, None)));
        assert_eq!(page.len(), 6);
        assert!(!page.iter().any(|m| m.2 == update));
    }

    #[test]
    fn only_a_later_thinking_block_of_a_main_thread_5_1_message_is_shown() {
        let record = |model: &str, index: Option<u64>, content: Value| {
            let mut v = json!({"type":"assistant","uuid":"a1","sessionId":CLAUDE,
                "message":{"model":model,"role":"assistant","content":content}});
            if let Some(i) = index {
                v["apiBlockIndex"] = i.into();
            }
            v
        };
        let shown = |v: &Value, parent: Option<&str>| {
            normalize(v, Kind::Claude, CLAUDE, false, "f".into(), "", parent)
                .into_iter()
                .filter(|m| m.tool.is_none())
                .map(|m| m.text)
                .collect::<Vec<_>>()
        };
        let thinking = |s: &str| json!({"type":"thinking","thinking":s,"signature":"c2ln"});
        let tool = json!({"type":"tool_use","id":"c1","name":"Bash","input":{}});
        let both = json!([thinking("Reasoning"), thinking("Update"), tool]);
        assert_eq!(
            shown(&record("claude-opus-5-5", None, both.clone()), None),
            ["Update"]
        );
        assert!(shown(&record("claude-opus-5-5", None, both.clone()), Some("c0")).is_empty());
        assert!(shown(&record("claude-opus-5", None, both), None).is_empty());
        let lone = json!([thinking("Reasoning")]);
        assert!(shown(&record("claude-fable-5-1", Some(0), lone.clone()), None).is_empty());
        assert!(shown(&record("claude-fable-5-1", None, lone), None).is_empty());
        let later = json!([thinking("Update")]);
        assert_eq!(
            shown(&record("claude-fable-5-1", Some(1), later.clone()), None),
            ["Update"]
        );
        assert!(shown(&record("claude-haiku-4-5-20251001", Some(1), later), None).is_empty());
    }

    #[test]
    fn claude_models_from_5_1_write_updates_as_thinking() {
        for model in [
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-fable-5-1",
            "claude-opus-5-1-20261001",
            "claude-opus-5-5[1m]",
            "claude-opus-6",
            "claude-opus-10-0",
        ] {
            assert!(writes_updates_as_thinking(model), "{model}");
        }
        for model in [
            "claude-opus-5",
            "claude-opus-5-20260101",
            "claude-haiku-4-5-20251001",
            "claude-sonnet-4-20250514",
            "claude-3-5-sonnet-20241022",
            "<synthetic>",
            "gpt-5-5",
            "",
        ] {
            assert!(!writes_updates_as_thinking(model), "{model}");
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
    fn latest_reads_the_last_two_user_and_agent_messages_without_tools() {
        let reply = |uuid: &str, content: Value| {
            line(json!({"type":"assistant","uuid":uuid,"sessionId":CLAUDE,
                "message":{"role":"assistant","content":content}}))
        };
        let (_dir, t, a, path) = setup("claude", &row(0, "Only prompt"));
        let latest = HistoryParams {
            latest: true,
            ..params(None, None, None)
        };
        let texts = |page: HistoryPage| {
            assert!(page.available && page.before.is_none() && page.after.is_none());
            page.messages
                .into_iter()
                .map(|m| (m.role, m.text))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            texts(t.history(&a, &latest)),
            [("user".into(), "Only prompt".into())]
        );
        append(
            &path,
            &[
                reply("a1", json!("Reply one")),
                row(1, "Second"),
                reply(
                    "a2",
                    json!([{"type":"text","text":"Reply two"},
                    {"type":"tool_use","id":"c1","name":"Bash","input":{"command":"ls"}}]),
                ),
                line(
                    json!({"type":"user","uuid":"r1","sessionId":CLAUDE,"message":{"role":"user",
                    "content":[{"type":"tool_result","tool_use_id":"c1","content":"out"}]}}),
                ),
                reply("a3", json!("Reply three")),
                row(2, "Third"),
            ]
            .concat(),
        );
        assert_eq!(
            texts(t.history(&a, &latest)),
            [
                ("user".into(), "Second".into()),
                ("assistant".into(), "Reply two".into()),
                ("assistant".into(), "Reply three".into()),
                ("user".into(), "Third".into()),
            ]
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

    #[test]
    fn slash_commands_show_as_markers_and_hide_their_output() {
        let user = |uuid: &str, extra: Value, content: Value| {
            let mut v = json!({"type":"user","uuid":uuid,"sessionId":CLAUDE,
                "message":{"role":"user","content":content}});
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            line(v)
        };
        let invoke = |name: &str, args: &str| {
            format!(
                "<command-name>{name}</command-name>\n            <command-message>{}</command-message>\n            <command-args>{args}</command-args>",
                &name[1..]
            )
        };
        let stdout = |text: &str| {
            json!(format!(
                "<local-command-stdout>{text}</local-command-stdout>"
            ))
        };
        let content = [
            user("exit", json!({}), json!(invoke("/exit", ""))),
            user("exit-out", json!({}), stdout("(no content)")),
            user("effort", json!({}), json!(invoke("/effort", "high"))),
            user("effort-out", json!({}), stdout("Set effort level to high")),
            user(
                "skill",
                json!({"origin":{"kind":"human"}}),
                json!("<command-message>review</command-message>\n<command-name>/review</command-name>\n<command-args>pr 12</command-args>"),
            ),
            user(
                "skill-body",
                json!({"isMeta":true}),
                json!([{"type":"text","text":"<command-message>review</command-message>\n<command-name>review</command-name>\n<skill-format>true</skill-format>\nskill body"}]),
            ),
            user(
                "long",
                json!({}),
                json!(invoke("/plan", &format!("do\u{1b}[31m it\nnow {}", "x".repeat(300)))),
            ),
            line(json!({"type":"system","subtype":"local_command","uuid":"sys","sessionId":CLAUDE,
                "isMeta":false,"level":"info","content":invoke("/model", "")})),
            line(json!({"type":"system","subtype":"local_command","uuid":"sys-out","sessionId":CLAUDE,
                "isMeta":false,"level":"info","content":"<local-command-stdout></local-command-stdout>",
                "commandRun":{"command":"model","args":""}})),
            user(
                "typed",
                json!({"origin":{"kind":"human"},"promptSource":"typed"}),
                json!(invoke("/exit", "typed")),
            ),
            user(
                "typed-out",
                json!({"origin":{"kind":"human"},"promptSource":"typed"}),
                stdout("typed"),
            ),
            user("angle", json!({}), json!("<div>typed exit</div>")),
        ]
        .concat();
        let (_dir, t, a, _path) = setup("claude", &content);
        let p = t.history(&a, &params(None, None, None));
        let shown = p
            .messages
            .iter()
            .map(|m| (m.id.as_str(), m.role.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            shown,
            [
                ("exit:0", "event"),
                ("effort:0", "event"),
                ("skill:0", "event"),
                ("long:0", "event"),
                ("typed:0", "user"),
                ("typed-out:0", "user"),
                ("angle:0", "user"),
            ]
        );
        assert_eq!(p.messages[0].text, "/exit");
        assert_eq!(p.messages[1].text, "/effort high");
        assert_eq!(p.messages[2].text, "/review pr 12");
        let long = &p.messages[3].text;
        assert!(long.starts_with("/plan do[31m it now xx"));
        assert_eq!(long.chars().count(), 120);
        for (query, hits) in [("exit", 2), ("effort", 0), ("review", 0), ("no content", 0)] {
            let search = t.history(&a, &params(None, None, Some(query)));
            assert_eq!(search.messages.len(), hits, "{query}");
        }
    }

    const COPILOT: &str = "20000000-0000-4000-8000-000000000002";
    const COPILOT_HISTORY: &str =
        include_str!("../../tests/fixtures/transcripts/copilot-history.jsonl");

    struct Copilot {
        _dir: tempfile::TempDir,
        t: Transcripts,
        a: AgentInfo,
        root: PathBuf,
        _holds: Vec<std::fs::File>,
    }

    impl Copilot {
        /// A pane whose process holds `COPILOT`, named in the pane's title; herdr's id is stale.
        fn new(content: &str) -> Self {
            let (dir, mut t, mut a, _) = setup("copilot", "");
            let root = dir.path().join("session-state");
            t.copilot_sessions = Some(root.clone());
            a.terminal_title_stripped = Some("Fixture conversation - GitHub Copilot".into());
            let pid = std::process::id();
            t.set_pane(
                "t1".into(),
                "w1:p1".into(),
                Some(crate::herdr::PaneProcesses {
                    pids: vec![pid],
                    attach: None,
                }),
            );
            let mut c = Self {
                _dir: dir,
                t,
                a,
                root,
                _holds: vec![],
            };
            c.session(COPILOT, "Fixture conversation", content);
            c
        }

        fn session(&mut self, id: &str, name: &str, content: &str) {
            let dir = self.root.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("workspace.yaml"),
                format!("id: {id}\ncwd: /test\nname: {name}\n"),
            )
            .unwrap();
            std::fs::write(dir.join("events.jsonl"), content.replace(COPILOT, id)).unwrap();
            let pid = std::process::id();
            std::fs::write(dir.join(format!("inuse.{pid}.lock")), pid.to_string()).unwrap();
            let hold = std::fs::File::create(dir.join(format!("inuse.{pid}.hold"))).unwrap();
            hold.lock().unwrap();
            self._holds.push(hold);
        }

        fn path(&self) -> PathBuf {
            self.root.join(COPILOT).join("events.jsonl")
        }

        fn history(&self, p: HistoryParams) -> HistoryPage {
            self.t.history_reader().history(&self.a, &p)
        }
    }

    fn copilot_header() -> String {
        COPILOT_HISTORY.split_inclusive('\n').next().unwrap().into()
    }

    fn copilot_row(n: usize, text: &str) -> String {
        line(json!({"type":"assistant.message","id":format!("c{n}"),
            "data":{"content":text,"toolRequests":[],"turnId":"0"}}))
    }

    #[test]
    fn latest_counts_only_readable_text_for_every_agent() {
        let latest = HistoryParams {
            latest: true,
            ..params(None, None, None)
        };
        let texts = |page: HistoryPage| {
            page.messages
                .into_iter()
                .map(|m| (m.role, m.text))
                .collect::<Vec<_>>()
        };
        let expected: Vec<(String, String)> = vec![
            ("user".into(), "Prompt".into()),
            ("assistant".into(), "Answer two".into()),
            ("assistant".into(), "Mixed".into()),
        ];
        let reply = |uuid: &str, content: Value| {
            line(json!({"type":"assistant","uuid":uuid,"sessionId":CLAUDE,
                "message":{"role":"assistant","content":content}}))
        };
        let tool = json!({"type":"tool_use","id":"c1","name":"Bash","input":{"command":"ls"}});
        let claude = [
            row(0, "Prompt"),
            reply("a1", json!("Answer one")),
            reply("a2", json!("Answer two")),
            reply("a3", json!([{"type":"text","text":"Mixed"}, tool])),
            line(
                json!({"type":"user","uuid":"r1","sessionId":CLAUDE,"message":{"role":"user",
                "content":[{"type":"tool_result","tool_use_id":"c1","content":"out"}]}}),
            ),
            reply("a4", json!([{"type":"thinking","thinking":"Hidden"}])),
            reply("a5", json!([{"type":"text","text":" \n "}])),
            reply("a6", json!([tool])),
        ]
        .concat();
        let (_dir, t, a, _) = setup("claude", &claude);
        assert_eq!(texts(t.history(&a, &latest)), expected);

        let item = |id: &str, kind: &str, text: &str| {
            line(
                json!({"type":"event_msg","payload":{"type":"item_completed","thread_id":CODEX,
                "item":{"type":kind,"id":id,"content":[{"type":"Text","text":text}]}}}),
            )
        };
        let response = |payload: Value| line(json!({"type":"response_item","payload":payload}));
        let codex = [
            line(json!({"type":"session_meta","payload":{"id":CODEX,"cli_version":"0.160.1"}})),
            item("u1", "UserMessage", "Prompt"),
            item("a1", "AgentMessage", "Answer one"),
            item("a2", "AgentMessage", "Answer two"),
            item("a3", "AgentMessage", "Mixed"),
            response(json!({"type":"function_call","name":"exec_command","arguments":"{}","call_id":"c1"})),
            response(json!({"type":"function_call_output","output":"out","call_id":"c1"})),
            response(json!({"type":"reasoning","summary":[{"type":"summary_text","text":"Hidden"}]})),
            item("a4", "Reasoning", "Hidden"),
            item("a5", "AgentMessage", " \n "),
        ]
        .concat();
        let (_dir, t, a, _) = setup("codex", &codex);
        assert_eq!(texts(t.history(&a, &latest)), expected);

        let assistant = |id: &str, content: &str, tools: Value| {
            line(
                json!({"type":"assistant.message","id":id,"data":{"content":content,
                "reasoningText":"Hidden","toolRequests":tools,"turnId":"0"}}),
            )
        };
        let request = json!([{"toolCallId":"c1","name":"bash","arguments":{"command":"ls"}}]);
        let copilot = [
            copilot_header(),
            line(json!({"type":"user.message","id":"u1","data":{"content":"Prompt"}})),
            assistant("a1", "Answer one", json!([])),
            assistant("a2", "Answer two", json!([])),
            assistant("a3", "Mixed", request.clone()),
            line(json!({"type":"tool.execution_complete","id":"r1",
                "data":{"toolCallId":"c1","result":{"content":"out"}}})),
            assistant("a4", "", request),
            assistant("a5", " \n ", json!([])),
        ]
        .concat();
        assert_eq!(texts(Copilot::new(&copilot).history(latest)), expected);
    }

    #[test]
    fn copilot_messages_tools_results_and_subagent_steps() {
        let c = Copilot::new(COPILOT_HISTORY);
        let p = c.history(params(None, None, None));
        assert!(p.available);
        assert!(
            p.session
                .unwrap()
                .starts_with(&format!("copilot:{COPILOT}:"))
        );
        let shown = p
            .messages
            .iter()
            .map(|m| {
                (
                    m.role.as_str(),
                    m.tool.as_deref(),
                    m.call_id.as_deref(),
                    m.parent_call_id.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        let (call, child) = (Some("fixture-call-4"), Some("fixture-call-5"));
        assert_eq!(
            shown,
            [
                ("user", None, None, None),
                ("assistant", None, None, None),
                ("assistant", Some("bash"), Some("fixture-call-1"), None),
                ("tool", Some("Result"), Some("fixture-call-1"), None),
                ("assistant", Some("view"), Some("fixture-call-2"), None),
                (
                    "assistant",
                    Some("apply_patch"),
                    Some("fixture-call-3"),
                    None
                ),
                ("tool", Some("Result"), Some("fixture-call-2"), None),
                ("tool", Some("Result"), Some("fixture-call-3"), None),
                ("assistant", Some("task"), call, None),
                ("user", None, None, call),
                ("tool", Some("Result"), call, None),
                ("assistant", None, None, None),
                ("assistant", Some("view"), child, call),
                ("tool", Some("Result"), child, call),
                ("assistant", None, None, call),
                ("event", None, None, None),
                ("assistant", None, None, None),
            ]
        );
        let m = &p.messages;
        assert_eq!(m[0].text, "List the **files** in the project");
        assert!(m[2].text.contains("\"command\":\"ls\""));
        assert_eq!(m[5].text, "*** Begin Patch\n*** End Patch");
        assert_eq!(m[6].text, "Path does not exist");
        assert_eq!(m[9].text, "Summarize the README");
        assert_eq!(m[15].text, "Subagent finished");
        assert_eq!(m[16].id, "fixture-26:0");
        assert!(!m.iter().any(|m| m.text.contains("scrubbed")));
        let search = c.history(params(None, None, Some("single HEADING")));
        assert_eq!(search.messages.len(), 1);
        assert_eq!(search.messages[0].parent_call_id.as_deref(), call);
    }

    #[test]
    fn copilot_child_records_never_appear_as_the_parents_conversation() {
        let lines = COPILOT_HISTORY.split_inclusive('\n').collect::<Vec<_>>();
        let c = Copilot::new(&lines[..13].concat());
        let first = c.history(params(None, None, None));
        append(&c.path(), &lines[13..].concat());
        let next = c.history(params(None, first.after, None));
        assert_eq!(next.messages[0].text, "Summarize the README");
        assert_eq!(
            next.messages[0].parent_call_id.as_deref(),
            Some("fixture-call-4")
        );
        append(&c.path(), &lines[13].replace("30000000", "40000000"));
        let orphan = c.history(params(None, next.after, None));
        assert!(orphan.messages.is_empty());
        append(&c.path(), &lines[17].replace("30000000", "40000000"));
        let sibling = c.history(params(None, orphan.after, None));
        assert_eq!(sibling.messages.len(), 1);
        assert_eq!(
            sibling.messages[0].parent_call_id.as_deref(),
            Some("fixture-call-4")
        );
        let all = c.history(params(None, None, None)).messages;
        assert_eq!(all.iter().filter(|m| m.role == "user").count(), 3);
        assert!(
            all.iter()
                .filter(|m| m.role == "user")
                .skip(1)
                .all(|m| m.parent_call_id.as_deref() == Some("fixture-call-4"))
        );
    }

    #[test]
    fn copilot_pages_search_partial_and_oversized_records_have_no_gaps() {
        let c = Copilot::new(&copilot_header());
        let path = c.path();
        let first = c.history(params(None, None, None));
        assert!(first.available && first.messages.is_empty());
        for n in 0..85 {
            append(&path, &copilot_row(n, &format!("Reply {n}")));
        }
        append(
            &path,
            &copilot_row(85, &"x".repeat(SCAN_BYTES as usize * 3)),
        );
        append(&path, &copilot_row(86, "Reply 86"));
        let mut p = c.history(params(None, None, None));
        let mut backward = p.messages.clone();
        let mut omitted = p.omitted;
        while p.before.is_some() {
            p = c.history(params(p.before, None, None));
            omitted |= p.omitted;
            backward.splice(0..0, p.messages.clone());
        }
        assert!(omitted);
        assert_eq!(backward.len(), 86);
        assert_eq!(backward[85].id, "c86:0");
        let mut cursor = first.after;
        let mut forward = vec![];
        for _ in 0..20 {
            p = c.history(params(None, cursor, None));
            cursor = p.after;
            forward.extend(p.messages);
            if !p.more {
                break;
            }
        }
        assert!(!p.more);
        assert_eq!(backward, forward);
        let mut p = c.history(params(None, None, Some("REPLY 42")));
        let mut found = p.messages.clone();
        while p.before.is_some() {
            p = c.history(params(p.before, None, Some("REPLY 42")));
            found.extend(p.messages.clone());
        }
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "c42:0");
        append(&path, copilot_row(87, "partial").trim_end());
        let partial = c.history(params(None, cursor.clone(), None));
        assert!(partial.messages.is_empty());
        assert_eq!(partial.after, cursor);
        append(&path, "\n");
        let done = c.history(params(None, cursor, None));
        assert_eq!(done.messages.len(), 1);
        assert_eq!(done.messages[0].id, "c87:0");
    }

    #[test]
    fn copilot_cursors_reset_after_a_new_session_or_replacement() {
        let mut c = Copilot::new(&(copilot_header() + &copilot_row(1, "old")));
        let p = c.history(params(None, None, None));
        let other = "50000000-0000-4000-8000-000000000005";
        c.session(
            other,
            "Next conversation",
            &(copilot_header() + &copilot_row(2, "new")),
        );
        c.a.terminal_title_stripped = Some("Next conversation - GitHub Copilot".into());
        let switched = c.history(params(None, p.after.clone(), None));
        assert!(switched.reset);
        assert_ne!(switched.session, p.session);
        assert_eq!(switched.messages[0].text, "new");
        c.a.terminal_title_stripped = Some("Fixture conversation - GitHub Copilot".into());
        std::fs::rename(c.path(), c.path().with_extension("old")).unwrap();
        std::fs::write(c.path(), copilot_header() + &copilot_row(3, "replaced")).unwrap();
        let replaced = c.history(params(None, p.after, None));
        assert!(replaced.reset);
        assert_eq!(replaced.messages[0].text, "replaced");
    }

    #[test]
    fn copilot_missing_or_ambiguous_identity_never_guesses() {
        let mut c = Copilot::new(COPILOT_HISTORY);
        let unconfirmed = |c: &Copilot| {
            let page = c.history(params(None, None, None));
            assert!(!page.available && page.messages.is_empty() && page.session.is_none());
            assert!(
                page.unavailable_reason
                    .unwrap()
                    .contains("Copilot CLI conversation")
            );
        };
        c.a.terminal_title_stripped = Some("GitHub Copilot".into());
        unconfirmed(&c);
        c.a.terminal_title_stripped = Some("Fixture conversation - GitHub Copilot".into());
        assert!(c.history(params(None, None, None)).available);
        c.session(
            "60000000-0000-4000-8000-000000000006",
            "Fixture conversation",
            COPILOT_HISTORY,
        );
        unconfirmed(&c);
        let mut moved = c.a.clone();
        moved.pane_id = "w1:p2".into();
        c.a = moved;
        unconfirmed(&c);
    }

    #[test]
    fn copilot_transcript_must_be_the_selected_session_and_not_a_symlink() {
        let c = Copilot::new(COPILOT_HISTORY);
        assert!(c.history(params(None, None, None)).available);
        let target = c.path().with_extension("secret");
        std::fs::rename(c.path(), &target).unwrap();
        std::os::unix::fs::symlink(&target, c.path()).unwrap();
        let page = c.history(params(None, None, None));
        assert!(!page.available && page.messages.is_empty());
    }

    #[test]
    fn copilot_user_provenance_is_shown_and_injected_sources_are_not() {
        let prompt = |source: Value, agent: Option<&str>| {
            let mut v =
                json!({"type":"user.message","id":"p","data":{"content":"typed","source":source}});
            if let Some(agent) = agent {
                v["agentId"] = agent.into();
            }
            line(v)
        };
        for (source, shown) in [
            (Value::Null, true),
            (json!("user"), true),
            (json!("system"), false),
            (json!("autopilot"), false),
            (json!("skill-fixture"), false),
            (json!("agent-30000000-0000-4000-8000-000000000003"), false),
        ] {
            let c = Copilot::new(&(copilot_header() + &prompt(source.clone(), None)));
            let p = c.history(params(None, None, None));
            assert_eq!(p.messages.len(), usize::from(shown), "{source}");
            let search = c.history(params(None, None, Some("TYPED")));
            assert_eq!(search.messages.len(), usize::from(shown), "{source}");
        }
        let child = "30000000-0000-4000-8000-000000000003";
        let lines = COPILOT_HISTORY.split_inclusive('\n').collect::<Vec<_>>();
        let c = Copilot::new(
            &(lines[..13].concat() + &prompt(json!(format!("agent-{child}")), Some(child))),
        );
        let last = c.history(params(None, None, None)).messages.pop().unwrap();
        assert_eq!(
            (last.text.as_str(), last.parent_call_id.as_deref()),
            ("typed", Some("fixture-call-4"))
        );
    }

    #[test]
    fn copilot_page_read_before_a_new_session_is_not_current() {
        let mut c = Copilot::new(&(copilot_header() + &copilot_row(1, "old")));
        let read = c.a.clone();
        let page = c.history(params(None, None, None));
        assert!(c.t.history_current(&read, &page));
        c.session(
            "50000000-0000-4000-8000-000000000005",
            "Next conversation",
            &(copilot_header() + &copilot_row(2, "new")),
        );
        let mut now = read.clone();
        now.terminal_title_stripped = Some("Next conversation - GitHub Copilot".into());
        assert_eq!(now.agent_session, read.agent_session);
        assert!(!c.t.history_current(&now, &page));
        now.terminal_title_stripped = Some("GitHub Copilot".into());
        assert!(!c.t.history_current(&now, &page));
        let unavailable = c.t.history(&now, &params(None, None, None));
        assert!(!unavailable.available);
        assert!(c.t.history_current(&now, &unavailable));
        assert!(c.t.history_current(&read, &page));
    }
}
