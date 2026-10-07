use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use apns_h2::PushType;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use collied::activity::Live;
use collied::approvals::Approvals;
use collied::audit::Audit;
use collied::drive::{Authorized, Reply};
use collied::flock::StatusTracker;
use collied::hooks::Report;
use collied::push::{Alert, Delivery, Device, Push, Rejection, Sender};
use futures_util::future::BoxFuture;
use protocol::{
    ActivityId, ApnsEnvironment, Approval, ApprovalDecideParams, ApprovalOutcome, Decision,
    ErrorCode, Event, Nonce, NotificationKey, PendingTool, PromptText, PushToken, Response,
    TerminalId,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::broadcast;

const TERMINAL: &str = "term_0a1b2c3d4e5f60";
const PHONE: &str = "nPHONE";
const LABEL: &str = "Test iPhone";
const TTL: Duration = Duration::from_secs(600);
const SETTLE: Duration = Duration::from_millis(600);

const BASH: &str = include_str!("fixtures/claude/bash.txt");
const QUESTION: &str = include_str!("fixtures/claude/question.txt");
const PLAN: &str = include_str!("fixtures/claude/plan.txt");

// Claude Code 2.1.289 (tmux, 100 columns): a rule splits the options, and the trust
// prompt is unnumbered.
const QUESTION_LIVE: &str = include_str!("fixtures/claude-2.1.289/question.tmux.txt");
const TRUST_LIVE: &str = include_str!("fixtures/claude-2.1.289/trust.tmux.txt");
// Claude Code 2.1.292 in herdr 0.9.3: the same unnumbered trust prompt, "No, exit" first.
const TRUST_292: &str = include_str!("fixtures/claude-2.1.292/trust.detection.txt");
const TRUST_OPTIONS: [&str; 2] = ["No, exit", "Yes, I trust this folder"];

// Claude Code 2.1.289 in herdr 0.9.3, the working directory renamed.
const BASH_LIVE: &str = include_str!("fixtures/claude-2.1.289/bash.detection.txt");
const PLAN_LIVE: &str = include_str!("fixtures/claude-2.1.289/plan.detection.txt");

// Codex CLI and Copilot CLI prompts as herdr rules them; see `prompt::fixtures`.
const OTHER_KINDS: [(&str, &str, &str); 6] = [
    (
        "codex",
        "live_strong_blocker",
        include_str!("fixtures/codex/exec.detection.txt"),
    ),
    (
        "codex",
        "live_strong_blocker",
        include_str!("fixtures/codex/patch.detection.txt"),
    ),
    (
        "codex",
        "trust_directory",
        include_str!("fixtures/codex/trust.detection.txt"),
    ),
    (
        "copilot",
        "selection_blocker",
        include_str!("fixtures/copilot/shell.detection.txt"),
    ),
    (
        "copilot",
        "selection_blocker",
        include_str!("fixtures/copilot/edit.detection.txt"),
    ),
    (
        "copilot",
        "selection_blocker",
        include_str!("fixtures/copilot/trust.detection.txt"),
    ),
];

const MUTATING: [&str; 6] = [
    "agent.prompt",
    "agent.send_keys",
    "agent.start",
    "pane.send_text",
    "pane.send_input",
    "pane.send_keys",
];

struct Herdr {
    agent: Value,
    rule: String,
    text: String,
    calls: Vec<(String, Value)>,
    unblock_on_keys: bool,
    arrows_move: bool,
    tab_amends: bool,
    text_lands: bool,
    tab_moves_on: bool,
}

struct Mock {
    state: Arc<Mutex<Herdr>>,
    socket: PathBuf,
}

impl Mock {
    fn start(dir: &std::path::Path) -> Self {
        let socket = dir.join("herdr.sock");
        let state = Arc::new(Mutex::new(Herdr {
            agent: json!({
                "terminal_id": TERMINAL, "workspace_id": "w7", "pane_id": "w7:p1",
                "tab_id": "w7:t1", "focused": false, "revision": 0,
                "agent": "claude", "name": "api-fixer", "agent_status": "blocked",
                "state_change_seq": 4,
                "agent_session": {"source": "herdr:claude", "agent": "claude", "kind": "id", "value": "sess-1"},
            }),
            rule: "bash_permission_prompt".into(),
            text: BASH.into(),
            calls: Vec::new(),
            unblock_on_keys: true,
            arrows_move: true,
            tab_amends: true,
            text_lands: true,
            tab_moves_on: false,
        }));
        let listener = UnixListener::bind(&socket).unwrap();
        let shared = state.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let state = shared.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut line = String::new();
                    tokio::io::BufReader::new(r)
                        .read_line(&mut line)
                        .await
                        .unwrap();
                    let req: Value = serde_json::from_str(&line).unwrap();
                    let result = answer(&mut state.lock().unwrap(), &req);
                    let resp = json!({"id": req["id"], "result": result});
                    let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
                });
            }
        });
        Self { state, socket }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Herdr) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    fn params(&self, method: &str) -> Vec<Value> {
        self.with(|h| {
            h.calls
                .iter()
                .filter(|(m, _)| m == method)
                .map(|(_, p)| p.clone())
                .collect()
        })
    }

    fn set_status(&self, status: &str) {
        self.with(|h| set_status(h, status));
    }
}

fn set_status(h: &mut Herdr, status: &str) {
    h.agent["agent_status"] = json!(status);
    let seq = h.agent["state_change_seq"].as_u64().unwrap();
    h.agent["state_change_seq"] = json!(seq + 1);
}

fn answer(h: &mut Herdr, req: &Value) -> Value {
    let method = req["method"].as_str().unwrap().to_owned();
    let p = req["params"].clone();
    h.calls.push((method.clone(), p.clone()));
    match method.as_str() {
        "agent.list" => json!({"type": "agent_list", "agents": [h.agent]}),
        "workspace.list" => json!({"type": "workspace_list", "workspaces": [
            {"workspace_id": "w7", "number": 2, "label": "api", "focused": false,
             "pane_count": 1, "tab_count": 1, "active_tab_id": "w7:t1", "agent_status": "blocked"},
        ]}),
        "agent.explain" => json!({"type": "agent_explain", "explain": {
            "agent": "claude", "state": h.agent["agent_status"],
            "matched_rule": {"id": h.rule, "priority": 850, "region": "whole_recent", "state": "blocked"},
            "evaluated_rules": [{"id": h.rule, "matched": true, "evidence": {"region_preview": h.text}}],
        }}),
        "pane.read" => json!({"type": "pane_read", "read": {
            "pane_id": p["pane_id"], "workspace_id": "w7", "tab_id": "w7:t1",
            "source": p["source"], "format": "text", "text": h.text, "revision": 0, "truncated": false,
        }}),
        "agent.send_keys" => {
            for key in p["keys"].as_array().unwrap() {
                match key.as_str().unwrap() {
                    "down" if h.arrows_move => h.text = move_cursor(&h.text, 1),
                    "up" if h.arrows_move => h.text = move_cursor(&h.text, -1),
                    "enter" | "esc" if h.unblock_on_keys => set_status(h, "working"),
                    "tab" if h.tab_moves_on => {
                        let seq = h.agent["state_change_seq"].as_u64().unwrap();
                        h.agent["state_change_seq"] = json!(seq + 1);
                    }
                    "tab" if h.tab_amends => {
                        h.text = relabel(&h.text, |l| {
                            if l == "Yes" {
                                "Yes, and tell Claude what to do next".into()
                            } else if l == "No" {
                                "No, and tell Claude what to do differently".into()
                            } else {
                                l.into()
                            }
                        })
                        .replace(" · Tab to amend", "");
                    }
                    _ => {}
                }
            }
            json!({"type": "ok"})
        }
        "pane.send_text" => {
            let typed = p["text"].as_str().unwrap().to_owned();
            if h.text_lands {
                h.text = relabel(&h.text, |l| {
                    if l.starts_with("Yes, and tell") {
                        format!("Yes, {typed}")
                    } else if l.starts_with("No, and tell") {
                        format!("No, {typed}")
                    } else {
                        format!("{l}{typed}")
                    }
                });
            }
            json!({"type": "ok"})
        }
        other => panic!("unexpected herdr call {other}"),
    }
}

fn move_cursor(text: &str, by: isize) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let options: Vec<usize> = (0..lines.len())
        .filter(|&i| {
            let t = lines[i].trim_start().trim_start_matches('❯').trim_start();
            (t.as_bytes().first().is_some_and(u8::is_ascii_digit) && t[1..].starts_with(". "))
                || TRUST_OPTIONS.contains(&t)
        })
        .collect();
    let at = options
        .iter()
        .position(|&i| lines[i].contains('❯'))
        .unwrap();
    let to = at.saturating_add_signed(by).min(options.len() - 1);
    lines[options[at]] = lines[options[at]].replacen('❯', " ", 1);
    lines[options[to]] = lines[options[to]].replacen("   ", " ❯ ", 1);
    lines.join("\n") + "\n"
}

/// Replaces the label of the option under the cursor.
fn relabel(text: &str, f: impl Fn(&str) -> String) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let line = lines
        .iter_mut()
        .rev()
        .find(|l| l.trim_start().starts_with('❯') && l.contains(". "))
        .unwrap();
    let at = line.find(". ").unwrap() + 2;
    *line = format!("{}{}", &line[..at], f(&line[at..]));
    lines.join("\n") + "\n"
}

#[derive(Default)]
struct Apns {
    sent: Mutex<Vec<(String, Alert)>>,
    live_fails: Mutex<Option<Rejection>>,
    live_attempts: AtomicUsize,
}

impl Sender for Apns {
    fn send<'a>(
        &'a self,
        device: &'a Device,
        alert: &'a Alert,
    ) -> BoxFuture<'a, Result<(), Rejection>> {
        Box::pin(async move {
            if matches!(alert.delivery, Delivery::LiveActivity { .. }) {
                self.live_attempts.fetch_add(1, Ordering::SeqCst);
                if let Some(e) = self.live_fails.lock().unwrap().clone() {
                    return Err(e);
                }
            }
            self.sent
                .lock()
                .unwrap()
                .push((device.token.as_str().to_owned(), alert.clone()));
            Ok(())
        })
    }
}

struct Rig {
    herdr: Mock,
    apns: Arc<Apns>,
    push: Arc<Push>,
    approvals: Arc<Approvals>,
    live: Mutex<(Live, StatusTracker)>,
    events: broadcast::Receiver<Event>,
    audit: PathBuf,
    audit_log: Arc<Audit>,
    _dir: tempfile::TempDir,
}

fn token() -> PushToken {
    PushToken::new("ab".repeat(32)).unwrap()
}

const OTHER: &str = "nOTHER";

fn other_token() -> PushToken {
    PushToken::new("cd".repeat(32)).unwrap()
}

fn activity_token() -> PushToken {
    PushToken::new("ef".repeat(80)).unwrap()
}

impl Rig {
    async fn start(ttl: Duration) -> Self {
        Self::start_with_budget(ttl, None).await
    }

    async fn start_with_budget(ttl: Duration, budget: Option<Duration>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let herdr = Mock::start(dir.path());
        let apns = Arc::new(Apns::default());
        let push = Push::open(
            dir.path().join("push.json"),
            Some(apns.clone()),
            Arc::new(|_| true),
        )
        .unwrap();
        push.register(PHONE, token(), ApnsEnvironment::Sandbox, key())
            .unwrap();
        let audit = dir.path().join("audit.log");
        let audit_log = Arc::new(Audit::open(&audit).unwrap());
        let (tx, events) = broadcast::channel(64);
        let mut approvals = Approvals::new(
            herdr.socket.clone(),
            "nMAC".into(),
            tx,
            audit_log.clone(),
            Some(push.clone()),
        )
        .with_timing(ttl, SETTLE);
        if let Some(budget) = budget {
            approvals = approvals.with_budget(budget);
        }
        let approvals = Arc::new(approvals);
        Self {
            herdr,
            apns,
            push,
            approvals,
            live: Mutex::new((Live::default(), StatusTracker::default())),
            events,
            audit,
            audit_log,
            _dir: dir,
        }
    }

    fn follow(&self) {
        self.push
            .register_activity(
                PHONE,
                ActivityId::new("ACT-1").unwrap(),
                TerminalId::new(TERMINAL).unwrap(),
                activity_token(),
                true,
            )
            .unwrap();
    }

    /// One reconcile as the server runs it: approvals, then Live Activities.
    async fn tick(&self) {
        let agents = collied::herdr::agent_list(&self.herdr.socket)
            .await
            .unwrap();
        let workspaces = collied::herdr::workspace_list(&self.herdr.socket)
            .await
            .unwrap();
        self.approvals.observe(&agents, &workspaces).await;
        let pending = self.approvals.pending();
        let (live, tracker) = &mut *self.live.lock().unwrap();
        live.observe(
            &self.push,
            &self.audit_log,
            &agents,
            &workspaces,
            &pending,
            tracker,
        );
    }

    async fn ticked_needed(&mut self) -> Approval {
        self.tick().await;
        loop {
            match self.event().await {
                Event::ApprovalNeeded { approval } => return approval,
                Event::ApprovalResolved { .. } => {}
                other => panic!("expected approval.needed, got {other:?}"),
            }
        }
    }

    /// Everything sent so far, once the queue has drained.
    async fn sent(&self) -> Vec<(String, Alert)> {
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.apns.sent.lock().unwrap().clone()
    }

    async fn observe(&self) {
        let agents = collied::herdr::agent_list(&self.herdr.socket)
            .await
            .unwrap();
        let workspaces = collied::herdr::workspace_list(&self.herdr.socket)
            .await
            .unwrap();
        self.approvals.observe(&agents, &workspaces).await;
    }

    async fn event(&mut self) -> Event {
        tokio::time::timeout(Duration::from_secs(2), self.events.recv())
            .await
            .expect("no event")
            .unwrap()
    }

    fn no_event(&mut self) {
        assert!(
            matches!(
                self.events.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ),
            "unexpected event"
        );
    }

    async fn needed(&mut self) -> Approval {
        self.observe().await;
        loop {
            match self.event().await {
                Event::ApprovalNeeded { approval } => return approval,
                Event::ApprovalResolved { .. } => {}
                other => panic!("expected approval.needed, got {other:?}"),
            }
        }
    }

    async fn decide(&self, a: &Approval, decision: Decision, nonce: &Nonce) -> Reply {
        self.decide_as(a, decision, nonce, Arc::new(|| true)).await
    }

    async fn decide_as(
        &self,
        a: &Approval,
        decision: Decision,
        nonce: &Nonce,
        auth: Authorized,
    ) -> Reply {
        let p = ApprovalDecideParams {
            approval_id: a.approval_id.clone(),
            decision,
            choice: None,
            nonce: nonce.clone(),
            note: None,
        };
        self.approvals.decide(LABEL, PHONE, p, &auth).await
    }

    async fn decide_with_note(
        &self,
        a: &Approval,
        decision: Decision,
        note: &str,
        auth: Authorized,
    ) -> Reply {
        let p = ApprovalDecideParams {
            approval_id: a.approval_id.clone(),
            decision,
            choice: None,
            nonce: a.nonce.clone(),
            note: Some(PromptText::new(note).unwrap()),
        };
        self.approvals.decide(LABEL, PHONE, p, &auth).await
    }

    async fn choose(&self, a: &Approval, choice: u8, nonce: &Nonce) -> Reply {
        let p = ApprovalDecideParams {
            approval_id: a.approval_id.clone(),
            decision: Decision::Choose,
            choice: Some(choice),
            nonce: nonce.clone(),
            note: None,
        };
        let auth: Authorized = Arc::new(|| true);
        self.approvals.decide(LABEL, PHONE, p, &auth).await
    }

    async fn alerts(&self, n: usize) -> Vec<(String, Alert)> {
        for _ in 0..200 {
            if self.apns.sent.lock().unwrap().len() >= n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        self.apns.sent.lock().unwrap().clone()
    }

    fn audit(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.audit)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn mutations(&self) -> Vec<String> {
        self.herdr.with(|h| {
            h.calls
                .iter()
                .map(|(m, _)| m.clone())
                .filter(|m| MUTATING.contains(&m.as_str()))
                .collect()
        })
    }
}

fn wrong(n: &Nonce) -> Nonce {
    let flipped = if n.as_str().starts_with('A') {
        'B'
    } else {
        'A'
    };
    Nonce::new(format!("{flipped}{}", &n.as_str()[1..])).unwrap()
}

fn resolved(reply: Reply) -> ApprovalOutcome {
    match reply {
        Ok(Response::ApprovalResolved { outcome, .. }) => outcome,
        other => panic!("expected a resolution, got {other:?}"),
    }
}

fn key() -> NotificationKey {
    NotificationKey::new(URL_SAFE_NO_PAD.encode([7u8; 32])).unwrap()
}

/// The phone's side: `enc` opened with the device key, AAD = approval_id.
fn open_context(payload: &Value) -> Value {
    let combined = STANDARD.decode(payload["enc"].as_str().unwrap()).unwrap();
    let (nonce, sealed) = combined.split_at(12);
    let plain = ChaCha20Poly1305::new_from_slice(&[7u8; 32])
        .unwrap()
        .decrypt(
            nonce.try_into().unwrap(),
            Payload {
                msg: sealed,
                aad: payload["approval_id"].as_str().unwrap().as_bytes(),
            },
        )
        .unwrap();
    serde_json::from_slice(&plain).unwrap()
}

fn code(reply: Reply) -> ErrorCode {
    reply.expect_err("expected an error").0
}

#[tokio::test]
async fn blocked_agent_gets_one_approval_and_one_alert() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    assert_eq!(a.terminal_id.as_str(), TERMINAL);
    assert_eq!(a.agent_label, "api-fixer");
    assert_eq!(a.workspace_label, "api");
    assert_eq!(
        a.options,
        [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
    );
    assert_eq!(
        a.snippet,
        "Bash command\nrm -rf build\nRemove the build directory\nDo you want to proceed?"
    );
    assert_eq!(a.tool, None);
    let labels: Vec<(u8, &str, bool)> = a
        .choices
        .iter()
        .map(|c| (c.index, c.label.as_str(), c.current))
        .collect();
    assert_eq!(
        labels,
        [
            (0, "Yes", true),
            (
                1,
                "Yes, and don't ask again for rm commands in /Users/me/src/app",
                false
            ),
            (2, "No, and tell Claude what to do differently (esc)", false),
        ]
    );
    assert_eq!(a.expires_at_ms - a.created_at_ms, TTL.as_millis() as u64);
    assert_eq!(rig.approvals.pending(), std::slice::from_ref(&a));

    let alerts = rig.alerts(1).await;
    assert_eq!(alerts.len(), 1);
    let (to, alert) = &alerts[0];
    assert_eq!(to, token().as_str());
    assert_eq!(
        open_context(&alert.payload),
        json!({"v": 1, "body": "Bash: rm -rf build\nRemove the build directory"})
    );
    let mut clear = alert.payload.clone();
    clear.as_object_mut().unwrap().remove("enc");
    assert_eq!(
        clear,
        json!({
            "aps": {
                "alert": {"title": "api-fixer", "body": "Blocked in api"},
                "category": "APPROVAL",
                "thread-id": TERMINAL,
                "mutable-content": 1,
            },
            "approval_id": a.approval_id.as_str(),
            "node_id": "nMAC",
        })
    );
    assert_eq!(alert.collapse_id.as_deref(), Some(TERMINAL));
    assert_eq!(alert.expiration, Some(a.expires_at_ms / 1000));
    let wire = alert.payload.to_string();
    assert!(!wire.contains(a.nonce.as_str()) && !wire.contains("rm -rf"));

    rig.observe().await;
    rig.no_event();
    assert_eq!(rig.alerts(2).await.len(), 1);
    assert_eq!(
        rig.herdr.params("agent.explain")[0],
        json!({"target": "w7:p1"})
    );
    assert_eq!(
        rig.herdr.params("pane.read")[0],
        json!({"pane_id": "w7:p1", "source": "detection", "format": "text"})
    );
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn decide_sends_the_mapped_keys_once() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    let outcome = resolved(rig.decide(&a, Decision::Approve, &a.nonce).await);
    assert_eq!(
        outcome,
        ApprovalOutcome::Applied {
            decision: Decision::Approve,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["enter"]})]
    );
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Applied { .. } }
            if approval_id == a.approval_id
    ));
    assert!(rig.approvals.pending().is_empty());

    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::ApprovalAlreadyResolved
    );
    assert_eq!(rig.herdr.params("agent.send_keys").len(), 1);

    rig.herdr.set_status("blocked");
    let b = rig.needed().await;
    assert_ne!(b.approval_id, a.approval_id);
    assert_ne!(b.nonce, a.nonce);
    assert_eq!(rig.alerts(2).await.len(), 2, "a new prompt alerts again");
    resolved(rig.decide(&b, Decision::Deny, &b.nonce).await);
    assert_eq!(
        rig.herdr.params("agent.send_keys")[1],
        json!({"target": "w7:p1", "keys": ["esc"]})
    );

    let results: Vec<String> = rig
        .audit()
        .iter()
        .map(|e| {
            assert_eq!(e["method"], "approval.decide");
            assert_eq!(e["peer"], LABEL);
            e["result"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        results,
        [
            format!("approve: applied terminal={TERMINAL} keys=enter"),
            "approve: rejected: replayed".to_owned(),
            format!("deny: applied terminal={TERMINAL} keys=esc"),
        ]
    );
    assert_eq!(rig.audit()[0]["target"], a.approval_id.as_str());
}

#[tokio::test]
async fn expired_approvals_are_rejected_and_reissued_with_a_fresh_alert() {
    let mut rig = Rig::start(Duration::from_millis(200)).await;
    let a = rig.needed().await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::ApprovalExpired
    );
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved {
            outcome: ApprovalOutcome::Expired,
            ..
        }
    ));
    let b = rig.needed().await;
    assert_ne!(b.approval_id, a.approval_id);
    let alerts = rig.alerts(2).await;
    assert_eq!(
        alerts.len(),
        2,
        "the reissued approval replaces the dead alert"
    );
    assert_eq!(alerts[1].1.payload["approval_id"], b.approval_id.as_str());
    assert_eq!(alerts[1].1.collapse_id, alerts[0].1.collapse_id);

    tokio::time::sleep(Duration::from_millis(250)).await;
    rig.observe().await;
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Expired } if approval_id == b.approval_id
    ));
    assert!(rig.mutations().is_empty());
    assert!(rig.audit()[0]["result"] == "approve: rejected: expired");
}

#[tokio::test]
async fn a_changed_prompt_supersedes_the_approval() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    rig.herdr
        .with(|h| h.text = BASH.replace("rm -rf build", "rm -rf ~"));
    let outcome = resolved(rig.decide(&a, Decision::Approve, &a.nonce).await);
    assert_eq!(outcome, ApprovalOutcome::Superseded);
    assert!(rig.mutations().is_empty());
    assert_eq!(
        rig.audit()[0]["result"],
        "approve: superseded: fingerprint mismatch"
    );

    let b = rig.needed().await;
    assert!(b.snippet.contains("rm -rf ~"));
    assert_eq!(rig.alerts(2).await.len(), 2);
    rig.herdr.with(|h| {
        h.text = h
            .text
            .replace(" ❯ 1. Yes", "   1. Yes")
            .replace("   2. Yes,", " ❯ 2. Yes,");
    });
    assert_eq!(
        resolved(rig.decide(&b, Decision::Approve, &b.nonce).await),
        ApprovalOutcome::Superseded,
        "the cursor moved on the Mac"
    );
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn a_hook_report_names_only_the_prompt_that_shows_it() {
    let mut rig = Rig::start(TTL).await;
    let report = |cmd: &str| {
        Report::new(
            "Bash",
            vec![cmd.into(), "Remove the build directory".into()],
        )
        .unwrap()
    };
    let named = |cmd: &str| {
        Some(PendingTool {
            name: "Bash".into(),
            summary: format!("{cmd}\nRemove the build directory"),
        })
    };
    rig.approvals.hook("sess-2".into(), report("rm -rf build"));
    rig.approvals.hook("sess-1".into(), report("rm -rf"));
    let a = rig.needed().await;
    assert_eq!(
        a.tool, None,
        "a report for another call, or another session, names nothing"
    );

    rig.approvals.hook("sess-1".into(), report("rm -rf build"));
    rig.herdr
        .with(|h| h.text = BASH.replace("Do you want to proceed?", "Do you want to proceed now?"));
    let b = rig.needed().await;
    assert_eq!(
        b.tool,
        named("rm -rf build"),
        "a report arriving after the dialog"
    );

    let mut rig = Rig::start(TTL).await;
    rig.approvals.hook("sess-1".into(), report("rm -rf build"));
    let a = rig.needed().await;
    assert_eq!(a.tool, named("rm -rf build"));
    assert_eq!(
        open_context(&rig.alerts(1).await[0].1.payload),
        json!({"v": 1, "body": "Bash: rm -rf build\nRemove the build directory"})
    );
    rig.herdr
        .with(|h| h.text = BASH.replace("rm -rf build", "rm -rf dist"));
    let c = rig.needed().await;
    assert_eq!(
        c.tool, None,
        "a report never names a later prompt with another call"
    );
}

#[tokio::test]
async fn every_attempt_burns_the_nonce() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &wrong(&a.nonce)).await),
        ErrorCode::ApprovalNonceMismatch
    );
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::ApprovalAlreadyResolved
    );
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved {
            outcome: ApprovalOutcome::Superseded,
            ..
        }
    ));

    rig.herdr.with(|h| {
        h.text = BASH.replace(
            "   2. Yes, and don't ask again for rm commands in /Users/me/src/app\n   3. No",
            "   2. No",
        );
    });
    let b = rig.needed().await;
    assert_eq!(b.options, [Decision::Approve, Decision::Deny]);
    assert_eq!(
        code(rig.decide(&b, Decision::ApproveAlways, &b.nonce).await),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        code(rig.decide(&b, Decision::Approve, &b.nonce).await),
        ErrorCode::ApprovalAlreadyResolved
    );

    let c = rig.needed().await;
    let revoked: Authorized = Arc::new(|| false);
    assert_eq!(
        code(
            rig.decide_as(&c, Decision::Approve, &c.nonce, revoked)
                .await
        ),
        ErrorCode::NotPaired
    );
    assert!(rig.mutations().is_empty());
    let results: Vec<Value> = rig.audit().iter().map(|e| e["result"].clone()).collect();
    assert_eq!(
        results,
        [
            "approve: rejected: bad nonce",
            "approve: rejected: replayed",
            "approve_always: rejected: decision not offered",
            "approve: rejected: replayed",
            "approve: rejected: peer no longer authorized",
        ]
    );
}

#[tokio::test]
async fn leaving_blocked_without_a_decision_supersedes() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    rig.herdr.set_status("working");
    rig.observe().await;
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Superseded } if approval_id == a.approval_id
    ));
    assert!(rig.approvals.pending().is_empty());
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::ApprovalAlreadyResolved
    );
}

#[tokio::test]
async fn the_pending_count_follows_every_approval() {
    let mut rig = Rig::start(TTL).await;
    let mut count = rig.approvals.watch_pending();
    assert_eq!(*count.borrow_and_update(), 0);
    let a = rig.needed().await;
    assert!(count.has_changed().unwrap());
    assert_eq!(*count.borrow_and_update(), 1);
    rig.observe().await;
    assert!(!count.has_changed().unwrap(), "no change, no update");
    resolved(rig.decide(&a, Decision::Approve, &a.nonce).await);
    assert_eq!(*count.borrow_and_update(), 0);

    rig.herdr.set_status("blocked");
    rig.needed().await;
    assert_eq!(*count.borrow_and_update(), 1);
    rig.herdr.set_status("working");
    rig.observe().await;
    assert_eq!(*count.borrow_and_update(), 0, "superseded");
}

#[tokio::test]
async fn arrows_are_seen_on_the_target_before_enter() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    let outcome = resolved(rig.decide(&a, Decision::ApproveAlways, &a.nonce).await);
    assert_eq!(
        outcome,
        ApprovalOutcome::Applied {
            decision: Decision::ApproveAlways,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
    assert_eq!(
        rig.audit()[0]["result"],
        format!("approve_always: applied terminal={TERMINAL} keys=down,enter")
    );
}

#[tokio::test]
async fn lost_arrows_never_confirm() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| h.arrows_move = false);
    let a = rig.needed().await;
    let outcome = resolved(rig.decide(&a, Decision::ApproveAlways, &a.nonce).await);
    assert_eq!(outcome, ApprovalOutcome::Superseded);
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["down"]})]
    );
    assert_eq!(
        rig.audit()[0]["result"],
        format!("approve_always: superseded: cursor not on target terminal={TERMINAL} keys=down")
    );
    assert_eq!(
        rig.herdr.with(|h| h.agent["agent_status"].clone()),
        "blocked"
    );
}

#[tokio::test]
async fn keys_without_effect_are_unconfirmed() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| h.unblock_on_keys = false);
    let a = rig.needed().await;
    let outcome = resolved(rig.decide(&a, Decision::Approve, &a.nonce).await);
    assert_eq!(
        outcome,
        ApprovalOutcome::Unconfirmed {
            decision: Decision::Approve,
            by: LABEL.into()
        }
    );
    assert_eq!(rig.herdr.params("agent.send_keys").len(), 1);
}

#[tokio::test]
async fn unknown_prompts_get_a_notification_without_actions() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| h.rule = "mcp_elicitation_prompt".into());
    let a = rig.needed().await;
    assert!(a.options.is_empty());
    assert!(a.snippet.contains("Do you want to proceed?"));
    let alerts = rig.alerts(1).await;
    assert!(alerts[0].1.payload["aps"].get("category").is_none());
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::InvalidParams
    );
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn codex_and_copilot_prompts_are_answered_in_the_terminal() {
    for (kind, rule, text) in OTHER_KINDS {
        let mut rig = Rig::start(TTL).await;
        rig.follow();
        rig.herdr.with(|h| {
            h.agent["agent"] = json!(kind);
            h.agent["agent_session"] = json!({"source": format!("herdr:{kind}"), "agent": kind, "kind": "id", "value": "sess-1"});
            h.rule = rule.into();
            h.text = text.into();
        });
        let a = rig.ticked_needed().await;
        assert!(
            a.options.is_empty() && a.choices.is_empty() && a.tool.is_none(),
            "{kind} {rule}"
        );
        assert!(!a.accepts_input && !a.has_text_field && !a.supports_note);
        let sent = rig.sent().await;
        let alerts = alerting(&sent);
        assert_eq!(alerts.len(), 1, "{sent:?}");
        assert_eq!(alerts[0].0, token().as_str(), "not on the activity");
        assert_eq!(alerts[0].1.delivery, Delivery::Alert);
        assert!(alerts[0].1.payload["aps"].get("category").is_none());
        assert!(sent.iter().all(|(_, s)| {
            s.payload["aps"]["content-state"]
                .get("approvalId")
                .is_none()
        }));

        assert_eq!(
            code(rig.decide(&a, Decision::Approve, &a.nonce).await),
            ErrorCode::InvalidParams
        );
        let b = rig.ticked_needed().await;
        assert_eq!(
            code(rig.decide(&b, Decision::Deny, &b.nonce).await),
            ErrorCode::InvalidParams
        );
        let c = rig.ticked_needed().await;
        assert_eq!(
            code(rig.choose(&c, 0, &c.nonce).await),
            ErrorCode::InvalidParams
        );
        assert!(rig.mutations().is_empty(), "{kind} {rule}");
    }
}

#[tokio::test]
async fn a_new_codex_prompt_under_a_still_blocked_agent_supersedes_the_old_one() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.agent["agent"] = json!("codex");
        h.rule = OTHER_KINDS[0].1.into();
        h.text = OTHER_KINDS[0].2.into();
    });
    let a = rig.needed().await;
    assert!(a.snippet.contains("cargo test"), "{}", a.snippet);
    rig.observe().await;
    rig.no_event();

    rig.herdr.with(|h| h.text = OTHER_KINDS[1].2.into());
    rig.observe().await;
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Superseded }
            if approval_id == a.approval_id
    ));
    let Event::ApprovalNeeded { approval: b } = rig.event().await else {
        panic!("expected approval.needed");
    };
    assert!(!b.snippet.contains("cargo test"), "{}", b.snippet);
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn decide_attempts_are_rate_limited() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    let mut codes = Vec::new();
    for _ in 0..6 {
        codes.push(code(rig.decide(&a, Decision::Deny, &wrong(&a.nonce)).await));
    }
    assert_eq!(codes[0], ErrorCode::ApprovalNonceMismatch);
    assert_eq!(codes[5], ErrorCode::RateLimited);
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn enter_is_never_sent_once_the_budget_has_passed() {
    let budget = Duration::from_millis(500);
    let mut rig = Rig::start_with_budget(TTL, Some(budget)).await;
    let a = rig.needed().await;
    let checks = Arc::new(AtomicUsize::new(0));
    let seen = checks.clone();
    // The second check runs once the cursor is seen on the target, just before Enter.
    let slow: Authorized = Arc::new(move || {
        if seen.fetch_add(1, Ordering::SeqCst) == 1 {
            std::thread::sleep(budget + Duration::from_millis(100));
        }
        true
    });
    let outcome = resolved(
        rig.decide_as(&a, Decision::ApproveAlways, &a.nonce, slow)
            .await,
    );
    assert_eq!(outcome, ApprovalOutcome::Superseded);
    assert_eq!(checks.load(Ordering::SeqCst), 2);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["down"]})]
    );
    assert_eq!(
        rig.audit()[0]["result"],
        format!("approve_always: superseded: herdr too slow terminal={TERMINAL} keys=down")
    );
}

#[tokio::test]
async fn reissues_cannot_flood_the_phone() {
    let mut rig = Rig::start(TTL).await;
    let mut a = rig.needed().await;
    for _ in 0..3 {
        assert_eq!(
            code(rig.decide(&a, Decision::Deny, &wrong(&a.nonce)).await),
            ErrorCode::ApprovalNonceMismatch
        );
        let b = rig.needed().await;
        assert_ne!(b.approval_id, a.approval_id, "sessions get every reissue");
        a = b;
    }
    assert_eq!(
        rig.alerts(2).await.len(),
        1,
        "the same prompt reissued within 30 s does not alert again"
    );
    assert_eq!(rig.approvals.pending(), std::slice::from_ref(&a));

    rig.herdr.set_status("blocked");
    let b = rig.needed().await;
    let alerts = rig.alerts(2).await;
    assert_eq!(alerts.len(), 2, "a new prompt always alerts");
    assert_eq!(alerts[1].1.payload["approval_id"], b.approval_id.as_str());
    assert!(rig.mutations().is_empty());
}

fn clears(sent: &[(String, Alert)]) -> Vec<(String, Alert)> {
    sent.iter()
        .filter(|(_, a)| a.delivery == Delivery::Background)
        .cloned()
        .collect()
}

#[tokio::test]
async fn an_alert_is_cleared_once_the_agent_moves_on() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    rig.observe().await;
    assert!(clears(&rig.sent().await).is_empty(), "still blocked");

    rig.herdr.set_status("working");
    rig.observe().await;
    rig.observe().await;
    let sent = rig.sent().await;
    let cleared = clears(&sent);
    assert_eq!(cleared.len(), 1, "{sent:?}");
    let (to, clear) = &cleared[0];
    assert_eq!(to, token().as_str());
    assert_eq!(
        clear.payload,
        json!({
            "aps": {"content-available": 1},
            "approval_id": a.approval_id.as_str(),
            "node_id": "nMAC",
        })
    );
    let headers = clear.headers("dev.rbstp.collie");
    assert_eq!(headers.push_type, PushType::Background);
    assert_eq!(headers.topic, "dev.rbstp.collie");
    assert_eq!(headers.priority, 5);
    assert_eq!(headers.expiration, Some(0));
    assert_eq!(headers.collapse_id, None);
    assert_eq!(sent.last(), cleared.last());
}

#[tokio::test]
async fn a_clear_names_only_the_alerted_approval() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    code(rig.decide(&a, Decision::Deny, &wrong(&a.nonce)).await);
    let b = rig.needed().await;
    assert_eq!(alerting(&rig.sent().await).len(), 1, "b is a quiet reissue");
    rig.herdr.set_status("working");
    rig.observe().await;
    let cleared = clears(&rig.sent().await);
    assert_eq!(cleared.len(), 1);
    assert_eq!(
        cleared[0].1.payload["approval_id"],
        a.approval_id.as_str(),
        "the phone still shows a's alert, never b's"
    );
    assert_ne!(a.approval_id, b.approval_id);

    // A decision from the phone clears too: the watch and the Live Activity decide
    // without removing the phone's alert.
    rig.herdr.set_status("blocked");
    let c = rig.needed().await;
    resolved(rig.decide(&c, Decision::Approve, &c.nonce).await);
    rig.observe().await;
    let cleared = clears(&rig.sent().await);
    assert_eq!(cleared.len(), 2);
    assert_eq!(cleared[1].1.payload["approval_id"], c.approval_id.as_str());

    let mut rig = Rig::start(Duration::from_millis(200)).await;
    rig.needed().await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    rig.needed().await;
    let sent = rig.sent().await;
    assert_eq!(alerting(&sent).len(), 2);
    assert!(
        clears(&sent).is_empty(),
        "a fresh alert replaces an expired one"
    );
}

fn alerting(sent: &[(String, Alert)]) -> Vec<(String, Alert)> {
    sent.iter()
        .filter(|(_, a)| a.payload["aps"].get("alert").is_some())
        .cloned()
        .collect()
}

/// The widget's side: `enc` from the content-state opened with the device key, AAD = approvalId.
fn open_offered(state: &Value) -> Value {
    open_context(&json!({"enc": state["enc"], "approval_id": state["approvalId"]}))
}

#[tokio::test]
async fn a_followed_terminal_alerts_on_its_activity_only() {
    let mut rig = Rig::start(TTL).await;
    rig.push
        .register(OTHER, other_token(), ApnsEnvironment::Sandbox, key())
        .unwrap();
    rig.push
        .register_activity(
            OTHER,
            ActivityId::new("ACT-2").unwrap(),
            TerminalId::new("term_elsewhere").unwrap(),
            PushToken::new("12".repeat(80)).unwrap(),
            true,
        )
        .unwrap();
    rig.follow();
    let a = rig.ticked_needed().await;
    let sent = rig.sent().await;
    let alerts = alerting(&sent);
    assert_eq!(alerts.len(), 2, "{sent:?}");
    let (to, live) = alerts
        .iter()
        .find(|(_, a)| a.delivery == (Delivery::LiveActivity { urgent: true }))
        .unwrap();
    assert_eq!(to, activity_token().as_str());
    assert_eq!(live.headers("dev.rbstp.collie").priority, 10);
    let aps = &live.payload["aps"];
    assert_eq!(
        aps["alert"],
        json!({"title": "api-fixer", "body": "Blocked in api"})
    );
    let state = &aps["content-state"];
    assert_eq!(state["status"], "blocked");
    assert_eq!(state["approvalId"], a.approval_id.as_str());
    assert_eq!(
        open_offered(state),
        json!({"v": 1, "body": "Bash: rm -rf build\nRemove the build directory"})
    );
    let wire = live.payload.to_string();
    assert!(
        !wire.contains("rm -rf") && !wire.contains(a.nonce.as_str()),
        "{wire}"
    );
    let (to, regular) = alerts
        .iter()
        .find(|(_, a)| a.delivery == Delivery::Alert)
        .unwrap();
    assert_eq!(
        to,
        other_token().as_str(),
        "a device that does not follow it"
    );
    assert_eq!(regular.payload["approval_id"], a.approval_id.as_str());
    assert!(
        !sent.iter().any(|(t, _)| t == token().as_str()),
        "no approval notification for the follower"
    );

    for _ in 0..3 {
        rig.tick().await;
    }
    assert_eq!(
        alerting(&rig.sent().await).len(),
        2,
        "exactly one alert per approval"
    );

    assert_eq!(
        code(rig.decide(&a, Decision::Deny, &wrong(&a.nonce)).await),
        ErrorCode::ApprovalNonceMismatch
    );
    let b = rig.ticked_needed().await;
    let sent = rig.sent().await;
    assert_eq!(
        alerting(&sent).len(),
        2,
        "a reissue within 30 s stays quiet"
    );
    let (to, quiet) = sent.last().unwrap();
    assert_eq!(to, activity_token().as_str());
    assert_eq!(quiet.delivery, Delivery::LiveActivity { urgent: false });
    let state = &quiet.payload["aps"]["content-state"];
    assert_eq!(state["approvalId"], b.approval_id.as_str());
    assert_eq!(open_offered(state)["v"], 1);

    assert!(matches!(
        resolved(rig.decide(&b, Decision::Approve, &b.nonce).await),
        ApprovalOutcome::Applied { .. }
    ));
    tokio::time::sleep(collied::activity::MIN_GAP).await;
    rig.tick().await;
    let sent = rig.sent().await;
    let (to, cleared) = sent.last().unwrap();
    assert_eq!(to, activity_token().as_str());
    assert_eq!(
        cleared.delivery,
        Delivery::LiveActivity { urgent: true },
        "the update that drops the buttons goes at priority 10"
    );
    assert!(cleared.payload["aps"].get("alert").is_none());
    let state = &cleared.payload["aps"]["content-state"];
    assert_eq!(state["status"], "working");
    assert!(state.get("approvalId").is_none() && state.get("enc").is_none());
    assert_eq!(alerting(&sent).len(), 2);
}

#[tokio::test]
async fn a_failed_activity_alert_falls_back_to_the_notification() {
    for rejection in [
        Rejection::Unregistered,
        Rejection::Other("503 ServiceUnavailable".into()),
    ] {
        let mut rig = Rig::start(TTL).await;
        rig.follow();
        *rig.apns.live_fails.lock().unwrap() = Some(rejection.clone());
        let a = rig.ticked_needed().await;
        let sent = rig.sent().await;
        assert_eq!(rig.apns.live_attempts.load(Ordering::SeqCst), 1);
        assert_eq!(sent.len(), 1, "{rejection}");
        let (to, alert) = &sent[0];
        assert_eq!(to, token().as_str());
        assert_eq!(alert.delivery, Delivery::Alert);
        assert_eq!(alert.payload["approval_id"], a.approval_id.as_str());
        assert_eq!(open_context(&alert.payload)["v"], 1);
        assert_eq!(
            rig.push.activities().is_empty(),
            rejection == Rejection::Unregistered,
            "only a dead token is dropped"
        );

        for _ in 0..3 {
            rig.tick().await;
        }
        assert_eq!(alerting(&rig.sent().await).len(), 1, "{rejection}");
    }
}

#[tokio::test]
async fn an_activity_without_approvals_keeps_the_notification() {
    let mut rig = Rig::start(TTL).await;
    rig.push
        .register_activity(
            PHONE,
            ActivityId::new("ACT-1").unwrap(),
            TerminalId::new(TERMINAL).unwrap(),
            activity_token(),
            false,
        )
        .unwrap();
    let a = rig.ticked_needed().await;
    let sent = rig.sent().await;
    let alerts = alerting(&sent);
    assert_eq!(alerts.len(), 1, "{sent:?}");
    assert_eq!(alerts[0].0, token().as_str());
    assert_eq!(alerts[0].1.delivery, Delivery::Alert);
    assert_eq!(alerts[0].1.payload["approval_id"], a.approval_id.as_str());
    let (to, live) = sent.last().unwrap();
    assert_eq!(
        to,
        activity_token().as_str(),
        "the activity still shows the status"
    );
    let state = &live.payload["aps"]["content-state"];
    assert_eq!(state["status"], "blocked");
    assert!(state.get("approvalId").is_none() && state.get("enc").is_none());
}

fn choice_labels(a: &Approval) -> Vec<(u8, String, bool)> {
    a.choices
        .iter()
        .map(|c| (c.index, c.label.clone(), c.current))
        .collect()
}

#[tokio::test]
async fn a_question_is_answered_by_choosing_an_option() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = QUESTION.into();
    });
    let a = rig.needed().await;
    assert!(a.options.is_empty());
    assert_eq!(
        choice_labels(&a),
        [
            (0, "SQLite".to_owned(), true),
            (1, "Redis".to_owned(), false),
            (2, "Type something.".to_owned(), false),
        ]
    );
    assert_eq!(
        a.choices[1].detail.as_deref(),
        Some("Shared across processes")
    );
    let alerts = rig.alerts(1).await;
    assert!(alerts[0].1.payload["aps"].get("category").is_none());

    let outcome = resolved(rig.choose(&a, 1, &a.nonce).await);
    assert_eq!(
        outcome,
        ApprovalOutcome::Chosen {
            choice: 1,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Chosen { choice: 1, .. } }
            if approval_id == a.approval_id
    ));
    assert_eq!(
        rig.audit()[0]["result"],
        format!("choose 1: applied terminal={TERMINAL} keys=down,enter")
    );
    assert_eq!(
        code(rig.choose(&a, 1, &a.nonce).await),
        ErrorCode::ApprovalAlreadyResolved
    );

    rig.herdr.with(|h| {
        h.unblock_on_keys = false;
        h.text = QUESTION.into();
        set_status(h, "blocked");
    });
    let b = rig.needed().await;
    assert_eq!(
        resolved(rig.choose(&b, 0, &b.nonce).await),
        ApprovalOutcome::ChosenUnconfirmed {
            choice: 0,
            by: LABEL.into()
        },
        "Enter alone, the cursor is already there"
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys")[2],
        json!({"target": "w7:p1", "keys": ["enter"]})
    );
}

#[tokio::test]
async fn a_plan_is_chosen_with_arrows_never_esc() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = PLAN.into();
    });
    let a = rig.needed().await;
    assert!(
        a.options.is_empty(),
        "no approve option: not a permission prompt"
    );
    assert_eq!(
        choice_labels(&a),
        [
            (0, "Yes, and auto-accept edits".to_owned(), true),
            (1, "Yes, and manually approve edits".to_owned(), false),
            (2, "No, keep planning".to_owned(), false),
        ]
    );
    assert_eq!(
        code(rig.decide(&a, Decision::Deny, &a.nonce).await),
        ErrorCode::InvalidParams
    );
    let b = rig.needed().await;
    assert!(matches!(
        resolved(rig.choose(&b, 2, &b.nonce).await),
        ApprovalOutcome::Chosen { choice: 2, .. }
    ));
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down", "down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
}

#[tokio::test]
async fn the_unnumbered_trust_prompt_is_decided_like_a_numbered_one() {
    let mut rig = Rig::start(TTL).await;
    let trust = |h: &mut Herdr| {
        h.rule = "live_blocked_form".into();
        h.text = TRUST_292.into();
        set_status(h, "blocked");
    };
    rig.herdr.with(trust);
    let a = rig.needed().await;
    assert_eq!(a.options, [Decision::Approve, Decision::Deny]);
    assert_eq!(
        choice_labels(&a),
        [
            (0, TRUST_OPTIONS[0].to_owned(), true),
            (1, TRUST_OPTIONS[1].to_owned(), false),
        ]
    );
    assert!(!a.accepts_input && !a.has_text_field && !a.supports_note);
    assert_eq!(
        code(rig.choose(&a, 1, &a.nonce).await),
        ErrorCode::InvalidParams,
        "decided, never chosen by index"
    );

    rig.herdr.with(trust);
    let b = rig.needed().await;
    assert!(matches!(
        resolved(rig.decide(&b, Decision::Approve, &b.nonce).await),
        ApprovalOutcome::Applied { .. }
    ));
    rig.herdr.with(trust);
    let c = rig.needed().await;
    resolved(rig.decide(&c, Decision::Deny, &c.nonce).await);
    rig.herdr.with(|h| {
        trust(h);
        h.arrows_move = false;
    });
    let d = rig.needed().await;
    assert_eq!(
        resolved(rig.decide(&d, Decision::Approve, &d.nonce).await),
        ApprovalOutcome::Superseded
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
            json!({"target": "w7:p1", "keys": ["esc"]}),
            json!({"target": "w7:p1", "keys": ["down"]}),
        ]
    );
    let results: Vec<Value> = rig.audit().iter().map(|e| e["result"].clone()).collect();
    assert_eq!(
        results,
        [
            "choose 1: rejected: choice not offered".to_owned(),
            format!("approve: applied terminal={TERMINAL} keys=down,enter"),
            format!("deny: applied terminal={TERMINAL} keys=esc"),
            format!("approve: superseded: cursor not on target terminal={TERMINAL} keys=down"),
        ]
    );
}

#[tokio::test]
async fn approvals_say_whether_keys_and_text_are_taken() {
    let mut rig = Rig::start(TTL).await;
    let two = BASH.replace(
        "   2. Yes, and don't ask again for rm commands in /Users/me/src/app\n",
        "",
    );
    let two = two.replace("3. No", "2. No").replace(" · Tab to amend", "");
    for (rule, text, input, field, note) in [
        ("live_blocked_form", QUESTION_LIVE, true, true, false),
        ("live_blocked_form", QUESTION, true, true, false),
        ("live_blocked_form", PLAN, false, false, false),
        ("live_blocked_form", TRUST_LIVE, false, false, false),
        ("live_blocked_form", TRUST_292, false, false, false),
        ("bash_permission_prompt", BASH, false, false, true),
        ("bash_permission_prompt", &two, false, false, false),
        ("live_blocked_form", BASH, false, false, true),
        ("bash_permission_prompt", BASH_LIVE, false, false, true),
        ("legacy_no_prompt_blocker", PLAN_LIVE, false, true, false),
        ("live_blocked_form", PLAN_LIVE, false, false, false),
        ("mcp_elicitation_prompt", BASH_LIVE, false, false, false),
        (
            "live_blocked_form",
            &QUESTION.replace("   3. Type something.\n", ""),
            true,
            false,
            false,
        ),
    ] {
        rig.herdr.with(|h| {
            h.rule = rule.into();
            h.text = text.into();
            set_status(h, "blocked");
        });
        let a = rig.needed().await;
        assert_eq!(
            (a.accepts_input, a.has_text_field, a.supports_note),
            (input, field, note),
            "{rule} {text}"
        );
    }
}

#[tokio::test]
async fn choices_are_refused_where_a_decision_is_offered_or_out_of_range() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    assert_eq!(a.choices.len(), 3);
    assert_eq!(
        code(rig.choose(&a, 0, &a.nonce).await),
        ErrorCode::InvalidParams,
        "a permission prompt is answered with its decisions only"
    );
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved {
            outcome: ApprovalOutcome::Superseded,
            ..
        }
    ));

    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = QUESTION.into();
        set_status(h, "blocked");
    });
    let b = rig.needed().await;
    assert_eq!(
        code(rig.choose(&b, 3, &b.nonce).await),
        ErrorCode::InvalidParams
    );
    let c = rig.needed().await;
    assert_eq!(
        code(rig.decide(&c, Decision::Choose, &c.nonce).await),
        ErrorCode::InvalidParams,
        "choose without a choice"
    );
    assert!(rig.mutations().is_empty());
    let results: Vec<Value> = rig.audit().iter().map(|e| e["result"].clone()).collect();
    assert_eq!(
        results,
        [
            "choose 0: rejected: choice not offered",
            "choose 3: rejected: choice not offered",
            "choose: rejected: decision not offered",
        ]
    );
}

#[tokio::test]
async fn a_changed_question_supersedes_the_choice() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = QUESTION.into();
    });
    let a = rig.needed().await;
    rig.herdr
        .with(|h| h.text = QUESTION.replace("2. Redis", "2. Postgres"));
    assert_eq!(
        resolved(rig.choose(&a, 1, &a.nonce).await),
        ApprovalOutcome::Superseded
    );
    assert!(rig.mutations().is_empty());

    let b = rig.needed().await;
    rig.herdr.with(|h| h.arrows_move = false);
    assert_eq!(
        resolved(rig.choose(&b, 2, &b.nonce).await),
        ApprovalOutcome::Superseded,
        "lost arrows never confirm"
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["down", "down"]})]
    );
}

#[tokio::test]
async fn a_screen_that_drifts_under_a_blocked_agent_reissues_quietly() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = QUESTION.replace("   3. Type something.\n", "");
    });
    let a = rig.needed().await;
    assert_eq!((a.accepts_input, a.has_text_field), (true, false));

    rig.herdr.with(|h| h.text = move_cursor(&h.text, 1));
    rig.observe().await;
    rig.no_event();
    assert_eq!(rig.approvals.pending(), std::slice::from_ref(&a));

    rig.herdr.with(|h| h.text = QUESTION.into());
    rig.observe().await;
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Superseded }
            if approval_id == a.approval_id
    ));
    let Event::ApprovalNeeded { approval: b } = rig.event().await else {
        panic!("expected approval.needed");
    };
    assert_eq!((b.accepts_input, b.has_text_field), (true, true));
    assert_eq!(b.choices.len(), 3);

    rig.herdr.with(|h| h.text = PLAN.into());
    let c = rig.needed().await;
    assert_ne!(c.approval_id, b.approval_id);
    assert_eq!((c.accepts_input, c.has_text_field), (false, false));
    assert_eq!(
        rig.alerts(2).await.len(),
        1,
        "a rebuilt approval within 30 s does not alert again"
    );

    rig.herdr.with(|h| {
        h.text = QUESTION.into();
        set_status(h, "blocked");
    });
    rig.needed().await;
    assert_eq!(rig.alerts(2).await.len(), 2, "a new episode alerts");
    assert!(rig.mutations().is_empty());
}

fn yes() -> Authorized {
    Arc::new(|| true)
}

fn live(rig: &Rig) {
    rig.herdr.with(|h| h.text = BASH_LIVE.into());
}

#[tokio::test]
async fn a_note_is_typed_into_the_amend_field_then_enter() {
    let mut rig = Rig::start(TTL).await;
    live(&rig);
    let a = rig.needed().await;
    assert!(a.supports_note);
    assert_eq!(
        a.options,
        [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
    );
    let alerts = rig.alerts(1).await;
    assert_eq!(
        open_context(&alerts[0].1.payload),
        json!({"v": 1, "body": "Bash: Create empty probe2.txt file\ntouch probe2.txt"})
    );
    let note = "use a .tmp extension";
    let outcome = resolved(
        rig.decide_with_note(&a, Decision::Approve, note, yes())
            .await,
    );
    assert_eq!(
        outcome,
        ApprovalOutcome::Applied {
            decision: Decision::Approve,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["tab"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
    assert_eq!(
        rig.herdr.params("pane.send_text"),
        [json!({"pane_id": "w7:p1", "text": note})]
    );
    let audit = rig.audit();
    assert_eq!(
        audit[0]["result"],
        format!("approve with note: applied terminal={TERMINAL} keys=tab,enter")
    );
    assert!(
        !audit[0].to_string().contains(".tmp"),
        "the note is never logged"
    );

    rig.herdr.with(|h| {
        h.text = BASH_LIVE.into();
        set_status(h, "blocked");
    });
    let b = rig.needed().await;
    let outcome = resolved(
        rig.decide_with_note(&b, Decision::Deny, "run the tests first", yes())
            .await,
    );
    assert!(matches!(
        outcome,
        ApprovalOutcome::Applied {
            decision: Decision::Deny,
            ..
        }
    ));
    assert_eq!(
        rig.herdr.params("agent.send_keys")[2..],
        [
            json!({"target": "w7:p1", "keys": ["down", "down", "down"]}),
            json!({"target": "w7:p1", "keys": ["tab"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ],
        "Deny with a note goes by arrows, never Esc"
    );
    assert_eq!(
        rig.audit()[1]["result"],
        format!("deny with note: applied terminal={TERMINAL} keys=down,down,down,tab,enter")
    );
}

#[tokio::test]
async fn a_note_that_does_not_land_is_never_confirmed() {
    let mut rig = Rig::start(TTL).await;
    live(&rig);
    rig.herdr.with(|h| h.tab_amends = false);
    let a = rig.needed().await;
    assert_eq!(
        resolved(
            rig.decide_with_note(&a, Decision::Approve, "use a .tmp extension", yes())
                .await
        ),
        ApprovalOutcome::Superseded
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["tab"]})],
        "no text, no Enter, no Esc"
    );
    assert!(rig.herdr.params("pane.send_text").is_empty());
    assert_eq!(
        rig.audit()[0]["result"],
        format!(
            "approve with note: superseded: amend field not shown terminal={TERMINAL} keys=tab"
        )
    );

    rig.herdr.with(|h| {
        h.text = BASH_LIVE.into();
        h.tab_amends = true;
        h.text_lands = false;
    });
    let b = rig.needed().await;
    assert_eq!(
        resolved(
            rig.decide_with_note(&b, Decision::Approve, "use a .tmp extension", yes())
                .await
        ),
        ApprovalOutcome::Superseded
    );
    assert_eq!(rig.herdr.params("pane.send_text").len(), 1);
    assert_eq!(
        rig.herdr.params("agent.send_keys")[1..],
        [json!({"target": "w7:p1", "keys": ["tab"]})],
        "no Enter, no Esc"
    );
    assert_eq!(
        rig.audit()[1]["result"],
        format!("approve with note: superseded: note not shown terminal={TERMINAL} keys=tab")
    );

    rig.herdr.with(|h| {
        h.text = BASH_LIVE.into();
        h.text_lands = true;
        h.tab_moves_on = true;
    });
    let c = rig.needed().await;
    assert_eq!(
        resolved(
            rig.decide_with_note(&c, Decision::Approve, "use a .tmp extension", yes())
                .await
        ),
        ApprovalOutcome::Superseded,
        "a new prompt under the field"
    );
    assert_eq!(rig.herdr.params("pane.send_text").len(), 1);
    assert_eq!(rig.herdr.params("agent.send_keys").len(), 3);
    assert_eq!(
        rig.herdr.with(|h| h.agent["agent_status"].clone()),
        "blocked"
    );
}

#[tokio::test]
async fn a_note_needs_authorization_before_every_write() {
    let mut rig = Rig::start(TTL).await;
    live(&rig);
    for (allowed, tabs, texts) in [(1, 0, 0), (2, 1, 0), (3, 1, 1)] {
        rig.herdr.with(|h| {
            h.text = BASH_LIVE.into();
            set_status(h, "blocked");
        });
        let a = rig.needed().await;
        let before = (
            rig.herdr.params("agent.send_keys").len(),
            rig.herdr.params("pane.send_text").len(),
        );
        let checks = Arc::new(AtomicUsize::new(0));
        let seen = checks.clone();
        let auth: Authorized = Arc::new(move || seen.fetch_add(1, Ordering::SeqCst) < allowed);
        assert_eq!(
            code(
                rig.decide_with_note(&a, Decision::Approve, "use a .tmp extension", auth)
                    .await
            ),
            ErrorCode::NotPaired,
            "{allowed}"
        );
        assert_eq!(
            (
                rig.herdr.params("agent.send_keys").len() - before.0,
                rig.herdr.params("pane.send_text").len() - before.1,
            ),
            (tabs, texts),
            "{allowed}: never Enter"
        );
    }
}

#[tokio::test]
async fn a_note_is_refused_where_it_is_not_offered() {
    let mut rig = Rig::start(TTL).await;
    live(&rig);
    let a = rig.needed().await;
    assert_eq!(
        code(
            rig.decide_with_note(&a, Decision::ApproveAlways, "x", yes())
                .await
        ),
        ErrorCode::InvalidParams
    );
    rig.herdr.with(|h| {
        h.text = BASH_LIVE.replace(" · Tab to amend", "");
        set_status(h, "blocked");
    });
    let b = rig.needed().await;
    assert!(!b.supports_note);
    assert_eq!(
        code(
            rig.decide_with_note(&b, Decision::Approve, "x", yes())
                .await
        ),
        ErrorCode::InvalidParams
    );
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = QUESTION.into();
        set_status(h, "blocked");
    });
    let c = rig.needed().await;
    assert_eq!(
        code(rig.decide_with_note(&c, Decision::Deny, "x", yes()).await),
        ErrorCode::InvalidParams
    );
    assert!(rig.mutations().is_empty());
    let results: Vec<Value> = rig.audit().iter().map(|e| e["result"].clone()).collect();
    assert_eq!(
        results,
        [
            "approve_always with note: rejected: note not offered",
            "approve with note: rejected: note not offered",
            "deny with note: rejected: decision not offered",
        ]
    );
}

#[tokio::test]
async fn a_plan_is_chosen_but_takes_no_note() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.rule = "legacy_no_prompt_blocker".into();
        h.text = PLAN_LIVE.into();
    });
    let a = rig.needed().await;
    assert!(a.options.is_empty());
    assert_eq!(
        (a.accepts_input, a.has_text_field, a.supports_note),
        (false, true, false)
    );
    assert_eq!(
        choice_labels(&a),
        [
            (0, "Yes, and use auto mode".to_owned(), true),
            (1, "Yes, manually approve edits".to_owned(), false),
            (2, "Tell Claude what to change".to_owned(), false),
        ]
    );
    assert!(matches!(
        resolved(rig.choose(&a, 1, &a.nonce).await),
        ApprovalOutcome::Chosen { choice: 1, .. }
    ));
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
}
