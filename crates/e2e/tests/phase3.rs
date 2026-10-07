#[allow(dead_code)]
mod common;

use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use collie_core::{
    ApprovalDecision, ApprovalEvent, ApprovalFeed, BackgroundOutcome, CollieCore, CoreError,
    DecisionOutcome, PendingApproval, PushEnvironment,
};
use collied::config::{PUSH_FILE, TasksConfig};
use collied::control::{Client, Reply, Request};
use collied::push::{Alert, Delivery, Device, Devices, Rejection, Sender};
use collied::server::{self, ServerConfig, ServerHandle};
use common::*;
use futures_util::{SinkExt, StreamExt};
use protocol::{ApnsEnvironment, Approval, ErrorCode, PairingInvite, Response, ServerFrame};
use serde_json::{Value, json};
use tailnet::Node;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, header};

// testcontrol demands the auth key on every registration, even from a node key it
// already knows, which real control accepts. tsnet falls back to TS_AUTHKEY, so the
// phone's cold start from cached state passes no key itself.
const KEY: &str = "test-authkey-colliee2ephase3";
const TERMINAL: &str = "term_0a1b2c3d4e5f60";
const PANE: &str = "w7:p1";
const TOKEN: &str = "c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00";
const ACTIVITY: &str = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";
const ACTIVITY_TOKEN: &str =
    "a11ce0a11ce0a11ce0a11ce0a11ce0a11ce0a11ce0a11ce0a11ce0a11ce0a11ce0a11ce0a11ce0";
const SECOND_TOKEN: &str =
    "b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0";
const BUDGET_MS: u64 = 20_000;
// The shared test vector's key (docs/protocol/notification-vector.json).
const NOTIFY_KEY: [u8; 32] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32,
];
const PROBE: &str = "E2E probe";
const SNIPPET: &str =
    "Bash command\nrm -rf build\nRemove the build directory\nDo you want to proceed?";
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

#[test]
fn phase3_end_to_end() {
    if !in_child_with("phase3_end_to_end", &[("TS_AUTHKEY", KEY)]) {
        return;
    }
    let t0 = Instant::now();
    let root = TempDir::new("e2e3");
    let net = Net::with_key(&root.0, KEY.into());
    wait_ready(&net.mac, 0);
    let probe = start_node(&root.0, "probe", &net.key, &net.url, &[]);
    let group = root.0.join("group");
    std::fs::create_dir(&group).unwrap();
    let core = phone(&root.0, "phone", &net);
    core.set_app_group_dir(group.to_str().unwrap().into())
        .unwrap();
    let rt = runtime();
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    let mac_ip = wait_ready(&net.mac, 2)
        .self_node
        .unwrap()
        .tailscale_ips
        .unwrap()
        .into_iter()
        .find(|ip| ip.is_ipv4())
        .unwrap();
    wait_ready(&probe, 1);
    wait_phone(&rt, &core);
    println!(
        "tailnet: control, Mac, phone, probe up in {:?}",
        t0.elapsed()
    );

    let herdr = rt.block_on(async { Mock::start(&root.0.join("herdr.sock")) });
    let apns = Arc::new(Apns::default());
    let data_dir = root.0.join("collied");
    let handle = rt.block_on(start_collied(
        &net,
        &data_dir,
        &herdr,
        apns.clone(),
        collied::approvals::TTL,
    ));
    let rig = Rig {
        herdr,
        apns,
        data_dir,
        control: handle.control_path(),
        probe: Probe {
            node: probe,
            target: format!("{mac_ip}:{PORT}"),
            data_dir: root.0.join("collied"),
        },
    };
    let (machine, phone_id, nonces) = rt.block_on(in_app(&rig, &core));

    println!(
        "lock-screen action: a new CollieCore in this process restarts the node from cached state"
    );
    let alert = rig.apns.last();
    let node = alert.payload["node_id"].as_str().unwrap().to_owned();
    assert_eq!(node, machine.node_id);
    let pending_id = alert.payload["approval_id"].as_str().unwrap().to_owned();
    let before = rig.registrations();
    drop(core);
    let core = phone(&root.0, "phone", &net);
    core.set_app_group_dir(group.to_str().unwrap().into())
        .unwrap();
    let report = rt.block_on(core.decide_from_notification(
        node.clone(),
        pending_id.clone(),
        ApprovalDecision::Approve,
        Some(BUDGET_MS),
    ));
    println!(
        "  node up {:?} ms, connect {:?} ms, lookup {:?} ms, decide {:?} ms, total {} ms",
        report.node_up_ms, report.connect_ms, report.lookup_ms, report.decide_ms, report.total_ms
    );
    assert_eq!(
        report.outcome,
        BackgroundOutcome::Applied {
            decision: ApprovalDecision::Approve
        },
        "{report:?}"
    );
    assert!(!report.node_was_running);
    assert!(report.total_ms <= BUDGET_MS, "{report:?}");
    assert!(!format!("{report:?}").contains(&nonces[1]));
    assert!(
        [
            report.node_up_ms,
            report.connect_ms,
            report.lookup_ms,
            report.decide_ms
        ]
        .iter()
        .all(Option::is_some),
        "{report:?}"
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys").last(),
        Some(&json!({"target": PANE, "keys": ["enter"]}))
    );
    let seen = reachability(&group, &node);
    assert!(
        seen["last_ok_ms"].as_u64() > seen["last_fail_ms"].as_u64(),
        "{seen}"
    );
    assert_eq!(
        rig.registrations(),
        before,
        "the one-shot lock-screen session does not register push"
    );

    rt.block_on(after_restart(&rig, &core, &machine.id, &phone_id, &node));
    drop(core);
    rt.block_on(handle.shutdown());
    drop(rt);
    for nonce in &nonces {
        assert_eq!(files_containing(&root.0, nonce), Vec::<PathBuf>::new());
    }
    println!("total {:?}", t0.elapsed());
}

// collied's reconcile expires a pending approval on its own once herdr answers, so herdr
// is taken down while the approval runs out: the decision is what finds it expired.
#[test]
fn phase3_expired_approvals() {
    if !in_child_with("phase3_expired_approvals", &[("TS_AUTHKEY", KEY)]) {
        return;
    }
    let root = TempDir::new("e2e3x");
    let net = Net::with_key(&root.0, KEY.into());
    wait_ready(&net.mac, 0);
    let core = phone(&root.0, "phone", &net);
    let rt = runtime();
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    wait_ready(&net.mac, 1);
    wait_phone(&rt, &core);
    let herdr = rt.block_on(async { Mock::start(&root.0.join("herdr.sock")) });
    let apns = Arc::new(Apns::default());
    let data_dir = root.0.join("collied");
    let ttl = Duration::from_secs(3);
    let handle = rt.block_on(start_collied(&net, &data_dir, &herdr, apns.clone(), ttl));
    rt.block_on(async {
        let (machine, _) = pair(&handle.control_path(), &core, LABEL).await;
        let m = machine.id.clone();
        connected_flock(&core, &m).await;
        core.register_push(
            machine.id.clone(),
            TOKEN.into(),
            PushEnvironment::Sandbox,
            NOTIFY_KEY.to_vec(),
        )
        .unwrap();
        let rev = core.approval_feed(m.clone(), 0).unwrap().revision;

        println!("in-app decide on an expired approval");
        herdr.set_status("blocked");
        let (a, rev) = needed(&core, &m, rev).await;
        herdr.with(|h| h.down = true);
        tokio::time::sleep(ttl + Duration::from_millis(200)).await;
        let err = core
            .decide(
                m.clone(),
                a.approval_id.clone(),
                ApprovalDecision::Approve,
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::ApprovalExpired), "{err:?}");
        let (rev, outcome) = resolved(&core, &m, rev, &a.approval_id).await;
        assert_eq!(outcome, DecisionOutcome::Expired);

        println!("the reissued approval alerts again, replacing the dead alert");
        herdr.with(|h| h.down = false);
        let (b, _) = needed(&core, &m, rev).await;
        assert_ne!(b.approval_id, a.approval_id);
        let alerts = apns.wait(2).await;
        assert_eq!(alerts.len(), 2, "{alerts:?}");
        assert_eq!(alerts[0].1.payload["approval_id"], a.approval_id.as_str());
        assert_eq!(alerts[1].1.payload["approval_id"], b.approval_id.as_str());
        for (_, alert) in &alerts {
            assert_eq!(alert.collapse_id.as_deref(), Some(TERMINAL));
        }

        println!("lock-screen decide on an expired approval");
        herdr.with(|h| h.down = true);
        tokio::time::sleep(ttl + Duration::from_millis(200)).await;
        let report = core
            .decide_from_notification(
                machine.node_id.clone(),
                b.approval_id.clone(),
                ApprovalDecision::Approve,
                Some(BUDGET_MS),
            )
            .await;
        assert_eq!(report.outcome, BackgroundOutcome::Expired, "{report:?}");

        let decisions: Vec<Value> = audit_lines(&data_dir.join("audit.log"))
            .into_iter()
            .filter(|l| l["method"] == "approval.decide")
            .map(|l| l["result"].clone())
            .collect();
        assert_eq!(
            decisions,
            ["approve: rejected: expired", "approve: rejected: expired"]
        );
        assert!(herdr.params("agent.send_keys").is_empty());
        assert!(herdr.mutations().is_empty());
    });
    drop(core);
    rt.block_on(handle.shutdown());
}

// The followed agent's Live Activity: an update token registered from the app, a push
// with the Live Activity topic when the agent blocks, and an end when its pane closes.
#[test]
fn live_activity_follows_an_agent() {
    if !in_child_with("live_activity_follows_an_agent", &[("TS_AUTHKEY", KEY)]) {
        return;
    }
    let root = TempDir::new("e2e4");
    let net = Net::with_key(&root.0, KEY.into());
    wait_ready(&net.mac, 0);
    let core = phone(&root.0, "phone", &net);
    let rt = runtime();
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    wait_ready(&net.mac, 1);
    wait_phone(&rt, &core);
    let herdr = rt.block_on(async { Mock::start(&root.0.join("herdr.sock")) });
    let apns = Arc::new(Apns::default());
    let data_dir = root.0.join("collied");
    let handle = rt.block_on(start_collied(
        &net,
        &data_dir,
        &herdr,
        apns.clone(),
        collied::approvals::TTL,
    ));
    rt.block_on(async {
        let (machine, _) = pair(&handle.control_path(), &core, LABEL).await;
        let m = machine.id.clone();
        connected_flock(&core, &m).await;
        core.register_push(
            m.clone(),
            TOKEN.into(),
            PushEnvironment::Production,
            NOTIFY_KEY.to_vec(),
        )
        .unwrap();
        core.register_activity_token(
            m.clone(),
            ACTIVITY.into(),
            TERMINAL.into(),
            ACTIVITY_TOKEN.into(),
        )
        .unwrap();
        let activities = || match std::fs::read_to_string(data_dir.join(PUSH_FILE)) {
            Ok(text) => serde_json::from_str::<Devices>(&text).unwrap().activities,
            Err(_) => Vec::new(),
        };
        wait_for("push.activity_token", || activities().len() == 1).await;
        let stored = &activities()[0];
        assert_eq!(stored.terminal_id.as_str(), TERMINAL);
        assert_eq!(stored.environment, ApnsEnvironment::Production);

        println!("the activity is synced quietly, then a blocked agent pushes with priority 10");
        let (_, sync) = live(&apns, "the first sync", |_, _| true).await;
        assert_eq!(sync.delivery, Delivery::LiveActivity { urgent: false });
        assert_eq!(sync.payload["aps"]["content-state"]["status"], "working");
        let rev = core.approval_feed(m.clone(), 0).unwrap().revision;
        herdr.set_status("blocked");
        let (approval, _) = needed(&core, &m, rev).await;
        let (device, blocked) = live(&apns, "the blocked update", |_, a| {
            a.payload["aps"]["content-state"]["status"] == "blocked"
        })
        .await;
        assert_eq!(device.token.as_str(), ACTIVITY_TOKEN);
        assert_eq!(device.environment, ApnsEnvironment::Production);
        assert_eq!(device.notification_key, None);
        let headers = blocked.headers("dev.rbstp.collie");
        assert_eq!(headers.push_type.to_string(), "liveactivity");
        assert_eq!(headers.topic, "dev.rbstp.collie.push-type.liveactivity");
        assert_eq!(headers.priority, 10);
        let aps = &blocked.payload["aps"];
        assert_eq!(aps["event"], "update");
        let state = &aps["content-state"];
        assert_eq!(
            *state,
            json!({
                "status": "blocked",
                "statusSince": state["statusSince"],
                "title": "api-fixer",
                "kind": "claude",
                "workspace": "api",
                "approvals": 1,
                "approvalId": state["approvalId"],
                "enc": state["enc"],
            })
        );
        assert_eq!(state["approvalId"], approval.approval_id.as_str());
        assert_eq!(
            phone_opens(&json!({"enc": state["enc"], "approval_id": state["approvalId"]})),
            json!({"v": 1, "body": "Bash: rm -rf build\nRemove the build directory"})
        );
        assert!(
            !apns
                .sent
                .lock()
                .unwrap()
                .iter()
                .any(|(_, a)| a.delivery == Delivery::Alert),
            "the follower gets the approval on its activity only"
        );
        assert_eq!(
            aps["alert"],
            json!({"title": "api-fixer", "body": "Blocked in api"})
        );
        let now = collied::now_ms() / 1000;
        let since = aps["content-state"]["statusSince"].as_i64().unwrap();
        assert!(
            (now as i64 - 978_307_200 - since).abs() < 30,
            "seconds since 2001: {since}"
        );
        assert!(aps["stale-date"].as_u64().unwrap() >= now + 800);
        let text = blocked.payload.to_string();
        assert!(!text.contains("rm -rf") && !text.contains("Bash"), "{text}");

        println!("ending it from the phone removes the token");
        core.end_activity(m.clone(), ACTIVITY.into()).unwrap();
        wait_for("push.activity_end", || activities().is_empty()).await;

        println!("a closed pane ends the activity with a dismissal date");
        core.register_activity_token(
            m.clone(),
            "SECOND".into(),
            TERMINAL.into(),
            SECOND_TOKEN.into(),
        )
        .unwrap();
        live(&apns, "the second sync", |d, _| {
            d.token.as_str() == SECOND_TOKEN
        })
        .await;
        herdr.with(|h| h.gone = true);
        let (_, ended) = live(&apns, "the end", |d, a| {
            d.token.as_str() == SECOND_TOKEN && a.payload["aps"]["event"] == "end"
        })
        .await;
        assert_eq!(ended.payload["aps"]["event"], "end");
        assert!(ended.payload["aps"]["dismissal-date"].as_u64().unwrap() >= now + 60);
        wait_for("token dropped", || activities().is_empty()).await;

        let audit = audit_lines(&data_dir.join("audit.log"));
        let lines: Vec<(String, String)> = audit
            .iter()
            .filter(|l| {
                l["method"]
                    .as_str()
                    .is_some_and(|m| m.starts_with("push.activity"))
            })
            .map(|l| {
                (
                    l["method"].as_str().unwrap().to_owned(),
                    l["result"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(
            lines,
            [
                ("push.activity_token".into(), "ok".into()),
                ("push.activity_end".into(), "ok".into()),
                ("push.activity_token".into(), "ok".into()),
                ("push.activity_end".into(), "ended: agent gone".into()),
            ]
        );
        let text = std::fs::read_to_string(data_dir.join("audit.log")).unwrap();
        assert!(
            !text.contains(ACTIVITY_TOKEN) && !text.contains(SECOND_TOKEN),
            "token in the audit log"
        );
    });
    drop(core);
    rt.block_on(handle.shutdown());
}

/// The first Live Activity push that matches.
async fn live(
    apns: &Apns,
    what: &str,
    matches: impl Fn(&Device, &Alert) -> bool,
) -> (Device, Alert) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let found = apns.sent.lock().unwrap().iter().find_map(|(d, a)| {
            (matches!(a.delivery, Delivery::LiveActivity { .. }) && matches(d, a))
                .then(|| (d.clone(), a.clone()))
        });
        if let Some(found) = found {
            return found;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

struct Rig {
    herdr: Mock,
    apns: Arc<Apns>,
    data_dir: PathBuf,
    control: PathBuf,
    probe: Probe,
}

impl Rig {
    fn audit(&self) -> Vec<Value> {
        audit_lines(&self.data_dir.join("audit.log"))
    }

    fn decisions(&self) -> Vec<String> {
        self.audit()
            .iter()
            .filter(|l| l["method"] == "approval.decide")
            .map(|l| {
                assert!(l["peer"] == LABEL || l["peer"] == PROBE, "{l}");
                l["result"].as_str().unwrap().to_owned()
            })
            .collect()
    }

    fn registrations(&self) -> usize {
        self.audit()
            .iter()
            .filter(|l| l["method"] == "push.register" && l["result"] == "ok")
            .count()
    }

    fn devices(&self) -> Vec<Device> {
        let text = std::fs::read_to_string(self.data_dir.join(PUSH_FILE)).unwrap();
        serde_json::from_str::<Devices>(&text).unwrap().devices
    }
}

async fn in_app(rig: &Rig, core: &Arc<CollieCore>) -> (collie_core::Machine, String, [String; 2]) {
    let (machine, _) = pair(&rig.control, core, LABEL).await;
    let m = machine.id.clone();
    connected_flock(core, &m).await;
    let Some(Reply::Peers { peers, .. }) =
        collied::control::request(&rig.control, &Request::PeersList)
            .await
            .unwrap()
    else {
        panic!("no peers reply");
    };
    let phone_id = peers[0].stable_id.clone();
    rig.probe.pair(&rig.control).await;

    println!("register_push is stored per peer by collied, never on the phone's disk");
    core.register_push(
        m.clone(),
        TOKEN.into(),
        PushEnvironment::Sandbox,
        NOTIFY_KEY.to_vec(),
    )
    .unwrap();
    wait_for("push.register", || rig.registrations() == 1).await;
    let devices = rig.devices();
    assert_eq!(devices.len(), 1, "{devices:?}");
    assert_eq!(devices[0].stable_id, phone_id);
    assert_eq!(devices[0].token.as_str(), TOKEN);
    assert_eq!(devices[0].environment, ApnsEnvironment::Sandbox);
    assert_eq!(
        devices[0].notification_key.as_ref().map(|k| k.as_str()),
        Some(encoded_key().as_str())
    );
    assert_private(&rig.data_dir.join(PUSH_FILE));
    let phone_dir = rig.data_dir.parent().unwrap().join("phone");
    assert_eq!(
        files_containing(&phone_dir, &encoded_key()),
        Vec::<PathBuf>::new()
    );

    println!("register_push is sent again after a reconnect");
    let first = devices[0].registered_at;
    core.resume(60);
    wait_for("push.register after reconnect", || rig.registrations() == 2).await;
    assert!(rig.devices()[0].registered_at > first);
    connected_flock(core, &m).await;

    println!("a blocked agent raises approval.needed and one APNs alert, its context sealed");
    let t = Instant::now();
    let rev = core.approval_feed(m.clone(), 0).unwrap().revision;
    rig.herdr.set_status("blocked");
    let (a, rev) = needed(core, &m, rev).await;
    assert_eq!(a.terminal_id, TERMINAL);
    assert_eq!(a.agent_label, "api-fixer");
    assert_eq!(a.workspace_label, "api");
    assert_eq!(a.snippet, SNIPPET);
    assert_eq!(
        a.options,
        [
            ApprovalDecision::Approve,
            ApprovalDecision::ApproveAlways,
            ApprovalDecision::Deny
        ]
    );
    let alerts = rig.apns.wait(1).await;
    assert_eq!(alerts.len(), 1);
    let (device, alert) = &alerts[0];
    assert_eq!(device.token.as_str(), TOKEN);
    assert_eq!(device.environment, ApnsEnvironment::Sandbox);
    assert_eq!(
        phone_opens(&alert.payload),
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
            "approval_id": a.approval_id,
            "node_id": machine.node_id,
        })
    );
    assert_eq!(alert.collapse_id.as_deref(), Some(TERMINAL));
    let wire = alert.payload.to_string();
    for line in SNIPPET.lines() {
        assert!(!wire.contains(line), "snippet in the APNs payload: {wire}");
    }
    assert!(!wire.contains(&encoded_key()));
    let nonce = rig.probe.nonce(&a.approval_id).await;
    let feed = core.approval_feed(m.clone(), 0).unwrap();
    assert!(!format!("{feed:?}").contains(&nonce));
    for (_, alert) in &alerts {
        assert!(!alert.payload.to_string().contains(&nonce));
    }
    println!("  needed in {:?}", t.elapsed());

    println!("a second session deciding while the first is sending keys is refused");
    let t = Instant::now();
    rig.herdr.hold(true);
    let (applied, (raced, keys_while_held)) = tokio::join!(
        core.decide(
            m.clone(),
            a.approval_id.clone(),
            ApprovalDecision::Approve,
            None
        ),
        async {
            wait_for("the first decide to reach herdr", || rig.herdr.held() == 1).await;
            let raced = core
                .decide_from_notification(
                    machine.node_id.clone(),
                    a.approval_id.clone(),
                    ApprovalDecision::Approve,
                    Some(BUDGET_MS),
                )
                .await;
            let keys = rig.herdr.params("agent.send_keys").len();
            rig.herdr.hold(false);
            (raced, keys)
        },
    );
    assert_eq!(
        raced.outcome,
        BackgroundOutcome::AlreadyResolved,
        "{raced:?}"
    );
    assert!(raced.node_was_running);
    assert!(!format!("{raced:?}").contains(&nonce));
    assert_eq!(keys_while_held, 0);
    assert_eq!(
        applied.unwrap(),
        DecisionOutcome::Applied {
            decision: ApprovalDecision::Approve,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": PANE, "keys": ["enter"]})]
    );
    assert_eq!(rig.herdr.status(), "working");
    let (rev, outcome) = resolved(core, &m, rev, &a.approval_id).await;
    assert!(
        matches!(outcome, DecisionOutcome::Applied { .. }),
        "{outcome:?}"
    );

    println!("replaying the used nonce is refused and audited");
    let replay = rig.probe.decide(&a.approval_id, &nonce).await;
    assert_eq!(replay, Err(ErrorCode::ApprovalAlreadyResolved));
    let err = core
        .decide(
            m.clone(),
            a.approval_id.clone(),
            ApprovalDecision::Approve,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::ApprovalNotFound), "{err:?}");
    assert_eq!(
        rig.decisions(),
        [
            "approve: rejected: replayed".into(),
            format!("approve: applied terminal={TERMINAL} keys=enter"),
            "approve: rejected: replayed".into(),
        ]
    );
    let last = rig
        .audit()
        .into_iter()
        .rfind(|l| l["method"] == "approval.decide");
    assert_eq!(last.unwrap()["peer"], PROBE);
    assert_eq!(rig.herdr.params("agent.send_keys").len(), 1);
    println!("  decided in {:?}", t.elapsed());

    println!("a prompt that changes before the decision supersedes it");
    rig.herdr.set_status("blocked");
    let (b, rev) = needed(core, &m, rev).await;
    rig.herdr
        .with(|h| h.text = BASH.replace("rm -rf build", "rm -rf ~"));
    let outcome = core
        .decide(
            m.clone(),
            b.approval_id.clone(),
            ApprovalDecision::Approve,
            None,
        )
        .await
        .unwrap();
    assert_eq!(outcome, DecisionOutcome::Superseded);
    assert_eq!(rig.herdr.params("agent.send_keys").len(), 1);
    assert_eq!(
        rig.decisions().last().unwrap(),
        "approve: superseded: fingerprint mismatch"
    );
    let (rev, outcome) = resolved(core, &m, rev, &b.approval_id).await;
    assert_eq!(outcome, DecisionOutcome::Superseded);
    let (c, _) = needed(core, &m, rev).await;
    assert!(c.snippet.contains("rm -rf ~"), "{c:?}");
    let alerts = rig.apns.wait(3).await;
    assert_eq!(alerts.len(), 3, "the changed prompt alerts again");
    assert_eq!(alerts[2].1.payload["approval_id"], c.approval_id.as_str());
    let lock_screen = rig.probe.nonce(&c.approval_id).await;
    (machine, phone_id, [nonce, lock_screen])
}

async fn after_restart(rig: &Rig, core: &Arc<CollieCore>, m: &str, phone_id: &str, node: &str) {
    println!("the restarted app registers again from its Keychain key");
    let before = rig.registrations();
    connected_flock(core, m).await;
    core.register_push(
        m.into(),
        TOKEN.into(),
        PushEnvironment::Sandbox,
        NOTIFY_KEY.to_vec(),
    )
    .unwrap();
    wait_for("push.register from the new process", || {
        rig.registrations() == before + 1
    })
    .await;
    assert_eq!(rig.devices()[0].stable_id, phone_id);
    assert_eq!(
        rig.decisions(),
        [
            "approve: rejected: replayed".into(),
            format!("approve: applied terminal={TERMINAL} keys=enter"),
            "approve: rejected: replayed".into(),
            "approve: superseded: fingerprint mismatch".into(),
            format!("approve: applied terminal={TERMINAL} keys=enter"),
        ]
    );

    println!("deny sends the agent's own deny key");
    let rev = core.approval_feed(m.into(), 0).unwrap().revision;
    rig.herdr.set_status("blocked");
    let (e, rev) = needed(core, m, rev).await;
    let outcome = core
        .decide(
            m.into(),
            e.approval_id.clone(),
            ApprovalDecision::Deny,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        outcome,
        DecisionOutcome::Applied {
            decision: ApprovalDecision::Deny,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys").last(),
        Some(&json!({"target": PANE, "keys": ["esc"]}))
    );
    assert_eq!(
        rig.decisions().last().unwrap(),
        &format!("deny: applied terminal={TERMINAL} keys=esc")
    );
    let (rev, outcome) = resolved(core, m, rev, &e.approval_id).await;
    assert!(
        matches!(outcome, DecisionOutcome::Applied { .. }),
        "{outcome:?}"
    );

    println!("a revoked phone can no longer decide or receive alerts");
    rig.herdr.set_status("blocked");
    let (d, _) = needed(core, m, rev).await;
    rig.apns.wait(5).await;
    let revoked = collied::control::request(
        &rig.control,
        &Request::PeersRevoke {
            target: LABEL.into(),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(&revoked, Some(Reply::Revoked { peer, .. }) if peer.stable_id == phone_id),
        "{revoked:?}"
    );
    assert!(rig.devices().is_empty(), "the token outlived the revoke");
    assert_eq!(
        files_containing(&rig.data_dir, &encoded_key()),
        Vec::<PathBuf>::new(),
        "the notification key outlived the revoke"
    );
    let keys = rig.herdr.params("agent.send_keys").len();
    let err = core
        .decide(
            m.into(),
            d.approval_id.clone(),
            ApprovalDecision::Approve,
            None,
        )
        .await
        .unwrap_err();
    println!("  in-app decide: {err:?}");
    let report = core
        .decide_from_notification(
            node.into(),
            d.approval_id.clone(),
            ApprovalDecision::Approve,
            Some(BUDGET_MS),
        )
        .await;
    println!("  lock-screen decide: {report:?}");
    assert!(
        matches!(report.outcome, BackgroundOutcome::Unauthorized { .. }),
        "{report:?}"
    );
    rig.herdr.set_status("blocked");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !rig
        .probe
        .pending()
        .await
        .iter()
        .any(|p| p.approval_id.as_str() != d.approval_id)
    {
        assert!(
            Instant::now() < deadline,
            "no new approval after the revoke"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        rig.apns.wait(6).await.len(),
        5,
        "a revoked phone got an alert"
    );
    assert_eq!(rig.herdr.params("agent.send_keys").len(), keys);
    assert_eq!(
        rig.decisions().len(),
        6,
        "a revoked phone reached a decision"
    );
    assert_eq!(
        rig.herdr.mutations(),
        ["agent.send_keys", "agent.send_keys", "agent.send_keys"]
    );
    assert_eq!(kernel_tcp_listeners(), Vec::<String>::new());
}

async fn start_collied(
    net: &Net,
    data_dir: &Path,
    herdr: &Mock,
    apns: Arc<Apns>,
    approval_ttl: Duration,
) -> ServerHandle {
    server::start_with(
        net.mac.clone(),
        ServerConfig {
            data_dir: data_dir.to_owned(),
            port: PORT,
            owner_user_id: None,
            herdr_session: "e2e".into(),
            machine_name: "e2e-mac".into(),
            approval_ttl,
            attachments_dir: data_dir.join("attachments"),
            terminals: false,
            terminal_grant_ttl: collied::terminal::GRANT_TTL,
        },
        herdr.socket.clone(),
        &TasksConfig::default(),
        Some(apns),
    )
    .await
    .unwrap()
}

fn encoded_key() -> String {
    URL_SAFE_NO_PAD.encode(NOTIFY_KEY)
}

/// What ColliePush does with `enc`: CryptoKit `ChaChaPoly.SealedBox(combined:)`, AAD =
/// approval_id.
fn phone_opens(payload: &Value) -> Value {
    let combined = STANDARD.decode(payload["enc"].as_str().unwrap()).unwrap();
    let (nonce, sealed) = combined.split_at(12);
    let plain = ChaCha20Poly1305::new_from_slice(&NOTIFY_KEY)
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

fn reachability(group: &Path, node: &str) -> Value {
    let text = std::fs::read_to_string(group.join("reachability.json")).unwrap();
    serde_json::from_str::<Value>(&text).unwrap()[node].clone()
}

fn assert_private(path: &Path) {
    let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "{}", path.display());
}

async fn feed_until(
    core: &CollieCore,
    m: &str,
    rev: u64,
    what: &str,
    mut done: impl FnMut(&ApprovalFeed) -> bool,
) -> ApprovalFeed {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(feed) = core.approval_feed(m.into(), rev)
            && done(&feed)
        {
            return feed;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn needed(core: &CollieCore, m: &str, rev: u64) -> (PendingApproval, u64) {
    let feed = feed_until(core, m, rev, "approval.needed", |f| {
        f.events
            .iter()
            .any(|e| matches!(e, ApprovalEvent::Needed { .. }))
    })
    .await;
    assert!(!feed.missed);
    let approval = feed
        .events
        .into_iter()
        .find_map(|e| match e {
            ApprovalEvent::Needed { approval } => Some(approval),
            _ => None,
        })
        .unwrap();
    assert!(feed.pending.contains(&approval), "{:?}", feed.pending);
    (approval, feed.revision)
}

async fn resolved(core: &CollieCore, m: &str, rev: u64, id: &str) -> (u64, DecisionOutcome) {
    let feed = feed_until(core, m, rev, "approval.resolved", |f| {
        f.events
            .iter()
            .any(|e| matches!(e, ApprovalEvent::Resolved { approval_id, .. } if approval_id == id))
    })
    .await;
    assert!(feed.pending.iter().all(|p| p.approval_id != id));
    let outcome = feed
        .events
        .into_iter()
        .find_map(|e| match e {
            ApprovalEvent::Resolved {
                approval_id,
                outcome,
            } if approval_id == id => Some(outcome),
            _ => None,
        })
        .unwrap();
    (feed.revision, outcome)
}

async fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            hits.extend(files_containing(&path, needle));
        } else if kind.is_file()
            && std::fs::read(&path)
                .is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes()))
        {
            hits.push(path);
        }
    }
    hits
}

type Ws = WebSocketStream<ProbeStream>;

struct Probe {
    node: Node,
    target: String,
    data_dir: PathBuf,
}

impl Probe {
    async fn session(&self) -> Ws {
        let stream = {
            let (node, target) = (self.node.clone(), self.target.clone());
            // netstack occasionally stalls one SYN for ~63 s; bounded retries keep the test fast.
            tokio::task::spawn_blocking(move || {
                (0..6)
                    .find_map(|_| {
                        node.dial_timeout("tcp", &target, Duration::from_secs(10))
                            .ok()
                    })
                    .expect("probe could not dial collied")
            })
            .await
            .unwrap()
        };
        stream.set_nonblocking(true).unwrap();
        let mut req = format!("ws://{}{}", self.target, protocol::WS_PATH)
            .into_client_request()
            .unwrap();
        req.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static(protocol::WS_SUBPROTOCOL),
        );
        let stream = probe_tls(UnixStream::from_std(stream).unwrap(), &self.data_dir).await;
        let (mut ws, _) = tokio_tungstenite::client_async(req, stream).await.unwrap();
        let hello = json!({"protocol_version": protocol::PROTOCOL_VERSION, "app_version": "e2e"});
        call(&mut ws, "hello", hello).await.unwrap();
        ws
    }

    async fn pair(&self, control: &Path) {
        let mut cli = Client::connect(control).await.unwrap();
        let Reply::Invite { uri, .. } = cli.call(&Request::Pair).await.unwrap() else {
            panic!("no invite");
        };
        let code = PairingInvite::parse(&uri).unwrap().code;
        let mut ws = self.session().await;
        let unpaired = call(&mut ws, "approval.list", json!({})).await;
        assert_eq!(unpaired, Err(ErrorCode::NotPaired));
        let mut ws = self.session().await;
        let params = json!({"pairing_code": code.as_str(), "device_label": PROBE});
        let (paired, ()) = tokio::join!(call(&mut ws, "pair.complete", params), async {
            let Reply::Confirm(candidate) = cli.recv().await.unwrap() else {
                panic!("no confirmation request");
            };
            assert_eq!(candidate.device_label, PROBE);
            cli.send(&Request::Confirm { accept: true }).await.unwrap();
            assert!(matches!(
                cli.recv().await.unwrap(),
                Reply::PairDone { paired: true, .. }
            ));
        });
        assert!(matches!(paired, Ok(Response::Paired { .. })), "{paired:?}");
    }

    async fn pending(&self) -> Vec<Approval> {
        let mut ws = self.session().await;
        match call(&mut ws, "approval.list", json!({})).await {
            Ok(Response::Approvals { approvals }) => approvals,
            other => panic!("approval.list: {other:?}"),
        }
    }

    async fn nonce(&self, id: &str) -> String {
        let pending = self.pending().await;
        let approval = pending.iter().find(|a| a.approval_id.as_str() == id);
        approval.unwrap().nonce.as_str().to_owned()
    }

    async fn decide(&self, id: &str, nonce: &str) -> Result<Response, ErrorCode> {
        let mut ws = self.session().await;
        let params = json!({"approval_id": id, "decision": "approve", "nonce": nonce});
        call(&mut ws, "approval.decide", params).await
    }
}

async fn call(ws: &mut Ws, method: &str, params: Value) -> Result<Response, ErrorCode> {
    let frame = json!({"id": 1, "method": method, "params": params});
    ws.send(Message::text(frame.to_string())).await.unwrap();
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(15), ws.next())
            .await
            .expect("no frame within 15 s")
            .expect("connection ended")
            .expect("websocket error");
        let Message::Text(text) = msg else { continue };
        match serde_json::from_str::<ServerFrame>(text.as_str()).unwrap() {
            ServerFrame::Result { result, .. } => return Ok(result),
            ServerFrame::Error { error, .. } => return Err(error.code),
            ServerFrame::Event { .. } => {}
        }
    }
}

#[derive(Default)]
struct Apns {
    sent: Mutex<Vec<(Device, Alert)>>,
}

impl Apns {
    /// Everything but the background clears, which follow each agent that moves on.
    fn alerts(&self) -> Vec<(Device, Alert)> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, a)| a.delivery != Delivery::Background)
            .cloned()
            .collect()
    }

    async fn wait(&self, n: usize) -> Vec<(Device, Alert)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.alerts().len() < n && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.alerts()
    }

    fn last(&self) -> Alert {
        self.alerts().last().unwrap().1.clone()
    }
}

impl Sender for Apns {
    fn send<'a>(
        &'a self,
        device: &'a Device,
        alert: &'a Alert,
    ) -> Pin<Box<dyn Future<Output = Result<(), Rejection>> + Send + 'a>> {
        Box::pin(async move {
            self.sent
                .lock()
                .unwrap()
                .push((device.clone(), alert.clone()));
            Ok(())
        })
    }
}

struct Herdr {
    agent: Value,
    text: String,
    calls: Vec<(String, Value)>,
    held: usize,
    down: bool,
    gone: bool,
}

struct Mock {
    state: Arc<Mutex<Herdr>>,
    socket: PathBuf,
    hold: watch::Sender<bool>,
}

impl Mock {
    fn start(socket: &Path) -> Self {
        let state = Arc::new(Mutex::new(Herdr {
            agent: json!({
                "terminal_id": TERMINAL, "workspace_id": "w7", "pane_id": PANE,
                "tab_id": "w7:t1", "focused": false, "revision": 0,
                "agent": "claude", "name": "api-fixer", "agent_status": "working",
                "state_change_seq": 1, "cwd": "/Users/me/src/api",
                "agent_session": {"source": "herdr:claude", "agent": "claude", "kind": "id", "value": "sess-1"},
            }),
            text: BASH.into(),
            calls: Vec::new(),
            held: 0,
            down: false,
            gone: false,
        }));
        let (hold, gate) = watch::channel(false);
        let listener = UnixListener::bind(socket).unwrap();
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let shared = state.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let state = shared.clone();
                let mut gate = gate.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut line = String::new();
                    if tokio::io::BufReader::new(r)
                        .read_line(&mut line)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let req: Value = serde_json::from_str(&line).unwrap();
                    if req["method"] == "agent.send_keys" && *gate.borrow() {
                        state.lock().unwrap().held += 1;
                        let _ = gate.wait_for(|held| !*held).await;
                    }
                    let mut resp = json!({ "id": req["id"] });
                    match answer(&mut state.lock().unwrap(), &req) {
                        Ok(result) => resp["result"] = result,
                        Err(code) => resp["error"] = json!({ "code": code, "message": "mock" }),
                    }
                    let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
                });
            }
        });
        Self {
            state,
            socket: socket.to_owned(),
            hold,
        }
    }

    fn hold(&self, on: bool) {
        self.hold.send_replace(on);
    }

    fn held(&self) -> usize {
        self.with(|h| h.held)
    }

    fn with<R>(&self, f: impl FnOnce(&mut Herdr) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    fn set_status(&self, status: &str) {
        self.with(|h| set_status(h, status));
    }

    fn status(&self) -> String {
        self.with(|h| h.agent["agent_status"].as_str().unwrap().to_owned())
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

    fn mutations(&self) -> Vec<String> {
        self.with(|h| {
            h.calls
                .iter()
                .map(|(m, _)| m.clone())
                .filter(|m| MUTATING.contains(&m.as_str()))
                .collect()
        })
    }
}

fn set_status(h: &mut Herdr, status: &str) {
    h.agent["agent_status"] = json!(status);
    let seq = h.agent["state_change_seq"].as_u64().unwrap();
    h.agent["state_change_seq"] = json!(seq + 1);
}

fn answer(h: &mut Herdr, req: &Value) -> Result<Value, String> {
    let method = req["method"].as_str().unwrap_or_default().to_owned();
    let p = req["params"].clone();
    h.calls.push((method.clone(), p.clone()));
    if h.down {
        return Err("unavailable".into());
    }
    let agents = if h.gone { json!([]) } else { json!([h.agent]) };
    let workspace = json!({"workspace_id": "w7", "number": 1, "label": "api", "focused": false,
        "pane_count": 1, "tab_count": 1, "active_tab_id": "w7:t1",
        "agent_status": h.agent["agent_status"]});
    Ok(match method.as_str() {
        "ping" => json!({"type": "pong", "version": "0.9.3", "protocol": 1}),
        "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
            "workspaces": [workspace],
            "panes": [{"pane_id": PANE, "terminal_id": TERMINAL, "workspace_id": "w7",
                "tab_id": "w7:t1", "cwd": "/Users/me/src/api"}],
            "agents": agents,
        }}),
        "agent.list" => json!({"type": "agent_list", "agents": agents}),
        "workspace.list" => json!({"type": "workspace_list", "workspaces": [workspace]}),
        "agent.get" if p["target"] == PANE || p["target"] == TERMINAL => {
            json!({"type": "agent_info", "agent": h.agent})
        }
        "agent.explain" => json!({"type": "agent_explain", "explain": {
            "agent": "claude", "state": h.agent["agent_status"],
            "matched_rule": {"id": "bash_permission_prompt", "priority": 850,
                "region": "whole_recent", "state": "blocked"},
        }}),
        "pane.read" if p["source"] == "detection" => json!({"type": "pane_read", "read": {
            "pane_id": p["pane_id"], "workspace_id": "w7", "tab_id": "w7:t1",
            "source": "detection", "format": "text", "text": h.text, "revision": 0,
            "truncated": false,
        }}),
        "agent.send_keys" if p["target"] == PANE => {
            set_status(h, "working");
            json!({"type": "ok"})
        }
        _ => return Err("unknown_method".into()),
    })
}
