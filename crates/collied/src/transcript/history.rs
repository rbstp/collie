use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;

use protocol::{HistoryMessage, HistoryPage, HistoryParams};
use serde_json::Value;

use super::{Kind, Transcripts, open, session_id};
use crate::herdr::AgentInfo;

const SCAN_BYTES: u64 = 2 * 1024 * 1024;
const PAGE_BYTES: usize = 40 * 1024;
const RECORD_TEXT_BYTES: usize = 6000;

impl Transcripts {
    pub fn history_reader(&self) -> Self {
        Self::new(self.claude_projects.clone(), self.codex_sessions.clone())
    }

    pub fn history(&self, agent: &AgentInfo, params: &HistoryParams) -> HistoryPage {
        self.read_history(agent, params).unwrap_or(HistoryPage {
            session: None,
            available: false,
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
        let id = session_id(agent)?;
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
        for (a, b) in positions {
            let record = serde_json::from_slice::<Value>(&bytes[a..b]);
            let mut batch = match record {
                Ok(v) => normalize(&v, kind, id, modern, start + a as u64, &query),
                Err(_) => {
                    omitted = true;
                    Vec::new()
                }
            };
            let batch_size = serde_json::to_vec(&batch).ok()?.len();
            if size + batch_size > PAGE_BYTES || messages.len() + batch.len() > 40 {
                break;
            }
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
    offset: u64,
    query: &str,
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
    .unwrap_or_else(|| offset.to_string());
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
        });
    };
    match kind {
        Kind::Claude => {
            if v["isSidechain"] == true
                || v["sessionId"].as_str().is_some_and(|s| s != session)
                || v["message"]["model"] == "<synthetic>"
            {
                return out;
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
}
