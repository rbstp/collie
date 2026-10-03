use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use collied::drive::{Authorized, Driver};
use collied::herdr;
use protocol::{
    AgentKind, AgentPromptParams, Cwd, ErrorCode, Label, OpId, PaneCloseParams, PromptText,
    ReadParams, ReadSource, Response, TaskNewParams, TerminalId, WorkspaceCloseParams, WorkspaceId,
};

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
    let mut session = HerdrSession::start();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(scenario(&session));
    drop(rt);
    assert!(session.stop(), "dedicated herdr session did not stop");
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
    assert_eq!(
        drive.read(read, true).await.unwrap_err().0,
        ErrorCode::NotFound
    );
    let prompt = AgentPromptParams {
        op_id: OpId::new("P".repeat(22)).unwrap(),
        terminal_id: terminal.clone(),
        text: PromptText::new("echo should-not-run").unwrap(),
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
struct HerdrSession {
    child: Child,
    root: PathBuf,
    work: PathBuf,
    name: String,
    socket: PathBuf,
}

impl HerdrSession {
    fn start() -> Self {
        let pid = std::process::id();
        // sun_path is 104 bytes on macOS and herdr nests its socket four levels deep.
        let root = PathBuf::from(format!("/tmp/cdrv-{pid}"));
        let _ = std::fs::remove_dir_all(&root);
        let work = root.join("work");
        for dir in [&root, &root.join("home"), &work] {
            std::fs::DirBuilder::new().mode(0o700).create(dir).unwrap();
        }
        let name = format!("collied-drive-{pid}");
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
    let mut cmd = Command::new("herdr");
    cmd.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
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
