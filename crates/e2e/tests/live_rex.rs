#[allow(dead_code)]
mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use collie_core::{AgentState, CollieCore, MachineFlock};
use collied::approvals::Approvals;
use collied::audit::Audit;
use collied::config::TasksConfig;
use collied::drive::{Authorized, Driver};
use collied::runtime::rex::{GROUP_CLOSE, Rex};
use collied::runtime::{AgentInfo, Error, Runtime};
use collied::server::{self, ServerConfig};
use common::*;
use protocol::{
    AgentAnswerNoticeParams, AgentPromptParams, AgentSendKeysParams, AgentSlashDraftParams,
    Approval, ApprovalDecideParams, ApprovalOutcome, Decision, DraftText, ErrorCode, Event, Key,
    KeyCombo, Label, NoticeDigit, OpId, PromptText, Response, SlashCommand, TerminalId,
};
use tokio::sync::{broadcast, watch};

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

/// A stand-in agent that reports idle and appends each byte it reads, as hex, to `keys`:
/// raw mode without echo, signals or flow control, so every byte reaches it as sent.
const KEY_DUMP: &str = r#"use POSIX;
$| = 1;
print "\e]7501;state=idle:app=claude-code\e\\";
my $t = POSIX::Termios->new; $t->getattr(0);
$t->setlflag($t->getlflag & ~(ECHO|ICANON|ISIG|IEXTEN));
$t->setiflag($t->getiflag & ~(ICRNL|IXON|INLCR|IGNCR|ISTRIP|BRKINT|PARMRK));
$t->setcc(VMIN, 1); $t->setcc(VTIME, 0);
$t->setattr(0, TCSANOW);
open my $f, '>>', 'keys' or die; select((select($f), $| = 1)[0]);
while (sysread(STDIN, my $b, 1)) { printf $f "%02x", ord $b }
"#;

/// Beyond `COMBOS`: what herdr-core sends for these.
const MORE: [(&str, &str); 7] = [
    ("f1", "1b4f50"),
    ("f5", "1b5b31357e"),
    ("ctrl+shift+up", "1b5b313b3641"),
    ("ctrl+alt+shift+x", "1b78"),
    ("ctrl+2", "00"),
    ("ctrl+/", "1f"),
    ("ctrl+]", "1d"),
];

#[test]
fn live_rex_input() {
    let Some(cli) = rex_cli() else {
        return;
    };
    let mut server = RexServer::start(&cli, "i");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(input_scenario(&server));
    drop(rt);
    assert!(server.stop(), "dedicated Rex server did not stop");
}

async fn input_scenario(server: &RexServer) {
    let (_, dump) = new_session(server, "collie-keys", &["/usr/bin/perl", "-e", KEY_DUMP]);
    let (_, silent) = new_session(
        server,
        "collie-silent",
        &["/bin/bash", "-c", "exec -a claude sleep 600"],
    );
    let (dump, silent) = (format!("rex:{dump}"), format!("rex:{silent}"));

    let rex = Arc::new(Rex::new(server.socket.clone()));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(rex.clone().run(stopped));
    let none = server.root.join("no-herdr.sock");
    let rt = Runtime::new(none.clone()).with_rex(Some(rex.clone()));
    let find = async |id: &str| -> Option<AgentInfo> {
        (rt.agent_list().await.ok()?.into_iter()).find(|a| a.terminal_id == id)
    };
    until("the key dump's idle report", async || {
        find(&dump).await.filter(|a| a.agent_status == "idle")
    })
    .await;

    // Each key sent on its own, in order on one connection, so every byte the program
    // receives, a stray one included, shows in the next check.
    let file = server.work.join("keys");
    let dumped = async |expected: &str| {
        let arrived = until(expected, async || {
            let keys = std::fs::read_to_string(&file).unwrap_or_default();
            (keys.len() >= expected.len()).then_some(keys)
        })
        .await;
        assert_eq!(arrived, expected);
    };
    let mut expected = String::new();
    for key in &KEYS {
        rt.agent_send_keys(&dump, &[key.herdr_name()])
            .await
            .unwrap();
        expected.push_str(key_bytes(key));
    }
    dumped(&expected).await;
    let chars = "abcdefghijklmnopqrstuvwxyz0123456789',-./;=\\]`";
    // As herdr-core: Shift makes a letter a capital and leaves any other key as it is.
    for c in chars.chars() {
        let shifted = format!("shift+{c}");
        rt.agent_send_keys(&dump, &[c.to_string().as_str(), shifted.as_str()])
            .await
            .unwrap();
        let capital = c.to_ascii_uppercase();
        expected.push_str(&format!("{:02x}{:02x}", c as u32, capital as u32));
    }
    for (name, bytes) in COMBOS.into_iter().chain(MORE) {
        assert!(KeyCombo::new(name).is_ok(), "{name}");
        rt.agent_send_keys(&dump, &[name]).await.unwrap();
        expected.push_str(bytes);
    }
    rt.scroll_bottom(&dump).await.unwrap();
    expected.push_str("1b5b313b3546");
    rt.notice_digit(&dump, "1").await.unwrap();
    expected.push_str("31");
    dumped(&expected).await;

    // Refused before anything is sent, the keys before them in the call included.
    for name in [
        "cmd+k",
        "ctrl+[",
        "ctrl+alt+[",
        "ctrl+shift+a",
        "ctrl+i",
        "ctrl+shift+space",
    ] {
        assert!(KeyCombo::new(name).is_ok(), "{name}");
        assert!(
            rt.agent_send_keys(&dump, &["a", name]).await.is_err(),
            "{name}"
        );
    }

    // A prompt is a paste and Enter, text alone a paste; this program asked for no
    // bracketed paste, so a newline arrives as a carriage return.
    rt.agent_prompt(&dump, "hi\nyo", "idle").await.unwrap();
    expected.push_str("68690d796f0d");
    rt.pane_send_text(&dump, "\u{1b}[200~/x\u{1b}[201~")
        .await
        .unwrap();
    expected.push_str("2f78");
    dumped(&expected).await;

    // No status: listed, read-only, and refused by the ready check.
    let s = until("the agent with no status", async || find(&silent).await).await;
    assert!(s.read_only && s.agent_status == "unknown", "{s:?}");
    let drive = Driver::new(none, vec![], &[]).unwrap().with_rex(Some(rex));
    let yes: Authorized = Arc::new(|| true);
    let keys = AgentSendKeysParams {
        op_id: OpId::new("K".repeat(22)).unwrap(),
        terminal_id: TerminalId::new(silent).unwrap(),
        keys: vec![Key::Enter],
    };
    assert_eq!(
        drive.send_keys(keys, &yes).await.0.unwrap_err().0,
        ErrorCode::AgentNotReady
    );

    stop.send_replace(true);
    task.await.unwrap();
}

/// A stand-in Claude Code that asks the same permission question when the test creates
/// `go`, then twice in a row from `go2`, in one process so its reports come from the
/// foreground process. Arrows move the cursor and redraw; Enter answers with the option, Esc
/// with `esc`, each appended to `answers`, then it reports `working`. Once `again` exists it
/// reports the dialog anew without being answered, as a second identical prompt would.
const ASKER: &str = r#"use POSIX; use utf8; binmode STDOUT, ':utf8'; $| = 1;
sub st { print "\e]7501;$_[0]\e\\" }
sub wait_for { select(undef, undef, undef, 0.05) until -e $_[0] }
my $t = POSIX::Termios->new; $t->getattr(0);
$t->setlflag($t->getlflag & ~(ECHO|ICANON|ISIG|IEXTEN));
$t->setiflag($t->getiflag & ~(ICRNL|IXON|INLCR|IGNCR|ISTRIP|BRKINT|PARMRK));
$t->setcc(VMIN, 1); $t->setcc(VTIME, 0);
$t->setattr(0, TCSANOW);
my @opts = ("Yes", "Yes, and don't ask again for touch commands", "No");
my ($cur, $flipped) = (0, 0);
sub draw {
  print "\e[2J\e[H", "\x{2500}" x 40, "\n Bash command\n\n   touch /tmp/collie-rex-approval\n",
    "   Create an empty file\n\n Do you want to proceed?\n";
  printf "%s%d. %s\n", ($_ == $cur ? " \x{276f} " : "   "), $_ + 1, $opts[$_] for 0 .. $#opts;
  print "\n Esc to cancel \x{b7} Tab to amend\n";
}
sub blocked { st("state=blocked:app=claude-code:kind=permission:msg=approve Bash") }
sub answer {
  open my $f, '>>', 'answers' or die; print $f "$_[0]\n"; close $f;
  print "\e[2J\e[Hanswered $_[0]\n"; st("state=working:app=claude-code");
}
sub ask {
  $cur = 0; draw(); blocked();
  my $rin = ''; vec($rin, 0, 1) = 1;
  while (1) {
    if (-e "again" && !$flipped) { $flipped = 1; st("state=working:app=claude-code"); draw(); blocked() }
    next unless select(my $r = $rin, undef, undef, 0.05);
    sysread(STDIN, my $k, 16) or exit;
    if ($k eq "\e[B") { $cur++ if $cur < $#opts; draw() }
    elsif ($k eq "\e[A") { $cur-- if $cur > 0; draw() }
    elsif ($k eq "\r") { answer($cur + 1); return }
    elsif ($k eq "\e") { answer("esc"); return }
  }
}
st("state=idle:app=claude-code");
wait_for("go"); ask();
wait_for("go2"); ask(); ask();
select(undef, undef, undef, 600);
"#;

#[test]
fn live_rex_approvals() {
    let Some(cli) = rex_cli() else {
        return;
    };
    let mut server = RexServer::start(&cli, "a");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(approvals_scenario(&server));
    drop(rt);
    assert!(server.stop(), "dedicated Rex server did not stop");
}

async fn approvals_scenario(server: &RexServer) {
    let (_, block) = new_session(server, "collie-asks", &["/usr/bin/perl", "-e", ASKER]);
    let agent = format!("rex:{block}");
    let rex = Arc::new(Rex::new(server.socket.clone()));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(rex.clone().run(stopped));
    let none = server.root.join("no-herdr.sock");
    let rt = Runtime::new(none.clone()).with_rex(Some(rex.clone()));
    let audit = Arc::new(Audit::open(&server.root.join("audit.log")).unwrap());
    let (tx, mut events) = broadcast::channel(64);
    let approvals = Approvals::new(none, "nMAC".into(), tx, audit, None).with_rex(Some(rex));
    let find = async || -> Option<AgentInfo> {
        (rt.agent_list().await.ok()?.into_iter()).find(|a| a.terminal_id == agent)
    };
    let idle = until("the asker's idle report", async || {
        find().await.filter(|a| a.agent_status == "idle")
    })
    .await;

    // Each dialog is a new blocked report, read and raised once.
    let raise = async |events: &mut broadcast::Receiver<Event>, after: u64| {
        let blocked = until("a new dialog", async || {
            find()
                .await
                .filter(|a| a.agent_status == "blocked" && a.state_change_seq != after)
        })
        .await;
        let approval = until("the approval", async || {
            approvals.observe(&rt.agent_list().await.ok()?, &[]).await;
            std::iter::from_fn(|| events.try_recv().ok()).find_map(|e| match e {
                Event::ApprovalNeeded { approval } => Some(approval),
                _ => None,
            })
        })
        .await;
        assert_eq!(
            approval.options,
            [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
        );
        (blocked.state_change_seq, approval)
    };
    let yes: Authorized = Arc::new(|| true);
    let decide = async |a: &Approval, decision: Decision| {
        let p = ApprovalDecideParams {
            approval_id: a.approval_id.clone(),
            decision,
            choice: None,
            nonce: a.nonce.clone(),
            note: None,
        };
        match approvals.decide("phone", "nPHONE", p, &yes).await {
            Ok(Response::ApprovalResolved { outcome, .. }) => outcome,
            other => panic!("{other:?}"),
        }
    };
    let answers = server.work.join("answers");
    let answered = async |want: &str| {
        until(want, async || {
            let got = std::fs::read_to_string(&answers).unwrap_or_default();
            (got.lines().collect::<Vec<_>>().join(" ") == want).then_some(())
        })
        .await;
    };

    // Deny is "3. No": two arrows, a read that finds the cursor there, then Enter.
    std::fs::write(server.work.join("go"), "").unwrap();
    let (seq1, first) = raise(&mut events, idle.state_change_seq).await;
    assert!(matches!(
        decide(&first, Decision::Deny).await,
        ApprovalOutcome::Applied {
            decision: Decision::Deny,
            ..
        }
    ));
    answered("3").await;

    // Approve is Enter on "1. Yes"; the same question then asks again at once.
    std::fs::write(server.work.join("go2"), "").unwrap();
    let (seq2, second) = raise(&mut events, seq1).await;
    assert!(matches!(
        decide(&second, Decision::Approve).await,
        ApprovalOutcome::Applied {
            decision: Decision::Approve,
            ..
        }
    ));
    answered("3 1").await;
    let (seq3, third) = raise(&mut events, seq2).await;
    assert_ne!(third.approval_id, second.approval_id);
    assert_eq!(third.snippet, second.snippet);

    // The same dialog reported anew: the decision meant for the one before answers nothing.
    std::fs::write(server.work.join("again"), "").unwrap();
    until("the dialog reported anew", async || {
        find()
            .await
            .filter(|a| a.agent_status == "blocked" && a.state_change_seq != seq3)
    })
    .await;
    assert_eq!(
        decide(&third, Decision::Approve).await,
        ApprovalOutcome::Superseded
    );
    let (_, fourth) = raise(&mut events, seq3).await;
    assert!(matches!(
        decide(&fourth, Decision::Approve).await,
        ApprovalOutcome::Applied { .. }
    ));
    answered("3 1 1").await;

    stop.send_replace(true);
    task.await.unwrap();
}

/// A stand-in Claude Code input box, idle: typed and pasted text shows in the box, Enter
/// logs `prompt <text>` to `log`, Ctrl+U clears, Ctrl+End logs `end` and takes down the
/// "Jump to bottom" banner `banner` puts up, and a digit in an empty box under the notice
/// `notice` puts up logs `notice <digit>` and takes it down.
const BOXED: &str = r#"use POSIX; use utf8; binmode STDOUT, ':utf8'; $| = 1;
my $t = POSIX::Termios->new; $t->getattr(0);
$t->setlflag($t->getlflag & ~(ECHO|ICANON|ISIG|IEXTEN));
$t->setiflag($t->getiflag & ~(ICRNL|IXON|INLCR|IGNCR|ISTRIP|BRKINT|PARMRK));
$t->setcc(VMIN, 1); $t->setcc(VTIME, 0);
$t->setattr(0, TCSANOW);
sub logit { open my $f, '>>', 'log' or die; print $f "$_[0]\n"; close $f }
my $buf = '';
sub shown { (-e 'banner' ? 1 : 0) . (-e 'notice' ? 1 : 0) }
sub draw {
  print "\e[2J\e[H";
  print "\e[44m Jump to bottom \e[0m\n" if -e 'banner';
  print "\x{2726} Heads up: plans can change\n  1: Show me  0: Dismiss\n\n" if -e 'notice';
  my $rule = "\x{2500}" x 40;
  print "$rule\n\x{276f} ", ($buf eq '' ? "\e[2mTry \"fix lint errors\"\e[0m" : $buf), "\n$rule\n";
}
print "\e]7501;state=idle:app=claude-code\e\\";
my $was = shown(); draw();
my $rin = ''; vec($rin, 0, 1) = 1;
while (1) {
  if (shown() ne $was) { $was = shown(); draw() }
  next unless select(my $r = $rin, undef, undef, 0.05);
  sysread(STDIN, my $k, 4096) or exit;
  utf8::decode($k);
  while ($k =~ s/^(\e\[1;5F|.)//s) {
    my $c = $1;
    if ($c eq "\e[1;5F") { logit("end"); unlink 'banner' }
    elsif ($c eq "\r") { logit("prompt $buf"); $buf = '' }
    elsif ($c eq "\x15") { $buf = '' }
    elsif ($c eq "\x7f") { chop $buf }
    elsif ($c =~ /^\d$/ && $buf eq '' && -e 'notice') { logit("notice $c"); unlink 'notice' }
    elsif ($c ge ' ') { $buf .= $c }
  }
  $was = shown(); draw();
}
"#;

#[test]
fn live_rex_drive() {
    let Some(cli) = rex_cli() else {
        return;
    };
    let mut server = RexServer::start(&cli, "d");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(drive_scenario(&server));
    drop(rt);
    assert!(server.stop(), "dedicated Rex server did not stop");
}

/// What drive does with a Claude Code on screen, through Rex's screens and input together.
async fn drive_scenario(server: &RexServer) {
    let (_, block) = new_session(server, "collie-box", &["/usr/bin/perl", "-e", BOXED]);
    let agent = format!("rex:{block}");
    let rex = Arc::new(Rex::new(server.socket.clone()));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(rex.clone().run(stopped));
    let none = server.root.join("no-herdr.sock");
    let rt = Runtime::new(none.clone()).with_rex(Some(rex.clone()));
    let drive = Driver::new(none, vec![], &[]).unwrap().with_rex(Some(rex));
    let yes: Authorized = Arc::new(|| true);
    until("the box's idle report", async || {
        (rt.agent_get(&agent).await.ok()).filter(|a| a.agent_status == "idle")
    })
    .await;
    let terminal = TerminalId::new(agent.clone()).unwrap();
    let shows = async |text: &str| {
        until(text, async || {
            let screen = rt.agent_read(&agent, "visible", None).await.ok()?.text;
            screen.contains(text).then_some(())
        })
        .await;
    };
    let log = server.work.join("log");
    let logged = async |want: &str| {
        until(want, async || {
            let got = std::fs::read_to_string(&log).unwrap_or_default();
            (got.lines().collect::<Vec<_>>().join(", ") == want).then_some(())
        })
        .await;
    };
    let prompt = |text: &str| AgentPromptParams {
        op_id: OpId::new(text.repeat(22)[..22].to_owned()).unwrap(),
        terminal_id: terminal.clone(),
        text: PromptText::new(text.to_owned()).unwrap(),
        expected_draft: None,
    };
    shows("Try \"fix lint errors\"").await;

    // A prompt reads the box empty, then pastes and presses Enter.
    assert_eq!(drive.prompt(prompt("a"), &yes).await, Ok(Response::Ok));
    logged("prompt a").await;

    // The slash mirror pastes the command without Enter, then clears it.
    let slash = |command: &str, expected: Option<&str>| AgentSlashDraftParams {
        terminal_id: terminal.clone(),
        command: SlashCommand::new(command.to_owned()).unwrap(),
        expected_draft: expected.map(|d| DraftText::new(d.to_owned()).unwrap()),
    };
    assert_eq!(
        drive.slash_draft(slash("/model", None), &yes).await,
        Ok(Response::Ok)
    );
    shows("\u{276f} /model").await;
    assert_eq!(
        drive.slash_draft(slash("", Some("/model")), &yes).await,
        Ok(Response::Ok)
    );
    shows("Try \"fix lint errors\"").await;

    // Scrolled up: no prompt, and Ctrl+End brings the transcript back.
    std::fs::write(server.work.join("banner"), "").unwrap();
    shows("Jump to bottom").await;
    assert_eq!(
        drive.prompt(prompt("b"), &yes).await.unwrap_err().0,
        ErrorCode::AgentNotReady
    );
    assert_eq!(drive.scroll_bottom(&terminal, &yes).await, Ok(Response::Ok));
    logged("prompt a, end").await;

    // A notice's option is its digit alone.
    std::fs::write(server.work.join("notice"), "").unwrap();
    shows("1: Show me").await;
    let notice = AgentAnswerNoticeParams {
        terminal_id: terminal.clone(),
        digit: NoticeDigit::new(1).unwrap(),
        label: Label::new("Show me".to_owned()).unwrap(),
    };
    assert_eq!(drive.answer_notice(&notice, &yes).await, Ok(Response::Ok));
    logged("prompt a, end, notice 1").await;

    stop.send_replace(true);
    task.await.unwrap();
}
