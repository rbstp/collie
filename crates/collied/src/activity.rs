use std::collections::HashMap;
use std::time::Duration;

use protocol::{ActivityId, AgentStatus, Approval, PushToken, TerminalId};
use serde::Serialize;
use serde_json::json;
use tokio::time::Instant;

use crate::approvals;
use crate::audit::Audit;
use crate::flock::{self, StatusTracker};
use crate::herdr::{AgentInfo, WorkspaceInfo};
use crate::push::{Activity, Alert, Delivery, Push};

pub const MIN_GAP: Duration = Duration::from_secs(2);
pub const REFRESH: Duration = Duration::from_secs(600);
const STALE_AFTER: u64 = 900;
const DISMISS_AFTER: u64 = 60;
const EXPIRES_AFTER: u64 = 300;
/// ActivityKit ends an activity 8 h after it starts, and the phone registers its token
/// after the start and again on every foreground, so an older registration is dead.
pub const MAX_AGE: Duration = Duration::from_secs(8 * 3600);
// Reconciles in a row without the terminal before its activity ends, so a terminal
// polled just before its activity was registered is not taken for gone.
const GONE_AFTER: u8 = 2;
/// 2001-01-01T00:00:00Z in Unix seconds: Swift's default `Date` Codable value, which
/// ActivityKit uses to decode `content-state`, counts seconds from it.
pub const REFERENCE_DATE: u64 = 978_307_200;

/// Plaintext to Apple: labels, status and a count only, never terminal text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContentState {
    pub status: AgentStatus,
    #[serde(rename = "statusSince")]
    pub status_since: i64,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub approvals: u32,
}

pub fn reference_seconds(unix_ms: u64) -> i64 {
    (unix_ms / 1000) as i64 - REFERENCE_DATE as i64
}

pub fn content(
    a: &AgentInfo,
    workspaces: &[WorkspaceInfo],
    pending: &[Approval],
    tracker: &mut StatusTracker,
    now_ms: u64,
) -> Option<ContentState> {
    let agent = flock::map_agent(a, tracker, now_ms)?;
    Some(ContentState {
        status: agent.status,
        status_since: reference_seconds(agent.status_since_ms),
        title: approvals::alert_title(a),
        workspace: Some(approvals::workspace_label(&a.workspace_id, workspaces)),
        approvals: pending
            .iter()
            .filter(|p| p.terminal_id.as_str() == a.terminal_id)
            .count() as u32,
    })
}

/// `urgent` is a change into `blocked`, at most once per `approvals::ALERT_GAP` per
/// activity: priority 10 and an alert that expands the Dynamic Island, with the same
/// plaintext as an approval alert.
pub fn update(state: &ContentState, urgent: bool, now_ms: u64) -> Alert {
    let now = now_ms / 1000;
    let mut aps = json!({
        "timestamp": now,
        "event": "update",
        "content-state": state,
        "stale-date": now + STALE_AFTER,
        "relevance-score": if state.status == AgentStatus::Blocked { 100 } else { 50 },
    });
    if urgent {
        aps["alert"] = json!({
            "title": state.title,
            "body": format!("Blocked in {}", state.workspace.as_deref().unwrap_or("workspace")),
        });
    }
    live(aps, urgent, now)
}

pub fn end(last: Option<&ContentState>, now_ms: u64) -> Alert {
    let now = now_ms / 1000;
    let mut aps = json!({
        "timestamp": now,
        "event": "end",
        "dismissal-date": now + DISMISS_AFTER,
    });
    if let Some(state) = last {
        aps["content-state"] = json!(state);
    }
    live(aps, false, now)
}

fn live(aps: serde_json::Value, urgent: bool, now: u64) -> Alert {
    Alert {
        payload: json!({ "aps": aps }),
        collapse_id: None,
        expiration: Some(now + EXPIRES_AFTER),
        context: None,
        delivery: Delivery::LiveActivity { urgent },
    }
}

struct Tracked {
    token: PushToken,
    sent: Option<ContentState>,
    at: Option<Instant>,
    alerted: Option<Instant>,
    misses: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub activity: Activity,
    pub alert: Alert,
    pub end: bool,
}

/// What each activity last showed, so a push goes out only when its content changes
/// (at most one per `MIN_GAP`) or its stale date needs moving (every `REFRESH`).
#[derive(Default)]
pub struct Live {
    tracked: HashMap<(String, String), Tracked>,
}

impl Live {
    pub fn plan(
        &mut self,
        activities: &[Activity],
        agents: &[AgentInfo],
        workspaces: &[WorkspaceInfo],
        pending: &[Approval],
        tracker: &mut StatusTracker,
        now_ms: u64,
    ) -> Vec<Planned> {
        let now = Instant::now();
        let mut out = Vec::new();
        self.tracked.retain(|(stable_id, activity_id), _| {
            activities
                .iter()
                .any(|a| a.stable_id == *stable_id && a.activity_id.as_str() == activity_id)
        });
        for activity in activities {
            let key = (
                activity.stable_id.clone(),
                activity.activity_id.as_str().to_owned(),
            );
            let fresh = || Tracked {
                token: activity.token.clone(),
                sent: None,
                at: None,
                alerted: None,
                misses: 0,
            };
            let t = self.tracked.entry(key.clone()).or_insert_with(fresh);
            if t.token != activity.token {
                *t = fresh();
            }
            let Some(a) = agents
                .iter()
                .find(|a| a.terminal_id == activity.terminal_id.as_str())
            else {
                t.misses += 1;
                if t.misses >= GONE_AFTER {
                    out.push(Planned {
                        activity: activity.clone(),
                        alert: end(t.sent.as_ref(), now_ms),
                        end: true,
                    });
                    self.tracked.remove(&key);
                }
                continue;
            };
            t.misses = 0;
            let Some(state) = content(a, workspaces, pending, tracker, now_ms) else {
                continue;
            };
            let since_sent = t.at.map(|at| now.duration_since(at));
            let urgent = match &t.sent {
                None => false,
                Some(prev) if *prev != state => {
                    if since_sent.is_some_and(|d| d < MIN_GAP) {
                        continue;
                    }
                    state.status == AgentStatus::Blocked
                        && prev.status != AgentStatus::Blocked
                        && t.alerted
                            .is_none_or(|at| now.duration_since(at) >= approvals::ALERT_GAP)
                }
                Some(_) if since_sent.is_some_and(|d| d >= REFRESH) => false,
                Some(_) => continue,
            };
            out.push(Planned {
                activity: activity.clone(),
                alert: update(&state, urgent, now_ms),
                end: false,
            });
            t.sent = Some(state);
            t.at = Some(now);
            if urgent {
                t.alerted = Some(now);
            }
        }
        out
    }

    /// Sends what `plan` decided; the activity of a terminal that is gone is ended and
    /// its token dropped.
    pub fn observe(
        &mut self,
        push: &Push,
        audit: &Audit,
        agents: &[AgentInfo],
        workspaces: &[WorkspaceInfo],
        pending: &[Approval],
        tracker: &mut StatusTracker,
    ) {
        let now_ms = crate::now_ms();
        let mut activities = push.activities();
        let cutoff = now_ms.saturating_sub(MAX_AGE.as_millis() as u64);
        if activities.iter().any(|a| a.registered_at < cutoff) {
            match push.expire_activities(cutoff) {
                Ok(expired) => {
                    for a in expired {
                        audit.log(
                            "collied",
                            "push.activity_end",
                            Some(&target(&a.terminal_id, &a.activity_id)),
                            "ended: expired",
                        );
                    }
                }
                Err(e) => tracing::error!(error = %e, "could not drop an expired Live Activity"),
            }
            activities.retain(|a| a.registered_at >= cutoff);
        }
        if activities.is_empty() && self.tracked.is_empty() {
            return;
        }
        for p in self.plan(&activities, agents, workspaces, pending, tracker, now_ms) {
            if p.end {
                let a = &p.activity;
                if let Err(e) = push.end_activity(&a.stable_id, &a.activity_id) {
                    tracing::error!(error = %e, "could not drop an ended Live Activity");
                }
                audit.log(
                    "collied",
                    "push.activity_end",
                    Some(&target(&a.terminal_id, &a.activity_id)),
                    "ended: agent gone",
                );
            }
            push.notify_activity(p.activity, p.alert);
        }
    }
}

pub fn target(terminal_id: &TerminalId, activity_id: &ActivityId) -> String {
    format!("{} activity={}", terminal_id.as_str(), activity_id.as_str())
}

#[cfg(test)]
mod tests {
    use protocol::{ApnsEnvironment, ApprovalId, Nonce};
    use serde_json::{Value, json};

    use super::*;

    const T0: u64 = 1_791_028_800_000;

    fn agent(status: &str) -> AgentInfo {
        serde_json::from_value(json!({
            "terminal_id": "term_1", "workspace_id": "w7", "pane_id": "w7:p1",
            "agent": "claude", "name": "api-fixer", "agent_status": status,
            "terminal_title_stripped": "SECRET terminal title",
        }))
        .unwrap()
    }

    fn workspaces() -> Vec<WorkspaceInfo> {
        serde_json::from_value(json!([{
            "workspace_id": "w7", "number": 1, "label": "api", "agent_status": "working",
        }]))
        .unwrap()
    }

    fn activity(token: char) -> Activity {
        Activity {
            stable_id: "nPhone".into(),
            activity_id: ActivityId::new("3F2504E0-4F89-11D3-9A0C-0305E82C3301").unwrap(),
            terminal_id: TerminalId::new("term_1").unwrap(),
            token: PushToken::new(token.to_string().repeat(64)).unwrap(),
            environment: ApnsEnvironment::Production,
            registered_at: 0,
        }
    }

    fn pending() -> Vec<Approval> {
        vec![Approval {
            approval_id: ApprovalId::new("AAAAAAAAAAAAAAAAAAAAAA").unwrap(),
            terminal_id: TerminalId::new("term_1").unwrap(),
            agent_label: "SECRET terminal title".into(),
            workspace_label: "api".into(),
            snippet: "SECRET snippet".into(),
            tool: None,
            options: vec![],
            choices: vec![],
            accepts_input: false,
            has_text_field: false,
            supports_note: false,
            nonce: Nonce::new("N".repeat(43)).unwrap(),
            created_at_ms: T0,
            expires_at_ms: T0 + 600_000,
        }]
    }

    struct Rig {
        live: Live,
        tracker: StatusTracker,
        clock_ms: u64,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                live: Live::default(),
                tracker: StatusTracker::default(),
                clock_ms: T0,
            }
        }

        async fn advance(&mut self, by: Duration) {
            tokio::time::advance(by).await;
            self.clock_ms += by.as_millis() as u64;
        }

        fn plan(
            &mut self,
            activities: &[Activity],
            agents: &[AgentInfo],
            pending: &[Approval],
        ) -> Vec<Planned> {
            self.live.plan(
                activities,
                agents,
                &workspaces(),
                pending,
                &mut self.tracker,
                self.clock_ms,
            )
        }
    }

    fn aps(p: &Planned) -> &Value {
        &p.alert.payload["aps"]
    }

    #[test]
    fn content_state_matches_the_shared_fixture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/protocol/live-activity-content-state.json"
        );
        let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let unix = fixture["status_since_unix"].as_u64().unwrap();
        let mut tracker = StatusTracker::default();
        let state = content(
            &agent("blocked"),
            &workspaces(),
            &pending(),
            &mut tracker,
            unix * 1000,
        )
        .unwrap();
        assert_eq!(json!(state), fixture["content_state"], "{path} is stale");
        assert_eq!(state.status_since, 812_721_600);
        let statuses: Vec<Value> = ["idle", "working", "blocked", "done", "unknown"]
            .iter()
            .map(|s| json!(flock::status(s)))
            .collect();
        assert_eq!(json!(statuses), fixture["statuses"]);
    }

    #[test]
    fn payloads_and_headers() {
        let mut tracker = StatusTracker::default();
        let state = content(
            &agent("blocked"),
            &workspaces(),
            &pending(),
            &mut tracker,
            T0,
        )
        .unwrap();
        let urgent = update(&state, true, T0 + 999);
        let now = T0 / 1000;
        assert_eq!(
            urgent.payload,
            json!({"aps": {
                "timestamp": now,
                "event": "update",
                "content-state": {
                    "status": "blocked", "statusSince": now - REFERENCE_DATE,
                    "title": "api-fixer", "workspace": "api", "approvals": 1,
                },
                "stale-date": now + 900,
                "relevance-score": 100,
                "alert": {"title": "api-fixer", "body": "Blocked in api"},
            }})
        );
        let text = urgent.payload.to_string();
        assert!(!text.contains("SECRET") && !text.contains("NNNN"), "{text}");
        let headers = urgent.headers("dev.rbstp.collie");
        assert_eq!(headers.push_type.to_string(), "liveactivity");
        assert_eq!(headers.topic, "dev.rbstp.collie.push-type.liveactivity");
        assert_eq!(headers.priority, 10);
        assert_eq!(headers.expiration, Some(now + 300));
        assert_eq!(headers.collapse_id, None);

        let quiet = update(&state, false, T0);
        assert!(quiet.payload["aps"].get("alert").is_none());
        assert_eq!(quiet.headers("dev.rbstp.collie").priority, 5);

        let gone = end(Some(&state), T0);
        assert_eq!(gone.payload["aps"]["event"], "end");
        assert_eq!(gone.payload["aps"]["dismissal-date"], now + 60);
        assert_eq!(gone.payload["aps"]["content-state"]["status"], "blocked");
        assert!(gone.payload["aps"].get("stale-date").is_none());
        assert_eq!(gone.headers("dev.rbstp.collie").priority, 5);
        assert!(end(None, T0).payload["aps"].get("content-state").is_none());

        let alert = crate::push::test_alert().headers("dev.rbstp.collie");
        assert_eq!(
            (
                alert.push_type.to_string(),
                alert.topic.as_str(),
                alert.priority
            ),
            ("alert".to_owned(), "dev.rbstp.collie", 10)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn updates_are_coalesced_and_refreshed() {
        let mut rig = Rig::new();
        let acts = [activity('a')];

        let first = rig.plan(&acts, &[agent("working")], &[]);
        assert_eq!(first.len(), 1, "a new activity is synced once, quietly");
        assert_eq!(
            first[0].alert.delivery,
            Delivery::LiveActivity { urgent: false }
        );
        assert!(aps(&first[0]).get("alert").is_none());
        assert!(rig.plan(&acts, &[agent("working")], &[]).is_empty());

        rig.advance(Duration::from_millis(500)).await;
        assert!(
            rig.plan(&acts, &[agent("blocked")], &pending()).is_empty(),
            "within 2 s of the last push"
        );
        rig.advance(Duration::from_millis(1500)).await;
        let blocked = rig.plan(&acts, &[agent("blocked")], &pending());
        assert_eq!(blocked.len(), 1);
        assert_eq!(
            blocked[0].alert.delivery,
            Delivery::LiveActivity { urgent: true }
        );
        let state = &aps(&blocked[0])["content-state"];
        assert_eq!(state["status"], "blocked");
        assert_eq!(state["approvals"], 1);
        assert_eq!(
            state["statusSince"],
            reference_seconds(T0 + 500),
            "since the status was first seen"
        );
        assert_eq!(aps(&blocked[0])["alert"]["body"], "Blocked in api");

        rig.advance(Duration::from_secs(2)).await;
        let cleared = rig.plan(&acts, &[agent("blocked")], &[]);
        assert_eq!(cleared.len(), 1, "the approvals count changed");
        assert_eq!(
            cleared[0].alert.delivery,
            Delivery::LiveActivity { urgent: false },
            "still blocked: no second alert"
        );

        rig.advance(REFRESH - Duration::from_secs(1)).await;
        assert!(rig.plan(&acts, &[agent("blocked")], &[]).is_empty());
        rig.advance(Duration::from_secs(1)).await;
        let refresh = rig.plan(&acts, &[agent("blocked")], &[]);
        assert_eq!(refresh.len(), 1, "the stale date moves every 10 min");
        assert_eq!(
            refresh[0].alert.delivery,
            Delivery::LiveActivity { urgent: false }
        );
        assert_eq!(
            aps(&refresh[0])["stale-date"],
            rig.clock_ms / 1000 + STALE_AFTER
        );

        rig.advance(Duration::from_secs(3)).await;
        let working = rig.plan(&acts, &[agent("working")], &[]);
        assert_eq!(
            working[0].alert.delivery,
            Delivery::LiveActivity { urgent: false }
        );
        rig.advance(Duration::from_secs(3)).await;
        let again = rig.plan(&acts, &[agent("blocked")], &[]);
        assert_eq!(
            again[0].alert.delivery,
            Delivery::LiveActivity { urgent: true }
        );

        let rotated = [activity('b')];
        let resync = rig.plan(&rotated, &[agent("blocked")], &[]);
        assert_eq!(resync.len(), 1, "a new token is synced at once");
        assert_eq!(
            resync[0].alert.delivery,
            Delivery::LiveActivity { urgent: false }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn flapping_alerts_once_per_gap() {
        let mut rig = Rig::new();
        let acts = [activity('a')];
        let step = Duration::from_secs(3);
        let mut urgent = Vec::new();
        assert_eq!(rig.plan(&acts, &[agent("working")], &[]).len(), 1);
        for i in 0..20 {
            rig.advance(step).await;
            let status = if i % 2 == 0 { "blocked" } else { "working" };
            let sent = rig.plan(&acts, &[agent(status)], &[]);
            assert_eq!(sent.len(), 1, "every change is still shown");
            if sent[0].alert.delivery == (Delivery::LiveActivity { urgent: true }) {
                urgent.push(i);
            } else {
                assert!(aps(&sent[0]).get("alert").is_none());
                assert_eq!(sent[0].alert.headers("dev.rbstp.collie").priority, 5);
            }
        }
        assert_eq!(urgent, [0, 10], "one alert per 30 s while flapping");
    }

    #[tokio::test]
    async fn old_registrations_expire_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("push.json");
        let now = crate::now_ms();
        let max_age = MAX_AGE.as_millis() as u64;
        let mut old = activity('a');
        old.registered_at = now - max_age - 1000;
        let mut fresh = activity('b');
        fresh.activity_id = ActivityId::new("FRESH").unwrap();
        fresh.terminal_id = TerminalId::new("term_2").unwrap();
        fresh.registered_at = now - max_age + 60_000;
        let stored = crate::push::Devices {
            devices: vec![],
            activities: vec![old.clone(), fresh.clone()],
        };
        crate::peers::save_json(&path, &stored).unwrap();
        let push = Push::open(path, None, std::sync::Arc::new(|_: &str| true)).unwrap();
        let audit_path = dir.path().join("audit.log");
        let audit = Audit::open(&audit_path).unwrap();
        let mut live = Live::default();
        let mut tracker = StatusTracker::default();
        live.observe(
            &push,
            &audit,
            &[agent("working")],
            &workspaces(),
            &[],
            &mut tracker,
        );
        assert_eq!(push.activities(), [fresh]);
        assert_eq!(live.tracked.len(), 1);
        let log = std::fs::read_to_string(&audit_path).unwrap();
        assert_eq!(log.lines().count(), 1, "{log}");
        assert!(log.contains("ended: expired") && log.contains(old.activity_id.as_str()));
        assert!(!log.contains(old.token.as_str()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_gone_terminal_ends_its_activity() {
        let mut rig = Rig::new();
        let acts = [activity('a')];
        assert!(rig.plan(&acts, &[], &[]).is_empty(), "one miss is not gone");
        rig.advance(Duration::from_secs(1)).await;
        assert_eq!(rig.plan(&acts, &[agent("working")], &[]).len(), 1);
        rig.advance(Duration::from_secs(1)).await;
        assert!(rig.plan(&acts, &[], &[]).is_empty());
        rig.advance(Duration::from_secs(1)).await;
        let gone = rig.plan(&acts, &[], &[]);
        assert_eq!(gone.len(), 1);
        assert!(gone[0].end);
        assert_eq!(aps(&gone[0])["event"], "end");
        assert_eq!(aps(&gone[0])["content-state"]["status"], "working");
        assert_eq!(aps(&gone[0])["dismissal-date"], rig.clock_ms / 1000 + 60);
        assert!(rig.live.tracked.is_empty());

        assert_eq!(rig.plan(&acts, &[agent("working")], &[]).len(), 1);
        assert!(rig.plan(&[], &[agent("working")], &[]).is_empty());
        assert!(
            rig.live.tracked.is_empty(),
            "an ended activity is forgotten"
        );
    }
}
