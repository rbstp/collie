#[allow(dead_code)]
mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use collied::config::{LIMITS_FILE, TasksConfig};
use collied::control::{Client, Reply, Request};
use collied::limits::{CONTINUE, GRACE, Limits};
use collied::server::{self, ServerConfig};
use common::*;

const SESSION: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

/// A stand-in Claude Code: it writes its `sessions/<pid>.json`, reports idle with an empty
/// input box as Claude Code does, and prints the first prompt it reads.
fn fake_claude(sessions: &Path) -> String {
    let sessions = sessions.display();
    format!(
        r#"mkdir -p '{sessions}'
printf '{{"pid":%d,"sessionId":"{SESSION}","cwd":"%s","startedAt":%s000}}' $$ "$PWD" "$(date +%s)" > '{sessions}'/$$.json
st() {{ printf '\033]7501;state=%s:app=claude-code\033\\' "$1"; }}
cols=$(stty size | cut -d' ' -f2)
rule() {{ i=0; while [ $i -lt $cols ]; do printf '\342\224\200'; i=$((i+1)); done; printf '\n'; }}
st idle; rule; printf '\342\235\257 \033[2mTry "fix lint errors"\033[0m\n'; rule
IFS= read -r p; st working; printf '\033[2J\033[Hgot %s\n' "$p"
exec sleep 600"#
    )
}

/// collied stops a Claude Code agent at the plan limit when its PreToolUse hook asks, then
/// prompts it to continue once the limit resets, in a herdr-core terminal of the test's own.
#[test]
fn live_limit_resume() {
    if !hc_installed() {
        return;
    }
    // Named once, in the parent: the child must see the same values to run the test.
    let agents = std::env::var("COLLIE_E2E_AGENTS")
        .unwrap_or_else(|_| format!("/tmp/hcl-{}", std::process::id()));
    let claude = format!("{agents}/claude");
    let codex = format!("{agents}/codex");
    let env = [
        ("COLLIE_E2E_AGENTS", agents.as_str()),
        ("CLAUDE_CONFIG_DIR", claude.as_str()),
        ("CODEX_HOME", codex.as_str()),
    ];
    if !in_child_with("live_limit_resume", &env) {
        let _ = std::fs::remove_dir_all(&agents);
        return;
    }
    let mut hc = CoreSession::start("hcl");
    let root = TempDir::new("e2e-limits");
    let net = Net::start(&root.0);
    wait_ready(&net.mac, 0);
    let rt = runtime();
    rt.block_on(scenario(&root.0, &net, &hc, Path::new(&claude)));
    drop(rt);
    assert!(hc.stop(), "dedicated herdr-core did not stop");
}

async fn scenario(root: &Path, net: &Net, hc: &CoreSession, claude: &Path) {
    let work = std::fs::canonicalize(&hc.work).unwrap();
    let started = hc.hc(&[
        "run",
        "--bg",
        "--json",
        "--name",
        "collie-limit",
        "--cwd",
        work.to_str().unwrap(),
        "/bin/bash",
        "-c",
        r#"exec -a collie-fake /bin/bash -c "$1""#,
        "fake",
        &fake_claude(&claude.join("sessions")),
    ]);
    let address = started["address"].as_str().unwrap().to_owned();
    let data_dir = root.join("collied");
    collied::ensure_private_dir(&data_dir).unwrap();
    Limits::load(data_dir.join(LIMITS_FILE), GRACE, 0).set_enabled(true);
    let handle = server::start_with(
        net.mac.clone(),
        ServerConfig {
            herdr_core: Some(hc.socket.clone()),
            resume_grace: Duration::from_secs(1),
            ..server_config(&data_dir, "e2e")
        },
        root.join("no-herdr.sock"),
        &TasksConfig {
            roots: Some(vec![work.clone()]),
            ..TasksConfig::default()
        },
        None,
    )
    .await
    .unwrap();
    let control = handle.control_path();

    // The status line tap records the 5-hour limit reached, resetting in two seconds.
    let now = collied::now_ms();
    let resets = now / 1000 + 2;
    let status_line = serde_json::json!({
        "session_id": SESSION,
        "rate_limits": {
            "five_hour": {"used_percentage": 100, "resets_at": resets},
            "seven_day": {"used_percentage": 40, "resets_at": resets + 86_400},
        },
    });
    collied::usage::record(&data_dir, status_line.to_string().as_bytes(), now).unwrap();

    // The hook's question, until collied lists the agent's session and stops it.
    let ask = Request::Limit {
        session_id: SESSION.into(),
        subagent: false,
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    let reason = loop {
        assert!(Instant::now() < deadline, "the agent was never stopped");
        match Client::connect(&control)
            .await
            .unwrap()
            .call(&ask)
            .await
            .unwrap()
        {
            Reply::Limit { stop: Some(reason) } => break reason,
            Reply::Limit { stop: None } => tokio::time::sleep(Duration::from_millis(200)).await,
            other => panic!("{other:?}"),
        }
    };
    assert!(reason.contains("5-hour plan limit"), "{reason}");
    match Client::connect(&control)
        .await
        .unwrap()
        .call(&Request::Watch)
        .await
        .unwrap()
    {
        Reply::Watch { limited, .. } => {
            assert_eq!(limited.len(), 1);
            assert_eq!(limited[0].resume_at_ms, Some(resets * 1000 + 1000));
        }
        other => panic!("{other:?}"),
    }

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(
            Instant::now() < deadline,
            "the continuation never reached the pane"
        );
        let screen = hc.hc(&["read", "--json", &address])["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if screen.contains(&format!("got {CONTINUE}")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let lines = audit_lines(&data_dir.join("audit.log"));
    let results: Vec<(&str, &str)> = lines
        .iter()
        .filter(|l| {
            l["method"]
                .as_str()
                .is_some_and(|m| m.starts_with("limit."))
        })
        .map(|l| (l["method"].as_str().unwrap(), l["result"].as_str().unwrap()))
        .collect();
    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[0].0, "limit.stop");
    assert_eq!(results[1], ("limit.resume", "ok"));
    handle.shutdown().await;
}
