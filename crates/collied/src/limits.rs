use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use protocol::{
    AgentPromptParams, AgentStatus, LimitWindow, OpId, PlanLimit, PlanUsage, PromptText,
    TerminalId, UsageWindow,
};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::drive::{Authorized, Driver};
use crate::runtime::AgentInfo;
use crate::runtime::herdr_core::owns;
use crate::{approvals, flock, peers};

pub const GRACE: Duration = Duration::from_secs(60);
/// A reading older than this stops nothing: a stop needs a limit reached now.
const FRESH_MS: u64 = 15 * 60_000;
/// A stop this soon after a resume is the same limit hit again.
const REHIT_MS: u64 = 15 * 60_000;
/// A resume missed by more than this (the machine slept, collied was off) is dropped.
pub const OVERDUE_MS: u64 = 3 * 3_600_000;
/// collied types it with no phone action behind it, so it is fixed: never text from a
/// transcript, a screen or the network.
pub const CONTINUE: &str = "Continue.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    pub session_id: String,
    pub title: String,
    pub window: LimitWindow,
    pub resets_at_ms: u64,
    pub stopped_ms: u64,
    #[serde(default)]
    pub resume_at_ms: Option<u64>,
    /// The agent was seen idle after the stop: working again means someone prompted it.
    #[serde(default)]
    idle_seen: bool,
    /// Prompted by hand while limited: it runs on extra usage until the next limit.
    #[serde(default)]
    continued: bool,
    #[serde(default)]
    resumed_ms: Option<u64>,
    #[serde(default)]
    rehits: u8,
}

impl Stop {
    fn held(&self) -> bool {
        !self.continued && self.resumed_ms.is_none()
    }
}

/// For the desktop bars.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limited {
    pub title: String,
    pub resets_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_at_ms: Option<u64>,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    enabled: bool,
    /// By terminal id.
    #[serde(default)]
    stops: BTreeMap<String, Stop>,
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Go,
    Hold(LimitWindow),
    Stopped(String, Stop),
    Continued(String),
}

#[derive(Debug, PartialEq)]
pub enum Change {
    /// Prompted by hand after the stop.
    Prompted(String),
    Gone(String),
}

/// Agents collied stopped at the plan limit, kept on disk so an armed resume survives a
/// restart, and the switch that turns stopping on.
pub struct Limits {
    saved: Saved,
    path: Option<PathBuf>,
    grace_ms: u64,
    started_ms: u64,
    /// Claude Code session id to its terminal and title, from the last poll.
    sessions: HashMap<String, (String, String)>,
    shown: watch::Sender<Vec<Limited>>,
}

/// The latest reset among the windows at their limit, from a reading taken while collied
/// ran and recently; else none, so a stale `usage.json` stops nothing.
pub fn reached(
    plan: Option<&PlanUsage>,
    started_ms: u64,
    now_ms: u64,
) -> Option<(LimitWindow, u64)> {
    let plan = plan.filter(|p| p.recorded_ms >= started_ms && now_ms < p.recorded_ms + FRESH_MS)?;
    exhausted(plan, now_ms)
}

/// Within a window usage only grows, so a window at its limit stays there until its reset,
/// however old the reading.
fn exhausted(plan: &PlanUsage, now_ms: u64) -> Option<(LimitWindow, u64)> {
    let at = |w: &Option<UsageWindow>| {
        w.as_ref()
            .filter(|w| w.used_percent >= 100 && w.resets_at_ms > now_ms)
            .map(|w| w.resets_at_ms)
    };
    [
        (LimitWindow::FiveHour, at(&plan.five_hour)),
        (LimitWindow::SevenDay, at(&plan.seven_day)),
    ]
    .into_iter()
    .filter_map(|(w, at)| Some((w, at?)))
    .max_by_key(|(_, at)| *at)
}

impl Limits {
    pub fn load(path: PathBuf, grace: Duration, started_ms: u64) -> Self {
        let saved = peers::load_json(&path).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the plan limit stops");
            Saved::default()
        });
        let mut limits = Self {
            saved,
            path: Some(path),
            grace_ms: grace.as_millis() as u64,
            started_ms,
            sessions: HashMap::new(),
            shown: watch::channel(Vec::new()).0,
        };
        limits.publish();
        limits
    }

    fn save(&mut self) {
        if let Some(path) = self.path.as_deref()
            && let Err(e) = peers::replace_json(path, &self.saved)
        {
            tracing::warn!(error = %e, "could not save the plan limit stops");
        }
        self.publish();
    }

    fn publish(&mut self) {
        let now = crate::now_ms();
        let limited: Vec<Limited> = self
            .saved
            .stops
            .values()
            .filter(|s| s.held() && (s.resume_at_ms.is_some() || s.resets_at_ms > now))
            .map(|s| Limited {
                title: s.title.clone(),
                resets_at_ms: s.resets_at_ms,
                resume_at_ms: s.resume_at_ms,
            })
            .collect();
        self.shown.send_if_modified(|shown| {
            let changed = *shown != limited;
            *shown = limited;
            changed
        });
    }

    pub fn watch(&self) -> watch::Receiver<Vec<Limited>> {
        self.shown.subscribe()
    }

    pub fn enabled(&self) -> bool {
        self.saved.enabled
    }

    /// Turning it off also drops every stop, and with them every armed resume.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.saved.enabled = enabled;
        if !enabled {
            self.saved.stops.clear();
        }
        self.save();
    }

    /// The PreToolUse hook's question. Only a session of an agent collied lists is stopped,
    /// since only those can be resumed; anything unknown goes on.
    pub fn check(
        &mut self,
        session_id: &str,
        subagent: bool,
        plan: Option<&PlanUsage>,
        now_ms: u64,
    ) -> Verdict {
        if !self.saved.enabled {
            return Verdict::Go;
        }
        let Some((terminal, title)) = self.sessions.get(session_id).cloned() else {
            return Verdict::Go;
        };
        let Some((window, resets_at_ms)) = reached(plan, self.started_ms, now_ms) else {
            return Verdict::Go;
        };
        let same = |s: &&mut Stop| {
            s.session_id == session_id && s.resets_at_ms == resets_at_ms && s.resumed_ms.is_none()
        };
        if let Some(s) = self.saved.stops.get_mut(&terminal).filter(same) {
            if s.continued {
                return Verdict::Go;
            }
            // Calls of the stopped turn and its subagents keep coming until it is idle.
            if subagent || !s.idle_seen {
                return Verdict::Hold(s.window);
            }
            s.continued = true;
            s.resume_at_ms = None;
            self.save();
            return Verdict::Continued(terminal);
        }
        let rehits = self
            .saved
            .stops
            .get(&terminal)
            .filter(|s| s.session_id == session_id)
            .and_then(|s| s.resumed_ms.map(|at| (s, at)))
            .filter(|(_, at)| now_ms < at + REHIT_MS)
            .map_or(0, |(s, _)| s.rehits.saturating_add(1));
        let stop = Stop {
            session_id: session_id.to_owned(),
            title,
            window,
            resets_at_ms,
            stopped_ms: now_ms,
            // Re-armed once after a resume that hit the limit again, then given up.
            resume_at_ms: (rehits <= 1).then_some(resets_at_ms + self.grace_ms),
            idle_seen: false,
            continued: false,
            resumed_ms: None,
            rehits,
        };
        self.saved.stops.insert(terminal.clone(), stop.clone());
        self.save();
        Verdict::Stopped(terminal, stop)
    }

    /// Follows the polled agents: maps sessions to terminals, notices a stopped agent going
    /// idle and then working again, and drops stops whose agent is gone or whose limit has
    /// long passed. A runtime that lists no agent is not trusted, as for pull request watches.
    pub fn observe(&mut self, agents: &[AgentInfo], now_ms: u64) -> Vec<Change> {
        self.sessions = agents
            .iter()
            .filter(|a| a.agent.as_deref() == Some("claude"))
            .filter_map(|a| {
                let session = a.agent_session.as_ref()?.value.clone();
                Some((session, (a.terminal_id.clone(), approvals::alert_title(a))))
            })
            .collect();
        let mut changes = Vec::new();
        let mut dirty = false;
        let terminals: Vec<String> = self.saved.stops.keys().cloned().collect();
        for terminal in terminals {
            let core = owns(&terminal);
            let listed = agents.iter().any(|a| owns(&a.terminal_id) == core);
            let agent = agents.iter().find(|a| a.terminal_id == terminal);
            let stop = &self.saved.stops[&terminal];
            // A session not reported yet is not another one.
            let gone = match agent {
                None => listed,
                Some(a) => {
                    a.agent.as_deref().is_some_and(|k| k != "claude")
                        || a.agent_session
                            .as_ref()
                            .is_some_and(|s| s.value != stop.session_id)
                }
            };
            let last = stop.resumed_ms.unwrap_or(0).max(stop.resets_at_ms);
            if gone || (stop.resume_at_ms.is_none() && now_ms >= last + REHIT_MS) {
                self.saved.stops.remove(&terminal);
                dirty = true;
                if gone {
                    changes.push(Change::Gone(terminal));
                }
                continue;
            }
            let Some(a) = agent else { continue };
            let stop = self.saved.stops.get_mut(&terminal).expect("listed above");
            if !stop.held() {
                continue;
            }
            match flock::status(&a.agent_status) {
                AgentStatus::Idle | AgentStatus::Done if !stop.idle_seen => {
                    stop.idle_seen = true;
                    dirty = true;
                }
                AgentStatus::Working | AgentStatus::Blocked if stop.idle_seen => {
                    stop.continued = true;
                    stop.resume_at_ms = None;
                    dirty = true;
                    changes.push(Change::Prompted(terminal));
                }
                _ => {}
            }
        }
        if dirty {
            self.save();
        }
        changes
    }

    /// Armed resumes whose time has come, in the order their agents were stopped.
    pub fn due(&self, now_ms: u64) -> Vec<(String, Stop)> {
        let mut due: Vec<(String, Stop)> = self
            .saved
            .stops
            .iter()
            .filter(|(_, s)| s.held() && s.resume_at_ms.is_some_and(|at| at <= now_ms))
            .map(|(t, s)| (t.clone(), s.clone()))
            .collect();
        due.sort_by_key(|(_, s)| s.stopped_ms);
        due
    }

    /// Whether a resume is still armed for this terminal and stop.
    pub fn armed(&self, terminal_id: &str, stop: &Stop) -> bool {
        self.saved.enabled
            && self.saved.stops.get(terminal_id).is_some_and(|s| {
                s.held() && s.stopped_ms == stop.stopped_ms && s.resume_at_ms.is_some()
            })
    }

    /// At the resume time another window may still be at its limit (both were exhausted):
    /// the resume then waits for that reset. Returns the new resume time.
    pub fn rearm(
        &mut self,
        terminal_id: &str,
        plan: Option<&PlanUsage>,
        now_ms: u64,
    ) -> Option<u64> {
        let (window, resets_at_ms) = plan.and_then(|p| exhausted(p, now_ms))?;
        let grace = self.grace_ms;
        let s = self.saved.stops.get_mut(terminal_id).filter(|s| s.held())?;
        s.window = window;
        s.resets_at_ms = resets_at_ms;
        s.resume_at_ms = Some(resets_at_ms + grace);
        let at = s.resume_at_ms;
        self.save();
        at
    }

    pub fn resumed(&mut self, terminal_id: &str, now_ms: u64) {
        if let Some(s) = self.saved.stops.get_mut(terminal_id) {
            s.resumed_ms = Some(now_ms);
            s.resume_at_ms = None;
            self.save();
        }
    }

    /// Cancels an armed resume; the agent stays stopped until someone prompts it.
    pub fn cancel(&mut self, terminal_id: &str) -> bool {
        let Some(s) = self
            .saved
            .stops
            .get_mut(terminal_id)
            .filter(|s| s.held() && s.resume_at_ms.is_some())
        else {
            return false;
        };
        s.resume_at_ms = None;
        self.save();
        true
    }

    /// A prompt from the phone goes through like one typed on the machine.
    pub fn prompted(&mut self, terminal_id: &str) -> bool {
        let Some(s) = self.saved.stops.get_mut(terminal_id).filter(|s| s.held()) else {
            return false;
        };
        s.continued = true;
        s.resume_at_ms = None;
        self.save();
        true
    }

    pub fn shown(&self, terminal_id: &str, now_ms: u64) -> Option<PlanLimit> {
        self.saved
            .stops
            .get(terminal_id)
            .filter(|s| s.held() && (s.resume_at_ms.is_some() || s.resets_at_ms > now_ms))
            .map(|s| PlanLimit {
                window: s.window,
                resets_at_ms: s.resets_at_ms,
                resume_at_ms: s.resume_at_ms,
            })
    }
}

/// Types the fixed continuation prompt into an idle Claude Code agent still running the
/// stopped session, through `Driver::prompt_agent` and its guards: a box holding text, a
/// scrolled screen or a blocked agent refuses it. `auth` is checked right before the write.
pub async fn resume(
    drive: &Driver,
    agent: Option<&AgentInfo>,
    stop: &Stop,
    auth: &Authorized,
) -> Result<(), String> {
    let agent = agent.ok_or("the agent is gone")?;
    let session = agent.agent_session.as_ref().map(|s| s.value.as_str());
    if agent.agent.as_deref() != Some("claude") || session != Some(stop.session_id.as_str()) {
        return Err("another session runs in the pane".into());
    }
    if !matches!(
        flock::status(&agent.agent_status),
        AgentStatus::Idle | AgentStatus::Done
    ) {
        return Err(BUSY.into());
    }
    let terminal_id =
        TerminalId::new(agent.terminal_id.clone()).map_err(|_| "invalid terminal id")?;
    let mut op = [0u8; 16];
    getrandom::fill(&mut op).map_err(|_| "no randomness")?;
    let params = AgentPromptParams {
        op_id: OpId::new(URL_SAFE_NO_PAD.encode(op)).map_err(|_| "invalid op id")?,
        terminal_id,
        text: PromptText::new(CONTINUE.to_owned()).map_err(|_| "invalid prompt")?,
        expected_draft: None,
    };
    drive
        .prompt_agent(params, auth, true)
        .await
        .map(drop)
        .map_err(|(_, message)| message)
}

/// Someone prompted the agent by hand: not resumed, and no alert.
pub const BUSY: &str = "the agent is not idle";

/// "2 h 5 min", "45 min" or "3 days": a push names no clock time, which would be in the
/// machine's time zone rather than the phone's.
pub fn wait(ms: u64) -> String {
    let minutes = ms.div_ceil(60_000).max(1);
    match minutes {
        m if m >= 48 * 60 => format!("{} days", m.div_ceil(24 * 60)),
        m if m >= 60 && m % 60 == 0 => format!("{} h", m / 60),
        m if m >= 60 => format!("{} h {} min", m / 60, m % 60),
        m => format!("{m} min"),
    }
}

/// The PreToolUse hook's `stopReason`, which Claude Code shows and keeps in the conversation.
pub fn reason(w: LimitWindow) -> String {
    format!(
        "collie stopped Claude Code at the {} plan limit so it does not run on extra usage; it continues after the reset",
        window_name(w)
    )
}

pub fn window_name(w: LimitWindow) -> &'static str {
    match w {
        LimitWindow::FiveHour => "5-hour",
        LimitWindow::SevenDay => "weekly",
        LimitWindow::Unknown => "plan",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    const T: &str = "w1:p1";
    const START: u64 = 1_000_000;
    const RESET: u64 = START + 3_600_000;
    const WEEK: u64 = START + 5 * 86_400_000;

    fn agent(terminal: &str, status: &str, session: &str) -> AgentInfo {
        serde_json::from_value(serde_json::json!({
            "terminal_id": terminal, "workspace_id": "w1", "pane_id": terminal,
            "agent_status": status, "agent": "claude", "name": "fixer",
            "agent_session": {"value": session},
        }))
        .unwrap()
    }

    fn plan(five: u8, seven: u8, recorded_ms: u64) -> PlanUsage {
        PlanUsage {
            five_hour: Some(UsageWindow {
                used_percent: five,
                resets_at_ms: RESET,
            }),
            seven_day: Some(UsageWindow {
                used_percent: seven,
                resets_at_ms: WEEK,
            }),
            recorded_ms,
            codex: None,
            copilot: None,
        }
    }

    fn limits(dir: &tempfile::TempDir) -> Limits {
        let mut l = Limits::load(dir.path().join("limits.json"), GRACE, START);
        l.set_enabled(true);
        l.observe(&[agent(T, "working", SESSION)], START);
        l
    }

    fn stopped(l: &mut Limits, now: u64) -> Stop {
        match l.check(SESSION, false, Some(&plan(100, 50, now)), now) {
            Verdict::Stopped(t, s) => {
                assert_eq!(t, T);
                s
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn arms_from_the_latest_reset_among_the_limits_reached() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let now = START + 1_000;
        let weekly = l.check(SESSION, false, Some(&plan(99, 100, now)), now);
        assert!(matches!(&weekly, Verdict::Stopped(t, _) if t == T));
        let s = &l.saved.stops[T];
        assert_eq!((s.window, s.resets_at_ms), (LimitWindow::SevenDay, WEEK));
        assert_eq!(s.resume_at_ms, Some(WEEK + 60_000));
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let both = l.check(SESSION, false, Some(&plan(100, 100, now)), now);
        assert!(matches!(both, Verdict::Stopped(_, s) if s.resets_at_ms == WEEK));
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let s = stopped(&mut l, now);
        assert_eq!(
            (s.window, s.resume_at_ms, s.title.as_str()),
            (LimitWindow::FiveHour, Some(RESET + 60_000), "fixer")
        );
        assert_eq!(
            l.shown(T, now),
            Some(PlanLimit {
                window: LimitWindow::FiveHour,
                resets_at_ms: RESET,
                resume_at_ms: Some(RESET + 60_000),
            })
        );
    }

    #[test]
    fn nothing_stops_while_off_unlisted_or_under_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let now = START + 1_000;
        assert_eq!(
            l.check(SESSION, false, Some(&plan(99, 99, now)), now),
            Verdict::Go
        );
        assert_eq!(l.check(SESSION, false, None, now), Verdict::Go);
        let other = "00000000-0000-4000-8000-000000000001";
        assert_eq!(
            l.check(other, false, Some(&plan(100, 0, now)), now),
            Verdict::Go
        );
        l.set_enabled(false);
        assert_eq!(
            l.check(SESSION, false, Some(&plan(100, 0, now)), now),
            Verdict::Go
        );
        assert!(l.saved.stops.is_empty());
    }

    #[test]
    fn a_stale_or_pre_start_reading_never_arms() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let now = START + 1_000;
        // Recorded before collied started, as a usage.json read after a long sleep.
        assert_eq!(
            l.check(SESSION, false, Some(&plan(100, 0, START - 1)), now),
            Verdict::Go
        );
        let later = START + FRESH_MS + 1;
        assert_eq!(
            l.check(SESSION, false, Some(&plan(100, 0, START)), later),
            Verdict::Go
        );
        // A window past its reset is not at the limit.
        assert_eq!(
            l.check(SESSION, false, Some(&plan(100, 0, RESET)), RESET),
            Verdict::Go
        );
        assert!(l.saved.stops.is_empty());
    }

    #[test]
    fn the_stopped_turn_is_held_until_idle_then_a_prompt_by_hand_goes_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let now = START + 1_000;
        stopped(&mut l, now);
        let limit = plan(100, 0, now);
        // Parallel calls of the same turn, and subagents, are stopped too.
        assert_eq!(
            l.check(SESSION, false, Some(&limit), now + 1),
            Verdict::Hold(LimitWindow::FiveHour)
        );
        l.observe(&[agent(T, "idle", SESSION)], now + 2);
        assert_eq!(
            l.check(SESSION, true, Some(&limit), now + 3),
            Verdict::Hold(LimitWindow::FiveHour)
        );
        // A call after the agent went idle means someone prompted it.
        assert_eq!(
            l.check(SESSION, false, Some(&limit), now + 4),
            Verdict::Continued(T.into())
        );
        assert_eq!(l.shown(T, now + 4), None);
        assert!(l.due(RESET + 60_000).is_empty());
        assert_eq!(l.check(SESSION, false, Some(&limit), now + 5), Verdict::Go);
        assert_eq!(l.check(SESSION, true, Some(&limit), now + 6), Verdict::Go);
    }

    #[test]
    fn cancel_rules() {
        let now = START + 1_000;
        // Prompted on the machine: idle, then working.
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, now);
        assert!(
            l.observe(&[agent(T, "working", SESSION)], now + 1)
                .is_empty()
        );
        assert!(l.observe(&[agent(T, "idle", SESSION)], now + 2).is_empty());
        assert_eq!(
            l.observe(&[agent(T, "working", SESSION)], now + 3),
            vec![Change::Prompted(T.into())]
        );
        assert!(l.due(RESET + 60_000).is_empty());
        // Prompted from the phone, and cancelled from the phone.
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, now);
        assert!(l.prompted(T));
        assert!(!l.prompted(T));
        assert!(l.due(RESET + 60_000).is_empty());
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, now);
        assert!(l.cancel(T));
        assert!(!l.cancel(T));
        assert!(l.due(RESET + 60_000).is_empty());
        assert_eq!(l.shown(T, now).unwrap().resume_at_ms, None);
        assert_eq!(l.shown(T, RESET), None);
        // Closed, or another session in the pane (exited, /clear, a new agent).
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, now);
        let other = agent("w1:p2", "idle", SESSION);
        assert_eq!(l.observe(&[other], now + 1), vec![Change::Gone(T.into())]);
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, now);
        let new = "00000000-0000-4000-8000-000000000002";
        assert_eq!(
            l.observe(&[agent(T, "idle", new)], now + 1),
            vec![Change::Gone(T.into())]
        );
        // A session not reported yet keeps it.
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, now);
        let mut quiet = agent(T, "idle", SESSION);
        quiet.agent_session = None;
        assert!(l.observe(&[quiet], now + 1).is_empty());
        assert_eq!(l.due(RESET + 60_000).len(), 1);
        // An empty list is not trusted.
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, now);
        assert!(l.observe(&[], now + 1).is_empty());
        assert_eq!(l.due(RESET + 60_000).len(), 1);
        // Turned off.
        l.set_enabled(false);
        assert!(l.due(RESET + 60_000).is_empty());
    }

    #[test]
    fn resumes_run_in_stop_order_and_rearm_for_another_exhausted_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let other = "00000000-0000-4000-8000-000000000003";
        l.observe(
            &[
                agent("w1:p2", "working", other),
                agent(T, "working", SESSION),
            ],
            START,
        );
        let now = START + 1_000;
        assert!(matches!(
            l.check(other, false, Some(&plan(100, 0, now)), now),
            Verdict::Stopped(..)
        ));
        stopped(&mut l, now + 1);
        let due = l.due(RESET + 60_000);
        assert_eq!(
            due.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(),
            ["w1:p2", T]
        );
        assert!(l.armed(T, &due[1].1));
        // The weekly limit is spent too: wait for it, from an old reading.
        let old = plan(100, 100, now);
        assert_eq!(l.rearm(T, Some(&old), RESET + 60_000), Some(WEEK + 60_000));
        assert_eq!(l.due(RESET + 60_000).len(), 1);
        assert_eq!(
            l.shown(T, RESET + 60_000).unwrap().window,
            LimitWindow::SevenDay
        );
        assert_eq!(l.rearm(T, Some(&plan(100, 99, now)), RESET + 60_000), None);
        l.set_enabled(true);
        assert!(!l.armed("w1:p9", &due[1].1));
        l.set_enabled(false);
        assert!(!l.armed(T, &due[1].1));
    }

    #[test]
    fn a_limit_hit_right_after_a_resume_rearms_once_then_gives_up() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let now = START + 1_000;
        stopped(&mut l, now);
        l.resumed(T, RESET + 60_000);
        assert_eq!(l.shown(T, RESET + 60_000), None);
        let next = RESET + 5 * 3_600_000;
        let again = |l: &mut Limits, at: u64, resets: u64| {
            let mut p = plan(100, 0, at);
            p.five_hour.as_mut().unwrap().resets_at_ms = resets;
            match l.check(SESSION, false, Some(&p), at) {
                Verdict::Stopped(_, s) => s,
                other => panic!("{other:?}"),
            }
        };
        let s = again(&mut l, RESET + 120_000, next);
        assert_eq!((s.rehits, s.resume_at_ms), (1, Some(next + 60_000)));
        l.resumed(T, next + 60_000);
        let after = next + 5 * 3_600_000;
        let s = again(&mut l, next + 120_000, after);
        assert_eq!((s.rehits, s.resume_at_ms), (2, None));
        assert!(l.due(after + 60_000).is_empty());
        assert_eq!(l.shown(T, next + 120_000).unwrap().resume_at_ms, None);
        // A limit long after a resume is a new one.
        l.saved.stops.get_mut(T).unwrap().resumed_ms = Some(next);
        let s = again(&mut l, next + REHIT_MS, after);
        assert_eq!((s.rehits, s.resume_at_ms), (0, Some(after + 60_000)));
    }

    #[test]
    fn stops_survive_a_restart_and_overdue_ones_are_left_to_the_caller() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, START + 1_000);
        drop(l);
        let path = dir.path().join("limits.json");
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(&path).unwrap().permissions(),
        );
        assert_eq!(mode & 0o777, 0o600);
        let l = Limits::load(path, GRACE, START + 10);
        assert!(l.enabled());
        let due = l.due(RESET + 60_000 + OVERDUE_MS + 1);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].1.resume_at_ms, Some(RESET + 60_000));
        assert_eq!(
            *l.watch().borrow(),
            vec![Limited {
                title: "fixer".into(),
                resets_at_ms: RESET,
                resume_at_ms: Some(RESET + 60_000),
            }]
        );
        // Unreadable: nothing is stopped and nothing resumes.
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        let bad = Limits::load(dir.path().join("bad.json"), GRACE, START);
        assert!(!bad.enabled() && bad.due(u64::MAX).is_empty());
    }

    #[test]
    fn the_tapped_status_line_at_its_limit_is_reached() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("collie");
        let input = include_str!("../tests/fixtures/claude/statusline-at-limit.synthetic.json");
        let reset = 1_738_425_600_000;
        let now = reset - 3_600_000;
        crate::usage::record(&data, input.as_bytes(), now).unwrap();
        let plan = crate::usage::last_recorded(&data).unwrap().plan;
        assert_eq!(
            reached(plan.as_ref(), now - 1, now),
            Some((LimitWindow::FiveHour, reset))
        );
        let below = include_str!("../tests/fixtures/statusline.json");
        crate::usage::record(&data, below.as_bytes(), reset).unwrap();
        let plan = crate::usage::last_recorded(&data).unwrap().plan;
        assert_eq!(reached(plan.as_ref(), now, reset), None);
    }

    #[test]
    fn waits_read_as_durations() {
        assert_eq!(wait(1), "1 min");
        assert_eq!(wait(45 * 60_000), "45 min");
        assert_eq!(wait(60 * 60_000), "1 h");
        assert_eq!(wait(125 * 60_000), "2 h 5 min");
        assert_eq!(wait(3 * 86_400_000 - 1), "3 days");
    }
}
