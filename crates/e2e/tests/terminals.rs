#[allow(dead_code)]
mod common;

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use collie_core::{AgentKey, CollieCore, CoreError};
use collie_tls::rustls::sign::SigningKey;
use collied::server::{self, ServerConfig};
use common::*;

#[test]
fn live_terminal() {
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
    if !in_child("live_terminal") {
        return;
    }
    let mut herdr = HerdrSession::start();
    let root = TempDir::new("e2e-terminal");
    let net = Net::start(&root.0);
    wait_ready(&net.mac, 0);
    let core = phone(&root.0, "phone", &net);
    let key = collie_tls::load(&collie_tls::generate().unwrap()).unwrap();
    core.set_terminal_key(key.public_key().unwrap().as_ref().to_vec())
        .unwrap();
    let rt = runtime();
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    wait_ready(&net.mac, 1);
    wait_phone(&rt, &core);
    rt.block_on(scenario(&root.0, &net, &core, &herdr, key));
    drop(core);
    drop(rt);
    assert!(herdr.stop(), "dedicated herdr session did not stop");
}

fn sign(key: &Arc<dyn SigningKey>, message: &[u8]) -> Vec<u8> {
    key.choose_scheme(&[collie_tls::SCHEME])
        .unwrap()
        .sign(message)
        .unwrap()
}

async fn unlock(core: &CollieCore, m: &str, shell: &str, key: &Arc<dyn SigningKey>) {
    let message = core
        .terminal_challenge(m.into(), shell.into())
        .await
        .unwrap();
    core.terminal_grant(m.into(), shell.into(), sign(key, &message))
        .await
        .unwrap();
}

async fn wait_view(
    core: &CollieCore,
    m: &str,
    shell: &str,
    what: &str,
    ok: impl Fn(&collie_core::AgentView) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if core
            .agent_view(m.into(), shell.into(), 0)
            .is_some_and(|v| ok(&v))
        {
            return;
        }
        assert!(Instant::now() < deadline, "never saw {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn watches(audit: &Path) -> usize {
    audit_lines(audit)
        .iter()
        .filter(|e| e["method"] == "terminal.watch")
        .count()
}

async fn scenario(
    root: &Path,
    net: &Net,
    core: &Arc<CollieCore>,
    herdr: &HerdrSession,
    key: Arc<dyn SigningKey>,
) {
    let data_dir = root.join("collied");
    let audit = data_dir.join("audit.log");
    let handle = server::start(
        net.mac.clone(),
        ServerConfig {
            attachments_dir: data_dir.join("attachments"),
            data_dir: data_dir.clone(),
            port: PORT,
            owner_user_id: None,
            herdr_session: herdr.name.clone(),
            machine_name: "e2e-mac".into(),
            approval_ttl: collied::approvals::TTL,
            terminals: true,
            terminal_grant_ttl: collied::terminal::GRANT_TTL,
        },
        herdr.socket.clone(),
    )
    .await
    .unwrap();
    let (machine, _) = pair(&handle.control_path(), core, LABEL).await;
    assert!(
        !machine.terminal_key.is_empty(),
        "the key goes with the pairing"
    );
    let m = machine.id.clone();
    let snapshot = collied::herdr::session_snapshot(&herdr.socket)
        .await
        .unwrap();
    let shell = snapshot.panes[0].terminal_id.clone();

    println!("the shell pane is listed, locked");
    let flock = connected_flock(core, &m).await;
    assert!(flock.terminals_enabled);
    let listed = flock
        .terminals
        .iter()
        .find(|t| t.terminal_id == shell)
        .expect("the shell pane is listed");
    assert!(listed.locked);
    assert_eq!(listed.workspace_label.as_deref(), Some("collie-e2e"));
    let err = core
        .terminal_run(m.clone(), shell.clone(), "echo locked".into())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::TerminalLocked), "{err:?}");
    let err = core
        .terminal_run(m.clone(), shell.clone(), "echo a\necho b".into())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::InvalidInput { .. }), "{err:?}");

    println!("a signed grant unlocks it: watch, run, keys");
    unlock(core, &m, &shell, &key).await;
    let view = core.agent_view(m.clone(), shell.clone(), 0).unwrap();
    assert!(view.agent.is_none() && view.terminal.is_some() && view.terminals_enabled);
    assert!(!view.terminal_locked);
    core.watch_terminal(m.clone(), shell.clone(), 200)
        .await
        .unwrap();
    core.terminal_run(m.clone(), shell.clone(), "echo collie-$((6 * 7))".into())
        .await
        .unwrap();
    wait_view(core, &m, &shell, "the command's output", |v| {
        v.output
            .as_ref()
            .is_some_and(|o| o.ansi.contains("collie-42"))
    })
    .await;
    core.terminal_send_keys(m.clone(), shell.clone(), vec![AgentKey::CtrlC])
        .await
        .unwrap();

    println!("locking ends the grant here and on the machine");
    core.lock_terminals().await;
    assert!(
        core.agent_view(m.clone(), shell.clone(), 0)
            .unwrap()
            .terminal_locked
    );
    let err = core
        .terminal_run(m.clone(), shell.clone(), "echo after lock".into())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::TerminalLocked), "{err:?}");

    println!("a reconnect starts locked and does not watch the shell again");
    unlock(core, &m, &shell, &key).await;
    core.watch_terminal(m.clone(), shell.clone(), 200)
        .await
        .unwrap();
    let before = watches(&audit);
    core.resume(60);
    wait_view(core, &m, &shell, "a new session", |v| {
        v.terminal_locked && format!("{:?}", v.link) == "Connected"
    })
    .await;
    connected_flock(core, &m).await;
    let err = core
        .terminal_run(m.clone(), shell.clone(), "echo new session".into())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::TerminalLocked), "{err:?}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(watches(&audit), before);

    let lines = audit_lines(&audit);
    assert!(
        lines
            .iter()
            .any(|e| e["method"] == "terminal.grant" && e["result"] == "granted ttl=300s"),
        "{lines:#?}"
    );
    let text = std::fs::read_to_string(&audit).unwrap();
    assert!(!text.contains("collie-$"), "typed text in the audit log");
    handle.shutdown().await;
}
