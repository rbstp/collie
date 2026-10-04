use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use collied::approvals::Approvals;
use collied::audit::Audit;
use collied::drive::{Authorized, Driver};
use collied::herdr;
use protocol::{
    AgentKind, AgentPromptParams, AgentSendKeysParams, AgentTypeTextParams, ApprovalDecideParams,
    ApprovalOutcome, Cwd, Decision, ErrorCode, Event, Key, Label, OpId, PaneCloseParams,
    PromptText, ReadParams, ReadSource, Response, TaskNewParams, TerminalId, WorkspaceCloseParams,
    WorkspaceId,
};
use tokio::sync::broadcast;

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
    let mut session = HerdrSession::start("drv");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(scenario(&session));
    drop(rt);
    assert!(session.stop(), "dedicated herdr session did not stop");
}

// A stand-in `claude` that draws Claude Code's Bash permission prompt and waits for one
// line of input, so real herdr detection, `agent.explain` and `agent.send_keys` drive the
// approval flow without the real agent.
const FAKE_CLAUDE: &str = r#"#!/bin/bash
exec -a claude /bin/bash -c '
cat "$1"
IFS= read -r answer
printf "\033[2J\033[H"
printf "\342\217\272 Removed build/\n"
exec -a claude sleep 600
' claude "$0.screen"
"#;

const FAKE_SCREEN: &str = "\
────────────────────────────────────────
 Bash command

   rm -rf build
   Remove the build directory

 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don't ask again for rm commands
   3. No, and tell Claude what to do differently (esc)

 Esc to cancel · Tab to amend · ctrl+e to explain
";

#[test]
fn live_herdr_approval() {
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
    let mut session = HerdrSession::start("apv");
    let bin = session.root.join("bin/claude");
    std::fs::write(&bin, FAKE_CLAUDE).unwrap();
    std::fs::write(session.root.join("bin/claude.screen"), FAKE_SCREEN).unwrap();
    std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(approval_scenario(&session));
    drop(rt);
    assert!(session.stop(), "dedicated herdr session did not stop");
}

async fn approval_scenario(session: &HerdrSession) {
    let socket = &session.socket;
    let work = std::fs::canonicalize(&session.work).unwrap();
    let created = herdr::workspace_create(socket, work.to_str().unwrap(), Some("collie-live"))
        .await
        .unwrap();
    let pane = created.root_pane.pane_id.clone();
    let terminal = created.root_pane.terminal_id.clone();
    let mut started = None;
    for _ in 0..100 {
        match herdr::agent_start(socket, "collie-live-claude", "claude", &pane).await {
            Ok(a) => {
                started = Some(a);
                break;
            }
            Err(herdr::Error::Herdr { code, .. }) if code == "agent_pane_busy" => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => panic!("agent.start: {e}"),
        }
    }
    assert_eq!(started.expect("agent never started").terminal_id, terminal);

    let blocked = async {
        loop {
            let agents = herdr::agent_list(socket).await.unwrap();
            if agents
                .iter()
                .any(|a| a.terminal_id == terminal && a.agent_status == "blocked")
            {
                return agents;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    let agents = tokio::time::timeout(Duration::from_secs(20), blocked)
        .await
        .expect("the fake claude never showed as blocked");
    let workspaces = herdr::workspace_list(socket).await.unwrap();

    let audit_dir = tempfile::tempdir().unwrap();
    let audit = Arc::new(Audit::open(&audit_dir.path().join("audit.log")).unwrap());
    let (tx, mut events) = broadcast::channel(16);
    let approvals = Approvals::new(socket.clone(), "nMAC".into(), tx, audit, None);
    approvals.observe(&agents, &workspaces).await;
    let Ok(Event::ApprovalNeeded { approval }) = events.try_recv() else {
        panic!("no approval for the blocked agent");
    };
    assert_eq!(approval.terminal_id.as_str(), terminal);
    assert_eq!(approval.workspace_label, "collie-live");
    assert_eq!(
        approval.options,
        [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
    );
    assert_eq!(
        approval.snippet,
        "Bash command\nrm -rf build\nRemove the build directory\nDo you want to proceed?"
    );
    assert_eq!(approval.choices.len(), 3);
    assert!(approval.choices[0].current);

    let yes: Authorized = Arc::new(|| true);
    let drive = Driver::new(
        socket.clone(),
        vec![AgentKind::new("claude").unwrap()],
        std::slice::from_ref(&work),
    )
    .unwrap();
    let terminal_id = TerminalId::new(terminal.clone()).unwrap();
    let (keys, _) = drive
        .send_keys(
            AgentSendKeysParams {
                op_id: OpId::new("K".repeat(22)).unwrap(),
                terminal_id: terminal_id.clone(),
                keys: vec![Key::Enter],
            },
            &yes,
        )
        .await;
    assert_eq!(keys.unwrap_err().0, ErrorCode::AgentBlocked);
    let typed = drive
        .type_text(
            AgentTypeTextParams {
                op_id: OpId::new("Y".repeat(22)).unwrap(),
                terminal_id,
                text: PromptText::new("yes").unwrap(),
            },
            &yes,
        )
        .await;
    assert_eq!(typed.unwrap_err().0, ErrorCode::AgentBlocked);
    let reply = approvals
        .decide(
            "live",
            "nLIVE",
            ApprovalDecideParams {
                approval_id: approval.approval_id.clone(),
                decision: Decision::Approve,
                choice: None,
                nonce: approval.nonce.clone(),
                note: None,
            },
            &yes,
        )
        .await;
    assert_eq!(
        reply,
        Ok(Response::ApprovalResolved {
            approval_id: approval.approval_id.clone(),
            outcome: ApprovalOutcome::Applied {
                decision: Decision::Approve,
                by: "live".into()
            },
        })
    );
    let screen = herdr::detection_text(socket, &pane).await.unwrap();
    assert!(screen.contains("Removed build/"), "{screen}");
    close_workspace(
        &Driver::new(socket.clone(), vec![], std::slice::from_ref(&work)).unwrap(),
        socket,
        WorkspaceId::new(created.root_pane.workspace_id).unwrap(),
    )
    .await;
}

async fn scenario(session: &HerdrSession) {
    let socket = &session.socket;
    let work = std::fs::canonicalize(&session.work).unwrap();
    let pi = which("pi");
    let mut agents = vec![AgentKind::new("claude").unwrap()];
    if pi {
        agents.insert(0, AgentKind::new("pi").unwrap());
    }
    let drive = Arc::new(Driver::new(socket.clone(), agents, std::slice::from_ref(&work)).unwrap());
    let yes: Authorized = Arc::new(|| true);

    let Ok(Response::TaskOptions(opts)) = drive.task_options().await else {
        panic!("no task options");
    };
    assert!(opts.agents.iter().any(|a| a.as_str() == "claude"));
    assert!(
        opts.recent_cwds
            .iter()
            .any(|c| Path::new(c.as_str()) == work),
        "{opts:?}"
    );

    let outside = TaskNewParams {
        op_id: OpId::new("O".repeat(22)).unwrap(),
        cwd: Cwd::new("/").unwrap(),
        agent: AgentKind::new("claude").unwrap(),
        prompt: PromptText::new("x").unwrap(),
        label: None,
    };
    assert_eq!(
        drive.task_new(outside, &yes).await.0.unwrap_err().0,
        ErrorCode::InvalidParams
    );

    if pi {
        let reply = drive
            .task_new(
                TaskNewParams {
                    op_id: OpId::new("T".repeat(22)).unwrap(),
                    cwd: Cwd::new(work.to_str().unwrap()).unwrap(),
                    agent: AgentKind::new("pi").unwrap(),
                    prompt: PromptText::new("say hello").unwrap(),
                    label: Some(Label::new("collie-live").unwrap()),
                },
                &yes,
            )
            .await
            .0;
        let Ok(Response::TaskStarted { workspace_id, .. }) = reply else {
            panic!("task.new with pi failed: {reply:?}");
        };
        close_workspace(&drive, socket, workspace_id).await;
        return;
    }
    println!("pi is not installed: exercising a plain shell workspace instead");

    let created = herdr::workspace_create(socket, work.to_str().unwrap(), Some("collie-live"))
        .await
        .unwrap();
    let terminal = TerminalId::new(created.root_pane.terminal_id.clone()).unwrap();
    let workspace = WorkspaceId::new(created.root_pane.workspace_id.clone()).unwrap();
    let pane = herdr::pane_get(socket, &created.root_pane.pane_id)
        .await
        .unwrap();
    assert_eq!(pane.terminal_id, terminal.as_str());
    let mut shell_seen = false;
    for _ in 0..50 {
        let info = herdr::pane_process_info(socket, &pane.pane_id)
            .await
            .unwrap();
        if collied::drive::shell_in_foreground(&info) {
            shell_seen = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        shell_seen,
        "the new pane's shell never showed in the foreground"
    );
    let read = ReadParams {
        terminal_id: terminal.clone(),
        source: ReadSource::Visible,
        lines: Some(20),
    };
    let Ok(Response::Terminal(screen)) = drive.read(read.clone(), false).await else {
        panic!("pane.read failed");
    };
    assert_eq!(screen.terminal_id, terminal);
    assert!(
        !screen
            .ansi
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\u{1b}' | '\r' | '\n' | '\t'))
    );
    let recent = ReadParams {
        source: ReadSource::Recent,
        ..read.clone()
    };
    let Ok(Response::Terminal(recent)) = drive.read(recent, false).await else {
        panic!("pane.read recent_unwrapped failed");
    };
    assert_eq!(recent.source, ReadSource::Recent);
    assert_eq!(
        drive.read(read, true).await.unwrap_err().0,
        ErrorCode::NotFound
    );
    let prompt = AgentPromptParams {
        op_id: OpId::new("P".repeat(22)).unwrap(),
        terminal_id: terminal.clone(),
        text: PromptText::new("echo should-not-run").unwrap(),
        expected_draft: None,
    };
    assert_eq!(
        drive.prompt(prompt, &yes).await.unwrap_err().0,
        ErrorCode::NotFound
    );

    let close = |confirm| PaneCloseParams {
        terminal_id: terminal.clone(),
        confirm,
    };
    assert_eq!(
        drive.pane_close(close(false), &yes).await.unwrap_err().0,
        ErrorCode::ConfirmRequired
    );
    assert_eq!(drive.pane_close(close(true), &yes).await, Ok(Response::Ok));
    let snap = herdr::session_snapshot(socket).await.unwrap();
    assert!(
        !snap
            .panes
            .iter()
            .any(|p| p.terminal_id == terminal.as_str())
    );
    // Closing a workspace's only pane may close the workspace with it.
    if snap
        .workspaces
        .iter()
        .any(|w| w.workspace_id == workspace.as_str())
    {
        close_workspace(&drive, socket, workspace).await;
    }

    let second = herdr::workspace_create(socket, work.to_str().unwrap(), None)
        .await
        .unwrap();
    close_workspace(
        &drive,
        socket,
        WorkspaceId::new(second.root_pane.workspace_id).unwrap(),
    )
    .await;
}

async fn close_workspace(drive: &Driver, socket: &Path, workspace_id: WorkspaceId) {
    let yes: Authorized = Arc::new(|| true);
    let params = |confirm| WorkspaceCloseParams {
        workspace_id: workspace_id.clone(),
        confirm,
    };
    assert_eq!(
        drive
            .workspace_close(params(false), &yes)
            .await
            .unwrap_err()
            .0,
        ErrorCode::ConfirmRequired
    );
    assert_eq!(
        drive.workspace_close(params(true), &yes).await,
        Ok(Response::Ok)
    );
    let left = herdr::workspace_list(socket).await.unwrap();
    assert!(!left.iter().any(|w| w.workspace_id == workspace_id.as_str()));
}

fn which(bin: &str) -> bool {
    Command::new("/bin/sh")
        .args(["-c", &format!("command -v {bin}")])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A dedicated named herdr session in its own HOME and XDG dirs with a scrubbed
/// environment, so neither `--session` precedence nor an inherited HERDR_SOCKET_PATH
/// can ever reach the user's default session.
/// `root/bin` comes first on the session's PATH, for executables a test provides.
struct HerdrSession {
    child: Child,
    root: PathBuf,
    work: PathBuf,
    name: String,
    socket: PathBuf,
}

impl HerdrSession {
    fn start(tag: &str) -> Self {
        let pid = std::process::id();
        // sun_path is 104 bytes on macOS and herdr nests its socket four levels deep.
        let root = PathBuf::from(format!("/tmp/c{tag}-{pid}"));
        let _ = std::fs::remove_dir_all(&root);
        let work = root.join("work");
        for dir in [&root, &root.join("home"), &root.join("bin"), &work] {
            std::fs::DirBuilder::new().mode(0o700).create(dir).unwrap();
        }
        let name = format!("collied-{tag}-{pid}");
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
            work,
            name,
            socket,
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while !pings(&session.socket) {
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
    let mut path = std::ffi::OsString::from(root.join("bin"));
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    let mut cmd = Command::new("herdr");
    cmd.env_clear()
        .env("PATH", path)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root)
        .env("XDG_STATE_HOME", root.join("state"));
    cmd
}

fn pings(socket: &Path) -> bool {
    let Ok(mut conn) = StdUnixStream::connect(socket) else {
        return false;
    };
    let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
    if conn
        .write_all(b"{\"id\":\"live\",\"method\":\"ping\",\"params\":{}}\n")
        .is_err()
    {
        return false;
    }
    let mut line = String::new();
    BufReader::new(conn).read_line(&mut line).is_ok() && line.contains("\"pong\"")
}
