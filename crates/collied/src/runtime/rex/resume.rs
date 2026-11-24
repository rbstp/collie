use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::client::{Client, Method, TIMEOUT};
use super::model::{self, Listing, PREFIX};
use super::screen::MAX_TAIL;
use super::tasks::{FLAVOR, disown};
use super::wire::{Formatted, Process, Split};
use super::{CLOSE_TIMEOUT, Rex};
use crate::runtime::Error;
use crate::runtime::agents::{self, uuid};
use crate::runtime::screen::Screen;

const GRACE: Duration = Duration::from_secs(15);
const CONFIRM: Duration = Duration::from_millis(if cfg!(test) { 200 } else { 3000 });
const SETTLE: Duration = Duration::from_millis(if cfg!(test) { 100 } else { 2000 });
const START_LIMIT: Duration = Duration::from_secs(if cfg!(test) { 2 } else { 30 });
const WINDOW: Duration = Duration::from_secs(90);
const MAX_AGENTS: usize = 256;
const MAX_FILE: u64 = 256 << 10;
const MAX_FOLDER: usize = 4096;
const SHELLS: [&str; 2] = ["zsh", "bash"];

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
    until: Option<Instant>,
    typed: BTreeSet<String>,
    starting: BTreeMap<String, Starting>,
    waits: BTreeMap<String, &'static str>,
    screens: BTreeMap<String, Baseline>,
    resumed: BTreeMap<String, Agent>,
}

struct Baseline {
    text: String,
    first: Instant,
    changed: Instant,
}

enum Settled {
    Yes,
    Not(&'static str),
    Changed,
}

impl Baseline {
    fn check(
        seen: &mut BTreeMap<String, Baseline>,
        block: &str,
        text: String,
        now: Instant,
    ) -> Settled {
        let Some(b) = seen.get_mut(block) else {
            seen.insert(
                block.to_owned(),
                Baseline {
                    text,
                    first: now,
                    changed: now,
                },
            );
            return Settled::Not("collied read its screen for the first time");
        };
        if b.text != text {
            if now.duration_since(b.first) >= SETTLE {
                return Settled::Changed;
            }
            (b.text, b.changed) = (text, now);
        }
        match now.duration_since(b.changed) >= SETTLE {
            true => Settled::Yes,
            false => Settled::Not("its screen is still settling"),
        }
    }
}

#[derive(Clone)]
struct Starting {
    session: String,
    new: String,
    kind: String,
    conversation: String,
    shell: (u32, (u16, u16)),
    screen: String,
    folder: String,
    since: Instant,
    seen: bool,
    running: Option<Instant>,
}

pub(super) fn args(agent: &Agent) -> Option<Vec<String>> {
    let id = agent.conversation.as_str();
    if !uuid(id) {
        return None;
    }
    let args = match agent.kind.as_str() {
        "claude" => vec!["--resume".to_owned(), id.to_owned()],
        "codex" => vec!["resume".to_owned(), id.to_owned(), "--no-daemon".to_owned()],
        "copilot" => vec![format!("--resume={id}")],
        "agy" => vec!["--conversation".to_owned(), id.to_owned()],
        _ => return None,
    };
    Some(args)
}

// ADR-0013
fn command(agent: &Agent) -> Option<String> {
    let mut words = vec![agent.kind.clone()];
    words.extend(args(agent)?);
    let plain = |w: &String| {
        !w.is_empty()
            && (w.bytes())
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-=".contains(&b))
    };
    words.iter().all(plain).then(|| words.join(" "))
}

pub(super) fn login_shell() -> Option<String> {
    let shell = std::env::var("SHELL").ok()?;
    let name = Path::new(&shell).file_name()?.to_str()?;
    (shell.starts_with('/') && !shell.contains('=') && SHELLS.contains(&name)).then_some(shell)
}

fn fits(block: &str, agent: &Agent) -> bool {
    model::valid(block, "block")
        && agent.folder.starts_with('/')
        && agent.folder.len() <= MAX_FOLDER
        && args(agent).is_some()
}

fn bounded(mut record: Record) -> Record {
    record.instance = record.instance.filter(|i| i.len() <= 128);
    record.agents.retain(|block, agent| fits(block, agent));
    while record.agents.len() > MAX_AGENTS {
        record.agents.pop_last();
    }
    record
}

fn read(path: &Path) -> Record {
    let bytes = match crate::peers::read_private_at_most(path, MAX_FILE + 1) {
        Ok((bytes, _)) => bytes,
        Err(crate::peers::Error::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
            return Record::default();
        }
        Err(crate::peers::Error::Io { source, .. }) => {
            let kind = source.kind();
            tracing::warn!(?kind, "could not read the Rex agents to resume");
            return Record::default();
        }
        Err(_) => {
            tracing::warn!("the Rex agents to resume are not a private file; not read");
            return Record::default();
        }
    };
    if bytes.len() as u64 > MAX_FILE {
        tracing::warn!("the Rex agents to resume are too large to read");
        return Record::default();
    }
    match serde_json::from_slice::<Record>(&bytes) {
        Ok(record) => bounded(record),
        Err(e) => {
            let category = e.classify();
            tracing::warn!(?category, "the Rex agents to resume are malformed");
            Record::default()
        }
    }
}

fn fit(record: &mut Record) {
    let size = |r: &Record| serde_json::to_vec_pretty(r).map_or(u64::MAX, |v| v.len() as u64 + 1);
    let mut total = size(record);
    while total > MAX_FILE {
        let Some(entry) = record.agents.pop_last() else {
            return;
        };
        total = total.saturating_sub(serde_json::to_vec(&entry).map_or(0, |v| v.len() as u64));
        if total <= MAX_FILE {
            total = size(record);
        }
    }
}

fn running(listing: &Listing, conversation: &str) -> bool {
    (listing.agents.iter())
        .any(|a| (a.agent_session.as_ref()).is_some_and(|s| s.value == conversation))
}

impl Resume {
    pub(super) fn load(record: PathBuf, off: PathBuf) -> Self {
        let is_off = std::fs::symlink_metadata(&off).is_ok();
        let saved = match is_off {
            true => Record::default(),
            false => read(&record),
        };
        Self {
            files: Some((record, off)),
            off: is_off,
            record: saved,
            ..Self::default()
        }
    }

    fn save(&mut self) -> bool {
        fit(&mut self.record);
        let Some((path, _)) = &self.files else {
            return true;
        };
        let saved = crate::peers::save_json(path, &self.record);
        if let Err(e) = &saved {
            tracing::warn!(error = %e, "could not save the Rex agents to resume");
        }
        saved.is_ok()
    }

    pub(super) fn plan(
        &mut self,
        instance: Option<&str>,
        started_ms: Option<u64>,
        now_ms: u64,
        now: Instant,
    ) {
        let Some(instance) = instance.filter(|_| !self.off) else {
            return;
        };
        if self.record.instance.as_deref() == Some(instance) {
            return;
        }
        self.pending.clear();
        self.missing.clear();
        self.typed.clear();
        self.starting.clear();
        self.waits.clear();
        self.screens.clear();
        self.resumed.clear();
        self.until = None;
        let earlier = self.record.instance.replace(instance.to_owned()).is_some();
        let agents = std::mem::take(&mut self.record.agents);
        let saved = self.save();
        let age = started_ms
            .and_then(|s| now_ms.checked_sub(s))
            .map(Duration::from_millis);
        let skip = match age {
            _ if agents.is_empty() => return,
            _ if !earlier => "no earlier server was recorded",
            _ if !saved => "the record could not be saved",
            None => "the server's start time is unknown or later than now",
            Some(age) if age >= WINDOW => "the server started more than 90 s ago",
            Some(age) => {
                for (block, agent) in &agents {
                    tracing::warn!(block, kind = %agent.kind, "Rex resume: planned");
                }
                self.pending = agents;
                self.until = Some(now + (WINDOW - age));
                return;
            }
        };
        tracing::warn!(agents = agents.len(), "Rex resume: nothing planned, {skip}");
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
        let mut candidate = Record {
            instance: self.record.instance.clone(),
            agents,
        };
        fit(&mut candidate);
        if candidate.agents != self.record.agents {
            self.record.agents = candidate.agents;
            let _ = self.save();
        }
    }

    fn open(&self, now: Instant) -> bool {
        !self.off && self.until.is_some_and(|until| now < until)
    }

    pub(super) fn due(&mut self, now: Instant) -> Vec<(String, Agent)> {
        if !self.open(now) {
            for (block, agent) in std::mem::take(&mut self.pending) {
                let why = self.waits.get(&block).copied().unwrap_or("not checked");
                tracing::warn!(block, kind = %agent.kind, "Rex resume: window ended before its shell was ready ({why})");
            }
            self.waits.clear();
        }
        (self.pending.iter())
            .map(|(b, a)| (b.clone(), a.clone()))
            .collect()
    }

    pub(super) fn claim(&mut self, block: &str, conversation: &str, now: Instant) -> bool {
        let planned = self.pending.remove(block).is_some();
        planned && self.open(now) && self.typed.insert(conversation.to_owned())
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
        self.resumed.clear();
        self.until = None;
        Ok(())
    }
}

enum Step {
    Wait(&'static str),
    Drop(&'static str),
    Started,
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

    pub fn with_agent_roots(mut self, roots: crate::costs::Roots) -> Self {
        self.agent_roots = roots;
        self
    }

    fn claude_dir(&self) -> Option<&Path> {
        self.claude_sessions.as_deref()?.parent()
    }

    fn transcript(&self, agent: &Agent) -> bool {
        let (id, folder) = (agent.conversation.as_str(), agent.folder.as_str());
        let roots = &self.agent_roots;
        match agent.kind.as_str() {
            "claude" => (self.claude_dir())
                .is_some_and(|c| crate::transcript::claude_transcript(c, folder, id)),
            "codex" => (roots.codex.as_deref())
                .is_some_and(|s| crate::transcript::codex_transcript(s, folder, id)),
            "copilot" => (roots.copilot.as_deref())
                .is_some_and(|r| crate::transcript::copilot_transcript(r, id)),
            "agy" => {
                (roots.agy.as_deref()).is_some_and(|b| crate::transcript::agy_transcript(b, id))
            }
            _ => false,
        }
    }

    fn held(&self, agent: &Agent) -> bool {
        let (id, roots) = (agent.conversation.as_str(), &self.agent_roots);
        match agent.kind.as_str() {
            "claude" => (self.claude_sessions.as_deref())
                .is_some_and(|s| crate::transcript::claude_holds(s, id)),
            "codex" => (roots.codex.as_deref())
                .and_then(|s| crate::transcript::codex_writer_lock(s, id))
                .is_some_and(|l| crate::transcript::held(&l) != Some(false)),
            "copilot" => (roots.copilot.as_deref())
                .is_some_and(|r| crate::transcript::copilot_held(r, id) != Some(false)),
            "agy" => {
                let locked = (self.presence_dir()).is_some_and(|d| {
                    crate::transcript::held(&d.join(format!("{id}.lock"))) != Some(false)
                });
                let open = || {
                    let listing = self.listing();
                    (listing.iter().flat_map(|l| &l.agents))
                        .filter(|a| a.agent.as_deref() == Some("agy"))
                        .filter_map(|a| a.terminal_id.strip_prefix(PREFIX))
                        .filter_map(|b| self.live(|l| l.model.foreground(b)).ok().flatten())
                        .any(|pid| self.holds("agy", pid, id))
                };
                locked || open()
            }
            _ => false,
        }
    }

    pub(super) fn presence_dir(&self) -> Option<PathBuf> {
        Some(self.agent_roots.agy.as_deref()?.parent()?.join("presence"))
    }

    // ADR-0013
    fn holds(&self, kind: &str, pid: u32, conversation: &str) -> bool {
        let roots = &self.agent_roots;
        match kind {
            "codex" => roots.codex.as_deref().is_some_and(|s| {
                let open = agents::open_files(pid);
                let has = |p: &Path| {
                    std::fs::metadata(p).is_ok_and(|m| open.contains(&(m.dev(), m.ino())))
                };
                (crate::transcript::codex_writer_lock(s, conversation))
                    .is_some_and(|l| has(&l) && crate::transcript::held(&l) == Some(true))
                    && crate::transcript::codex_rollout(s, conversation).is_some_and(|r| has(&r))
            }),
            "copilot" => (roots.copilot.as_deref())
                .is_some_and(|r| crate::transcript::copilot_holds(r, pid, conversation)),
            "agy" => (self.presence_dir())
                .is_some_and(|d| agents::agy_lock(&d, pid).as_deref() == Some(conversation)),
            _ => false,
        }
    }

    fn recorded(&self, listing: &Listing) -> BTreeMap<String, Agent> {
        let resumed = crate::lock(&self.resume).resumed.clone();
        (listing.agents.iter())
            .filter_map(|a| {
                let block = a.terminal_id.strip_prefix(PREFIX)?;
                let (kind, folder) = (a.agent.clone()?, a.foreground_cwd.clone()?);
                let pid = || self.live(|l| l.model.foreground(block)).ok().flatten();
                let bound =
                    match kind.as_str() {
                        "copilot" => (self.agent_roots.copilot.as_deref()).zip(pid()).and_then(
                            |(root, pid)| crate::transcript::copilot_session(root, pid, &folder),
                        ),
                        _ => a.agent_session.as_ref().map(|s| s.value.clone()),
                    };
                let proven = || {
                    let r = resumed.get(block)?;
                    let holds = r.kind == kind
                        && r.folder == folder
                        && pid().is_some_and(|p| self.holds(&kind, p, &r.conversation));
                    holds.then(|| r.conversation.clone())
                };
                let conversation = bound.or_else(proven)?;
                let agent = Agent {
                    kind,
                    conversation,
                    folder,
                };
                (fits(block, &agent) && self.transcript(&agent)).then(|| (block.to_owned(), agent))
            })
            .collect()
    }

    fn proven(&self, listing: &Listing, start: &Starting) -> bool {
        let id = format!("{PREFIX}{}", start.new);
        let Some(a) = listing.agents.iter().find(|a| a.terminal_id == id) else {
            return false;
        };
        let there = a.agent.as_deref() == Some(start.kind.as_str())
            && a.foreground_cwd.as_deref() == Some(start.folder.as_str());
        let bound = (a.agent_session.as_ref()).is_some_and(|s| s.value == start.conversation);
        let proof = || {
            (self.live(|l| l.model.foreground(&start.new)).ok().flatten())
                .is_some_and(|pid| self.holds(&start.kind, pid, &start.conversation))
        };
        there
            && match start.kind.as_str() {
                "codex" | "copilot" => proof(),
                "agy" => bound || proof(),
                _ => bound,
            }
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
        let now = (crate::now_ms(), Instant::now());
        crate::lock(&self.resume).plan(instance.as_deref(), started, now.0, now.1);
    }

    pub async fn resume(&self) {
        self.resume_with(login_shell).await;
    }

    pub(super) async fn resume_with(&self, shell: impl Fn() -> Option<String>) {
        let Some(listing) = self.listing() else {
            return;
        };
        let instance = self.instance();
        let current = self.recorded(&listing);
        let (due, starting) = {
            let mut resume = crate::lock(&self.resume);
            resume.resumed.retain(|b, a| current.get(b) == Some(a));
            resume.observe(instance.as_deref(), &current, Instant::now());
            (resume.due(Instant::now()), resume.starting.clone())
        };
        for (block, start) in starting {
            self.advance(&block, start, &listing).await;
        }
        for (block, agent) in due {
            match self.resume_one(&block, &agent, &shell).await {
                Step::Wait(why) => {
                    crate::lock(&self.resume).waits.insert(block, why);
                }
                Step::Drop(why) => {
                    tracing::warn!(block, kind = %agent.kind, "Rex resume: not started, {why}");
                    crate::lock(&self.resume).pending.remove(&block);
                }
                Step::Started => {
                    tracing::warn!(block, kind = %agent.kind, "Rex resume: started beside its restored shell");
                }
            }
        }
    }

    // ADR-0013
    async fn resume_one(
        &self,
        block: &str,
        agent: &Agent,
        shell: &impl Fn() -> Option<String>,
    ) -> Step {
        let Some(command) = command(agent) else {
            return Step::Drop("no command for its kind");
        };
        let Some(login) = shell() else {
            return Step::Drop("the account's login shell is not zsh or bash");
        };
        if !Path::new(&agent.folder).is_dir() {
            return Step::Drop("its folder is gone");
        }
        if !self.transcript(agent) {
            return Step::Drop("its conversation has no transcript");
        }
        let shell = self.live(|l| {
            (l.model.shell_block(block)).map(|(session, size)| (l.client.clone(), session, size))
        });
        let Ok(Some((client, session, size))) = shell else {
            return Step::Wait("no shell in front");
        };
        let Some(pid) = idle(&client, &session, block).await else {
            return Step::Wait("the shell runs something");
        };
        let Some(screen) = screen(&client, &session, block, size).await else {
            return Step::Wait("its screen could not be read");
        };
        let settled = {
            let mut resume = crate::lock(&self.resume);
            Baseline::check(&mut resume.screens, block, screen.clone(), Instant::now())
        };
        match settled {
            Settled::Yes => {}
            Settled::Not(why) => return Step::Wait(why),
            Settled::Changed => {
                return Step::Drop("its screen changed after collied first read it");
            }
        }
        if self.held(agent) {
            return Step::Wait("a live agent holds the conversation");
        }
        let Some(listing) = self.listing() else {
            return Step::Wait("not following Rex");
        };
        if running(&listing, &agent.conversation) {
            return Step::Drop("its conversation runs elsewhere");
        }
        if !crate::lock(&self.resume).claim(block, &agent.conversation, Instant::now()) {
            return Step::Drop("claimed already, or resuming stopped");
        }
        let script = format!("builtin cd -- \"$1\" && {command}");
        let mut flags = ["-l", "-i", "-c"].map(str::to_owned).to_vec();
        flags.extend([script, "collied".to_owned(), agent.folder.clone()]);
        let argv = agents::task_argv(&login, &flags, None);
        match start(&client, &session, block, &agent.folder, argv).await {
            Ok(new) => {
                let starting = Starting {
                    session,
                    new,
                    kind: agent.kind.clone(),
                    conversation: agent.conversation.clone(),
                    shell: (pid, size),
                    screen,
                    folder: agent.folder.clone(),
                    since: Instant::now(),
                    seen: false,
                    running: None,
                };
                crate::lock(&self.resume)
                    .starting
                    .insert(block.to_owned(), starting);
                Step::Started
            }
            Err(e) => {
                tracing::warn!(block, error = %e, "Rex resume: could not start the agent");
                Step::Drop("Rex refused the new block")
            }
        }
    }

    fn exit(&self, start: &Starting) -> Option<bool> {
        self.live(|l| {
            let (holds, exited) = (l.model.holds(&start.new), l.model.exited(&start.new));
            l.synced.then_some((holds, exited))
        })
        .ok()
        .flatten()
        .map(|(holds, exited)| exited || (start.seen && !holds))
    }

    async fn advance(&self, block: &str, mut start: Starting, listing: &Listing) {
        start.seen |= (self.live(|l| l.synced && l.model.has(&start.new))).unwrap_or(false);
        let kind = start.kind.clone();
        let outcome = if self.exit(&start) == Some(true) {
            "the agent exited before it ran its conversation; its block and the restored shell stay"
        } else if self.proven(listing, &start) {
            let since = *start.running.get_or_insert_with(Instant::now);
            if since.elapsed() < CONFIRM {
                crate::lock(&self.resume)
                    .starting
                    .insert(block.to_owned(), start);
                return;
            }
            self.finish(block, &start).await
        } else if start.since.elapsed() >= START_LIMIT {
            "the agent did not show its conversation in its folder in time; its block and the restored shell stay"
        } else {
            start.running = None;
            crate::lock(&self.resume)
                .starting
                .insert(block.to_owned(), start);
            return;
        };
        crate::lock(&self.resume).starting.remove(block);
        tracing::warn!(block, kind = %kind, "Rex resume: {outcome}");
    }

    // ADR-0013
    async fn finish(&self, block: &str, start: &Starting) -> &'static str {
        let Ok(client) = self.live(|l| l.client.clone()) else {
            return "the connection ended before its restored shell closed; both stay";
        };
        let (pid, size) = start.shell;
        let idle_now = idle(&client, &start.session, block).await;
        let screen_now = screen(&client, &start.session, block, size).await;
        let shell = idle_now == Some(pid) && screen_now.as_ref() == Some(&start.screen);
        let exited = self.exit(start) != Some(false);
        let agent = !exited && self.listing().is_some_and(|l| self.proven(&l, start));
        let gate = crate::lock(&self.resume).open(Instant::now());
        if !agent {
            return "the agent stopped before its restored shell closed; its block and the shell stay";
        }
        let confirmed = Agent {
            kind: start.kind.clone(),
            conversation: start.conversation.clone(),
            folder: start.folder.clone(),
        };
        crate::lock(&self.resume)
            .resumed
            .insert(start.new.clone(), confirmed);
        if !shell {
            return "resumed beside its restored shell, which changed";
        }
        if !gate {
            return "resumed beside its restored shell; resuming stopped or its window ended";
        }
        self.move_task(block, &start.new);
        match self.close_block(&start.session, block).await {
            Ok(()) => "resumed in place of its restored shell",
            Err(_) => "resumed; its restored shell could not be closed",
        }
    }

    async fn close_block(&self, session: &str, block: &str) -> Result<(), Error> {
        let client = self.live(|l| l.client.clone())?;
        let payload = json!({ "session_id": session, "block_id": block });
        client
            .call::<Value>(Method::BlockClose, payload, CLOSE_TIMEOUT)
            .await?;
        Ok(())
    }
}

async fn start(
    client: &Client,
    session: &str,
    block: &str,
    cwd: &str,
    argv: Vec<String>,
) -> Result<String, Error> {
    let options = json!({ "command": argv, "cwd": cwd, "shell": "none", "login": false });
    let layout = json!({ "block": { "flavor": FLAVOR, "options": options } });
    let payload = json!({
        "session_id": session, "anchor_id": block, "direction": "horizontal",
        "side": "before", "layout": layout,
    });
    let (_, split): (_, Split) = client
        .call(Method::SessionNewSplit, payload, TIMEOUT)
        .await?;
    let new = (split.block_ids.into_iter().flatten().flatten())
        .find(|b| model::valid(b, "block"))
        .ok_or_else(|| {
            Error::Unsupported("Rex split the block without a block collied can name".to_owned())
        })?;
    let _ = disown(client, session, &new).await;
    Ok(new)
}

async fn idle(client: &Client, session: &str, block: &str) -> Option<u32> {
    let (_, p): (_, Process) = (client.terminal(Method::Process, session, block, json!({})))
        .await
        .ok()?;
    let (child, fg) = (p.child?, p.foreground?);
    let alone = match p.ancestors.as_slice() {
        [] => fg.pid == child.pid,
        [parent] => parent.pid == child.pid && child.name == "login",
        _ => false,
    };
    (p.last_exit.is_none() && model::shell(&fg) && alone).then_some(fg.pid)
}

async fn screen(client: &Client, session: &str, block: &str, size: (u16, u16)) -> Option<String> {
    let args = json!({ "format": "vt", "max_bytes": MAX_TAIL });
    let (_, f): (_, Formatted) = (client.terminal(Method::Format, session, block, args))
        .await
        .ok()?;
    let mut screen = Screen::new(size.0, size.1, 0);
    screen.feed(f.content.as_bytes());
    f.id.is_none().then(|| screen.text())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::fake::{self, Fake};
    use super::super::tests::{follow, id};
    use super::*;
    use crate::runtime::STRIPPED;

    const S: &str = "session:019a7d2e-0000-7000-8000-0000000000a1";
    const READY: &str = "block:019a7d2e-0000-7000-8000-0000000000e1";
    const TWICE: &str = "block:019a7d2e-0000-7000-8000-0000000000e2";
    const TYPED: &str = "block:019a7d2e-0000-7000-8000-0000000000e3";
    const BUSY: &str = "block:019a7d2e-0000-7000-8000-0000000000e4";
    const GONE: &str = "block:019a7d2e-0000-7000-8000-0000000000e5";
    const OTHER: &str = "block:019a7d2e-0000-7000-8000-0000000000e6";
    const NESTED: &str = "block:019a7d2e-0000-7000-8000-0000000000e7";
    const FIRST: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    const SECOND: &str = "0a0a0a0a-0000-4000-8000-000000000002";
    const THIRD: &str = "0a0a0a0a-0000-4000-8000-000000000003";
    const FOURTH: &str = "0a0a0a0a-0000-4000-8000-000000000004";
    const FIFTH: &str = "0a0a0a0a-0000-4000-8000-000000000005";
    const SIXTH: &str = "0a0a0a0a-0000-4000-8000-000000000006";
    const PROMPT: &str = "user@host app % \x1b[1;17H";

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

    fn through_env(cli: &str, args: &[&str]) -> Vec<String> {
        let mut argv = vec!["/usr/bin/env".to_owned()];
        for name in STRIPPED {
            argv.extend(["-u".to_owned(), name.to_owned()]);
        }
        argv.push(cli.to_owned());
        argv.extend(args.iter().map(|a| (*a).to_owned()));
        argv
    }

    fn login_shell() -> serde_json::Value {
        serde_json::json!({
            "last_exit": null,
            "child": { "pid": 1, "user_id": 501, "name": "login", "argv0": "login" },
            "foreground": { "pid": 10, "user_id": 501, "name": "zsh", "argv0": "-zsh", "cwd": "/work/app" },
            "ancestors": [{ "pid": 1, "user_id": 501, "name": "login", "argv0": "login" }],
        })
    }

    fn in_vim() -> serde_json::Value {
        let mut p = fake::under_shell(11, "vim", "vim");
        p["ancestors"][0]["pid"] = serde_json::json!(10);
        p
    }

    fn nested() -> serde_json::Value {
        let mut p = fake::under_shell(12, "bash", "bash");
        p["ancestors"][0]["pid"] = serde_json::json!(10);
        p
    }

    fn zsh() -> Option<String> {
        Some("/bin/zsh".to_owned())
    }

    #[test]
    fn each_kind_resumes_through_its_bare_name_and_only_a_checked_id_reaches_the_shell() {
        let of = |kind, conversation| command(&agent(kind, conversation, "/work"));
        let expected = [
            ("claude", format!("claude --resume {FIRST}")),
            ("codex", format!("codex resume {FIRST} --no-daemon")),
            ("copilot", format!("copilot --resume={FIRST}")),
            ("agy", format!("agy --conversation {FIRST}")),
        ];
        for (kind, line) in expected {
            assert_eq!(of(kind, FIRST), Some(line));
            for bad in [
                "------------------------------------",
                "3F1B2C4D-5E6F-4A7B-8C9D-0E1F2A3B4C5D",
                "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5;",
                "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d ",
                "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3$(id)",
                "",
            ] {
                assert_eq!(of(kind, bad), None, "{kind} {bad:?}");
            }
        }
        assert_eq!(of("pi", FIRST), None);
        assert_eq!(of("claude; rm", FIRST), None);
    }

    #[test]
    fn only_a_new_instance_that_started_just_now_plans_what_the_last_one_ran() {
        let (now, t) = (10_000_000, Instant::now());
        let mut r = Resume::default();
        let ran = BTreeMap::from([(READY.to_owned(), agent("claude", FIRST, "/work"))]);
        r.plan(Some("one"), Some(now), now, t);
        assert!(r.due(t).is_empty(), "the first instance collied sees");
        r.record.agents = ran.clone();
        r.plan(Some("two"), Some(now - 90_000), now, t);
        assert!(r.due(t).is_empty(), "an instance that started too long ago");
        assert!(r.record.agents.is_empty());
        r.record.agents = ran.clone();
        r.plan(Some("three"), Some(now + 1), now, t);
        assert!(
            r.due(t).is_empty(),
            "an instance that says it started later than now"
        );
        r.record.agents = ran.clone();
        r.plan(Some("four"), Some(now - 30_000), now, t);
        assert_eq!(r.due(t), ran.clone().into_iter().collect::<Vec<_>>());
        assert!(
            r.record.agents.is_empty(),
            "taken off the record before acting"
        );
        r.plan(Some("four"), Some(now - 30_000), now, t);
        assert_eq!(
            r.due(t).len(),
            1,
            "a reconnect to the same instance keeps the plan"
        );
        assert_eq!(r.due(t + Duration::from_secs(59)).len(), 1);
        assert!(
            r.due(t + Duration::from_secs(60)).is_empty(),
            "90 s after the start, on collied's clock"
        );
        r.record.agents = ran;
        r.off = true;
        r.plan(Some("five"), Some(now), now, t);
        assert!(r.due(t).is_empty(), "turned off");
    }

    #[test]
    fn a_claim_needs_the_plan_the_window_and_the_option_at_that_moment() {
        let t = Instant::now();
        let planned = || {
            let mut r = Resume::default();
            r.record.instance = Some("one".to_owned());
            r.pending = BTreeMap::from([
                (READY.to_owned(), agent("claude", FIRST, "/w")),
                (TWICE.to_owned(), agent("claude", FIRST, "/w")),
            ]);
            r.until = Some(t + Duration::from_secs(10));
            r
        };
        let mut r = planned();
        assert!(r.claim(READY, FIRST, t));
        assert!(!r.claim(READY, FIRST, t), "once");
        assert!(!r.claim(TWICE, FIRST, t), "one block per conversation");
        let mut late = planned();
        assert!(!late.claim(READY, FIRST, t + Duration::from_secs(10)));
        let mut off = planned();
        off.off = true;
        assert!(!off.claim(READY, FIRST, t));
    }

    #[test]
    fn an_agent_that_left_its_block_is_kept_for_the_grace_even_across_a_reconnect() {
        let t = Instant::now();
        let mut r = Resume::default();
        r.plan(Some("one"), None, 0, t);
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
        r.plan(Some("one"), None, 0, t + Duration::from_secs(10));
        r.observe(Some("one"), &none, t + GRACE - Duration::from_millis(1));
        assert_eq!(
            r.record.agents, running,
            "a shutdown ends agents before the server"
        );
        r.observe(Some("one"), &none, t + GRACE);
        assert!(
            r.record.agents.is_empty(),
            "the reconnect did not restart the grace"
        );
    }

    #[test]
    fn a_record_too_large_or_out_of_bounds_is_cut_when_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rex-resume.json");
        let block = |n: usize| format!("block:019a7d2e-0000-7000-8000-{n:012x}");
        let mut agents: BTreeMap<String, Agent> = (0..MAX_AGENTS + 5)
            .map(|n| (block(n), agent("claude", FIRST, "/w")))
            .collect();
        agents.insert(
            block(9999),
            agent("claude", FIRST, &format!("/{}", "x".repeat(MAX_FOLDER))),
        );
        agents.insert("../x".to_owned(), agent("claude", FIRST, "/w"));
        agents.insert(block(9998), agent("claude", FIRST, "relative"));
        let record = Record {
            instance: Some("one".to_owned()),
            agents,
        };
        crate::peers::save_json(&path, &record).unwrap();
        let cut = read(&path);
        assert_eq!(cut.agents.len(), MAX_AGENTS);
        assert!(cut.agents.keys().all(|b| model::valid(b, "block")));
        assert!(cut.agents.values().all(|a| a.folder == "/w"));
        let folder = format!("/{}", "x".repeat(2000));
        let large = Record {
            instance: Some("one".to_owned()),
            agents: (0..200)
                .map(|n| (block(n), agent("claude", FIRST, &folder)))
                .collect(),
        };
        crate::peers::save_json(&path, &large).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > MAX_FILE);
        assert_eq!(read(&path), Record::default(), "too large to read at all");
    }

    #[test]
    fn turning_it_off_forgets_the_record_and_stays_off_across_a_restart() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("rex-resume.json");
        let off = dir.path().join("rex-resume-off");
        let ran = Record {
            instance: Some("one".to_owned()),
            agents: BTreeMap::from([(READY.to_owned(), agent("claude", FIRST, "/work"))]),
        };
        crate::peers::save_json(&record, &ran).unwrap();
        let mut r = Resume::load(record.clone(), off.clone());
        assert!(!r.off);
        assert_eq!(r.record, ran);
        r.set(false, Some("one".to_owned())).unwrap();
        assert!(!record.exists(), "the record is deleted");
        let mode = std::fs::metadata(&off).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let (now, t) = (10_000_000, Instant::now());
        let mut again = Resume::load(record.clone(), off.clone());
        assert!(again.off);
        again.plan(Some("two"), Some(now), now, t);
        again.observe(Some("two"), &ran.agents, t);
        assert!(again.due(t).is_empty() && !record.exists());
        again.set(true, Some("two".to_owned())).unwrap();
        assert!(!again.off && !off.exists());
        again.observe(Some("two"), &ran.agents, t);
        let saved: Record = crate::peers::load_json(&record).unwrap();
        assert_eq!(saved.instance.as_deref(), Some("two"));
        assert_eq!(saved.agents, ran.agents);
    }

    #[tokio::test]
    async fn a_followed_claude_code_with_its_session_is_recorded_with_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir(&sessions).unwrap();
        let (me, now) = (std::process::id(), crate::now_ms());
        let claim = serde_json::json!({"pid": me, "sessionId": FIRST, "cwd": "/work/app", "startedAt": now});
        std::fs::write(sessions.join(format!("{me}.json")), claim.to_string()).unwrap();
        let record = dir.path().join("rex-resume.json");
        let fake = Fake::start().await;
        {
            let mut s = fake.state();
            s.instance = "one";
            s.sessions.push((S.to_owned(), "work".to_owned()));
            s.add(S, READY, fake::process(me, "claude", "2.1.322"), Vec::new());
            s.add(
                S,
                TWICE,
                fake::process(u32::MAX - 1, "claude", "2.1.322"),
                Vec::new(),
            );
            s.add(S, TYPED, login_shell(), Vec::new());
        }
        let rex = Rex::new(fake.socket.clone())
            .with_claude_sessions(Some(sessions))
            .with_resume(record.clone(), dir.path().join("rex-resume-off"));
        let rex = Arc::new(rex);
        let _task = follow(&rex).await;
        assert!(
            rex.get(&id(TWICE)).is_ok(),
            "an agent with no session is listed"
        );
        rex.resume_with(zsh).await;
        let saved: Record = crate::peers::load_json(&record).unwrap();
        assert!(
            saved.agents.is_empty(),
            "a conversation with no transcript yet"
        );
        transcribe(dir.path(), "/work/app", FIRST);
        rex.resume_with(zsh).await;
        let saved: Record = crate::peers::load_json(&record).unwrap();
        assert_eq!(saved.instance.as_deref(), Some("one"));
        assert_eq!(
            saved.agents,
            BTreeMap::from([(READY.to_owned(), agent("claude", FIRST, "/work/app"))])
        );
    }

    fn key(folder: &str) -> String {
        folder
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    }

    fn transcribe(claude: &Path, folder: &str, conversation: &str) {
        let dir = claude.join("projects").join(key(folder));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{conversation}.jsonl")), "{}\n").unwrap();
    }

    fn pid_file(sessions: &Path, conversation: &str) {
        let me = std::process::id();
        let claim = serde_json::json!({"pid": me, "sessionId": conversation, "cwd": "/w", "startedAt": crate::now_ms()});
        std::fs::write(sessions.join(format!("{me}.json")), claim.to_string()).unwrap();
    }

    struct Restarted {
        fake: Fake,
        rex: Arc<Rex>,
        record: PathBuf,
        sessions: PathBuf,
    }

    async fn restarted(
        dir: &Path,
        agents: &[(&str, Agent)],
        blocks: &[(&str, serde_json::Value, &str)],
        untranscribed: &[&str],
    ) -> Restarted {
        let record = dir.join("rex-resume.json");
        let claude = dir.join("claude");
        let sessions = claude.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        for (_, a) in agents {
            if !untranscribed.contains(&a.conversation.as_str()) {
                transcribe(&claude, &a.folder, &a.conversation);
            }
        }
        let saved = Record {
            instance: Some("one".to_owned()),
            agents: (agents.iter())
                .map(|(b, a)| ((*b).to_owned(), a.clone()))
                .collect(),
        };
        crate::peers::save_json(&record, &saved).unwrap();
        let fake = Fake::start().await;
        {
            let mut s = fake.state();
            s.instance = "two";
            s.started_at = rfc3339(crate::now_ms());
            s.split_pid = Some(std::process::id());
            s.sessions.push((S.to_owned(), "work".to_owned()));
            for (block, process, screen) in blocks {
                s.add(S, block, process.clone(), Vec::new());
                s.block(block).screen = (*screen).to_owned();
            }
        }
        let rex = Rex::new(fake.socket.clone())
            .with_claude_sessions(Some(sessions.clone()))
            .with_resume(record.clone(), dir.join("rex-resume-off"));
        Restarted {
            fake,
            rex: Arc::new(rex),
            record,
            sessions,
        }
    }

    fn anchors(fake: &Fake) -> Vec<String> {
        (fake.state().splits.iter())
            .map(|s| s["anchor_id"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    fn blocks(fake: &Fake) -> Vec<String> {
        fake.state().blocks.iter().map(|b| b.id.clone()).collect()
    }

    async fn ticks(rex: &Rex, until: impl Fn() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            rex.reread_sessions();
            rex.resume_with(zsh).await;
            if until() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    #[tokio::test]
    async fn only_idle_shells_of_transcribed_conversations_get_an_agent_once() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let folder = folder.to_str().unwrap();
        let r = restarted(
            dir.path(),
            &[
                (READY, agent("claude", FIRST, folder)),
                (TWICE, agent("codex", FIRST, folder)),
                (TYPED, agent("claude", SECOND, folder)),
                (BUSY, agent("claude", THIRD, folder)),
                (GONE, agent("claude", FOURTH, "/no/such/folder")),
                (NESTED, agent("claude", FIFTH, folder)),
                (OTHER, agent("claude", SIXTH, folder)),
            ],
            &[
                (READY, login_shell(), PROMPT),
                (TWICE, login_shell(), PROMPT),
                (TYPED, login_shell(), "user@host app % ls"),
                (BUSY, in_vim(), PROMPT),
                (GONE, login_shell(), PROMPT),
                (NESTED, nested(), PROMPT),
                (OTHER, login_shell(), PROMPT),
            ],
            &[SIXTH],
        )
        .await;
        let _task = follow(&r.rex).await;
        assert!(ticks(&r.rex, || !anchors(&r.fake).is_empty()).await);
        for _ in 0..3 {
            r.rex.resume_with(zsh).await;
            tokio::time::sleep(SETTLE).await;
        }
        assert_eq!(anchors(&r.fake), [READY, TYPED]);
        let split = r.fake.state().splits[0].clone();
        assert_eq!(split["side"], "before");
        let script = format!("builtin cd -- \"$1\" && claude --resume {FIRST}");
        assert_eq!(
            split["layout"]["block"]["options"],
            serde_json::json!({
                "command": through_env("/bin/zsh", &["-l", "-i", "-c", &script, "collied", folder]),
                "cwd": folder,
                "shell": "none",
                "login": false,
            })
        );
        let on_disk: Record = crate::peers::load_json(&r.record).unwrap();
        assert_eq!(on_disk.instance.as_deref(), Some("two"));
        assert!(on_disk.agents.is_empty());
    }

    #[tokio::test]
    async fn a_shell_whose_screen_changes_after_collied_first_read_it_gets_no_agent() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let r = restarted(
            dir.path(),
            &[(READY, agent("claude", FIRST, folder.to_str().unwrap()))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await;
        let _task = follow(&r.rex).await;
        r.rex.resume_with(zsh).await;
        tokio::time::sleep(SETTLE).await;
        r.fake.state().block(READY).screen = "user@host app % ls".to_owned();
        for _ in 0..6 {
            r.rex.resume_with(zsh).await;
            tokio::time::sleep(SETTLE).await;
        }
        assert!(anchors(&r.fake).is_empty(), "typed after the first read");
        assert!(
            crate::lock(&r.rex.resume).pending.is_empty(),
            "refused, not waiting"
        );
    }

    #[tokio::test]
    async fn a_prompt_that_redraws_right_after_the_first_read_still_settles() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let r = restarted(
            dir.path(),
            &[(READY, agent("claude", FIRST, folder.to_str().unwrap()))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await;
        let _task = follow(&r.rex).await;
        r.rex.resume_with(zsh).await;
        r.fake.state().block(READY).screen = "user@host app on main % \x1b[1;25H".to_owned();
        assert!(ticks(&r.rex, || !anchors(&r.fake).is_empty()).await);
    }

    #[tokio::test]
    async fn common_prompts_get_their_agent_back() {
        let prompts = [
            "user@host ~/app\r\n\u{276f}\x1b[2;2H",
            "user@host app %                                     [12:00:01]\x1b[1;17H",
            "~/app on main\r\n$\x1b[2;2H",
        ];
        for prompt in prompts {
            let dir = tempfile::tempdir().unwrap();
            let folder = std::fs::canonicalize(dir.path()).unwrap();
            let r = restarted(
                dir.path(),
                &[(READY, agent("claude", FIRST, folder.to_str().unwrap()))],
                &[(READY, login_shell(), prompt)],
                &[],
            )
            .await;
            let _task = follow(&r.rex).await;
            assert!(
                ticks(&r.rex, || !anchors(&r.fake).is_empty()).await,
                "{prompt:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_restored_shell_closes_only_once_the_agent_runs_its_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let r = restarted(
            dir.path(),
            &[(READY, agent("claude", FIRST, folder.to_str().unwrap()))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await;
        let _task = follow(&r.rex).await;
        assert!(ticks(&r.rex, || !anchors(&r.fake).is_empty()).await);
        assert_eq!(anchors(&r.fake), [READY]);
        for _ in 0..5 {
            r.rex.resume_with(zsh).await;
            tokio::time::sleep(CONFIRM / 2).await;
        }
        assert!(
            blocks(&r.fake).contains(&READY.to_owned()),
            "not before the agent runs it"
        );
        pid_file(&r.sessions, FIRST);
        assert!(ticks(&r.rex, || !blocks(&r.fake).contains(&READY.to_owned())).await);
        let new = blocks(&r.fake);
        assert_eq!(new.len(), 1);
        assert_eq!(
            r.fake.state().block(&new[0]).owner,
            None,
            "the new block was given up"
        );
    }

    #[tokio::test]
    async fn a_new_block_not_seen_yet_is_never_taken_for_one_that_exited() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let r = restarted(
            dir.path(),
            &[(READY, agent("claude", FIRST, folder.to_str().unwrap()))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await;
        r.fake.state().split_host = Some("another-host".to_owned());
        let _task = follow(&r.rex).await;
        assert!(ticks(&r.rex, || !anchors(&r.fake).is_empty()).await);
        assert_eq!(anchors(&r.fake), [READY]);
        assert!(ticks(&r.rex, || crate::lock(&r.rex.resume).starting.is_empty()).await);
        assert_eq!(blocks(&r.fake).len(), 2, "the new block and the shell stay");
    }

    #[tokio::test]
    async fn an_agent_that_exits_at_once_keeps_its_block_beside_the_restored_shell() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let r = restarted(
            dir.path(),
            &[(READY, agent("claude", FIRST, folder.to_str().unwrap()))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await;
        r.fake.state().split_exits = true;
        let _task = follow(&r.rex).await;
        assert!(ticks(&r.rex, || !anchors(&r.fake).is_empty()).await);
        assert_eq!(anchors(&r.fake), [READY]);
        assert!(ticks(&r.rex, || crate::lock(&r.rex.resume).starting.is_empty()).await);
        let ids = blocks(&r.fake);
        assert_eq!(ids.len(), 2, "its output stays readable: {ids:?}");
        assert!(ids.contains(&READY.to_owned()));
    }

    #[tokio::test]
    async fn a_conversation_a_live_claude_holds_waits_until_it_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let r = restarted(
            dir.path(),
            &[(READY, agent("claude", FIRST, folder.to_str().unwrap()))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await;
        pid_file(&r.sessions, FIRST);
        let _task = follow(&r.rex).await;
        for _ in 0..4 {
            r.rex.resume_with(zsh).await;
            tokio::time::sleep(SETTLE).await;
        }
        assert!(anchors(&r.fake).is_empty(), "held by a live process");
        std::fs::remove_file(r.sessions.join(format!("{}.json", std::process::id()))).unwrap();
        assert!(ticks(&r.rex, || !anchors(&r.fake).is_empty()).await);
        assert_eq!(anchors(&r.fake), [READY]);
    }

    #[tokio::test]
    async fn a_conversation_running_in_another_block_is_not_started_again() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let me = std::process::id();
        let r = restarted(
            dir.path(),
            &[(READY, agent("claude", FIRST, folder.to_str().unwrap()))],
            &[
                (READY, login_shell(), PROMPT),
                (OTHER, fake::process(me, "claude", "2.1.322"), ""),
            ],
            &[],
        )
        .await;
        pid_file(&r.sessions, FIRST);
        let _task = follow(&r.rex).await;
        let bound = |rex: &Rex| {
            rex.get(&id(OTHER))
                .ok()
                .and_then(|a| a.agent_session)
                .map(|s| s.value)
        };
        assert_eq!(bound(&r.rex).as_deref(), Some(FIRST));
        std::fs::remove_file(r.sessions.join(format!("{me}.json"))).unwrap();
        for _ in 0..4 {
            r.rex.resume_with(zsh).await;
            tokio::time::sleep(SETTLE).await;
        }
        assert_eq!(
            bound(&r.rex).as_deref(),
            Some(FIRST),
            "still listed as running it"
        );
        assert!(anchors(&r.fake).is_empty());
    }

    #[tokio::test]
    async fn a_shell_that_changes_or_a_resume_turned_off_before_the_close_keeps_both() {
        for change in ["process", "screen", "off"] {
            let dir = tempfile::tempdir().unwrap();
            let folder = std::fs::canonicalize(dir.path()).unwrap();
            let r = restarted(
                dir.path(),
                &[(READY, agent("claude", FIRST, folder.to_str().unwrap()))],
                &[(READY, login_shell(), PROMPT)],
                &[],
            )
            .await;
            let _task = follow(&r.rex).await;
            assert!(
                ticks(&r.rex, || !anchors(&r.fake).is_empty()).await,
                "{change}"
            );
            assert_eq!(anchors(&r.fake), [READY], "{change}");
            match change {
                "process" => r.fake.state().block(READY).process = in_vim(),
                "screen" => r.fake.state().block(READY).screen = "user@host app % ls".to_owned(),
                _ => r.rex.set_resumes(false).unwrap(),
            }
            pid_file(&r.sessions, FIRST);
            assert!(
                ticks(&r.rex, || crate::lock(&r.rex.resume).starting.is_empty()).await,
                "{change}"
            );
            assert_eq!(
                blocks(&r.fake).len(),
                2,
                "{change}: the agent and the user's shell stay"
            );
            assert!(blocks(&r.fake).contains(&READY.to_owned()), "{change}");
        }
    }

    #[tokio::test]
    async fn a_resumed_task_keeps_its_record_on_its_new_block() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let tasks = dir.path().join("rex-tasks.json");
        let task = serde_json::json!({ READY: { "name": "collie-0a1b2c3d", "session": S } });
        crate::peers::save_json(&tasks, &task).unwrap();
        let r = restarted(
            dir.path(),
            &[(READY, agent("claude", FIRST, folder.to_str().unwrap()))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await;
        let rex = Rex::new(r.fake.socket.clone())
            .with_tasks(tasks.clone())
            .with_claude_sessions(Some(r.sessions.clone()))
            .with_resume(r.record.clone(), dir.path().join("rex-resume-off"));
        let rex = Arc::new(rex);
        let _task = follow(&rex).await;
        assert!(ticks(&rex, || !anchors(&r.fake).is_empty()).await);
        pid_file(&r.sessions, FIRST);
        assert!(ticks(&rex, || !blocks(&r.fake).contains(&READY.to_owned())).await);
        let new = blocks(&r.fake)[0].clone();
        super::super::tests::eventually("the resumed task listed", || {
            rex.get(&id(&new))
                .is_ok_and(|a| a.name.as_deref() == Some("collie-0a1b2c3d"))
        })
        .await;
        let saved: serde_json::Value = crate::peers::load_json(&tasks).unwrap();
        assert!(
            saved.get(READY).is_none() && saved.get(&new).is_some(),
            "{saved}"
        );
    }

    fn new_block(fake: &Fake) -> String {
        fake.state().blocks.last().unwrap().id.clone()
    }

    async fn running_since(rex: &Rex) {
        let end = Instant::now() + Duration::from_secs(5);
        while (crate::lock(&rex.resume).starting.get(READY)).is_none_or(|s| s.running.is_none()) {
            assert!(
                Instant::now() < end,
                "the agent never showed its conversation"
            );
            rex.reread_sessions();
            rex.resume_with(zsh).await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn started_ready(dir: &Path) -> Restarted {
        let folder = std::fs::canonicalize(dir).unwrap();
        restarted(
            dir,
            &[(READY, agent("claude", FIRST, folder.to_str().unwrap()))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await
    }

    #[tokio::test]
    async fn a_seen_block_missing_from_a_rebuilt_model_is_never_taken_for_an_exit() {
        let dir = tempfile::tempdir().unwrap();
        let r = started_ready(dir.path()).await;
        let task = follow(&r.rex).await;
        assert!(ticks(&r.rex, || !anchors(&r.fake).is_empty()).await);
        let new = new_block(&r.fake);
        assert!(
            ticks(&r.rex, || crate::lock(&r.rex.resume)
                .starting
                .get(READY)
                .is_some_and(|s| s.seen))
            .await
        );
        r.fake.state().block(&new).process = serde_json::json!({ "foreground": 5 });
        r.fake.drop_connection();
        let _ = task.await;
        let _task = follow(&r.rex).await;
        assert!(ticks(&r.rex, || crate::lock(&r.rex.resume).starting.is_empty()).await);
        let ids = blocks(&r.fake);
        assert!(
            ids.contains(&new) && ids.contains(&READY.to_owned()),
            "{ids:?}"
        );
    }

    #[tokio::test]
    async fn an_agent_that_exits_during_the_last_shell_reads_leaves_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let r = started_ready(dir.path()).await;
        let _task = follow(&r.rex).await;
        assert!(ticks(&r.rex, || !anchors(&r.fake).is_empty()).await);
        let new = new_block(&r.fake);
        pid_file(&r.sessions, FIRST);
        running_since(&r.rex).await;
        r.fake.state().hook(
            Method::Format,
            Some(READY),
            true,
            Box::new(move |s| {
                let block = s.blocks.last().unwrap().id.clone();
                s.block(&block).process = serde_json::json!({
                    "last_exit": { "exit_code": 1 }, "child": null, "foreground": null,
                });
                vec![fake::block_event(
                    S,
                    &block,
                    "child_exited",
                    serde_json::json!({}),
                )]
            }),
        );
        assert!(ticks(&r.rex, || crate::lock(&r.rex.resume).starting.is_empty()).await);
        assert!(
            blocks(&r.fake).contains(&READY.to_owned()),
            "the shell stays"
        );
        let _ = new;
    }

    #[tokio::test]
    async fn an_agent_in_another_folder_does_not_replace_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let r = started_ready(dir.path()).await;
        r.fake.state().split_cwd = Some("/elsewhere".to_owned());
        let _task = follow(&r.rex).await;
        assert!(ticks(&r.rex, || !anchors(&r.fake).is_empty()).await);
        pid_file(&r.sessions, FIRST);
        assert!(ticks(&r.rex, || crate::lock(&r.rex.resume).starting.is_empty()).await);
        assert_eq!(blocks(&r.fake).len(), 2, "both stay");
    }

    #[tokio::test]
    async fn a_shell_is_never_closed_after_the_window_ends() {
        let dir = tempfile::tempdir().unwrap();
        let r = started_ready(dir.path()).await;
        let _task = follow(&r.rex).await;
        assert!(ticks(&r.rex, || !anchors(&r.fake).is_empty()).await);
        crate::lock(&r.rex.resume).until = Some(Instant::now());
        pid_file(&r.sessions, FIRST);
        assert!(ticks(&r.rex, || crate::lock(&r.rex.resume).starting.is_empty()).await);
        assert_eq!(blocks(&r.fake).len(), 2, "both stay");
    }

    fn v7(n: u64) -> String {
        let ms = crate::now_ms();
        format!("{:08x}-{:04x}-7000-8000-{n:012x}", ms >> 16, ms & 0xffff)
    }

    fn with_roots(r: &Restarted, dir: &Path) -> Arc<Rex> {
        let roots = crate::costs::Roots {
            claude: None,
            codex: Some(dir.join("codex/sessions")),
            copilot: Some(dir.join("copilot/session-state")),
            agy: Some(dir.join("agy/brain")),
        };
        Arc::new(
            Rex::new(r.fake.socket.clone())
                .with_claude_sessions(Some(r.sessions.clone()))
                .with_agent_roots(roots)
                .with_resume(r.record.clone(), dir.join("rex-resume-off")),
        )
    }

    fn rollout(dir: &Path, id: &str, folder: &str) -> PathBuf {
        let day = rfc3339(crate::now_ms())[..10].to_owned();
        let path = (dir.join("codex/sessions").join(day.replace('-', "/")))
            .join(format!("rollout-{day}T00-00-00-{id}.jsonl"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let meta =
            serde_json::json!({"type": "session_meta", "payload": {"id": id, "cwd": folder}});
        std::fs::write(&path, format!("{meta}\n")).unwrap();
        path
    }

    fn writer_lock(dir: &Path, id: &str) -> std::fs::File {
        let path = dir
            .join("codex/thread-writer-locks")
            .join(format!("{id}.lock"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(&path).unwrap()
    }

    fn copilot_session(dir: &Path, id: &str, folder: &str) -> PathBuf {
        let session = dir.join("copilot/session-state").join(id);
        std::fs::create_dir_all(&session).unwrap();
        let workspace = format!("id: {id}\ncwd: {folder}\n");
        std::fs::write(session.join("workspace.yaml"), workspace).unwrap();
        let start = serde_json::json!({"type": "session.start", "data": {"sessionId": id}});
        std::fs::write(session.join("events.jsonl"), format!("{start}\n")).unwrap();
        session
    }

    fn copilot_marks(session: &Path) -> std::fs::File {
        let me = std::process::id();
        std::fs::write(session.join(format!("inuse.{me}.lock")), me.to_string()).unwrap();
        let hold = std::fs::File::create(session.join(format!("inuse.{me}.hold"))).unwrap();
        hold.lock().unwrap();
        hold
    }

    fn agy_transcript(dir: &Path, id: &str) {
        let logs = dir
            .join("agy/brain")
            .join(id)
            .join(".system_generated/logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("transcript.jsonl"), "{}\n").unwrap();
    }

    async fn still_there(rex: &Rex, fake: &Fake) -> bool {
        for _ in 0..4 {
            rex.resume_with(zsh).await;
            tokio::time::sleep(CONFIRM / 2).await;
        }
        blocks(fake).contains(&READY.to_owned())
    }

    #[tokio::test]
    async fn a_resumed_codex_replaces_the_shell_only_once_its_process_holds_the_lock_and_rollout() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let folder = folder.to_str().unwrap();
        let id = v7(1);
        let r = restarted(
            dir.path(),
            &[(READY, agent("codex", &id, folder))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await;
        let path = rollout(dir.path(), &id, folder);
        let lock = writer_lock(dir.path(), &id);
        let rex = with_roots(&r, dir.path());
        let _task = follow(&rex).await;
        assert!(ticks(&rex, || !anchors(&r.fake).is_empty()).await);
        lock.lock().unwrap();
        assert!(still_there(&rex, &r.fake).await, "the lock alone");
        let _open = std::fs::File::open(&path).unwrap();
        assert!(ticks(&rex, || !blocks(&r.fake).contains(&READY.to_owned())).await);
        let new = new_block(&r.fake);
        let expected = agent("codex", &id, folder);
        let recorded = || {
            crate::peers::load_json::<Record>(&r.record)
                .is_ok_and(|s| s.agents.get(&new) == Some(&expected))
        };
        assert!(
            ticks(&rex, recorded).await,
            "recorded under its new block before its first prompt"
        );
    }

    #[tokio::test]
    async fn a_resumed_copilot_replaces_the_shell_only_once_its_process_marks_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let folder = folder.to_str().unwrap();
        let r = restarted(
            dir.path(),
            &[(READY, agent("copilot", FIRST, folder))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await;
        let session = copilot_session(dir.path(), FIRST, folder);
        let rex = with_roots(&r, dir.path());
        let _task = follow(&rex).await;
        assert!(ticks(&rex, || !anchors(&r.fake).is_empty()).await);
        assert!(still_there(&rex, &r.fake).await, "no marker of its process");
        let hold = copilot_marks(&session);
        let other = copilot_marks(&copilot_session(dir.path(), SECOND, folder));
        assert!(
            still_there(&rex, &r.fake).await,
            "its process marks two sessions"
        );
        drop(other);
        assert!(ticks(&rex, || !blocks(&r.fake).contains(&READY.to_owned())).await);
        drop(hold);
    }

    #[tokio::test]
    async fn a_resumed_agy_replaces_the_shell_only_once_its_hook_names_the_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let r = restarted(
            dir.path(),
            &[(READY, agent("agy", FIRST, folder.to_str().unwrap()))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await;
        agy_transcript(dir.path(), FIRST);
        let rex = with_roots(&r, dir.path());
        let _task = follow(&rex).await;
        assert!(ticks(&rex, || !anchors(&r.fake).is_empty()).await);
        assert!(still_there(&rex, &r.fake).await, "an agy with no report");
        assert!(rex.bind(&id(&new_block(&r.fake)), FIRST, std::process::id()));
        assert!(ticks(&rex, || !blocks(&r.fake).contains(&READY.to_owned())).await);
    }

    fn presence_lock(dir: &Path, id: &str) -> std::fs::File {
        let presence = dir.join("agy/presence");
        std::fs::create_dir_all(&presence).unwrap();
        std::fs::File::create(presence.join(format!("{id}.lock"))).unwrap()
    }

    #[tokio::test]
    async fn a_resumed_agy_replaces_the_shell_once_its_process_has_only_its_presence_lock_open() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let r = restarted(
            dir.path(),
            &[(READY, agent("agy", FIRST, folder.to_str().unwrap()))],
            &[(READY, login_shell(), PROMPT)],
            &[],
        )
        .await;
        agy_transcript(dir.path(), FIRST);
        drop(presence_lock(dir.path(), THIRD));
        let rex = with_roots(&r, dir.path());
        let _task = follow(&rex).await;
        assert!(ticks(&rex, || !anchors(&r.fake).is_empty()).await);
        let own = presence_lock(dir.path(), FIRST);
        let other = presence_lock(dir.path(), SECOND);
        assert!(still_there(&rex, &r.fake).await, "two presence locks open");
        drop(other);
        assert!(ticks(&rex, || !blocks(&r.fake).contains(&READY.to_owned())).await);
        drop(own);
    }

    #[tokio::test]
    async fn a_copilot_is_recorded_from_the_one_session_its_process_marks() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("rex-resume.json");
        let fake = Fake::start().await;
        {
            let mut s = fake.state();
            s.instance = "one";
            s.sessions.push((S.to_owned(), "work".to_owned()));
            let me = std::process::id();
            s.add(
                S,
                READY,
                fake::process(me, "copilot", "copilot"),
                Vec::new(),
            );
        }
        let roots = crate::costs::Roots {
            copilot: Some(dir.path().join("copilot/session-state")),
            ..crate::costs::Roots::default()
        };
        let rex = Rex::new(fake.socket.clone())
            .with_agent_roots(roots)
            .with_resume(record.clone(), dir.path().join("rex-resume-off"));
        let rex = Arc::new(rex);
        let _task = follow(&rex).await;
        let session = copilot_session(dir.path(), FIRST, "/work/app");
        let other = copilot_session(dir.path(), SECOND, "/work/app");
        rex.resume_with(zsh).await;
        let saved: Record = crate::peers::load_json(&record).unwrap();
        assert!(saved.agents.is_empty(), "no marker of its process");
        let _hold = copilot_marks(&session);
        let second = copilot_marks(&other);
        rex.resume_with(zsh).await;
        let saved: Record = crate::peers::load_json(&record).unwrap();
        assert!(
            saved.agents.is_empty(),
            "two marked sessions record nothing"
        );
        drop(second);
        rex.resume_with(zsh).await;
        let saved: Record = crate::peers::load_json(&record).unwrap();
        let expected = BTreeMap::from([(READY.to_owned(), agent("copilot", FIRST, "/work/app"))]);
        assert_eq!(saved.agents, expected);
    }

    #[tokio::test]
    async fn an_agy_is_recorded_from_the_one_presence_lock_its_process_has_open() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("rex-resume.json");
        let fake = Fake::start().await;
        {
            let mut s = fake.state();
            s.instance = "one";
            s.sessions.push((S.to_owned(), "work".to_owned()));
            let me = std::process::id();
            s.add(S, READY, fake::process(me, "agy", "agy"), Vec::new());
        }
        let roots = crate::costs::Roots {
            agy: Some(dir.path().join("agy/brain")),
            ..crate::costs::Roots::default()
        };
        let rex = Rex::new(fake.socket.clone())
            .with_agent_roots(roots)
            .with_resume(record.clone(), dir.path().join("rex-resume-off"));
        let rex = Arc::new(rex);
        let _task = follow(&rex).await;
        agy_transcript(dir.path(), FIRST);
        let own = presence_lock(dir.path(), FIRST);
        let other = presence_lock(dir.path(), SECOND);
        rex.reread_sessions();
        rex.resume_with(zsh).await;
        let saved: Record = crate::peers::load_json(&record).unwrap();
        assert!(saved.agents.is_empty(), "two presence locks record nothing");
        drop(other);
        rex.reread_sessions();
        rex.resume_with(zsh).await;
        let saved: Record = crate::peers::load_json(&record).unwrap();
        let expected = BTreeMap::from([(READY.to_owned(), agent("agy", FIRST, "/work/app"))]);
        assert_eq!(saved.agents, expected);
        drop(own);
    }

    #[tokio::test]
    async fn a_conversation_a_live_codex_copilot_or_agy_holds_waits_until_it_lets_go() {
        for kind in ["codex", "copilot", "agy"] {
            let dir = tempfile::tempdir().unwrap();
            let folder = std::fs::canonicalize(dir.path()).unwrap();
            let folder = folder.to_str().unwrap();
            let id = v7(2);
            let r = restarted(
                dir.path(),
                &[(READY, agent(kind, &id, folder))],
                &[(READY, login_shell(), PROMPT)],
                &[],
            )
            .await;
            let hold = match kind {
                "codex" => {
                    rollout(dir.path(), &id, folder);
                    let lock = writer_lock(dir.path(), &id);
                    lock.lock().unwrap();
                    lock
                }
                "copilot" => copilot_marks(&copilot_session(dir.path(), &id, folder)),
                _ => {
                    agy_transcript(dir.path(), &id);
                    let lock = presence_lock(dir.path(), &id);
                    lock.lock().unwrap();
                    lock
                }
            };
            let rex = with_roots(&r, dir.path());
            let _task = follow(&rex).await;
            for _ in 0..4 {
                rex.resume_with(zsh).await;
                tokio::time::sleep(SETTLE).await;
            }
            assert!(
                anchors(&r.fake).is_empty(),
                "{kind}: held by a live process"
            );
            drop(hold);
            assert!(ticks(&rex, || !anchors(&r.fake).is_empty()).await, "{kind}");
        }
    }

    #[tokio::test]
    async fn no_kind_starts_a_conversation_without_its_transcript() {
        for kind in ["codex", "copilot", "agy"] {
            let dir = tempfile::tempdir().unwrap();
            let folder = std::fs::canonicalize(dir.path()).unwrap();
            let r = restarted(
                dir.path(),
                &[(READY, agent(kind, &v7(3), folder.to_str().unwrap()))],
                &[(READY, login_shell(), PROMPT)],
                &[],
            )
            .await;
            let rex = with_roots(&r, dir.path());
            let _task = follow(&rex).await;
            for _ in 0..4 {
                rex.resume_with(zsh).await;
                tokio::time::sleep(SETTLE).await;
            }
            assert!(anchors(&r.fake).is_empty(), "{kind}");
            assert!(
                crate::lock(&rex.resume).pending.is_empty(),
                "{kind}: dropped"
            );
        }
    }

    #[tokio::test]
    async fn an_account_whose_login_shell_is_not_zsh_or_bash_gets_no_agent() {
        let dir = tempfile::tempdir().unwrap();
        let r = started_ready(dir.path()).await;
        let _task = follow(&r.rex).await;
        for _ in 0..4 {
            r.rex.resume_with(|| None).await;
            tokio::time::sleep(SETTLE).await;
        }
        assert!(anchors(&r.fake).is_empty());
        assert!(crate::lock(&r.rex.resume).pending.is_empty(), "dropped");
    }

    #[test]
    fn what_collied_writes_it_can_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rex-resume.json");
        let mut r = Resume::load(path.clone(), dir.path().join("rex-resume-off"));
        r.plan(Some("one"), None, 0, Instant::now());
        let block = |n: usize| format!("block:019a7d2e-0000-7000-8000-{n:012x}");
        let folder = format!("/{}", "x".repeat(MAX_FOLDER - 1));
        let current = (0..MAX_AGENTS)
            .map(|n| (block(n), agent("claude", FIRST, &folder)))
            .collect();
        r.observe(Some("one"), &current, Instant::now());
        assert!(std::fs::metadata(&path).unwrap().len() <= MAX_FILE);
        let back = read(&path);
        assert!(!back.agents.is_empty());
        assert_eq!(back.agents, r.record.agents);
        let written = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        r.observe(Some("one"), &current, Instant::now());
        let again = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            written, again,
            "a trimmed record is not written again on every tick"
        );
    }

    #[test]
    fn the_record_is_read_at_most_to_its_limit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big");
        std::fs::write(&path, vec![b'x'; 1 << 20]).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (bytes, _) = crate::peers::read_private_at_most(&path, 100).unwrap();
        assert_eq!(bytes.len(), 100);
    }

    #[test]
    fn a_malformed_record_is_logged_without_its_contents() {
        let logs = super::super::tests::Logs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let _logging = tracing::subscriber::set_default(subscriber);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rex-resume.json");
        let bad = serde_json::json!({ "instance": "one", "agents": "/private/secret-folder" });
        crate::peers::save_json(&path, &bad).unwrap();
        assert_eq!(read(&path), Record::default());
        let text = String::from_utf8_lossy(&crate::lock(&logs.0)).into_owned();
        assert!(text.contains("malformed"), "{text}");
        assert!(!text.contains("secret-folder"), "{text}");
    }
}
