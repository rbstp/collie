#[allow(dead_code)]
mod common;

use std::collections::VecDeque;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use collie_core::{AgentKey, CollieCore, CoreError, TaskOptions, TaskStarted, TerminalSource};
use collied::config::TasksConfig;
use collied::control::{Reply, Request};
use collied::server::{self, ServerConfig, ServerHandle};
use common::*;
use protocol::AgentKind;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::watch;

const CLAUDE: &str = "term_65ce7ae4fd5731";
const CODEX_BLOCKED: &str = "term_0a1b2c3d4e5f60";
const NEW_TERMINAL: &str = "term_e2e00000000009";
const HELD_PROMPT: &str = "held until the phone has reconnected";
const RULE: &str = "\u{1b}[38;2;136;136;136m────────────────────────────────────────\u{1b}[39m";
const PLACEHOLDER: &str = "❯ \u{1b}[0m\u{1b}[2mTry \"create a util logging.py that...\"\u{1b}[0m";
const BASH: &str = "\
────────────────────────────────────────
 Bash command

   rm -rf build

 Do you want to proceed?
 ❯ 1. Yes
   2. No, and tell Claude what to do differently (esc)
";
const QUESTION: &str = "\
────────────────────────────────────────
 Which storage backend should the cache use?

   1. SQLite
   2. Redis
 ❯ 3. Type something.

 Enter to select · ↑/↓ to navigate · Esc to cancel
";
const MUTATING: [&str; 10] = [
    "agent.prompt",
    "agent.send_keys",
    "agent.focus",
    "agent.start",
    "workspace.create",
    "workspace.close",
    "pane.close",
    "pane.send_text",
    "pane.send_input",
    "pane.send_keys",
];

#[test]
fn phase2_end_to_end() {
    if !in_child("phase2_end_to_end") {
        return;
    }
    let t0 = Instant::now();
    let root = TempDir::new("e2e2");
    let net = Net::start(&root.0);
    wait_ready(&net.mac, 0);
    let core = phone(&root.0, "phone", &net);
    let rt = runtime();
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    wait_ready(&net.mac, 1);
    wait_phone(&rt, &core);
    println!("tailnet: control, Mac, phone up in {:?}", t0.elapsed());
    rt.block_on(scenario(&root.0, &net, &core));
    drop(core);
    drop(rt);
    println!("total {:?}", t0.elapsed());
}

async fn scenario(root: &Path, net: &Net, core: &Arc<CollieCore>) {
    let projects = root.join("projects");
    let app = projects.join("app");
    let outside = root.join("outside");
    for dir in [&app, &outside] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::os::unix::fs::symlink(&outside, projects.join("escape")).unwrap();
    let app_real = std::fs::canonicalize(&app).unwrap();
    let outside_real = std::fs::canonicalize(&outside).unwrap();

    let herdr = Mock::start(&root.join("herdr.sock"), &app_real, &outside_real);
    let data_dir = root.join("collied");
    let handle = start_collied(
        net,
        &data_dir,
        "e2e",
        herdr.socket.clone(),
        "claude",
        &projects,
    )
    .await;
    let control = handle.control_path();
    let audit = data_dir.join("audit.log");
    let (machine, _) = pair(&control, core, LABEL).await;
    let m = machine.id.clone();
    connected_flock(core, &m).await;

    println!("agent.read returns sanitized ANSI");
    let t = Instant::now();
    let read = core
        .agent_read(m.clone(), CLAUDE.into(), TerminalSource::Visible, None)
        .await
        .unwrap();
    assert_eq!(read.ansi, "\u{1b}[1mhello\u{1b}[0m world\r\nline 2");
    assert!(read.truncated);
    assert_eq!(read.source, TerminalSource::Visible);
    assert_sgr_only(&read.ansi);
    assert_eq!(
        herdr.params("agent.read").last().unwrap(),
        &json!({"target": "w6:p1", "source": "visible", "format": "ansi"})
    );
    println!("  read in {:?}", t.elapsed());

    println!("agent.watch delivers agent.output in order, and stops after unwatch");
    let t = Instant::now();
    herdr.with(|h| h.recent = recent(0));
    core.watch_agent(m.clone(), Some(CLAUDE.into()), 300)
        .await
        .unwrap();
    let mut seen: Vec<String> = Vec::new();
    let mut revision = 0;
    for step in 0..4 {
        herdr.with(|h| h.recent = recent(step));
        let want = format!("{}step {step}\r\n", history());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(view) = core.agent_view(m.clone(), CLAUDE.into(), revision)
                && let Some(out) = view.output
            {
                assert!(view.output_revision > revision);
                revision = view.output_revision;
                assert_eq!(out.source, TerminalSource::Recent);
                assert_sgr_only(&out.ansi);
                if seen.last() != Some(&out.ansi) {
                    seen.push(out.ansi.clone());
                }
                if out.ansi == want {
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "no output {want:?}, saw {seen:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    let expected: Vec<String> = (0..4)
        .map(|s| format!("{}step {s}\r\n", history()))
        .collect();
    assert_eq!(seen, expected, "outputs out of order or unexpected");
    assert_eq!(herdr.params("agent.read").last().unwrap()["lines"], 300);
    core.watch_agent(m.clone(), None, 200).await.unwrap();
    let polls = herdr.params("agent.read").len();
    herdr.with(|h| h.recent = recent(9));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        herdr.params("agent.read").len(),
        polls,
        "collied kept polling after unwatch"
    );
    let view = core.agent_view(m.clone(), CLAUDE.into(), 0).unwrap();
    assert!(view.output.is_none(), "{view:?}");
    assert_eq!(view.output_revision, revision);
    println!("  watched in {:?}", t.elapsed());

    println!("suspend drops the session and its watch at once, resume brings both back");
    core.watch_agent(m.clone(), Some(CLAUDE.into()), 300)
        .await
        .unwrap();
    wait_output(core, &m, CLAUDE, "step 9").await;
    let t = Instant::now();
    core.suspend(core.begin_suspend()).await;
    let deadline = Instant::now() + Duration::from_secs(2);
    while sessions(&control).await > 0 {
        assert!(
            Instant::now() < deadline,
            "collied kept the suspended phone's session"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    println!("  session gone {:?} after suspend", t.elapsed());
    let polls = herdr.params("agent.read").len();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        herdr.params("agent.read").len(),
        polls,
        "collied kept watching for a suspended phone"
    );
    assert_eq!(sessions(&control).await, 0, "a suspended phone dialed");
    core.resume(1);
    herdr.with(|h| h.recent = recent(10));
    wait_output(core, &m, CLAUDE, "step 10").await;
    assert_eq!(sessions(&control).await, 1);
    assert_eq!(herdr.params("agent.read").last().unwrap()["lines"], 300);
    core.watch_agent(m.clone(), None, 200).await.unwrap();

    println!("agent.prompt reaches herdr with the exact text");
    let text = "Fix the flaky test\nthen run `cargo test` \"quoted\"\tand say ✓ é";
    core.prompt(m.clone(), CLAUDE.into(), text.into(), None)
        .await
        .unwrap();
    assert_eq!(
        herdr.params("agent.prompt"),
        vec![json!({"target": "w6:p1", "text": text})]
    );

    println!("a blocked agent takes no prompt, and keys or text only without a decision");
    let before = herdr.mutations();
    for terminal in [CODEX_BLOCKED, CLAUDE] {
        // CLAUDE is listed as working, but agent.get just before the write says blocked.
        herdr.with(|h| h.claude_blocked = terminal == CLAUDE);
        let err = core
            .prompt(m.clone(), terminal.into(), "yes".into(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::AgentBlocked), "{err:?}");
    }
    herdr.with(|h| h.blocked_on = Some(BASH.into()));
    let err = core
        .send_keys(m.clone(), CLAUDE.into(), vec![AgentKey::Enter])
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::AgentBlocked), "{err:?}");
    let err = core
        .type_text(m.clone(), CLAUDE.into(), "yes".into())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::AgentBlocked), "{err:?}");
    assert_eq!(herdr.mutations(), before, "a permission prompt got input");
    herdr.with(|h| h.blocked_on = Some(QUESTION.into()));
    core.send_keys(m.clone(), CLAUDE.into(), vec![AgentKey::Down])
        .await
        .unwrap();
    core.type_text(m.clone(), CLAUDE.into(), "DuckDB".into())
        .await
        .unwrap();
    assert_eq!(
        herdr.mutation_calls()[before.len()..],
        [
            (
                "agent.send_keys".to_owned(),
                json!({"target": "w6:p1", "keys": ["down"]})
            ),
            (
                "pane.send_text".to_owned(),
                json!({"pane_id": "w6:p1", "text": "DuckDB"})
            ),
            (
                "agent.send_keys".to_owned(),
                json!({"target": "w6:p1", "keys": ["enter"]})
            ),
        ]
    );
    herdr.with(|h| {
        h.claude_blocked = false;
        h.blocked_on = None;
    });
    let lines = audit_lines(&audit);
    let keyed: Vec<(&Value, &Value)> = lines
        .iter()
        .filter(|l| l["method"] == "agent.send_keys" || l["method"] == "agent.type_text")
        .map(|l| (&l["target"], &l["result"]))
        .collect();
    assert_eq!(
        keyed[keyed.len() - 2..],
        [
            (&json!(format!("{CLAUDE} blocked keys=down")), &json!("ok")),
            (&json!(CLAUDE), &json!("ok")),
        ]
    );
    assert!(
        !std::fs::read_to_string(&audit).unwrap().contains("DuckDB"),
        "typed text reached the audit log"
    );

    println!("send_keys maps every key in order");
    let keys = vec![
        AgentKey::Esc,
        AgentKey::Enter,
        AgentKey::Up,
        AgentKey::Down,
        AgentKey::Tab,
        AgentKey::ShiftTab,
        AgentKey::CtrlC,
    ];
    core.send_keys(m.clone(), CLAUDE.into(), keys)
        .await
        .unwrap();
    assert_eq!(
        herdr.params("agent.send_keys").last(),
        Some(&json!({"target": "w6:p1", "keys": [
            "esc", "enter", "up", "down", "tab", "shift+tab", "ctrl+c"
        ]}))
    );

    println!("task.options and task.new");
    let t = Instant::now();
    let options = core.task_options(m.clone()).await.unwrap();
    assert_eq!(
        options,
        TaskOptions {
            agents: vec!["claude".into()],
            default_agent: "claude".into(),
            recent_cwds: vec![app_real.to_str().unwrap().into()],
        }
    );
    let before = herdr.mutations();
    let s = |p: &Path| p.to_str().unwrap().to_owned();
    for (cwd, agent, message) in [
        (s(&outside), "claude", "outside the allowed roots"),
        (
            s(&projects.join("escape")),
            "claude",
            "outside the allowed roots",
        ),
        (
            s(&app.join("../../outside")),
            "claude",
            "outside the allowed roots",
        ),
        (s(&projects.join("missing")), "claude", "does not exist"),
        (s(&app), "codex", "not allowed"),
    ] {
        let err = core
            .task_new(m.clone(), cwd.clone(), agent.into(), "go".into(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CoreError::InvalidInput { message: msg, .. } if msg.contains(message)),
            "{cwd} {agent}: {err:?}"
        );
    }
    assert_eq!(herdr.mutations(), before, "a rejected task reached herdr");
    let started = core
        .task_new(
            m.clone(),
            s(&app),
            "claude".into(),
            "Write the tests".into(),
            Some("app task".into()),
        )
        .await
        .unwrap();
    assert_eq!(
        started,
        TaskStarted {
            workspace_id: "w9".into(),
            terminal_id: NEW_TERMINAL.into(),
        }
    );
    let calls = herdr.mutation_calls();
    let task: Vec<&(String, Value)> = calls[before.len()..].iter().collect();
    assert_eq!(task.len(), 3, "{task:?}");
    assert_eq!(
        task[0],
        &(
            "workspace.create".to_owned(),
            json!({"cwd": s(&app_real), "label": "app task", "focus": false})
        )
    );
    assert_eq!(task[1].0, "agent.start");
    assert_eq!(task[1].1["kind"], "claude");
    assert_eq!(task[1].1["pane_id"], "w9:p1");
    assert!(
        task[1].1["name"]
            .as_str()
            .is_some_and(|n| n.starts_with("collie-")),
        "{task:?}"
    );
    assert_eq!(
        task[2],
        &(
            "agent.prompt".to_owned(),
            json!({"target": task[1].1["name"], "text": "Write the tests"})
        )
    );
    println!("  task.new in {:?}", t.elapsed());

    println!("a draft typed on the Mac is shown, then replaced, never appended to");
    let input_box =
        |rows: &str| format!("⏺ Done.\r\n{RULE}\r\n{rows}\r\n{RULE}\r\n  ⏵⏵ auto mode on\r\n");
    let typed = input_box("❯\u{a0}half a thought\n  on two lines");
    herdr.with(|h| h.screens = [typed.clone()].into());
    let draft = core.agent_draft(m.clone(), CLAUDE.into()).await.unwrap();
    assert_eq!(draft.as_deref(), Some("half a thought\non two lines"));
    assert_eq!(
        core.agent_draft(m.clone(), CODEX_BLOCKED.into())
            .await
            .unwrap(),
        None
    );
    let before = herdr.mutations();
    let err = core
        .prompt(m.clone(), CLAUDE.into(), "replace it".into(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CoreError::DraftChanged { current } if Some(current) == draft.as_ref()),
        "{err:?}"
    );
    assert_eq!(herdr.mutations(), before, "a changed draft got input");
    herdr.with(|h| h.screens = [typed, input_box(PLACEHOLDER)].into());
    core.prompt(m.clone(), CLAUDE.into(), "replace it".into(), draft)
        .await
        .unwrap();
    assert_eq!(
        herdr.mutation_calls()[before.len()..],
        [
            (
                "agent.send_keys".to_owned(),
                json!({"target": "w6:p1", "keys": [
                    "down", "ctrl+e", "ctrl+u", "backspace", "ctrl+e", "ctrl+u", "backspace"
                ]})
            ),
            (
                "agent.prompt".to_owned(),
                json!({"target": "w6:p1", "text": "replace it"})
            ),
        ]
    );
    assert_eq!(
        core.agent_draft(m.clone(), CLAUDE.into()).await.unwrap(),
        Some(String::new())
    );
    herdr.with(|h| h.screens.clear());
    let results: Vec<Value> = audit_lines(&audit)
        .into_iter()
        .filter(|l| l["method"] == "agent.prompt")
        .map(|l| l["result"].clone())
        .collect();
    assert!(results.contains(&json!("draft_changed")), "{results:?}");
    assert!(
        !std::fs::read_to_string(&audit)
            .unwrap()
            .contains("half a thought"),
        "the draft reached the audit log"
    );

    println!("a prompt retried after a dropped connection runs once");
    let t = Instant::now();
    assert_eq!(sessions(&control).await, 1);
    let pending = tokio::spawn({
        let (core, m) = (core.clone(), m.clone());
        async move {
            core.prompt(m, CLAUDE.into(), HELD_PROMPT.into(), None)
                .await
        }
    });
    wait_for("herdr to accept the held prompt", || {
        herdr.prompts(HELD_PROMPT) == 1
    })
    .await;
    // Simulates iOS killing the socket: the core drops the connection and reconnects.
    core.resume(60);
    let deadline = Instant::now() + Duration::from_secs(4);
    while sessions(&control).await < 2 {
        assert!(Instant::now() < deadline, "the phone did not reconnect");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    herdr.release.send_replace(true);
    pending.await.unwrap().unwrap();
    assert_eq!(
        herdr.prompts(HELD_PROMPT),
        1,
        "the retry ran the prompt again"
    );
    let lines = audit_lines(&audit);
    let results: Vec<&str> = lines
        .iter()
        .filter(|l| l["method"] == "agent.prompt" && l["target"] == CLAUDE)
        .filter_map(|l| l["result"].as_str())
        .collect();
    assert_eq!(
        results.iter().filter(|r| **r == "ok (replayed)").count(),
        1,
        "{results:?}"
    );
    println!("  replayed in {:?}", t.elapsed());

    println!("closing without confirm is refused before herdr");
    let before = herdr.mutations();
    let err = core
        .close_workspace(m.clone(), "w9".into(), false)
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::ConfirmRequired), "{err:?}");
    let err = core
        .close_pane(m.clone(), NEW_TERMINAL.into(), false)
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::ConfirmRequired), "{err:?}");
    assert_eq!(herdr.mutations(), before);
    core.close_workspace(m.clone(), "w9".into(), true)
        .await
        .unwrap();
    assert_eq!(
        herdr.params("workspace.close"),
        vec![json!({"workspace_id": "w9"})]
    );

    println!("every herdr write, in order, and no kernel TCP listener");
    let writes: Vec<String> = herdr.mutations();
    assert_eq!(
        writes,
        [
            "agent.prompt",
            "agent.send_keys",
            "pane.send_text",
            "agent.send_keys",
            "agent.send_keys",
            "workspace.create",
            "agent.start",
            "agent.prompt",
            "agent.send_keys",
            "agent.prompt",
            "agent.prompt",
            "workspace.close",
        ]
    );
    assert_eq!(kernel_tcp_listeners(), Vec::<String>::new());
    handle.shutdown().await;
}

#[test]
fn live_herdr_drive() {
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
    if !in_child("live_herdr_drive") {
        return;
    }
    let t0 = Instant::now();
    let mut herdr = HerdrSession::start();
    let fake_pi = herdr.root.join("bin/pi");
    std::fs::write(
        &fake_pi,
        "#!/bin/sh\nprintf 'fake pi ready\\n> '\n\
         while IFS= read -r line; do printf 'pi got: %s.\\n> ' \"$line\"; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake_pi, std::fs::Permissions::from_mode(0o700)).unwrap();
    // agent.start types `pi` into the pane's login shell; anything else on PATH named pi
    // would be a real agent receiving the test's prompts.
    let pi_is_fake = ["-c", "-lc"].iter().all(|flag| {
        isolated("/bin/sh", &herdr.root)
            .args([*flag, "command -v pi"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == fake_pi.to_str().unwrap())
    });
    println!(
        "dedicated herdr session {} up in {:?}",
        herdr.name,
        t0.elapsed()
    );
    let root = TempDir::new("e2e2-herdr");
    let net = Net::start(&root.0);
    wait_ready(&net.mac, 0);
    let core = phone(&root.0, "phone", &net);
    let rt = runtime();
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    wait_ready(&net.mac, 1);
    wait_phone(&rt, &core);
    rt.block_on(live_scenario(&root.0, &net, &core, &herdr, pi_is_fake));
    drop(core);
    drop(rt);
    assert!(herdr.stop(), "dedicated herdr session did not stop");
    println!("total {:?}", t0.elapsed());
}

async fn live_scenario(
    root: &Path,
    net: &Net,
    core: &Arc<CollieCore>,
    herdr: &HerdrSession,
    pi_is_fake: bool,
) {
    let work = herdr.root.join("collie-e2e");
    let project = work.join("proj");
    std::fs::create_dir(&project).unwrap();
    std::os::unix::fs::symlink(herdr.root.join("home"), work.join("escape")).unwrap();
    let handle = start_collied(
        net,
        &root.join("collied"),
        &herdr.name,
        herdr.socket.clone(),
        "pi",
        &work,
    )
    .await;
    let (machine, _) = pair(&handle.control_path(), core, LABEL).await;
    let m = machine.id.clone();
    connected_flock(core, &m).await;
    let snapshot = collied::herdr::session_snapshot(&herdr.socket)
        .await
        .unwrap();
    let shell = snapshot.panes[0].terminal_id.clone();
    let workspaces = || async {
        collied::herdr::workspace_list(&herdr.socket)
            .await
            .unwrap()
            .into_iter()
            .map(|w| w.workspace_id)
            .collect::<Vec<_>>()
    };
    let initial = workspaces().await;

    println!("task.options, refused cwds, shell read and unconfirmed close");
    let options = core.task_options(m.clone()).await.unwrap();
    assert_eq!(options.agents, ["pi"]);
    let s = |p: &Path| p.to_str().unwrap().to_owned();
    for cwd in [s(&work.join("escape")), "/tmp".into()] {
        let err = core
            .task_new(m.clone(), cwd, "pi".into(), "go".into(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidInput { .. }), "{err:?}");
    }
    assert_eq!(workspaces().await, initial);
    let err = core
        .agent_read(m.clone(), shell.clone(), TerminalSource::Recent, None)
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound), "{err:?}");
    let err = core
        .close_pane(m.clone(), shell.clone(), false)
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::ConfirmRequired), "{err:?}");
    let err = core
        .close_workspace(m.clone(), initial[0].clone(), false)
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::ConfirmRequired), "{err:?}");
    assert_eq!(workspaces().await, initial);

    if !pi_is_fake {
        println!("skipped task.new: another `pi` executable is on the dedicated session's PATH");
        handle.shutdown().await;
        return;
    }

    println!("task.new starts the fake pi agent and prompts it");
    let t = Instant::now();
    let started = core
        .task_new(
            m.clone(),
            s(&project),
            "pi".into(),
            "hello from collie".into(),
            Some("e2e task".into()),
        )
        .await
        .unwrap();
    println!("  started in {:?}", t.elapsed());
    let term = started.terminal_id.clone();
    core.watch_agent(m.clone(), Some(term.clone()), 200)
        .await
        .unwrap();
    wait_output(core, &m, &term, "pi got: hello from collie").await;
    core.prompt(m.clone(), term.clone(), "second prompt".into(), None)
        .await
        .unwrap();
    wait_output(core, &m, &term, "pi got: second prompt").await;
    core.send_keys(m.clone(), term.clone(), vec![AgentKey::Enter])
        .await
        .unwrap();
    wait_output(core, &m, &term, "pi got: .\r\n").await;
    let visible = core
        .agent_read(m.clone(), term.clone(), TerminalSource::Visible, None)
        .await
        .unwrap();
    assert!(
        visible.ansi.contains("pi got: second prompt"),
        "{visible:?}"
    );
    assert_sgr_only(&visible.ansi);
    core.watch_agent(m.clone(), None, 200).await.unwrap();

    println!("confirmed workspace.close removes the task workspace");
    assert!(workspaces().await.contains(&started.workspace_id));
    let err = core
        .close_workspace(m.clone(), started.workspace_id.clone(), false)
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::ConfirmRequired), "{err:?}");
    core.close_workspace(m.clone(), started.workspace_id.clone(), true)
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while workspaces().await.contains(&started.workspace_id) {
        assert!(Instant::now() < deadline, "workspace still open");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(workspaces().await, initial);
    handle.shutdown().await;
}

async fn start_collied(
    net: &Net,
    data_dir: &Path,
    session: &str,
    socket: PathBuf,
    agent: &str,
    root: &Path,
) -> ServerHandle {
    server::start_with(
        net.mac.clone(),
        ServerConfig {
            data_dir: data_dir.to_owned(),
            port: PORT,
            owner_user_id: None,
            herdr_session: session.into(),
            machine_name: "e2e-mac".into(),
            approval_ttl: collied::approvals::TTL,
            attachments_dir: data_dir.join("attachments"),
            terminals: false,
            terminal_grant_ttl: collied::terminal::GRANT_TTL,
        },
        socket,
        &TasksConfig {
            agents: vec![AgentKind::new(agent).unwrap()],
            roots: Some(vec![root.to_owned()]),
        },
        None,
    )
    .await
    .unwrap()
}

async fn sessions(control: &Path) -> usize {
    match collied::control::request(control, &Request::Status).await {
        Ok(Some(Reply::Status(s))) => s.sessions,
        other => panic!("no status: {other:?}"),
    }
}

async fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_output(core: &CollieCore, m: &str, terminal: &str, want: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut last = None;
    loop {
        if let Some(out) = core
            .agent_view(m.into(), terminal.into(), 0)
            .and_then(|v| v.output)
        {
            assert_sgr_only(&out.ansi);
            if out.ansi.contains(want) {
                return;
            }
            last = Some(out.ansi);
        }
        assert!(
            Instant::now() < deadline,
            "output never showed {want:?}: {last:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Every ESC must open a plain SGR (`ESC [ digits ; : m`), and no other control is left
/// but CR, LF and TAB.
fn assert_sgr_only(s: &str) {
    let mut rest = s;
    while let Some(i) = rest.find('\u{1b}') {
        let params = rest[i + 1..]
            .strip_prefix('[')
            .unwrap_or_else(|| panic!("bare ESC in {s:?}"));
        let end = params
            .find(|c: char| !matches!(c, '0'..='9' | ';' | ':'))
            .unwrap_or(params.len());
        assert!(params[end..].starts_with('m'), "non-SGR escape in {s:?}");
        rest = &params[end + 1..];
    }
    assert!(
        !s.chars()
            .any(|c| c.is_control() && !matches!(c, '\u{1b}' | '\r' | '\n' | '\t')),
        "control character in {s:?}"
    );
}

fn recent(step: u32) -> String {
    format!(
        "{}step {step}\u{1b}]52;c;ZXZpbA==\u{7}\u{1b}[H\r\n",
        history()
    )
}

/// Long enough that collied sends each later step as an `agent.output_patch`.
fn history() -> String {
    "\u{1b}[2mhistory\u{1b}[0m\r\n".repeat(50)
}

#[derive(Default)]
struct Herdr {
    snapshot: Value,
    calls: Vec<(String, Value)>,
    recent: String,
    screens: VecDeque<String>,
    claude_blocked: bool,
    /// What `pane.read source=detection` shows; `pane.send_text` types into its field.
    blocked_on: Option<String>,
    started: Option<String>,
}

/// One request per connection, like herdr. An `agent.prompt` carrying `HELD_PROMPT` is
/// recorded on arrival but answered only once `release` is set.
struct Mock {
    state: Arc<Mutex<Herdr>>,
    socket: PathBuf,
    release: watch::Sender<bool>,
}

impl Mock {
    fn start(socket: &Path, app: &Path, outside: &Path) -> Self {
        let fixture: Value =
            serde_json::from_str(include_str!("fixtures/session.snapshot.json")).unwrap();
        let mut snapshot = fixture["result"]["snapshot"].clone();
        for pane in snapshot["panes"].as_array_mut().unwrap() {
            let cwd = if pane["workspace_id"] == "w6" {
                app
            } else {
                outside
            };
            pane["cwd"] = json!(cwd);
        }
        let state = Arc::new(Mutex::new(Herdr {
            snapshot,
            ..Default::default()
        }));
        let (release, _) = watch::channel(false);
        let listener = UnixListener::bind(socket).unwrap();
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (shared, gate) = (state.clone(), release.clone());
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (state, mut released) = (shared.clone(), gate.subscribe());
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
                    let body = answer(&mut state.lock().unwrap(), &req);
                    if req["method"] == "agent.prompt" && req["params"]["text"] == HELD_PROMPT {
                        let _ = released.wait_for(|r| *r).await;
                    }
                    let mut resp = json!({ "id": req["id"] });
                    match body {
                        Ok(result) => resp["result"] = result,
                        Err(code) => resp["error"] = json!({ "code": code, "message": "mock" }),
                    }
                    let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
                });
            }
        });
        Self {
            state,
            socket: socket.to_owned(),
            release,
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Herdr) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    fn params(&self, method: &str) -> Vec<Value> {
        self.with(|h| {
            h.calls
                .iter()
                .filter(|(m, _)| m == method)
                .map(|(_, p)| p.clone())
                .collect()
        })
    }

    fn prompts(&self, text: &str) -> usize {
        self.params("agent.prompt")
            .iter()
            .filter(|p| p["text"] == text)
            .count()
    }

    fn mutation_calls(&self) -> Vec<(String, Value)> {
        self.with(|h| {
            h.calls
                .iter()
                .filter(|(m, _)| MUTATING.contains(&m.as_str()))
                .cloned()
                .collect()
        })
    }

    fn mutations(&self) -> Vec<String> {
        self.mutation_calls().into_iter().map(|(m, _)| m).collect()
    }
}

fn answer(h: &mut Herdr, req: &Value) -> Result<Value, String> {
    let method = req["method"].as_str().unwrap_or_default().to_owned();
    let p = req["params"].clone();
    h.calls.push((method.clone(), p.clone()));
    let agent_by_pane = |h: &Herdr, pane: &Value| {
        h.snapshot["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["pane_id"] == *pane)
            .cloned()
            .ok_or_else(|| "agent_not_found".to_owned())
    };
    Ok(match method.as_str() {
        "ping" => json!({"type": "pong", "version": "0.9.3", "protocol": 1}),
        "session.snapshot" => json!({"type": "session_snapshot", "snapshot": h.snapshot}),
        "agent.list" => json!({"type": "agent_list", "agents": h.snapshot["agents"]}),
        "workspace.list" => {
            json!({"type": "workspace_list", "workspaces": h.snapshot["workspaces"]})
        }
        "server.agent_manifests" => json!({"type": "agent_manifest_status", "manifests": [
            {"agent": "pi", "source": "builtin", "source_kind": "builtin", "local_override_shadowing_remote": false},
            {"agent": "claude", "source": "builtin", "source_kind": "builtin", "local_override_shadowing_remote": false},
        ]}),
        "agent.get" => {
            let target = &p["target"];
            let agent = if target == "w9:p1" || (h.started.is_some() && *target == json!(h.started))
            {
                new_agent(h.started.as_deref(), "idle", true, false)
            } else {
                let mut a = agent_by_pane(h, target)?;
                if h.claude_blocked && a["terminal_id"] == CLAUDE {
                    a["agent_status"] = json!("blocked");
                }
                a
            };
            json!({"type": "agent_info", "agent": agent})
        }
        "agent.read" => {
            let text = match p["source"].as_str() {
                Some("visible") => {
                    "\u{1b}[1mhello\u{1b}[0m\u{1b}]52;c;cm0gLXJmIH4K\u{7} \u{1b}[2J\u{1b}[10;5Hworld\u{1b}[?1049h\r\nline 2\u{1b}[6n"
                        .to_owned()
                }
                _ => h.recent.clone(),
            };
            json!({"type": "pane_read", "read": {
                "pane_id": p["target"], "workspace_id": "w6", "tab_id": "w6:t1",
                "source": p["source"], "format": "ansi", "text": text, "revision": 0,
                "truncated": true,
            }})
        }
        "pane.read" if p["source"] == "detection" => json!({"type": "pane_read", "read": {
            "pane_id": p["pane_id"], "workspace_id": "w6", "tab_id": "w6:t1",
            "source": "detection", "format": "text",
            "text": h.blocked_on.as_deref().unwrap_or(""), "revision": 0, "truncated": false,
        }}),
        "agent.explain" => json!({"type": "agent_explain", "explain": {
            "matched_rule": h.blocked_on.as_ref().map(|_| json!({"id": "live_blocked_form"})),
        }}),
        "pane.send_text" => {
            if let Some(screen) = &mut h.blocked_on {
                let typed = format!("❯ 3. {}", p["text"].as_str().unwrap());
                *screen = screen.replace("❯ 3. Type something.", &typed);
            }
            json!({"type": "ok"})
        }
        "pane.read" => {
            let text = match h.screens.len() {
                0 => format!("{RULE}\r\n{PLACEHOLDER}\r\n{RULE}\r\n"),
                1 => h.screens[0].clone(),
                _ => h.screens.pop_front().unwrap(),
            };
            json!({"type": "pane_read", "read": {
                "pane_id": p["pane_id"], "workspace_id": "w6", "tab_id": "w6:t1",
                "source": p["source"], "format": "ansi", "text": text, "revision": 0,
                "truncated": false,
            }})
        }
        "agent.prompt" => {
            json!({"type": "agent_prompted", "agent": agent_by_pane(h, &p["target"]).unwrap_or(json!({}))})
        }
        "agent.send_keys" | "workspace.close" | "pane.close" => {
            json!({"type": "ok"})
        }
        "workspace.create" => json!({"type": "workspace_created",
            "workspace": {"workspace_id": "w9", "number": 3, "label": p["label"], "focused": false,
                "pane_count": 1, "tab_count": 1, "active_tab_id": "w9:t1", "agent_status": "unknown"},
            "tab": {"tab_id": "w9:t1", "workspace_id": "w9", "number": 1, "label": "1",
                "focused": false, "pane_count": 1, "agent_status": "unknown"},
            "root_pane": {"pane_id": "w9:p1", "terminal_id": NEW_TERMINAL, "workspace_id": "w9",
                "tab_id": "w9:t1", "focused": false, "cwd": p["cwd"], "agent_status": "unknown",
                "revision": 0},
        }),
        "agent.start" => {
            h.started = p["name"].as_str().map(str::to_owned);
            json!({"type": "agent_started", "argv": [p["kind"]],
                "agent": new_agent(h.started.as_deref(), "unknown", false, true)})
        }
        _ => return Err("unknown_method".into()),
    })
}

fn new_agent(name: Option<&str>, status: &str, ready: bool, pending: bool) -> Value {
    json!({
        "terminal_id": NEW_TERMINAL, "pane_id": "w9:p1", "workspace_id": "w9", "tab_id": "w9:t1",
        "focused": false, "name": name, "agent": "claude", "agent_status": status,
        "interactive_ready": ready, "launch_pending": pending, "revision": 0,
    })
}
