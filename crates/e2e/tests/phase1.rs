use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use collie_core::{AgentState, CollieCore, CoreError, Machine, MachineFlock, TailnetState};
use collied::control::{Client, Reply, Request};
use collied::server::{self, ServerConfig};
use protocol::PairingInvite;
use serde_json::{Value, json};
use tailnet::{BackendState, Config, Node, Status};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use zeroize::Zeroizing;

const KNOBS: [(&str, &str); 1] = [("TS_DISABLE_PORTMAPPER", "1")];
const PORT: u16 = protocol::DEFAULT_PORT;
const WATCHDOG: Duration = Duration::from_secs(240);
const PHONE_TAG: &str = "tag:collie-phone";
const LABEL: &str = "E2E iPhone";
const READ_ONLY_HERDR: [&str; 4] = ["ping", "session.snapshot", "agent.list", "workspace.list"];

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
    let herdr_calls = mock_herdr(&herdr_socket);
    let data_dir = root.join("collied");
    let handle = server::start(
        net.mac.clone(),
        ServerConfig {
            data_dir: data_dir.clone(),
            port: PORT,
            owner_user_id: None,
            herdr_session: "e2e".into(),
            machine_name: "e2e-mac".into(),
        },
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
    println!("  flock in {:?}", t.elapsed());

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

    println!("phone refuses a Mac whose StableID differs from the invite");
    let code = PairingInvite::parse(&uri).unwrap().code;
    let tagged_host = tagged_self.dns_name.trim_end_matches('.').to_owned();
    let before = audit_lines(&audit).len();
    for (host, node_id) in [
        (mac_host.clone(), tagged_self.stable_id.clone()),
        (tagged_host, mac_self.stable_id.clone()),
    ] {
        let forged = PairingInvite {
            host,
            port: PORT,
            node_id,
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

    println!("revoked phone is closed and then rejected");
    let t = Instant::now();
    let revoked = collied::control::request(
        &control,
        &Request::PeersRevoke {
            target: LABEL.into(),
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

#[test]
fn live_herdr_session() {
    let installed = Command::new("herdr")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !installed {
        println!("skipped: herdr is not installed");
        return;
    }
    if !in_child("live_herdr_session") {
        return;
    }
    let t0 = Instant::now();
    let mut herdr = HerdrSession::start();
    println!(
        "dedicated herdr session {} up in {:?}",
        herdr.name,
        t0.elapsed()
    );
    let root = TempDir::new("e2e-herdr");
    let net = Net::start(&root.0);
    wait_ready(&net.mac, 0);
    let core = phone(&root.0, "phone", &net);
    let rt = runtime();
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    wait_ready(&net.mac, 1);
    wait_phone(&rt, &core);

    rt.block_on(async {
        let expected = collied::herdr::workspace_list(&herdr.socket).await.unwrap();
        assert!(
            expected.iter().any(|w| w.label == "collie-e2e"),
            "{expected:?}"
        );
        let data_dir = root.0.join("collied");
        let handle = server::start(
            net.mac.clone(),
            ServerConfig {
                data_dir,
                port: PORT,
                owner_user_id: None,
                herdr_session: herdr.name.clone(),
                machine_name: "e2e-mac".into(),
            },
            herdr.socket.clone(),
        )
        .await
        .unwrap();
        let (machine, _) = pair(&handle.control_path(), &core, LABEL).await;
        let flock = connected_flock(&core, &machine.id).await;
        assert_eq!(flock.details.unwrap().herdr_session, herdr.name);
        let got: Vec<(&str, &str)> = flock
            .workspaces
            .iter()
            .map(|w| (w.workspace_id.as_str(), w.label.as_str()))
            .collect();
        let want: Vec<(&str, &str)> = expected
            .iter()
            .map(|w| (w.workspace_id.as_str(), w.label.as_str()))
            .collect();
        assert_eq!(got, want);
        handle.shutdown().await;
    });
    drop(core);
    drop(rt);
    assert!(herdr.stop(), "dedicated herdr session did not stop");
    println!("total {:?}", t0.elapsed());
}

// libtailscale's Go runtime reads TS_* knobs once at load, so each test re-runs itself
// in a child process that has them (same approach as crates/tailnet/tests/end_to_end.rs).
fn in_child(test: &str) -> bool {
    if KNOBS
        .iter()
        .all(|(k, v)| std::env::var(k).as_deref() == Ok(*v))
    {
        std::thread::spawn(|| {
            std::thread::sleep(WATCHDOG);
            eprintln!("watchdog: test still running after {WATCHDOG:?}");
            std::process::exit(101);
        });
        return true;
    }
    let out = Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture"])
        .envs(KNOBS)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    print!("{stdout}");
    eprint!("{}", String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "child run failed: {}", out.status);
    assert!(stdout.contains("1 passed"), "child did not run the test");
    false
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn pair(control: &Path, core: &Arc<CollieCore>, label: &str) -> (Machine, String) {
    let mut cli = Client::connect(control).await.unwrap();
    let Reply::Invite { uri, .. } = cli.call(&Request::Pair).await.unwrap() else {
        panic!("no invite");
    };
    let pairing = tokio::spawn({
        let (core, uri, label) = (core.clone(), uri.clone(), label.to_owned());
        async move { core.pair(uri, label).await }
    });
    let reply = tokio::time::timeout(Duration::from_secs(30), cli.recv())
        .await
        .expect("no pairing attempt within 30 s")
        .unwrap();
    let Reply::Confirm(candidate) = reply else {
        panic!("expected a confirmation request, got {reply:?}");
    };
    assert_eq!(candidate.device_label, label);
    cli.send(&Request::Confirm { accept: true }).await.unwrap();
    assert!(matches!(
        cli.recv().await.unwrap(),
        Reply::PairDone { paired: true, .. }
    ));
    (pairing.await.unwrap().unwrap(), uri)
}

fn connected(f: &MachineFlock) -> bool {
    format!("{:?}", f.link) == "Connected"
}

async fn connected_flock(core: &CollieCore, machine_id: &str) -> MachineFlock {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let f = core.flock(machine_id.into()).await.unwrap();
        if connected(&f) && f.details.is_some() {
            return f;
        }
        assert!(Instant::now() < deadline, "never connected: {f:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
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
    conn.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
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

fn audit_lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
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

// One request per connection, like herdr. Fixtures are sanitized live samples.
fn mock_herdr(path: &Path) -> Arc<Mutex<Vec<String>>> {
    let listener = UnixListener::bind(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let seen = seen.clone();
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
                let method = req["method"].as_str().unwrap_or_default().to_owned();
                seen.lock().unwrap().push(method.clone());
                let fixture = match method.as_str() {
                    "ping" => Some(include_str!("fixtures/ping.json")),
                    "session.snapshot" => Some(include_str!("fixtures/session.snapshot.json")),
                    "agent.list" => Some(include_str!("fixtures/agent.list.json")),
                    "workspace.list" => Some(include_str!("fixtures/workspace.list.json")),
                    _ => None,
                };
                let mut resp = match fixture {
                    Some(text) => serde_json::from_str(text).unwrap(),
                    None => {
                        json!({"error": {"code": "unknown_method", "message": "not mocked"}})
                    }
                };
                resp["id"] = req["id"].clone();
                let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
            });
        }
    });
    calls
}

struct Net {
    _control: TestControl,
    key: String,
    url: String,
    mac: Node,
}

impl Net {
    fn start(root: &Path) -> Self {
        let key = format!("tskey-auth-colliee2e{}", std::process::id());
        let control = TestControl::start(&key, root);
        let url = control.url.clone();
        let mac = start_node(
            root,
            "collie-e2e-mac",
            &key,
            &url,
            &[collied::daemon::MAC_TAG],
        );
        Self {
            _control: control,
            key,
            url,
            mac,
        }
    }
}

fn start_node(root: &Path, name: &str, key: &str, url: &str, tags: &[&str]) -> Node {
    let node = Node::new(&Config {
        state_dir: root.join(name),
        hostname: name.into(),
        auth_key: Some(Zeroizing::new(key.to_owned())),
        control_url: Some(url.to_owned()),
        advertise_tags: tags.iter().map(|t| t.to_string()).collect(),
        log_to_stderr: false,
    })
    .unwrap();
    node.start().unwrap();
    node
}

fn phone(root: &Path, name: &str, net: &Net) -> Arc<CollieCore> {
    CollieCore::with_control_url(root.join(name), net.url.clone()).unwrap()
}

fn wait_phone(rt: &tokio::runtime::Runtime, core: &CollieCore) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while rt.block_on(core.node_state()).unwrap().backend_state != TailnetState::Running {
        assert!(Instant::now() < deadline, "phone never reached Running");
        std::thread::sleep(Duration::from_millis(50));
    }
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
            && st.peer.as_ref().map_or(0, |p| {
                p.values().filter(|p| p.tailscale_ips.is_some()).count()
            }) >= peers;
        if ready {
            return st;
        }
        assert!(Instant::now() < deadline, "node not ready: {st:#?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

// Holds node keys and collied state for the throwaway tailnet; removed on panic too.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct TestControl {
    child: Child,
    url: String,
}

impl TestControl {
    fn start(auth_key: &str, dir: &Path) -> Self {
        let bin = dir.join("testcontrol");
        let status = Command::new(std::env::var("GO").unwrap_or_else(|_| "go".into()))
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("testcontrol"))
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

/// A dedicated named herdr session in its own HOME and XDG dirs with a scrubbed
/// environment, so neither `--session` precedence nor an inherited HERDR_SOCKET_PATH
/// can ever reach the user's default session.
struct HerdrSession {
    child: Child,
    root: PathBuf,
    name: String,
    socket: PathBuf,
}

impl HerdrSession {
    fn start() -> Self {
        let pid = std::process::id();
        // sun_path is 104 bytes on macOS and herdr nests its socket four levels deep.
        let root = PathBuf::from(format!("/tmp/ce2e-{pid}"));
        let _ = std::fs::remove_dir_all(&root);
        let work = root.join("collie-e2e");
        for dir in [&root, &root.join("home"), &work] {
            std::fs::DirBuilder::new().mode(0o700).create(dir).unwrap();
        }
        let name = format!("collie-e2e-{pid}");
        let socket = root.join("herdr/sessions").join(&name).join("herdr.sock");
        let child = herdr_cmd(&root)
            .args(["--session", &name, "server"])
            .env("HERDR_STARTUP_CWD", &work)
            .env("SHELL", "/bin/sh")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let session = Self {
            child,
            root,
            name,
            socket,
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while !herdr_pings(&session.socket) {
            assert!(Instant::now() < deadline, "dedicated herdr never answered");
            std::thread::sleep(Duration::from_millis(100));
        }
        session
    }

    fn stop(&mut self) -> bool {
        let _ = herdr_cmd(&self.root)
            .args(["session", "stop", &self.name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }
}

impl Drop for HerdrSession {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() && !self.stop() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn herdr_cmd(root: &Path) -> Command {
    let mut cmd = Command::new("herdr");
    cmd.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root)
        .env("XDG_STATE_HOME", root.join("state"));
    cmd
}

fn herdr_pings(socket: &Path) -> bool {
    let Ok(mut conn) = StdUnixStream::connect(socket) else {
        return false;
    };
    let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
    if conn
        .write_all(b"{\"id\":\"e2e\",\"method\":\"ping\",\"params\":{}}\n")
        .is_err()
    {
        return false;
    }
    let mut line = String::new();
    BufReader::new(conn).read_line(&mut line).is_ok() && line.contains("\"pong\"")
}
