use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use protocol::{
    Agent, AgentStatus, Flock, MachineInfo, Terminal, TerminalId, Workspace, WorkspaceId,
};

use crate::herdr::{AgentInfo, PaneInfo, SessionSnapshot, WorkspaceInfo};
use crate::transcript::{Derived, Transcripts};

/// herdr reports no timestamp for a status, so collied records when it first saw each one,
/// and keeps it on disk so a restart does not reset it.
#[derive(Default)]
pub struct StatusTracker {
    seen: HashMap<String, (AgentStatus, u64, u64)>,
    path: Option<PathBuf>,
    dirty: bool,
}

impl StatusTracker {
    pub fn load(path: PathBuf) -> Self {
        let seen = crate::peers::load_json(&path).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the saved status times");
            HashMap::new()
        });
        Self {
            seen,
            path: Some(path),
            dirty: false,
        }
    }

    pub fn save(&mut self) {
        let Some(path) = self.path.as_deref().filter(|_| self.dirty) else {
            return;
        };
        if let Err(e) = crate::peers::save_json(path, &self.seen) {
            tracing::warn!(error = %e, "could not save the status times");
        }
        self.dirty = false;
    }

    fn observe(&mut self, terminal_id: &str, status: AgentStatus, seq: u64, now_ms: u64) -> u64 {
        match self.seen.get(terminal_id) {
            Some(&(s, q, since)) if s == status && q == seq && since <= now_ms => since,
            _ => {
                self.seen
                    .insert(terminal_id.to_owned(), (status, seq, now_ms));
                self.dirty = true;
                now_ms
            }
        }
    }

    pub fn retain<'a>(&mut self, live: impl Iterator<Item = &'a str>) {
        let live: std::collections::HashSet<&str> = live.collect();
        let before = self.seen.len();
        self.seen.retain(|id, _| live.contains(id.as_str()));
        self.dirty |= self.seen.len() != before;
    }
}

pub fn status(s: &str) -> AgentStatus {
    serde_json::from_value(serde_json::Value::String(s.to_owned())).unwrap_or(AgentStatus::Unknown)
}

fn non_empty(s: &Option<String>) -> Option<String> {
    s.as_deref().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn title(a: &AgentInfo) -> Option<String> {
    non_empty(&a.terminal_title_stripped).or_else(|| non_empty(&a.title))
}

fn cwd(a: &AgentInfo) -> Option<String> {
    non_empty(&a.foreground_cwd).or_else(|| non_empty(&a.cwd))
}

/// `pane_id` is deliberately dropped: it changes on moves and is never sent to the phone.
pub fn map_agent(
    a: &AgentInfo,
    tracker: &mut StatusTracker,
    now_ms: u64,
    derived: Option<Derived>,
) -> Option<Agent> {
    let (Ok(terminal_id), Ok(workspace_id)) = (
        TerminalId::new(a.terminal_id.clone()),
        WorkspaceId::new(a.workspace_id.clone()),
    ) else {
        tracing::warn!(terminal_id = %a.terminal_id, "skipping agent with an invalid id");
        return None;
    };
    let status = status(&a.agent_status);
    let derived = derived.unwrap_or_default();
    Some(Agent {
        status_since_ms: tracker.observe(&a.terminal_id, status, a.state_change_seq, now_ms),
        terminal_id,
        workspace_id,
        kind: non_empty(&a.agent),
        name: non_empty(&a.name),
        title: title(a),
        status,
        cwd: cwd(a),
        last_line: derived.last_line,
        context_left: derived.context_left,
        last_prompt: derived.last_prompt,
        last_activity_ms: derived.last_activity_ms,
    })
}

pub fn map_agents(
    agents: &[AgentInfo],
    tracker: &mut StatusTracker,
    now_ms: u64,
    mut transcripts: Option<&mut Transcripts>,
) -> Vec<Agent> {
    tracker.retain(agents.iter().map(|a| a.terminal_id.as_str()));
    if let Some(t) = transcripts.as_deref_mut() {
        t.retain(agents);
    }
    agents
        .iter()
        .filter_map(|a| {
            let derived = transcripts.as_deref_mut().and_then(|t| t.derive(a, false));
            map_agent(a, tracker, now_ms, derived)
        })
        .collect()
}

pub fn map_workspace(w: &WorkspaceInfo, panes: &[PaneInfo]) -> Option<Workspace> {
    let workspace_id = WorkspaceId::new(w.workspace_id.clone()).ok()?;
    let cwd = panes
        .iter()
        .find(|p| p.workspace_id == w.workspace_id && Some(&p.tab_id) == w.active_tab_id.as_ref())
        .and_then(|p| non_empty(&p.cwd));
    Some(Workspace {
        workspace_id,
        label: w.label.clone(),
        number: w.number,
        status: status(&w.agent_status),
        cwd,
    })
}

pub fn map_workspaces(snap: &SessionSnapshot) -> Vec<Workspace> {
    snap.workspaces
        .iter()
        .filter_map(|w| map_workspace(w, &snap.panes))
        .collect()
}

/// Panes with no agent, launching ones included. The label is the name given in herdr: a
/// title the pane's program sets (a shell theme puts the running command there) never
/// leaves the machine.
pub fn map_terminals(snap: &SessionSnapshot) -> Vec<Terminal> {
    snap.panes
        .iter()
        .filter(|p| {
            p.agent.is_none() && !snap.agents.iter().any(|a| a.terminal_id == p.terminal_id)
        })
        .filter_map(|p| {
            Some(Terminal {
                terminal_id: TerminalId::new(p.terminal_id.clone()).ok()?,
                workspace_id: WorkspaceId::new(p.workspace_id.clone()).ok()?,
                label: non_empty(&p.label),
                cwd: non_empty(&p.foreground_cwd).or_else(|| non_empty(&p.cwd)),
            })
        })
        .collect()
}

pub fn map_flock(
    snap: &SessionSnapshot,
    tracker: &mut StatusTracker,
    now_ms: u64,
    machine: MachineInfo,
    seq: u64,
    mut transcripts: Option<&mut Transcripts>,
) -> Flock {
    Flock {
        seq,
        machine,
        workspaces: map_workspaces(snap),
        plan_usage: transcripts.as_deref_mut().and_then(Transcripts::plan),
        agents: map_agents(&snap.agents, tracker, now_ms, transcripts),
        approvals: Vec::new(),
        terminals: Vec::new(),
        terminals_enabled: false,
    }
}

/// What the phone shows of an agent, so that a change to any of it sends `agent.status`.
type Shown = (u64, AgentStatus, [Option<String>; 4]);
/// Context left, last line and last prompt. The activity time is left out: it moves on
/// every transcript write, and the events and snapshots that are sent carry it anyway.
type Said = Option<(Option<u8>, Option<String>, Option<String>)>;

#[derive(Debug, Default, PartialEq)]
pub struct Baseline {
    agents: BTreeMap<String, (Shown, Said, String)>,
    workspaces: Vec<(String, String, u32)>,
    /// `Said` is compared only between two baselines that read transcripts, so a phone
    /// connecting or leaving sends nothing.
    read: bool,
}

impl Baseline {
    /// Without `transcripts` the transcript values are not compared, and no transcript is read.
    pub fn new(
        agents: &[AgentInfo],
        workspaces: &[WorkspaceInfo],
        mut transcripts: Option<&mut Transcripts>,
    ) -> Self {
        let read = transcripts.is_some();
        Self {
            agents: agents
                .iter()
                .map(|a| {
                    let shown = (
                        a.state_change_seq,
                        status(&a.agent_status),
                        [non_empty(&a.agent), non_empty(&a.name), title(a), cwd(a)],
                    );
                    let said = transcripts
                        .as_deref_mut()
                        .and_then(|t| t.derive(a, false))
                        .map(|d| (d.context_left, d.last_line, d.last_prompt));
                    (a.terminal_id.clone(), (shown, said, a.workspace_id.clone()))
                })
                .collect(),
            workspaces: workspaces
                .iter()
                .map(|w| (w.workspace_id.clone(), w.label.clone(), w.number))
                .collect(),
            read,
        }
    }

    pub fn diff(&self, next: &Self) -> (Vec<String>, bool) {
        let both_read = self.read && next.read;
        let changed = next
            .agents
            .iter()
            .filter(|(id, (shown, said, _))| {
                self.agents
                    .get(*id)
                    .is_some_and(|(s, d, _)| s != shown || (both_read && d != said))
            })
            .map(|(id, _)| id.clone())
            .collect();
        let shape = |b: &Self| -> Vec<(String, String)> {
            b.agents
                .iter()
                .map(|(id, (_, _, ws))| (id.clone(), ws.clone()))
                .collect()
        };
        let flock_changed = shape(self) != shape(next) || self.workspaces != next.workspaces;
        (changed, flock_changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> SessionSnapshot {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/session.snapshot.json")).unwrap();
        serde_json::from_value(v["result"]["snapshot"].clone()).unwrap()
    }

    fn machine() -> MachineInfo {
        MachineInfo {
            name: "Mac".into(),
            node_id: "nMAC".into(),
            herdr_session: "default".into(),
        }
    }

    #[test]
    fn maps_session_snapshot() {
        let snap = fixture();
        let mut tracker = StatusTracker::default();
        let flock = map_flock(&snap, &mut tracker, 1000, machine(), 3, None);
        assert_eq!(flock.seq, 3);
        assert!(flock.approvals.is_empty());
        assert_eq!(flock.workspaces.len(), 2);
        let w = &flock.workspaces[0];
        assert_eq!(w.workspace_id.as_str(), "w6");
        assert_eq!((w.label.as_str(), w.number), ("collie", 1));
        assert_eq!(w.status, AgentStatus::Working);
        assert_eq!(w.cwd.as_deref(), Some("/Users/me/src/collie"));
        assert_eq!(flock.workspaces[1].status, AgentStatus::Blocked);

        assert_eq!(flock.agents.len(), 2);
        let a = &flock.agents[0];
        assert_eq!(a.terminal_id.as_str(), "term_65ce7ae4fd5731");
        assert_eq!(a.kind.as_deref(), Some("claude"));
        assert_eq!(a.name, None);
        assert_eq!(a.title.as_deref(), Some("Collie iOS remote control"));
        assert_eq!(a.status, AgentStatus::Working);
        assert_eq!(a.status_since_ms, 1000);
        assert_eq!(a.last_line, None);
        let b = &flock.agents[1];
        assert_eq!(b.name.as_deref(), Some("api-fixer"));
        assert_eq!(b.title.as_deref(), Some("Fix flaky test"));
        assert_eq!(b.cwd.as_deref(), Some("/Users/me/src/api/server"));
        assert_eq!(b.workspace_id.as_str(), "w7");

        let json = serde_json::to_string(&flock).unwrap();
        assert!(!json.contains("w6:p1") && !json.contains("pane_id"));
    }

    #[test]
    fn terminals_are_panes_without_an_agent() {
        let mut snap = fixture();
        let shells = map_terminals(&snap);
        assert_eq!(shells.len(), 1);
        let t = &shells[0];
        assert_eq!(t.terminal_id.as_str(), "term_ffffffffffff01");
        assert_eq!(t.workspace_id.as_str(), "w7");
        assert_eq!(
            (t.label.as_deref(), t.cwd.as_deref()),
            (None, Some("/Users/me/src/api"))
        );
        snap.panes[2].label = Some("logs".into());
        snap.panes[2].foreground_cwd = Some("/var/log".into());
        let t = &map_terminals(&snap)[0];
        assert_eq!(
            (t.label.as_deref(), t.cwd.as_deref()),
            (Some("logs"), Some("/var/log"))
        );

        // An agent herdr is still launching has no `agent` on its pane yet.
        let mut pending = snap.agents[0].clone();
        pending.terminal_id = snap.panes[2].terminal_id.clone();
        snap.agents.push(pending);
        assert!(map_terminals(&snap).is_empty());
        snap.agents.pop();
        snap.panes[2].agent = Some("claude".into());
        assert!(map_terminals(&snap).is_empty());

        let json = serde_json::to_string(&map_terminals(&fixture())).unwrap();
        assert!(
            !json.contains("pane_id") && !json.contains("title"),
            "{json}"
        );
    }

    #[test]
    fn agents_carry_what_their_transcript_derives() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("projects/-Users-me-src-collie");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("00000000-0000-4000-8000-000000000000.jsonl"),
            include_str!("../tests/fixtures/transcripts/claude.jsonl"),
        )
        .unwrap();
        let mut transcripts = Transcripts::new(Some(dir.path().join("projects")), None);
        let flock = map_flock(
            &fixture(),
            &mut StatusTracker::default(),
            0,
            machine(),
            0,
            Some(&mut transcripts),
        );
        let a = &flock.agents[0];
        assert_eq!(a.context_left, Some(94));
        assert_eq!(a.last_line.as_deref(), Some("**Fixed** the flaky test."));
        assert!(
            a.last_prompt
                .as_deref()
                .unwrap()
                .starts_with("fix the flaky")
        );
        assert!(a.last_activity_ms.is_some());
        let b = &flock.agents[1];
        assert_eq!(
            (b.context_left, &b.last_line, b.last_activity_ms),
            (None, &None, None)
        );
        let json = serde_json::to_string(b).unwrap();
        assert!(!json.contains("context_left") && !json.contains("last_prompt"));
    }

    #[test]
    fn status_since_moves_only_on_change() {
        let mut snap = fixture();
        let mut tracker = StatusTracker::default();
        map_flock(&snap, &mut tracker, 1000, machine(), 0, None);
        let again = map_flock(&snap, &mut tracker, 2000, machine(), 0, None);
        assert_eq!(again.agents[0].status_since_ms, 1000);
        snap.agents[0].agent_status = "idle".into();
        let changed = map_flock(&snap, &mut tracker, 3000, machine(), 0, None);
        assert_eq!(changed.agents[0].status_since_ms, 3000);
        assert_eq!(changed.agents[1].status_since_ms, 1000);
        snap.agents.remove(1);
        map_flock(&snap, &mut tracker, 4000, machine(), 0, None);
        assert_eq!(tracker.seen.len(), 1);
    }

    #[test]
    fn status_since_survives_a_restart() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status.json");
        let mut snap = fixture();
        let mut tracker = StatusTracker::load(path.clone());
        map_flock(&snap, &mut tracker, 1000, machine(), 0, None);
        tracker.save();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        let mut restarted = StatusTracker::load(path.clone());
        snap.agents[1].agent_status = "idle".into();
        let f = map_flock(&snap, &mut restarted, 5000, machine(), 0, None);
        assert_eq!(f.agents[0].status_since_ms, 1000);
        assert_eq!(f.agents[1].status_since_ms, 5000);
        restarted.save();

        let mut early = StatusTracker::load(path.clone());
        let f = map_flock(&snap, &mut early, 500, machine(), 0, None);
        assert_eq!(f.agents[0].status_since_ms, 500);
        assert_eq!(f.agents[1].status_since_ms, 500);
        early.save();

        let mut again = StatusTracker::load(path);
        snap.agents[0].state_change_seq += 1;
        let f = map_flock(&snap, &mut again, 6000, machine(), 0, None);
        assert_eq!(f.agents[0].status_since_ms, 6000);
        assert_eq!(f.agents[1].status_since_ms, 500);
    }

    #[test]
    fn unknown_status_and_bad_ids() {
        assert_eq!(status("sleeping"), AgentStatus::Unknown);
        let mut snap = fixture();
        snap.agents[0].terminal_id = "bad id".into();
        let flock = map_flock(&snap, &mut StatusTracker::default(), 0, machine(), 0, None);
        assert_eq!(flock.agents.len(), 1);
    }

    #[test]
    fn reconcile_diff() {
        let snap = fixture();
        let base = Baseline::new(&snap.agents, &snap.workspaces, None);
        assert_eq!(base.diff(&base), (vec![], false));

        let mut agents = snap.agents.clone();
        agents[1].state_change_seq += 1;
        let (changed, shape) = base.diff(&Baseline::new(&agents, &snap.workspaces, None));
        assert_eq!(changed, vec!["term_0a1b2c3d4e5f60".to_owned()]);
        assert!(!shape);

        let (changed, shape) = base.diff(&Baseline::new(&agents[..1], &snap.workspaces, None));
        assert!(changed.is_empty() && shape);

        let mut ws = snap.workspaces.clone();
        ws[0].label = "renamed".into();
        assert_eq!(
            base.diff(&Baseline::new(&snap.agents, &ws, None)),
            (vec![], true)
        );
    }

    #[test]
    fn what_the_phone_shows_changes_without_a_status_change() {
        let snap = fixture();
        let base = Baseline::new(&snap.agents, &snap.workspaces, None);
        let diff = |edit: fn(&mut AgentInfo)| {
            let mut agents = snap.agents.clone();
            agents.iter_mut().for_each(edit);
            base.diff(&Baseline::new(&agents, &snap.workspaces, None))
        };
        let both = vec![
            "term_0a1b2c3d4e5f60".to_owned(),
            "term_65ce7ae4fd5731".to_owned(),
        ];
        assert_eq!(
            diff(|a| a.terminal_title_stripped = Some("Renamed".into())),
            (both.clone(), false)
        );
        assert_eq!(
            diff(|a| a.name = Some("worker".into())),
            (both.clone(), false)
        );
        assert_eq!(
            diff(|a| a.agent = Some("copilot".into())),
            (both.clone(), false)
        );
        assert_eq!(
            diff(|a| a.foreground_cwd = Some("/tmp".into())),
            (both, false)
        );
        // The stripped title wins, so herdr's own title is not shown for the first agent.
        assert_eq!(
            diff(|a| a.title = Some("other".into())),
            (vec!["term_0a1b2c3d4e5f60".to_owned()], false)
        );
    }

    #[test]
    fn transcripts_are_compared_only_when_both_read_them() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("projects/-Users-me-src-collie");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join("00000000-0000-4000-8000-000000000000.jsonl");
        std::fs::write(
            &path,
            include_str!("../tests/fixtures/transcripts/claude.jsonl"),
        )
        .unwrap();
        let mut transcripts = Transcripts::new(Some(dir.path().join("projects")), None);
        let snap = fixture();
        let base = Baseline::new(&snap.agents, &snap.workspaces, Some(&mut transcripts));
        let same = Baseline::new(&snap.agents, &snap.workspaces, Some(&mut transcripts));
        assert_eq!(base.diff(&same), (vec![], false));
        let unread = Baseline::new(&snap.agents, &snap.workspaces, None);
        assert_eq!(unread.diff(&base), (vec![], false));
        assert_eq!(base.diff(&unread), (vec![], false));

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
            .unwrap();
        let touched = Baseline::new(&snap.agents, &snap.workspaces, Some(&mut transcripts));
        assert_eq!(base.diff(&touched), (vec![], false));

        std::io::Write::write_all(
            &mut file,
            br#"{"isSidechain":false,"type":"assistant","message":{"model":"claude-opus-4-7","role":"assistant","content":[{"type":"text","text":"Pushed the fix."}]}}
"#,
        )
        .unwrap();
        let next = Baseline::new(&snap.agents, &snap.workspaces, Some(&mut transcripts));
        assert_eq!(
            base.diff(&next),
            (vec!["term_65ce7ae4fd5731".to_owned()], false)
        );
        assert_eq!(
            unread.diff(&Baseline::new(&snap.agents, &snap.workspaces, None)),
            (vec![], false)
        );
    }
}
