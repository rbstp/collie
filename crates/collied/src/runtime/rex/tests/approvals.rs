use std::sync::Arc;

use protocol::{Approval, ApprovalDecideParams, ApprovalOutcome, Decision, Event, Response};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::{B1, S1, T0, claude, eventually, follow, id};
use crate::approvals::Approvals;
use crate::audit::Audit;
use crate::drive::Authorized;
use crate::runtime::rex::Rex;
use crate::runtime::rex::client::Method;
use crate::runtime::rex::fake::{self, Fake};
use crate::runtime::{Runtime, WorkspaceInfo};

const VT: &str =
    include_str!("../../../../tests/fixtures/rex-0.1.0-preview.19/claude-2.1.325-permission.vt");
const FORMAT: &str = "com.superlogical.terminal.format";
const T1: &str = "2026-11-12T16:47:02.000001Z";
const T2: &str = "2026-11-12T16:47:03.000001Z";
const T3: &str = "2026-11-12T16:47:04.000001Z";

/// Claude Code's report: `blocked` on a permission prompt names its kind.
fn report(state: &str, at: &str) -> Value {
    let mut r = fake::record(state, 10, "2.1.322", at);
    if state == "blocked" {
        r["kind"] = json!("permission");
    }
    r
}

struct Blocked {
    fake: Fake,
    rex: Arc<Rex>,
    rt: Runtime,
    approvals: Approvals,
    events: broadcast::Receiver<Event>,
    _following: tokio::task::JoinHandle<(crate::runtime::Error, bool)>,
    _dir: tempfile::TempDir,
}

async fn blocked() -> Blocked {
    blocked_timed(crate::approvals::TTL, crate::approvals::SETTLE).await
}

async fn blocked_settling(settle: std::time::Duration) -> Blocked {
    blocked_timed(crate::approvals::TTL, settle).await
}

/// A Claude Code in `B1` blocked on Rex's own screen of a permission dialog; a decision waits
/// `settle` for it to move on.
async fn blocked_timed(ttl: std::time::Duration, settle: std::time::Duration) -> Blocked {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "blocked", T0);
    {
        let mut s = fake.state();
        let b = s.block(B1);
        b.records = vec![report("blocked", T0)];
        b.screen = VT.to_owned();
        b.size = (198, 68);
    }
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let following = follow(&rex).await;
    let dir = tempfile::tempdir().unwrap();
    let none = dir.path().join("no-herdr.sock");
    let rt = Runtime::new(none.clone()).with_rex(Some(rex.clone()));
    let audit = Arc::new(Audit::open(&dir.path().join("audit.log")).unwrap());
    let (tx, events) = broadcast::channel(64);
    let approvals = Approvals::new(none, "nMAC".into(), tx, audit, None)
        .with_rex(Some(rex.clone()))
        .with_timing(ttl, settle);
    Blocked {
        fake,
        rex,
        rt,
        approvals,
        events,
        _following: following,
        _dir: dir,
    }
}

impl Blocked {
    async fn observe(&self) {
        let agents = self.rt.agent_list().await.unwrap();
        self.approvals
            .observe(&agents, &[] as &[WorkspaceInfo])
            .await;
    }

    fn raised(&mut self) -> Vec<Approval> {
        std::iter::from_fn(|| self.events.try_recv().ok())
            .filter_map(|e| match e {
                Event::ApprovalNeeded { approval } => Some(approval),
                _ => None,
            })
            .collect()
    }

    /// Claude Code's next report, as Rex pushes it.
    fn reports(&self, state: &'static str, at: &'static str) {
        self.fake.emit(move |s| {
            s.block(B1).records = vec![report(state, at)];
            vec![fake::status_changed(S1, B1, report(state, at))]
        });
    }

    async fn seq_moves(&self, from: u64) -> u64 {
        eventually("the next report", || {
            self.rex.get(&id(B1)).unwrap().state_change_seq != from
        })
        .await;
        self.rex.get(&id(B1)).unwrap().state_change_seq
    }

    /// The program takes the key that answers and reports `working`.
    fn answers_with_work(&self) {
        self.fake.state().hook(
            Method::BlockEvent,
            Some(B1),
            true,
            Box::new(|s| {
                s.block(B1).records = vec![report("working", T3)];
                vec![fake::status_changed(S1, B1, report("working", T3))]
            }),
        );
    }

    async fn decide(&self, a: &Approval, decision: Decision) -> ApprovalOutcome {
        let p = ApprovalDecideParams {
            approval_id: a.approval_id.clone(),
            decision,
            choice: None,
            nonce: a.nonce.clone(),
            note: None,
        };
        let auth: Authorized = Arc::new(|| true);
        match self.approvals.decide("phone", "nPHONE", p, &auth).await {
            Ok(Response::ApprovalResolved { outcome, .. }) => outcome,
            other => panic!("{other:?}"),
        }
    }

    fn keys(&self) -> Vec<Value> {
        std::mem::take(&mut self.fake.state().block(B1).events)
    }

    fn formats(&self) -> usize {
        self.fake.calls().iter().filter(|c| *c == FORMAT).count()
    }
}

fn enter() -> Value {
    json!({ "type": "key", "payload": {
        "key": "enter", "mods": "", "consumed_mods": "", "text": "\r",
        "unshifted_codepoint": 13, "composing": false, "action": "press",
    }})
}

#[tokio::test]
async fn a_permission_dialog_raises_an_approval_that_approve_answers_with_enter() {
    let mut b = blocked().await;
    b.observe().await;
    let raised = b.raised();
    let [a] = raised.as_slice() else {
        panic!("{raised:?}")
    };
    assert_eq!(
        a.options,
        [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
    );
    b.answers_with_work();
    let outcome = b.decide(a, Decision::Approve).await;
    assert!(
        matches!(
            outcome,
            ApprovalOutcome::Applied {
                decision: Decision::Approve,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(b.keys(), [enter()]);
}

#[tokio::test]
async fn a_decision_for_the_first_of_two_identical_dialogs_never_answers_the_second() {
    let mut b = blocked().await;
    let first_seq = b.rex.get(&id(B1)).unwrap().state_change_seq;
    b.observe().await;
    let [first] = b.raised().try_into().unwrap();

    // Answered on the Mac, then the same call asks again: the same screen, a new report.
    b.reports("working", T1);
    b.reports("blocked", T2);
    let second_seq = b.seq_moves(first_seq).await;
    assert_eq!(b.rex.get(&id(B1)).unwrap().agent_status, "blocked");
    assert_eq!(
        b.decide(&first, Decision::Approve).await,
        ApprovalOutcome::Superseded
    );
    assert!(b.keys().is_empty(), "a key reached the second dialog");

    b.observe().await;
    let [second] = b.raised().try_into().unwrap();
    assert_ne!(second.approval_id, first.approval_id);
    assert_ne!(second_seq, first_seq);
    b.answers_with_work();
    assert!(matches!(
        b.decide(&second, Decision::Approve).await,
        ApprovalOutcome::Applied { .. }
    ));
    assert_eq!(b.keys(), [enter()]);
}

#[tokio::test]
async fn an_approval_outlives_a_reconnect_when_its_report_did_not_change() {
    let mut b = blocked().await;
    let seq = b.rex.get(&id(B1)).unwrap().state_change_seq;
    b.observe().await;
    let [a] = b.raised().try_into().unwrap();

    b.fake.drop_connection();
    eventually("the connection to drop", || b.rex.listing().is_none()).await;
    let _again = follow(&b.rex).await;
    assert_eq!(b.rex.get(&id(B1)).unwrap().state_change_seq, seq);
    b.observe().await;
    assert!(b.raised().is_empty());
    let pending: Vec<_> = b
        .approvals
        .pending()
        .into_iter()
        .map(|p| p.approval_id)
        .collect();
    assert_eq!(pending, std::slice::from_ref(&a.approval_id));

    // The decision's own read finds the same seq and fingerprint, so the key is sent.
    b.answers_with_work();
    assert!(matches!(
        b.decide(&a, Decision::Approve).await,
        ApprovalOutcome::Applied { .. }
    ));
    assert_eq!(b.keys(), [enter()]);
}

#[tokio::test]
async fn a_blocked_agent_has_its_screen_read_on_three_reconciles_per_report() {
    let mut b = blocked().await;
    b.observe().await;
    let per_read = b.formats();
    assert!(per_read > 0);
    for _ in 0..20 {
        b.observe().await;
    }
    assert_eq!(b.formats(), 3 * per_read);
    let [first] = b.raised().try_into().unwrap();

    // A new report reads again, three times at most.
    let seq = b.rex.get(&id(B1)).unwrap().state_change_seq;
    b.reports("blocked", T1);
    b.seq_moves(seq).await;
    for _ in 0..20 {
        b.observe().await;
    }
    assert_eq!(b.formats(), 6 * per_read);
    let [second] = b.raised().try_into().unwrap();
    assert_ne!(second.approval_id, first.approval_id);
}

#[tokio::test]
async fn a_decision_the_agent_did_not_take_lets_the_same_report_raise_a_fresh_approval() {
    let mut b = blocked_settling(std::time::Duration::from_millis(50)).await;
    for _ in 0..5 {
        b.observe().await;
    }
    let [a] = b.raised().try_into().unwrap();
    assert!(matches!(
        b.decide(&a, Decision::Approve).await,
        ApprovalOutcome::Unconfirmed { .. }
    ));
    b.observe().await;
    let [again] = b.raised().try_into().unwrap();
    assert_ne!(again.approval_id, a.approval_id);
}

/// Whether the screen reads fail: Rex answers `format` with an error.
fn reads_fail(b: &Blocked, fail: bool) {
    let mut s = b.fake.state();
    s.terminal.retain(|m| *m != "format");
    if !fail {
        s.terminal.push("format");
    }
}

#[tokio::test]
async fn an_approval_that_expires_is_raised_again_for_a_dialog_still_open() {
    let mut b = blocked_timed(
        std::time::Duration::from_millis(300),
        crate::approvals::SETTLE,
    )
    .await;
    b.observe().await;
    let [first] = b.raised().try_into().unwrap();
    let mut again = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while again.is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "never raised again");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        b.observe().await;
        again = b.raised();
    }
    assert_ne!(again[0].approval_id, first.approval_id);
}

#[tokio::test]
async fn failed_screen_reads_do_not_use_up_the_reads_of_a_report() {
    let mut b = blocked().await;
    reads_fail(&b, true);
    for _ in 0..3 {
        b.observe().await;
    }
    assert!(b.raised().is_empty());
    reads_fail(&b, false);
    b.observe().await;
    let [a] = b.raised().try_into().unwrap();
    assert_eq!(
        a.options,
        [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
    );
}

#[tokio::test]
async fn an_approval_raised_on_a_dialog_still_drawing_is_replaced_once_it_is_drawn() {
    let mut b = blocked().await;
    b.fake.state().block(B1).screen = "\r\n Bash command\r\n\r\n   touch x\r\n".to_owned();
    reads_fail(&b, true);
    b.observe().await;
    reads_fail(&b, false);
    b.observe().await;
    b.observe().await;
    let [drawing] = b.raised().try_into().unwrap();
    assert!(drawing.options.is_empty());
    // Drawn on the third read: the approval it ends is replaced, past this report's reads.
    b.fake.state().block(B1).screen = VT.to_owned();
    for _ in 0..3 {
        b.observe().await;
    }
    let raised = b.raised();
    let pending: Vec<_> = b.approvals.pending();
    assert_eq!(pending.len(), 1);
    assert_ne!(pending[0].approval_id, drawing.approval_id);
    assert_eq!(
        pending[0].options,
        [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
    );
    assert_eq!(raised.len(), 1);
}

/// Copilot CLI draws a numbered menu like Claude Code's, and only Claude Code's is parsed: a
/// blocked Copilot raises an approval with nothing to decide, and a decision sends nothing.
#[tokio::test]
async fn a_blocked_copilot_raises_an_approval_with_no_option_and_takes_no_decision() {
    let fake = Fake::start().await;
    let record = json!({
        "state": "blocked", "kind": "permission", "app": "copilot", "updated_at": T0,
        "owner": { "pid": 10, "name": "copilot" },
    });
    {
        let mut s = fake.state();
        s.sessions.push((S1.to_owned(), "work".to_owned()));
        let copilot = fake::process(10, "/opt/homebrew/bin/copilot", "copilot");
        s.add(S1, B1, copilot, vec![record]);
        s.block(B1).screen = include_str!("../../../../tests/fixtures/copilot/shell.detection.txt")
            .replace('\n', "\r\n");
    }
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _following = follow(&rex).await;
    let dir = tempfile::tempdir().unwrap();
    let none = dir.path().join("no-herdr.sock");
    let rt = Runtime::new(none.clone()).with_rex(Some(rex.clone()));
    let audit = Arc::new(Audit::open(&dir.path().join("audit.log")).unwrap());
    let (tx, mut events) = broadcast::channel(64);
    let approvals = Approvals::new(none, "nMAC".into(), tx, audit, None).with_rex(Some(rex));
    let agents = rt.agent_list().await.unwrap();
    assert!(agents[0].read_only);
    approvals.observe(&agents, &[] as &[WorkspaceInfo]).await;
    let raised: Vec<Approval> = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|e| match e {
            Event::ApprovalNeeded { approval } => Some(approval),
            _ => None,
        })
        .collect();
    let [a] = raised.as_slice() else {
        panic!("{raised:?}")
    };
    assert!(a.options.is_empty() && a.choices.is_empty());
    assert!(!a.accepts_input && !a.has_text_field);
    let auth: Authorized = Arc::new(|| true);
    let p = ApprovalDecideParams {
        approval_id: a.approval_id.clone(),
        decision: Decision::Approve,
        choice: None,
        nonce: a.nonce.clone(),
        note: None,
    };
    let refused = approvals.decide("phone", "nPHONE", p, &auth).await;
    assert_eq!(refused.unwrap_err().0, protocol::ErrorCode::InvalidParams);
    assert!(fake.state().block(B1).events.is_empty());
    assert!(!fake.calls().iter().any(|c| c == "block.event"));
}
