use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use collied::drive::{Authorized, Driver, Origin, Reply, Watched};
use protocol::{
    AgentKind, AgentPromptParams, AgentSendKeysParams, Cwd, ErrorCode, Key, Label, OpId,
    PaneCloseParams, PromptText, ReadParams, ReadSource, Request, Response, TaskNewParams,
    TerminalId, WorkspaceCloseParams, WorkspaceId,
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
    manifests: Vec<String>,
    errors: HashMap<String, VecDeque<String>>,
    gets: VecDeque<Value>,
    started: Option<String>,
    shell_busy: bool,
    new_pane_terminal: Option<String>,
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
    let read = |h: &Herdr, pane: &Value| {
        json!({"type": "pane_read", "read": {
            "pane_id": pane, "workspace_id": "w6", "tab_id": "w6:t1", "source": p["source"],
            "format": "ansi", "text": h.text, "revision": 0, "truncated": true,
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
        "agent.read" => read(h, &p["target"]),
        "pane.read" => read(h, &p["pane_id"]),
        "agent.prompt" => {
            json!({"type": "agent_prompted", "agent": agent_by_pane(h, p["target"].as_str().unwrap()).unwrap_or(json!({}))})
        }
        "agent.send_keys" | "workspace.close" | "pane.close" => json!({"type": "ok"}),
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

    assert_eq!(drive.prompt(prompt(CLAUDE), &yes()).await, Ok(Response::Ok));
    assert_eq!(
        herdr.methods(),
        vec!["agent.list", "agent.get", "agent.prompt"]
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
async fn op_id_replays_the_first_outcome() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
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
        Ok(Response::Ok)
    );
    assert_eq!(
        herdr.params("agent.send_keys"),
        vec![
            json!({"target": "w6:p1", "keys": ["esc", "enter", "up", "down", "tab", "shift+tab", "ctrl+c", "y", "n"]})
        ]
    );
    assert_eq!(
        code(drive.send_keys(keys(CODEX_BLOCKED), &yes()).await),
        ErrorCode::AgentBlocked
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
            "pane.process_info",
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

    // A busy pane is retried only while it holds the new terminal and its shell is starting.
    for (busy, moved) in [(true, false), (false, true)] {
        herdr.with(|h| {
            h.calls.clear();
            h.shell_busy = busy;
            h.new_pane_terminal = moved.then(|| "term_other".to_owned());
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
        drive.watch(tid("term_gone")).await,
        Err((ErrorCode::NotFound, _))
    ));

    let mut watcher = drive.watch(tid(CLAUDE)).await.unwrap();
    let Ok(Some(Watched::Output(first))) = next(&mut watcher).await else {
        panic!("no first output");
    };
    assert_eq!(first.ansi, "\u{1b}[1mhi\u{1b}[0m\r\n");
    assert_eq!(first.source, ReadSource::Recent);
    let reads = herdr.params("agent.read");
    assert_eq!(
        reads[0],
        json!({"target": "w6:p1", "source": "recent_unwrapped", "lines": 240, "format": "ansi"})
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

    herdr.with(|h| {
        h.snapshot["agents"].as_array_mut().unwrap().remove(0);
    });
    assert!(matches!(next(&mut watcher).await, Ok(Some(Watched::Gone))));
    assert!(matches!(next(&mut watcher).await, Ok(None)));

    let watcher = drive.watch(tid(CODEX_BLOCKED)).await.unwrap();
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

async fn next(
    w: &mut collied::drive::Watcher,
) -> Result<Option<Watched>, tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_secs(2), w.recv()).await
}
