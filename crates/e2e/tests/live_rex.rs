#[allow(dead_code)]
mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use collie_core::{AgentState, CollieCore, MachineFlock};
use collied::config::TasksConfig;
use collied::runtime::rex::{GROUP_CLOSE, Rex};
use collied::runtime::{AgentInfo, Error, Runtime};
use collied::server::{self, ServerConfig};
use common::*;
use tokio::sync::watch;

/// A stand-in agent: it reports OSC 7501 as Claude Code does, from the process that leads the
/// terminal, and moves on once the test creates `go`, then `block`, in its folder. One
/// process with no child, so the reporter is always the foreground process.
const AGENT: &str = r#"$| = 1;
sub st { print "\e]7501;$_[0]\e\\" }
sub wait_for { select(undef, undef, undef, 0.05) until -e $_[0] }
print "collie screen probe\r\n";
st("state=idle:app=claude-code");
wait_for("go");
st("state=working:app=claude-code");
wait_for("block");
st("state=blocked:app=claude-code:kind=permission:msg=approve Bash");
select(undef, undef, undef, 600);
"#;

/// An agent below 100,000 lines of scrollback.
const LONG: &str = r#"$| = 1;
print "line $_\r\n" for 1 .. 100000;
print "\e]7501;state=idle:app=claude-code\e\\";
select(undef, undef, undef, 600);
"#;

/// An agent whose last line, soft-wrapped across the whole screen, is longer than any tail.
const WIDE: &str = r#"$| = 1;
print "short $_\r\n" for 1 .. 50;
print "x" x 2000000, "\r\nprompt> ";
print "\e]7501;state=idle:app=claude-code\e\\";
select(undef, undef, undef, 600);
"#;

/// A shell that reported a status and leads its terminal.
const SHELL: &str = r#"printf '\033]7501;state=idle:app=claude-code\033\\'; read -r -t 600 _"#;

#[test]
fn live_rex() {
    let Some(cli) = rex_cli() else {
        return;
    };
    let mut server = RexServer::start(&cli, "x");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(scenario(&server));
    drop(rt);
    assert!(server.stop(), "dedicated Rex server did not stop");
}

fn new_session(server: &RexServer, label: &str, command: &[&str]) -> (String, String) {
    let work = server.work.to_str().unwrap();
    let mut args = vec!["new", label, "--shell=none", "-c", work, "--json", "--"];
    args.extend(command);
    let created = server.rex(&args);
    let session = created["session_id"].as_str();
    let block = created["initial_windows"][0]["block_ids"][0].as_str();
    match (session, block) {
        (Some(s), Some(b)) => (s.to_owned(), b.to_owned()),
        _ => panic!("rex new: {created}"),
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

async fn scenario(server: &RexServer) {
    let (session, block) = new_session(server, "collie-live", &["/usr/bin/perl", "-e", AGENT]);
    let (_, shell) = new_session(server, "collie-shell", &["/bin/bash", "-c", SHELL]);
    let agent = format!("rex:{block}");

    let rex = Arc::new(Rex::new(server.socket.clone()));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(rex.clone().run(stopped));
    tokio::time::timeout(Duration::from_secs(5), rex.settled())
        .await
        .unwrap();
    let report = rex.report();
    assert!(
        report.version.is_some() && report.refused.is_none(),
        "{report:?}"
    );
    assert!(
        report.api_revision >= Some(13) && report.remote.is_empty(),
        "{report:?}"
    );

    let rt = Runtime::new(server.root.join("no-herdr.sock")).with_rex(Some(rex.clone()));
    let find = async |status: &str| -> Option<AgentInfo> {
        (rt.agent_list().await.ok()?.into_iter())
            .find(|a| a.terminal_id == agent && a.agent_status == status)
    };
    let idle = until("the agent's idle report", async || find("idle").await).await;
    assert_eq!(idle.agent.as_deref(), Some("claude"));
    assert_eq!(idle.workspace_id, format!("rex:{session}"));
    let poll = rt.poll().await.unwrap();
    let ids: Vec<&str> = poll.agents.iter().map(|a| a.terminal_id.as_str()).collect();
    assert_eq!(ids, [agent.as_str()], "a shell in front is never an agent");
    assert!(!ids.contains(&format!("rex:{shell}").as_str()));
    let groups: Vec<(&str, &str)> = (poll.workspaces.iter())
        .map(|w| (w.workspace_id.as_str(), w.label.as_str()))
        .collect();
    assert_eq!(groups, [(idle.workspace_id.as_str(), "collie-live")]);

    let size = || server.rex(&["block", "call", "-b", &block, "size"]);
    let before = size();
    assert!(before["columns"].as_u64().is_some(), "{before}");
    let visible = rt.agent_read(&agent, "visible", None).await.unwrap().text;
    assert!(visible.contains("collie screen probe"), "{visible:?}");
    let detected = rt.detection_text(&agent).await.unwrap();
    assert!(detected.starts_with("collie screen probe"), "{detected:?}");
    let columns = rt.pane_columns(&agent).await.unwrap();
    assert_eq!(columns.map(u64::from), before["columns"].as_u64());
    assert_eq!(
        size(),
        before,
        "a read changed the terminal's size or owner"
    );

    let (_, long) = new_session(server, "collie-long", &["/usr/bin/perl", "-e", LONG]);
    let long = format!("rex:{long}");
    let read = until("the long terminal's agent", async || {
        rt.agent_get(&long).await.ok()?;
        let read = rt
            .agent_read(&long, "recent_unwrapped", Some(50))
            .await
            .ok()?;
        read.text.contains("line 100000").then_some(read.text)
    })
    .await;
    let rows: Vec<&str> = read.split("\r\n").collect();
    assert_eq!(rows.len(), 50, "{read:?}");
    assert_eq!(rows[0], "line 99951");
    assert_eq!(rows[49], "line 100000");

    let (_, wide) = new_session(server, "collie-wide", &["/usr/bin/perl", "-e", WIDE]);
    let wide = format!("rex:{wide}");
    let read = until("the wide terminal's agent", async || {
        rt.agent_get(&wide).await.ok()?;
        let read = rt.agent_read(&wide, "visible", None).await.ok()?;
        read.text.ends_with("prompt>").then_some(read.text)
    })
    .await;
    let rows: Vec<&str> = read.split("\r\n").collect();
    let (prompt, xs) = rows.split_last().unwrap();
    assert_eq!(*prompt, "prompt>");
    assert!(
        xs.len() > 1
            && xs
                .iter()
                .all(|r| !r.is_empty() && r.bytes().all(|b| b == b'x'))
    );
    assert!(
        rt.agent_read(&wide, "recent_unwrapped", Some(10))
            .await
            .is_err()
    );

    std::fs::write(server.work.join("go"), "").unwrap();
    let working = until("the working report", async || find("working").await).await;
    std::fs::write(server.work.join("block"), "").unwrap();
    let blocked = until("the blocked report", async || find("blocked").await).await;
    let seqs = [
        idle.state_change_seq,
        working.state_change_seq,
        blocked.state_change_seq,
    ];
    assert!(seqs[0] != seqs[1] && seqs[1] != seqs[2], "{seqs:?}");
    let explained = rt.agent_explain(&agent).await.unwrap();
    assert_eq!(
        explained.matched_rule.map(|r| r.id).as_deref(),
        Some("permission")
    );

    assert!(matches!(
        rt.workspace_close(&idle.workspace_id).await,
        Err(Error::Unsupported(why)) if why == GROUP_CLOSE
    ));
    rt.pane_close(&agent).await.unwrap();
    until("the closed agent to leave", async || {
        rt.agent_list()
            .await
            .ok()
            .filter(|l| l.iter().all(|a| a.terminal_id != agent))
    })
    .await;
    let sessions = server.rex(&["ls", "--json"]);
    let listed = sessions.to_string();
    assert!(
        listed.contains(&session) && listed.contains("collie-shell"),
        "{listed}"
    );

    stop.send_replace(true);
    task.await.unwrap();
}

/// The session in the Claude Code transcript fixture.
const CLAUDE_SESSION: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
const CODEX_SESSION: &str = "01a10f07-b5ed-79a3-989d-fa10b96c5cab";
const CLOSED_SESSION: &str = "0a0a0a0a-0000-4000-8000-000000000001";

/// A stand-in Claude Code: it writes its `sessions/<pid>.json` in the folder it is given for
/// the first session it is given, then reports `idle`, as Claude Code does. Once the test
/// creates `clear`, it names the second session in that file and reports nothing, as
/// Claude Code's `/clear` does.
const CLAUDE: &str = r#"$| = 1;
use Cwd;
my ($dir, $id, $cleared) = @ARGV;
my $started = time() * 1000;
sub claim {
    open(my $f, ">", "$dir/$$.tmp") or die;
    printf $f '{"pid":%d,"sessionId":"%s","cwd":"%s","startedAt":%d}', $$, $_[0], getcwd(), $started;
    close $f;
    rename("$dir/$$.tmp", "$dir/$$.json") or die;
}
claim($id);
print "\e]7501;state=idle:app=claude-code\e\\";
select(undef, undef, undef, 0.05) until -e "clear";
claim($cleared);
select(undef, undef, undef, 600);
"#;

/// The phone over the tailnet, with a Rex server of the test's own and no herdr. collied
/// reads Claude Code's and Codex's files from scratch folders, never the user's.
#[test]
fn live_rex_phone() {
    let Some(cli) = rex_cli() else {
        return;
    };
    let preset = std::env::var("COLLIE_E2E_AGENTS").ok();
    let agents = (preset.clone()).unwrap_or_else(|| format!("/tmp/rxa-{}", std::process::id()));
    // Removed even when the child run fails; a folder the caller named is left alone.
    let _scratch = preset.is_none().then(|| TempDir(agents.clone().into()));
    let claude = format!("{agents}/claude");
    let codex = format!("{agents}/codex");
    let env = [
        ("COLLIE_E2E_AGENTS", agents.as_str()),
        ("CLAUDE_CONFIG_DIR", claude.as_str()),
        ("CODEX_HOME", codex.as_str()),
    ];
    if !in_child_with("live_rex_phone", &env) {
        return;
    }
    let mut server = RexServer::start(&cli, "p");
    let root = TempDir::new("e2e-rex");
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
        &server,
        Path::new(&agents),
    ));
    drop(phone);
    drop(rt);
    assert!(server.stop(), "dedicated Rex server did not stop");
}

/// Transcripts for the stand-ins' sessions and one closed Claude Code session, in `work`.
fn transcripts(agents: &Path, work: &Path) {
    let folder = work.to_str().unwrap();
    let project: String = folder
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let project = agents.join("claude/projects").join(project);
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(agents.join("claude/sessions")).unwrap();
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

async fn phone_scenario(
    root: &Path,
    net: &Net,
    phone: &Arc<CollieCore>,
    server: &RexServer,
    agents: &Path,
) {
    let work = std::fs::canonicalize(&server.work).unwrap();
    transcripts(agents, &work);
    let sessions = agents.join("claude/sessions");
    let (_, claude) = new_session(
        server,
        "collie-claude",
        &[
            "/usr/bin/perl",
            "-e",
            CLAUDE,
            sessions.to_str().unwrap(),
            CLAUDE_SESSION,
            CLOSED_SESSION,
        ],
    );
    // A stand-in Codex: this test binary, run as `codex` (`fake_codex`), and its hook as
    // `hook` (`fake_hook`).
    let bin = server.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in ["codex", "hook"] {
        std::fs::copy(std::env::current_exe().unwrap(), bin.join(name)).unwrap();
    }
    let codex_exe = bin.join("codex");
    let (_, codex) = new_session(
        server,
        "collie-codex",
        &[
            codex_exe.to_str().unwrap(),
            "fake_codex",
            "--exact",
            "--nocapture",
        ],
    );
    let data_dir = root.join("collied");
    let handle = server::start_with(
        net.mac.clone(),
        ServerConfig {
            rex: Some(server.socket.clone()),
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
    std::fs::write(
        work.join("control"),
        handle.control_path().to_str().unwrap(),
    )
    .unwrap();
    let (machine, _) = pair(&handle.control_path(), phone, LABEL).await;
    let m = machine.id.clone();

    let (claude_id, codex_id) = (format!("rex:{claude}"), format!("rex:{codex}"));
    flock_until(phone, &m, "the Rex agents", |f| {
        f.agents.len() == 2 && f.agents.iter().all(|a| a.status == AgentState::Idle)
    })
    .await;
    let chat = phone
        .agent_history(m.clone(), claude_id.clone(), None, None, None)
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

    // `/clear`: a new session in the pid file and no report, followed by the reconcile tick.
    std::fs::write(work.join("clear"), "").unwrap();
    let mut listed = Vec::new();
    for _ in 0..50 {
        let sessions = phone
            .task_sessions(m.clone(), work.to_str().unwrap().to_owned())
            .await
            .unwrap();
        listed = sessions.iter().map(|s| s.session_id.to_string()).collect();
        if listed == [CLAUDE_SESSION] {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(listed, [CLAUDE_SESSION], "the cleared session is open");

    phone.close_pane(m.clone(), claude_id, true).await.unwrap();
    phone.close_pane(m.clone(), codex_id, true).await.unwrap();
    flock_until(phone, &m, "the closed agents leave", |f| {
        f.agents.is_empty()
    })
    .await;
    handle.shutdown().await;
}

/// Codex, as the phone scenario runs it in its Rex block: it reports OSC 7501, then runs its
/// SessionStart hook (`fake_hook`) once the test creates `report`, as Codex runs `collied
/// hook`, from the process that leads the block.
/// A process named `codex`, which a copied shell cannot be on macOS.
#[test]
fn fake_codex() {
    let exe = std::env::current_exe().unwrap();
    if exe.file_name().and_then(|n| n.to_str()) != Some("codex") {
        return;
    }
    use std::io::Write;
    print!("\x1b]7501;state=idle:app=codex\x1b\\");
    std::io::stdout().flush().unwrap();
    while !Path::new("report").exists() {
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut hook = std::process::Command::new(exe.with_file_name("hook"))
        .args(["fake_hook", "--exact", "--nocapture"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = serde_json::json!({
        "session_id": CODEX_SESSION, "hook_event_name": "SessionStart", "source": "startup",
    });
    hook.stdin
        .take()
        .unwrap()
        .write_all(start.to_string().as_bytes())
        .unwrap();
    hook.wait().unwrap();
    std::thread::sleep(Duration::from_secs(600));
}

/// `collied hook` under the stand-in Codex, with the environment Rex gave the block.
#[test]
fn fake_hook() {
    let exe = std::env::current_exe().unwrap();
    if exe.file_name().and_then(|n| n.to_str()) != Some("hook") {
        return;
    }
    let control = std::fs::read_to_string("control").unwrap();
    runtime().block_on(collied::hooks::run(Path::new(control.trim())));
}
