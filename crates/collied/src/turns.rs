use std::collections::HashMap;

use protocol::AgentStatus;

use crate::flock;
use crate::herdr::AgentInfo;

/// The shortest turn that alerts when it ends: quick replies never do, and since it equals
/// approvals' `ALERT_GAP` a flapping agent alerts at most once per 30 s per terminal.
pub const MIN_TURN_MS: u64 = 30_000;

/// When each agent's current turn started, to tell which ones just finished one.
#[derive(Default)]
pub struct Turns {
    seen: HashMap<String, (AgentStatus, Option<u64>)>,
}

impl Turns {
    /// The agents whose turn of `MIN_TURN_MS` or more just went from working to done or
    /// idle. A blocked agent never finishes (approvals alert), and a first sight never alerts.
    pub fn observe<'a>(&mut self, agents: &'a [AgentInfo], now_ms: u64) -> Vec<&'a AgentInfo> {
        let mut finished = Vec::new();
        let mut seen = HashMap::with_capacity(agents.len());
        for a in agents {
            let status = flock::status(&a.agent_status);
            let prev = self.seen.get(&a.terminal_id).copied();
            let start = match status {
                AgentStatus::Working | AgentStatus::Blocked => {
                    let kept = match prev {
                        Some((AgentStatus::Working | AgentStatus::Blocked, start)) => start,
                        _ => None,
                    };
                    kept.or((status == AgentStatus::Working).then_some(now_ms))
                }
                AgentStatus::Done | AgentStatus::Idle => {
                    if let Some((AgentStatus::Working, Some(start))) = prev
                        && now_ms.saturating_sub(start) >= MIN_TURN_MS
                    {
                        finished.push(a);
                    }
                    None
                }
                AgentStatus::Unknown => None,
            };
            seen.insert(a.terminal_id.clone(), (status, start));
        }
        self.seen = seen;
        finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(status: &str) -> AgentInfo {
        serde_json::from_value(serde_json::json!({
            "terminal_id": "t1", "workspace_id": "w1", "pane_id": "w1:p1", "agent_status": status,
        }))
        .unwrap()
    }

    /// Feeds `steps` of (status, seconds) and returns how many polls alerted.
    fn run(turns: &mut Turns, steps: &[(&str, u64)]) -> usize {
        steps
            .iter()
            .filter(|(s, at)| !turns.observe(&[agent(s)], at * 1000).is_empty())
            .count()
    }

    #[test]
    fn a_long_turn_alerts_once_when_it_ends() {
        for end in ["done", "idle"] {
            let mut t = Turns::default();
            assert_eq!(
                run(
                    &mut t,
                    &[
                        ("idle", 0),
                        ("working", 1),
                        ("working", 20),
                        (end, 31),
                        (end, 40)
                    ]
                ),
                1,
                "{end}"
            );
        }
    }

    #[test]
    fn short_blocked_and_unseen_turns_stay_quiet() {
        let quiet: [&[(&str, u64)]; 6] = [
            &[("idle", 0), ("working", 1), ("done", 30)],
            &[("idle", 0), ("working", 1), ("blocked", 40)],
            &[("blocked", 0), ("blocked", 40), ("idle", 41)],
            &[("working", 0), ("blocked", 10), ("idle", 50)],
            &[("done", 0), ("idle", 100)],
            &[("working", 0), ("unknown", 20), ("done", 60)],
        ];
        for steps in quiet {
            assert_eq!(run(&mut Turns::default(), steps), 0, "{steps:?}");
        }
    }

    #[test]
    fn blocked_time_counts_toward_the_turn() {
        assert_eq!(
            run(&mut Turns::default(), &[("working", 0), ("done", 31)]),
            1,
            "a turn first seen mid-way is timed from that sight"
        );
        let mut t = Turns::default();
        let steps = [
            ("idle", 0),
            ("working", 1),
            ("blocked", 5),
            ("working", 25),
            ("done", 31),
        ];
        assert_eq!(run(&mut t, &steps), 1);
    }

    #[test]
    fn a_closed_pane_is_forgotten() {
        let mut t = Turns::default();
        run(&mut t, &[("idle", 0), ("working", 1)]);
        assert!(t.observe(&[], 10_000).is_empty());
        assert!(t.seen.is_empty());
        assert_eq!(run(&mut t, &[("done", 60)]), 0);
    }
}
