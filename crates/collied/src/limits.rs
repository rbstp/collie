use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use protocol::{
    AgentPromptParams, AgentStatus, ErrorCode, LimitWindow, OpId, PlanLimit, PromptText, TerminalId,
};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::drive::{Authorized, Driver};
use crate::runtime::AgentInfo;
use crate::runtime::herdr_core::owns;
use crate::usage::Reached;
use crate::{approvals, flock, peers};

pub const GRACE: Duration = Duration::from_secs(60);
/// A reading older than this stops nothing: a stop needs a limit reached now.
const FRESH_MS: u64 = 15 * 60_000;
/// A stop this soon after a resume is the same limit hit again.
const REHIT_MS: u64 = 15 * 60_000;
/// A resume missed by more than this (the machine slept, collied was off) is dropped.
pub const OVERDUE_MS: u64 = 3 * 3_600_000;
/// Claude Code's `resets_at` for one window can move by a second between readings.
const SAME_RESET_MS: u64 = 60_000;
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
    /// The prompt of the stopped turn: its other calls are the same turn, not a new prompt.
    #[serde(default)]
    pub prompt_id: Option<String>,
    #[serde(default)]
    pub resume_at_ms: Option<u64>,
    /// Released by a prompt typed on the machine or sent from the phone: it runs on extra
    /// usage until the next limit.
    #[serde(default)]
    continued: bool,
    /// Set right before collied types the continuation, so its own prompt is never held.
    #[serde(default)]
    resumed_ms: Option<u64>,
    #[serde(default)]
    rehits: u8,
}

impl Stop {
    fn held(&self) -> bool {
        !self.continued && self.resumed_ms.is_none()
    }

    fn shows(&self, now_ms: u64) -> bool {
        self.held() && (self.resume_at_ms.is_some() || self.resets_at_ms > now_ms)
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

/// A PreToolUse call: `prompt_id` names the turn it belongs to.
#[derive(Debug, Default)]
pub struct Call<'a> {
    pub subagent: bool,
    pub prompt_id: Option<&'a str>,
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Go,
    Hold(LimitWindow),
    Stopped(String, Stop),
    Continued(String),
}

/// What the resume task does with a due resume before it types anything.
#[derive(Debug, PartialEq)]
pub enum Attempt {
    Prompt,
    /// Try again at the next tick: nothing tells the agent is gone or busy.
    Retry,
    Cancel(&'static str),
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Resumed,
    Retry,
    Cancel(&'static str),
    /// The resume was cancelled or the switch turned off while it was being typed.
    Withdrawn,
}

#[derive(Debug, PartialEq)]
pub enum Finish {
    Resumed,
    Retry,
    Cancel,
}

/// Agents collied stopped at the plan limit, kept on disk so an armed resume survives a
/// restart, and the switch that turns stopping on.
pub struct Limits {
    saved: Saved,
    path: Option<PathBuf>,
    grace_ms: u64,
    started_ms: u64,
    /// Claude Code session id to its agent, from the last poll.
    sessions: HashMap<String, AgentInfo>,
    shown: watch::Sender<Vec<Limited>>,
}

/// The latest reset among the windows the session itself reported at the limit, from a
/// reading taken while collied ran and recently; else none, so a stale `usage.json` stops
/// nothing.
pub fn reached(
    reading: Option<&Reached>,
    started_ms: u64,
    now_ms: u64,
) -> Option<(LimitWindow, u64)> {
    let r = reading.filter(|r| r.seen_ms >= started_ms && now_ms < r.seen_ms + FRESH_MS)?;
    exhausted(r, now_ms)
}

/// Within a window usage only grows, so a window at its limit stays there until its reset,
/// however old the reading.
fn exhausted(r: &Reached, now_ms: u64) -> Option<(LimitWindow, u64)> {
    [
        (LimitWindow::FiveHour, r.five_hour_resets_ms),
        (LimitWindow::SevenDay, r.seven_day_resets_ms),
    ]
    .into_iter()
    .filter_map(|(w, at)| Some((w, at.filter(|at| *at > now_ms)?)))
    .max_by_key(|(_, at)| *at)
}

fn same_reset(a: u64, b: u64) -> bool {
    a.abs_diff(b) <= SAME_RESET_MS
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
            .filter(|s| s.shows(now))
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

    fn held(&self, session_id: &str) -> Option<&Stop> {
        let terminal = &self.sessions.get(session_id)?.terminal_id;
        self.saved
            .stops
            .get(terminal)
            .filter(|s| s.session_id == session_id && s.held())
    }

    /// The agent whose transcript says whether the call's prompt was typed: asked only of
    /// a held agent's own call from another prompt than the stopped one.
    pub fn typed_check(&self, session_id: &str, call: &Call) -> Option<AgentInfo> {
        let s = self.held(session_id).filter(|_| self.saved.enabled)?;
        let other = call
            .prompt_id
            .is_some_and(|p| s.prompt_id.as_deref() != Some(p));
        (other && !call.subagent)
            .then(|| self.sessions.get(session_id).cloned())
            .flatten()
    }

    /// The PreToolUse hook's question. Only a session of an agent collied lists is stopped,
    /// since only those can be resumed; anything unknown goes on. While held, only a prompt
    /// typed on the machine (`typed`, from the transcript) releases the agent: a background
    /// task, a subagent's report, a scheduled prompt or another session's message is
    /// stopped again, with no new alert.
    pub fn check(
        &mut self,
        session_id: &str,
        call: &Call,
        reading: Option<&Reached>,
        now_ms: u64,
        typed: bool,
    ) -> Verdict {
        if !self.saved.enabled {
            return Verdict::Go;
        }
        let Some(agent) = self.sessions.get(session_id) else {
            return Verdict::Go;
        };
        let (terminal, title) = (agent.terminal_id.clone(), approvals::alert_title(agent));
        let grace = self.grace_ms;
        if let Some(s) = self
            .saved
            .stops
            .get_mut(&terminal)
            .filter(|s| s.session_id == session_id && s.held())
        {
            if now_ms >= s.resets_at_ms {
                let Some((window, resets_at_ms)) = reading.and_then(|r| exhausted(r, now_ms))
                else {
                    s.resumed_ms = Some(now_ms);
                    s.resume_at_ms = None;
                    self.save();
                    return Verdict::Go;
                };
                s.window = window;
                s.resets_at_ms = resets_at_ms;
                if s.resume_at_ms.is_some() {
                    s.resume_at_ms = Some(resets_at_ms + grace);
                }
                let window = s.window;
                self.save();
                return Verdict::Hold(window);
            }
            let other = call
                .prompt_id
                .is_some_and(|p| s.prompt_id.as_deref() != Some(p));
            if typed && other && !call.subagent {
                s.continued = true;
                s.resume_at_ms = None;
                self.save();
                return Verdict::Continued(terminal);
            }
            return Verdict::Hold(s.window);
        }
        let Some((window, resets_at_ms)) = reached(reading, self.started_ms, now_ms) else {
            return Verdict::Go;
        };
        let prev = self
            .saved
            .stops
            .get(&terminal)
            .filter(|s| s.session_id == session_id);
        if prev.is_some_and(|s| s.continued && same_reset(s.resets_at_ms, resets_at_ms)) {
            return Verdict::Go;
        }
        let rehits = prev
            .and_then(|s| Some((s, s.resumed_ms?)))
            .filter(|(_, at)| now_ms < at + REHIT_MS)
            .map_or(0, |(s, _)| s.rehits.saturating_add(1));
        let stop = Stop {
            session_id: session_id.to_owned(),
            title,
            window,
            resets_at_ms,
            stopped_ms: now_ms,
            prompt_id: call.prompt_id.map(str::to_owned),
            // Re-armed once after a resume that hit the limit again, then given up.
            resume_at_ms: (rehits <= 1).then_some(resets_at_ms + grace),
            continued: false,
            resumed_ms: None,
            rehits,
        };
        self.saved.stops.insert(terminal.clone(), stop.clone());
        self.save();
        Verdict::Stopped(terminal, stop)
    }

    /// Maps sessions to terminals from the polled agents and drops stops whose agent is
    /// gone, returning their terminals, or whose limit has long passed. A runtime that lists
    /// no agent is not trusted, as for pull request watches. An agent working again does not
    /// release it: something other than a person may have woken it.
    pub fn observe(&mut self, agents: &[AgentInfo], now_ms: u64) -> Vec<String> {
        self.sessions = agents
            .iter()
            .filter(|a| a.agent.as_deref() == Some("claude"))
            .filter_map(|a| Some((a.agent_session.as_ref()?.value.clone(), a.clone())))
            .collect();
        let mut gone = Vec::new();
        let before = self.saved.stops.len();
        self.saved.stops.retain(|terminal, s| {
            let core = owns(terminal);
            let listed = agents.iter().any(|a| owns(&a.terminal_id) == core);
            // A session not reported yet is not another one.
            let left = match agents.iter().find(|a| a.terminal_id == *terminal) {
                None => listed,
                Some(a) => {
                    a.agent.as_deref().is_some_and(|k| k != "claude")
                        || a.agent_session
                            .as_ref()
                            .is_some_and(|v| v.value != s.session_id)
                }
            };
            if left {
                gone.push(terminal.clone());
            }
            let last = s.resumed_ms.unwrap_or(0).max(s.resets_at_ms);
            !left && (s.resume_at_ms.is_some() || now_ms < last + REHIT_MS)
        });
        if self.saved.stops.len() != before {
            self.save();
        }
        gone
    }

    /// While held, collied's own wakes (a pull request watch) wait for the resume.
    pub fn holds(&self, terminal_id: &str) -> bool {
        self.saved.stops.get(terminal_id).is_some_and(Stop::held)
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

    /// At the resume time another window the session reported may still be at its limit:
    /// the resume then waits for that reset. Returns the new resume time.
    pub fn rearm(
        &mut self,
        terminal_id: &str,
        reading: Option<&Reached>,
        now_ms: u64,
    ) -> Option<u64> {
        let (window, resets_at_ms) = reading.and_then(|r| exhausted(r, now_ms))?;
        let grace = self.grace_ms;
        let s = self.saved.stops.get_mut(terminal_id).filter(|s| s.held())?;
        s.window = window;
        s.resets_at_ms = resets_at_ms;
        s.resume_at_ms = Some(resets_at_ms + grace);
        let at = s.resume_at_ms;
        self.save();
        at
    }

    /// Marks the resume before the write, so the continuation's own calls are not held.
    pub fn begin_resume(&mut self, terminal_id: &str, stop: &Stop, now_ms: u64) -> bool {
        let enabled = self.saved.enabled;
        let Some(s) = self.saved.stops.get_mut(terminal_id).filter(|s| {
            enabled && s.held() && s.stopped_ms == stop.stopped_ms && s.resume_at_ms.is_some()
        }) else {
            return false;
        };
        s.resumed_ms = Some(now_ms);
        self.save();
        true
    }

    /// Checked right before each write of the continuation.
    pub fn resuming(&self, terminal_id: &str, stopped_ms: u64) -> bool {
        self.saved.enabled
            && self.saved.stops.get(terminal_id).is_some_and(|s| {
                s.stopped_ms == stopped_ms && s.resumed_ms.is_some() && s.resume_at_ms.is_some()
            })
    }

    pub fn finish_resume(&mut self, terminal_id: &str, stopped_ms: u64, finish: Finish) {
        let Some(s) = self
            .saved
            .stops
            .get_mut(terminal_id)
            .filter(|s| s.stopped_ms == stopped_ms)
        else {
            return;
        };
        match finish {
            Finish::Resumed => s.resume_at_ms = None,
            Finish::Retry => s.resumed_ms = None,
            Finish::Cancel => {
                s.resumed_ms = None;
                s.resume_at_ms = None;
            }
        }
        self.save();
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
            .filter(|s| s.shows(now_ms))
            .map(|s| PlanLimit {
                window: s.window,
                resets_at_ms: s.resets_at_ms,
                resume_at_ms: s.resume_at_ms,
            })
    }
}

pub fn overdue(stop: &Stop, now_ms: u64) -> bool {
    stop.resume_at_ms
        .is_some_and(|at| now_ms > at.saturating_add(OVERDUE_MS))
}

/// A failed listing or an agent missing from it is retried: only the reconcile's trusted
/// list ends a stop for an agent that is gone.
pub fn before_resume(agent: Option<&AgentInfo>, stop: &Stop) -> Attempt {
    let Some(a) = agent else {
        return Attempt::Retry;
    };
    let session = a.agent_session.as_ref().map(|s| s.value.as_str());
    if a.agent.as_deref() != Some("claude") || session.is_some_and(|s| s != stop.session_id) {
        return Attempt::Cancel("another session runs in its pane");
    }
    if session.is_none() {
        return Attempt::Retry;
    }
    match flock::status(&a.agent_status) {
        AgentStatus::Idle | AgentStatus::Done => Attempt::Prompt,
        _ => Attempt::Cancel("it was not idle at the reset"),
    }
}

/// Fixed reasons only: a driver's message can hold the text of the agent's input box.
pub fn outcome(result: Result<(), ErrorCode>) -> Outcome {
    match result {
        Ok(()) => Outcome::Resumed,
        Err(ErrorCode::NotPaired) => Outcome::Withdrawn,
        Err(ErrorCode::DraftChanged) => Outcome::Cancel("text is waiting in its input box"),
        Err(ErrorCode::DraftNotCleared | ErrorCode::InputUnfocused) => {
            Outcome::Cancel("its input box could not be used")
        }
        Err(ErrorCode::AgentNotReady | ErrorCode::AgentBlocked) => {
            Outcome::Cancel("it was not ready for a prompt")
        }
        Err(_) => Outcome::Retry,
    }
}

/// Types the fixed continuation through `Driver::prompt_agent` and its guards, with
/// `idle_only` and no `expected_draft`: text in the box, a scrolled screen or a busy or
/// blocked agent refuses it. `auth` is checked right before each write.
pub async fn resume(drive: &Driver, agent: &AgentInfo, auth: &Authorized) -> Result<(), ErrorCode> {
    let terminal_id =
        TerminalId::new(agent.terminal_id.clone()).map_err(|_| ErrorCode::InvalidParams)?;
    let mut op = [0u8; 16];
    getrandom::fill(&mut op).map_err(|_| ErrorCode::Internal)?;
    let params = AgentPromptParams {
        op_id: OpId::new(URL_SAFE_NO_PAD.encode(op)).map_err(|_| ErrorCode::Internal)?,
        terminal_id,
        text: PromptText::new(CONTINUE.to_owned()).map_err(|_| ErrorCode::Internal)?,
        expected_draft: None,
    };
    drive
        .prompt_agent(params, auth, true)
        .await
        .map(drop)
        .map_err(|(code, _)| code)
}

/// "2 h 5 min", "45 min" or "3 days 4 h": a push names no clock time, which would be in the
/// machine's time zone rather than the phone's.
pub fn wait(ms: u64) -> String {
    let minutes = ms.div_ceil(60_000).max(1);
    let (days, hours, mins) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
    match minutes {
        m if m >= 48 * 60 && hours == 0 => format!("{days} days"),
        m if m >= 48 * 60 => format!("{days} days {hours} h"),
        m if m >= 60 && m % 60 == 0 => format!("{} h", m / 60),
        m if m >= 60 => format!("{} h {mins} min", m / 60),
        m => format!("{m} min"),
    }
}

/// The PreToolUse hook's `stopReason`, which Claude Code shows and keeps in the conversation.
pub fn reason(w: LimitWindow) -> String {
    format!(
        "collie stopped Claude Code at the {} plan limit so it does not run on extra usage. It continues after the reset. To continue now on extra usage, type a prompt here, use Continue now in collie, or turn off Stop at the plan limit.",
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
    const STOPPED: &str = "11111111-1111-4111-8111-111111111111";
    const LATER: &str = "22222222-2222-4222-8222-222222222222";

    fn agent(terminal: &str, status: &str, session: &str) -> AgentInfo {
        serde_json::from_value(serde_json::json!({
            "terminal_id": terminal, "workspace_id": "w1", "pane_id": terminal,
            "agent_status": status, "agent": "claude", "name": "fixer",
            "agent_session": {"value": session},
        }))
        .unwrap()
    }

    fn reading(five: bool, seven: bool, seen_ms: u64) -> Reached {
        Reached {
            five_hour_resets_ms: five.then_some(RESET),
            seven_day_resets_ms: seven.then_some(WEEK),
            seen_ms,
        }
    }

    fn call(prompt_id: &str) -> Call<'_> {
        Call {
            subagent: false,
            prompt_id: Some(prompt_id),
        }
    }

    fn limits(dir: &tempfile::TempDir) -> Limits {
        let mut l = Limits::load(dir.path().join("limits.json"), GRACE, START);
        l.set_enabled(true);
        l.observe(&[agent(T, "working", SESSION)], START);
        l
    }

    fn stopped(l: &mut Limits, now: u64) -> Stop {
        let r = reading(true, false, now);
        match l.check(SESSION, &call(STOPPED), Some(&r), now, false) {
            Verdict::Stopped(t, s) => {
                assert_eq!(t, T);
                s
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn arms_from_the_latest_reset_among_the_limits_reached() {
        let now = START + 1_000;
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let both = l.check(
            SESSION,
            &call(STOPPED),
            Some(&reading(true, true, now)),
            now,
            false,
        );
        assert!(matches!(&both, Verdict::Stopped(_, s) if s.resets_at_ms == WEEK));
        let s = &l.saved.stops[T];
        assert_eq!(
            (s.window, s.resume_at_ms),
            (LimitWindow::SevenDay, Some(WEEK + 60_000))
        );
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let s = stopped(&mut l, now);
        assert_eq!(
            (s.window, s.resume_at_ms, s.title.as_str()),
            (LimitWindow::FiveHour, Some(RESET + 60_000), "fixer")
        );
        assert_eq!(s.prompt_id.as_deref(), Some(STOPPED));
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
            l.check(SESSION, &call(STOPPED), None, now, false),
            Verdict::Go
        );
        let other = "00000000-0000-4000-8000-000000000001";
        let r = reading(true, false, now);
        assert_eq!(
            l.check(other, &call(STOPPED), Some(&r), now, false),
            Verdict::Go
        );
        l.set_enabled(false);
        assert_eq!(
            l.check(SESSION, &call(STOPPED), Some(&r), now, false),
            Verdict::Go
        );
        assert!(l.saved.stops.is_empty());
    }

    #[test]
    fn only_the_session_that_reached_the_limit_stops() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let other = "00000000-0000-4000-8000-000000000002";
        l.observe(
            &[
                agent(T, "working", SESSION),
                agent("w1:p2", "working", other),
            ],
            START,
        );
        let now = START + 1_000;
        // Another account's session on the same machine: it reported nothing at the limit.
        assert_eq!(l.check(other, &call(LATER), None, now, false), Verdict::Go);
        let r = reading(true, false, now);
        assert!(matches!(
            l.check(SESSION, &call(STOPPED), Some(&r), now, false),
            Verdict::Stopped(..)
        ));
        assert!(!l.holds("w1:p2"));
    }

    #[test]
    fn a_stale_or_pre_start_reading_never_arms() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let now = START + 1_000;
        let before = reading(true, false, START - 1);
        assert_eq!(
            l.check(SESSION, &call(STOPPED), Some(&before), now, false),
            Verdict::Go
        );
        let later = START + FRESH_MS + 1;
        let old = reading(true, false, START);
        assert_eq!(
            l.check(SESSION, &call(STOPPED), Some(&old), later, false),
            Verdict::Go
        );
        let past = reading(true, false, RESET);
        assert_eq!(
            l.check(SESSION, &call(STOPPED), Some(&past), RESET, false),
            Verdict::Go
        );
        assert!(l.saved.stops.is_empty());
    }

    #[test]
    fn a_held_agent_is_released_only_by_a_prompt_typed_on_the_machine() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let now = START + 1_000;
        stopped(&mut l, now);
        let r = reading(true, false, now);
        // The stopped turn's other calls, and a subagent's, whatever the transcript says.
        assert_eq!(
            l.check(SESSION, &call(STOPPED), Some(&r), now + 1, true),
            Verdict::Hold(LimitWindow::FiveHour)
        );
        assert!(l.typed_check(SESSION, &call(STOPPED)).is_none());
        let sub = Call {
            subagent: true,
            prompt_id: Some(LATER),
        };
        assert!(l.typed_check(SESSION, &sub).is_none());
        assert_eq!(
            l.check(SESSION, &sub, Some(&r), now + 2, true),
            Verdict::Hold(LimitWindow::FiveHour)
        );
        // Working again after a wakeup releases nothing.
        l.observe(&[agent(T, "idle", SESSION)], now + 3);
        l.observe(&[agent(T, "working", SESSION)], now + 4);
        assert!(l.holds(T));
        // A turn the transcript does not show as typed: held again, the resume still armed.
        assert!(l.typed_check(SESSION, &call(LATER)).is_some());
        assert_eq!(
            l.check(SESSION, &call(LATER), None, now + 5, false),
            Verdict::Hold(LimitWindow::FiveHour)
        );
        assert_eq!(l.due(RESET + 60_000).len(), 1);
        // A typed prompt goes on, on extra usage, and the resume is cancelled.
        assert_eq!(
            l.check(SESSION, &call(LATER), None, now + 6, true),
            Verdict::Continued(T.into())
        );
        assert_eq!(l.shown(T, now + 6), None);
        assert!(l.due(RESET + 60_000).is_empty());
        assert_eq!(
            l.check(SESSION, &call(LATER), Some(&r), now + 7, false),
            Verdict::Go
        );
        // The same window read a second later is not a new limit.
        let jitter = Reached {
            five_hour_resets_ms: Some(RESET + 1_000),
            ..r
        };
        assert_eq!(
            l.check(SESSION, &call(LATER), Some(&jitter), now + 8, false),
            Verdict::Go
        );
    }

    #[test]
    fn a_background_subagent_and_its_report_stay_held() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let now = START + 1_000;
        let r = reading(true, false, now);
        // A background subagent's call trips the stop while the main session is idle.
        let sub = Call {
            subagent: true,
            prompt_id: Some(STOPPED),
        };
        let mut pushes = 0;
        for (i, c) in [&sub, &sub, &call(LATER)].into_iter().enumerate() {
            match l.check(SESSION, c, Some(&r), now + i as u64, false) {
                Verdict::Stopped(..) => pushes += 1,
                Verdict::Hold(_) => {}
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(pushes, 1, "one alert per stop");
        assert_eq!(l.due(RESET + 60_000).len(), 1);
    }

    #[test]
    fn the_end_of_the_window_releases_a_held_agent_unless_another_is_spent() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, START + 1_000);
        assert_eq!(
            l.check(
                SESSION,
                &call(LATER),
                Some(&reading(true, true, START)),
                RESET,
                false
            ),
            Verdict::Hold(LimitWindow::SevenDay)
        );
        assert_eq!(l.due(RESET + 60_000).len(), 0);
        assert_eq!(l.due(WEEK + 60_000).len(), 1);
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, START + 1_000);
        assert_eq!(
            l.check(SESSION, &call(LATER), None, RESET, false),
            Verdict::Go
        );
        assert!(!l.holds(T));
        assert!(l.due(RESET + 60_000).is_empty());
    }

    #[test]
    fn cancel_rules() {
        let now = START + 1_000;
        // From the phone: a prompt, Continue now included, or Cancel.
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
        assert!(l.holds(T), "still stopped");
        // Closed, or another session in the pane (exited, /clear, a new agent).
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, now);
        let other = agent("w1:p2", "idle", SESSION);
        assert_eq!(l.observe(&[other], now + 1), vec![T.to_owned()]);
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, now);
        let new = "00000000-0000-4000-8000-000000000002";
        assert_eq!(
            l.observe(&[agent(T, "idle", new)], now + 1),
            vec![T.to_owned()]
        );
        // A session not reported yet keeps it; an empty list is not trusted.
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, now);
        let mut quiet = agent(T, "idle", SESSION);
        quiet.agent_session = None;
        assert!(l.observe(&[quiet], now + 1).is_empty());
        assert!(l.observe(&[], now + 1).is_empty());
        assert_eq!(l.due(RESET + 60_000).len(), 1);
        l.set_enabled(false);
        assert!(l.due(RESET + 60_000).is_empty());
    }

    #[test]
    fn a_pull_request_watch_waits_while_the_agent_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        assert!(!l.holds(T));
        stopped(&mut l, START + 1_000);
        assert!(l.holds(T));
        assert!(l.cancel(T));
        assert!(l.holds(T), "a cancelled resume leaves it stopped");
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let s2 = stopped(&mut l, START + 1_000);
        assert!(l.begin_resume(T, &s2, RESET + 60_000));
        assert!(!l.holds(T), "the wake follows the resume");
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        stopped(&mut l, START + 1_000);
        assert!(l.prompted(T));
        assert!(!l.holds(T));
    }

    #[test]
    fn resumes_run_in_stop_order_and_rearm_for_another_spent_window() {
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
        let r = reading(true, false, now);
        assert!(matches!(
            l.check(other, &call(STOPPED), Some(&r), now, false),
            Verdict::Stopped(..)
        ));
        stopped(&mut l, now + 1);
        let due = l.due(RESET + 60_000);
        assert_eq!(
            due.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(),
            ["w1:p2", T]
        );
        let old = reading(true, true, now);
        assert_eq!(l.rearm(T, Some(&old), RESET + 60_000), Some(WEEK + 60_000));
        assert_eq!(l.due(RESET + 60_000).len(), 1);
        assert_eq!(
            l.shown(T, RESET + 60_000).unwrap().window,
            LimitWindow::SevenDay
        );
        assert_eq!(l.rearm(T, Some(&r), RESET + 60_000), None);
    }

    #[test]
    fn a_resume_is_marked_before_it_is_typed_and_retried_or_cancelled_after() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let s = stopped(&mut l, START + 1_000);
        let at = RESET + 60_000;
        assert!(l.begin_resume(T, &s, at));
        assert!(l.resuming(T, s.stopped_ms));
        assert!(l.due(at).is_empty(), "not resumed twice");
        // Its own continuation's calls are not held.
        assert!(!l.holds(T));
        assert_eq!(
            l.check(SESSION, &call(LATER), None, at + 1, false),
            Verdict::Go
        );
        l.finish_resume(T, s.stopped_ms, Finish::Retry);
        assert!(!l.resuming(T, s.stopped_ms));
        assert_eq!(l.due(at).len(), 1, "a transient failure keeps it armed");
        assert!(l.begin_resume(T, &s, at));
        l.finish_resume(T, s.stopped_ms, Finish::Cancel);
        assert!(l.due(at).is_empty());
        assert!(l.holds(T));
        // Turned off while typing: the write is refused.
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let s = stopped(&mut l, START + 1_000);
        assert!(l.begin_resume(T, &s, at));
        l.set_enabled(false);
        assert!(!l.resuming(T, s.stopped_ms));
        assert!(!l.begin_resume(T, &s, at));
    }

    #[test]
    fn the_resume_task_retries_what_may_pass_and_cancels_what_will_not() {
        let s = Stop {
            session_id: SESSION.into(),
            title: "fixer".into(),
            window: LimitWindow::FiveHour,
            resets_at_ms: RESET,
            stopped_ms: START,
            prompt_id: None,
            resume_at_ms: Some(RESET + 60_000),
            continued: false,
            resumed_ms: None,
            rehits: 0,
        };
        assert_eq!(before_resume(None, &s), Attempt::Retry);
        let mut quiet = agent(T, "idle", SESSION);
        quiet.agent_session = None;
        assert_eq!(before_resume(Some(&quiet), &s), Attempt::Retry);
        let new = "00000000-0000-4000-8000-000000000002";
        assert!(matches!(
            before_resume(Some(&agent(T, "idle", new)), &s),
            Attempt::Cancel(_)
        ));
        for status in ["working", "blocked", "unknown"] {
            assert!(matches!(
                before_resume(Some(&agent(T, status, SESSION)), &s),
                Attempt::Cancel(_)
            ));
        }
        for status in ["idle", "done"] {
            assert_eq!(
                before_resume(Some(&agent(T, status, SESSION)), &s),
                Attempt::Prompt
            );
        }
        assert_eq!(outcome(Ok(())), Outcome::Resumed);
        assert_eq!(outcome(Err(ErrorCode::NotPaired)), Outcome::Withdrawn);
        for transient in [
            ErrorCode::HerdrUnavailable,
            ErrorCode::NotFound,
            ErrorCode::Internal,
        ] {
            assert_eq!(outcome(Err(transient)), Outcome::Retry);
        }
        assert_eq!(
            outcome(Err(ErrorCode::DraftChanged)),
            Outcome::Cancel("text is waiting in its input box")
        );
        assert!(!overdue(&s, RESET + 60_000 + OVERDUE_MS));
        assert!(overdue(&s, RESET + 60_001 + OVERDUE_MS));
    }

    #[test]
    fn a_limit_hit_right_after_a_resume_rearms_once_then_gives_up() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = limits(&dir);
        let now = START + 1_000;
        let s = stopped(&mut l, now);
        assert!(l.begin_resume(T, &s, RESET + 60_000));
        l.finish_resume(T, s.stopped_ms, Finish::Resumed);
        assert_eq!(l.shown(T, RESET + 60_000), None);
        let next = RESET + 5 * 3_600_000;
        let again = |l: &mut Limits, at: u64, resets: u64| {
            let r = Reached {
                five_hour_resets_ms: Some(resets),
                seven_day_resets_ms: None,
                seen_ms: at,
            };
            match l.check(SESSION, &call(LATER), Some(&r), at, false) {
                Verdict::Stopped(_, s) => s,
                other => panic!("{other:?}"),
            }
        };
        let s = again(&mut l, RESET + 120_000, next);
        assert_eq!((s.rehits, s.resume_at_ms), (1, Some(next + 60_000)));
        assert!(l.begin_resume(T, &s, next + 60_000));
        l.finish_resume(T, s.stopped_ms, Finish::Resumed);
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
    fn stops_survive_a_restart() {
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
        assert!(overdue(&due[0].1, RESET + 60_000 + OVERDUE_MS + 1));
        assert_eq!(
            *l.watch().borrow(),
            vec![Limited {
                title: "fixer".into(),
                resets_at_ms: RESET,
                resume_at_ms: Some(RESET + 60_000),
            }]
        );
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        let bad = Limits::load(dir.path().join("bad.json"), GRACE, START);
        assert!(!bad.enabled() && bad.due(u64::MAX).is_empty());
    }

    #[test]
    fn the_tapped_status_line_at_99_percent_is_reached() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("collie");
        let input = include_str!("../tests/fixtures/claude/statusline-at-99.synthetic.json");
        let session = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
        let reset = 1_738_425_600_000;
        let now = reset - 3_600_000;
        crate::usage::record(&data, input.as_bytes(), now).unwrap();
        let r = crate::usage::last_recorded(&data).unwrap().reached;
        assert_eq!(
            reached(r.get(session), now - 1, now),
            Some((LimitWindow::FiveHour, reset))
        );
        let below = include_str!("../tests/fixtures/statusline.json");
        crate::usage::record(&data, below.as_bytes(), now + 1).unwrap();
        let r = crate::usage::last_recorded(&data).unwrap().reached;
        assert_eq!(reached(r.get(session), now, now + 1), None);
    }

    #[test]
    fn waits_read_as_durations() {
        assert_eq!(wait(1), "1 min");
        assert_eq!(wait(45 * 60_000), "45 min");
        assert_eq!(wait(60 * 60_000), "1 h");
        assert_eq!(wait(125 * 60_000), "2 h 5 min");
        assert_eq!(wait(2 * 86_400_000 + 60_000), "2 days");
        assert_eq!(wait(3 * 86_400_000 - 3_600_000), "2 days 23 h");
        assert_eq!(wait(3 * 86_400_000), "3 days");
    }
}
