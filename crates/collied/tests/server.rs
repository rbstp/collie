use std::io::{BufRead, BufReader};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use collied::control::{Client, Reply, Request};
use collied::server::{self, ServerConfig};
use futures_util::{SinkExt, StreamExt};
use protocol::{AgentStatus, ErrorCode, Event, PairingInvite, Response, ServerFrame};
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
const WATCHDOG: Duration = Duration::from_secs(300);
const HERDR_CALLED: [&str; 9] = [
    "ping",
    "session.snapshot",
    "agent.list",
    "workspace.list",
    "agent.read",
    "agent.focus",
    "agent.get",
    "agent.explain",
    "pane.read",
];

type Ws = WebSocketStream<UnixStream>;

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
        mac,
        ServerConfig {
            data_dir: data_dir.clone(),
            port: PORT,
            owner_user_id: None,
            herdr_session: "default".into(),
            machine_name: "it-mac".into(),
            approval_ttl: collied::approvals::TTL,
            attachments_dir: data_dir.join("attachments"),
        },
        herdr_socket,
    )
    .await
    .unwrap();
    let control = handle.control_path();
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
    assert_eq!(seq, 2);
    assert!(matches!(
        event,
        Event::AgentStatus { agent } if agent.terminal_id.as_str() == "term_65ce7ae4fd5731" && agent.status == AgentStatus::Idle
    ));
    send(
        &mut ws,
        json!({"id": 6, "method": "flock.snapshot", "params": {}}),
    )
    .await;
    assert!(matches!(result(recv(&mut ws).await), Response::Flock(f) if f.seq == 2));

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
    ] {
        assert!(audit.contains(needle), "audit log lacks {needle}:\n{audit}");
    }
    assert!(
        !audit.contains(NOTIFY_KEY),
        "notification key in the audit log"
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

async fn connect(node: &Node, target: &str, path: &str, proto: Option<&str>) -> Result<Ws, String> {
    let stream = {
        let node = node.clone();
        let target = target.to_owned();
        tokio::task::spawn_blocking(move || dial(&node, &target))
            .await
            .unwrap()
    };
    stream.set_nonblocking(true).unwrap();
    let stream = UnixStream::from_std(stream).unwrap();
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
    let out = Command::new("lsof")
        .args([
            "-nP",
            "-a",
            "-p",
            &std::process::id().to_string(),
            "-iTCP",
            "-sTCP:LISTEN",
        ])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .map(str::to_owned)
        .collect()
}

struct MockHerdr {
    snapshot: Arc<Mutex<Value>>,
    methods: Arc<Mutex<Vec<String>>>,
    snapshot_delay: Arc<Mutex<Duration>>,
}

impl MockHerdr {
    fn start(path: &Path) -> Self {
        let fixture: Value =
            serde_json::from_str(include_str!("fixtures/session.snapshot.json")).unwrap();
        let snapshot = Arc::new(Mutex::new(fixture["result"]["snapshot"].clone()));
        let methods = Arc::new(Mutex::new(Vec::new()));
        let snapshot_delay = Arc::new(Mutex::new(Duration::ZERO));
        let listener = UnixListener::bind(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (snap, seen, delay) = (snapshot.clone(), methods.clone(), snapshot_delay.clone());
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let (snap, seen, delay) = (snap.clone(), seen.clone(), delay.clone());
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
            snapshot_delay,
        }
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
