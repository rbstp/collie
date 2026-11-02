#[allow(dead_code)]
mod common;

use std::io::Read;
use std::path::Path;
use std::process::Child;
use std::sync::Arc;
use std::time::Duration;

use collie_core::{AgentState, CollieCore, MachineFlock, TerminalSource};
use collied::control::{Client, Reply, Request};
use collied::drive::{Authorized, Driver, Watched};
use collied::runtime::herdr_core::{Core, GROUP_LABEL};
use collied::runtime::{AgentInfo, Runtime, herdr};
use collied::server::{self, ServerConfig};
use common::*;
use protocol::{
    ErrorCode, PaneCloseParams, ReadParams, ReadSource, Response, TerminalId, WorkspaceCloseParams,
    WorkspaceId,
};
use tokio::sync::watch;

// A stand-in agent that reports OSC 7501 as Claude Code does, and blocks once the test
// creates `block` in its folder.
const FAKE_AGENT: &str = r#"printf '\033]0;\342\240\213 Fix the flaky test\007'
printf '\033[1mready\033[0m\n'
printf '\033]7501;state=working:app=claude-code\033\\'
while [ ! -e block ]; do sleep 0.1; done
printf '\033]7501;state=blocked:app=claude-code:kind=permission:msg=approve Bash\033\\'
exec sleep 600"#;

// Draws once a desktop client sized its terminal: styled text, then Claude Code's input box
// with its dim placeholder, and a line longer than the terminal once the test creates `more`.
const SCREEN_AGENT: &str = r#"printf '\033]7501;state=idle:app=claude-code\033\\'
while [ ! -e draw ]; do sleep 0.1; done
cols=$(stty size | cut -d' ' -f2)
rule() { i=0; while [ $i -lt $cols ]; do printf '\342\224\200'; i=$((i+1)); done; printf '\n'; }
printf '\033[1mbold\033[0m and \033[38;2;215;119;87mcolored\033[0m\n\n'
rule
printf '\342\235\257 \033[2mTry "fix lint errors"\033[0m\n'
rule
while [ ! -e more ]; do sleep 0.1; done
i=0; while [ $i -lt 150 ]; do printf x; i=$((i+1)); done; printf '\n'
exec sleep 600"#;

#[test]
fn live_herdr_core() {
    if !hc_installed() {
        return;
    }
    let mut herdr = herdr_installed().then(|| HerdrSession::start("hcb"));
    let mut core = CoreSession::start("hc");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(scenario(&core, herdr.as_ref()));
    drop(rt);
    assert!(core.stop(), "dedicated herdr-core did not stop");
    if let Some(herdr) = herdr.as_mut() {
        assert!(herdr.stop(), "dedicated herdr session did not stop");
    }
}

async fn until<T>(what: &str, mut ready: impl AsyncFnMut() -> Option<T>) -> T {
    for _ in 0..100 {
        if let Some(v) = ready().await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

fn run(core: &CoreSession, name: &str, work: &Path, script: &str) -> String {
    let started = core.hc(&[
        "run",
        "--bg",
        "--json",
        "--name",
        name,
        "--cwd",
        work.to_str().unwrap(),
        "/bin/sh",
        "-c",
        script,
    ]);
    started["address"]
        .as_str()
        .unwrap_or_else(|| panic!("hc run: {started}"))
        .to_owned()
}

async fn scenario(core: &CoreSession, herdr: Option<&HerdrSession>) {
    let work = std::fs::canonicalize(&core.work).unwrap();
    let herdr_socket = herdr.map_or_else(|| core.root.join("no-herdr.sock"), |h| h.socket.clone());
    let mut herdr_workspaces = Vec::new();
    if let Some(h) = herdr {
        let cwd = std::fs::canonicalize(&h.work).unwrap();
        herdr::workspace_create(&h.socket, cwd.to_str().unwrap(), Some("collie-live"))
            .await
            .unwrap();
        herdr_workspaces = herdr::workspace_list(&h.socket).await.unwrap();
    }

    let agent = run(core, "collie-fake", &work, FAKE_AGENT);
    let shell = run(core, "collie-shell", &work, "exec sleep 600");

    let followed = Arc::new(Core::new(core.socket.clone()));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(followed.clone().run(stopped));
    tokio::time::timeout(Duration::from_secs(5), followed.settled())
        .await
        .unwrap();
    let rt = Runtime::new(herdr_socket.clone()).with_core(Some(followed.clone()));
    let find = async |status: &str| -> Option<AgentInfo> {
        rt.agent_list()
            .await
            .ok()?
            .into_iter()
            .find(|a| a.terminal_id.ends_with(&format!(":{agent}")) && a.agent_status == status)
    };

    let working = until("a working agent", async || find("working").await).await;
    let group = working.workspace_id.clone();
    assert!(group.starts_with("hc:") && group.len() == 35, "{group}");
    assert_eq!(working.terminal_id, format!("{group}:{agent}"));
    assert_eq!(working.pane_id, working.terminal_id);
    assert_eq!(working.agent.as_deref(), Some("claude"));
    assert_eq!(working.name.as_deref(), Some("collie-fake"));
    assert_eq!(working.title.as_deref(), Some("Fix the flaky test"));
    assert_eq!(working.cwd.as_deref(), work.to_str());
    TerminalId::new(working.terminal_id.clone()).unwrap();
    let listed = rt.agent_list().await.unwrap();
    assert!(
        !listed
            .iter()
            .any(|a| a.terminal_id.ends_with(&format!(":{shell}")))
    );
    assert_eq!(
        listed
            .iter()
            .filter(|a| a.terminal_id.starts_with("hc:"))
            .count(),
        1
    );

    let workspaces = rt.poll().await.unwrap().workspaces;
    let herdr_number = herdr_workspaces.iter().map(|w| w.number).max();
    let g = workspaces.iter().find(|w| w.workspace_id == group).unwrap();
    assert_eq!(
        (g.label.as_str(), g.number),
        (GROUP_LABEL, herdr_number.unwrap_or(0) + 1)
    );
    assert_eq!(workspaces.len(), herdr_workspaces.len() + 1);
    let snap = rt.session_snapshot().await.unwrap();
    assert!(
        snap.panes
            .iter()
            .any(|p| p.terminal_id == working.terminal_id)
    );
    assert_eq!(snap.workspaces.len(), workspaces.len());

    std::fs::write(work.join("block"), "").unwrap();
    let blocked = until("a blocked agent", async || find("blocked").await).await;
    assert!(blocked.state_change_seq > working.state_change_seq);

    let drive = Driver::new(herdr_socket, vec![], std::slice::from_ref(&work))
        .unwrap()
        .with_core(Some(followed.clone()));
    let yes: Authorized = Arc::new(|| true);
    let refused = drive
        .workspace_close(
            WorkspaceCloseParams {
                workspace_id: WorkspaceId::new(group.clone()).unwrap(),
                confirm: true,
            },
            &yes,
        )
        .await;
    assert_eq!(refused.unwrap_err().0, ErrorCode::Internal);
    for t in [&agent, &shell] {
        assert_eq!(core.hc(&["get", "--json", t])["state"], "live");
    }

    let closed = drive
        .pane_close(
            PaneCloseParams {
                terminal_id: TerminalId::new(blocked.terminal_id.clone()).unwrap(),
                confirm: true,
            },
            &yes,
        )
        .await;
    assert_eq!(closed, Ok(Response::Ok));
    assert!(core.hc(&["get", "--json", &agent]).is_null());
    assert_eq!(core.hc(&["get", "--json", &shell])["state"], "live");
    until("the closed agent to leave", async || {
        let agents = rt.agent_list().await.ok()?;
        (!agents.iter().any(|a| a.workspace_id == group)).then_some(())
    })
    .await;
    // Without herdr the followed core still answers, with nothing in it.
    assert_eq!(
        rt.poll().await.unwrap().workspaces.len(),
        herdr_workspaces.len()
    );

    stop.send_replace(true);
    task.await.unwrap();
}

/// The phone over the tailnet, on a machine with herdr-core and no herdr.
#[test]
fn live_herdr_core_phone() {
    if !hc_installed() || !in_child("live_herdr_core_phone") {
        return;
    }
    let mut hc = CoreSession::start("hcp");
    let root = TempDir::new("e2e-herdr-core");
    let net = Net::start(&root.0);
    wait_ready(&net.mac, 0);
    let phone = phone(&root.0, "phone", &net);
    let rt = runtime();
    rt.block_on(phone.node_start(Some(net.key.clone())))
        .unwrap();
    wait_ready(&net.mac, 1);
    wait_phone(&rt, &phone);
    rt.block_on(phone_scenario(&root.0, &net, &phone, &hc));
    drop(phone);
    drop(rt);
    assert!(hc.stop(), "dedicated herdr-core did not stop");
}

async fn flock_until(
    phone: &CollieCore,
    m: &str,
    what: &str,
    ok: impl Fn(&MachineFlock) -> bool,
) -> MachineFlock {
    for _ in 0..150 {
        let f = phone.flock(m.into()).await.unwrap();
        if ok(&f) {
            return f;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the phone never saw {what}");
}

async fn phone_scenario(root: &Path, net: &Net, phone: &Arc<CollieCore>, hc: &CoreSession) {
    let work = std::fs::canonicalize(&hc.work).unwrap();
    let agent = run(hc, "collie-fake", &work, FAKE_AGENT);
    let data_dir = root.join("collied");
    let handle = server::start(
        net.mac.clone(),
        ServerConfig {
            herdr_core: Some(hc.socket.clone()),
            ..server_config(&data_dir, "e2e")
        },
        root.join("no-herdr.sock"),
    )
    .await
    .unwrap();
    let (machine, _) = pair(&handle.control_path(), phone, LABEL).await;
    let m = machine.id.clone();

    let f = flock_until(phone, &m, "the herdr-core agent", |f| {
        f.agents.iter().any(|a| a.status == AgentState::Working)
    })
    .await;
    let a = &f.agents[0];
    let group = f
        .workspaces
        .iter()
        .find(|w| w.workspace_id == a.workspace_id)
        .unwrap();
    assert_eq!((group.label.as_str(), group.number), (GROUP_LABEL, 1));
    assert_eq!(group.status, AgentState::Working);
    assert_eq!(a.terminal_id, format!("{}:{agent}", group.workspace_id));
    assert_eq!(a.kind.as_deref(), Some("claude"));
    assert_eq!(a.title.as_deref(), Some("Fix the flaky test"));
    let (terminal_id, workspace_id) = (a.terminal_id.clone(), a.workspace_id.clone());

    let read = phone
        .agent_read(m.clone(), terminal_id.clone(), TerminalSource::Recent, None)
        .await
        .unwrap();
    assert_eq!(read.ansi, "\u{1b}[0m\u{1b}[1mready\u{1b}[0m");
    phone
        .watch_agent(m.clone(), Some(terminal_id.clone()), 50)
        .await
        .unwrap();
    for _ in 0..100 {
        let view = phone.agent_view(m.clone(), terminal_id.clone(), 0);
        if view
            .and_then(|v| v.output)
            .is_some_and(|o| o.ansi == read.ansi)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let view = phone.agent_view(m.clone(), terminal_id.clone(), 0).unwrap();
    assert_eq!(
        view.output.unwrap().ansi,
        read.ansi,
        "the watch sends output"
    );
    phone.watch_agent(m.clone(), None, 50).await.unwrap();

    let mut cli = Client::connect(&handle.control_path()).await.unwrap();
    let Reply::Status(status) = cli.call(&Request::Status).await.unwrap() else {
        panic!("no status");
    };
    let core = status.herdr_core.unwrap();
    assert_eq!(core.socket, hc.socket.display().to_string());
    assert!(core.version.is_some() && core.missing.is_empty());
    assert_eq!((core.generation, core.folder_mode), (Some(1), Some(0o700)));
    assert_eq!(status.herdr_version, None);
    assert!(
        status
            .flock
            .unwrap()
            .agents
            .iter()
            .any(|a| a.terminal_id.as_str() == terminal_id)
    );

    std::fs::write(work.join("block"), "").unwrap();
    flock_until(phone, &m, "the agent blocked", |f| {
        f.agents.iter().any(|a| a.status == AgentState::Blocked)
    })
    .await;

    let refused = phone.close_workspace(m.clone(), workspace_id, true).await;
    assert!(
        refused.is_err_and(|e| e.to_string().contains("cannot be closed")),
        "the group is not closed"
    );
    assert_eq!(hc.hc(&["get", "--json", &agent])["state"], "live");
    phone
        .close_pane(m.clone(), terminal_id, true)
        .await
        .unwrap();
    assert!(hc.hc(&["get", "--json", &agent]).is_null());
    flock_until(phone, &m, "the closed agent leave", |f| {
        f.agents.is_empty() && f.workspaces.is_empty()
    })
    .await;
    handle.shutdown().await;
}

/// `hc attach` on a pty of the given size: a desktop client that sizes the terminal.
struct Desktop(Child);

impl Desktop {
    fn attach(core: &CoreSession, terminal: &str, cols: u16, rows: u16) -> Self {
        use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let name = ptsname(&master, Vec::new()).unwrap();
        let tty = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(name.to_str().unwrap())
            .unwrap();
        let size = rustix::termios::Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        rustix::termios::tcsetwinsize(&tty, size).unwrap();
        let child = core
            .command(&["attach", terminal])
            .env("TERM", "xterm-256color")
            .stdin(tty.try_clone().unwrap())
            .stdout(tty.try_clone().unwrap())
            .stderr(tty)
            .spawn()
            .unwrap();
        let mut screen = std::fs::File::from(master);
        std::thread::spawn(move || {
            let mut buf = [0; 4096];
            while screen.read(&mut buf).is_ok_and(|n| n > 0) {}
        });
        Self(child)
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn size(core: &CoreSession, terminal: &str) -> (u64, u64) {
    let t = core.hc(&["get", "--json", terminal]);
    (t["cols"].as_u64().unwrap(), t["rows"].as_u64().unwrap())
}

/// Rows without SGR or trailing blanks, and no blank rows at the bottom.
fn plain(ansi: &str) -> String {
    let mut rows: Vec<String> = ansi
        .split('\n')
        .map(|row| {
            let mut out = String::new();
            let mut rest = row.strip_suffix('\r').unwrap_or(row);
            while let Some(at) = rest.find("\u{1b}[") {
                out.push_str(&rest[..at]);
                rest = &rest[at..];
                rest = &rest[rest.find('m').unwrap() + 1..];
            }
            out.push_str(rest);
            out.trim_end().to_owned()
        })
        .collect();
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows.join("\n")
}

#[test]
fn live_herdr_core_screen() {
    if !hc_installed() {
        return;
    }
    let mut core = CoreSession::start("hcs");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(screen_scenario(&core));
    drop(rt);
    assert!(core.stop(), "dedicated herdr-core did not stop");
}

async fn screen_scenario(core: &CoreSession) {
    let work = std::fs::canonicalize(&core.work).unwrap();
    let agent = run(core, "collie-screen", &work, SCREEN_AGENT);
    let desktop = Desktop::attach(core, &agent, 70, 20);
    until("the desktop client's size", async || {
        (size(core, &agent) == (70, 20)).then_some(())
    })
    .await;
    std::fs::write(work.join("draw"), "").unwrap();

    let followed = Arc::new(Core::new(core.socket.clone()));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(followed.clone().run(stopped));
    let drive = Arc::new(
        Driver::new(
            core.root.join("no-herdr.sock"),
            vec![],
            std::slice::from_ref(&work),
        )
        .unwrap()
        .with_core(Some(followed.clone())),
    );
    let rt = Runtime::new(core.root.join("no-herdr.sock")).with_core(Some(followed.clone()));
    let id = until("the agent", async || {
        rt.agent_list()
            .await
            .ok()?
            .into_iter()
            .find(|a| a.terminal_id.ends_with(&format!(":{agent}")))
    })
    .await
    .terminal_id;
    let tid = TerminalId::new(id.clone()).unwrap();
    let read = async |source: ReadSource| match drive
        .read(
            ReadParams {
                terminal_id: tid.clone(),
                source,
                lines: Some(50),
            },
            true,
        )
        .await
    {
        Ok(Response::Terminal(t)) => t.ansi,
        other => panic!("read: {other:?}"),
    };

    let styled =
        "\u{1b}[0m\u{1b}[1mbold\u{1b}[0m and \u{1b}[0m\u{1b}[38;2;215;119;87mcolored\u{1b}[0m";
    let visible = until("the drawn screen", async || {
        let ansi = read(ReadSource::Visible).await;
        ansi.contains('\u{276f}').then_some(ansi)
    })
    .await;
    assert!(visible.starts_with(&format!("{styled}\r\n")), "{visible:?}");
    assert!(plain(&visible).lines().all(|row| row.chars().count() <= 70));
    let text = core.hc(&["read", "--json", &agent])["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(plain(&visible), plain(&text));
    assert_eq!(
        drive.draft(&tid).await,
        Ok(Response::Draft {
            text: Some(String::new())
        })
    );
    assert_eq!(size(core, &agent), (70, 20));

    let mut watcher = drive.watch(tid.clone(), 50, false).await.unwrap();
    let Some(Watched::Output(first)) = watcher.recv().await else {
        panic!("no first output");
    };
    assert!(first.ansi.starts_with(styled), "{:?}", first.ansi);
    tokio::time::sleep(Duration::from_millis(500)).await;
    std::fs::write(work.join("more"), "").unwrap();
    let Some(Watched::Patch(_) | Watched::Output(_)) = watcher.recv().await else {
        panic!("no output after the change");
    };
    // Wrapped while the watch held the terminal, the long line reads back whole.
    let recent = read(ReadSource::Recent).await;
    assert!(
        recent.contains(&format!("\r\n{}", "x".repeat(150))),
        "{recent:?}"
    );
    assert_eq!(size(core, &agent), (70, 20));
    drop(watcher);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(size(core, &agent), (70, 20));
    drop(desktop);

    stop.send_replace(true);
    task.await.unwrap();
}
