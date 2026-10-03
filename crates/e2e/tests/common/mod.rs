use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use collie_core::{CollieCore, Machine, MachineFlock, TailnetState};
use collied::control::{Client, Reply, Request};
use serde_json::Value;
use tailnet::{BackendState, Config, Node, Status};
use zeroize::Zeroizing;

pub const KNOBS: [(&str, &str); 1] = [("TS_DISABLE_PORTMAPPER", "1")];
pub const PORT: u16 = protocol::DEFAULT_PORT;
pub const WATCHDOG: Duration = Duration::from_secs(240);
pub const LABEL: &str = "E2E iPhone";

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

pub fn phone(root: &Path, name: &str, net: &Net) -> Arc<CollieCore> {
    CollieCore::with_control_url(root.join(name), net.url.clone()).unwrap()
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
