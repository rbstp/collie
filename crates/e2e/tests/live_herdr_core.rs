#[allow(dead_code)]
mod common;

use std::io::Read;
use std::path::Path;
use std::process::Child;
use std::sync::Arc;
use std::time::Duration;

use collie_core::{AgentState, CollieCore, MachineFlock, TerminalSource};
use collied::approvals::Approvals;
use collied::audit::Audit;
use collied::config::TasksConfig;
use collied::control::{Client, Reply, Request};
use collied::drive::{Authorized, Driver, Watched};
use collied::runtime::herdr_core::{Core, GROUP_LABEL};
use collied::runtime::{AgentInfo, Runtime, herdr};
use collied::server::{self, ServerConfig};
use common::*;
use protocol::{
    AgentPromptParams, AgentSendKeysParams, AgentTypeTextParams, ApprovalDecideParams,
    ApprovalOutcome, Decision, ErrorCode, Event, Key, OpId, PaneCloseParams, PromptText,
    ReadParams, ReadSource, Response, TerminalId, WorkspaceCloseParams, WorkspaceId,
};
use tokio::sync::{broadcast, watch};

/// The session in the Claude Code transcript fixture.
const CLAUDE_SESSION: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
const CODEX_SESSION: &str = "01a10f07-b5ed-79a3-989d-fa10b96c5cab";
const CLOSED_SESSION: &str = "0a0a0a0a-0000-4000-8000-000000000001";

/// A stand-in Claude Code: it writes its `sessions/<pid>.json` in `sessions`, reports OSC
/// 7501 as Claude Code does, and blocks once the test creates `block` in its folder.
fn fake_claude(sessions: &Path) -> String {
    let sessions = sessions.display();
    format!(
        r#"mkdir -p '{sessions}'
printf '{{"pid":%d,"sessionId":"{CLAUDE_SESSION}","cwd":"%s","startedAt":%s000}}' $$ "$PWD" "$(date +%s)" > '{sessions}'/$$.json
printf '\033]0;\342\240\213 Fix the flaky test\007'
printf '\033[1mready\033[0m\n'
printf '\033]7501;state=working:app=claude-code\033\\'
while [ ! -e block ]; do sleep 0.1; done
printf '\033]7501;state=blocked:app=claude-code:kind=permission:msg=approve Bash\033\\'
exec sleep 600"#
    )
}

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

/// Under a name that is not a shell's: collied lists no terminal a shell leads.
fn run(core: &CoreSession, name: &str, work: &Path, script: &str) -> String {
    let exec = r#"exec -a collie-fake /bin/bash -c "$1""#;
    run_argv(core, name, work, &["/bin/bash", "-c", exec, "fake", script])
}

fn run_argv(core: &CoreSession, name: &str, work: &Path, argv: &[&str]) -> String {
    let mut args = vec!["run", "--bg", "--json", "--name", name, "--cwd"];
    args.push(work.to_str().unwrap());
    args.extend(argv);
    let started = core.hc(&args);
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

    let sessions = core.root.join("claude/sessions");
    let agent = run(core, "collie-fake", &work, &fake_claude(&sessions));
    let shell = run(core, "collie-shell", &work, "exec sleep 600");

    let followed =
        Arc::new(Core::new(core.socket.clone()).with_claude_sessions(Some(sessions.clone())));
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
    // The foreground process comes with an event of its own, after the status.
    let session = until("the Claude Code session", async || {
        find("working").await?.agent_session.map(|s| s.value)
    })
    .await;
    assert_eq!(session, CLAUDE_SESSION);
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

/// The phone over the tailnet, on a machine with herdr-core and no herdr. collied reads
/// Claude Code's and Codex's files from scratch folders, never the user's.
#[test]
fn live_herdr_core_phone() {
    if !hc_installed() {
        return;
    }
    let agents = std::env::var("COLLIE_E2E_AGENTS")
        .unwrap_or_else(|_| format!("/tmp/hca-{}", std::process::id()));
    let claude = format!("{agents}/claude");
    let codex = format!("{agents}/codex");
    let env = [
        ("COLLIE_E2E_AGENTS", agents.as_str()),
        ("CLAUDE_CONFIG_DIR", claude.as_str()),
        ("CODEX_HOME", codex.as_str()),
    ];
    if !in_child_with("live_herdr_core_phone", &env) {
        let _ = std::fs::remove_dir_all(&agents);
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
    rt.block_on(phone_scenario(
        &root.0,
        &net,
        &phone,
        &hc,
        Path::new(&agents),
    ));
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

/// Transcripts for the fake agents' sessions and one closed Claude Code session, in `work`.
fn transcripts(agents: &Path, work: &Path) {
    let folder = work.to_str().unwrap();
    let project: String = folder
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let project = agents.join("claude/projects").join(project);
    std::fs::create_dir_all(&project).unwrap();
    let claude = include_str!("../../collied/tests/fixtures/transcripts/claude.jsonl");
    std::fs::write(project.join(format!("{CLAUDE_SESSION}.jsonl")), claude).unwrap();
    std::fs::write(
        project.join(format!("{CLOSED_SESSION}.jsonl")),
        claude.replace(CLAUDE_SESSION, CLOSED_SESSION),
    )
    .unwrap();
    let day = agents.join("codex/sessions/2026/10/05");
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(
        day.join(format!("rollout-2026-10-05T22-25-26-{CODEX_SESSION}.jsonl")),
        include_str!("../../collied/tests/fixtures/transcripts/codex.jsonl")
            .replace("/Users/me/src/collie", folder),
    )
    .unwrap();
}

async fn phone_scenario(
    root: &Path,
    net: &Net,
    phone: &Arc<CollieCore>,
    hc: &CoreSession,
    agents: &Path,
) {
    let work = std::fs::canonicalize(&hc.work).unwrap();
    transcripts(agents, &work);
    let agent = run(
        hc,
        "collie-fake",
        &work,
        &fake_claude(&agents.join("claude/sessions")),
    );
    let data_dir = root.join("collied");
    let handle = server::start_with(
        net.mac.clone(),
        ServerConfig {
            herdr_core: Some(hc.socket.clone()),
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
    // A stand-in Codex: this test binary, run as `codex` (`fake_codex`).
    let codex = hc.root.join("bin/codex");
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::copy(std::env::current_exe().unwrap(), &codex).unwrap();
    std::fs::write(
        work.join("control"),
        handle.control_path().to_str().unwrap(),
    )
    .unwrap();
    let codex = run_argv(
        hc,
        "collie-codex",
        &work,
        &[
            codex.to_str().unwrap(),
            "fake_codex",
            "--exact",
            "--nocapture",
        ],
    );
    let (machine, _) = pair(&handle.control_path(), phone, LABEL).await;
    let m = machine.id.clone();

    let f = flock_until(phone, &m, "the herdr-core agents", |f| {
        f.agents.iter().any(|a| a.status == AgentState::Working) && f.agents.len() == 2
    })
    .await;
    let a = f
        .agents
        .iter()
        .find(|a| a.kind.as_deref() == Some("claude"))
        .unwrap();
    let codex_id = f
        .agents
        .iter()
        .find(|a| a.terminal_id.ends_with(&format!(":{codex}")))
        .unwrap()
        .terminal_id
        .clone();
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
    let chat = phone
        .agent_history(m.clone(), terminal_id.clone(), None, None, None)
        .await
        .unwrap();
    assert!(chat.available, "{:?}", chat.unavailable_reason);
    assert!(
        chat.messages
            .iter()
            .any(|msg| msg.text.contains("fix the flaky approval test"))
    );
    std::fs::write(work.join("report"), "").unwrap();
    let mut codex_chat = None;
    for _ in 0..50 {
        let page = phone
            .agent_history(m.clone(), codex_id.clone(), None, None, None)
            .await
            .unwrap();
        if page.available {
            codex_chat = Some(page);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        codex_chat.is_some_and(|p| !p.messages.is_empty()),
        "Codex's reported session"
    );
    let sessions = phone
        .task_sessions(m.clone(), work.to_str().unwrap().to_owned())
        .await
        .unwrap();
    let listed: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
    assert_eq!(listed, [CLOSED_SESSION], "open sessions are left out");

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
    phone.close_pane(m.clone(), codex_id, true).await.unwrap();
    flock_until(phone, &m, "the closed agents leave", |f| {
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

/// Codex, as the phone scenario runs it in its herdr-core terminal: it reports OSC 7501, then
/// its session over the control socket as `collied hook` does, once the test creates `report`.
/// A process named `codex` that leads its terminal, which a copied shell cannot be on macOS.
#[test]
fn fake_codex() {
    let exe = std::env::current_exe().unwrap();
    if exe.file_name().and_then(|n| n.to_str()) != Some("codex") {
        return;
    }
    use std::io::{BufRead, Write};
    print!("\x1b]7501;state=idle:app=codex\x1b\\");
    std::io::stdout().flush().unwrap();
    while !Path::new("report").exists() {
        std::thread::sleep(Duration::from_millis(100));
    }
    let control = std::fs::read_to_string("control").unwrap();
    let mut conn = std::os::unix::net::UnixStream::connect(control).unwrap();
    let report = serde_json::json!({
        "cmd": "session",
        "terminal_id": std::env::var("HC_TERMINAL_ID").unwrap(),
        "session_id": CODEX_SESSION,
    });
    writeln!(conn, "{report}").unwrap();
    std::io::BufReader::new(conn)
        .read_line(&mut String::new())
        .unwrap();
    std::thread::sleep(Duration::from_secs(600));
}

// A stand-in Claude Code for input: it reports `done`, `error` and `idle` in turn, each with
// an empty input box, and prints each prompt it reads. Then two Bash permission prompts:
// the first is drawn again once the test creates `redraw`, and answered with a line; the
// second shows once the test creates `next`, and is answered with one key, read raw. Then
// two prompts that take a note, read raw a byte at a time: ⇥ opens the amend field, typing
// fills it, Enter or Esc ends it. The second is replaced by another dialog after a second
// without input once the note reads `ok`, as when the dialog is answered on the machine and
// a new one opens.
const INPUT_AGENT: &str = r#"st() { printf '\033]7501;state=%s:app=claude-code%s\033\\' "$1" "$2"; }
cols=$(stty size | cut -d' ' -f2)
rule() { i=0; while [ $i -lt $cols ]; do printf '\342\224\200'; i=$((i+1)); done; printf '\n'; }
box() { rule; printf '\342\235\257 \033[2mTry "fix lint errors"\033[0m\n'; rule; }
prompt() { IFS= read -r p; st working; printf '\033[2J\033[Hgot %s\n' "$p"; }
dialog() { printf '\033[2J\033[H'; cat "$1"; st blocked ':kind=permission:msg=approve Bash'; }
st done; box
prompt; st error; box
prompt; st idle; box
prompt
dialog dialog1
( while [ ! -e redraw ]; do sleep 0.1; done; dialog dialog1; touch redrawn ) &
IFS= read -r answer
printf '\033[2J\033[Happroved\n'; st idle
while [ ! -e next ]; do sleep 0.1; done
dialog dialog2
stty raw -echo; k=$(dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); stty sane
printf '\033[2J\033[Hdenied %s\n' "$k"; st idle
draw() { printf '\033[2J\033[H'; sed "s/1\. Yes\$/1. $2/" "$1"; }
note() {
  draw "$1" Yes; st blocked ':kind=permission:msg=approve Bash'
  stty raw -echo min 0 time 10
  n=; swapped=
  while :; do
    b=$(dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n')
    case "$b" in
      09) draw "$1" 'Yes, and tell Claude what to do next' ;;
      0d) break ;;
      1b) n=esc; break ;;
      '') if [ -n "$2" ] && [ "$n" = ok ] && [ -z "$swapped" ]; then swapped=1; draw "$2" Yes; fi ;;
      *) n="$n$(printf "\\$(printf %o 0x$b)")"; draw "$1" "Yes, $n" ;;
    esac
  done
  stty sane
  printf '\033[2J\033[Hamended %s\n' "$n"; st idle
}
while [ ! -e note1 ]; do sleep 0.1; done
note dialog3
while [ ! -e note2 ]; do sleep 0.1; done
note dialog4 dialog5
exec sleep 600"#;

const DIALOG: &str = "\
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

// Each byte it reads, as hex on a line of its own in `keys`.
const KEY_DUMP: &str = r#"printf '\033]7501;state=idle:app=keydump\033\\'
stty raw -echo
while :; do dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n' >> keys; echo >> keys; done"#;

/// What each key reaches a program with no keyboard modes set as, from herdr-core 0.1.16.
fn key_bytes(key: Key) -> &'static str {
    match key {
        Key::Enter => "0d",
        Key::Esc => "1b",
        Key::Up => "1b5b41",
        Key::Down => "1b5b42",
        Key::Left => "1b5b44",
        Key::ShiftLeft => "1b5b313b3244",
        Key::Right => "1b5b43",
        Key::Tab => "09",
        Key::ShiftTab => "1b5b5a",
        Key::Backspace => "7f",
        Key::CtrlC => "03",
        Key::CtrlEnter => "1b5b32373b353b31337e",
        Key::CtrlA => "01",
        Key::CtrlB => "02",
        Key::CtrlE => "05",
        Key::CtrlK => "0b",
        Key::CtrlL => "0c",
        Key::CtrlU => "15",
        Key::CtrlW => "17",
    }
}

const KEYS: [Key; 19] = [
    Key::Enter,
    Key::Esc,
    Key::Up,
    Key::Down,
    Key::Left,
    Key::ShiftLeft,
    Key::Right,
    Key::Tab,
    Key::ShiftTab,
    Key::Backspace,
    Key::CtrlC,
    Key::CtrlEnter,
    Key::CtrlA,
    Key::CtrlB,
    Key::CtrlE,
    Key::CtrlK,
    Key::CtrlL,
    Key::CtrlU,
    Key::CtrlW,
];

#[test]
fn live_herdr_core_input() {
    if !hc_installed() {
        return;
    }
    let mut core = CoreSession::start("hci");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(input_scenario(&core));
    drop(rt);
    assert!(core.stop(), "dedicated herdr-core did not stop");
}

async fn input_scenario(core: &CoreSession) {
    let work = std::fs::canonicalize(&core.work).unwrap();
    std::fs::write(work.join("dialog1"), DIALOG).unwrap();
    std::fs::write(
        work.join("dialog2"),
        DIALOG
            .replace("build", "dist")
            .replace("rm commands", "rm -rf"),
    )
    .unwrap();
    for (name, dir) in [
        ("dialog3", "cache"),
        ("dialog4", "logs"),
        ("dialog5", "home"),
    ] {
        std::fs::write(work.join(name), DIALOG.replace("build", dir)).unwrap();
    }
    let claude = run(core, "collie-input", &work, INPUT_AGENT);
    let copilot = run_argv(
        core,
        "collie-copilot",
        &work,
        &["/bin/bash", "-c", "exec -a copilot sleep 600"],
    );
    let dump = run(core, "collie-keys", &work, KEY_DUMP);

    let followed = Arc::new(Core::new(core.socket.clone()));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(followed.clone().run(stopped));
    let none = core.root.join("no-herdr.sock");
    let rt = Runtime::new(none.clone()).with_core(Some(followed.clone()));
    let drive = Driver::new(none.clone(), vec![], std::slice::from_ref(&work))
        .unwrap()
        .with_core(Some(followed.clone()));
    let yes: Authorized = Arc::new(|| true);
    let find = async |address: &str| -> Option<AgentInfo> {
        rt.agent_list()
            .await
            .ok()?
            .into_iter()
            .find(|a| a.terminal_id.ends_with(&format!(":{address}")))
    };
    let status = async |address: &str, status: &str| {
        until(status, async || {
            find(address).await.filter(|a| a.agent_status == status)
        })
        .await
    };
    let shows = async |address: &str, text: &str| {
        until(text, async || {
            let screen = core.hc(&["read", "--json", address])["text"]
                .as_str()?
                .to_owned();
            screen.contains(text).then_some(())
        })
        .await;
    };
    let op = |c: char| OpId::new(c.to_string().repeat(22)).unwrap();
    let prompt = |id: &TerminalId, text: &str| AgentPromptParams {
        op_id: op('P'),
        terminal_id: id.clone(),
        text: PromptText::new(text.to_owned()).unwrap(),
        expected_draft: None,
    };
    let keys = |id: &TerminalId, keys: Vec<Key>| AgentSendKeysParams {
        op_id: op('K'),
        terminal_id: id.clone(),
        keys,
    };

    // No OSC 7501 report: read-only.
    let c = until("the Copilot stand-in", async || find(&copilot).await).await;
    assert_eq!(
        (c.agent.as_deref(), c.agent_status.as_str(), c.read_only),
        (Some("copilot"), "unknown", true)
    );
    let cid = TerminalId::new(c.terminal_id).unwrap();
    let refused = |r: Result<Response, (ErrorCode, String)>| r.unwrap_err().0;
    assert_eq!(
        refused(drive.prompt(prompt(&cid, "hi"), &yes).await),
        ErrorCode::AgentNotReady
    );
    assert_eq!(
        refused(drive.send_keys(keys(&cid, vec![Key::Enter]), &yes).await.0),
        ErrorCode::AgentNotReady
    );
    let typed = AgentTypeTextParams {
        op_id: op('T'),
        terminal_id: cid.clone(),
        text: PromptText::new("yes".to_owned()).unwrap(),
    };
    assert_eq!(
        refused(drive.type_text(typed, &yes).await),
        ErrorCode::AgentNotReady
    );

    // Prompts reach a done, an error and an idle agent.
    let a = status(&claude, "done").await;
    let id = TerminalId::new(a.terminal_id.clone()).unwrap();
    assert_eq!(
        drive.draft(&id).await,
        Ok(Response::Draft {
            text: Some(String::new())
        })
    );
    assert_eq!(
        drive.prompt(prompt(&id, "one"), &yes).await,
        Ok(Response::Ok)
    );
    shows(&claude, "got one").await;
    status(&claude, "unknown").await;
    assert_eq!(
        drive.prompt(prompt(&id, "two"), &yes).await,
        Ok(Response::Ok)
    );
    shows(&claude, "got two").await;
    status(&claude, "idle").await;
    assert_eq!(
        drive.prompt(prompt(&id, "three"), &yes).await,
        Ok(Response::Ok)
    );
    // The fake reads it, then shows its first dialog.

    // One dialog keeps its approval through a redraw; Approve answers it, Deny the next.
    let audit = Arc::new(Audit::open(&core.root.join("audit.log")).unwrap());
    let (tx, mut events) = broadcast::channel(16);
    let approvals =
        Approvals::new(none, "nMAC".into(), tx, audit, None).with_core(Some(followed.clone()));
    let raise = async |events: &mut broadcast::Receiver<Event>, after: u64| {
        let blocked = until("a new dialog", async || {
            find(&claude)
                .await
                .filter(|a| a.agent_status == "blocked" && a.state_change_seq > after)
        })
        .await;
        let workspaces = rt.poll().await.unwrap().workspaces;
        approvals
            .observe(&rt.agent_list().await.unwrap(), &workspaces)
            .await;
        let approval = std::iter::from_fn(|| events.try_recv().ok())
            .find_map(|e| match e {
                Event::ApprovalNeeded { approval } => Some(approval),
                _ => None,
            })
            .expect("no approval for the blocked agent");
        assert_eq!(
            approval.options,
            [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
        );
        (blocked.state_change_seq, approval)
    };
    let (seq, approval) = raise(&mut events, a.state_change_seq).await;
    assert!(
        approval.snippet.contains("rm -rf build"),
        "{}",
        approval.snippet
    );
    std::fs::write(work.join("redraw"), "").unwrap();
    until("the redrawn dialog", async || {
        work.join("redrawn").exists().then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let agents = rt.agent_list().await.unwrap();
    approvals
        .observe(&agents, &rt.poll().await.unwrap().workspaces)
        .await;
    assert!(events.try_recv().is_err(), "the redraw raised nothing");
    let pending = approvals.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].approval_id, approval.approval_id);
    let now = agents
        .iter()
        .find(|x| x.terminal_id == a.terminal_id)
        .unwrap();
    assert_eq!(now.state_change_seq, seq);
    let decide = |approval: &protocol::Approval, decision| ApprovalDecideParams {
        approval_id: approval.approval_id.clone(),
        decision,
        choice: None,
        nonce: approval.nonce.clone(),
        note: None,
    };
    let applied = |approval: &protocol::Approval, decision| {
        Ok(Response::ApprovalResolved {
            approval_id: approval.approval_id.clone(),
            outcome: ApprovalOutcome::Applied {
                decision,
                by: "live".into(),
            },
        })
    };
    assert_eq!(
        approvals
            .decide("live", "nLIVE", decide(&approval, Decision::Approve), &yes)
            .await,
        applied(&approval, Decision::Approve)
    );
    shows(&claude, "approved").await;
    std::fs::write(work.join("next"), "").unwrap();
    let (next_seq, next) = raise(&mut events, seq).await;
    assert!(next_seq > seq);
    assert_ne!(next.approval_id, approval.approval_id);
    assert!(next.snippet.contains("rm -rf dist"), "{}", next.snippet);
    assert_eq!(
        approvals
            .decide("live", "nLIVE", decide(&next, Decision::Deny), &yes)
            .await,
        applied(&next, Decision::Deny)
    );
    shows(&claude, "denied 1b").await;

    // A note goes with Approve; Enter waits out the core's hold on keys after the note's own
    // typing, then reads again, so it never lands in the dialog that replaced the noted one.
    let noted = |approval: &protocol::Approval| ApprovalDecideParams {
        note: Some(PromptText::new("ok".to_owned()).unwrap()),
        ..decide(approval, Decision::Approve)
    };
    std::fs::write(work.join("note1"), "").unwrap();
    let (note_seq, first) = raise(&mut events, next_seq).await;
    assert!(first.supports_note);
    assert_eq!(
        approvals.decide("live", "nLIVE", noted(&first), &yes).await,
        applied(&first, Decision::Approve)
    );
    shows(&claude, "amended ok").await;
    std::fs::write(work.join("note2"), "").unwrap();
    let (_, replaced) = raise(&mut events, note_seq).await;
    assert_eq!(
        approvals
            .decide("live", "nLIVE", noted(&replaced), &yes)
            .await,
        Ok(Response::ApprovalResolved {
            approval_id: replaced.approval_id.clone(),
            outcome: ApprovalOutcome::Superseded,
        })
    );
    rt.agent_send_keys(&a.pane_id, &["esc"]).await.unwrap();
    shows(&claude, "amended esc").await;

    // herdr-core keeps a program's status while it is suspended and after it exits, with
    // the shell leading the terminal; collied lists no terminal a shell leads.
    let shell = run_argv(core, "collie-shell", &work, &["/bin/sh", "-i"]);
    std::fs::write(
        work.join("reporter"),
        "printf '\\033]7501;state=idle:app=codex\\033\\\\'\nwhile [ ! -e leave ]; do sleep 0.1; done\n",
    )
    .unwrap();
    let leads = async |argv0: &str| {
        until(argv0, async || {
            let t = core.hc(&["get", "--json", &shell]);
            (t["foreground"]["argv"][0] == argv0).then_some(())
        })
        .await;
    };
    let listed = async |yes: bool| {
        until("the listing", async || {
            (find(&shell).await.is_some() == yes).then_some(())
        })
        .await;
    };
    core.hc(&[
        "send",
        "--json",
        &shell,
        "/bin/bash -c 'exec -a collie-reporter /bin/bash reporter'",
    ]);
    leads("collie-reporter").await;
    let reporter = status(&shell, "idle").await;
    assert_eq!(reporter.agent.as_deref(), Some("codex"));
    core.hc(&["keys", "--json", &shell, "ctrl+z"]);
    leads("/bin/sh").await;
    listed(false).await;
    let held = || core.hc(&["get", "--json", &shell])["status"]["app"].clone();
    assert_eq!(
        held(),
        "codex",
        "herdr-core holds a suspended program's status"
    );
    core.hc(&["send", "--json", &shell, "fg"]);
    leads("collie-reporter").await;
    listed(true).await;
    std::fs::write(work.join("leave"), "").unwrap();
    leads("/bin/sh").await;
    listed(false).await;
    assert_eq!(held(), "codex", "and an exited one's");

    // Every key, and Ctrl+End, a notice digit and a paste through a typing attach, which
    // leaves the size a desktop client chose.
    let d = until("the key dump", async || find(&dump).await).await;
    let desktop = Desktop::attach(core, &dump, 70, 20);
    until("the desktop client's size", async || {
        (size(core, &dump) == (70, 20)).then_some(())
    })
    .await;
    let mut expected = String::new();
    let dumped = async |expected: &str| {
        until(expected, async || {
            let keys = std::fs::read_to_string(work.join("keys")).ok()?;
            (keys.replace('\n', "") == expected).then_some(())
        })
        .await;
    };
    for key in KEYS {
        rt.agent_send_keys(&d.pane_id, &[key.herdr_name()])
            .await
            .unwrap();
        expected.push_str(key_bytes(key));
        dumped(&expected).await;
    }
    rt.scroll_bottom(&d.pane_id).await.unwrap();
    expected.push_str("1b5b313b3546");
    rt.notice_digit(&d.pane_id, "1").await.unwrap();
    expected.push_str("31");
    dumped(&expected).await;
    rt.pane_send_text(&d.pane_id, "/x").await.unwrap();
    expected.push_str("2f78");
    dumped(&expected).await;
    assert_eq!(size(core, &dump), (70, 20));
    drop(desktop);

    stop.send_replace(true);
    task.await.unwrap();
}

/// herdr's and Claude Code's session variables, as a core started inside a herdr pane
/// running Claude Code holds them: a task agent must get none.
const STARTER: [&str; 16] = [
    "HERDR_ENV",
    "HERDR_PANE_ID",
    "HERDR_SOCKET_PATH",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
    "HERDR_BIN_PATH",
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_EFFORT",
    "CLAUDE_PID",
];

/// New task, a worktree task and archive with herdr-core alone. collied finds stand-in
/// `claude` and `codex` CLIs first on its `PATH`; the core holds dummy values for every
/// starter variable, which point nowhere.
#[test]
fn live_herdr_core_tasks() {
    if !hc_installed() {
        return;
    }
    let (bin, path) = match std::env::var("COLLIE_E2E_BIN") {
        Ok(bin) => (bin, std::env::var("PATH").unwrap_or_default()),
        Err(_) => {
            let bin = format!("/tmp/hct-{}", std::process::id());
            let path = format!("{bin}:{}", std::env::var("PATH").unwrap_or_default());
            (bin, path)
        }
    };
    // A scratch HOME, so the owner's git and gh configuration never reach the archive.
    let home = format!("{bin}/home");
    let env = [
        ("COLLIE_E2E_BIN", bin.as_str()),
        ("PATH", path.as_str()),
        ("HOME", home.as_str()),
    ];
    if !in_child_with("live_herdr_core_tasks", &env) {
        let _ = std::fs::remove_dir_all(&bin);
        return;
    }
    std::fs::create_dir_all(&home).unwrap();
    let dummies: Vec<(&str, &str)> = STARTER.iter().map(|n| (*n, "collie-e2e")).collect();
    let mut core = CoreSession::start_with("hct", &dummies);
    let rt = runtime();
    rt.block_on(tasks_scenario(&core, Path::new(&bin)));
    drop(rt);
    assert!(core.stop(), "dedicated herdr-core did not stop");
}

/// A stand-in agent CLI: it records its environment and arguments, reports OSC 7501 as
/// `app`, and records each line typed into it. Run under the CLI's name, not a shell's.
fn stand_in(bin: &Path, cli: &str, app: &str) {
    use std::os::unix::fs::PermissionsExt;
    let record = bin.join("record");
    let record = record.display();
    let agent = format!(
        r#"printf '\033]7501;state=idle:app={app}\033\\'
while IFS= read -r p; do printf '%s\n' "$p" >> '{record}/prompts-{cli}'; done
exec sleep 600"#
    );
    let script = format!(
        r#"#!/bin/bash
env > '{record}/env-{cli}'
printf '%s\n' "$@" > '{record}/args-{cli}'
exec -a {cli} /bin/bash -c "$(cat '{record}/../{cli}.agent')"
"#
    );
    std::fs::write(bin.join(format!("{cli}.agent")), agent).unwrap();
    let path = bin.join(cli);
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=e2e",
            "-c",
            "user.email=e2e@example.invalid",
        ])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

async fn tasks_scenario(core: &CoreSession, bin: &Path) {
    use protocol::{
        AgentKind, Cwd, Label, TaskArchiveParams, TaskNewParams, TaskWorktree, TaskWorktreesParams,
    };
    std::fs::create_dir_all(bin.join("record")).unwrap();
    stand_in(bin, "claude", "claude-code");
    stand_in(bin, "codex", "codex");
    let record = |name: &str| std::fs::read_to_string(bin.join("record").join(name)).ok();

    let work = std::fs::canonicalize(&core.work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["commit", "-q", "--allow-empty", "-m", "start"]);
    let held = run(core, "collie-env", &work, "env > held; exec sleep 600");
    let held_env = until("the core's environment", async || {
        std::fs::read_to_string(work.join("held")).ok()
    })
    .await;
    assert!(
        held_env.contains("HERDR_PANE_ID=collie-e2e")
            && held_env.contains("CLAUDE_CODE_CHILD_SESSION=collie-e2e"),
        "the core passes its starter's variables to a terminal"
    );
    core.hc(&["kill", "--json", &held]);
    std::fs::remove_file(work.join("held")).unwrap();

    let followed = Arc::new(Core::new(core.socket.clone()));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(followed.clone().run(stopped));
    tokio::time::timeout(Duration::from_secs(5), followed.settled())
        .await
        .unwrap();
    let kinds = vec![
        AgentKind::new("claude").unwrap(),
        AgentKind::new("codex").unwrap(),
    ];
    let drive = Driver::new(
        core.root.join("no-herdr.sock"),
        kinds.clone(),
        std::slice::from_ref(&work),
    )
    .unwrap()
    .with_core(Some(followed.clone()));
    let yes: Authorized = Arc::new(|| true);

    let Ok(Response::TaskOptions(options)) = drive.task_options().await else {
        panic!("no task options");
    };
    assert_eq!(options.agents, kinds);

    let new = |agent: &str, prompt: &str, worktree| TaskNewParams {
        op_id: OpId::new("N".repeat(22)).unwrap(),
        cwd: Cwd::new(work.to_str().unwrap()).unwrap(),
        agent: AgentKind::new(agent).unwrap(),
        prompt: Some(PromptText::new(prompt.to_owned()).unwrap()),
        label: Some(Label::new("e2e").unwrap()),
        new_folder: None,
        worktree,
        clone: None,
        resume: None,
        ultracode: false,
    };
    let started = |reply| match reply {
        (
            Ok(Response::TaskStarted {
                workspace_id,
                terminal_id,
            }),
            _,
        ) => (workspace_id, terminal_id),
        (other, _) => panic!("task not started: {other:?}"),
    };

    let (group, claude) = started(
        drive
            .task_new(new("claude", "fix the test", None), &yes)
            .await,
    );
    assert!(group.as_str().starts_with("hc:"), "{group:?}");
    assert!(claude.as_str().starts_with(&format!("{}:", group.as_str())));
    until("the prompt in Claude Code", async || {
        record("prompts-claude").filter(|p| p.contains("fix the test"))
    })
    .await;
    assert_eq!(record("args-claude").as_deref(), Some("\n"));
    let stripped = |env: String| {
        for name in STARTER {
            assert!(
                !env.lines().any(|l| l.starts_with(&format!("{name}="))),
                "{name} reached the agent"
            );
        }
        env
    };
    let env = stripped(record("env-claude").unwrap());
    assert!(env.contains("TERM=xterm-256color") && env.contains("COLORTERM=truecolor"));

    let branch = TaskWorktree::Create {
        branch: "feat/hc-task".to_owned(),
    };
    let (_, codex) = started(
        drive
            .task_new(new("codex", "review it", Some(branch)), &yes)
            .await,
    );
    let checkout = work.join(".worktree/feat-hc-task");
    until("the prompt in Codex", async || {
        record("prompts-codex").filter(|p| p.contains("review it"))
    })
    .await;
    assert_eq!(record("args-codex").as_deref(), Some("--no-daemon\n"));
    stripped(record("env-codex").unwrap());
    assert!(
        git(&work, &["worktree", "list", "--porcelain"])
            .contains(&format!("worktree {}\nHEAD", checkout.display()))
    );
    let codex_address = codex.as_str().rsplit(':').next().unwrap().to_owned();
    assert_eq!(
        core.hc(&["get", "--json", &codex_address])["cwd"],
        checkout.to_str().unwrap()
    );
    assert_eq!(
        core.hc(&["get", "--json", &codex_address])["labels"],
        serde_json::json!({"collie.kind": "codex", "collie.label": "e2e"})
    );

    let Ok(Response::TaskWorktrees { source, worktrees }) = drive
        .task_worktrees(
            TaskWorktreesParams {
                cwd: Cwd::new(work.to_str().unwrap()).unwrap(),
            },
            &yes,
        )
        .await
    else {
        panic!("no worktrees");
    };
    assert_eq!(source.as_str(), work.to_str().unwrap());
    assert_eq!(worktrees.len(), 1);
    assert_eq!(worktrees[0].branch.as_deref(), Some("feat/hc-task"));

    let archive = |id: &TerminalId| TaskArchiveParams {
        op_id: OpId::new("A".repeat(22)).unwrap(),
        terminal_id: id.clone(),
        confirm: true,
    };
    let Ok(Response::TaskArchived { message }) = drive.task_archive(archive(&codex), &yes).await
    else {
        panic!("the worktree task was not archived");
    };
    assert!(
        message.starts_with("Removed worktree checkout"),
        "{message}"
    );
    assert!(!checkout.exists());
    assert!(core.hc(&["get", "--json", &codex_address]).is_null());
    assert!(!git(&work, &["worktree", "list"]).contains("feat-hc-task"));

    let claude_address = claude.as_str().rsplit(':').next().unwrap().to_owned();
    let Ok(Response::TaskArchived { message }) = drive.task_archive(archive(&claude), &yes).await
    else {
        panic!("the task was not archived");
    };
    assert!(message.starts_with("Closed pane."), "{message}");
    assert!(core.hc(&["get", "--json", &claude_address]).is_null());
    assert!(work.exists());

    stop.send_replace(true);
    task.await.unwrap();
}
