use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use protocol::{ScheduleId, ScheduledTask, TaskNewParams, TerminalId};
use serde::{Deserialize, Serialize};

use crate::peers;

pub const MAX_TASKS: usize = 10;
pub const MAX_AHEAD_MS: u64 = 24 * 60 * 60 * 1000;
pub const LATE_MS: u64 = 60 * 60 * 1000;
const MAX_FILE: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub by: String,
    pub start_at_ms: u64,
    pub task: TaskNewParams,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub starting: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<TerminalId>,
}

impl Entry {
    pub fn new(by: String, start_at_ms: u64, task: TaskNewParams) -> Self {
        Self {
            by,
            start_at_ms,
            task,
            starting: false,
            blocked: None,
        }
    }

    fn waiting(&self) -> bool {
        !self.starting && self.blocked.is_none()
    }

    fn same(&self, other: &Entry) -> bool {
        let mut task = other.task.clone();
        if self.blocked.is_some() {
            task.prompt = None;
        }
        self.by == other.by && self.start_at_ms == other.start_at_ms && self.task == task
    }
}

#[derive(Debug, PartialEq)]
pub enum Due {
    Start(ScheduleId, Entry),
    Late(ScheduleId, Entry),
}

#[derive(Debug, PartialEq)]
pub enum Cancel {
    Done,
    Missing,
    Starting,
    Unsaved,
}

#[derive(Default)]
pub struct Schedule {
    tasks: BTreeMap<ScheduleId, Entry>,
    path: Option<PathBuf>,
}

impl Schedule {
    pub fn load(path: PathBuf) -> Self {
        Self {
            tasks: read(&path),
            path: Some(path),
        }
    }

    fn save(&self) -> bool {
        let Some(path) = self.path.as_deref() else {
            return true;
        };
        match peers::save_json(path, &self.tasks) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, "could not save the scheduled tasks");
                false
            }
        }
    }

    pub fn add(&mut self, entry: Entry, now_ms: u64) -> Result<ScheduledTask, &'static str> {
        let sent =
            (self.tasks.iter()).find(|(_, e)| e.by == entry.by && e.task.op_id == entry.task.op_id);
        if let Some((id, e)) = sent {
            return if e.same(&entry) {
                Ok(shown(id, e))
            } else {
                Err("this request already scheduled another task")
            };
        }
        if entry.start_at_ms <= now_ms {
            return Err("the start time has passed");
        }
        if entry.start_at_ms > now_ms + MAX_AHEAD_MS {
            return Err("a task can start at most 24 hours from now");
        }
        if self.tasks.len() >= MAX_TASKS {
            return Err("10 tasks already wait on this computer");
        }
        let id = new_id()?;
        let shown = shown(&id, &entry);
        self.tasks.insert(id.clone(), entry);
        if !self.save() {
            self.tasks.remove(&id);
            return Err("could not save the task");
        }
        Ok(shown)
    }

    pub fn cancel(&mut self, id: &ScheduleId) -> Cancel {
        match self.tasks.get(id) {
            None => return Cancel::Missing,
            Some(e) if e.starting => return Cancel::Starting,
            Some(_) => {}
        }
        let entry = self.tasks.remove(id);
        if !self.save() {
            self.tasks.extend(entry.map(|e| (id.clone(), e)));
            return Cancel::Unsaved;
        }
        Cancel::Done
    }

    pub fn list(&self) -> Vec<ScheduledTask> {
        let mut tasks: Vec<ScheduledTask> = self.tasks.iter().map(|(id, e)| shown(id, e)).collect();
        tasks.sort_by_key(|t| t.start_at_ms);
        tasks
    }

    pub fn retain_paired(&mut self, paired: impl Fn(&str) -> bool) -> bool {
        let before = self.tasks.len();
        self.tasks.retain(|_, e| paired(&e.by));
        let ended = self.tasks.len() != before;
        if ended {
            self.save();
        }
        ended
    }

    pub fn forget(&mut self, by: &str) -> bool {
        self.retain_paired(|b| b != by)
    }

    pub fn interrupted(&mut self) -> Vec<(ScheduleId, Entry)> {
        let ids: Vec<ScheduleId> = (self.tasks.iter())
            .filter(|(_, e)| e.starting)
            .map(|(id, _)| id.clone())
            .collect();
        let taken: Vec<(ScheduleId, Entry)> = (ids.into_iter())
            .filter_map(|id| self.tasks.remove(&id).map(|e| (id, e)))
            .collect();
        if !taken.is_empty() {
            self.save();
        }
        taken
    }

    pub fn expire(&mut self, now_ms: u64) {
        let before = self.tasks.len();
        self.tasks
            .retain(|_, e| e.blocked.is_none() || e.start_at_ms + MAX_AHEAD_MS >= now_ms);
        if self.tasks.len() != before {
            self.save();
        }
    }

    pub fn claim(&mut self, now_ms: u64) -> Option<Due> {
        let id = (self.tasks.iter())
            .filter(|(_, e)| e.waiting() && e.start_at_ms <= now_ms)
            .min_by_key(|(_, e)| e.start_at_ms)
            .map(|(id, _)| id.clone())?;
        let late = now_ms - self.tasks[&id].start_at_ms > LATE_MS;
        if late {
            let entry = self.tasks.remove(&id)?;
            if !self.save() {
                self.tasks.insert(id, entry);
                return None;
            }
            return Some(Due::Late(id, entry));
        }
        self.tasks.get_mut(&id)?.starting = true;
        if !self.save() {
            self.tasks.get_mut(&id)?.starting = false;
            return None;
        }
        Some(Due::Start(id.clone(), self.tasks[&id].clone()))
    }

    pub fn finish(&mut self, id: &ScheduleId) {
        if self.tasks.remove(id).is_some() {
            self.save();
        }
    }

    pub fn block(&mut self, id: &ScheduleId, terminal_id: TerminalId) {
        let Some(entry) = self.tasks.get_mut(id) else {
            return;
        };
        entry.task.prompt = None;
        entry.starting = false;
        entry.blocked = Some(terminal_id);
        self.save();
    }
}

fn shown(id: &ScheduleId, e: &Entry) -> ScheduledTask {
    ScheduledTask {
        schedule_id: id.clone(),
        start_at_ms: e.start_at_ms,
        cwd: e.task.cwd.clone(),
        agent: e.task.agent.clone(),
        label: e.task.label.clone(),
        blocked: e.blocked.clone(),
    }
}

fn new_id() -> Result<ScheduleId, &'static str> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "no randomness")?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    ScheduleId::new(hex).map_err(|_| "invalid schedule id")
}

fn read(path: &Path) -> BTreeMap<ScheduleId, Entry> {
    let bytes = match peers::read_private_at_most(path, MAX_FILE + 1) {
        Ok((bytes, _)) => bytes,
        Err(peers::Error::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
            return BTreeMap::new();
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not read the scheduled tasks");
            return BTreeMap::new();
        }
    };
    if bytes.len() as u64 > MAX_FILE {
        tracing::warn!("the scheduled tasks are too large to read");
        return BTreeMap::new();
    }
    match serde_json::from_slice::<BTreeMap<ScheduleId, Entry>>(&bytes) {
        Ok(tasks) if tasks.len() <= MAX_TASKS => tasks,
        Ok(_) => {
            tracing::warn!("the scheduled tasks hold more than the limit; not read");
            BTreeMap::new()
        }
        Err(e) => {
            let category = e.classify();
            tracing::warn!(?category, "the scheduled tasks are malformed");
            BTreeMap::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use protocol::{AgentKind, Cwd, OpId, PromptText, limits};

    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn entry(by: &str, start_at_ms: u64, prompt: &str) -> Entry {
        Entry::new(
            by.into(),
            start_at_ms,
            TaskNewParams {
                op_id: OpId::new(format!("{start_at_ms:A>22}")).unwrap(),
                cwd: Cwd::new("/a").unwrap(),
                agent: AgentKind::new("claude").unwrap(),
                prompt: Some(PromptText::new(prompt).unwrap()),
                label: None,
                new_folder: None,
                worktree: None,
                clone: None,
                resume: None,
                ultracode: false,
                code_mode: None,
                runtime: None,
            },
        )
    }

    fn fresh(e: Entry, op: &str) -> Entry {
        let mut e = e;
        e.task.op_id = OpId::new(op.repeat(22)).unwrap();
        e
    }

    fn store() -> (tempfile::TempDir, PathBuf, Schedule) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scheduled-tasks.json");
        let schedule = Schedule::load(path.clone());
        (dir, path, schedule)
    }

    #[test]
    fn a_full_store_of_the_longest_prompts_survives_a_restart_and_takes_no_more() {
        let (_dir, path, mut schedule) = store();
        let longest = "\"".repeat(limits::MAX_PROMPT_BYTES);
        for op in "ABCDEFGHIJ".chars() {
            let e = fresh(entry("nA", NOW + 60_000, &longest), &op.to_string());
            schedule.add(e, NOW).unwrap();
        }
        assert_eq!(
            schedule.add(fresh(entry("nA", NOW + 60_000, "go"), "K"), NOW),
            Err("10 tasks already wait on this computer")
        );
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let reloaded = Schedule::load(path);
        assert_eq!(reloaded.list(), schedule.list());
        assert_eq!(reloaded.list().len(), MAX_TASKS);
    }

    #[test]
    fn a_resent_request_returns_its_task_and_never_adds_another() {
        let (_dir, path, mut schedule) = store();
        let first = schedule.add(entry("nA", NOW + 1000, "go"), NOW).unwrap();
        let mut restarted = Schedule::load(path);
        assert_eq!(
            restarted.add(entry("nA", NOW + 1000, "go"), NOW + 2000),
            Ok(first.clone()),
            "a retry after a restart, past the start time too"
        );
        let mut other = entry("nA", NOW + 1000, "else");
        other.task.op_id = entry("nA", NOW + 1000, "go").task.op_id;
        assert_eq!(
            restarted.add(other, NOW),
            Err("this request already scheduled another task")
        );
        let Some(Due::Start(id, _)) = restarted.claim(NOW + 1000) else {
            panic!("not claimed");
        };
        restarted.block(&id, TerminalId::new("t1").unwrap());
        assert_eq!(
            restarted
                .add(entry("nA", NOW + 1000, "go"), NOW)
                .map(|t| t.schedule_id),
            Ok(first.schedule_id),
            "a blocked record keeps no prompt but still answers the retry"
        );
        assert_eq!(restarted.list().len(), 1);
    }

    #[test]
    fn a_start_time_is_in_the_next_24_hours() {
        let (_dir, _path, mut schedule) = store();
        for (op, at, refused) in [
            ("A", NOW, Some("the start time has passed")),
            (
                "B",
                NOW + MAX_AHEAD_MS + 1,
                Some("a task can start at most 24 hours from now"),
            ),
            ("C", NOW + MAX_AHEAD_MS, None),
            ("D", NOW + 1, None),
        ] {
            let added = schedule.add(fresh(entry("nA", at, "go"), op), NOW);
            assert_eq!(added.err(), refused, "{at}");
        }
    }

    #[test]
    fn each_task_is_claimed_alone_and_judged_late_when_claimed() {
        let (_dir, path, mut schedule) = store();
        let first = schedule.add(entry("nA", NOW + 1000, "first"), NOW).unwrap();
        let second = schedule
            .add(entry("nA", NOW + 2000, "second"), NOW)
            .unwrap();
        assert_eq!(schedule.claim(NOW + 999), None);
        let Some(Due::Start(id, _)) = schedule.claim(NOW + 1000 + LATE_MS) else {
            panic!("an hour late still starts");
        };
        assert_eq!(id, first.schedule_id);
        assert_eq!(schedule.list().len(), 2, "listed while it starts");
        assert_eq!(schedule.cancel(&id), Cancel::Starting);
        let reloaded = Schedule::load(path.clone()).list();
        assert_eq!(reloaded.len(), 2, "the next one is still there for a crash");
        schedule.finish(&id);
        let Some(Due::Late(late, _)) = schedule.claim(NOW + 2001 + LATE_MS) else {
            panic!("judged again when its own turn comes");
        };
        assert_eq!(late, second.schedule_id);
        assert_eq!(schedule.claim(NOW + 3 * LATE_MS), None);
        assert!(Schedule::load(path).list().is_empty());
    }

    #[test]
    fn a_start_cut_short_by_a_restart_is_reported_and_never_started_again() {
        let (_dir, path, mut schedule) = store();
        let added = schedule.add(entry("nA", NOW + 1000, "go"), NOW).unwrap();
        let waiting = schedule.add(entry("nA", NOW + 5000, "later"), NOW).unwrap();
        assert!(matches!(schedule.claim(NOW + 1000), Some(Due::Start(..))));
        let mut restarted = Schedule::load(path);
        let cut = restarted.interrupted();
        assert_eq!(cut.len(), 1);
        assert_eq!(cut[0].0, added.schedule_id);
        assert_eq!(restarted.list(), vec![waiting]);
        assert!(matches!(restarted.claim(NOW + 5000), Some(Due::Start(..))));
        assert_eq!(restarted.claim(NOW + 5000), None);
    }

    #[test]
    fn a_cancelled_task_stays_cancelled_after_a_restart() {
        let (_dir, path, mut schedule) = store();
        let kept = schedule.add(entry("nA", NOW + 1000, "go"), NOW).unwrap();
        let cancelled = schedule.add(entry("nA", NOW + 2000, "go"), NOW).unwrap();
        assert_eq!(schedule.cancel(&cancelled.schedule_id), Cancel::Done);
        assert_eq!(schedule.cancel(&cancelled.schedule_id), Cancel::Missing);
        assert_eq!(Schedule::load(path).list(), vec![kept]);
    }

    #[test]
    fn a_revoked_phone_leaves_no_task_behind() {
        let (_dir, path, mut schedule) = store();
        schedule.add(entry("nA", NOW + 1000, "go"), NOW).unwrap();
        let kept = schedule.add(entry("nB", NOW + 1000, "go"), NOW).unwrap();
        assert!(schedule.forget("nA"));
        assert!(!schedule.retain_paired(|by| by == "nB"));
        assert_eq!(Schedule::load(path.clone()).list(), vec![kept]);
        assert!(schedule.retain_paired(|_| false), "a phone revoked offline");
        assert!(Schedule::load(path).list().is_empty());
    }

    #[test]
    fn a_blocked_task_keeps_no_prompt_its_slot_and_ends_a_day_later() {
        let (_dir, path, mut schedule) = store();
        let added = schedule
            .add(entry("nA", NOW + 1000, "secret plan"), NOW)
            .unwrap();
        for op in "ABCDEFGHI".chars() {
            let e = fresh(entry("nA", NOW + 60_000, "go"), &op.to_string());
            schedule.add(e, NOW).unwrap();
        }
        let Some(Due::Start(id, _)) = schedule.claim(NOW + 1000) else {
            panic!("not started");
        };
        assert_eq!(
            schedule.add(fresh(entry("nA", NOW + 60_000, "go"), "K"), NOW),
            Err("10 tasks already wait on this computer"),
            "a starting task keeps its slot"
        );
        let terminal = TerminalId::new("t1").unwrap();
        schedule.block(&id, terminal.clone());
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("secret plan")
        );
        let listed = Schedule::load(path.clone()).list();
        assert_eq!(listed.len(), MAX_TASKS);
        assert_eq!(listed[0].blocked, Some(terminal));
        assert_eq!(listed[0].schedule_id, added.schedule_id);
        assert_eq!(schedule.cancel(&id), Cancel::Done, "a phone clears it");
        schedule
            .add(fresh(entry("nA", NOW + 60_000, "go"), "K"), NOW)
            .unwrap();
        let Some(Due::Start(id, _)) = schedule.claim(NOW + 60_000) else {
            panic!("not started");
        };
        schedule.block(&id, TerminalId::new("t2").unwrap());
        schedule.expire(NOW + 60_000 + MAX_AHEAD_MS);
        assert_eq!(schedule.list().len(), MAX_TASKS);
        schedule.expire(NOW + 60_001 + MAX_AHEAD_MS);
        assert_eq!(Schedule::load(path).list().len(), MAX_TASKS - 1);
    }
}
