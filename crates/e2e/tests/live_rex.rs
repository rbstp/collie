#[allow(dead_code)]
mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use collie_core::{AgentState, CollieCore, CoreError, MachineFlock};
use collied::approvals::Approvals;
use collied::audit::Audit;
use collied::config::TasksConfig;
use collied::drive::{Authorized, Driver, HOSTS_AGENT};
use collied::runtime::rex::{GROUP_CLOSE, Rex};
use collied::runtime::{AgentInfo, Error, Runtime};
use collied::server::{self, ServerConfig};
use common::*;
use protocol::{
    AgentAnswerNoticeParams, AgentPromptParams, AgentSendKeysParams, AgentSlashDraftParams,
    Approval, ApprovalDecideParams, ApprovalOutcome, Decision, DraftText, ErrorCode, Event, Key,
    KeyCombo, Label, NoticeDigit, OpId, PromptText, Response, SendTextParams, SlashCommand,
    TerminalId, TerminalRunParams, TypedText,
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
    let (shell_session, shell) = new_session(server, "collie-shell", &["/bin/bash", "-c", SHELL]);
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
    // The shell is a terminal, in a group of its own session.
    let shell_group = format!("rex:{shell_session}");
    assert_eq!(
        groups,
        [
            (idle.workspace_id.as_str(), "collie-live"),
            (shell_group.as_str(), "collie-shell")
        ]
    );

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

/// A changing agent: 3,000 colored lines, more than one `format` answer keeps whole, then
/// forty more each time the test creates `more1`, `more2`, `more3` in its folder.
const STREAMED: &str = r#"$| = 1;
sub wait_for { select(undef, undef, undef, 0.05) until -e $_[0] }
printf "\e[3%dmline %d\e[0m plain \e[1mbold\e[0m\r\n", $_ % 8, $_ for 1 .. 3000;
print "\e]7501;state=idle:app=claude-code\e\\";
for my $round (1 .. 3) {
    wait_for("more$round");
    print "round $round line $_\r\n" for 1 .. 40;
}
select(undef, undef, undef, 600);
"#;

#[test]
fn live_rex_stream() {
    let Some(cli) = rex_cli() else {
        return;
    };
    let mut server = RexServer::start(&cli, "s");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(stream_scenario(&server));
    drop(rt);
    assert!(server.stop(), "dedicated Rex server did not stop");
}

/// Relays collied's connections to the scratch server from a socket of its own, to see each
/// data connection's first handshake message (1 HELLO, 2 RESUME) and to cut them as a
/// network would, with no hook in collied.
struct Relay {
    socket: std::path::PathBuf,
    /// Each data connection: its handshake, whether it is open, and how to cut it.
    data: Relayed,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Relay {
    /// collied follows only a personal server's socket: `0600` in a `0700` folder.
    fn start(server: &RexServer) -> Self {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let folder = server.root.join("relay");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&folder)
            .unwrap();
        let socket = folder.join("rex.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let data = Arc::default();
        let task = tokio::spawn({
            let (upstream, data) = (server.socket.clone(), Arc::clone(&data));
            async move {
                while let Ok((client, _)) = listener.accept().await {
                    let server = tokio::net::UnixStream::connect(&upstream).await.unwrap();
                    tokio::spawn(relay(client, server, Arc::clone(&data)));
                }
            }
        });
        Self { socket, data, task }
    }

    fn data(&self) -> Vec<(u8, bool)> {
        (self.data.lock().unwrap().iter())
            .map(|(kind, open, _)| (*kind, *open))
            .collect()
    }

    fn cut(&self) {
        for (_, open, cut) in self.data.lock().unwrap().iter_mut().filter(|c| c.1) {
            cut.abort();
            *open = false;
        }
    }
}

type Relayed = Arc<std::sync::Mutex<Vec<(u8, bool, tokio::task::AbortHandle)>>>;

async fn relay(client: tokio::net::UnixStream, server: tokio::net::UnixStream, data: Relayed) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut client_in, mut client_out) = client.into_split();
    let (mut server_in, mut server_out) = server.into_split();
    // The request, then the client's first frame (masked, under 126 bytes): its payload's
    // first byte unmasked is the handshake message's kind.
    let mut head = Vec::new();
    let end = loop {
        let mut buf = [0u8; 4096];
        let n = client_in.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        head.extend_from_slice(&buf[..n]);
        if let Some(at) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let down = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut server_in, &mut client_out).await;
    });
    let cut = down.abort_handle();
    let index = if head.starts_with(b"GET /api/data ") {
        let mut frame = head.split_off(end);
        server_out.write_all(&head).await.unwrap();
        while frame.len() < 7 {
            let mut buf = [0u8; 64];
            match client_in.read(&mut buf).await {
                Ok(n) if n > 0 => frame.extend_from_slice(&buf[..n]),
                _ => return,
            }
        }
        server_out.write_all(&frame).await.unwrap();
        let mut data = data.lock().unwrap();
        data.push((frame[6] ^ frame[2], true, cut.clone()));
        Some(data.len() - 1)
    } else {
        server_out.write_all(&head).await.unwrap();
        None
    };
    let up = tokio::io::copy(&mut client_in, &mut server_out);
    tokio::select! {
        _ = up => cut.abort(),
        _ = down => {}
    }
    if let Some(i) = index {
        data.lock().unwrap()[i].1 = false;
    }
}

/// A control connection of the test's own to the scratch server, attached to `session`.
struct Control(tokio::sync::Mutex<tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>>);

impl Control {
    async fn open(socket: &Path, session: &str) -> Self {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let stream = tokio::net::UnixStream::connect(socket).await.unwrap();
        let mut request = "ws://localhost/api/control".into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", "rex.control.v1".parse().unwrap());
        let (mut ws, _) = tokio_tungstenite::client_async(request, stream)
            .await
            .unwrap();
        let hello = serde_json::json!({
            "type": "hello", "api_revision": 13, "api_min_revision": 13,
            "client": {"kind": "other", "name": "e2e", "version": "0"},
        });
        ws.send(Message::text(hello.to_string())).await.unwrap();
        let control = Self(tokio::sync::Mutex::new(ws));
        control
            .call(
                "session.attach",
                serde_json::json!({ "session_id": session }),
            )
            .await;
        control
    }

    async fn call(&self, method: &str, payload: serde_json::Value) -> serde_json::Value {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let mut ws = self.0.lock().await;
        let request = serde_json::json!({ "type": "request", "id": method, "method": method, "payload": payload });
        ws.send(Message::text(request.to_string())).await.unwrap();
        loop {
            let Some(Ok(Message::Text(text))) = ws.next().await else {
                continue;
            };
            let reply: serde_json::Value = serde_json::from_str(&text).unwrap();
            if reply["type"] == "response" && reply["id"] == method {
                assert!(reply["error"].is_null(), "{method}: {reply}");
                return reply["payload"].clone();
            }
        }
    }

    /// The id Rex gives a `format` result it keeps, counted per block: a `format` of this
    /// terminal keeps one, so ids one apart mean nobody else formatted it in between.
    async fn last_format(&self, session: &str, block: &str) -> u64 {
        let args = serde_json::json!({ "format": "text", "max_bytes": 1 });
        let payload = serde_json::json!({ "session_id": session, "block_id": block, "args": args });
        let f = self.call("com.superlogical.terminal.format", payload).await;
        f["id"].as_u64().unwrap()
    }
}

type Reads = (String, String, String);

async fn reads(rt: &Runtime, agent: &str) -> Reads {
    (
        rt.agent_read(agent, "recent_unwrapped", Some(200))
            .await
            .unwrap()
            .text,
        rt.agent_read(agent, "visible", None).await.unwrap().text,
        rt.detection_text(agent).await.unwrap(),
    )
}

/// Reads ending with `last`, made by no `format`: from the stream.
async fn streamed(
    rt: &Runtime,
    control: &Control,
    session: &str,
    block: &str,
    last: &str,
) -> Reads {
    let agent = format!("rex:{block}");
    until(last, async || {
        let before = control.last_format(session, block).await;
        let read = reads(rt, &agent).await;
        let after = control.last_format(session, block).await;
        (after == before + 1 && read.0.ends_with(last)).then_some(read)
    })
    .await
}

async fn polled(rt: &Runtime, agent: &str, last: &str) -> Reads {
    until(last, async || {
        let read = reads(rt, agent).await;
        read.0.ends_with(last).then_some(read)
    })
    .await
}

/// A watched terminal is read from one stream shared by its watches, without `format`, and
/// reads as polling reads it: after output, after a resize by the block's owner, and after a
/// dropped connection, which resumes. The stream closes with the last watch.
async fn stream_scenario(server: &RexServer) {
    let relay = Relay::start(server);
    let (session, block) = new_session(server, "collie-stream", &["/usr/bin/perl", "-e", STREAMED]);
    let agent = format!("rex:{block}");
    let rex = Arc::new(Rex::new(relay.socket.clone()));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(rex.clone().run(stopped));
    let rt = Runtime::new(server.root.join("no-herdr.sock")).with_rex(Some(rex.clone()));
    until("the agent's idle report", async || {
        (rt.agent_get(&agent).await.ok()).filter(|a| a.agent_status == "idle")
    })
    .await;
    let control = Control::open(&server.socket, &session).await;
    let open = async |want: &[(u8, bool)]| {
        until("the data connections", async || {
            (relay.data() == want).then_some(())
        })
        .await;
    };

    let held = rt.watch(&agent).unwrap();
    let again = rt.watch(&agent).unwrap();
    let first = streamed(
        &rt,
        &control,
        &session,
        &block,
        "line 3000\u{1b}[0m plain \u{1b}[0m\u{1b}[1mbold\u{1b}[0m",
    )
    .await;
    assert!(first.0.contains("line 2801"), "{:?}", first.0);
    assert_eq!(relay.data(), [(1, true)], "one stream for both watches");
    drop(again);
    std::fs::write(server.work.join("more1"), "").unwrap();
    let round1 = streamed(&rt, &control, &session, &block, "round 1 line 40").await;
    drop(held);
    open(&[(1, false)]).await;
    assert_eq!(
        polled(&rt, &agent, "round 1 line 40").await,
        round1,
        "after output"
    );

    let held = rt.watch(&agent).unwrap();
    streamed(&rt, &control, &session, &block, "round 1 line 40").await;
    // A block takes its owner's size, so the test owns it to resize it.
    let at = serde_json::json!({ "session_id": session, "block_ids": [block], "mode": "take" });
    control.call("session.own_block", at).await;
    let size = serde_json::json!({ "session_id": session, "block_id": block, "args": { "columns": 100, "rows": 30 } });
    control.call("com.superlogical.terminal.resize", size).await;
    until("the new size", async || {
        (rt.pane_columns(&agent).await.ok()? == Some(100)).then_some(())
    })
    .await;
    std::fs::write(server.work.join("more2"), "").unwrap();
    let round2 = streamed(&rt, &control, &session, &block, "round 2 line 40").await;
    // Thirty rows: lines 12 to 40 and the cursor's empty row.
    assert!(
        round2.1.starts_with("round 2 line 12\r\n"),
        "{:?}",
        round2.1
    );
    drop(held);
    open(&[(1, false), (1, false)]).await;
    assert_eq!(
        polled(&rt, &agent, "round 2 line 40").await,
        round2,
        "after a resize"
    );

    let held = rt.watch(&agent).unwrap();
    streamed(&rt, &control, &session, &block, "round 2 line 40").await;
    relay.cut();
    std::fs::write(server.work.join("more3"), "").unwrap();
    let round3 = streamed(&rt, &control, &session, &block, "round 3 line 40").await;
    assert_eq!(
        relay.data(),
        [(1, false), (1, false), (1, false), (2, true)],
        "resumed"
    );
    drop(held);
    open(&[(1, false), (1, false), (1, false), (2, false)]).await;
    assert_eq!(
        polled(&rt, &agent, "round 3 line 40").await,
        round3,
        "after a drop"
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
    let copilot = format!("{agents}/copilot");
    let env = [
        ("COLLIE_E2E_AGENTS", agents.as_str()),
        ("CLAUDE_CONFIG_DIR", claude.as_str()),
        ("CODEX_HOME", codex.as_str()),
        ("COPILOT_HOME", copilot.as_str()),
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
    for name in ["codex", "hook", "copilot"] {
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
    std::fs::write(
        work.join("copilot-home"),
        agents.join("copilot").to_str().unwrap(),
    )
    .unwrap();
    let copilot_exe = bin.join("copilot");
    let (_, copilot) = new_session(
        server,
        "collie-copilot",
        &[
            copilot_exe.to_str().unwrap(),
            "fake_copilot",
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
    let copilot_id = format!("rex:{copilot}");
    flock_until(phone, &m, "the Rex agents", |f| {
        f.agents.len() == 3 && f.agents.iter().all(|a| a.status == AgentState::Idle)
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
        // The session can bind between history's two lookups, which asks for a retry.
        let page = match phone
            .agent_history(m.clone(), codex_id.clone(), None, None, None)
            .await
        {
            Err(CoreError::AgentNotReady) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            page => page.unwrap(),
        };
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

    let chat = chat_until(phone, &m, &copilot_id, "Copilot's marked session", |p| {
        p.available
    })
    .await;
    assert!(chat.messages.iter().any(|msg| msg.role == "user"));
    std::fs::write(work.join("copilot-new"), "").unwrap();
    let chat = chat_until(phone, &m, &copilot_id, "no session after /new", |p| {
        !p.available
    })
    .await;
    assert!((chat.unavailable_reason.unwrap_or_default()).contains("marked exactly one session"));

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
    phone.close_pane(m.clone(), copilot_id, true).await.unwrap();
    flock_until(phone, &m, "the closed agents leave", |f| {
        f.agents.is_empty()
    })
    .await;
    handle.shutdown().await;
}

#[test]
fn fake_copilot() {
    copilot_stand_in();
}

#[test]
fn fake_copilot_box() {
    copilot_box_stand_in();
}

#[test]
fn live_rex_copilot() {
    let Some(cli) = rex_cli() else {
        return;
    };
    let mut server = RexServer::start(&cli, "cp");
    let bin = server.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let copilot = bin.join("copilot");
    std::fs::copy(std::env::current_exe().unwrap(), &copilot).unwrap();
    let rt = runtime();
    rt.block_on(async {
        let argv = [
            copilot.to_str().unwrap(),
            "fake_copilot_box",
            "--exact",
            "--nocapture",
        ];
        let (_, block) = new_session(&server, "collie-copilot", &argv);
        let rex = Arc::new(Rex::new(server.socket.clone()));
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(rex.clone().run(stopped));
        tokio::time::timeout(Duration::from_secs(5), rex.settled())
            .await
            .unwrap();
        let rt = Runtime::new(server.root.join("no-herdr.sock")).with_rex(Some(rex.clone()));
        let terminal = format!("rex:{block}");
        let listed = until("the stand-in's idle report", async || {
            (rt.agent_list().await.ok()?.into_iter())
                .find(|a| a.terminal_id == terminal && a.agent_status == "idle")
        })
        .await;
        assert_eq!(
            (listed.agent.as_deref(), listed.read_only),
            (Some("copilot"), false)
        );
        let drive = Driver::new(server.root.join("no-herdr.sock"), vec![], &[])
            .unwrap()
            .with_rex(Some(rex));
        copilot_takes_input_only_under_its_box(&drive, &rt, &server.work, &terminal).await;
        stop.send(true).unwrap();
        let _ = task.await;
    });
    drop(rt);
    assert!(server.stop(), "dedicated Rex server did not stop");
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
    rt.pane_paste(&dump, "/x").await.unwrap();
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

/// herdr's and Claude Code's session variables, as a Rex server started inside a herdr pane
/// running Claude Code would hold them: a task agent must get none.
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

/// A stand-in agent CLI: it records its environment and arguments, reports OSC 7501 as `app`
/// and records each line typed into it. Perl, as one process that never execs another of the
/// same name: Rex sends no `process_changed` when the foreground process execs a program of
/// the same name, so a bash script that execs bash would keep the first one's `argv[0]`.
fn stand_in(bin: &Path, cli: &str, app: &str) {
    use std::os::unix::fs::PermissionsExt;
    let record = bin.join("record");
    let record = record.display();
    let script = format!(
        r#"#!/usr/bin/perl
$| = 1;
open(my $e, '>', '{record}/env-{cli}') or die;
print $e "$_=$ENV{{$_}}\n" for sort keys %ENV;
close $e;
open(my $a, '>', '{record}/args-{cli}') or die;
print $a "$_\n" for @ARGV;
close $a;
print "\e]7501;state=idle:app={app}\e\\";
while (my $p = <STDIN>) {{
    open(my $f, '>>', '{record}/prompts-{cli}') or die;
    print $f $p;
    close $f;
}}
select(undef, undef, undef, 600);
"#
    );
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

/// New task, a worktree task and archive with Rex alone (no herdr, no herdr-core). collied
/// finds stand-in `claude` and `codex` CLIs first on its `PATH`; the server holds dummy values
/// for every starter variable, which point nowhere, and its home's login profile puts a
/// folder of its own on `PATH`.
#[test]
fn live_rex_tasks() {
    let Some(cli) = rex_cli() else {
        return;
    };
    let (bin, path) = match std::env::var("COLLIE_E2E_BIN") {
        Ok(bin) => (bin, std::env::var("PATH").unwrap_or_default()),
        Err(_) => {
            let bin = format!("/tmp/rxt-{}", std::process::id());
            let path = format!("{bin}:{}", std::env::var("PATH").unwrap_or_default());
            (bin, path)
        }
    };
    // A scratch HOME and configuration, so the owner's git and gh setup never reach the
    // archive.
    let home = format!("{bin}/home");
    let config = format!("{bin}/home/.config");
    let gh = format!("{bin}/home/.config/gh");
    let env = [
        ("COLLIE_E2E_BIN", bin.as_str()),
        ("PATH", path.as_str()),
        ("HOME", home.as_str()),
        ("XDG_CONFIG_HOME", config.as_str()),
        ("GH_CONFIG_DIR", gh.as_str()),
    ];
    if std::env::var_os("COLLIE_E2E_BIN").is_none() {
        let _scratch = TempDir(bin.clone().into());
        in_child_with("live_rex_tasks", &env);
        return;
    }
    std::fs::create_dir_all(&gh).unwrap();
    let dummies: Vec<(&str, &str)> = STARTER.iter().map(|n| (*n, "collie-e2e")).collect();
    let mut server = RexServer::start_with(&cli, "t", &dummies);
    let rt = runtime();
    rt.block_on(tasks_scenario(&server, Path::new(&bin)));
    drop(rt);
    assert!(server.stop(), "dedicated Rex server did not stop");
}

const FULLSCREEN: &str = r#"$| = 1;
my $log = shift;
sub draw {
    my ($rows, $cols) = split ' ', `stty size`;
    open(my $f, '>>', $log) or die;
    print $f "${cols}x$rows\n";
    close $f;
    print "\e[2J";
    print "\e[$_;1Hrow $_" for 1 .. $rows - 1;
    print "\e[$rows;1H> ${cols}x$rows";
}
$SIG{WINCH} = \&draw;
print "\e[?1049h\e]7501;state=idle:app=claude-code\e\\";
draw();
select(undef, undef, undef, 600) while 1;
"#;

#[test]
fn live_rex_task_size() {
    let Some(cli) = rex_cli() else {
        return;
    };
    let mut server = RexServer::start(&cli, "z");
    runtime().block_on(task_size_scenario(&server));
    assert!(server.stop(), "dedicated Rex server did not stop");
}

async fn task_size_scenario(server: &RexServer) {
    let tasks = server.root.join("tasks.json");
    let follow = async || {
        let rex = Arc::new(Rex::new(server.socket.clone()).with_tasks(tasks.clone()));
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(rex.clone().run(stopped));
        tokio::time::timeout(Duration::from_secs(5), rex.settled())
            .await
            .unwrap();
        (rex, stop, task)
    };
    let log = server.root.join("sizes");
    let drawn = || std::fs::read_to_string(&log).unwrap_or_default();
    let fullscreen = ["/usr/bin/perl", "-e", FULLSCREEN, log.to_str().unwrap()].map(str::to_owned);
    let work = server.work.to_str().unwrap();

    let (rex, stop, task) = follow().await;
    let id = (rex.start_task(work, "size", None, None, fullscreen.to_vec()))
        .await
        .unwrap();
    until("the first draw", async || {
        (drawn() == "80x24\n").then_some(())
    })
    .await;
    stop.send(true).unwrap();
    let _ = task.await;

    let (session, block) = new_session(server, "collie-sized", &["/bin/sh", "-c", "read -r _"]);
    let app = Control::open(&server.socket, &session).await;
    let take = serde_json::json!({ "session_id": session, "block_ids": [block], "mode": "take" });
    app.call("session.own_block", take).await;
    let size = serde_json::json!({ "columns": 120, "rows": 40 });
    let resize = serde_json::json!({ "session_id": session, "block_id": block, "args": size });
    app.call("com.superlogical.terminal.resize", resize).await;
    let sized = server.rex(&["block", "call", "-b", &block, "size"]);
    assert_eq!(
        (&sized["columns"], &sized["rows"]),
        (&120.into(), &40.into())
    );

    let (rex, stop, task) = follow().await;
    until(
        "the same agent drawn again at the sized block's size",
        async || (drawn() == "80x24\n120x40\n").then_some(()),
    )
    .await;
    let _watch = rex.watch(&id).unwrap();
    until("collied serving the whole screen", async || {
        let read = rex.read(&id, "visible", None).await.ok()?.text;
        (rex.terminal_size(&id).ok() == Some((120, 40))
            && read.lines().count() == 40
            && read.lines().last() == Some("> 120x40"))
        .then_some(())
    })
    .await;
    let given = id.strip_prefix("rex:").unwrap();
    let given_up = server.rex(&["block", "call", "-b", given, "size"]);
    assert!(given_up["owner"].is_null(), "{given_up}");

    std::fs::remove_file(&log).unwrap();
    rex.start_task(work, "sized", None, None, fullscreen.to_vec())
        .await
        .unwrap();
    until("a new task started at the sized block's size", async || {
        (drawn() == "120x40\n").then_some(())
    })
    .await;
    stop.send(true).unwrap();
    let _ = task.await;

    let records: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&tasks).unwrap()).unwrap();
    let task_session = records[given]["session"].as_str().unwrap();
    let user = Control::open(&server.socket, task_session).await;
    let back = serde_json::json!({ "columns": 80, "rows": 24 });
    let back = serde_json::json!({ "session_id": task_session, "block_id": given, "args": back });
    user.call("com.superlogical.terminal.resize", back).await;
    let size = || server.rex(&["block", "call", "-b", given, "size"]);
    let picked = size();
    assert_eq!(
        (&picked["columns"], &picked["rows"]),
        (&80.into(), &24.into())
    );
    let (_rex, stop, task) = follow().await;
    let kept = size();
    assert_eq!(
        (&kept["columns"], &kept["rows"]),
        (&80.into(), &24.into()),
        "the user's size stays"
    );
    stop.send(true).unwrap();
    let _ = task.await;
}

#[test]
fn live_rex_terminals() {
    let Some(cli) = rex_cli() else {
        return;
    };
    let mut server = RexServer::start(&cli, "m");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(terminals_scenario(&server));
    drop(rt);
    assert!(server.stop(), "dedicated Rex server did not stop");
}

/// A stand-in agent run under `sh -c`, which stays in front: idle, then working once the test
/// creates `go`, then gone once it creates `quit`.
const WRAPPED: &str = r#"$| = 1;
sub wait_for { select(undef, undef, undef, 0.05) until -e $_[0] }
print "\e]7501;state=idle:app=claude-code\e\\";
wait_for("go");
print "\e]7501;state=working:app=claude-code\e\\";
wait_for("quit");
"#;

/// An interactive shell is a terminal in its session's group, stays one while it runs a
/// command, takes the phone's lines, keys and text, and is streamed while watched; a program
/// Rex runs with no shell is not listed, and neither is a shell whose `sh -c` keeps an agent
/// in front (#755). The shell is `/bin/sh -i` started directly, with the scratch home: Rex's
/// default login shell takes the account's home from `login`, and with it the user's own
/// startup files and history.
async fn terminals_scenario(server: &RexServer) {
    let (session, block) = new_session(server, "collie-term", &["/bin/sh", "-i"]);
    let (_, alone) = new_session(server, "collie-alone", &["/bin/sleep", "600"]);
    let terminal = format!("rex:{block}");

    let rex = Arc::new(Rex::new(server.socket.clone()));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(rex.clone().run(stopped));
    let rt = Runtime::new(server.root.join("no-herdr.sock")).with_rex(Some(rex.clone()));
    let drive = Arc::new(
        Driver::new(server.root.join("no-herdr.sock"), vec![], &[])
            .unwrap()
            .with_rex(Some(rex.clone())),
    );
    let listed = async || -> Option<Vec<(String, String)>> {
        let snap = rt.session_snapshot().await.ok()?;
        let terminals = collied::flock::map_terminals(&snap).into_iter();
        Some(
            terminals
                .map(|t| {
                    (
                        t.terminal_id.as_str().to_owned(),
                        t.workspace_id.as_str().to_owned(),
                    )
                })
                .collect(),
        )
    };
    let shown = until("the shell listed", async || {
        listed().await.filter(|l| !l.is_empty())
    })
    .await;
    assert_eq!(shown, [(terminal.clone(), format!("rex:{session}"))]);
    assert!(!format!("{shown:?}").contains(&alone));
    let snap = rt.session_snapshot().await.unwrap();
    let groups: Vec<&str> = snap.workspaces.iter().map(|w| w.label.as_str()).collect();
    assert_eq!(groups, ["collie-term"]);
    assert!(rt.agent_list().await.unwrap().is_empty());

    let yes: Authorized = Arc::new(|| true);
    let id = TerminalId::new(terminal.clone()).unwrap();
    let _watch = drive
        .watch_terminal(id.clone(), 200, false, yes.clone())
        .await
        .unwrap();
    let control = Control::open(&server.socket, &session).await;
    let op = || OpId::new("L".repeat(22)).unwrap();
    let line = |text: &str| TerminalRunParams {
        op_id: op(),
        terminal_id: id.clone(),
        text: PromptText::new(text.to_owned()).unwrap(),
    };
    let keys = |key: Key| AgentSendKeysParams {
        op_id: op(),
        terminal_id: id.clone(),
        keys: vec![key],
    };
    let ctrl_c = || keys(Key::Combo(KeyCombo::new("ctrl+c").unwrap()));
    let in_front = |what: &str| {
        let p = process_of(server, &session, &block);
        let fg = &p["foreground"];
        match what {
            "shell" => fg["pid"] == p["child"]["pid"],
            name => fg["argv0"] == name,
        }
    };
    until("the shell in front", async || {
        in_front("shell").then_some(())
    })
    .await;
    drive
        .terminal_run(line("echo collie-$((6*7)); sleep 600"), &yes)
        .await
        .unwrap();
    streamed(&rt, &control, &session, &block, "collie-42").await;
    until("sleep in front", async || in_front("sleep").then_some(())).await;
    assert_eq!(
        listed().await.unwrap(),
        shown,
        "a command ended the terminal"
    );
    drive.terminal_send_keys(ctrl_c(), &yes).await.unwrap();
    until("the shell in front again", async || {
        in_front("shell").then_some(())
    })
    .await;

    let typed = SendTextParams {
        op_id: op(),
        terminal_id: id.clone(),
        text: TypedText::new("echo typed-$((1+1)); sleep 600".to_owned()).unwrap(),
    };
    drive.terminal_send_text(typed, &yes).await.unwrap();
    drive
        .terminal_send_keys(keys(Key::Enter), &yes)
        .await
        .unwrap();
    streamed(&rt, &control, &session, &block, "typed-2").await;
    until("sleep in front", async || in_front("sleep").then_some(())).await;
    drive.terminal_send_keys(ctrl_c(), &yes).await.unwrap();
    until("the shell in front again", async || {
        in_front("shell").then_some(())
    })
    .await;

    // An agent under `sh -c` that keeps the terminal owns its reports through that `sh`: the
    // block is no terminal from its first report, idle included, and a line is refused as on
    // a pane hosting an agent; once it exits, the shell is listed again.
    std::fs::write(server.work.join("wrapped.pl"), WRAPPED).unwrap();
    drive
        .terminal_run(line("sh -c '/usr/bin/perl wrapped.pl; true'"), &yes)
        .await
        .unwrap();
    until("the idle agent's shell unlisted", async || {
        listed().await.filter(|l| l.is_empty())
    })
    .await;
    assert!(rt.agent_list().await.unwrap().is_empty());
    let refused = drive.terminal_run(line("yes"), &yes).await.unwrap_err();
    assert_eq!(refused, (ErrorCode::NotFound, HOSTS_AGENT.to_owned()));
    std::fs::write(server.work.join("go"), "").unwrap();
    std::fs::write(server.work.join("quit"), "").unwrap();
    let back = until("the shell listed again", async || {
        listed().await.filter(|l| !l.is_empty())
    })
    .await;
    assert_eq!(back, shown);

    stop.send_replace(true);
    task.await.unwrap();
}

fn blocks_of(server: &RexServer, session: &str) -> Vec<serde_json::Value> {
    let listed = server.rex(&["api", "call", "-s", session, "session.list_blocks"]);
    listed["blocks"].as_array().cloned().unwrap_or_default()
}

fn process_of(server: &RexServer, session: &str, block: &str) -> serde_json::Value {
    let args = serde_json::json!({ "block_id": block }).to_string();
    let method = "com.superlogical.terminal.process";
    server.rex(&["api", "call", "-s", session, method, &args])
}

fn sessions(server: &RexServer) -> Vec<String> {
    let listed = server.rex(&["api", "call", "session.list"]);
    (listed["sessions"].as_array().into_iter().flatten())
        .filter_map(|s| s["session_id"].as_str().map(str::to_owned))
        .collect()
}

async fn tasks_scenario(server: &RexServer, bin: &Path) {
    use collied::runtime::TaskHost;
    use protocol::{
        AgentKind, Cwd, RuntimeAgents, TaskArchiveParams, TaskNewParams, TaskRuntime, TaskWorktree,
        WorkspaceId,
    };
    std::fs::create_dir_all(bin.join("record")).unwrap();
    stand_in(bin, "claude", "claude-code");
    stand_in(bin, "codex", "codex");
    let record = |name: &str| std::fs::read_to_string(bin.join("record").join(name)).ok();
    // What the user's login shell reads first: whichever shell the account has.
    let profiled = server.root.join("profiled");
    let profile = format!(
        "export PATH='{}':\"$PATH\"\nexport CLAUDE_CODE_EXPERIMENTAL_CODE_MODE=0\n",
        profiled.display()
    );
    for file in [".zprofile", ".bash_profile", ".profile"] {
        std::fs::write(server.root.join("home").join(file), &profile).unwrap();
    }

    let work = std::fs::canonicalize(&server.work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["commit", "-q", "--allow-empty", "-m", "start"]);

    let rex = Arc::new(Rex::new(server.socket.clone()).with_tasks(bin.join("rex-tasks.json")));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(rex.clone().run(stopped));
    tokio::time::timeout(Duration::from_secs(5), rex.settled())
        .await
        .unwrap();
    let kinds = vec![
        AgentKind::new("claude").unwrap(),
        AgentKind::new("codex").unwrap(),
    ];
    transcripts(&bin.join("agents"), &work);
    let drive = Driver::new(
        server.root.join("no-herdr.sock"),
        kinds.clone(),
        std::slice::from_ref(&work),
    )
    .unwrap()
    .with_rex(Some(rex.clone()))
    .with_transcripts(collied::transcript::Transcripts::new(
        Some(bin.join("agents/claude/projects")),
        None,
    ));
    let yes: Authorized = Arc::new(|| true);

    let Ok(Response::TaskOptions(options)) = drive.task_options().await else {
        panic!("no task options");
    };
    assert_eq!(options.agents, kinds);
    assert_eq!(options.runtimes, Some(vec![TaskRuntime::Rex]));
    assert_eq!(
        options.runtime_agents,
        [RuntimeAgents {
            runtime: TaskRuntime::Rex,
            agents: kinds.clone(),
        }]
    );

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
        code_mode: None,
        runtime: None,
    };
    let started = |reply| match reply {
        (
            Ok(Response::TaskStarted {
                workspace_id,
                terminal_id,
                startup_blocked: false,
            }),
            _,
        ) => (workspace_id, terminal_id),
        (other, _) => panic!("task not started: {other:?}"),
    };
    let ids = |group: &WorkspaceId, agent: &TerminalId| {
        let session = group.as_str().strip_prefix("rex:").unwrap().to_owned();
        let block = agent.as_str().strip_prefix("rex:").unwrap().to_owned();
        (session, block)
    };

    let code_mode = TaskNewParams {
        code_mode: Some(true),
        ..new("claude", "fix the test", None)
    };
    let (group, claude) = started(drive.task_new(code_mode, &yes).await);
    let (session, block) = ids(&group, &claude);
    assert!(session.starts_with("session:") && block.starts_with("block:"));
    until("the prompt in Claude Code", async || {
        record("prompts-claude").filter(|p| p.contains("fix the test"))
    })
    .await;
    assert_eq!(record("args-claude").as_deref(), Some(""));
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
    let path = (env.lines())
        .find_map(|l| l.strip_prefix("PATH="))
        .unwrap()
        .to_owned();
    assert!(
        path.starts_with(&format!("{}:", profiled.display())),
        "the login profile's PATH: {path}"
    );
    assert!(env.contains(&format!("REX_BLOCK={block}")));
    assert!(
        env.lines()
            .any(|l| l == "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE=1"),
        "Code Mode wins over the login profile's"
    );
    let blocks = blocks_of(server, &session);
    assert_eq!(blocks.len(), 1, "a session of its own: {blocks:?}");
    assert!(blocks[0]["owner"].is_null(), "given up: {}", blocks[0]);
    assert_eq!(
        blocks[0]["label"],
        rex.get(claude.as_str()).unwrap().name.unwrap()
    );
    let listed = server.rex(&["api", "call", "session.list"]);
    assert!(
        (listed["sessions"].as_array().into_iter().flatten())
            .any(|s| s["session_id"] == session.as_str() && s["label"] == "e2e"),
        "{listed}"
    );

    let branch = TaskWorktree::Create {
        branch: "feat/rex-task".to_owned(),
    };
    let (group, codex) = started(
        drive
            .task_new(new("codex", "review it", Some(branch)), &yes)
            .await,
    );
    let (codex_session, codex_block) = ids(&group, &codex);
    let checkout = work.join(".worktree/feat-rex-task");
    until("the prompt in Codex", async || {
        record("prompts-codex").filter(|p| p.contains("review it"))
    })
    .await;
    assert_eq!(record("args-codex").as_deref(), Some("--no-daemon\n"));
    let env = stripped(record("env-codex").unwrap());
    assert!(
        env.lines()
            .any(|l| l == "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE=0"),
        "no Code Mode asked, the login profile's stays"
    );
    assert!(
        git(&work, &["worktree", "list", "--porcelain"])
            .contains(&format!("worktree {}\nHEAD", checkout.display()))
    );
    assert_eq!(
        process_of(server, &codex_session, &codex_block)["foreground"]["cwd"],
        checkout.to_str().unwrap()
    );

    // An argument vector reaches the program unchanged through the login shell.
    let argv = bin.join("argv");
    {
        use std::os::unix::fs::PermissionsExt;
        let script = format!(
            "#!/bin/bash\nprintf '%s\\0' \"$@\" > '{}/record/argv'\nexec sleep 600\n",
            bin.display()
        );
        std::fs::write(&argv, script).unwrap();
        std::fs::set_permissions(&argv, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let hostile = [
        "two words",
        "it's \"quoted\"",
        "; touch pwned",
        "$(touch pwned)",
        "`touch pwned`",
        "line\nbreak",
        "-n",
        "$HOME",
        "*",
    ];
    let Ok(Some(host @ TaskHost::Rex(_))) = Runtime::new(server.root.join("no-herdr.sock"))
        .with_rex(Some(rex.clone()))
        .task_host(None)
        .await
    else {
        panic!("Rex is not where tasks start");
    };
    let hostile_id = host
        .start(
            work.to_str().unwrap(),
            "collie-argv",
            None,
            "claude",
            None,
            argv.to_str().unwrap(),
            &hostile.map(String::from),
            None,
        )
        .await
        .unwrap();
    let got = until("the arguments", async || record("argv")).await;
    let got: Vec<&str> = got.strip_suffix('\0').unwrap().split('\0').collect();
    assert_eq!(got, hostile);
    assert!(!work.join("pwned").exists());
    server.rex(&["block", "close", hostile_id.strip_prefix("rex:").unwrap()]);

    let archive = |id: &TerminalId| TaskArchiveParams {
        op_id: OpId::new("A".repeat(22)).unwrap(),
        terminal_id: id.clone(),
        confirm: true,
    };
    // A shell the test starts at the repository, then moves into the checkout with `cd`: the
    // archive must see its new folder and leave the checkout alone.
    let (shell_session, shell) = new_session(server, "collie-cd", &["/bin/sh", "-i"]);
    let cd = format!("cd '{}'\n", checkout.display());
    server.rex(&["send", "-b", &shell, &cd]);
    until("the shell's new folder", async || {
        let p = process_of(server, &shell_session, &shell);
        (p["foreground"]["cwd"].as_str() == checkout.to_str()).then_some(())
    })
    .await;
    let refused = drive.task_archive(archive(&codex), &yes).await;
    let Err((_, message)) = refused else {
        panic!("archived under a shell in the checkout: {refused:?}");
    };
    assert!(message.contains("did not start"), "{message}");
    assert!(checkout.exists());
    assert_eq!(blocks_of(server, &codex_session).len(), 1);
    server.rex(&["block", "close", &shell]);
    until("the shell to close", async || {
        blocks_of(server, &shell_session).is_empty().then_some(())
    })
    .await;

    let Ok(Response::TaskArchived { message }) = drive.task_archive(archive(&codex), &yes).await
    else {
        panic!("the worktree task was not archived");
    };
    assert!(
        message.starts_with("Removed worktree checkout")
            && message.contains("Removed its Rex session."),
        "{message}"
    );
    assert!(!checkout.exists());
    assert!(!git(&work, &["worktree", "list"]).contains("feat-rex-task"));
    assert!(
        !sessions(server).contains(&codex_session),
        "the task's own empty session is removed"
    );

    // A window the user opened in the task's session keeps it.
    let window = serde_json::json!({ "layout": { "block": {
        "flavor": "com.superlogical.terminal.shell",
        "options": { "command": ["/bin/sleep", "600"], "cwd": "/" },
    }}});
    server.rex(&[
        "api",
        "call",
        "-s",
        &session,
        "session.new_window",
        &window.to_string(),
    ]);
    assert_eq!(blocks_of(server, &session).len(), 2);
    let Ok(Response::TaskArchived { message }) = drive.task_archive(archive(&claude), &yes).await
    else {
        panic!("the task was not archived");
    };
    assert!(
        message.starts_with("Closed pane.")
            && message.contains(
                "Kept its Rex session, which still holds something or could not be read."
            ),
        "{message}"
    );
    let kept = blocks_of(server, &session);
    assert_eq!(kept.len(), 1, "only the task's block closed: {kept:?}");
    assert!(sessions(server).contains(&session));
    assert!(work.exists());

    // An agent collie did not start ends alone: its session, now empty, stays.
    let (mine, block) = new_session(server, "collie-mine", &["/usr/bin/perl", "-e", AGENT]);
    let agent = TerminalId::new(format!("rex:{block}")).unwrap();
    until("the user's agent", async || rex.get(agent.as_str()).ok()).await;
    let Ok(Response::TaskArchived { message }) = drive.task_archive(archive(&agent), &yes).await
    else {
        panic!("the user's agent was not archived");
    };
    assert!(!message.contains("Rex session"), "{message}");
    assert!(blocks_of(server, &mine).is_empty());
    assert!(
        sessions(server).contains(&mine),
        "a session collie did not create stays"
    );

    // An archived task resumes, in a new session of its own.
    let resume = TaskNewParams {
        resume: Some(protocol::SessionId::new(CLOSED_SESSION).unwrap()),
        ..new("claude", "carry on", None)
    };
    let (group, resumed) = started(drive.task_new(resume, &yes).await);
    let (resumed_session, _) = ids(&group, &resumed);
    until("the resumed Claude Code", async || {
        record("prompts-claude").filter(|p| p.contains("carry on"))
    })
    .await;
    assert_eq!(
        record("args-claude").as_deref(),
        Some(format!("--resume\n{CLOSED_SESSION}\n").as_str())
    );
    let Ok(Response::TaskArchived { message }) = drive.task_archive(archive(&resumed), &yes).await
    else {
        panic!("the resumed task was not archived");
    };
    assert!(message.contains("Removed its Rex session."), "{message}");
    assert!(!sessions(server).contains(&resumed_session));

    // herdr answers, and a task asked of Rex starts there with nothing sent to herdr.
    let herdr = mock_herdr(&server.root.join("herdr.sock"), |method, _| {
        (method == "server.agent_manifests").then(|| {
            serde_json::json!({"result": {"type": "agent_manifest_status", "manifests": [
                {"agent": "claude", "source": "builtin", "source_kind": "builtin", "local_override_shadowing_remote": false},
            ]}})
        })
    });
    let beside = Driver::new(
        server.root.join("herdr.sock"),
        kinds.clone(),
        std::slice::from_ref(&work),
    )
    .unwrap()
    .with_rex(Some(rex.clone()));
    let options = match beside.task_options().await {
        Ok(Response::TaskOptions(options)) => options,
        other => panic!("no task options beside herdr: {other:?}"),
    };
    assert_eq!(
        options.runtimes,
        Some(vec![TaskRuntime::Herdr, TaskRuntime::Rex])
    );
    assert_eq!(
        options.runtime_agents,
        [
            RuntimeAgents {
                runtime: TaskRuntime::Herdr,
                agents: vec![AgentKind::new("claude").unwrap()],
            },
            RuntimeAgents {
                runtime: TaskRuntime::Rex,
                agents: kinds.clone(),
            },
        ]
    );
    let called = herdr.lock().unwrap().len();
    let asked = TaskNewParams {
        runtime: Some(TaskRuntime::Rex),
        ..new("claude", "start beside herdr", None)
    };
    let (group, chosen) = started(beside.task_new(asked, &yes).await);
    let (chosen_session, _) = ids(&group, &chosen);
    until("the prompt beside herdr", async || {
        record("prompts-claude").filter(|p| p.contains("start beside herdr"))
    })
    .await;
    assert_eq!(blocks_of(server, &chosen_session).len(), 1);
    let after = herdr.lock().unwrap()[called..].to_vec();
    assert!(
        !after.iter().any(|m| matches!(
            m.as_str(),
            "ping" | "workspace.create" | "agent.start" | "agent.prompt"
        )),
        "herdr was asked: {after:?}"
    );
    let Ok(Response::TaskArchived { .. }) = drive.task_archive(archive(&chosen), &yes).await else {
        panic!("the task started beside herdr was not archived");
    };
    assert!(!sessions(server).contains(&chosen_session));

    // A herdr that takes the connection and never answers leaves Rex to answer task.options.
    let busy = server.root.join("busy.sock");
    let listener = tokio::net::UnixListener::bind(&busy).unwrap();
    let closer = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
    let stuck = Driver::new(busy, kinds.clone(), std::slice::from_ref(&work))
        .unwrap()
        .with_rex(Some(rex.clone()));
    let Ok(Response::TaskOptions(options)) = stuck.task_options().await else {
        panic!("no task options while herdr does not answer");
    };
    assert_eq!(options.runtimes, Some(vec![TaskRuntime::Rex]));
    closer.abort();

    stop.send_replace(true);
    task.await.unwrap();
}

#[test]
fn live_rex_resume() {
    let Some(cli) = rex_cli() else {
        return;
    };
    if !Path::new("/usr/bin/sandbox-exec").is_file() {
        println!("skipped: a restoring Rex server runs only inside sandbox-exec");
        return;
    }
    let (bin, path) = match std::env::var("COLLIE_E2E_BIN") {
        Ok(bin) => (bin, std::env::var("PATH").unwrap_or_default()),
        Err(_) => {
            let bin = format!("/tmp/rxr-{}", std::process::id());
            let path = format!("{bin}:{}", std::env::var("PATH").unwrap_or_default());
            (bin, path)
        }
    };
    let home = format!("{bin}/home");
    let env = [
        ("COLLIE_E2E_BIN", bin.as_str()),
        ("PATH", path.as_str()),
        ("HOME", home.as_str()),
    ];
    if std::env::var_os("COLLIE_E2E_BIN").is_none() {
        let _scratch = TempDir(bin.clone().into());
        in_child_with("live_rex_resume", &env);
        return;
    }
    std::fs::create_dir_all(Path::new(&bin).join("record")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    stand_in(Path::new(&bin), "claude", "claude-code");
    let mut server = RexServer::start_restoring(&cli, "z", &[]);
    let rt = runtime();
    rt.block_on(resume_scenario(&mut server, Path::new(&bin)));
    drop(rt);
    assert!(server.stop(), "dedicated Rex server did not stop");
}

fn layout_of(server: &RexServer, session: &str) -> serde_json::Value {
    let view = server.rex(&["api", "call", "-s", session, "session.view"]);
    view["windows"][0]["layers"][0]["layout"].clone()
}

fn screen_of(server: &RexServer, session: &str, block: &str) -> String {
    let args = serde_json::json!({ "block_id": block, "args": { "format": "text" } }).to_string();
    let method = "com.superlogical.terminal.format";
    let text = server.rex(&["api", "call", "-s", session, method, &args]);
    text["content"].as_str().unwrap_or_default().to_owned()
}

fn block_ids(server: &RexServer, session: &str) -> Vec<String> {
    (blocks_of(server, session).iter())
        .filter_map(|b| b["block_id"].as_str().map(str::to_owned))
        .collect()
}

fn claim(sessions: &Path, pid: u64, conversation: &str, cwd: &str) {
    let claim = serde_json::json!({
        "pid": pid, "sessionId": conversation, "cwd": cwd, "startedAt": collied::now_ms(),
    });
    std::fs::write(sessions.join(format!("{pid}.json")), claim.to_string()).unwrap();
}

async fn resume_scenario(server: &mut RexServer, bin: &Path) {
    let conversation = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    let missing = "0a0a0a0a-0000-4000-8000-00000000000f";
    let rc = format!(
        "export PATH='{}':\"$PATH\"\nexport CLAUDE_CODE_EXPERIMENTAL_CODE_MODE=1\nclaude() {{ if [ \"$2\" = {missing} ]; then echo $2 > '{record}/failed'; echo \"No conversation found with session ID: $2\"; sleep 1; return 1; fi; COLLIE_WRAPPED=1 command claude \"$@\"; }}\n",
        bin.display(),
        record = bin.join("record").display(),
    );
    let zshrc = format!("PROMPT=$'%~\\n%#'\nRPROMPT='[right]'\n{rc}");
    std::fs::write(server.root.join("home/.zshrc"), zshrc).unwrap();
    std::fs::write(server.root.join("home/.zprofile"), "cd /\n").unwrap();
    std::fs::write(
        server.root.join("home/.bash_profile"),
        format!("cd /\n{rc}"),
    )
    .unwrap();
    let work = std::fs::canonicalize(&server.work).unwrap();
    let w = work.to_str().unwrap().to_owned();
    let sessions = server.root.join("claude/sessions");
    let key: String = (w.chars())
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let project = server.root.join("claude/projects").join(key);
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let record = server.root.join("rex-resume.json");
    let rex = Arc::new(
        Rex::new(server.socket.clone())
            .with_claude_sessions(Some(sessions.clone()))
            .with_resume(record.clone(), server.root.join("rex-resume-off")),
    );
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(rex.clone().run(stopped));
    tokio::time::timeout(Duration::from_secs(5), rex.settled())
        .await
        .unwrap();
    let mut started = Vec::new();
    for (label, id) in [
        ("collie-resume", conversation),
        ("collie-resume-fail", missing),
    ] {
        let (session, block) = new_session(server, label, &["/bin/zsh", "-i"]);
        std::fs::write(project.join(format!("{id}.jsonl")), "{}\n").unwrap();
        until("the two-line prompt with a right prompt", async || {
            let text = screen_of(server, &session, &block);
            (text.contains("[right]") && text.lines().count() >= 2).then_some(())
        })
        .await;
        let typed = serde_json::json!({ "block_id": block, "args": { "data": format!("{}/claude\r", bin.display()) } });
        server.rex(&[
            "api",
            "call",
            "-s",
            &session,
            "com.superlogical.terminal.write",
            &typed.to_string(),
        ]);
        let pid = until("the stand-in in front", async || {
            let p = process_of(server, &session, &block);
            (p["foreground"]["name"].as_str() != Some("zsh"))
                .then(|| p["foreground"]["pid"].as_u64())
                .flatten()
        })
        .await;
        claim(&sessions, pid, id, &w);
        started.push((session, block));
    }
    let (failing_session, failing) = started.pop().unwrap();
    let (session, block) = started.pop().unwrap();
    until("the conversations recorded", async || {
        rex.reread_sessions();
        rex.resume().await;
        let text = std::fs::read_to_string(&record).ok()?;
        let both = [block.as_str(), failing.as_str(), conversation, missing];
        both.iter().all(|s| text.contains(s)).then_some(())
    })
    .await;
    let args = bin.join("record/args-claude");
    std::fs::remove_file(&args).unwrap();

    let before = layout_of(server, &session);
    server.restart();
    let ran = until("the resumed agent's arguments", async || {
        rex.resume().await;
        std::fs::read_to_string(&args).ok()
    })
    .await;
    assert_eq!(ran, format!("--resume\n{conversation}\n"));
    let env = std::fs::read_to_string(bin.join("record/env-claude")).unwrap();
    let home = format!("HOME={}\n", server.root.join("home").display());
    assert!(
        env.contains(&home),
        "the stand-in ran with the scratch home"
    );
    let pwd = format!("PWD={w}");
    for line in [
        "COLLIE_WRAPPED=1",
        "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE=1",
        &pwd,
    ] {
        assert!(
            env.lines().any(|l| l == line),
            "the login shell's startup files ran: {line}"
        );
    }
    assert_eq!(
        block_ids(server, &session).len(),
        2,
        "the shell stays until the agent runs"
    );
    assert!(
        screen_of(server, &session, &block).contains("[right]"),
        "the restored shell"
    );
    let new = until("the resumed block", async || {
        block_ids(server, &session)
            .into_iter()
            .find(|b| *b != block)
    })
    .await;
    let resumed = until("the resumed stand-in in front", async || {
        process_of(server, &session, &new)["foreground"]["pid"].as_u64()
    })
    .await;
    assert!(env.contains(&format!("REX_BLOCK={new}\n")));
    claim(&sessions, resumed, conversation, &w);
    until("the restored shell closed", async || {
        rex.reread_sessions();
        rex.resume().await;
        (!block_ids(server, &session).contains(&block)).then_some(())
    })
    .await;
    assert_eq!(block_ids(server, &session), std::slice::from_ref(&new));
    let after = layout_of(server, &session);
    assert_eq!(
        after["block_id"],
        new.as_str(),
        "in the old block's place: {after}"
    );
    assert_eq!(before["block_id"], block.as_str());
    for _ in 0..5 {
        rex.resume().await;
    }
    assert_eq!(block_ids(server, &session).len(), 1, "once");

    until("the failing agent's resume ran", async || {
        rex.resume().await;
        std::fs::read_to_string(bin.join("record/failed")).ok()
    })
    .await;
    for _ in 0..5 {
        rex.resume().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(
        block_ids(server, &failing_session).contains(&failing),
        "the shell stays when its agent fails"
    );
    assert!(screen_of(server, &failing_session, &failing).contains("[right]"));
    stop.send_replace(true);
    task.await.unwrap();
}
