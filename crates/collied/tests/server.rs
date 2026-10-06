use std::io::{BufRead, BufReader};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use collie_tls::rustls::sign::CertifiedKey;
use collie_tls::{Sniff, client::TlsStream};
use collied::control::{Client, Reply, Request};
use collied::server::{self, ServerConfig};
use futures_util::{SinkExt, StreamExt};
use protocol::{AgentStatus, ErrorCode, Event, KeyPin, PairingInvite, Response, ServerFrame};
use serde_json::{Value, json};
use tailnet::{BackendState, Config, Node, Status};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, header};
use zeroize::Zeroizing;

const KNOBS: [(&str, &str); 1] = [("TS_DISABLE_PORTMAPPER", "1")];
const PORT: u16 = 8457;
const NOTIFY_KEY: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";
const ACTIVITY: &str = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";
const ACTIVITY_TOKEN: &str =
    "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
const WATCHDOG: Duration = Duration::from_secs(300);
const HERDR_CALLED: [&str; 12] = [
    "ping",
    "session.snapshot",
    "agent.list",
    "workspace.list",
    "agent.read",
    "pane.layout",
    "agent.focus",
    "agent.get",
    "agent.explain",
    "pane.read",
    "pane.send_input",
    "pane.send_keys",
];
const SHELL: &str = "term_ffffffffffff01";
const SECRET: &str = "echo do-not-log-this";

type Ws = WebSocketStream<TlsStream<Sniff<UnixStream>>>;

static TLS: OnceLock<(KeyPin, Arc<CertifiedKey>)> = OnceLock::new();

fn tls_key() -> Arc<CertifiedKey> {
    collie_tls::certified(collie_tls::load(&collie_tls::generate().unwrap()).unwrap()).unwrap()
}

fn machine_pin(data_dir: &Path) -> KeyPin {
    let file: Value =
        serde_json::from_slice(&std::fs::read(data_dir.join("tls-key.json")).unwrap()).unwrap();
    let der = base64::engine::general_purpose::STANDARD
        .decode(file["pkcs8"].as_str().unwrap())
        .unwrap();
    let key = collie_tls::certified(collie_tls::load(&der).unwrap()).unwrap();
    collie_tls::pin(key.cert[0].as_ref())
}

#[test]
fn server_end_to_end() {
    // libtailscale reads TS_* knobs once at load, so they must be in the environment
    // before this process starts (same approach as crates/tailnet/tests/end_to_end.rs).
    if KNOBS
        .iter()
        .any(|(k, v)| std::env::var(k).as_deref() != Ok(*v))
    {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["server_end_to_end", "--exact", "--nocapture"])
            .envs(KNOBS)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        print!("{stdout}");
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "child run failed: {}", out.status);
        assert!(stdout.contains("1 passed"), "child did not run the test");
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        eprintln!("watchdog: test still running after {WATCHDOG:?}");
        std::process::exit(101);
    });

    let auth_key = format!("test-authkey-colliedit{}", std::process::id());
    let control = TestControl::start(&auth_key);
    let root = TempDir(
        PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("collied-it-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&root.0);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&root.0)
        .unwrap();

    let mac = start_node(&root.0, "it-mac", &control.url, &auth_key);
    let phone = start_node(&root.0, "it-phone", &control.url, &auth_key);
    let intruder = start_node(&root.0, "it-intruder", &control.url, &auth_key);
    let mac_st = wait_ready(&mac, 2);
    let phone_st = wait_ready(&phone, 2);
    let intruder_st = wait_ready(&intruder, 2);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(scenario(
        root.0.clone(),
        mac,
        mac_st,
        (phone, phone_st),
        (intruder, intruder_st),
    ));
    drop(control);
}

async fn scenario(
    root: PathBuf,
    mac: Node,
    mac_st: Status,
    (phone, phone_st): (Node, Status),
    (intruder, intruder_st): (Node, Status),
) {
    let mac_self = mac_st.self_node.unwrap();
    let phone_self = phone_st.self_node.unwrap();
    let intruder_self = intruder_st.self_node.unwrap();
    assert_ne!(phone_self.user_id, intruder_self.user_id);
    let mac_ip = mac_self
        .tailscale_ips
        .unwrap()
        .into_iter()
        .find(|ip| ip.is_ipv4())
        .unwrap();
    let target = format!("{mac_ip}:{PORT}");

    let herdr_socket = root.join("herdr.sock");
    let herdr = MockHerdr::start(&herdr_socket);
    let data_dir = root.join("data");
    let handle = server::start(
        mac.clone(),
        ServerConfig {
            data_dir: data_dir.clone(),
            port: PORT,
            owner_user_id: None,
            herdr_session: "default".into(),
            machine_name: "it-mac".into(),
            approval_ttl: collied::approvals::TTL,
            attachments_dir: data_dir.join("attachments"),
            terminals: false,
            terminal_grant_ttl: collied::terminal::GRANT_TTL,
        },
        herdr_socket.clone(),
    )
    .await
    .unwrap();
    let control = handle.control_path();
    assert_eq!(mode(&data_dir.join("tls-key.json")), 0o600);
    let phone_key = tls_key();
    TLS.set((machine_pin(&data_dir), phone_key.clone()))
        .unwrap();
    assert_eq!(mode(&control), 0o600);
    assert_eq!(mode(&data_dir), 0o700);
    let offline = collied::control::revoke_offline(&data_dir, "nobody").unwrap_err();
    assert!(
        matches!(
            offline.downcast_ref::<collied::peers::Error>(),
            Some(collied::peers::Error::Locked(_))
        ),
        "{offline:#}"
    );

    println!("unpaired phone without a window is rejected before the upgrade");
    assert!(
        connect(
            &phone,
            &target,
            protocol::WS_PATH,
            Some(protocol::WS_SUBPROTOCOL)
        )
        .await
        .is_err()
    );

    println!("open a pairing window");
    let mut cli = Client::connect(&control).await.unwrap();
    let Reply::Invite { uri, .. } = cli.call(&Request::Pair).await.unwrap() else {
        panic!("no invite");
    };
    let invite = PairingInvite::parse(&uri).unwrap();
    assert_eq!(invite.key, machine_pin(&data_dir));
    assert_eq!(invite.node_id, mac_self.stable_id);
    assert_eq!(invite.host, mac_self.dns_name.trim_end_matches('.'));
    assert_eq!(invite.port, PORT);
    let mut second = Client::connect(&control).await.unwrap();
    assert!(matches!(
        second.call(&Request::Pair).await.unwrap(),
        Reply::Error { .. }
    ));

    println!("wrong path and subprotocol are rejected");
    for (path, proto) in [
        ("/nope", Some(protocol::WS_SUBPROTOCOL)),
        (protocol::WS_PATH, Some("collie.v0")),
        (protocol::WS_PATH, None),
    ] {
        assert!(
            connect(&phone, &target, path, proto).await.is_err(),
            "{path} {proto:?}"
        );
    }

    println!("first frame must be hello");
    let mut ws = open(&phone, &target).await;
    send(
        &mut ws,
        json!({"id": 1, "method": "flock.snapshot", "params": {}}),
    )
    .await;
    assert_error(&recv(&mut ws).await, ErrorCode::HelloRequired);
    assert_closed(&mut ws).await;

    let mut ws = open(&phone, &target).await;
    send(&mut ws, hello_frame(99)).await;
    assert_error(&recv(&mut ws).await, ErrorCode::UnsupportedProtocol);
    assert_closed(&mut ws).await;

    println!("pairing-only session refuses anything but pair.complete");
    let mut ws = open(&phone, &target).await;
    send(&mut ws, hello_frame(protocol::PROTOCOL_VERSION)).await;
    assert!(matches!(
        result(recv(&mut ws).await),
        Response::Hello(h) if !h.paired
            && h.herdr_version.is_none()
            && h.machine.node_id == mac_self.stable_id
    ));
    send(
        &mut ws,
        json!({"id": 2, "method": "flock.snapshot", "params": {}}),
    )
    .await;
    assert_error(&recv(&mut ws).await, ErrorCode::NotPaired);
    assert_closed(&mut ws).await;

    println!("a pairing-only session ends with the window that admitted it");
    let mut ws = open(&phone, &target).await;
    send(&mut ws, hello_frame(protocol::PROTOCOL_VERSION)).await;
    result(recv(&mut ws).await);
    drop(cli);
    assert_closed(&mut ws).await;
    let (mut cli, uri) = open_window(&control).await;
    let invite = PairingInvite::parse(&uri).unwrap();

    println!("a pairing-only session closes on an invalid frame");
    let mut ws = open(&phone, &target).await;
    send(&mut ws, hello_frame(protocol::PROTOCOL_VERSION)).await;
    result(recv(&mut ws).await);
    send(&mut ws, json!({"id": 2, "method": "no.such", "params": {}})).await;
    assert_error(&recv(&mut ws).await, ErrorCode::UnknownMethod);
    assert_closed(&mut ws).await;

    println!("pair with confirmation on the Mac");
    let mut ws = open(&phone, &target).await;
    send(&mut ws, hello_frame(protocol::PROTOCOL_VERSION)).await;
    result(recv(&mut ws).await);
    send(&mut ws, pair_frame(invite.code.as_str())).await;
    let Reply::Confirm(candidate) = cli.recv().await.unwrap() else {
        panic!("no confirmation request");
    };
    assert_eq!(candidate.stable_id, phone_self.stable_id);
    assert_eq!(candidate.user_id, phone_self.user_id);
    assert_eq!(candidate.device_label, "Test iPhone");
    cli.send(&Request::Confirm { accept: true }).await.unwrap();
    assert!(matches!(
        cli.recv().await.unwrap(),
        Reply::PairDone { paired: true, .. }
    ));
    assert!(matches!(
        result(recv(&mut ws).await),
        Response::Paired { machine } if machine.node_id == mac_self.stable_id
    ));
    assert_closed(&mut ws).await;
    drop(cli);
    drop(second);
    assert_eq!(mode(&data_dir.join("peers.json")), 0o600);
    let Some(Reply::Peers {
        owner_user_id,
        peers,
    }) = collied::control::request(&control, &Request::PeersList)
        .await
        .unwrap()
    else {
        panic!("no peers reply");
    };
    assert_eq!(owner_user_id, Some(phone_self.user_id));
    assert_eq!(peers.len(), 1);
    assert_eq!(
        peers[0].tls_key,
        Some(collie_tls::pin(phone_key.cert[0].as_ref()))
    );

    println!("the paired phone must present the key it paired with");
    let other = (TLS.get().unwrap().0.clone(), tls_key());
    let refused = connect_as(&phone, &target, &other).await;
    assert!(refused.is_err(), "an unpinned phone key gets no session");
    let Some(Reply::Status(status)) = collied::control::request(&control, &Request::Status)
        .await
        .unwrap()
    else {
        panic!("no status reply");
    };
    assert_eq!(status.backend_state, "Running");
    assert_eq!(status.node_id, mac_self.stable_id);
    assert_eq!((status.port, status.peers), (PORT, 1));
    assert_eq!(status.herdr_version.as_deref(), Some("0.9.3"));
    // The test node advertises no tag: reported, and empty.
    assert_eq!(status.tags, Some(Vec::new()));
    assert!(!status.flock_too_large);
    let flock = status.flock.expect("status lists the herdr agents");
    let terminals: Vec<&str> = flock
        .agents
        .iter()
        .map(|a| a.terminal_id.as_str())
        .collect();
    assert_eq!(terminals, ["term_65ce7ae4fd5731", "term_0a1b2c3d4e5f60"]);
    assert_eq!(flock.workspaces.len(), 2);
    for a in &flock.agents {
        assert!(
            flock
                .workspaces
                .iter()
                .any(|w| w.workspace_id == a.workspace_id),
            "{a:?}"
        );
    }

    println!("another user is rejected even while a window is open");
    let (cli, _) = open_window(&control).await;
    assert!(
        connect(
            &intruder,
            &target,
            protocol::WS_PATH,
            Some(protocol::WS_SUBPROTOCOL)
        )
        .await
        .is_err()
    );
    drop(cli);

    println!("full session");
    // The first reconcile, about a second after start, creates the blocked agent's approval.
    while !herdr.methods().iter().any(|m| m == "pane.read") {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut ws = open(&phone, &target).await;
    send(&mut ws, hello_frame(protocol::PROTOCOL_VERSION)).await;
    let Response::Hello(hello) = result(recv(&mut ws).await) else {
        panic!("no hello");
    };
    assert!(hello.paired);
    assert_eq!(hello.herdr_version.as_deref(), Some("0.9.3"));
    send(
        &mut ws,
        json!({"id": 3, "method": "flock.snapshot", "params": {}}),
    )
    .await;
    let Response::Flock(flock) = result(recv(&mut ws).await) else {
        panic!("no flock");
    };
    assert_eq!(flock.seq, 0);
    assert_eq!(flock.agents.len(), 2);
    assert_eq!(flock.workspaces.len(), 2);
    assert_eq!(
        flock.approvals.len(),
        1,
        "the blocked agent has an approval"
    );
    assert_eq!(flock.machine.node_id, mac_self.stable_id);
    send(
        &mut ws,
        json!({"id": 4, "method": "workspace.list", "params": {}}),
    )
    .await;
    assert!(matches!(
        result(recv(&mut ws).await),
        Response::Workspaces { workspaces } if workspaces.len() == 2
    ));
    send(
        &mut ws,
        json!({"id": 5, "method": "agent.focus", "params": {"terminal_id": "term_65ce7ae4fd5731"}}),
    )
    .await;
    assert_eq!(result(recv(&mut ws).await), Response::Ok);
    send(
        &mut ws,
        json!({"id": 50, "method": "approval.list", "params": {}}),
    )
    .await;
    let Response::Approvals { approvals } = result(recv(&mut ws).await) else {
        panic!("no approvals");
    };
    assert_eq!(approvals, flock.approvals);
    let approval = &approvals[0];
    assert_eq!(approval.terminal_id.as_str(), "term_0a1b2c3d4e5f60");
    assert!(approval.options.is_empty(), "no mapping for this agent");
    assert_eq!(approval.snippet, "Allow command?\nrm -rf build");
    send(
        &mut ws,
        json!({"id": 56, "method": "push.register", "params": {
            "apns_token": "ab".repeat(32), "environment": "sandbox",
            "notification_key": NOTIFY_KEY}}),
    )
    .await;
    assert_eq!(result(recv(&mut ws).await), Response::Ok);
    assert_eq!(mode(&data_dir.join("push.json")), 0o600);
    send(
        &mut ws,
        json!({"id": 57, "method": "push.activity_token", "params": {
            "activity_id": ACTIVITY, "terminal_id": "term_0a1b2c3d4e5f60",
            "token": ACTIVITY_TOKEN}}),
    )
    .await;
    assert_eq!(result(recv(&mut ws).await), Response::Ok);
    let stored = std::fs::read_to_string(data_dir.join("push.json")).unwrap();
    assert!(stored.contains(ACTIVITY_TOKEN), "{stored}");
    send(
        &mut ws,
        json!({"id": 58, "method": "push.activity_end", "params": {"activity_id": ACTIVITY}}),
    )
    .await;
    assert_eq!(result(recv(&mut ws).await), Response::Ok);
    let stored = std::fs::read_to_string(data_dir.join("push.json")).unwrap();
    assert!(!stored.contains(ACTIVITY_TOKEN), "{stored}");

    println!("drive refusals go through the op cache and the task runner");
    let blocked = json!({"id": 53, "method": "agent.prompt", "params": {
        "op_id": "AAAAAAAAAAAAAAAAAAAAAA", "terminal_id": "term_0a1b2c3d4e5f60", "text": "go"}});
    send(&mut ws, blocked.clone()).await;
    assert_error(&recv(&mut ws).await, ErrorCode::AgentBlocked);
    send(&mut ws, blocked).await;
    assert_error(&recv(&mut ws).await, ErrorCode::AgentBlocked);
    send(
        &mut ws,
        json!({"id": 54, "method": "task.new", "params": {
            "op_id": "BBBBBBBBBBBBBBBBBBBBBB", "cwd": "/", "agent": "claude", "prompt": "go"}}),
    )
    .await;
    let frame = recv(&mut ws).await;
    assert!(
        matches!(&frame, ServerFrame::Error { id: Some(54), error } if error.code == ErrorCode::InvalidParams),
        "{frame:?}"
    );

    println!("agent.watch pushes sanitized output as events");
    send(
        &mut ws,
        json!({"id": 51, "method": "agent.watch", "params": {"terminal_id": "term_65ce7ae4fd5731"}}),
    )
    .await;
    assert_eq!(result(recv(&mut ws).await), Response::Ok);
    let ServerFrame::Event { seq, event } = recv(&mut ws).await else {
        panic!("expected an event");
    };
    assert_eq!(seq, 1);
    assert!(
        matches!(&event, Event::AgentOutput(read) if read.ansi == "\u{1b}[1mhi\u{1b}[0m\r\n"),
        "{event:?}"
    );
    assert_eq!(herdr.read_lines()[0], json!(200), "default watch depth");
    send(
        &mut ws,
        json!({"id": 55, "method": "agent.watch", "params": {"terminal_id": "term_65ce7ae4fd5731", "lines": 5000}}),
    )
    .await;
    assert_eq!(result(recv(&mut ws).await), Response::Ok);
    assert!(matches!(
        recv(&mut ws).await,
        ServerFrame::Event {
            seq: 2,
            event: Event::AgentOutput(_)
        }
    ));
    assert_eq!(herdr.read_lines().last(), Some(&json!(1000)), "clamped");
    send(
        &mut ws,
        json!({"id": 52, "method": "agent.watch", "params": {"terminal_id": null}}),
    )
    .await;
    assert_eq!(result(recv(&mut ws).await), Response::Ok);

    // The change must land after the reconcile task has taken its first baseline.
    while !herdr.methods().iter().any(|m| m == "workspace.list") {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    herdr.set_status(0, "idle");
    let ServerFrame::Event { seq, event } = recv(&mut ws).await else {
        panic!("expected an event");
    };
    assert_eq!(seq, 3);
    assert!(matches!(
        event,
        Event::AgentStatus { agent } if agent.terminal_id.as_str() == "term_65ce7ae4fd5731" && agent.status == AgentStatus::Idle
    ));
    send(
        &mut ws,
        json!({"id": 6, "method": "flock.snapshot", "params": {}}),
    )
    .await;
    assert!(matches!(result(recv(&mut ws).await), Response::Flock(f) if f.seq == 3));

    println!("terminals are off by default: no shells listed, every terminal method refused");
    send(
        &mut ws,
        json!({"id": 60, "method": "flock.snapshot", "params": {}}),
    )
    .await;
    let Response::Flock(flock) = result(recv(&mut ws).await) else {
        panic!("no flock");
    };
    assert!(flock.terminals.is_empty() && !flock.terminals_enabled);
    send(
        &mut ws,
        json!({"id": 61, "method": "pane.read", "params": {"terminal_id": SHELL, "source": "visible"}}),
    )
    .await;
    assert_error(&recv(&mut ws).await, ErrorCode::NotFound);
    for (method, params) in terminal_frames(SHELL) {
        send(
            &mut ws,
            json!({"id": 62, "method": method, "params": params}),
        )
        .await;
        assert_error(&recv(&mut ws).await, ErrorCode::TerminalsDisabled);
    }
    send(
        &mut ws,
        json!({"id": 63, "method": "terminal.lock", "params": {}}),
    )
    .await;
    assert_eq!(result(recv(&mut ws).await), Response::Ok);
    assert!(!herdr.methods().iter().any(|m| m.starts_with("pane.send")));

    println!("a fifth session from one node evicts its oldest");
    let mut extra = Vec::new();
    for _ in 0..3 {
        let mut s = open(&phone, &target).await;
        send(&mut s, hello_frame(protocol::PROTOCOL_VERSION)).await;
        result(recv(&mut s).await);
        extra.push(s);
    }
    let mut newest = open(&phone, &target).await;
    send(&mut newest, hello_frame(protocol::PROTOCOL_VERSION)).await;
    result(recv(&mut newest).await);
    assert_closed(&mut ws).await;
    let mut ws = newest;

    println!("revocation drops an in-flight reply and closes every live session");
    let snapshots = || {
        herdr
            .methods()
            .iter()
            .filter(|m| *m == "session.snapshot")
            .count()
    };
    let before = snapshots();
    herdr.set_snapshot_delay(Duration::from_secs(3));
    send(
        &mut ws,
        json!({"id": 7, "method": "flock.snapshot", "params": {}}),
    )
    .await;
    while snapshots() == before {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let revoked = collied::control::request(
        &control,
        &Request::PeersRevoke {
            target: "Test iPhone".into(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        revoked,
        Some(Reply::Revoked {
            closed_sessions: 4,
            ..
        })
    ));
    assert_closed(&mut ws).await;
    for s in &mut extra {
        assert_closed(s).await;
    }
    herdr.set_snapshot_delay(Duration::ZERO);
    assert!(
        connect(
            &phone,
            &target,
            protocol::WS_PATH,
            Some(protocol::WS_SUBPROTOCOL)
        )
        .await
        .is_err()
    );

    println!("a wrong code burns the window");
    let (mut cli, _) = open_window(&control).await;
    let mut ws = open(&phone, &target).await;
    send(&mut ws, hello_frame(protocol::PROTOCOL_VERSION)).await;
    result(recv(&mut ws).await);
    send(&mut ws, pair_frame("AAAAAAAAAAAAAAAAAAAAAA")).await;
    assert_error(&recv(&mut ws).await, ErrorCode::PairingFailed);
    assert!(matches!(
        cli.recv().await.unwrap(),
        Reply::PairDone { paired: false, .. }
    ));
    assert!(
        connect(
            &phone,
            &target,
            protocol::WS_PATH,
            Some(protocol::WS_SUBPROTOCOL)
        )
        .await
        .is_err()
    );

    println!("no kernel TCP listener, private files, read-only herdr use");
    assert_eq!(kernel_tcp_listeners(), Vec::<String>::new());
    let audit_path = data_dir.join("audit.log");
    assert_eq!(mode(&audit_path), 0o600);
    let audit = std::fs::read_to_string(&audit_path).unwrap();
    for needle in [
        "rejected: not paired",
        "rejected: not the owner",
        "per-node limit",
        "rejected: websocket handshake",
        "\"method\":\"pair.complete\"",
        "\"result\":\"paired\"",
        "\"result\":\"wrong code\"",
        "\"method\":\"peers.revoke\"",
        "\"method\":\"agent.focus\"",
        "\"target\":\"term_65ce7ae4fd5731\"",
        "\"method\":\"agent.watch\"",
        "\"result\":\"agent_blocked: agent is blocked; answer it through an approval (replayed)\"",
        "\"result\":\"invalid_params: cwd is outside the allowed roots\"",
        "\"result\":\"hello_required\"",
        "\"result\":\"unsupported_protocol\"",
        "\"result\":\"not_paired\"",
        "\"method\":\"push.register\"",
        "\"method\":\"push.activity_token\"",
        "\"target\":\"term_0a1b2c3d4e5f60 activity=3F2504E0-4F89-11D3-9A0C-0305E82C3301\"",
        "\"method\":\"push.activity_end\"",
        "\"result\":\"terminals_disabled: terminals are off on this machine; set [terminals] enabled = true in collied.toml and restart collied\"",
        "\"method\":\"terminal.challenge\"",
        "\"method\":\"terminal.run\"",
        "\"method\":\"terminal.lock\"",
    ] {
        assert!(audit.contains(needle), "audit log lacks {needle}:\n{audit}");
    }
    assert!(
        !audit.contains(NOTIFY_KEY),
        "notification key in the audit log"
    );
    assert!(
        !audit.contains(ACTIVITY_TOKEN),
        "activity token in the audit log"
    );
    for line in audit.lines() {
        let entry: Value = serde_json::from_str(line).unwrap();
        assert!(
            !(matches!(
                entry["method"].as_str(),
                Some("hello" | "flock.snapshot" | "workspace.list")
            ) && entry["result"] == "ok"),
            "{line}"
        );
    }
    let called = herdr.methods();
    assert!(
        called.iter().all(|m| HERDR_CALLED.contains(&m.as_str())),
        "{called:?}"
    );
    handle.shutdown().await;
    assert!(!control.exists());

    terminal_phase(mac, &data_dir, &herdr_socket, &herdr, &phone, &target).await;
}

fn terminal_frames(terminal: &str) -> Vec<(&'static str, Value)> {
    let op = "AAAAAAAAAAAAAAAAAAAAAA";
    vec![
        ("terminal.challenge", json!({"terminal_id": terminal})),
        (
            "terminal.grant",
            json!({"terminal_id": terminal, "challenge": "A".repeat(43), "signature": "MEUCIQDxyzAB"}),
        ),
        ("terminal.watch", json!({"terminal_id": terminal})),
        (
            "terminal.run",
            json!({"op_id": op, "terminal_id": terminal, "text": SECRET}),
        ),
        (
            "terminal.send_keys",
            json!({"op_id": op, "terminal_id": terminal, "keys": ["ctrl+c"]}),
        ),
    ]
}

struct TerminalKey(Arc<dyn collie_tls::rustls::sign::SigningKey>, String);

impl TerminalKey {
    fn new() -> Self {
        let key = collie_tls::load(&collie_tls::generate().unwrap()).unwrap();
        let spki = key.public_key().unwrap().as_ref().to_vec();
        Self(key, B64.encode(spki))
    }

    fn sign(&self, node_id: &str, terminal: &str, challenge: &protocol::Nonce) -> String {
        let message = protocol::terminal_grant_message(
            node_id,
            &protocol::TerminalId::new(terminal).unwrap(),
            challenge,
        );
        let signer = self.0.choose_scheme(&[collie_tls::SCHEME]).unwrap();
        B64.encode(signer.sign(&message).unwrap())
    }
}

/// Pairs the phone again through a window, with `terminal_key`, and returns what the
/// machine's y/N prompt showed.
async fn pair_again(
    control: &Path,
    phone: &Node,
    target: &str,
    terminal_key: Option<&str>,
) -> collied::control::Candidate {
    let (mut cli, uri) = open_window(control).await;
    let invite = PairingInvite::parse(&uri).unwrap();
    let mut ws = open(phone, target).await;
    send(&mut ws, hello_frame(protocol::PROTOCOL_VERSION)).await;
    result(recv(&mut ws).await);
    let mut frame = pair_frame(invite.code.as_str());
    if let Some(k) = terminal_key {
        frame["params"]["terminal_key"] = json!(k);
    }
    send(&mut ws, frame).await;
    let Reply::Confirm(candidate) = cli.recv().await.unwrap() else {
        panic!("no confirmation request");
    };
    cli.send(&Request::Confirm { accept: true }).await.unwrap();
    assert!(matches!(
        cli.recv().await.unwrap(),
        Reply::PairDone { paired: true, .. }
    ));
    assert!(matches!(
        result(recv(&mut ws).await),
        Response::Paired { .. }
    ));
    candidate
}

async fn hello(phone: &Node, target: &str) -> Ws {
    let mut ws = open(phone, target).await;
    send(&mut ws, hello_frame(protocol::PROTOCOL_VERSION)).await;
    result(recv(&mut ws).await);
    ws
}

async fn call(ws: &mut Ws, method: &str, params: Value) -> ServerFrame {
    send(ws, json!({"id": 70, "method": method, "params": params})).await;
    loop {
        match recv(ws).await {
            ServerFrame::Event { .. } => {}
            frame => return frame,
        }
    }
}

fn locked(frame: &ServerFrame, why: &str) {
    assert!(
        matches!(frame, ServerFrame::Error { error, .. }
            if error.code == ErrorCode::TerminalLocked && error.message.contains(why)),
        "expected terminal_locked ({why}), got {frame:?}"
    );
}

async fn challenge(ws: &mut Ws, terminal: &str) -> protocol::Nonce {
    let Response::TerminalChallenge {
        terminal_id,
        challenge,
        ttl_ms,
    } = result(call(ws, "terminal.challenge", json!({"terminal_id": terminal})).await)
    else {
        panic!("no challenge");
    };
    assert_eq!((terminal_id.as_str(), ttl_ms), (terminal, 60_000));
    challenge
}

async fn grant(ws: &mut Ws, key: &TerminalKey, node_id: &str, terminal: &str) -> ServerFrame {
    let c = challenge(ws, terminal).await;
    let signature = key.sign(node_id, terminal, &c);
    call(
        ws,
        "terminal.grant",
        json!({"terminal_id": terminal, "challenge": c, "signature": signature}),
    )
    .await
}

fn run_frame(op: char, text: &str) -> Value {
    json!({"op_id": op.to_string().repeat(22), "terminal_id": SHELL, "text": text})
}

/// The same machine started again with `[terminals] enabled = true` and a short grant.
async fn terminal_phase(
    mac: Node,
    data_dir: &Path,
    herdr_socket: &Path,
    herdr: &MockHerdr,
    phone: &Node,
    target: &str,
) {
    let node_id = mac.status().unwrap().self_node.unwrap().stable_id;
    let handle = server::start(
        mac,
        ServerConfig {
            data_dir: data_dir.to_owned(),
            port: PORT,
            owner_user_id: None,
            herdr_session: "default".into(),
            machine_name: "it-mac".into(),
            approval_ttl: collied::approvals::TTL,
            attachments_dir: data_dir.join("attachments"),
            terminals: true,
            terminal_grant_ttl: Duration::from_secs(3),
        },
        herdr_socket.to_owned(),
    )
    .await
    .unwrap();
    let control = handle.control_path();
    let key = TerminalKey::new();

    println!("pairing refuses the TLS key as the terminal key");
    let tls = TLS.get().unwrap().1.cert[0].as_ref().to_vec();
    let (cli, uri) = open_window(&control).await;
    let invite = PairingInvite::parse(&uri).unwrap();
    let mut ws = hello(phone, target).await;
    let mut frame = pair_frame(invite.code.as_str());
    frame["params"]["terminal_key"] = json!(B64.encode(tls));
    send(&mut ws, frame).await;
    assert_error(&recv(&mut ws).await, ErrorCode::PairingFailed);
    drop(cli);

    println!("pairing records the terminal key, shown on the machine's prompt");
    let candidate = pair_again(&control, phone, target, Some(&key.1)).await;
    assert_eq!(
        candidate.terminal_key.as_ref().map(|k| k.as_str()),
        Some(key.1.as_str())
    );
    assert_eq!(
        (candidate.replaces, candidate.terminal_key_change()),
        (false, "new")
    );
    let stored = |control: &Path| {
        let control = control.to_owned();
        async move {
            let Some(Reply::Peers { peers, .. }) =
                collied::control::request(&control, &Request::PeersList)
                    .await
                    .unwrap()
            else {
                panic!("no peers");
            };
            peers[0]
                .terminal_key
                .as_ref()
                .map(|k| k.as_str().to_owned())
        }
    };
    assert_eq!(stored(&control).await.as_deref(), Some(key.1.as_str()));

    println!("the snapshot lists shell panes, by label or cwd only");
    let mut ws = hello(phone, target).await;
    let Response::Flock(flock) = result(call(&mut ws, "flock.snapshot", json!({})).await) else {
        panic!("no flock");
    };
    assert!(flock.terminals_enabled);
    let shells: Vec<&str> = flock
        .terminals
        .iter()
        .map(|t| t.terminal_id.as_str())
        .collect();
    assert_eq!(shells, [SHELL]);

    println!("no input without a grant, and none into an agent's pane");
    locked(
        &call(&mut ws, "terminal.run", run_frame('A', SECRET)).await,
        "locked",
    );
    locked(
        &call(&mut ws, "terminal.watch", json!({"terminal_id": SHELL})).await,
        "locked",
    );
    assert_error(
        &call(
            &mut ws,
            "terminal.challenge",
            json!({"terminal_id": "term_65ce7ae4fd5731"}),
        )
        .await,
        ErrorCode::NotFound,
    );

    println!("a challenge is single use, bound to its terminal, and needs the key's signature");
    let c = challenge(&mut ws, SHELL).await;
    let other = TerminalKey::new();
    locked(
        &call(
            &mut ws,
            "terminal.grant",
            json!({"terminal_id": SHELL, "challenge": c, "signature": other.sign(&node_id, SHELL, &c)}),
        )
        .await,
        "signature did not verify",
    );
    locked(
        &call(
            &mut ws,
            "terminal.grant",
            json!({"terminal_id": SHELL, "challenge": c, "signature": key.sign(&node_id, SHELL, &c)}),
        )
        .await,
        "no challenge was issued",
    );
    let c = challenge(&mut ws, SHELL).await;
    locked(
        &call(
            &mut ws,
            "terminal.grant",
            json!({"terminal_id": "term_0a1b2c3d4e5f60", "challenge": c,
                "signature": key.sign(&node_id, "term_0a1b2c3d4e5f60", &c)}),
        )
        .await,
        "does not match",
    );
    let c = challenge(&mut ws, SHELL).await;
    locked(
        &call(
            &mut ws,
            "terminal.grant",
            json!({"terminal_id": SHELL, "challenge": c, "signature": key.sign("nOTHER", SHELL, &c)}),
        )
        .await,
        "signature did not verify",
    );

    println!("granted: watch, run and keys");
    assert!(matches!(
        result(grant(&mut ws, &key, &node_id, SHELL).await),
        Response::TerminalGranted { ttl_ms: 3000, .. }
    ));
    assert_eq!(
        result(call(&mut ws, "terminal.watch", json!({"terminal_id": SHELL})).await),
        Response::Ok
    );
    let ServerFrame::Event {
        event: Event::AgentOutput(read),
        ..
    } = recv(&mut ws).await
    else {
        panic!("no terminal output");
    };
    assert_eq!(read.terminal_id.as_str(), SHELL);
    let before = herdr.inputs().len();
    assert_eq!(
        result(call(&mut ws, "terminal.run", run_frame('B', SECRET)).await),
        Response::Ok
    );
    assert_eq!(
        result(
            call(
                &mut ws,
                "terminal.send_keys",
                json!({"op_id": "C".repeat(22), "terminal_id": SHELL, "keys": ["ctrl+c"]})
            )
            .await
        ),
        Response::Ok
    );
    assert_eq!(herdr.inputs()[before..], [json!(SECRET), json!(["ctrl+c"])]);

    println!("a grant belongs to its session");
    let mut second = hello(phone, target).await;
    locked(
        &call(&mut second, "terminal.run", run_frame('D', "ls")).await,
        "locked",
    );
    drop(second);

    println!("terminal.lock ends every grant of the session");
    assert_eq!(
        result(call(&mut ws, "terminal.lock", json!({})).await),
        Response::Ok
    );
    locked(
        &call(&mut ws, "terminal.run", run_frame('E', "ls")).await,
        "locked",
    );

    println!("a grant ends after its ttl");
    result(grant(&mut ws, &key, &node_id, SHELL).await);
    assert_eq!(
        result(call(&mut ws, "terminal.run", run_frame('F', "ls")).await),
        Response::Ok
    );
    tokio::time::sleep(Duration::from_millis(3100)).await;
    locked(
        &call(&mut ws, "terminal.run", run_frame('G', "ls")).await,
        "locked",
    );

    println!("pairing again, even with the same key, ends the grants of live sessions");
    result(grant(&mut ws, &key, &node_id, SHELL).await);
    let candidate = pair_again(&control, phone, target, Some(&key.1)).await;
    assert_eq!(
        (candidate.replaces, candidate.terminal_key_change()),
        (true, "unchanged")
    );
    locked(
        &call(&mut ws, "terminal.run", run_frame('I', "ls")).await,
        "locked",
    );

    println!("pairing again without a key forgets it: terminal_key_missing");
    result(grant(&mut ws, &key, &node_id, SHELL).await);
    let candidate = pair_again(&control, phone, target, None).await;
    assert_eq!(
        (candidate.replaces, candidate.terminal_key_change()),
        (true, "none")
    );
    assert_eq!(stored(&control).await, None);
    locked(
        &call(&mut ws, "terminal.run", run_frame('H', "ls")).await,
        "locked",
    );
    assert_error(
        &call(&mut ws, "terminal.challenge", json!({"terminal_id": SHELL})).await,
        ErrorCode::TerminalKeyMissing,
    );
    drop(ws);

    println!("typed text never reaches the audit log");
    let audit = std::fs::read_to_string(data_dir.join("audit.log")).unwrap();
    assert!(!audit.contains("do-not-log-this"), "{audit}");
    for needle in [
        "\"result\":\"invalid terminal key\"",
        "\"method\":\"terminal.grant\"",
        "\"result\":\"granted ttl=3s\"",
        "\"result\":\"terminal_locked: signature did not verify\"",
        "\"result\":\"terminal_key_missing: this phone has no terminal key here; pair it again\"",
        "\"method\":\"terminal.watch\"",
        "\"target\":\"term_ffffffffffff01 keys=ctrl+c\"",
    ] {
        assert!(audit.contains(needle), "audit log lacks {needle}:\n{audit}");
    }
    let challenges = audit
        .lines()
        .filter(|l| l.contains("\"terminal.challenge\"") && l.contains("\"result\":\"ok\""))
        .count();
    assert_eq!(challenges, 0, "a challenge that was issued is not audited");
    handle.shutdown().await;
}

fn hello_frame(version: u32) -> Value {
    json!({"id": 1, "method": "hello", "params": {"protocol_version": version, "app_version": "it"}})
}

fn pair_frame(code: &str) -> Value {
    json!({"id": 2, "method": "pair.complete", "params": {"pairing_code": code, "device_label": "Test iPhone"}})
}

// The daemon closes a cancelled window when it sees the previous CLI hang up.
async fn open_window(control: &Path) -> (Client, String) {
    for _ in 0..50 {
        let mut cli = Client::connect(control).await.unwrap();
        match cli.call(&Request::Pair).await.unwrap() {
            Reply::Invite { uri, .. } => return (cli, uri),
            Reply::Error { .. } => tokio::time::sleep(Duration::from_millis(100)).await,
            other => panic!("unexpected {other:?}"),
        }
    }
    panic!("pairing window never became available");
}

async fn open(node: &Node, target: &str) -> Ws {
    connect(
        node,
        target,
        protocol::WS_PATH,
        Some(protocol::WS_SUBPROTOCOL),
    )
    .await
    .unwrap()
}

async fn connect_as(
    node: &Node,
    target: &str,
    tls: &(KeyPin, Arc<CertifiedKey>),
) -> Result<Ws, String> {
    connect_with(
        node,
        target,
        protocol::WS_PATH,
        Some(protocol::WS_SUBPROTOCOL),
        tls,
    )
    .await
}

async fn connect(node: &Node, target: &str, path: &str, proto: Option<&str>) -> Result<Ws, String> {
    connect_with(node, target, path, proto, TLS.get().unwrap()).await
}

async fn connect_with(
    node: &Node,
    target: &str,
    path: &str,
    proto: Option<&str>,
    (pin, key): &(KeyPin, Arc<CertifiedKey>),
) -> Result<Ws, String> {
    let stream = {
        let node = node.clone();
        let target = target.to_owned();
        tokio::task::spawn_blocking(move || dial(&node, &target))
            .await
            .unwrap()
    };
    stream.set_nonblocking(true).unwrap();
    let stream = UnixStream::from_std(stream).unwrap();
    let stream = match collie_tls::connect(stream, "collie", pin.clone(), key.clone()).await {
        Ok(s) => s,
        Err(collie_tls::ConnectError::Forbidden) => return Err("HTTP 403".into()),
        Err(collie_tls::ConnectError::Tls(e)) => return Err(format!("tls: {e}")),
    };
    let mut req = format!("ws://{target}{path}")
        .into_client_request()
        .unwrap();
    if let Some(p) = proto {
        req.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_str(p).unwrap(),
        );
    }
    tokio::time::timeout(
        Duration::from_secs(20),
        tokio_tungstenite::client_async(req, stream),
    )
    .await
    .map_err(|_| "handshake timed out".to_owned())?
    .map(|(ws, _)| ws)
    .map_err(|e| e.to_string())
}

// netstack occasionally stalls one SYN for ~63 s; bounded retries keep the test fast.
fn dial(node: &Node, target: &str) -> std::os::unix::net::UnixStream {
    let mut last = None;
    for _ in 0..6 {
        match node.dial_timeout("tcp", target, Duration::from_secs(10)) {
            Ok(s) => return s,
            Err(e) => last = Some(e),
        }
    }
    panic!("dial {target}: {last:?}");
}

async fn send(ws: &mut Ws, frame: Value) {
    ws.send(Message::text(frame.to_string())).await.unwrap();
}

async fn recv(ws: &mut Ws) -> ServerFrame {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(15), ws.next())
            .await
            .expect("no frame within 15 s")
            .expect("connection ended")
            .expect("websocket error");
        if let Message::Text(t) = msg {
            return serde_json::from_str(t.as_str()).unwrap();
        }
    }
}

fn result(frame: ServerFrame) -> Response {
    match frame {
        ServerFrame::Result { result, .. } => result,
        other => panic!("expected a result, got {other:?}"),
    }
}

fn assert_error(frame: &ServerFrame, code: ErrorCode) {
    assert!(
        matches!(frame, ServerFrame::Error { error, .. } if error.code == code),
        "expected {code:?}, got {frame:?}"
    );
}

async fn assert_closed(ws: &mut Ws) {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next()).await {
            Ok(None | Some(Err(_)) | Some(Ok(Message::Close(_)))) => return,
            Ok(Some(Ok(Message::Text(t)))) => panic!("unexpected frame {t}"),
            Ok(Some(Ok(_))) => {}
            Err(_) => panic!("connection still open"),
        }
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

fn kernel_tcp_listeners() -> Vec<String> {
    tailnet::kernel_tcp_listeners(std::process::id()).expect("kernel TCP listeners")
}

struct MockHerdr {
    snapshot: Arc<Mutex<Value>>,
    methods: Arc<Mutex<Vec<String>>>,
    inputs: Arc<Mutex<Vec<Value>>>,
    read_lines: Arc<Mutex<Vec<Value>>>,
    snapshot_delay: Arc<Mutex<Duration>>,
}

impl MockHerdr {
    fn start(path: &Path) -> Self {
        let fixture: Value =
            serde_json::from_str(include_str!("fixtures/session.snapshot.json")).unwrap();
        let snapshot = Arc::new(Mutex::new(fixture["result"]["snapshot"].clone()));
        let methods = Arc::new(Mutex::new(Vec::new()));
        let read_lines = Arc::new(Mutex::new(Vec::new()));
        let snapshot_delay = Arc::new(Mutex::new(Duration::ZERO));
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let listener = UnixListener::bind(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (snap, seen, lines, delay, typed) = (
            snapshot.clone(),
            methods.clone(),
            read_lines.clone(),
            snapshot_delay.clone(),
            inputs.clone(),
        );
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let (snap, seen, lines, delay, typed) = (
                    snap.clone(),
                    seen.clone(),
                    lines.clone(),
                    delay.clone(),
                    typed.clone(),
                );
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut line = String::new();
                    tokio::io::BufReader::new(r)
                        .read_line(&mut line)
                        .await
                        .unwrap();
                    let req: Value = serde_json::from_str(&line).unwrap();
                    let method = req["method"].as_str().unwrap().to_owned();
                    seen.lock().unwrap().push(method.clone());
                    if method == "agent.read" {
                        lines.lock().unwrap().push(req["params"]["lines"].clone());
                    }
                    if method == "pane.send_input" {
                        assert_eq!(req["params"]["keys"], json!(["enter"]));
                        typed.lock().unwrap().push(req["params"]["text"].clone());
                    }
                    if method == "pane.send_keys" {
                        typed.lock().unwrap().push(req["params"]["keys"].clone());
                    }
                    if method == "session.snapshot" {
                        let delay = *delay.lock().unwrap();
                        tokio::time::sleep(delay).await;
                    }
                    let snap = snap.lock().unwrap().clone();
                    let result = match method.as_str() {
                        "ping" => json!({"type": "pong", "version": "0.9.3", "protocol": 22}),
                        "session.snapshot" => json!({"type": "session_snapshot", "snapshot": snap}),
                        "agent.list" => json!({"type": "agent_list", "agents": snap["agents"]}),
                        "workspace.list" => {
                            json!({"type": "workspace_list", "workspaces": snap["workspaces"]})
                        }
                        "agent.focus" => json!({"type": "agent_info", "agent": snap["agents"][0]}),
                        "agent.get" => json!({"type": "agent_info", "agent": snap["agents"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|a| a["pane_id"] == req["params"]["target"])
                            .unwrap()}),
                        "agent.explain" => json!({"type": "agent_explain", "explain": {
                            "agent": "codex", "state": "blocked",
                            "matched_rule": {"id": "live_strong_blocker"},
                        }}),
                        "pane.read" => json!({"type": "pane_read", "read": {
                            "pane_id": req["params"]["pane_id"], "workspace_id": "w7", "tab_id": "w7:t1",
                            "source": "detection", "format": "text", "revision": 0, "truncated": false,
                            "text": "› run it\n────────\nAllow command?\nrm -rf build\n",
                        }}),
                        "pane.send_input" | "pane.send_keys" => json!({"type": "ok"}),
                        "agent.read" => json!({"type": "pane_read", "read": {
                            "pane_id": req["params"]["target"], "workspace_id": "w6", "tab_id": "w6:t1",
                            "source": "recent", "format": "ansi", "revision": 0, "truncated": false,
                            "text": "\u{1b}[1mhi\u{1b}[0m\u{1b}]52;c;cm0gLXJmIH4=\u{7}\u{1b}[2J\r\n",
                        }}),
                        _ => Value::Null,
                    };
                    let resp = if result.is_null() {
                        json!({"id": req["id"], "error": {"code": "invalid_request", "message": "mock"}})
                    } else {
                        json!({"id": req["id"], "result": result})
                    };
                    let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
                });
            }
        });
        Self {
            snapshot,
            methods,
            inputs,
            read_lines,
            snapshot_delay,
        }
    }

    fn inputs(&self) -> Vec<Value> {
        self.inputs.lock().unwrap().clone()
    }

    fn set_snapshot_delay(&self, delay: Duration) {
        *self.snapshot_delay.lock().unwrap() = delay;
    }

    fn set_status(&self, agent: usize, status: &str) {
        let mut snap = self.snapshot.lock().unwrap();
        let a = &mut snap["agents"][agent];
        a["agent_status"] = json!(status);
        let seq = a["state_change_seq"].as_u64().unwrap();
        a["state_change_seq"] = json!(seq + 1);
    }

    fn methods(&self) -> Vec<String> {
        self.methods.lock().unwrap().clone()
    }

    fn read_lines(&self) -> Vec<Value> {
        self.read_lines.lock().unwrap().clone()
    }
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn start_node(root: &Path, name: &str, control_url: &str, auth_key: &str) -> Node {
    let node = Node::new(&Config {
        state_dir: root.join(name),
        hostname: name.into(),
        auth_key: Some(Zeroizing::new(auth_key.into())),
        control_url: Some(control_url.into()),
        advertise_tags: Vec::new(),
        log_to_stderr: false,
    })
    .unwrap();
    node.start().unwrap();
    node
}

fn wait_ready(node: &Node, peers: usize) -> Status {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let st = node.status().unwrap();
        let ready = st.backend_state == BackendState::Running
            && st
                .self_node
                .as_ref()
                .is_some_and(|s| s.tailscale_ips.as_ref().is_some_and(|ips| !ips.is_empty()))
            && st
                .peer
                .as_ref()
                .is_some_and(|p| p.len() >= peers && p.values().all(|p| p.tailscale_ips.is_some()));
        if ready {
            return st;
        }
        assert!(Instant::now() < deadline, "node not ready: {st:#?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct TestControl {
    child: Child,
    url: String,
}

impl TestControl {
    fn start(auth_key: &str) -> Self {
        let bin = Path::new(env!("CARGO_TARGET_TMPDIR")).join("collied-testcontrol");
        let status = Command::new(std::env::var("GO").unwrap_or_else(|_| "go".into()))
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tailnet/testcontrol"))
            .env("GOTOOLCHAIN", "local")
            .env("GOFLAGS", "-mod=readonly")
            .args(["build", "-o"])
            .arg(&bin)
            .arg(".")
            .status()
            .expect("go build testcontrol");
        assert!(status.success(), "go build testcontrol: {status}");
        // The helper exits when its stdin closes, so it dies with this process.
        let mut child = Command::new(&bin)
            .args(["-authkey", auth_key])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut url = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut url)
            .unwrap();
        let url = url.trim().to_owned();
        assert!(url.starts_with("http://127.0.0.1:"), "control url {url:?}");
        Self { child, url }
    }
}

impl Drop for TestControl {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
