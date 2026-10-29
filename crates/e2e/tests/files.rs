#[allow(dead_code)]
mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use collie_core::{CollieCore, CoreError, FileKind};
use collied::config::TasksConfig;
use collied::server;
use common::*;
use serde_json::{Value, json};

const TERMINAL: &str = "term_65ce7ae4fd5731";

#[test]
fn files_end_to_end() {
    if !in_child("files_end_to_end") {
        return;
    }
    let t0 = Instant::now();
    let root = TempDir::new("e2ef");
    let net = Net::start(&root.0);
    wait_ready(&net.mac, 0);
    let core = phone(&root.0, "phone", &net);
    let rt = runtime();
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    wait_ready(&net.mac, 1);
    wait_phone(&rt, &core);
    println!("tailnet: control, Mac, phone up in {:?}", t0.elapsed());
    rt.block_on(scenario(&root.0, &core, &net));
    drop(core);
    drop(rt);
    println!("total {:?}", t0.elapsed());
}

async fn scenario(root: &Path, core: &Arc<CollieCore>, net: &Net) {
    let work = std::fs::canonicalize(root).unwrap().join("work");
    let repo = work.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let image: Vec<u8> = [0x89, b'P', b'N', b'G']
        .into_iter()
        .chain((0..100_000u32).map(|i| (i % 251) as u8))
        .collect();
    std::fs::write(repo.join("shot.png"), &image).unwrap();
    std::fs::write(repo.join(".env"), "SECRET=1").unwrap();
    let agent = json!({
        "terminal_id": TERMINAL, "agent": "claude", "agent_status": "idle",
        "agent_session": {"source": "herdr:claude", "agent": "claude", "kind": "id",
            "value": "00000000-0000-4000-8000-000000000000"},
        "workspace_id": "w6", "tab_id": "w6:t1", "pane_id": "w6:p1", "focused": true,
        "state_change_seq": 5, "cwd": repo, "foreground_cwd": repo, "revision": 2,
    });
    mock_herdr(&root.join("herdr.sock"), move |method, _| {
        (method == "agent.get").then(|| json!({"result": {"type": "agent_info", "agent": agent}}))
    });
    let data_dir = root.join("collied");
    let handle = server::start_with(
        net.mac.clone(),
        server_config(&data_dir, "e2e"),
        root.join("herdr.sock"),
        &TasksConfig {
            roots: Some(vec![work.clone()]),
            ..TasksConfig::default()
        },
        None,
    )
    .await
    .unwrap();
    let (machine, _) = pair(&handle.control_path(), core, LABEL).await;
    let m = machine.id.clone();
    connected_flock(core, &m).await;

    println!("the phone opens an image the agent mentioned, relative to its folder");
    let t = Instant::now();
    let opened = core
        .open_file(m.clone(), TERMINAL.into(), "shot.png".into())
        .await
        .unwrap();
    assert_eq!(opened.name, "shot.png");
    assert_eq!(opened.kind, FileKind::Image);
    assert_eq!(opened.size, image.len() as u64);
    assert_eq!(opened.data, image);
    println!("  opened in {:?}", t.elapsed());

    println!("hidden files and paths outside the task folders are refused");
    for path in [".env", "/etc/hosts", "../repo/shot.png"] {
        let err = core
            .open_file(m.clone(), TERMINAL.into(), path.into())
            .await
            .unwrap_err();
        assert!(
            matches!(err, CoreError::InvalidInput { .. }),
            "{path}: {err:?}"
        );
    }

    let audit = data_dir.join("audit.log");
    let opens: Vec<Value> = audit_lines(&audit)
        .into_iter()
        .filter(|e| e["method"] == "file.open")
        .collect();
    assert_eq!(opens.len(), 4, "{opens:#?}");
    assert_eq!(opens[0]["target"], format!("{TERMINAL} path=\"shot.png\""));
    assert_eq!(opens[0]["result"], "ok");
    let log = std::fs::read_to_string(&audit).unwrap();
    assert!(!log.contains(&STANDARD.encode(&image[..48])));
    assert!(!log.contains("file.chunk"));
    handle.shutdown().await;
}
