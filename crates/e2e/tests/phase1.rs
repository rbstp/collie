#[allow(dead_code)]
mod common;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use collie_core::{AgentState, CollieCore, CoreError};
use collied::control::{Client, Reply, Request};
use collied::server;
use common::*;
use protocol::PairingInvite;
use tailnet::{Node, Status};

const PHONE_TAG: &str = "tag:collie-phone";
const READ_ONLY_HERDR: [&str; 6] = [
    "ping",
    "session.snapshot",
    "agent.list",
    "workspace.list",
    "agent.explain",
    "pane.read",
];

#[test]
fn phase1_end_to_end() {
    if !in_child("phase1_end_to_end") {
        return;
    }
    let t0 = Instant::now();
    let root = TempDir::new("e2e");
    let net = Net::start(&root.0);
    let tagged = start_node(
        &root.0,
        "e2e-tagged-phone",
        &net.key,
        &net.url,
        &[PHONE_TAG],
    );
    // The phones register after the Mac is up, so their first netmap already has it.
    wait_ready(&net.mac, 0);
    let phone_a = phone(&root.0, "phone-a", &net);
    let phone_b = phone(&root.0, "phone-b", &net);
    let rt = runtime();
    for core in [&phone_a, &phone_b] {
        rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    }
    let mac_st = wait_ready(&net.mac, 3);
    wait_ready(&tagged, 1);
    for core in [&phone_a, &phone_b] {
        wait_phone(&rt, core);
    }
    println!(
        "tailnet: control, Mac, 2 phones, tagged phone up in {:?}",
        t0.elapsed()
    );

    rt.block_on(scenario(
        &root.0, &net, &mac_st, &tagged, &phone_a, &phone_b,
    ));
    // Each core owns a runtime, which must not be dropped from async context.
    drop(phone_a);
    drop(phone_b);
    drop(rt);
    println!("total {:?}", t0.elapsed());
}

async fn scenario(
    root: &Path,
    net: &Net,
    mac_st: &Status,
    tagged: &Node,
    phone_a: &Arc<CollieCore>,
    phone_b: &Arc<CollieCore>,
) {
    let t = Instant::now();
    let mac_self = mac_st.self_node.clone().unwrap();
    let mac_host = mac_self.dns_name.trim_end_matches('.').to_owned();
    let herdr_socket = root.join("herdr.sock");
    let herdr_calls = mock_herdr(&herdr_socket, |_, _| None);
    let data_dir = root.join("collied");
    let statusline = include_str!("../../collied/tests/fixtures/statusline.json");
    collied::usage::record(&data_dir, statusline.as_bytes(), 1).unwrap();
    let handle = server::start(
        net.mac.clone(),
        server_config(&data_dir, "e2e"),
        herdr_socket,
    )
    .await
    .unwrap();
    let control = handle.control_path();
    let audit = data_dir.join("audit.log");

    println!("phone A pairs from the invite, confirmed on the Mac");
    let (machine, uri) = pair(&control, phone_a, LABEL).await;
    assert_eq!(machine.node_id, mac_self.stable_id);
    assert_eq!(machine.host, mac_host);
    assert_eq!(machine.port, PORT);
    assert_eq!(phone_a.machines(), vec![machine.clone()]);
    let Some(Reply::Peers {
        owner_user_id,
        peers,
    }) = collied::control::request(&control, &Request::PeersList)
        .await
        .unwrap()
    else {
        panic!("no peers reply");
    };
    assert_eq!(peers.len(), 1);
    let phone_a_id = peers[0].stable_id.clone();
    assert_eq!(owner_user_id, Some(peers[0].user_id));
    println!("  paired in {:?}", t.elapsed());

    println!("phone A sees the fixture flock keyed by terminal_id");
    let t = Instant::now();
    let flock = connected_flock(phone_a, &machine.id).await;
    let details = flock.details.clone().unwrap();
    assert_eq!(details.node_id, mac_self.stable_id);
    assert_eq!(details.name, "e2e-mac");
    assert_eq!(details.herdr_session, "e2e");
    let agents: BTreeMap<&str, _> = flock
        .agents
        .iter()
        .map(|a| {
            (
                a.terminal_id.as_str(),
                (
                    a.status,
                    a.kind.as_deref(),
                    a.workspace_id.as_str(),
                    a.title.as_deref(),
                ),
            )
        })
        .collect();
    assert_eq!(
        agents,
        BTreeMap::from([
            (
                "term_0a1b2c3d4e5f60",
                (
                    AgentState::Blocked,
                    Some("codex"),
                    "w7",
                    Some("Fix flaky test")
                )
            ),
            (
                "term_65ce7ae4fd5731",
                (
                    AgentState::Working,
                    Some("claude"),
                    "w6",
                    Some("Collie iOS remote control")
                )
            ),
        ])
    );
    let workspaces: Vec<(&str, &str)> = flock
        .workspaces
        .iter()
        .map(|w| (w.workspace_id.as_str(), w.label.as_str()))
        .collect();
    assert_eq!(workspaces, [("w6", "collie"), ("w7", "api")]);
    let plan = flock
        .plan_usage
        .expect("the snapshot carries the machine's plan usage");
    assert_eq!(plan.five_hour.map(|w| w.used_percent), Some(24));
    assert_eq!(
        plan.seven_day.map(|w| w.resets_at_ms),
        Some(1_738_857_600_000)
    );
    println!("  flock in {:?}", t.elapsed());

    println!("a newer plan reaches the phone while no agent's status changes");
    let t = Instant::now();
    collied::usage::record(&data_dir, statusline.replace("23.5", "57.0").as_bytes(), 2).unwrap();
    let five_hour = || {
        phone_a
            .cached_flock(machine.id.clone())
            .and_then(|f| f.plan_usage?.five_hour)
            .map(|w| w.used_percent)
    };
    while five_hour() != Some(57) {
        assert!(t.elapsed() < Duration::from_secs(10), "{:?}", five_hour());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    println!("  plan in {:?}", t.elapsed());

    let peers_of_mac = status(&net.mac).await.peer.unwrap_or_default();
    let phone_b_id = peers_of_mac
        .values()
        .find(|p| p.tags.as_deref().is_none_or(<[String]>::is_empty) && p.stable_id != phone_a_id)
        .map(|p| p.stable_id.clone())
        .expect("phone B in the Mac's netmap");
    let tagged_self = status(tagged).await.self_node.unwrap();

    println!("unpaired phone B is rejected before the upgrade");
    let t = Instant::now();
    let err = phone_b
        .pair(uri.clone(), "Other iPhone".into())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CoreError::Unauthorized { message } if message.contains("403")),
        "{err:?}"
    );
    assert!(phone_b.machines().is_empty());
    wait_audit(&audit, &phone_b_id, "connect", "rejected: not paired").await;
    println!("  rejected in {:?}", t.elapsed());

    println!("tagged phone is rejected before the upgrade, even with a window open");
    let t = Instant::now();
    let mut window = Client::connect(&control).await.unwrap();
    assert!(matches!(
        window.call(&Request::Pair).await.unwrap(),
        Reply::Invite { .. }
    ));
    let mac_ip = mac_self
        .tailscale_ips
        .clone()
        .unwrap()
        .into_iter()
        .find(|ip| ip.is_ipv4())
        .unwrap();
    let answered = {
        let (node, host) = (tagged.clone(), mac_host.clone());
        tokio::task::spawn_blocking(move || raw_upgrade(&node, &format!("{mac_ip}:{PORT}"), &host))
            .await
            .unwrap()
    };
    assert_eq!(
        String::from_utf8_lossy(&answered),
        "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        "a tagged node must get only the fixed refusal"
    );
    wait_audit(
        &audit,
        &tagged_self.stable_id,
        "connect",
        "rejected: tagged node",
    )
    .await;
    drop(window);
    wait_audit(&audit, "control", "pair.close", "cancelled").await;
    println!("  rejected in {:?}", t.elapsed());

    println!("phone refuses a Mac whose StableID differs from the invite, or without a collie tag");
    let PairingInvite { code, key, .. } = PairingInvite::parse(&uri).unwrap();
    let tagged_host = tagged_self.dns_name.trim_end_matches('.').to_owned();
    let before = audit_lines(&audit).len();
    for (host, node_id) in [
        (mac_host.clone(), tagged_self.stable_id.clone()),
        (tagged_host.clone(), mac_self.stable_id.clone()),
        (tagged_host, tagged_self.stable_id.clone()),
    ] {
        let forged = PairingInvite {
            host,
            port: PORT,
            node_id,
            key: key.clone(),
            code: code.clone(),
        }
        .to_uri();
        let err = phone_a.pair(forged, LABEL.into()).await.unwrap_err();
        assert!(matches!(err, CoreError::PinViolation { .. }), "{err:?}");
    }
    assert_eq!(phone_a.machines(), vec![machine.clone()]);
    assert_eq!(
        audit_lines(&audit).len(),
        before,
        "a pinned-out Mac must never be contacted"
    );

    println!("a phone the Mac still lists pairs again, as after removing an unreachable Mac");
    let err = phone_a.pair(uri.clone(), LABEL.into()).await.unwrap_err();
    assert!(format!("{err}").contains("already paired"), "{err}");
    let (machine, _) = pair(&control, phone_a, "Renamed iPhone").await;
    assert_eq!(machine.node_id, mac_self.stable_id);
    let Some(Reply::Peers { peers, .. }) = collied::control::request(&control, &Request::PeersList)
        .await
        .unwrap()
    else {
        panic!("no peers reply");
    };
    assert_eq!(peers.len(), 1, "the record is replaced, not added");
    assert_eq!(
        (peers[0].stable_id.as_str(), peers[0].label.as_str()),
        (phone_a_id.as_str(), "Renamed iPhone")
    );
    connected_flock(phone_a, &machine.id).await;

    println!("removing the Mac on the phone revokes the phone there");
    assert!(phone_a.remove_machine(machine.id.clone()).await.unwrap());
    wait_audit(
        &audit,
        "Renamed iPhone",
        "peers.revoke",
        "unpaired by the phone, 1 session(s) closed",
    )
    .await;
    let Some(Reply::Peers { peers, .. }) = collied::control::request(&control, &Request::PeersList)
        .await
        .unwrap()
    else {
        panic!("no peers reply");
    };
    assert!(peers.is_empty(), "{peers:?}");
    let (machine, _) = pair(&control, phone_a, "Renamed iPhone").await;
    connected_flock(phone_a, &machine.id).await;

    println!("revoked phone is closed and then rejected");
    let t = Instant::now();
    let revoked = collied::control::request(
        &control,
        &Request::PeersRevoke {
            target: "Renamed iPhone".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(&revoked, Some(Reply::Revoked { peer, closed_sessions: 1 }) if peer.stable_id == phone_a_id),
        "{revoked:?}"
    );
    phone_a.resume(60);
    wait_audit(&audit, &phone_a_id, "connect", "rejected: not paired").await;
    let after = phone_a.flock(machine.id.clone()).await.unwrap();
    assert!(!connected(&after), "{after:?}");
    println!("  rejected in {:?}", t.elapsed());

    println!("no kernel TCP listener, private audit log, read-only herdr use");
    assert_eq!(kernel_tcp_listeners(), Vec::<String>::new());
    let mode = std::fs::metadata(&audit).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let called = herdr_calls.lock().unwrap().clone();
    assert!(
        called.iter().all(|m| READ_ONLY_HERDR.contains(&m.as_str())),
        "{called:?}"
    );
    handle.shutdown().await;
}

async fn status(node: &Node) -> Status {
    let node = node.clone();
    tokio::task::spawn_blocking(move || node.status())
        .await
        .unwrap()
        .unwrap()
}

// Writes a WebSocket upgrade and returns whatever the Mac sent back before closing.
fn raw_upgrade(node: &Node, target: &str, host: &str) -> Vec<u8> {
    let mut conn = dial(node, target);
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {host}:{PORT}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Protocol: {}\r\n\r\n",
        protocol::WS_PATH,
        protocol::WS_SUBPROTOCOL
    );
    let _ = conn.write_all(request.as_bytes());
    // Darwin fails setsockopt with EINVAL once a refusing Mac has closed; the
    // read cannot block then.
    if let Err(e) = conn.set_read_timeout(Some(Duration::from_secs(10))) {
        assert_eq!(e.raw_os_error(), Some(22), "set_read_timeout: {e}");
    }
    let mut buf = Vec::new();
    match conn.read_to_end(&mut buf) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
        Err(e) => panic!("connection neither answered nor closed: {e}"),
    }
    buf
}

// netstack occasionally stalls one SYN for about 63 s; bounded retries keep the test fast.
fn dial(node: &Node, target: &str) -> StdUnixStream {
    let mut last = None;
    for _ in 0..6 {
        match node.dial_timeout("tcp", target, Duration::from_secs(10)) {
            Ok(s) => return s,
            Err(e) => last = Some(e),
        }
    }
    panic!("dial {target}: {last:?}");
}

async fn wait_audit(path: &Path, peer: &str, method: &str, result: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let lines = audit_lines(path);
        if lines
            .iter()
            .any(|e| e["peer"] == peer && e["method"] == method && e["result"] == result)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "audit log lacks {peer} {method} {result:?}:\n{lines:#?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
