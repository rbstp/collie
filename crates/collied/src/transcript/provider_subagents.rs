use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use protocol::Subagent;
use serde_json::Value;

use super::subagents::{bounded, timestamp_ms};
use super::{MAX_LISTED, civil, is_session_id, open, uuid_v7_ms};

const READ_BYTES: u64 = 512 * 1024;
const LINE_BYTES: usize = 64 * 1024;
const MAX_FILES: usize = 8;
const MAX_ENTRIES: usize = 256;

#[derive(Default)]
pub(super) struct Live {
    copilot: HashMap<String, Scan>,
    codex: HashMap<String, HashMap<String, Scan>>,
}

#[derive(Default)]
struct Scan {
    path: PathBuf,
    epoch: Option<SystemTime>,
    len: u64,
    modified: Option<SystemTime>,
    inode: Option<(u64, u64)>,
    offset: u64,
    ordinal: u64,
    skip_line: bool,
    trusted: bool,
    history_start: u64,
    title: String,
    agent_type: String,
    truncated: bool,
    agents: HashMap<String, Running>,
}

struct Running {
    call: String,
    turn: Option<String>,
    running: bool,
    final_response: bool,
    summary: Subagent,
}

impl Live {
    pub(super) fn retain(&mut self, live: impl Fn(&String) -> bool) {
        self.copilot.retain(|id, _| live(id));
        self.codex.retain(|id, _| live(id));
    }

    pub(super) fn copilot(&mut self, root: &Path, session: &str) -> (Vec<Subagent>, bool) {
        if !is_session_id(session) || !directory(root) {
            return (Vec::new(), false);
        }
        let dir = root.join(session);
        if !directory(&dir) {
            self.copilot.remove(session);
            return (Vec::new(), false);
        }
        let (files, truncated) = entries(&dir);
        let epoch = files
            .iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_prefix("inuse.")?.strip_suffix(".hold"))
                    .is_some_and(|pid| pid.parse::<u32>().is_ok_and(|p| p > 0))
            })
            .filter_map(|p| {
                held(p)
                    .then(|| open(p).map(|(_, _, modified)| modified))
                    .flatten()
            })
            .max();
        let Some(epoch) = epoch else {
            self.copilot.remove(session);
            return (Vec::new(), truncated);
        };
        let scan = self.copilot.entry(session.to_owned()).or_default();
        if scan.epoch != Some(epoch) {
            *scan = Scan {
                epoch: Some(epoch),
                path: dir.join("events.jsonl"),
                ..Scan::default()
            };
        }
        let caught_up = scan.read(|s, v, ordinal| s.copilot(v, ordinal, session, epoch));
        // A later unread completion must not leave an old start displayed as running.
        (
            if caught_up {
                scan.summaries()
            } else {
                Vec::new()
            },
            truncated || !caught_up || scan.truncated,
        )
    }

    pub(super) fn codex(&mut self, root: &Path, session: &str) -> (Vec<Subagent>, bool) {
        let Some(home) = root.parent() else {
            return (Vec::new(), false);
        };
        let locks = home.join("thread-writer-locks");
        if !is_session_id(session) || !directory(root) || !directory(&locks) {
            self.codex.remove(session);
            return (Vec::new(), false);
        }
        let Some((coordination, _, _)) = open(&locks.join(".coordination.lock")) else {
            return (Vec::new(), true);
        };
        // Codex takes this exclusively before trying a writer lock. Probes must not race it.
        if coordination.try_lock_shared().is_err() {
            return (Vec::new(), true);
        }
        if !held(&locks.join(format!("{session}.lock"))) {
            self.codex.remove(session);
            return (Vec::new(), false);
        }
        let (files, mut truncated) = entries(&locks);
        let files: Vec<_> = files
            .into_iter()
            .filter(|lock| {
                lock.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix(".lock"))
                    .is_some_and(|id| is_session_id(id) && id != session)
                    && held(lock)
            })
            .collect();
        drop(coordination);
        let old = self.codex.entry(session.to_owned()).or_default();
        let mut alive = HashSet::new();
        let mut opened = 0;
        let mut summaries = Vec::new();
        for lock in files {
            let Some(id) = lock
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".lock"))
                .filter(|id| is_session_id(id) && *id != session)
            else {
                continue;
            };
            alive.insert(id.to_owned());
            let scan = match old.entry(id.to_owned()) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let (path, limited) = rollout(root, id);
                    truncated |= limited;
                    let Some(path) = path else {
                        truncated = true;
                        continue;
                    };
                    e.insert(Scan {
                        path,
                        ..Scan::default()
                    })
                }
            };
            if !scan
                .path
                .parent()
                .and_then(|p| p.strip_prefix(root).ok())
                .is_some_and(|relative| {
                    let mut path = root.to_owned();
                    relative.components().all(|part| {
                        path.push(part);
                        directory(&path)
                    })
                })
            {
                scan.agents.clear();
                continue;
            }
            if !scan.current() {
                if opened == MAX_FILES {
                    truncated = true;
                    continue;
                }
                opened += 1;
            }
            if scan.read(|s, v, ordinal| s.codex(v, ordinal, id, session)) {
                summaries.extend(
                    scan.agents
                        .iter()
                        .filter(|(_, a)| a.running)
                        .map(|(key, a)| (key.clone(), a.summary.clone())),
                );
                truncated |= scan.truncated;
            } else {
                truncated = true;
            }
        }
        old.retain(|id, _| alive.contains(id));
        ordered(summaries, truncated)
    }
}

impl Scan {
    fn current(&self) -> bool {
        self.offset == self.len
            && std::fs::symlink_metadata(&self.path).is_ok_and(|m| {
                m.is_file()
                    && m.len() == self.len
                    && m.modified().ok() == self.modified
                    && Some((m.dev(), m.ino())) == self.inode
            })
    }

    fn read(&mut self, mut apply: impl FnMut(&mut Self, &Value, u64)) -> bool {
        if self.current() {
            return true;
        }
        let Some((mut file, len, modified)) = open(&self.path) else {
            self.agents.clear();
            return false;
        };
        let inode = file.metadata().ok().map(|m| (m.dev(), m.ino()));
        if inode != self.inode
            || len < self.len
            || (len == self.len && self.modified.is_some_and(|m| m != modified))
        {
            self.offset = 0;
            self.ordinal = 0;
            self.skip_line = false;
            self.trusted = false;
            self.truncated = false;
            self.agents.clear();
        }
        self.len = len;
        self.modified = Some(modified);
        self.inode = inode;
        if self.ordinal > 0 && !self.trusted {
            self.offset = len;
            return true;
        }
        if self.offset == len {
            return true;
        }
        let mut bytes = Vec::new();
        if file.seek(SeekFrom::Start(self.offset)).is_err()
            || file
                .take(READ_BYTES.min(len.saturating_sub(self.offset)))
                .read_to_end(&mut bytes)
                .is_err()
        {
            return false;
        }
        for chunk in bytes.split_inclusive(|b| *b == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            if !complete && !self.skip_line && chunk.len() <= LINE_BYTES {
                break;
            }
            self.offset += chunk.len() as u64;
            if complete {
                if !self.skip_line
                    && chunk.len() <= LINE_BYTES
                    && let Ok(v) = serde_json::from_slice(chunk)
                {
                    apply(self, &v, self.ordinal);
                }
                if self.ordinal == 0 && !self.trusted {
                    self.offset = len;
                    self.ordinal = 1;
                    return true;
                }
                self.ordinal += 1;
                self.skip_line = false;
            } else {
                self.skip_line = true;
            }
        }
        self.offset == len
    }

    fn summaries(&self) -> Vec<Subagent> {
        ordered(
            self.agents
                .iter()
                .filter(|(_, a)| a.running)
                .map(|(id, a)| (id.clone(), a.summary.clone()))
                .collect(),
            false,
        )
        .0
    }

    fn copilot(&mut self, v: &Value, ordinal: u64, session: &str, epoch: SystemTime) {
        let d = &v["data"];
        if ordinal == 0 {
            self.trusted = v["type"] == "session.start" && d["sessionId"] == session;
        }
        if !self.trusted {
            return;
        }
        let id = v["agentId"].as_str();
        match v["type"].as_str() {
            Some("subagent.started") => {
                let Some((id, call, started, agent_type, title)) = (|| {
                    let started = timestamp_ms(v["timestamp"].as_str()?)?;
                    (UNIX_EPOCH + std::time::Duration::from_millis(started) >= epoch)
                        .then_some(())?;
                    Some((
                        id.filter(|s| !s.is_empty() && s.len() <= 128)?,
                        d["toolCallId"].as_str().filter(|s| s.len() <= 256)?,
                        started,
                        bounded(d["agentName"].as_str()?, 64)?,
                        bounded(d["agentDisplayName"].as_str()?, 120)?,
                    ))
                })() else {
                    return;
                };
                if self.agents.len() == MAX_ENTRIES
                    && !self.agents.contains_key(id)
                    && let Some(finished) = self
                        .agents
                        .iter()
                        .filter(|(_, a)| !a.running)
                        .min_by_key(|(id, a)| (a.summary.started_at_ms, *id))
                        .map(|(id, _)| id.clone())
                {
                    self.agents.remove(&finished);
                }
                if self.agents.len() < MAX_ENTRIES || self.agents.contains_key(id) {
                    self.agents.insert(
                        id.to_owned(),
                        Running {
                            call: call.to_owned(),
                            turn: None,
                            running: true,
                            final_response: false,
                            summary: Subagent {
                                agent_type,
                                activity: title.clone(),
                                title,
                                started_at_ms: started,
                                tokens: 0,
                                called_tools: false,
                            },
                        },
                    );
                } else {
                    self.truncated = true;
                }
            }
            Some("subagent.completed" | "subagent.failed") => {
                for a in self
                    .agents
                    .values_mut()
                    .filter(|a| d["toolCallId"] == a.call)
                {
                    a.running = false;
                }
            }
            Some("abort") => match id {
                Some(id) => {
                    if let Some(a) = self.agents.get_mut(id) {
                        a.running = false;
                    }
                }
                None => self.agents.values_mut().for_each(|a| a.running = false),
            },
            Some("session.shutdown") => self.agents.values_mut().for_each(|a| a.running = false),
            Some("user.message") => {
                if let Some(a) = id.and_then(|id| self.agents.get_mut(id))
                    && !a.running
                    && let Some(started) = v["timestamp"].as_str().and_then(timestamp_ms)
                {
                    a.running = true;
                    a.final_response = false;
                    a.summary.started_at_ms = started;
                    a.summary.called_tools = false;
                    a.summary.tokens = 0;
                    a.summary.activity = a.summary.title.clone();
                }
            }
            Some("assistant.message") => {
                if let Some(a) = id.and_then(|id| self.agents.get_mut(id)) {
                    a.final_response = !d["toolRequests"]
                        .as_array()
                        .is_some_and(|tools| !tools.is_empty());
                    a.turn = d["turnId"]
                        .as_str()
                        .filter(|t| t.len() <= 128)
                        .map(str::to_owned);
                }
            }
            Some("assistant.turn_end") => {
                if let Some(a) = id.and_then(|id| self.agents.get_mut(id))
                    && a.final_response
                    && a.turn.as_deref().is_some_and(|t| d["turnId"] == t)
                {
                    a.running = false;
                }
            }
            Some("tool.execution_start") => {
                if let Some(a) = id.and_then(|id| self.agents.get_mut(id)) {
                    a.summary.called_tools = true;
                    if let Some(title) = d["toolTitle"].as_str().and_then(|s| bounded(s, 120)) {
                        a.summary.activity = title;
                    }
                }
            }
            Some("session.usage_record") => {
                if let Some(a) = id.and_then(|id| self.agents.get_mut(id)) {
                    let usage = &d["usage"];
                    a.summary.tokens = usage["inputTokens"]
                        .as_u64()
                        .unwrap_or(0)
                        .saturating_add(usage["outputTokens"].as_u64().unwrap_or(0));
                }
            }
            _ => {}
        }
    }

    fn codex(&mut self, v: &Value, ordinal: u64, id: &str, parent: &str) {
        let p = &v["payload"];
        if ordinal == 0 {
            let source = &p["source"]["subagent"]["thread_spawn"];
            let belongs = v["type"] == "session_meta"
                && p["id"] == id
                && source["parent_thread_id"] == parent;
            self.trusted = belongs
                && (p["subagent_history_start_ordinal"].is_u64()
                    || (p["forked_from_id"].is_null() && p["history_base"].is_null()));
            self.history_start = p["subagent_history_start_ordinal"].as_u64().unwrap_or(1);
            self.title = source["agent_path"]
                .as_str()
                .and_then(|s| s.rsplit('/').find(|s| !s.is_empty()))
                .or_else(|| source["agent_nickname"].as_str())
                .and_then(|s| bounded(s, 120))
                .unwrap_or_default();
            self.agent_type = source["agent_role"]
                .as_str()
                .and_then(|s| bounded(s, 64))
                .unwrap_or_else(|| "codex".into());
            self.trusted &= !self.title.is_empty();
            self.truncated = belongs && !self.trusted;
        }
        if !self.trusted || ordinal < self.history_start {
            return;
        }
        if v["type"] == "session_meta" {
            return;
        }
        if v["type"] == "event_msg" {
            match p["type"].as_str() {
                Some("task_started") => {
                    let Some(started) = v["timestamp"].as_str().and_then(timestamp_ms) else {
                        return;
                    };
                    let Some(turn) = p["turn_id"].as_str().filter(|t| t.len() <= 128) else {
                        return;
                    };
                    self.agents.insert(
                        id.to_owned(),
                        Running {
                            call: String::new(),
                            turn: Some(turn.to_owned()),
                            running: true,
                            final_response: false,
                            summary: Subagent {
                                agent_type: self.agent_type.clone(),
                                title: self.title.clone(),
                                activity: self.title.clone(),
                                started_at_ms: started,
                                tokens: 0,
                                called_tools: false,
                            },
                        },
                    );
                }
                Some("task_complete" | "turn_aborted") => {
                    self.agents
                        .retain(|_, a| a.turn.as_deref() != p["turn_id"].as_str());
                }
                Some("shutdown_complete") => self.agents.clear(),
                Some("token_count") => {
                    if let Some(a) = self.agents.get_mut(id) {
                        a.summary.tokens = p["info"]["total_token_usage"]["total_tokens"]
                            .as_u64()
                            .unwrap_or(a.summary.tokens);
                    }
                }
                _ => {}
            }
        } else if v["type"] == "response_item"
            && matches!(
                p["type"].as_str(),
                Some("function_call" | "custom_tool_call")
            )
            && let Some(a) = self.agents.get_mut(id)
        {
            a.summary.called_tools = true;
            if let Some(name) = p["name"].as_str().and_then(|s| bounded(s, 120)) {
                a.summary.activity = name;
            }
        }
    }
}

fn ordered(mut agents: Vec<(String, Subagent)>, truncated: bool) -> (Vec<Subagent>, bool) {
    agents.sort_by(|(id, a), (other, b)| (a.started_at_ms, id).cmp(&(b.started_at_ms, other)));
    (agents.into_iter().map(|(_, a)| a).collect(), truncated)
}

fn directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

fn entries(path: &Path) -> (Vec<PathBuf>, bool) {
    let Ok(files) = std::fs::read_dir(path) else {
        return (Vec::new(), true);
    };
    let mut paths: Vec<_> = files
        .flatten()
        .take(MAX_ENTRIES + 1)
        .map(|e| e.path())
        .collect();
    let truncated = paths.len() > MAX_ENTRIES;
    paths.truncate(MAX_ENTRIES);
    paths.sort();
    (paths, truncated)
}

fn held(path: &Path) -> bool {
    open(path).is_some_and(|(file, _, _)| {
        matches!(
            file.try_lock_shared(),
            Err(std::fs::TryLockError::WouldBlock)
        )
    })
}

fn rollout(root: &Path, id: &str) -> (Option<PathBuf>, bool) {
    let Some(ms) = uuid_v7_ms(id) else {
        return (None, false);
    };
    let day = (ms / 86_400_000) as i64;
    let suffix = format!("-{id}.jsonl");
    let mut truncated = false;
    for day in day - 1..=day + 1 {
        let (y, m, d) = civil(day);
        let year = root.join(format!("{y:04}"));
        let month = year.join(format!("{m:02}"));
        let dir = month.join(format!("{d:02}"));
        if ![&year, &month, &dir].into_iter().all(|p| directory(p)) {
            continue;
        }
        let mut files = std::fs::read_dir(&dir).into_iter().flatten().flatten();
        for file in files.by_ref().take(MAX_LISTED) {
            if file
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(&suffix))
            {
                return (Some(file.path()), false);
            }
        }
        truncated |= files.next().is_some();
    }
    (None, truncated)
}

#[cfg(test)]
mod tests {
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::symlink;

    use super::*;

    const SESSION: &str = "10000000-0000-4000-8000-000000000001";
    const AGENT: &str = "20000000-0000-4000-8000-000000000002";
    const PARENT: &str = "01a14efe-0000-7000-8000-000000000001";
    const CHILD: &str = "01a14efe-0000-7000-8000-000000000002";
    const COPILOT: &str = include_str!("../../tests/fixtures/transcripts/copilot-subagents.jsonl");
    const COPILOT_INTERRUPTED: &str =
        include_str!("../../tests/fixtures/transcripts/copilot-subagents-interrupted.jsonl");
    const CODEX: &str = include_str!("../../tests/fixtures/transcripts/codex-subagents.jsonl");
    const CODEX_INTERRUPTED: &str =
        include_str!("../../tests/fixtures/transcripts/codex-subagents-interrupted.jsonl");
    const INHERITED: &str =
        include_str!("../../tests/fixtures/transcripts/codex-subagents-inherited.jsonl");

    struct Setup {
        dir: tempfile::TempDir,
        copilot: PathBuf,
        codex: PathBuf,
        events: PathBuf,
        rollout: PathBuf,
        copilot_lock: File,
        parent_lock: File,
        child_lock: File,
    }

    fn locked(path: &Path) -> File {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = File::create(path).unwrap();
        file.set_modified(UNIX_EPOCH).unwrap();
        file.lock().unwrap();
        file
    }

    impl Setup {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let copilot = dir.path().join("copilot/session-state");
            let codex = dir.path().join("codex/sessions");
            let events = copilot.join(SESSION).join("events.jsonl");
            let rollout = codex
                .join("2026/10/18")
                .join(format!("rollout-2026-10-18T12-30-55-{CHILD}.jsonl"));
            let copilot_lock = locked(&copilot.join(SESSION).join("inuse.12345.hold"));
            let locks = dir.path().join("codex/thread-writer-locks");
            let parent_lock = locked(&locks.join(format!("{PARENT}.lock")));
            let child_lock = locked(&locks.join(format!("{CHILD}.lock")));
            File::create(locks.join(".coordination.lock")).unwrap();
            std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();
            Self {
                dir,
                copilot,
                codex,
                events,
                rollout,
                copilot_lock,
                parent_lock,
                child_lock,
            }
        }

        fn copilot(&self, live: &mut Live) -> (Vec<Subagent>, bool) {
            live.copilot(&self.copilot, SESSION)
        }

        fn codex(&self, live: &mut Live) -> (Vec<Subagent>, bool) {
            live.codex(&self.codex, PARENT)
        }
    }

    fn prefix(text: &str, lines: usize) -> String {
        text.lines().take(lines).map(|s| format!("{s}\n")).collect()
    }

    fn append(path: &Path, text: &str) {
        OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    #[test]
    fn copilot_real_tools_turns_and_completion() {
        let s = Setup::new();
        let mut live = Live::default();
        std::fs::write(&s.events, prefix(COPILOT, 7)).unwrap();
        let (agents, truncated) = s.copilot(&mut live);
        assert!(!truncated);
        assert_eq!(agents.len(), 1);
        let a = &agents[0];
        assert_eq!(a.title, "lifecycle-probe");
        assert_eq!(a.agent_type, "general-purpose");
        assert_eq!(a.activity, "Sleep for lifecycle probe");
        assert!(a.called_tools);
        assert_eq!(a.tokens, 6675);
        assert_eq!(
            a.started_at_ms,
            timestamp_ms("2026-10-18T12:30:37.343Z").unwrap()
        );
        assert_eq!(s.copilot(&mut live).0, agents);
        append(
            &s.events,
            &COPILOT
                .lines()
                .skip(7)
                .map(|s| format!("{s}\n"))
                .collect::<String>(),
        );
        assert!(s.copilot(&mut live).0.is_empty());
    }

    #[test]
    fn copilot_real_abort_and_cancelled_completion() {
        let s = Setup::new();
        let mut live = Live::default();
        let records: Vec<Value> = COPILOT_INTERRUPTED
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let abort = records.iter().position(|v| v["type"] == "abort").unwrap();
        std::fs::write(&s.events, prefix(COPILOT_INTERRUPTED, abort)).unwrap();
        assert_eq!(s.copilot(&mut live).0.len(), 1);
        append(&s.events, &format!("{}\n", records[abort]));
        assert!(s.copilot(&mut live).0.is_empty());
        assert!(
            records
                .iter()
                .any(|v| v["type"] == "subagent.completed" && v["data"]["cancelled"] == true)
        );
    }

    #[test]
    fn copilot_followup_reuses_identity_and_finishes_without_another_spawn() {
        let s = Setup::new();
        let mut live = Live::default();
        std::fs::write(&s.events, prefix(COPILOT, 11)).unwrap();
        assert!(s.copilot(&mut live).0.is_empty());
        let records: Vec<Value> = COPILOT
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let mut followup = records[2].clone();
        followup["timestamp"] = records[10]["timestamp"].clone();
        append(&s.events, &format!("{followup}\n"));
        let agents = s.copilot(&mut live).0;
        assert_eq!(agents.len(), 1);
        assert!(!agents[0].called_tools);
        assert_eq!(
            agents[0].started_at_ms,
            timestamp_ms(records[10]["timestamp"].as_str().unwrap()).unwrap()
        );
        append(&s.events, &format!("{}\n{}\n", records[8], records[9]));
        assert!(s.copilot(&mut live).0.is_empty());
    }

    #[test]
    fn copilot_refuses_foreign_stale_and_replaced_sessions() {
        let s = Setup::new();
        let mut live = Live::default();
        std::fs::write(&s.events, prefix(COPILOT, 7)).unwrap();
        assert_eq!(s.copilot(&mut live).0.len(), 1);
        s.copilot_lock.unlock().unwrap();
        assert!(s.copilot(&mut live).0.is_empty());
        s.copilot_lock.lock().unwrap();
        s.copilot_lock.set_modified(SystemTime::now()).unwrap();
        assert!(s.copilot(&mut live).0.is_empty());
        s.copilot_lock.set_modified(UNIX_EPOCH).unwrap();
        assert_eq!(s.copilot(&mut live).0.len(), 1);
        let replacement = s.events.with_extension("replacement");
        std::fs::write(&replacement, COPILOT.replace(SESSION, AGENT)).unwrap();
        std::fs::rename(replacement, &s.events).unwrap();
        assert!(s.copilot(&mut live).0.is_empty());
        assert!(live.copilot(&s.copilot, "../invalid").0.is_empty());
    }

    #[test]
    fn copilot_order_is_start_time_then_identity_not_title() {
        let s = Setup::new();
        let mut live = Live::default();
        std::fs::write(&s.events, prefix(COPILOT, 2)).unwrap();
        let mut start: Value = serde_json::from_str(COPILOT.lines().nth(1).unwrap()).unwrap();
        start["agentId"] = "00000000-0000-4000-8000-000000000000".into();
        start["data"]["toolCallId"] = "other-call".into();
        start["data"]["agentDisplayName"] = "z-first-by-id".into();
        append(&s.events, &format!("{start}\n"));
        let agents = s.copilot(&mut live).0;
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].title, "z-first-by-id");
        assert_eq!(agents[1].title, "lifecycle-probe");
    }

    #[test]
    fn codex_real_running_completion_and_interruption() {
        let s = Setup::new();
        let mut live = Live::default();
        std::fs::write(&s.rollout, prefix(CODEX, 7)).unwrap();
        let (agents, truncated) = s.codex(&mut live);
        assert!(!truncated);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].title, "lifecycle_probe");
        assert_eq!(agents[0].agent_type, "codex");
        assert_eq!(agents[0].activity, "wait");
        assert!(agents[0].called_tools);
        assert_eq!(agents[0].tokens, 91464);
        assert_eq!(
            agents[0].started_at_ms,
            timestamp_ms("2026-10-18T12:30:57.179Z").unwrap()
        );
        append(&s.rollout, &format!("{}\n", CODEX.lines().last().unwrap()));
        assert!(s.codex(&mut live).0.is_empty());
        let interrupted: Vec<&str> = CODEX_INTERRUPTED.lines().collect();
        let aborted = interrupted
            .iter()
            .position(|l| l.contains("\"turn_aborted\""))
            .unwrap();
        for line in &interrupted[1..aborted] {
            append(&s.rollout, &format!("{line}\n"));
        }
        assert_eq!(s.codex(&mut live).0.len(), 1);
        append(&s.rollout, &format!("{}\n", interrupted[aborted]));
        let (agents, truncated) = s.codex(&mut live);
        assert!(agents.is_empty());
        assert!(!truncated);
    }

    #[test]
    fn codex_ignores_inherited_history_and_foreign_turn_completions() {
        let s = Setup::new();
        let mut live = Live::default();
        let header: Value = serde_json::from_str(INHERITED.lines().next().unwrap()).unwrap();
        let boundary = header["payload"]["subagent_history_start_ordinal"]
            .as_u64()
            .unwrap() as usize;
        std::fs::write(&s.rollout, prefix(INHERITED, boundary)).unwrap();
        assert!(s.codex(&mut live).0.is_empty());
        append(
            &s.rollout,
            &format!("{}\n", INHERITED.lines().last().unwrap()),
        );
        assert_eq!(s.codex(&mut live).0.len(), 1);
        append(&s.rollout, &format!("{}\n", CODEX.lines().last().unwrap()));
        assert_eq!(s.codex(&mut live).0.len(), 1);
        let mut unsupported = header;
        unsupported["payload"]
            .as_object_mut()
            .unwrap()
            .remove("subagent_history_start_ordinal");
        std::fs::write(
            &s.rollout,
            format!("{unsupported}\n{}\n", INHERITED.lines().last().unwrap()),
        )
        .unwrap();
        let (agents, truncated) = s.codex(&mut live);
        assert!(agents.is_empty());
        assert!(truncated);
    }

    #[test]
    fn codex_excludes_foreign_parent_child_metadata_and_released_locks() {
        let s = Setup::new();
        let mut live = Live::default();
        std::fs::write(&s.rollout, prefix(CODEX, 7)).unwrap();
        assert_eq!(s.codex(&mut live).0.len(), 1);
        s.child_lock.unlock().unwrap();
        assert!(s.codex(&mut live).0.is_empty());
        s.child_lock.lock().unwrap();
        assert_eq!(s.codex(&mut live).0.len(), 1);
        s.parent_lock.unlock().unwrap();
        assert!(s.codex(&mut live).0.is_empty());
        s.parent_lock.lock().unwrap();
        std::fs::write(
            &s.rollout,
            format!(
                "{}{}\n",
                prefix(CODEX, 7).replace(PARENT, SESSION),
                "x".repeat(READ_BYTES as usize * 2)
            ),
        )
        .unwrap();
        let (agents, truncated) = s.codex(&mut live);
        assert!(agents.is_empty());
        assert!(!truncated);
        std::fs::write(&s.rollout, prefix(CODEX, 7).replace(CHILD, AGENT)).unwrap();
        assert!(s.codex(&mut live).0.is_empty());
    }

    #[test]
    fn codex_defers_probes_while_a_writer_is_starting() {
        let s = Setup::new();
        let mut live = Live::default();
        std::fs::write(&s.rollout, prefix(CODEX, 7)).unwrap();
        let coordination = File::open(
            s.dir
                .path()
                .join("codex/thread-writer-locks/.coordination.lock"),
        )
        .unwrap();
        coordination.lock().unwrap();
        let (agents, truncated) = s.codex(&mut live);
        assert!(agents.is_empty());
        assert!(truncated);
        coordination.unlock().unwrap();
        assert_eq!(s.codex(&mut live).0.len(), 1);
    }

    #[test]
    fn scans_are_bounded_keep_partial_lines_and_skip_oversized_records() {
        let s = Setup::new();
        let mut live = Live::default();
        std::fs::write(&s.events, prefix(COPILOT, 2)).unwrap();
        append(&s.events, &"x".repeat(READ_BYTES as usize + LINE_BYTES));
        append(
            &s.events,
            &format!("\n{}\n", COPILOT.lines().nth(10).unwrap()),
        );
        let (agents, truncated) = s.copilot(&mut live);
        assert!(agents.is_empty());
        assert!(truncated);
        assert!(live.copilot[SESSION].offset <= READ_BYTES);
        let (agents, truncated) = s.copilot(&mut live);
        assert!(agents.is_empty());
        assert!(!truncated);
        std::fs::write(&s.events, prefix(COPILOT, 2).trim_end()).unwrap();
        assert!(s.copilot(&mut live).1);
        append(&s.events, "\n");
        assert_eq!(s.copilot(&mut live).0.len(), 1);
        live.retain(|_| false);
        assert!(live.copilot.is_empty());
    }

    #[test]
    fn codex_event_budget_catches_up_without_starving_later_children() {
        let s = Setup::new();
        let mut live = Live::default();
        let mut locks = Vec::new();
        s.child_lock.unlock().unwrap();
        for n in 10..19 {
            let id = format!("01a14efe-0000-7000-8000-{n:012}");
            locks.push(locked(
                &s.dir
                    .path()
                    .join(format!("codex/thread-writer-locks/{id}.lock")),
            ));
            let path = s
                .rollout
                .parent()
                .unwrap()
                .join(format!("rollout-probe-{id}.jsonl"));
            std::fs::write(path, prefix(CODEX, 7).replace(CHILD, &id)).unwrap();
        }

        let (agents, truncated) = s.codex(&mut live);
        assert_eq!(agents.len(), MAX_FILES);
        assert!(truncated);
        let (agents, truncated) = s.codex(&mut live);
        assert_eq!(agents.len(), 9);
        assert!(!truncated);
    }

    #[test]
    fn codex_busy_day_lookup_and_missing_live_rollout_fail_closed() {
        let s = Setup::new();
        let day = s.rollout.parent().unwrap();
        for n in 0..MAX_ENTRIES + 10 {
            std::fs::write(day.join(format!("rollout-other-{n}.jsonl")), "").unwrap();
        }
        std::fs::write(&s.rollout, prefix(CODEX, 7)).unwrap();
        let mut live = Live::default();
        let (agents, truncated) = s.codex(&mut live);
        assert_eq!(agents.len(), 1);
        assert!(!truncated);
        std::fs::remove_file(&s.rollout).unwrap();
        live.retain(|_| false);
        let (agents, truncated) = s.codex(&mut live);
        assert!(agents.is_empty());
        assert!(truncated);
    }

    #[test]
    fn symlink_files_and_replaced_ancestor_directories_are_refused() {
        let s = Setup::new();
        let mut live = Live::default();
        std::fs::write(&s.rollout, prefix(CODEX, 7)).unwrap();
        symlink(&s.rollout, &s.events).unwrap();
        assert!(s.copilot(&mut live).0.is_empty());
        assert_eq!(s.codex(&mut live).0.len(), 1);
        let day = s.rollout.parent().unwrap();
        let moved = day.with_extension("moved");
        std::fs::rename(day, &moved).unwrap();
        symlink(&moved, day).unwrap();
        assert!(s.codex(&mut live).0.is_empty());
    }

    #[test]
    fn existing_derive_populates_both_provider_arrays() {
        let s = Setup::new();
        std::fs::write(&s.events, prefix(COPILOT, 7)).unwrap();
        std::fs::write(&s.rollout, prefix(CODEX, 7)).unwrap();
        let mut transcripts = super::super::Transcripts::new(None, Some(s.codex.clone()));
        transcripts.copilot_sessions = Some(s.copilot.clone());
        for (kind, id) in [("copilot", SESSION), ("codex", PARENT)] {
            let agent = serde_json::from_value(serde_json::json!({
                "terminal_id": kind,
                "workspace_id": "w1",
                "pane_id": "w1:p1",
                "agent": kind,
                "agent_status": "working",
                "agent_session": {"value": id}
            }))
            .unwrap();
            if kind == "codex" {
                let parent = s
                    .rollout
                    .with_file_name(format!("rollout-probe-{PARENT}.jsonl"));
                std::fs::write(parent, "{\"type\":\"session_meta\",\"payload\":{}}\n").unwrap();
            }
            assert_eq!(transcripts.derive(&agent, true).unwrap().subagents.len(), 1);
            assert_eq!(
                transcripts.derive(&agent, false).unwrap().subagents.len(),
                1
            );
            assert!(!transcripts.quiet(&agent));
        }
    }
}
