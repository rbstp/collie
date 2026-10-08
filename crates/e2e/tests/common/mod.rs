use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use collie_core::{CollieCore, IdentitySigner, Machine, MachineFlock, TailnetState};
use collie_tls::rustls::sign::SigningKey;
use collied::control::{Client, Reply, Request};
use collied::server::ServerConfig;
use futures_util::{SinkExt, StreamExt};
use protocol::{ErrorCode, PairingInvite, Response, ServerFrame};
use serde_json::{Value, json};
use tailnet::{BackendState, Config, Node, Status};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, header};
use zeroize::Zeroizing;

pub const KNOBS: [(&str, &str); 1] = [("TS_DISABLE_PORTMAPPER", "1")];
pub const PORT: u16 = protocol::DEFAULT_PORT;
pub const WATCHDOG: Duration = Duration::from_secs(240);
pub const LABEL: &str = "E2E iPhone";
// The tag collie-core's pin requires of the Mac node, whatever OS the test runs on.
pub const PHONE_PINNED_TAG: &str = "tag:collie-mac";
pub const PROBE: &str = "E2E probe";
pub const SNAPSHOT: &str = include_str!("../../../collied/tests/fixtures/session.snapshot.json");

// libtailscale's Go runtime reads TS_* knobs once at load, so each test re-runs itself
// in a child process that has them (same approach as crates/tailnet/tests/end_to_end.rs).
pub fn in_child(test: &str) -> bool {
    in_child_with(test, &[])
}

pub fn in_child_with(test: &str, env: &[(&str, &str)]) -> bool {
    if KNOBS
        .iter()
        .chain(env)
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
        .envs(env.iter().copied())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    print!("{stdout}");
    eprint!("{}", String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "child run failed: {}", out.status);
    assert!(stdout.contains("1 passed"), "child did not run the test");
    false
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

pub async fn pair(control: &Path, core: &Arc<CollieCore>, label: &str) -> (Machine, String) {
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

pub fn connected(f: &MachineFlock) -> bool {
    format!("{:?}", f.link) == "Connected"
}

pub async fn connected_flock(core: &CollieCore, machine_id: &str) -> MachineFlock {
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

pub fn audit_lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

pub fn kernel_tcp_listeners() -> Vec<String> {
    tailnet::kernel_tcp_listeners(std::process::id()).expect("kernel TCP listeners")
}

pub struct Net {
    _control: TestControl,
    pub key: String,
    pub url: String,
    pub mac: Node,
}

impl Net {
    pub fn start(root: &Path) -> Self {
        Self::with_key(
            root,
            format!("test-authkey-colliee2e{}", std::process::id()),
        )
    }

    pub fn with_key(root: &Path, key: String) -> Self {
        let control = TestControl::start(&key, root);
        let url = control.url.clone();
        let mac = start_node(root, "collie-e2e-mac", &key, &url, &[PHONE_PINNED_TAG]);
        Self {
            _control: control,
            key,
            url,
            mac,
        }
    }
}

pub fn start_node(root: &Path, name: &str, key: &str, url: &str, tags: &[&str]) -> Node {
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

struct Soft(Arc<dyn SigningKey>);

impl IdentitySigner for Soft {
    fn sign(&self, message: Vec<u8>) -> Option<Vec<u8>> {
        self.0
            .choose_scheme(&[collie_tls::SCHEME])?
            .sign(&message)
            .ok()
    }
}

pub type ProbeStream = collie_tls::client::TlsStream<collie_tls::Sniff<tokio::net::UnixStream>>;

/// A raw client's TLS, pinning the key collied keeps in `data_dir`, with one key of its own.
pub async fn probe_tls(
    stream: tokio::net::UnixStream,
    data_dir: &Path,
) -> std::io::Result<ProbeStream> {
    static KEY: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    let mine = KEY.get_or_init(|| collie_tls::generate().unwrap());
    let identity = collie_tls::certified(collie_tls::load(mine).unwrap()).unwrap();
    let file: Value =
        serde_json::from_slice(&std::fs::read(data_dir.join("tls-key.json")).unwrap()).unwrap();
    let der = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        file["pkcs8"].as_str().unwrap(),
    )
    .unwrap();
    let machine = collie_tls::certified(collie_tls::load(&der).unwrap()).unwrap();
    match collie_tls::connect(
        stream,
        "collie",
        collie_tls::pin(machine.cert[0].as_ref()),
        identity,
    )
    .await
    {
        Ok(s) => Ok(s),
        Err(collie_tls::ConnectError::Forbidden) => panic!("probe refused with 403"),
        Err(collie_tls::ConnectError::Tls(e)) => Err(e),
    }
}

/// One key per phone name, as a phone keeps its Secure Enclave key across launches.
pub fn phone(root: &Path, name: &str, net: &Net) -> Arc<CollieCore> {
    static KEYS: Mutex<Option<HashMap<String, Vec<u8>>>> = Mutex::new(None);
    let core = CollieCore::with_control_url(root.join(name), net.url.clone()).unwrap();
    let pkcs8 = KEYS
        .lock()
        .unwrap()
        .get_or_insert_default()
        .entry(name.to_owned())
        .or_insert_with(|| collie_tls::generate().unwrap())
        .clone();
    let key = collie_tls::load(&pkcs8).unwrap();
    let spki = key.public_key().unwrap().as_ref().to_vec();
    core.set_identity(spki, Box::new(Soft(key))).unwrap();
    core
}

pub fn wait_phone(rt: &tokio::runtime::Runtime, core: &CollieCore) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while rt.block_on(core.node_state()).unwrap().backend_state != TailnetState::Running {
        assert!(Instant::now() < deadline, "phone never reached Running");
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn wait_ready(node: &Node, peers: usize) -> Status {
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
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
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

pub struct TestControl {
    child: Child,
    url: String,
}

impl TestControl {
    fn start(auth_key: &str, dir: &Path) -> Self {
        let bin = dir.join("testcontrol");
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
        // The helper exits when its stdin closes, so it dies with this process. One user
        // owns every node, as on a personal tailnet: the phones and the Mac's tag owner
        // share a user ID, so the gate is exercised on pairing state and tags.
        let mut child = Command::new(&bin)
            .args(["-authkey", auth_key, "-same-user"])
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
pub struct HerdrSession {
    child: Child,
    pub root: PathBuf,
    pub name: String,
    pub socket: PathBuf,
}

impl HerdrSession {
    pub fn start() -> Self {
        let pid = std::process::id();
        // sun_path is 104 bytes on macOS and herdr nests its socket four levels deep.
        let root = PathBuf::from(format!("/tmp/ce2e-{pid}"));
        let _ = std::fs::remove_dir_all(&root);
        let work = root.join("collie-e2e");
        for dir in [&root, &root.join("home"), &root.join("bin"), &work] {
            std::fs::DirBuilder::new().mode(0o700).create(dir).unwrap();
        }
        let name = format!("collie-e2e-{pid}");
        let socket = root.join("herdr/sessions").join(&name).join("herdr.sock");
        let child = isolated("herdr", &root)
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

    pub fn stop(&mut self) -> bool {
        let _ = isolated("herdr", &self.root)
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

/// `root/bin` comes first on PATH, for executables a test provides to herdr's shells.
pub fn isolated(program: &str, root: &Path) -> Command {
    let mut path = std::ffi::OsString::from(root.join("bin"));
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    let mut cmd = Command::new(program);
    cmd.env_clear()
        .env("PATH", path)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root)
        .env("XDG_STATE_HOME", root.join("state"));
    cmd
}

pub fn herdr_pings(socket: &Path) -> bool {
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

pub fn herdr_installed() -> bool {
    let installed = Command::new("herdr")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !installed {
        println!("skipped: herdr is not installed");
    }
    installed
}

pub fn server_config(data_dir: &Path, herdr_session: &str) -> ServerConfig {
    ServerConfig {
        attachments_dir: data_dir.join("attachments"),
        data_dir: data_dir.to_owned(),
        port: PORT,
        owner_user_id: None,
        herdr_session: herdr_session.into(),
        machine_name: "e2e-mac".into(),
        approval_ttl: collied::approvals::TTL,
        terminals: false,
        terminal_grant_ttl: collied::terminal::GRANT_TTL,
    }
}

pub async fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// One request per connection, like herdr. Fixtures are sanitized live samples; `extra`
// answers anything else. Returns the methods called.
pub fn mock_herdr(
    path: &Path,
    extra: impl Fn(&str, &Value) -> Option<Value> + Send + Sync + 'static,
) -> Arc<Mutex<Vec<String>>> {
    let listener = UnixListener::bind(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let extra = Arc::new(extra);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (seen, extra) = (seen.clone(), extra.clone());
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
                let fixture = |text: &str| serde_json::from_str::<Value>(text).unwrap();
                let mut resp = match method.as_str() {
                    "ping" => fixture(include_str!("../fixtures/ping.json")),
                    "session.snapshot" => fixture(SNAPSHOT),
                    "agent.list" => fixture(include_str!("../fixtures/agent.list.json")),
                    "workspace.list" => fixture(include_str!("../fixtures/workspace.list.json")),
                    _ => extra(&method, &req["params"]).unwrap_or_else(
                        || json!({"error": {"code": "unknown_method", "message": "not mocked"}}),
                    ),
                };
                resp["id"] = req["id"].clone();
                let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
            });
        }
    });
    calls
}

pub type Ws = WebSocketStream<ProbeStream>;

pub struct Probe {
    pub node: Node,
    pub target: String,
    pub data_dir: PathBuf,
}

impl Probe {
    pub async fn session(&self) -> Ws {
        // collied closes a connection whose whois outlasts WHOIS_TIMEOUT, which a loaded
        // test host can reach; the phone dials again then, and so does the probe. Only that
        // audited close is retried: any other close without a reply still fails the test.
        let timeouts = || {
            audit_lines(&self.data_dir.join("audit.log"))
                .iter()
                .filter(|l| {
                    l["result"]
                        .as_str()
                        .is_some_and(|r| r.starts_with("rejected: whois timed out"))
                })
                .count()
        };
        let mut closed = 0;
        let stream = loop {
            let before = timeouts();
            let stream = self.dial().await;
            match probe_tls(UnixStream::from_std(stream).unwrap(), &self.data_dir).await {
                Ok(s) => break s,
                Err(e)
                    if e.kind() == std::io::ErrorKind::UnexpectedEof
                        && closed < 3
                        && timeouts() > before =>
                {
                    closed += 1;
                    eprintln!("probe: collied timed out whois ({e}), dialing again");
                }
                Err(e) => panic!("probe tls: {e}"),
            }
        };
        let mut req = format!("ws://{}{}", self.target, protocol::WS_PATH)
            .into_client_request()
            .unwrap();
        req.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static(protocol::WS_SUBPROTOCOL),
        );
        let (mut ws, _) = tokio_tungstenite::client_async(req, stream).await.unwrap();
        let hello = json!({"protocol_version": protocol::PROTOCOL_VERSION, "app_version": "e2e"});
        call(&mut ws, "hello", hello).await.unwrap();
        ws
    }

    async fn dial(&self) -> std::os::unix::net::UnixStream {
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
        stream
    }

    pub async fn pair(&self, control: &Path) {
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
}

pub async fn call(ws: &mut Ws, method: &str, params: Value) -> Result<Response, ErrorCode> {
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
