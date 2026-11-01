#[allow(dead_code)]
mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use collie_core::{CollieCore, CoreError};
use collied::config::TasksConfig;
use collied::server;
use common::*;
use serde_json::json;

const TERMINAL: &str = "term_65ce7ae4fd5731";

#[test]
fn git_worktrees_end_to_end() {
    if !in_child("git_worktrees_end_to_end") {
        return;
    }
    let t0 = Instant::now();
    let root = TempDir::new("e2eg");
    let net = Net::start(&root.0);
    let (rt, core) = phone_up(&root.0, &net);
    println!("tailnet: control, Mac, phone up in {:?}", t0.elapsed());
    rt.block_on(scenario(&root.0, &core, &net));
    drop(core);
    drop(rt);
    println!("total {:?}", t0.elapsed());
}

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "{args:?}");
}

async fn scenario(root: &Path, core: &Arc<CollieCore>, net: &Net) {
    let work = std::fs::canonicalize(root).unwrap().join("work");
    let repo = work.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "Start"]);
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "feature", "../feature"],
    );
    let feature = work.join("feature");
    std::fs::write(feature.join("a.txt"), "two\n").unwrap();
    git(&feature, &["commit", "-qam", "Change a"]);
    std::fs::write(feature.join("b.txt"), "new\n").unwrap();
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
    let handle = server::start_with(
        net.mac.clone(),
        server_config(&root.join("collied"), "e2e"),
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

    println!("the phone lists the worktrees with their last commit and changes");
    let list = core
        .agent_worktrees(m.clone(), TERMINAL.into())
        .await
        .unwrap();
    assert_eq!(list.root.as_deref(), repo.to_str());
    assert!(list.files.is_empty());
    let rows: Vec<_> = list
        .worktrees
        .iter()
        .map(|w| {
            (
                w.branch.as_deref(),
                w.subject.as_deref(),
                w.ahead,
                w.uncommitted,
                w.additions,
                w.deletions,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                Some("main"),
                Some("Start"),
                Some(0),
                Some(0),
                Some(0),
                Some(0)
            ),
            (
                Some("feature"),
                Some("Change a"),
                Some(1),
                Some(1),
                Some(1),
                Some(1)
            ),
        ]
    );
    assert!(list.worktrees.iter().all(|w| w.committed_at_ms.is_some()));

    println!("a pull request needs GitHub, which this repository has none of");
    let err = core
        .agent_pull_request_changes(m.clone(), TERMINAL.into(), 7)
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::InvalidInput { .. }), "{err:?}");
    let err = core
        .agent_diff(
            m.clone(),
            TERMINAL.into(),
            repo.to_str().unwrap().into(),
            "a.txt".into(),
            "pull_request".into(),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::InvalidInput { .. }), "{err:?}");
    handle.shutdown().await;
}
