use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tailnet::{Accepted, BackendState, Config, Error, Node, Status};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use zeroize::Zeroizing;

const KNOBS: [(&str, &str); 1] = [("TS_DISABLE_PORTMAPPER", "1")];
const PORT: u16 = 7000;
const REUSE_PORT: u16 = 7001;
const DIALERS: [&str; 2] = ["it-dialer-a", "it-dialer-b"];
const TAGGED: &str = "it-tagged";
const TAG: &str = "tag:collie-phone";
const UNASSIGNED_IP: &str = "100.127.255.254";
const CONNS_PER_DIALER: usize = 150;
const WORKERS_PER_DIALER: usize = 8;
// The phone's budget to reach collied. A dropped SYN costs at least the 1 s
// initial RTO, so a dial that needs one retransmit still fits.
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
// Sequential dials from one node to one ip:port draw source ports from 49536,
// so 1000 of them repeat a 4-tuple about 10 times (none at all: p < 1e-4).
const REUSE_CONNS: usize = 1000;
const WATCHDOG: Duration = Duration::from_secs(180);

type Owners = HashMap<String, (String, Vec<IpAddr>)>;

#[test]
fn end_to_end() {
    // libtailscale's Go runtime reads TS_* knobs once when the library loads, so
    // they must be set before this process starts. Without this one every node
    // probes the LAN gateway with UPnP/NAT-PMP/PCP. TS_DEBUG_ALWAYS_USE_DERP is
    // not usable: on rebind it swaps its blocking UDP conn without closing the
    // old one, so tailscale_close then hangs forever (tailscale v1.104.0).
    if KNOBS
        .iter()
        .any(|(k, v)| std::env::var(k).as_deref() != Ok(*v))
    {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["end_to_end", "--exact", "--nocapture"])
            .envs(KNOBS)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        print!("{stdout}");
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "child run failed: {}", out.status);
        assert!(
            stdout.contains("1 passed"),
            "child run did not execute the test"
        );
        return;
    }
    // A blocking libtailscale call that never returns cannot be cancelled.
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        eprintln!("watchdog: test still running after {WATCHDOG:?}");
        std::process::exit(101);
    });

    let t0 = Instant::now();
    let auth_key = format!("test-authkey-collieit{}", std::process::id());
    let control = TestControl::start(&auth_key);
    println!("testcontrol up in {:?} at {}", t0.elapsed(), control.url);

    let root = TempDir(
        PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("tailnet-it-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&root.0);
    let t1 = Instant::now();
    let listener = start_node(&root.0, "it-listener", &control.url, &auth_key, &[]);
    let dialers: Vec<Node> = DIALERS
        .iter()
        .map(|name| start_node(&root.0, name, &control.url, &auth_key, &[]))
        .collect();
    let tagged = start_node(&root.0, TAGGED, &control.url, &auth_key, &[TAG]);
    let peers = DIALERS.len() + 1;
    let listener_st = wait_ready(&listener, peers);
    let dialer_st: Vec<Status> = dialers.iter().map(|d| wait_ready(d, peers)).collect();
    let tagged_st = wait_ready(&tagged, peers);
    println!("4 nodes Running with full netmaps in {:?}", t1.elapsed());
    assert_eq!(
        kernel_tcp_listeners(),
        Vec::<String>::new(),
        "tailnet nodes must not listen on any kernel interface"
    );

    let listener_self = listener_st.self_node.clone().unwrap();
    let listener_ips = listener_self.tailscale_ips.clone().unwrap();
    assert!(listener_ips.iter().any(IpAddr::is_ipv4) && listener_ips.iter().any(IpAddr::is_ipv6));
    let mut owners = Owners::new();
    let mut user_ids = vec![listener_self.user_id];

    for (name, st) in DIALERS.iter().zip(&dialer_st) {
        let me = st.self_node.as_ref().unwrap();
        for ip in me.tailscale_ips.iter().flatten() {
            let t = Instant::now();
            let who = listener.whois(&ip.to_string()).unwrap();
            println!("whois {ip} ({name}) in {:?}", t.elapsed());
            assert_eq!(who.node.stable_id, me.stable_id, "whois {ip}");
            assert_eq!(who.node.user, me.user_id, "whois {ip}");
            assert_eq!(who.user_profile.unwrap().id, me.user_id, "whois {ip}");
            assert!(who.node.is_owned_by(me.user_id));
        }
        for ip in &listener_ips {
            let peer = st.peer_by_ip(*ip).expect("listener in dialer netmap");
            assert_eq!(
                peer.stable_id, listener_self.stable_id,
                "{name} peer_by_ip {ip}"
            );
        }
        user_ids.push(me.user_id);
        owners.insert(
            name.to_string(),
            (me.stable_id.clone(), me.tailscale_ips.clone().unwrap()),
        );
    }
    user_ids.sort();
    user_ids.dedup();
    assert_eq!(
        user_ids.len(),
        3,
        "testcontrol must give each node its own user"
    );
    assert!(listener.whois(UNASSIGNED_IP).is_err());

    let tagged_self = tagged_st.self_node.unwrap();
    assert_eq!(tagged_self.tags.as_deref(), Some(&[TAG.to_owned()][..]));
    for ip in tagged_self.tailscale_ips.iter().flatten() {
        let who = listener.whois(&ip.to_string()).unwrap();
        assert_eq!(who.node.stable_id, tagged_self.stable_id);
        assert_eq!(who.node.tags.as_deref(), Some(&[TAG.to_owned()][..]));
        assert_eq!(who.node.user, tagged_self.user_id);
        assert!(
            !who.node.is_owned_by(tagged_self.user_id),
            "a tagged node must never pass the ownership gate"
        );
    }
    drop(tagged);

    let t = Instant::now();
    let err = dialers[0]
        .dial_timeout(
            "tcp",
            &format!("{UNASSIGNED_IP}:{PORT}"),
            Duration::from_secs(1),
        )
        .unwrap_err();
    let waited = t.elapsed();
    println!("dial_timeout to an unassigned tailnet ip: {err} after {waited:?}");
    assert!(matches!(err, Error::Tailscale(_)), "{err:?}");
    assert!(
        (Duration::from_millis(900)..Duration::from_secs(2)).contains(&waited),
        "dial_timeout returned after {waited:?}"
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let reuse_target = SocketAddr::new(listener_ips[0], REUSE_PORT);
    let (reused, max_dial, failures) =
        rt.block_on(tuple_reuse(&listener, dialers[0].clone(), reuse_target));
    println!(
        "tuple reuse: {REUSE_CONNS} dials to {reuse_target}, {reused} reused a 4-tuple still in TIME-WAIT, slowest dial {max_dial:?}"
    );
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(
        reused > 0,
        "no 4-tuple was reused, so TIME-WAIT reopen was not exercised"
    );

    let report = rt.block_on(attribution(
        listener.clone(),
        dialers,
        listener_ips,
        Arc::new(owners),
    ));
    println!(
        "attribution: {} accepted ({} per dialer, {} workers each, v4+v6) in {:?}, {} fd churn rounds",
        report.accepted, CONNS_PER_DIALER, WORKERS_PER_DIALER, report.elapsed, report.churn_rounds
    );
    println!(
        "open fds: {} before, {} after",
        report.fds_before, report.fds_after
    );
    assert!(
        report.dial_failures.is_empty() && report.bad_conns.is_empty(),
        "{} dial failures: {:#?}\n{} bad accepted connections: {:#?}",
        report.dial_failures.len(),
        report.dial_failures,
        report.bad_conns.len(),
        report.bad_conns
    );
    assert_eq!(report.accepted, DIALERS.len() * CONNS_PER_DIALER);
    assert!(
        report.fds_after <= report.fds_before,
        "socketpair proxy leaked fds"
    );

    drop(listener);
    drop(control);
    println!("total {:?}", t0.elapsed());
}

// The listener closes first, so it holds each 4-tuple in TIME-WAIT. gVisor
// only reopens one on a new SYN for a listening endpoint; tailscale's netstack
// accepts through a forwarder, and without the tailscale-sys patch such a
// dial stalls 63 s in SYN retransmits. The listener is also left with more
// TIME-WAIT endpoints than netstack's 512-packet queue, which tailscale_close
// used to deadlock on. Each dialer read also needs the listener's close as EOF
// within the budget, which the blocking socketpair proxy sometimes missed.
async fn tuple_reuse(
    node: &Node,
    dialer: Node,
    target: SocketAddr,
) -> (usize, Duration, Vec<String>) {
    let listener = node.listen("tcp", &format!(":{}", target.port())).unwrap();
    let mut dial = tokio::task::spawn_blocking(move || {
        let mut max = Duration::ZERO;
        let mut failures = Vec::new();
        for seq in 0..REUSE_CONNS {
            let t = Instant::now();
            let res = dialer
                .dial_timeout("tcp", &target.to_string(), DIAL_TIMEOUT)
                .map_err(|e| e.to_string())
                .and_then(|mut conn| {
                    max = max.max(t.elapsed());
                    // Some dials read only after the listener has written and
                    // closed, so that path is covered on every run.
                    if seq % 100 == 0 {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    let mut buf = Vec::new();
                    read_timeout(&conn, DIAL_TIMEOUT)
                        .and_then(|()| conn.read_to_end(&mut buf).map_err(|e| format!("read: {e}")))
                        .and_then(|_| match &buf[..] {
                            b"k" => Ok(()),
                            other => Err(format!("read {other:?}")),
                        })
                });
            if let Err(e) = res {
                failures.push(format!("{seq} after {:?}: {e}", t.elapsed()));
            }
        }
        (max, failures)
    });
    let mut seen = HashSet::new();
    let mut reused = 0;
    loop {
        tokio::select! {
            res = listener.accept() => {
                let Accepted { mut stream, peer } = res.unwrap();
                if !seen.insert(peer) {
                    reused += 1;
                }
                let _ = stream.write_all(b"k").await;
            }
            res = &mut dial => {
                let (max, failures) = res.unwrap();
                return (reused, max, failures);
            }
        }
    }
}

// Holds node and machine keys for the throwaway tailnet; removed on panic too.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Report {
    fds_before: usize,
    fds_after: usize,
    accepted: usize,
    bad_conns: Vec<String>,
    dial_failures: Vec<String>,
    elapsed: Duration,
    churn_rounds: usize,
}

async fn attribution(
    node: Node,
    dialers: Vec<Node>,
    listener_ips: Vec<IpAddr>,
    owners: Arc<Owners>,
) -> Report {
    let listener = node.listen("tcp", &format!(":{PORT}")).unwrap();
    let fds_before = open_fds();
    let start = Instant::now();

    let stop = Arc::new(AtomicBool::new(false));
    let churn_rounds = Arc::new(AtomicUsize::new(0));
    let churn = std::thread::spawn({
        let stop = stop.clone();
        let rounds = churn_rounds.clone();
        move || {
            while !stop.load(Ordering::Relaxed) {
                let files: Vec<File> = (0..32).map(|_| File::open("/dev/null").unwrap()).collect();
                drop(files);
                rounds.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    let mut dial = tokio::task::spawn_blocking(move || dial_all(&dialers, &listener_ips));
    let mut checks = tokio::task::JoinSet::new();
    let mut accepted = 0;
    let dial_failures = loop {
        tokio::select! {
            res = listener.accept() => {
                let Accepted { stream, peer } = res.unwrap();
                accepted += 1;
                checks.spawn(check(stream, peer, node.clone(), owners.clone()));
            }
            res = &mut dial => break res.unwrap(),
        }
    };
    let mut bad_conns = Vec::new();
    while let Some(res) = checks.join_next().await {
        bad_conns.extend(res.unwrap());
    }
    let elapsed = start.elapsed();

    stop.store(true, Ordering::Relaxed);
    churn.join().unwrap();
    let settle = Instant::now() + Duration::from_secs(5);
    while open_fds() > fds_before && Instant::now() < settle {
        std::thread::sleep(Duration::from_millis(100));
    }
    Report {
        fds_before,
        fds_after: open_fds(),
        accepted,
        bad_conns,
        dial_failures,
        elapsed,
        churn_rounds: churn_rounds.load(Ordering::Relaxed),
    }
}

async fn check(
    stream: tokio::net::UnixStream,
    peer: SocketAddr,
    node: Node,
    owners: Arc<Owners>,
) -> Option<String> {
    let (read, mut write) = stream.into_split();
    let mut token = String::new();
    if let Err(e) = tokio::io::BufReader::new(read).read_line(&mut token).await {
        return Some(format!("from {peer}: read: {e}"));
    }
    let who = tokio::task::spawn_blocking(move || node.whois(&peer.to_string()))
        .await
        .unwrap();
    let _ = write.write_all(b"k").await;
    let token = token.trim();
    let Some((id, ips)) = owners.get(token.split(' ').next().unwrap()) else {
        return Some(format!(
            "from {peer}: stream hit EOF before the dialer's token ({token:?})"
        ));
    };
    match who {
        Ok(who) if who.node.stable_id == *id && ips.contains(&peer.ip()) => None,
        who => Some(format!(
            "misattributed: {token:?} arrived from {peer}, whois {:?}",
            who.map(|w| w.node.stable_id)
        )),
    }
}

fn dial_all(dialers: &[Node], listener_ips: &[IpAddr]) -> Vec<String> {
    std::thread::scope(|s| {
        let workers: Vec<_> = DIALERS
            .iter()
            .zip(dialers)
            .flat_map(|(name, node)| {
                (0..WORKERS_PER_DIALER).map(move |worker| {
                    s.spawn(move || {
                        let mut failures = Vec::new();
                        for seq in (worker..CONNS_PER_DIALER).step_by(WORKERS_PER_DIALER) {
                            let target =
                                SocketAddr::new(listener_ips[seq % listener_ips.len()], PORT);
                            if let Err(e) = dial_one(node, &target, &format!("{name} {seq}\n")) {
                                failures.push(format!("{name} {seq} -> {target}: {e}"));
                            }
                        }
                        failures
                    })
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    })
}

fn dial_one(node: &Node, target: &SocketAddr, token: &str) -> Result<(), String> {
    let mut conn = node
        .dial_timeout("tcp", &target.to_string(), DIAL_TIMEOUT)
        .map_err(|e| e.to_string())?;
    conn.set_read_timeout(Some(Duration::from_secs(30)))
        .and_then(|()| conn.write_all(token.as_bytes()))
        .and_then(|()| conn.read_exact(&mut [0u8; 1]))
        .map_err(|e| format!("ack: {e}"))
}

fn kernel_tcp_listeners() -> Vec<String> {
    tailnet::kernel_tcp_listeners(std::process::id()).expect("kernel TCP listeners")
}

fn open_fds() -> usize {
    std::fs::read_dir("/dev/fd").unwrap().count()
}

fn start_node(root: &Path, name: &str, control_url: &str, auth_key: &str, tags: &[&str]) -> Node {
    let node = Node::new(&Config {
        state_dir: root.join(name),
        hostname: name.into(),
        auth_key: Some(Zeroizing::new(auth_key.into())),
        control_url: Some(control_url.into()),
        advertise_tags: tags.iter().map(|t| t.to_string()).collect(),
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
        let bin = Path::new(env!("CARGO_TARGET_TMPDIR")).join("testcontrol");
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

/// Darwin fails setsockopt with EINVAL once the socketpair peer is closed; the read
/// cannot block then. Any other error (ENOTSOCK, EBADF) would mean a reused fd.
fn read_timeout(conn: &std::os::unix::net::UnixStream, timeout: Duration) -> Result<(), String> {
    match conn.set_read_timeout(Some(timeout)) {
        Err(e) if e.raw_os_error() != Some(22) => Err(format!("set_read_timeout: {e}")),
        _ => Ok(()),
    }
}
