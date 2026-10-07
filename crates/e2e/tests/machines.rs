#[allow(dead_code)]
mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use collie_core::{MachineFlock, MachineKind, TerminalSource};
use collied::server::{self, ServerConfig, ServerHandle};
use common::*;
use serde_json::json;
use tailnet::Node;

const LINUX_TAG: &str = "tag:collie-linux";
const AGENT: &str = "term_65ce7ae4fd5731";
const PROMPT: Duration = Duration::from_secs(2);

#[test]
fn machines_side_by_side() {
    if !in_child("machines_side_by_side") {
        return;
    }
    let t0 = Instant::now();
    let root = TempDir::new("e2em");
    let net = Net::start(&root.0);
    let linux = start_node(
        &root.0,
        "collie-e2e-linux",
        &net.key,
        &net.url,
        &[LINUX_TAG],
    );
    // The phone registers after both machines are up, so its first netmap has them.
    wait_ready(&net.mac, 0);
    wait_ready(&linux, 0);
    let core = phone(&root.0, "phone", &net);
    let rt = runtime();
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    wait_ready(&net.mac, 2);
    wait_ready(&linux, 2);
    wait_phone(&rt, &core);
    println!(
        "tailnet: control, Mac, Linux, phone up in {:?}",
        t0.elapsed()
    );

    let mac_server = rt.block_on(start_collied(&root.0, &net.mac, "mac"));
    // Its own runtime, so dropping it takes every task holding the Linux node with it.
    let linux_rt = runtime();
    let linux_server = linux_rt.block_on(start_collied(&root.0, &linux, "linux"));

    println!("the phone pairs both machines and records their kinds");
    let (mac_id, linux_id) = rt.block_on(async {
        let (mac, _) = pair(&mac_server.control_path(), &core, LABEL).await;
        let (linux, _) = pair(&linux_server.control_path(), &core, LABEL).await;
        (mac.id, linux.id)
    });
    let machines = core.machines();
    assert_eq!(machines.len(), 2);
    let kind = |id: &str| machines.iter().find(|m| m.id == id).unwrap().kind;
    assert_eq!(kind(&mac_id), MachineKind::Mac);
    assert_eq!(kind(&linux_id), MachineKind::Linux);
    rt.block_on(async {
        for (id, name) in [(&mac_id, "e2e-mac"), (&linux_id, "e2e-linux")] {
            let flock = connected_flock(&core, id).await;
            assert_eq!(flock.details.unwrap().name, name);
        }
        let read = core
            .agent_read(mac_id.clone(), AGENT.into(), TerminalSource::Recent, None)
            .await
            .unwrap();
        assert_eq!(read.ansi, "e2e-mac screen");
    });

    println!("the Linux machine powers off");
    linux_rt.block_on(linux_server.shutdown());
    // The session's close must reach the phone before the node goes; a session that dies
    // silently is the 45 s silence deadline's case, covered in collie-core.
    rt.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            // A snapshot sent as the session dies waits out CALL_TIMEOUT and errors.
            if let Ok(f) = core.flock(linux_id.clone()).await
                && !connected(&f)
            {
                break;
            }
            assert!(Instant::now() < deadline, "the Linux session never closed");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    drop(linux_rt);
    drop(linux);

    println!("the Mac keeps answering while the Linux link gives up");
    let t = Instant::now();
    let linux_flock = rt.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let linux = timed(core.flock(linux_id.clone())).await.unwrap();
            let mac = timed(core.flock(mac_id.clone())).await.unwrap();
            assert!(connected(&mac), "{mac:?}");
            let read =
                timed(core.agent_read(mac_id.clone(), AGENT.into(), TerminalSource::Recent, None))
                    .await
                    .unwrap();
            assert_eq!(read.ansi, "e2e-mac screen");
            if dial_failed(&linux) {
                return linux;
            }
            assert!(Instant::now() < deadline, "Linux never gave up: {linux:?}");
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    println!(
        "  Linux waiting in {:?}: {:?}",
        t.elapsed(),
        linux_flock.last_error
    );
    assert_eq!(linux_flock.agents.len(), 2, "the last flock is kept");

    assert_eq!(kernel_tcp_listeners(), Vec::<String>::new());
    rt.block_on(mac_server.shutdown());
    drop(core);
    drop(rt);
    println!("total {:?}", t0.elapsed());
}

/// Waiting after a redial failed, not just after the session dropped under it.
fn dial_failed(f: &MachineFlock) -> bool {
    format!("{:?}", f.link) == "Waiting"
        && f.last_error
            .as_deref()
            .is_some_and(|e| e.starts_with("not reachable ("))
}

async fn timed<T>(call: impl Future<Output = T>) -> T {
    let started = Instant::now();
    let out = call.await;
    assert!(
        started.elapsed() < PROMPT,
        "took {:?} while the other machine was down",
        started.elapsed()
    );
    out
}

async fn start_collied(root: &Path, node: &Node, name: &str) -> ServerHandle {
    let herdr_socket = root.join(format!("herdr-{name}.sock"));
    let screen = format!("e2e-{name} screen");
    mock_herdr(&herdr_socket, move |method, p| {
        (method == "agent.read").then(|| {
            json!({"result": {"type": "pane_read", "read": {
                "pane_id": p["target"], "workspace_id": "w6", "tab_id": "w6:t1",
                "source": p["source"], "format": "ansi", "text": screen, "revision": 0,
                "truncated": false,
            }}})
        })
    });
    let data_dir = root.join(format!("collied-{name}"));
    server::start(
        node.clone(),
        ServerConfig {
            machine_name: format!("e2e-{name}"),
            ..server_config(&data_dir, "e2e")
        },
        herdr_socket,
    )
    .await
    .unwrap()
}
