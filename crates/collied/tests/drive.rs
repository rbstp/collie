use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use collied::drive::{Authorized, Driver, Origin, Reply, Watched};
use protocol::{
    AgentKind, AgentPromptParams, AgentSendKeysParams, AgentTypeTextParams, Cwd, DraftText,
    ErrorCode, Key, Label, OpId, PaneCloseParams, PromptText, ReadParams, ReadSource, Request,
    Response, TaskNewParams, TerminalId, WorkspaceCloseParams, WorkspaceId,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

const CLAUDE: &str = "term_65ce7ae4fd5731";
const CODEX_BLOCKED: &str = "term_0a1b2c3d4e5f60";
const SHELL: &str = "term_ffffffffffff01";
const TRUST: &str = "\
────────────────────────────────────────────────────────────────────────────────
 Accessing workspace:

 /Users/me/src/new-project

 Quick safety check: Is this a project you created or one you trust?

 ❯ 1. Yes, I trust this folder
   2. No, exit

 Enter to confirm · Esc to cancel
";
const QUESTION: &str = "\
────────────────────────────────────────────────────────────────────────────────
 ☐ Storage

 Which storage backend should the cache use?

 ❯ 1. SQLite
      Embedded, no server
   2. Redis
      Shared across processes
   3. Type something.

 Enter to select · ↑/↓ to navigate · Esc to cancel
";
const BASH: &str = "\
────────────────────────────────────────────────────────────────────────────────
 Bash command

   rm -rf build
   Remove the build directory

 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don't ask again for rm commands in /Users/me/src/app
   3. No, and tell Claude what to do differently (esc)
";
const QUESTION_LIVE: &str = "\
────────────────────────────────────────────────────────────────────────────────
 ☐ Cache Backend

Which storage backend should the cache use?

❯ 1. SQLite
     Embedded, no server
  2. Redis
     Shared across processes
  3. Type something.
────────────────────────────────────────────────────────────────────────────────
  4. Chat about this

Enter to select · ↑/↓ to navigate · Esc to cancel
";
const PLAN: &str = "\
────────────────────────────────────────────────────────────────────────────────
 Would you like to proceed?

 ❯ 1. Yes, and auto-accept edits
   2. Yes, and manually approve edits
   3. No, keep planning
";
// Claude Code 2.1.289 in herdr 0.9.3 (rule legacy_no_prompt_blocker).
const PLAN_LIVE: &str = include_str!("fixtures/claude-2.1.289/plan.detection.txt");
const PLAN_TYPED_LIVE: &str =
    include_str!("fixtures/claude-2.1.289/plan-feedback-typed.detection.txt");
const MUTATING: [&str; 9] = [
    "agent.prompt",
    "agent.send_keys",
    "agent.focus",
    "agent.start",
    "workspace.create",
    "workspace.close",
    "pane.close",
    "pane.send_text",
    "pane.send_input",
];

#[derive(Default)]
struct Herdr {
    snapshot: Value,
    calls: Vec<(String, Value)>,
    text: String,
    screens: VecDeque<String>,
    manifests: Vec<String>,
    errors: HashMap<String, VecDeque<String>>,
    gets: VecDeque<Value>,
    started: Option<String>,
    shell_busy: bool,
    new_pane_terminal: Option<String>,
    rule: Option<String>,
}

struct Mock {
    state: Arc<Mutex<Herdr>>,
    socket: PathBuf,
    _dir: tempfile::TempDir,
}

impl Mock {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("herdr.sock");
        let fixture: Value =
            serde_json::from_str(include_str!("fixtures/session.snapshot.json")).unwrap();
        let state = Arc::new(Mutex::new(Herdr {
            snapshot: fixture["result"]["snapshot"].clone(),
            text: "\u{1b}[1mhi\u{1b}[0m\u{1b}]52;c;cm0gLXJmIH4=\u{7}\u{1b}[2J\r\n".into(),
            manifests: vec!["pi".into(), "claude".into()],
            ..Default::default()
        }));
        let listener = UnixListener::bind(&socket).unwrap();
        let shared = state.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let state = shared.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut line = String::new();
                    tokio::io::BufReader::new(r)
                        .read_line(&mut line)
                        .await
                        .unwrap();
                    let req: Value = serde_json::from_str(&line).unwrap();
                    let body = answer(&mut state.lock().unwrap(), &req);
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
            socket,
            _dir: dir,
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Herdr) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    fn methods(&self) -> Vec<String> {
        self.with(|h| h.calls.iter().map(|(m, _)| m.clone()).collect())
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

    fn fail_next(&self, method: &str, codes: &[&str]) {
        self.with(|h| {
            h.errors
                .entry(method.into())
                .or_default()
                .extend(codes.iter().map(|c| c.to_string()))
        });
    }

    fn mutations(&self) -> Vec<String> {
        self.methods()
            .into_iter()
            .filter(|m| MUTATING.contains(&m.as_str()))
            .collect()
    }

    fn driver(&self, agents: &[&str], root: &Path) -> Arc<Driver> {
        Arc::new(
            Driver::new(
                self.socket.clone(),
                agents.iter().map(|a| AgentKind::new(*a).unwrap()).collect(),
                &[root.to_owned()],
            )
            .unwrap(),
        )
    }
}

fn answer(h: &mut Herdr, req: &Value) -> Result<Value, String> {
    let method = req["method"].as_str().unwrap().to_owned();
    let p = req["params"].clone();
    h.calls.push((method.clone(), p.clone()));
    if let Some(code) = h.errors.get_mut(&method).and_then(VecDeque::pop_front) {
        return Err(code);
    }
    let agent_by_pane = |h: &Herdr, pane: &str| {
        h.snapshot["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["pane_id"] == pane)
            .cloned()
            .ok_or_else(|| "agent_not_found".to_owned())
    };
    let read = |h: &mut Herdr, pane: &Value| {
        let text = match h.screens.len() {
            0 => h.text.clone(),
            1 => h.screens[0].clone(),
            _ => h.screens.pop_front().unwrap(),
        };
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let keep = p["lines"].as_u64().map_or(lines.len(), |n| n as usize);
        let text = lines[lines.len().saturating_sub(keep)..].concat();
        json!({"type": "pane_read", "read": {
            "pane_id": pane, "workspace_id": "w6", "tab_id": "w6:t1", "source": p["source"],
            "format": "ansi", "text": text, "revision": 0, "truncated": true,
        }})
    };
    Ok(match method.as_str() {
        "session.snapshot" => json!({"type": "session_snapshot", "snapshot": h.snapshot}),
        "agent.list" => json!({"type": "agent_list", "agents": h.snapshot["agents"]}),
        "server.agent_manifests" => json!({
            "type": "agent_manifest_status",
            "manifests": h.manifests.iter().map(|a| json!({"agent": a, "source": "builtin", "source_kind": "builtin", "local_override_shadowing_remote": false})).collect::<Vec<_>>(),
        }),
        "agent.get" => {
            let agent = match h.gets.pop_front() {
                Some(mut a) => {
                    if a["name"] == "$started" {
                        a["name"] = json!(h.started);
                    }
                    a
                }
                None => agent_by_pane(h, p["target"].as_str().unwrap())?,
            };
            json!({"type": "agent_info", "agent": agent})
        }
        "agent.focus" => {
            json!({"type": "agent_info", "agent": agent_by_pane(h, p["target"].as_str().unwrap())?})
        }
        "agent.explain" => json!({"type": "agent_explain", "explain": {
            "matched_rule": h.rule.as_ref().map(|id| json!({"id": id, "priority": 980})),
        }}),
        "agent.read" => read(h, &p["target"]),
        "pane.read" => read(h, &p["pane_id"]),
        "agent.prompt" => {
            json!({"type": "agent_prompted", "agent": agent_by_pane(h, p["target"].as_str().unwrap()).unwrap_or(json!({}))})
        }
        "agent.send_keys" | "pane.send_text" | "workspace.close" | "pane.close" => {
            json!({"type": "ok"})
        }
        "pane.get" => {
            let pane = match p["pane_id"].as_str().unwrap() {
                "w9:p1" => json!({"pane_id": "w9:p1", "workspace_id": "w9", "tab_id": "w9:t1",
                    "terminal_id": h.new_pane_terminal.as_deref().unwrap_or("term_new"),
                    "focused": false, "agent_status": "unknown", "revision": 0}),
                id => h.snapshot["panes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|x| x["pane_id"] == id)
                    .cloned()
                    .ok_or_else(|| "pane_not_found".to_owned())?,
            };
            json!({"type": "pane_info", "pane": pane})
        }
        "pane.process_info" => {
            let (pgid, fg) = if h.shell_busy {
                (200, json!([{"pid": 200, "name": "vim"}]))
            } else {
                (100, json!([{"pid": 100, "name": "-zsh"}]))
            };
            json!({"type": "pane_process_info", "process_info": {"pane_id": p["pane_id"],
                "shell_pid": 100, "foreground_process_group_id": pgid, "foreground_processes": fg}})
        }
        "workspace.create" => json!({"type": "workspace_created",
            "workspace": {"workspace_id": "w9", "number": 3, "label": p["label"], "focused": false,
                "pane_count": 1, "tab_count": 1, "active_tab_id": "w9:t1", "agent_status": "unknown"},
            "tab": {"tab_id": "w9:t1", "workspace_id": "w9", "number": 1, "label": "1", "focused": false,
                "pane_count": 1, "agent_status": "unknown"},
            "root_pane": {"pane_id": "w9:p1", "terminal_id": "term_new", "workspace_id": "w9",
                "tab_id": "w9:t1", "focused": false, "cwd": p["cwd"], "agent_status": "unknown", "revision": 0},
        }),
        "agent.start" => {
            h.started = p["name"].as_str().map(str::to_owned);
            json!({"type": "agent_started", "argv": [p["kind"]], "agent": started_agent("unknown", false, true)})
        }
        _ => return Err("invalid_request".into()),
    })
}

fn started_agent(status: &str, ready: bool, pending: bool) -> Value {
    json!({
        "terminal_id": "term_new", "pane_id": "w9:p1", "workspace_id": "w9", "tab_id": "w9:t1",
        "focused": false, "name": "$started", "agent": "claude", "agent_status": status,
        "interactive_ready": ready, "launch_pending": pending, "revision": 0,
    })
}

fn yes() -> Authorized {
    Arc::new(|| true)
}

fn no() -> Authorized {
    Arc::new(|| false)
}

fn tid(s: &str) -> TerminalId {
    TerminalId::new(s).unwrap()
}

fn op(c: char) -> OpId {
    OpId::new(c.to_string().repeat(22)).unwrap()
}

fn code(reply: Reply) -> ErrorCode {
    reply.expect_err("expected an error").0
}

fn prompt(terminal: &str) -> AgentPromptParams {
    AgentPromptParams {
        op_id: op('P'),
        terminal_id: tid(terminal),
        text: PromptText::new("fix the build").unwrap(),
        expected_draft: None,
    }
}

const RULE: &str = "\u{1b}[38;2;136;136;136m────────────────────────────────────────\u{1b}[39m";
const PLACEHOLDER: &str = "❯ \u{1b}[0m\u{1b}[2mTry \"create a util logging.py that...\"\u{1b}[0m";

/// Claude Code's screen with `rows` in its input box.
fn screen(rows: &str) -> String {
    format!("⏺ Done.\r\n\r\n{RULE}\r\n{rows}\r\n{RULE}\r\n  Opus 5.5 high\r\n  ⏵⏵ auto mode on\r\n")
}

fn expecting(draft: &str) -> AgentPromptParams {
    AgentPromptParams {
        expected_draft: Some(DraftText::new(draft).unwrap()),
        ..prompt(CLAUDE)
    }
}

fn root() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    for d in ["root/a", "root/b", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    (dir, base)
}

fn task(cwd: &Path, agent: &str) -> TaskNewParams {
    TaskNewParams {
        op_id: op('T'),
        cwd: Cwd::new(cwd.to_str().unwrap()).unwrap(),
        agent: AgentKind::new(agent).unwrap(),
        prompt: PromptText::new("write the tests").unwrap(),
        label: Some(Label::new("tests").unwrap()),
    }
}

#[tokio::test]
async fn reads_resolve_the_pane_and_sanitize() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);

    let reply = drive
        .read(
            ReadParams {
                terminal_id: tid(CLAUDE),
                source: ReadSource::Recent,
                lines: Some(50),
            },
            true,
        )
        .await
        .unwrap();
    let Response::Terminal(read) = reply else {
        panic!("{reply:?}");
    };
    assert_eq!(read.ansi, "\u{1b}[1mhi\u{1b}[0m\r\n");
    assert!(read.truncated);
    assert_eq!(read.terminal_id.as_str(), CLAUDE);
    assert_eq!(
        herdr.params("agent.read"),
        vec![
            json!({"target": "w6:p1", "source": "recent_unwrapped", "lines": 50, "format": "ansi"})
        ]
    );

    let reply = drive
        .read(
            ReadParams {
                terminal_id: tid(SHELL),
                source: ReadSource::Visible,
                lines: None,
            },
            false,
        )
        .await;
    assert!(matches!(reply, Ok(Response::Terminal(_))));
    assert_eq!(
        herdr.params("pane.read"),
        vec![json!({"pane_id": "w7:p2", "source": "visible", "format": "ansi"})]
    );

    let not_agent = ReadParams {
        terminal_id: tid(SHELL),
        source: ReadSource::Recent,
        lines: None,
    };
    assert_eq!(code(drive.read(not_agent, true).await), ErrorCode::NotFound);
    let unknown = ReadParams {
        terminal_id: tid("term_gone"),
        source: ReadSource::Recent,
        lines: None,
    };
    assert_eq!(code(drive.read(unknown, false).await), ErrorCode::NotFound);
    assert!(herdr.mutations().is_empty());
}

#[tokio::test]
async fn prompt_rechecks_the_agent_before_writing() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    herdr.with(|h| h.text = screen(PLACEHOLDER));

    assert_eq!(drive.prompt(prompt(CLAUDE), &yes()).await, Ok(Response::Ok));
    assert_eq!(
        herdr.methods(),
        vec!["agent.list", "agent.get", "pane.read", "agent.prompt"]
    );
    assert_eq!(
        herdr.params("pane.read"),
        vec![json!({"pane_id": "w6:p1", "source": "visible", "format": "ansi"})]
    );
    assert_eq!(
        herdr.params("agent.prompt"),
        vec![json!({"target": "w6:p1", "text": "fix the build"})]
    );

    assert_eq!(
        code(drive.prompt(prompt(CODEX_BLOCKED), &yes()).await),
        ErrorCode::AgentBlocked
    );
    assert_eq!(
        code(drive.prompt(prompt(SHELL), &yes()).await),
        ErrorCode::NotFound
    );

    let mut blocked_now = herdr.with(|h| h.snapshot["agents"][0].clone());
    blocked_now["agent_status"] = json!("blocked");
    herdr.with(|h| h.gets.push_back(blocked_now));
    assert_eq!(
        code(drive.prompt(prompt(CLAUDE), &yes()).await),
        ErrorCode::AgentBlocked
    );

    let mut replaced = herdr.with(|h| h.snapshot["agents"][0].clone());
    replaced["agent_session"]["value"] = json!("11111111-0000-4000-8000-000000000000");
    herdr.with(|h| h.gets.push_back(replaced));
    assert_eq!(
        code(drive.prompt(prompt(CLAUDE), &yes()).await),
        ErrorCode::AgentNotReady
    );

    let mut pending = herdr.with(|h| h.snapshot["agents"][0].clone());
    pending["launch_pending"] = json!(true);
    herdr.with(|h| h.gets.push_back(pending));
    assert_eq!(
        code(drive.prompt(prompt(CLAUDE), &yes()).await),
        ErrorCode::AgentNotReady
    );

    let no: Authorized = Arc::new(|| false);
    assert_eq!(
        code(drive.prompt(prompt(CLAUDE), &no).await),
        ErrorCode::NotPaired
    );
    assert_eq!(herdr.params("agent.prompt").len(), 1);

    for (herdr_code, want) in [
        ("agent_blocked", ErrorCode::AgentBlocked),
        ("agent_not_ready", ErrorCode::AgentNotReady),
        ("empty_agent_prompt", ErrorCode::InvalidParams),
        ("timeout", ErrorCode::AgentNotReady),
    ] {
        herdr.fail_next("agent.prompt", &[herdr_code]);
        assert_eq!(
            code(drive.prompt(prompt(CLAUDE), &yes()).await),
            want,
            "{herdr_code}"
        );
    }
}

#[tokio::test]
async fn draft_reads_the_input_box_of_claude_only() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let draft = |t: &str| {
        let drive = drive.clone();
        let t = tid(t);
        async move { drive.draft(&t).await }
    };
    herdr.with(|h| h.text = screen("❯ one\n  two"));
    assert_eq!(
        draft(CLAUDE).await,
        Ok(Response::Draft {
            text: Some("one\ntwo".into())
        })
    );
    herdr.with(|h| h.text = screen(PLACEHOLDER));
    assert_eq!(
        draft(CLAUDE).await,
        Ok(Response::Draft {
            text: Some(String::new())
        })
    );
    herdr.with(|h| h.text = TRUST.into());
    assert_eq!(draft(CLAUDE).await, Ok(Response::Draft { text: None }));
    assert_eq!(
        draft(CODEX_BLOCKED).await,
        Ok(Response::Draft { text: None })
    );
    assert_eq!(code(draft(SHELL).await), ErrorCode::NotFound);
    assert_eq!(
        herdr.params("pane.read").len(),
        3,
        "codex's screen is not read"
    );
    assert!(herdr.mutations().is_empty());
}

#[tokio::test]
async fn prompt_replaces_the_draft_the_phone_saw() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    herdr.with(|h| {
        h.screens = [screen("❯ one  \n  two\n  three"), screen(PLACEHOLDER)].into();
    });
    assert_eq!(
        drive.prompt(expecting("one\ntwo\nthree\n"), &yes()).await,
        Ok(Response::Ok)
    );
    assert_eq!(
        herdr.methods(),
        vec![
            "agent.list",
            "agent.get",
            "pane.read",
            "agent.send_keys",
            "pane.read",
            "agent.prompt"
        ]
    );
    assert_eq!(
        herdr.params("agent.send_keys"),
        vec![json!({"target": "w6:p1", "keys": [
            "down", "down",
            "ctrl+e", "ctrl+u", "backspace",
            "ctrl+e", "ctrl+u", "backspace",
            "ctrl+e", "ctrl+u", "backspace",
        ]})]
    );
    assert_eq!(
        herdr.params("agent.prompt"),
        vec![json!({"target": "w6:p1", "text": "fix the build"})]
    );
}

#[tokio::test]
async fn prompt_refuses_a_draft_it_was_not_told_about() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    herdr.with(|h| h.text = screen("❯\u{a0}typed on the Mac"));
    for p in [prompt(CLAUDE), expecting(""), expecting("typed on the")] {
        assert_eq!(
            drive.prompt(p, &yes()).await,
            Err((ErrorCode::DraftChanged, "typed on the Mac".to_owned()))
        );
    }
    assert!(herdr.mutations().is_empty(), "{:?}", herdr.methods());

    herdr.with(|h| h.text = screen(PLACEHOLDER));
    assert_eq!(
        drive.prompt(expecting("stale"), &yes()).await,
        Ok(Response::Ok),
        "an empty box takes the prompt whatever the phone saw"
    );
    assert_eq!(herdr.mutations(), vec!["agent.prompt"]);
}

#[tokio::test]
async fn prompt_refuses_a_box_it_cannot_replace() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let tall: Vec<String> = (0..30).map(|i| format!("  row {i}")).collect();
    for (shown, why) in [
        (screen("! git push"), "bash mode"),
        (
            screen("❯ fix this [Pasted text #1 +40 lines]"),
            "collapsed paste",
        ),
        (screen("❯ [Image #1]"), "image"),
        (TRUST.to_owned(), "no input box"),
        (
            format!("{}\r\n{RULE}\r\n  ⏵⏵ auto mode on\r\n", tall.join("\r\n")),
            "top rule scrolled off",
        ),
    ] {
        herdr.with(|h| h.text = shown.clone());
        for p in [
            prompt(CLAUDE),
            expecting("git push"),
            expecting("fix this [Pasted text #1 +40 lines]"),
            expecting("[Image #1]"),
        ] {
            assert_eq!(
                code(drive.prompt(p, &yes()).await),
                ErrorCode::DraftNotCleared,
                "{why}"
            );
        }
        let t = tid(CLAUDE);
        assert_eq!(
            drive.draft(&t).await,
            Ok(Response::Draft { text: None }),
            "{why}"
        );
    }
    assert!(herdr.mutations().is_empty(), "{:?}", herdr.methods());
}

#[tokio::test]
async fn prompt_is_not_sent_when_the_draft_does_not_clear() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    herdr.with(|h| h.text = screen("❯ one\n  two"));
    let (err, message) = drive
        .prompt(expecting("one\ntwo"), &yes())
        .await
        .unwrap_err();
    assert_eq!(err, ErrorCode::DraftNotCleared);
    assert!(message.contains("nothing was sent"), "{message}");
    assert_eq!(herdr.mutations(), vec!["agent.send_keys"]);
    assert!(
        herdr.params("pane.read").len() > 2,
        "re-read until the deadline"
    );

    herdr.with(|h| {
        h.calls.clear();
        h.screens = [screen("❯ one"), TRUST.to_owned()].into();
    });
    assert_eq!(
        code(drive.prompt(expecting("one"), &yes()).await),
        ErrorCode::DraftNotCleared,
        "a dialog instead of an empty box is not cleared"
    );
    assert_eq!(herdr.mutations(), vec!["agent.send_keys"]);

    herdr.with(|h| {
        h.calls.clear();
        h.screens = [screen("❯ one"), screen(PLACEHOLDER)].into();
    });
    assert_eq!(
        code(drive.prompt(expecting("one"), &no()).await),
        ErrorCode::NotPaired
    );
    assert!(herdr.mutations().is_empty());

    let long: Vec<String> = (0..20).map(|i| format!("  line {i}")).collect();
    let rows = format!("❯ first\n{}", long.join("\n"));
    let expected = format!(
        "first\n{}",
        long.iter().map(|l| l.trim()).collect::<Vec<_>>().join("\n")
    );
    herdr.with(|h| {
        h.calls.clear();
        h.screens = [screen(&rows), screen(PLACEHOLDER)].into();
    });
    let checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = checks.clone();
    let revoked_after_first: Authorized =
        Arc::new(move || seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 1);
    assert_eq!(
        code(
            drive
                .prompt(expecting(&expected), &revoked_after_first)
                .await
        ),
        ErrorCode::NotPaired,
        "authorization is checked before every batch of keys"
    );
    let sent = herdr.params("agent.send_keys");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["keys"].as_array().unwrap().len(), 16);
    assert!(herdr.params("agent.prompt").is_empty());
}

#[tokio::test]
async fn other_kinds_never_read_the_screen() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    herdr.with(|h| {
        h.text = screen("❯ typed");
        h.snapshot["agents"][0]["agent"] = json!("pi");
    });
    let mut p = expecting("something else");
    p.op_id = op('Q');
    assert_eq!(drive.prompt(p, &yes()).await, Ok(Response::Ok));
    assert_eq!(
        herdr.methods(),
        vec!["agent.list", "agent.get", "agent.prompt"]
    );
}

#[tokio::test]
async fn op_id_replays_the_first_outcome() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    herdr.with(|h| h.text = screen(PLACEHOLDER));
    let p = prompt(CLAUDE);
    let fp = collied::drive::fingerprint(&Request::AgentPrompt(p.clone()));
    let attempt = || {
        let (d, p) = (drive.clone(), p.clone());
        async move {
            let (id, run) = (p.op_id.clone(), d.clone());
            d.once(
                "nPHONE",
                &id,
                fp,
                async move { run.prompt(p, &yes()).await },
            )
            .await
        }
    };
    assert_eq!(attempt().await, (Ok(Response::Ok), Origin::Ran));
    assert_eq!(attempt().await, (Ok(Response::Ok), Origin::Replayed));
    assert_eq!(herdr.params("agent.prompt").len(), 1);

    let mut other = p.clone();
    other.text = PromptText::new("something else").unwrap();
    let fp2 = collied::drive::fingerprint(&Request::AgentPrompt(other.clone()));
    let d = drive.clone();
    let (reply, origin) = drive
        .once("nPHONE", &other.op_id.clone(), fp2, async move {
            d.prompt(other, &yes()).await
        })
        .await;
    assert_eq!(code(reply), ErrorCode::InvalidParams);
    assert_eq!(origin, Origin::Refused);
    assert_eq!(herdr.params("agent.prompt").len(), 1);
}

#[tokio::test]
async fn send_keys_and_focus() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let keys = |terminal: &str| AgentSendKeysParams {
        op_id: op('K'),
        terminal_id: tid(terminal),
        keys: vec![
            Key::Esc,
            Key::Enter,
            Key::Up,
            Key::Down,
            Key::Tab,
            Key::ShiftTab,
            Key::CtrlC,
            Key::Y,
            Key::N,
        ],
    };
    assert_eq!(
        drive.send_keys(keys(CLAUDE), &yes()).await,
        (Ok(Response::Ok), None)
    );
    assert_eq!(
        herdr.params("agent.send_keys"),
        vec![
            json!({"target": "w6:p1", "keys": ["esc", "enter", "up", "down", "tab", "shift+tab", "ctrl+c", "y", "n"]})
        ]
    );
    assert_eq!(
        code(drive.send_keys(keys(CLAUDE), &no()).await.0),
        ErrorCode::NotPaired
    );
    assert_eq!(herdr.params("agent.send_keys").len(), 1);

    assert_eq!(
        drive.focus(&tid(CODEX_BLOCKED), &yes()).await,
        Ok(Response::Ok)
    );
    assert_eq!(
        herdr.params("agent.focus"),
        vec![json!({"target": "w7:p1"})]
    );
    assert_eq!(
        code(drive.focus(&tid("term_gone"), &yes()).await),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(drive.focus(&tid(CLAUDE), &no()).await),
        ErrorCode::NotPaired
    );
    assert_eq!(herdr.params("agent.focus").len(), 1);
}

#[tokio::test]
async fn task_options_intersects_and_filters() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let inside = base.join("root/a");
    herdr.with(|h| {
        h.snapshot["panes"][0]["cwd"] = json!(inside.join("../a"));
        h.snapshot["panes"][1]["cwd"] = json!(base.join("outside"));
    });
    let drive = herdr.driver(&["codex", "claude", "pi"], &base.join("root"));
    let Ok(Response::TaskOptions(opts)) = drive.task_options().await else {
        panic!("no options");
    };
    let names: Vec<&str> = opts.agents.iter().map(|a| a.as_str()).collect();
    assert_eq!(names, ["claude", "pi"]);
    assert_eq!(opts.default_agent.as_str(), "claude");
    let cwds: Vec<&str> = opts.recent_cwds.iter().map(|c| c.as_str()).collect();
    assert_eq!(cwds, [inside.to_str().unwrap()]);

    let only_codex = herdr.driver(&["codex"], &base);
    assert_eq!(code(only_codex.task_options().await), ErrorCode::NotFound);
}

#[tokio::test]
async fn task_new_starts_waits_and_prompts() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base.join("root"));
    herdr.fail_next("agent.start", &["agent_pane_busy"]);
    herdr.with(|h| {
        h.gets.extend([
            started_agent("unknown", false, true),
            started_agent("idle", true, false),
            started_agent("idle", true, false),
        ])
    });
    let (reply, cwd) = drive
        .task_new(task(&base.join("root/b/../a"), "claude"), &yes())
        .await;
    assert_eq!(cwd.unwrap().as_str(), base.join("root/a").to_str().unwrap());
    assert_eq!(
        reply,
        Ok(Response::TaskStarted {
            workspace_id: WorkspaceId::new("w9").unwrap(),
            terminal_id: tid("term_new"),
        })
    );
    assert_eq!(
        herdr.methods(),
        vec![
            "workspace.create",
            "agent.start",
            "pane.get",
            "agent.start",
            "agent.get",
            "agent.get",
            "agent.get",
            "agent.prompt"
        ]
    );
    assert_eq!(
        herdr.params("workspace.create"),
        vec![json!({"cwd": base.join("root/a"), "label": "tests", "focus": false})]
    );
    let start = &herdr.params("agent.start")[1];
    let name = start["name"].as_str().unwrap();
    assert!(name.len() == 15 && name.starts_with("collie-"), "{name}");
    assert_eq!(start["kind"], "claude");
    assert_eq!(start["pane_id"], "w9:p1");
    assert_eq!(herdr.params("agent.get")[0], json!({"target": name}));
    assert_eq!(herdr.params("agent.get")[2], json!({"target": name}));
    assert_eq!(
        herdr.params("agent.prompt"),
        vec![json!({"target": name, "text": "write the tests"})]
    );
}

#[tokio::test]
async fn task_new_refusals() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base.join("root"));
    let refused = [
        (task(&base.join("outside"), "claude"), "outside"),
        (task(&base.join("root/../outside"), "claude"), "outside"),
        (task(&base.join("root/missing"), "claude"), "does not exist"),
        (task(&base.join("root/a"), "codex"), "not allowed"),
    ];
    for (p, why) in refused {
        let (reply, cwd) = drive.task_new(p, &yes()).await;
        assert!(cwd.is_none());
        let (code, message) = reply.unwrap_err();
        assert_eq!(code, ErrorCode::InvalidParams);
        assert!(message.contains(why), "{message}");
    }
    assert!(herdr.methods().is_empty());

    herdr.with(|h| h.gets.push_back(started_agent("blocked", false, true)));
    let (code, message) = drive
        .task_new(task(&base.join("root/a"), "claude"), &yes())
        .await
        .0
        .unwrap_err();
    assert_eq!(code, ErrorCode::AgentBlocked);
    assert!(
        message.contains("workspace w9 was created and left open")
            && message.contains("startup prompt"),
        "{message}"
    );
    assert_eq!(herdr.mutations(), vec!["workspace.create", "agent.start"]);

    // Claude Code's folder trust question is never answered for the user, even when it
    // only shows up after the agent looked ready.
    herdr.with(|h| {
        h.calls.clear();
        h.text = TRUST.into();
        h.gets.extend([
            started_agent("idle", true, false),
            started_agent("blocked", true, false),
        ]);
    });
    let (code, message) = drive
        .task_new(task(&base.join("root/a"), "claude"), &yes())
        .await
        .0
        .unwrap_err();
    assert_eq!(code, ErrorCode::AgentBlocked);
    assert!(message.contains("trust this folder"), "{message}");
    assert_eq!(herdr.mutations(), vec!["workspace.create", "agent.start"]);
    assert_eq!(
        herdr.params("pane.read"),
        vec![json!({"pane_id": "w9:p1", "source": "detection", "format": "text"})]
    );

    herdr.with(|h| h.calls.clear());
    herdr.with(|h| h.gets.push_back(started_agent("idle", false, false)));
    let (code, message) = drive
        .task_new(task(&base.join("root/a"), "claude"), &yes())
        .await
        .0
        .unwrap_err();
    assert_eq!(code, ErrorCode::AgentNotReady);
    assert!(message.contains("exited"), "{message}");
    assert!(!herdr.methods().contains(&"agent.prompt".to_owned()));

    herdr.with(|h| h.calls.clear());
    herdr.fail_next("agent.start", &["unsupported_agent_kind"]);
    let (code, message) = drive
        .task_new(task(&base.join("root/a"), "claude"), &yes())
        .await
        .0
        .unwrap_err();
    assert_eq!(code, ErrorCode::Internal);
    assert!(message.contains("left open"), "{message}");
    assert_eq!(herdr.mutations(), vec!["workspace.create", "agent.start"]);

    herdr.with(|h| h.calls.clear());
    let checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = checks.clone();
    // Allows workspace.create and agent.start, then revokes before the prompt.
    let auth: Authorized =
        Arc::new(move || seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2);
    herdr.with(|h| {
        h.gets.extend([
            started_agent("idle", true, false),
            started_agent("idle", true, false),
        ])
    });
    let (code, _) = drive
        .task_new(task(&base.join("root/a"), "claude"), &auth)
        .await
        .0
        .unwrap_err();
    assert_eq!(code, ErrorCode::NotPaired);
    assert_eq!(checks.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(herdr.mutations(), vec!["workspace.create", "agent.start"]);

    // A busy pane is retried while it holds the new terminal, whatever runs in its
    // foreground (rc files run helpers there while the shell starts)...
    herdr.with(|h| {
        h.calls.clear();
        h.shell_busy = true;
        h.new_pane_terminal = None;
        h.gets.extend([
            started_agent("idle", true, false),
            started_agent("idle", true, false),
        ]);
    });
    herdr.fail_next("agent.start", &["agent_pane_busy"]);
    let (reply, _) = drive
        .task_new(task(&base.join("root/a"), "claude"), &yes())
        .await;
    assert!(reply.is_ok(), "{reply:?}");
    assert_eq!(
        herdr.mutations()[..3],
        ["workspace.create", "agent.start", "agent.start"]
    );
    // ...and not once the pane holds another terminal.
    herdr.with(|h| {
        h.calls.clear();
        h.shell_busy = false;
        h.new_pane_terminal = Some("term_other".to_owned());
    });
    herdr.fail_next("agent.start", &["agent_pane_busy"]);
    let (code, message) = drive
        .task_new(task(&base.join("root/a"), "claude"), &yes())
        .await
        .0
        .unwrap_err();
    assert_eq!(code, ErrorCode::AgentNotReady);
    assert!(message.contains("the pane is busy"), "{message}");
    assert_eq!(herdr.mutations(), vec!["workspace.create", "agent.start"]);
}

#[tokio::test]
async fn codex_and_copilot_startup_prompts_are_left_to_the_terminal() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude", "codex", "copilot"], &base.join("root"));
    for (kind, text) in [
        ("codex", include_str!("fixtures/codex/trust.detection.txt")),
        (
            "copilot",
            include_str!("fixtures/copilot/trust.detection.txt"),
        ),
    ] {
        herdr.with(|h| {
            h.calls.clear();
            h.text = text.into();
            let mut blocked = started_agent("blocked", false, true);
            blocked["agent"] = json!(kind);
            h.gets.push_back(blocked);
        });
        let (code, message) = drive
            .task_new(task(&base.join("root/a"), kind), &yes())
            .await
            .0
            .unwrap_err();
        assert_eq!(code, ErrorCode::AgentBlocked);
        assert!(message.contains("answer it in the terminal"), "{message}");
        assert_eq!(herdr.params("agent.start")[0]["kind"], kind);
        assert_eq!(herdr.mutations(), vec!["workspace.create", "agent.start"]);
        assert!(herdr.params("pane.read").is_empty());
    }
}

#[tokio::test]
async fn a_codex_startup_prompt_herdr_does_not_rule_blocked_gets_no_prompt() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude", "codex"], &base.join("root"));
    let mut unknown = started_agent("unknown", true, false);
    unknown["agent"] = json!("codex");
    herdr.with(|h| {
        h.text = include_str!("fixtures/codex/update.detection.txt").into();
        h.gets.extend([unknown.clone(), unknown.clone()]);
    });
    let (code, message) = drive
        .task_new(task(&base.join("root/a"), "codex"), &yes())
        .await
        .0
        .unwrap_err();
    assert_eq!(code, ErrorCode::AgentBlocked);
    assert!(message.contains("answer it in the terminal"), "{message}");
    assert_eq!(herdr.mutations(), vec!["workspace.create", "agent.start"]);

    herdr.with(|h| {
        h.calls.clear();
        h.text = "› Explain this codebase\n\n  100% context left · ? for shortcuts\n".into();
        h.gets.extend([unknown.clone(), unknown]);
    });
    let (reply, _) = drive
        .task_new(task(&base.join("root/a"), "codex"), &yes())
        .await;
    assert!(reply.is_ok(), "{reply:?}");
    assert_eq!(
        herdr.params("pane.read"),
        vec![json!({"pane_id": "w9:p1", "source": "detection", "format": "text"})]
    );
    assert_eq!(
        herdr.mutations(),
        vec!["workspace.create", "agent.start", "agent.prompt"]
    );
}

#[tokio::test]
async fn closes_need_confirmation() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let ws = |confirm| WorkspaceCloseParams {
        workspace_id: WorkspaceId::new("w7").unwrap(),
        confirm,
    };
    let pane = |confirm| PaneCloseParams {
        terminal_id: tid(SHELL),
        confirm,
    };
    assert_eq!(
        code(drive.workspace_close(ws(false), &yes()).await),
        ErrorCode::ConfirmRequired
    );
    assert_eq!(
        code(drive.pane_close(pane(false), &yes()).await),
        ErrorCode::ConfirmRequired
    );
    assert!(herdr.methods().is_empty());
    assert_eq!(
        code(drive.workspace_close(ws(true), &no()).await),
        ErrorCode::NotPaired
    );
    assert_eq!(
        code(drive.pane_close(pane(true), &no()).await),
        ErrorCode::NotPaired
    );
    assert!(herdr.mutations().is_empty());

    assert_eq!(
        drive.workspace_close(ws(true), &yes()).await,
        Ok(Response::Ok)
    );
    assert_eq!(drive.pane_close(pane(true), &yes()).await, Ok(Response::Ok));
    assert_eq!(
        herdr.params("workspace.close"),
        vec![json!({"workspace_id": "w7"})]
    );
    assert_eq!(
        herdr.params("pane.close"),
        vec![json!({"pane_id": "w7:p2"})]
    );

    herdr.fail_next("workspace.close", &["workspace_not_found"]);
    assert_eq!(
        code(drive.workspace_close(ws(true), &yes()).await),
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn watch_pushes_changes_only_and_ends_when_the_agent_goes() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    assert!(matches!(
        drive.watch(tid("term_gone"), 200).await,
        Err((ErrorCode::NotFound, _))
    ));

    let mut watcher = drive.watch(tid(CLAUDE), 200).await.unwrap();
    let Ok(Some(Watched::Output(first))) = next(&mut watcher).await else {
        panic!("no first output");
    };
    assert_eq!(first.ansi, "\u{1b}[1mhi\u{1b}[0m\r\n");
    assert_eq!(first.source, ReadSource::Recent);
    let reads = herdr.params("agent.read");
    assert_eq!(
        reads[0],
        json!({"target": "w6:p1", "source": "recent_unwrapped", "lines": 200, "format": "ansi"})
    );

    assert!(
        tokio::time::timeout(Duration::from_millis(700), watcher.recv())
            .await
            .is_err(),
        "unchanged output was pushed again"
    );
    assert!(herdr.params("agent.read").len() >= 3);

    herdr.with(|h| h.text = "next\r\n".into());
    let Ok(Some(Watched::Output(second))) = next(&mut watcher).await else {
        panic!("no second output");
    };
    assert_eq!(second.ansi, "next\r\n");

    let history = "\u{1b}[1mline\u{1b}[0m\r\n".repeat(100);
    herdr.with(|h| h.text = format!("{history}a\r\n"));
    let Ok(Some(Watched::Output(full))) = next(&mut watcher).await else {
        panic!("a mostly new screen is sent whole");
    };
    herdr.with(|h| h.text = format!("{history}b\r\n"));
    let Ok(Some(Watched::Patch(patch))) = next(&mut watcher).await else {
        panic!("unchanged history is not sent again");
    };
    assert_eq!((patch.skip, patch.keep), (0, 100));
    assert_eq!(patch.tail, ["b\r", ""]);
    assert_eq!(
        patch.apply(&full.ansi).unwrap().ansi,
        format!("{history}b\r\n")
    );

    herdr.with(|h| {
        h.snapshot["agents"].as_array_mut().unwrap().remove(0);
    });
    assert!(matches!(next(&mut watcher).await, Ok(Some(Watched::Gone))));
    assert!(matches!(next(&mut watcher).await, Ok(None)));

    let watcher = drive.watch(tid(CODEX_BLOCKED), 200).await.unwrap();
    drop(watcher);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let reads = herdr.params("agent.read").len();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        herdr.params("agent.read").len(),
        reads,
        "watch kept polling"
    );
}

#[tokio::test]
async fn a_reply_over_herdrs_line_limit_is_read_with_fewer_lines() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    // About 2.2 KB of JSON per line, so 500 lines are still over 1 MiB.
    let line = format!("{}\r\n", "\u{1b}[31mx".repeat(200));
    herdr.with(|h| h.text = line.repeat(1000));

    let mut watcher = drive.watch(tid(CLAUDE), 1000).await.unwrap();
    let Ok(Some(Watched::Output(read))) = next(&mut watcher).await else {
        panic!("no output");
    };
    assert_eq!(read.ansi, line.repeat(250));
    herdr.with(|h| h.text = line.repeat(999) + "y\r\n");
    let Ok(Some(Watched::Patch(_))) = next(&mut watcher).await else {
        panic!("no patch");
    };
    drop(watcher);
    let depths: Vec<Value> = herdr
        .params("agent.read")
        .iter()
        .map(|p| p["lines"].clone())
        .collect();
    assert_eq!(
        depths[..4],
        [json!(1000), json!(500), json!(250), json!(250)]
    );

    let reply = drive
        .read(
            ReadParams {
                terminal_id: tid(CLAUDE),
                source: ReadSource::Recent,
                lines: Some(1000),
            },
            true,
        )
        .await;
    let Ok(Response::Terminal(read)) = reply else {
        panic!("{reply:?}");
    };
    assert_eq!(read.ansi.lines().count(), 250);
}

async fn next(
    w: &mut collied::drive::Watcher,
) -> Result<Option<Watched>, tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_secs(2), w.recv()).await
}

fn block(herdr: &Mock, text: &str) {
    herdr.with(|h| {
        h.snapshot["agents"][0]["agent_status"] = json!("blocked");
        h.text = text.into();
        h.rule = Some("live_blocked_form".into());
    });
}

fn typed(terminal: &str, text: &str) -> AgentTypeTextParams {
    AgentTypeTextParams {
        op_id: op('Y'),
        terminal_id: tid(terminal),
        text: PromptText::new(text).unwrap(),
    }
}

fn keys(terminal: &str) -> AgentSendKeysParams {
    AgentSendKeysParams {
        op_id: op('K'),
        terminal_id: tid(terminal),
        keys: vec![Key::Down, Key::Enter],
    }
}

#[tokio::test]
async fn keys_reach_a_blocked_agent_only_on_a_question() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);

    block(&herdr, QUESTION);
    assert_eq!(
        drive.send_keys(keys(CLAUDE), &yes()).await,
        (
            Ok(Response::Ok),
            Some(format!("{CLAUDE} blocked keys=down,enter"))
        )
    );
    assert_eq!(
        herdr.methods(),
        [
            "agent.list",
            "agent.get",
            "agent.explain",
            "pane.read",
            "agent.send_keys"
        ]
    );
    assert_eq!(
        herdr.params("pane.read")[0],
        json!({"pane_id": "w6:p1", "source": "detection", "format": "text"})
    );
    block(&herdr, QUESTION_LIVE);
    assert_eq!(
        drive.send_keys(keys(CLAUDE), &yes()).await.0,
        Ok(Response::Ok)
    );

    for screen in [BASH, PLAN, TRUST] {
        block(&herdr, screen);
        let (reply, target) = drive.send_keys(keys(CLAUDE), &yes()).await;
        assert_eq!(code(reply), ErrorCode::AgentBlocked, "{screen}");
        assert_eq!(target, None);
    }
    assert_eq!(
        code(drive.send_keys(keys(CLAUDE), &no()).await.0),
        ErrorCode::AgentBlocked,
        "refused before authorization is even needed"
    );

    block(&herdr, QUESTION);
    assert_eq!(
        code(drive.send_keys(keys(CLAUDE), &no()).await.0),
        ErrorCode::NotPaired
    );

    let wrapped = BASH.replace(
        "   2. Yes, and don't ask again for rm commands in /Users/me/src/app\n",
        "   2. Yes, and don't ask again for rm commands in\n      /a\n      /b\n      /c\n      /d\n",
    );
    for (rule, screen) in [
        (Some("bash_permission_prompt"), QUESTION),
        (Some("bash_permission_prompt"), "Do you want to proceed?"),
        (Some("live_blocked_form"), wrapped.as_str()),
        (None, QUESTION),
    ] {
        herdr.with(|h| {
            h.rule = rule.map(str::to_owned);
            h.text = screen.into();
        });
        assert_eq!(
            code(drive.send_keys(keys(CLAUDE), &yes()).await.0),
            ErrorCode::AgentBlocked,
            "{rule:?} {screen}"
        );
    }
    herdr.with(|h| {
        h.rule = Some("live_strong_blocker".into());
        h.text = "Allow command `make`? [y/n]".into();
    });
    assert_eq!(
        code(drive.send_keys(keys(CODEX_BLOCKED), &yes()).await.0),
        ErrorCode::AgentBlocked,
        "no menu on screen is no licence: only Claude Code question forms take keys"
    );
    assert_eq!(herdr.params("agent.send_keys").len(), 2);
    assert!(herdr.params("agent.prompt").is_empty());
    block(&herdr, QUESTION);
    assert_eq!(
        code(drive.prompt(prompt(CLAUDE), &yes()).await),
        ErrorCode::AgentBlocked,
        "a prompt is still refused while blocked"
    );
}

#[tokio::test]
async fn typed_text_goes_into_the_question_field_then_enter() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let on_field = QUESTION_LIVE
        .replace("❯ 1. SQLite", "  1. SQLite")
        .replace("  3. Type something.", "❯ 3. Type something.");
    let filled = on_field.replace("❯ 3. Type something.", "❯ 3. DuckDB");

    block(&herdr, QUESTION_LIVE);
    herdr.with(|h| h.screens = [QUESTION_LIVE.to_owned(), on_field.clone(), filled.clone()].into());
    assert_eq!(
        drive.type_text(typed(CLAUDE, "DuckDB"), &yes()).await,
        Ok(Response::Ok)
    );
    assert_eq!(
        herdr.methods(),
        [
            "agent.list",
            "agent.get",
            "agent.explain",
            "pane.read",
            "agent.send_keys",
            "agent.get",
            "pane.read",
            "pane.send_text",
            "agent.get",
            "pane.read",
            "agent.send_keys"
        ]
    );
    assert_eq!(
        herdr.params("pane.send_text"),
        [json!({"pane_id": "w6:p1", "text": "DuckDB"})]
    );
    assert_eq!(
        herdr.params("agent.send_keys"),
        [
            json!({"target": "w6:p1", "keys": ["down", "down"]}),
            json!({"target": "w6:p1", "keys": ["enter"]})
        ]
    );

    herdr.with(|h| h.screens = [on_field.clone(), on_field.clone()].into());
    assert_eq!(
        code(drive.type_text(typed(CLAUDE, "DuckDB"), &yes()).await),
        ErrorCode::AgentNotReady,
        "the field never showed the text"
    );
    assert_eq!(herdr.params("pane.send_text").len(), 2);
    assert_eq!(herdr.params("agent.send_keys").len(), 2, "no Enter");

    herdr.with(|h| h.screens = [QUESTION_LIVE.to_owned()].into());
    assert_eq!(
        code(drive.type_text(typed(CLAUDE, "DuckDB"), &yes()).await),
        ErrorCode::AgentNotReady,
        "the cursor did not reach the field"
    );
    assert_eq!(
        herdr.params("pane.send_text").len(),
        2,
        "nothing typed under SQLite"
    );
    assert_eq!(herdr.params("agent.send_keys").len(), 3);

    let blocked = herdr.with(|h| h.snapshot["agents"][0].clone());
    let mut moved_on = blocked.clone();
    moved_on["state_change_seq"] = json!(6);
    herdr.with(|h| {
        h.screens = [on_field.clone(), filled.clone()].into();
        h.gets.extend([blocked, moved_on]);
    });
    assert_eq!(
        code(drive.type_text(typed(CLAUDE, "DuckDB"), &yes()).await),
        ErrorCode::AgentNotReady
    );
    assert_eq!(herdr.params("pane.send_text").len(), 3);
    assert_eq!(
        herdr.params("agent.send_keys").len(),
        3,
        "no Enter once the prompt changed"
    );

    let checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = checks.clone();
    let revoked_after_typing: Authorized =
        Arc::new(move || seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0);
    herdr.with(|h| h.screens = [on_field.clone(), filled.clone()].into());
    assert_eq!(
        code(
            drive
                .type_text(typed(CLAUDE, "DuckDB"), &revoked_after_typing)
                .await
        ),
        ErrorCode::NotPaired
    );
    assert_eq!(herdr.params("pane.send_text").len(), 4);
    assert_eq!(herdr.params("agent.send_keys").len(), 3);

    herdr.with(|h| h.screens.clear());
    for screen in [BASH, PLAN, TRUST] {
        block(&herdr, screen);
        assert_eq!(
            code(drive.type_text(typed(CLAUDE, "1"), &yes()).await),
            ErrorCode::AgentBlocked,
            "{screen}"
        );
    }
    block(&herdr, "Pick a name\nEnter to confirm · Esc to cancel");
    assert_eq!(
        code(drive.type_text(typed(CLAUDE, "DuckDB"), &yes()).await),
        ErrorCode::AgentBlocked,
        "no text field to type into"
    );

    herdr.with(|h| h.snapshot["agents"][0]["agent_status"] = json!("working"));
    assert_eq!(
        code(drive.type_text(typed(CLAUDE, "DuckDB"), &yes()).await),
        ErrorCode::AgentNotReady,
        "not blocked: a prompt is the way"
    );
    assert_eq!(herdr.params("pane.send_text").len(), 4);
    assert_eq!(herdr.params("agent.send_keys").len(), 3);
}

#[tokio::test]
async fn plan_feedback_is_typed_but_keys_are_refused() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let on_field = PLAN_LIVE
        .replace(
            "   ❯ 1. Yes, and use auto mode",
            "     1. Yes, and use auto mode",
        )
        .replace(
            "     3. Tell Claude what to change",
            "   ❯ 3. Tell Claude what to change",
        );
    let plan = |rule: &str| {
        block(&herdr, PLAN_LIVE);
        herdr.with(|h| h.rule = Some(rule.into()));
    };

    plan("legacy_no_prompt_blocker");
    assert_eq!(
        code(drive.send_keys(keys(CLAUDE), &yes()).await.0),
        ErrorCode::AgentBlocked,
        "a plan takes no keys"
    );
    herdr.with(|h| {
        h.screens = [
            PLAN_LIVE.to_owned(),
            on_field.clone(),
            PLAN_TYPED_LIVE.to_owned(),
        ]
        .into()
    });
    assert_eq!(
        drive
            .type_text(typed(CLAUDE, "use echo instead"), &yes())
            .await,
        Ok(Response::Ok)
    );
    assert_eq!(
        herdr.params("agent.send_keys"),
        [
            json!({"target": "w6:p1", "keys": ["down", "down"]}),
            json!({"target": "w6:p1", "keys": ["enter"]})
        ],
        "never shift+tab"
    );
    assert_eq!(
        herdr.params("pane.send_text"),
        [json!({"pane_id": "w6:p1", "text": "use echo instead"})]
    );

    herdr.with(|h| h.screens = [PLAN_LIVE.to_owned(), on_field.clone(), on_field.clone()].into());
    assert_eq!(
        code(
            drive
                .type_text(typed(CLAUDE, "use echo instead"), &yes())
                .await
        ),
        ErrorCode::AgentNotReady,
        "the field never showed the text"
    );
    assert_eq!(herdr.params("agent.send_keys").len(), 3, "no Enter");

    herdr.with(|h| h.screens.clear());
    for rule in ["live_blocked_form", "bash_permission_prompt"] {
        plan(rule);
        assert_eq!(
            code(
                drive
                    .type_text(typed(CLAUDE, "use echo instead"), &yes())
                    .await
            ),
            ErrorCode::AgentBlocked,
            "{rule}"
        );
    }
    plan("legacy_no_prompt_blocker");
    herdr.with(|h| h.text = PLAN_LIVE.replace("2. Yes, manually approve edits", "2. Yes"));
    assert_eq!(
        code(
            drive
                .type_text(typed(CLAUDE, "use echo instead"), &yes())
                .await
        ),
        ErrorCode::AgentBlocked,
        "a menu with a decision is a permission prompt"
    );
    assert_eq!(herdr.params("pane.send_text").len(), 2);
    assert_eq!(herdr.params("agent.send_keys").len(), 3);
}
