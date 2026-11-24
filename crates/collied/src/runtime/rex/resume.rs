use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Rex;
use super::client::Method;
use super::model::{Listing, PREFIX};
use super::screen::MAX_TAIL;
use super::wire::Formatted;
use crate::runtime::screen::Screen;

const GRACE: Duration = Duration::from_secs(15);
const WINDOW_MS: u64 = 90_000;
const MAX_AGENTS: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Agent {
    pub kind: String,
    pub conversation: String,
    pub folder: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Record {
    #[serde(default)]
    pub instance: Option<String>,
    #[serde(default)]
    pub agents: BTreeMap<String, Agent>,
}

#[derive(Default)]
pub(super) struct Resume {
    files: Option<(PathBuf, PathBuf)>,
    off: bool,
    record: Record,
    missing: BTreeMap<String, Instant>,
    pending: BTreeMap<String, Agent>,
    started_ms: u64,
    typed: BTreeSet<String>,
}

fn uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

pub(super) fn command(agent: &Agent) -> Option<String> {
    if !uuid(&agent.conversation) {
        return None;
    }
    let program = match agent.kind.as_str() {
        "claude" => "claude --resume",
        "codex" => "codex resume",
        "agy" => "agy --conversation",
        _ => return None,
    };
    Some(format!("{program} {}", agent.conversation))
}

fn recorded(listing: &Listing) -> BTreeMap<String, Agent> {
    (listing.agents.iter())
        .filter_map(|a| {
            let block = a.terminal_id.strip_prefix(PREFIX)?;
            let agent = Agent {
                kind: a.agent.clone()?,
                conversation: a.agent_session.as_ref()?.value.clone(),
                folder: a.foreground_cwd.clone().filter(|f| f.starts_with('/'))?,
            };
            command(&agent)?;
            Some((block.to_owned(), agent))
        })
        .collect()
}

impl Resume {
    pub(super) fn load(record: PathBuf, off: PathBuf) -> Self {
        let is_off = std::fs::symlink_metadata(&off).is_ok();
        let saved = match is_off {
            true => Record::default(),
            false => crate::peers::load_json(&record).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "could not read the Rex agents to resume");
                Record::default()
            }),
        };
        Self {
            files: Some((record, off)),
            off: is_off,
            record: saved,
            ..Self::default()
        }
    }

    fn save(&self) {
        if let Some((path, _)) = &self.files
            && let Err(e) = crate::peers::save_json(path, &self.record)
        {
            tracing::warn!(error = %e, "could not save the Rex agents to resume");
        }
    }

    pub(super) fn plan(&mut self, instance: Option<&str>, started_ms: Option<u64>, now_ms: u64) {
        self.pending.clear();
        self.missing.clear();
        self.typed.clear();
        let Some(instance) = instance.filter(|_| !self.off) else {
            return;
        };
        if self.record.instance.as_deref() == Some(instance) {
            return;
        }
        let earlier = self.record.instance.replace(instance.to_owned()).is_some();
        let agents = std::mem::take(&mut self.record.agents);
        if let Some(started) =
            started_ms.filter(|s| earlier && now_ms.saturating_sub(*s) < WINDOW_MS)
        {
            self.pending = agents;
            self.started_ms = started;
        }
        self.save();
    }

    pub(super) fn observe(
        &mut self,
        instance: Option<&str>,
        current: &BTreeMap<String, Agent>,
        now: Instant,
    ) {
        if self.off || instance.is_none() || self.record.instance.as_deref() != instance {
            return;
        }
        let mut agents = self.record.agents.clone();
        for (block, agent) in current {
            self.pending.remove(block);
            self.missing.remove(block);
            if agents.len() < MAX_AGENTS || agents.contains_key(block) {
                agents.insert(block.clone(), agent.clone());
            }
        }
        let missing = &mut self.missing;
        agents.retain(|block, _| {
            current.contains_key(block)
                || now.duration_since(*missing.entry(block.clone()).or_insert(now)) < GRACE
        });
        missing.retain(|block, _| agents.contains_key(block));
        if agents != self.record.agents {
            self.record.agents = agents;
            self.save();
        }
    }

    pub(super) fn due(&mut self, now_ms: u64) -> Vec<(String, Agent)> {
        if now_ms.saturating_sub(self.started_ms) >= WINDOW_MS {
            self.pending.clear();
        }
        (self.pending.iter())
            .map(|(b, a)| (b.clone(), a.clone()))
            .collect()
    }

    fn set(&mut self, on: bool, instance: Option<String>) -> std::io::Result<()> {
        let Some((record, off)) = self.files.clone() else {
            return Err(ErrorKind::Unsupported.into());
        };
        if on != self.off {
            return Ok(());
        }
        let gone = |r: std::io::Result<()>| match r {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
        if on {
            gone(std::fs::remove_file(&off))?;
        } else {
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .mode(0o600)
                .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                .open(&off)?;
            gone(std::fs::remove_file(&record))?;
        }
        self.off = !on;
        self.record = Record {
            instance: instance.filter(|_| on),
            agents: BTreeMap::new(),
        };
        self.pending.clear();
        self.missing.clear();
        Ok(())
    }
}

enum Step {
    Wait,
    Drop,
    Typed,
}

impl Rex {
    pub fn with_resume(self, record: PathBuf, off: PathBuf) -> Self {
        *crate::lock(&self.resume) = Resume::load(record, off);
        self
    }

    pub fn resumes(&self) -> bool {
        !crate::lock(&self.resume).off
    }

    pub fn set_resumes(&self, on: bool) -> std::io::Result<()> {
        let instance = self.instance();
        crate::lock(&self.resume).set(on, instance)
    }

    fn instance(&self) -> Option<String> {
        let inner = self.lock();
        inner.live.as_ref().filter(|l| l.synced)?;
        inner.server.as_ref()?.instance.clone()
    }

    pub(super) fn plan_resume(&self) {
        let (instance, started) = {
            let inner = self.lock();
            let server = inner.server.as_ref();
            (
                server.and_then(|s| s.instance.clone()),
                server.and_then(|s| s.started_ms),
            )
        };
        crate::lock(&self.resume).plan(instance.as_deref(), started, crate::now_ms());
    }

    pub async fn resume(&self) {
        let Some(listing) = self.listing() else {
            return;
        };
        let instance = self.instance();
        let due = {
            let mut resume = crate::lock(&self.resume);
            resume.observe(instance.as_deref(), &recorded(&listing), Instant::now());
            resume.due(crate::now_ms())
        };
        for (block, agent) in due {
            if let Step::Drop = self.resume_one(&block, &agent, &listing).await {
                crate::lock(&self.resume).pending.remove(&block);
            }
        }
    }

    // ADR-0013
    async fn resume_one(&self, block: &str, agent: &Agent, listing: &Listing) -> Step {
        let Some(line) = command(agent) else {
            return Step::Drop;
        };
        let id = format!("{PREFIX}{block}");
        let running = (listing.agents.iter()).any(|a| {
            a.terminal_id == id
                || (a.agent_session.as_ref()).is_some_and(|s| s.value == agent.conversation)
        });
        if running
            || crate::lock(&self.resume)
                .typed
                .contains(&agent.conversation)
        {
            return Step::Drop;
        }
        let shell = self.live(|l| {
            (l.model.idle_shell(block))
                .map(|(session, cwd, size)| (l.client.clone(), session, cwd, size))
        });
        let Ok(Some((client, session, cwd, size))) = shell else {
            return Step::Wait;
        };
        if cwd != agent.folder {
            return Step::Drop;
        }
        let args = json!({ "format": "vt", "extra_cursor": true, "max_bytes": MAX_TAIL });
        let Ok((_, f)) =
            (client.terminal::<Formatted>(Method::Format, &session, block, args)).await
        else {
            return Step::Wait;
        };
        if f.id.is_some() {
            return Step::Drop;
        }
        let mut screen = Screen::new(size.0, size.1, 0);
        screen.feed(f.content.as_bytes());
        if !screen.at_empty_prompt() {
            return Step::Wait;
        }
        {
            let mut resume = crate::lock(&self.resume);
            if resume.pending.remove(block).is_none() {
                return Step::Typed;
            }
            resume.typed.insert(agent.conversation.clone());
        }
        match self.type_line(&id, &line).await {
            Ok(()) => {
                tracing::info!(block, kind = %agent.kind, "resumed an agent in its restored Rex block")
            }
            Err(e) => {
                tracing::warn!(block, error = %e, "could not resume an agent in its restored Rex block")
            }
        }
        Step::Typed
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::fake::{self, Fake};
    use super::super::tests::{follow, id};
    use super::*;

    const S: &str = "session:019a7d2e-0000-7000-8000-0000000000a1";
    const READY: &str = "block:019a7d2e-0000-7000-8000-0000000000e1";
    const TWICE: &str = "block:019a7d2e-0000-7000-8000-0000000000e2";
    const TYPED: &str = "block:019a7d2e-0000-7000-8000-0000000000e3";
    const ELSEWHERE: &str = "block:019a7d2e-0000-7000-8000-0000000000e4";
    const FIRST: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    const SECOND: &str = "0a0a0a0a-0000-4000-8000-000000000002";
    const THIRD: &str = "0a0a0a0a-0000-4000-8000-000000000003";

    fn agent(kind: &str, conversation: &str, folder: &str) -> Agent {
        Agent {
            kind: kind.to_owned(),
            conversation: conversation.to_owned(),
            folder: folder.to_owned(),
        }
    }

    fn rfc3339(ms: u64) -> String {
        let (days, rest) = ((ms / 86_400_000) as i64, ms % 86_400_000);
        let z = days + 719_468;
        let era = z / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        let (h, mi, s) = (rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60);
        format!(
            "{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{:03}000Z",
            rest % 1000
        )
    }

    #[test]
    fn only_a_known_agent_with_a_uuid_conversation_has_a_resume_command() {
        let line = |kind, conversation| command(&agent(kind, conversation, "/work"));
        assert_eq!(
            line("claude", FIRST).as_deref(),
            Some("claude --resume 3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d")
        );
        assert_eq!(
            line("codex", FIRST).as_deref(),
            Some("codex resume 3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d")
        );
        assert_eq!(
            line("agy", FIRST).as_deref(),
            Some("agy --conversation 3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d")
        );
        assert_eq!(line("copilot", FIRST), None);
        for bad in [
            "------------------------------------",
            "3F1B2C4D-5E6F-4A7B-8C9D-0E1F2A3B4C5D",
            "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5;",
            "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d ",
            "",
        ] {
            assert_eq!(line("claude", bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_a_new_server_instance_that_started_just_now_plans_what_the_last_one_ran() {
        let now = 10_000_000;
        let mut r = Resume::default();
        let ran = BTreeMap::from([(READY.to_owned(), agent("claude", FIRST, "/work"))]);
        r.plan(Some("one"), Some(now), now);
        assert!(r.due(now).is_empty(), "the first instance collied sees");
        r.record.agents = ran.clone();
        r.plan(Some("one"), Some(now), now);
        assert!(r.due(now).is_empty(), "a reconnect to the same instance");
        assert_eq!(r.record.agents, ran);
        r.plan(Some("two"), Some(now - WINDOW_MS), now);
        assert!(
            r.due(now).is_empty(),
            "an instance that started too long ago"
        );
        assert!(r.record.agents.is_empty());
        r.record.agents = ran.clone();
        r.plan(Some("three"), Some(now - 1000), now);
        assert_eq!(r.due(now), ran.clone().into_iter().collect::<Vec<_>>());
        assert_eq!(r.record.instance.as_deref(), Some("three"));
        assert!(
            r.record.agents.is_empty(),
            "taken off the record before any is typed"
        );
        assert!(r.due(now - 1000 + WINDOW_MS).is_empty(), "the window ended");
        r.record.agents = ran;
        r.off = true;
        r.plan(Some("four"), Some(now), now);
        assert!(r.due(now).is_empty(), "turned off");
    }

    #[test]
    fn an_agent_that_left_its_block_is_kept_for_the_grace_then_dropped() {
        let t = Instant::now();
        let mut r = Resume::default();
        r.plan(Some("one"), None, 0);
        let running = BTreeMap::from([(READY.to_owned(), agent("claude", FIRST, "/work"))]);
        let none = BTreeMap::new();
        r.observe(Some("two"), &running, t);
        assert!(
            r.record.agents.is_empty(),
            "another instance than the planned one"
        );
        r.observe(Some("one"), &running, t);
        assert_eq!(r.record.agents, running);
        r.observe(Some("one"), &none, t);
        r.observe(Some("one"), &none, t + GRACE - Duration::from_millis(1));
        assert_eq!(
            r.record.agents, running,
            "a shutdown ends agents before the server"
        );
        r.observe(Some("one"), &none, t + GRACE);
        assert!(r.record.agents.is_empty());
    }

    #[tokio::test]
    async fn a_restarted_server_gets_each_resume_command_once_in_a_shell_at_an_empty_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("rex-resume.json");
        let off = dir.path().join("rex-resume-off");
        let saved = Record {
            instance: Some("one".to_owned()),
            agents: BTreeMap::from([
                (READY.to_owned(), agent("claude", FIRST, "/work/app")),
                (TWICE.to_owned(), agent("codex", FIRST, "/work/app")),
                (TYPED.to_owned(), agent("claude", SECOND, "/work/app")),
                (ELSEWHERE.to_owned(), agent("claude", THIRD, "/work/other")),
            ]),
        };
        crate::peers::save_json(&record, &saved).unwrap();
        let now = crate::now_ms();
        assert_eq!(crate::transcript::timestamp_ms(&rfc3339(now)), Some(now));
        let fake = Fake::start().await;
        {
            let mut s = fake.state();
            s.instance = "two";
            s.started_at = rfc3339(now);
            s.sessions.push((S.to_owned(), "work".to_owned()));
            for (block, screen) in [
                (READY, "user@host app % \x1b[1;17H"),
                (TWICE, "user@host app % \x1b[1;17H"),
                (TYPED, "user@host app % ls"),
                (ELSEWHERE, "user@host app % \x1b[1;17H"),
            ] {
                s.add(S, block, fake::process(10, "-zsh", "zsh"), Vec::new());
                s.block(block).screen = screen.to_owned();
            }
        }
        let rex = Arc::new(Rex::new(fake.socket.clone()).with_resume(record.clone(), off));
        let _task = follow(&rex).await;
        assert!(rex.get(&id(READY)).is_err(), "a shell is no agent");
        rex.resume().await;
        rex.resume().await;
        let typed = |block: &str| -> Vec<String> {
            (fake.state().block(block).events.iter())
                .map(|e| match e["type"].as_str() {
                    Some("paste") => e["payload"]["text"].as_str().unwrap().to_owned(),
                    _ => format!("{}+{}", e["payload"]["mods"], e["payload"]["key"]),
                })
                .collect()
        };
        assert_eq!(
            typed(READY),
            [
                "\"ctrl\"+\"key_e\"",
                "\"ctrl\"+\"key_u\"",
                "claude --resume 3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d",
                "\"\"+\"enter\""
            ]
        );
        assert!(typed(TWICE).is_empty(), "one conversation in two blocks");
        assert!(typed(TYPED).is_empty(), "text at the prompt");
        assert!(typed(ELSEWHERE).is_empty(), "a shell in another folder");
        let on_disk: Record = crate::peers::load_json(&record).unwrap();
        assert_eq!(on_disk.instance.as_deref(), Some("two"));
        assert!(on_disk.agents.is_empty());
        fake.state().block(TYPED).screen = "user@host app % \x1b[1;17H".to_owned();
        rex.resume().await;
        assert_eq!(
            typed(TYPED).get(2).map(String::as_str),
            Some("claude --resume 0a0a0a0a-0000-4000-8000-000000000002")
        );
        assert_eq!(typed(READY).len(), 4);
        assert!(typed(ELSEWHERE).is_empty());
    }
}
