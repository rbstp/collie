use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use collied::approvals::Approvals;
use collied::audit::Audit;
use collied::drive::{Authorized, Reply};
use collied::push::{Alert, Device, Push, Rejection, Sender};
use futures_util::future::BoxFuture;
use protocol::{
    ApnsEnvironment, Approval, ApprovalDecideParams, ApprovalOutcome, Decision, ErrorCode, Event,
    Nonce, NotificationKey, PushToken, Response,
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

const BASH: &str = "\
⏺ Bash(rm -rf build)
  ⎿  Running…

────────────────────────────────────────────────────────────────────────────────
 Bash command

   rm -rf build
   Remove the build directory

 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don't ask again for rm commands in /Users/me/src/app
   3. No, and tell Claude what to do differently (esc)

 Esc to cancel · Tab to amend · ctrl+e to explain
";

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
                    _ => {}
                }
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
            t.as_bytes().first().is_some_and(u8::is_ascii_digit) && t[1..].starts_with(". ")
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

#[derive(Default)]
struct Apns {
    sent: Mutex<Vec<(String, Alert)>>,
}

impl Sender for Apns {
    fn send<'a>(
        &'a self,
        device: &'a Device,
        alert: &'a Alert,
    ) -> BoxFuture<'a, Result<(), Rejection>> {
        Box::pin(async move {
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
    approvals: Arc<Approvals>,
    events: broadcast::Receiver<Event>,
    audit: PathBuf,
    _dir: tempfile::TempDir,
}

fn token() -> PushToken {
    PushToken::new("ab".repeat(32)).unwrap()
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
        let (tx, events) = broadcast::channel(64);
        let mut approvals = Approvals::new(
            herdr.socket.clone(),
            "nMAC".into(),
            tx,
            Arc::new(Audit::open(&audit).unwrap()),
            Some(push),
        )
        .with_timing(ttl, SETTLE);
        if let Some(budget) = budget {
            approvals = approvals.with_budget(budget);
        }
        let approvals = Arc::new(approvals);
        Self {
            herdr,
            apns,
            approvals,
            events,
            audit,
            _dir: dir,
        }
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
            nonce: nonce.clone(),
        };
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
