use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use collied::drive::{Authorized, Driver, Origin, Reply, Watched};
use protocol::{
    AgentAnswerNoticeParams, AgentKind, AgentPromptParams, AgentSendKeysParams,
    AgentSlashDraftParams, AgentTypeTextParams, Cwd, DraftText, ErrorCode, FolderName, Key, Label,
    NoticeDigit, OpId, PaneCloseParams, PromptText, ReadParams, ReadSource, Request, Response,
    SlashCommand, TaskArchiveParams, TaskNewParams, TaskWorktree, TaskWorktreesParams, TerminalId,
    TerminalRunParams, WorkspaceCloseParams, WorkspaceId,
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
const QUESTION: &str = include_str!("fixtures/claude/question.txt");
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
const PLAN: &str = include_str!("fixtures/claude/plan.txt");
const CODEX_QUEUED_QUESTION: &str = "Working\nQueued follow-up inputs\n  ? 1 question\n    ⇧← to answer\n› Ask Codex to do anything\n⚠ 1 warning · f2 to view\n";
const CODEX_QUESTION_OPEN: &str = "Which database should we use?\n\n  › 1. SQLite\n    2. Redis\n    3. Other\n\n  enter submit   ⌃] skip   ⇧→ main prompt\n";
// Claude Code 2.1.289 in herdr 0.9.3 (rule legacy_no_prompt_blocker).
const PLAN_LIVE: &str = include_str!("fixtures/claude-2.1.289/plan.detection.txt");
const PLAN_TYPED_LIVE: &str =
    include_str!("fixtures/claude-2.1.289/plan-feedback-typed.detection.txt");
// Claude Code 2.1.293 fullscreen in herdr 0.9.3, scrolled up with Page Up.
const SCROLLED: &str = include_str!("fixtures/claude-2.1.293/scrolled-idle.detection.txt");
const SCROLLED_DIALOG: &str =
    include_str!("fixtures/claude-2.1.293/scrolled-dialog-hidden.detection.txt");
const NEW_MESSAGE: &str = include_str!("fixtures/claude-2.1.293/new-message.detection.txt");
const BOTTOM: &str = include_str!("fixtures/claude-2.1.293/bottom-idle.detection.txt");
// Claude Code 2.1.293 notices above the input box; .ansi.txt are visible reads, where the
// placeholder is dim.
const SURVEY: &str = include_str!("fixtures/claude-2.1.293/survey.ansi.txt");
const SURVEY_NARROW: &str = include_str!("fixtures/claude-2.1.293/survey-narrow.detection.txt");
const SURVEY_STARTUP: &str = include_str!("fixtures/claude-2.1.293/survey-startup.detection.txt");
const SURVEY_TYPED: &str = include_str!("fixtures/claude-2.1.293/survey-digit-typed.detection.txt");
const SURVEY_SCROLLED: &str = include_str!("fixtures/claude-2.1.293/survey-scrolled.detection.txt");
const HEADS_UP: &str = include_str!("fixtures/claude-2.1.293/heads-up.ansi.txt");
const HEADS_UP_EXPLAINED: &str =
    include_str!("fixtures/claude-2.1.293/heads-up-explained.detection.txt");
const HEADS_UP_EXPLAINED_SURVEY: &str =
    include_str!("fixtures/claude-2.1.293/heads-up-explained-survey.detection.txt");
const HEADS_UP_DISMISSED: &str =
    include_str!("fixtures/claude-2.1.293/heads-up-dismissed.detection.txt");
const HEADS_UP_EXPLAINED_DISMISSED: &str =
    include_str!("fixtures/claude-2.1.293/heads-up-explained-dismissed.detection.txt");
const HEADS_UP_INTERNAL: &str =
    include_str!("fixtures/claude-2.1.293/heads-up-internal.detection.txt");
const YOU_SHOULD_KNOW: &str = include_str!("fixtures/claude-2.1.293/you-should-know.detection.txt");
const HEADS_UP_TITLED: &str = include_str!("fixtures/claude-2.1.293/heads-up-titled.detection.txt");
const HEADS_UP_HINT: &str = include_str!("fixtures/claude-2.1.293/heads-up-hint.detection.txt");
const HEADS_UP_EXPLAINED_HINT: &str =
    include_str!("fixtures/claude-2.1.293/heads-up-explained-hint.detection.txt");
const SLASH_MENU: &str = include_str!("fixtures/claude-2.1.293/slash-menu.ansi.txt");
const SLASH_TAB_HINT: &str = include_str!("fixtures/claude-2.1.293/slash-tab-hint.ansi.txt");
// Pasted with the command menu open; slash-statstatu is the box a later mirror pasted into
// before the earlier paste had rendered.
const SLASH_STA: &str = include_str!("fixtures/claude-2.1.293/slash-sta.ansi.txt");
const SLASH_STAT: &str = include_str!("fixtures/claude-2.1.293/slash-stat.ansi.txt");
const SLASH_STATU: &str = include_str!("fixtures/claude-2.1.293/slash-statu.ansi.txt");
const SLASH_STATSTATU: &str = include_str!("fixtures/claude-2.1.293/slash-statstatu.ansi.txt");
const MUTATING: [&str; 13] = [
    "agent.prompt",
    "agent.send_keys",
    "agent.focus",
    "agent.start",
    "workspace.create",
    "worktree.create",
    "worktree.open",
    "worktree.remove",
    "workspace.close",
    "pane.close",
    "pane.send_text",
    "pane.send_input",
    "pane.send_keys",
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
    new_pane_terminal: Option<String>,
    rule: Option<String>,
    slow: Option<Duration>,
    columns: Option<u16>,
    zoomed: bool,
    worktree_path: Option<String>,
    worktree_open: bool,
    worktree_workspace_id: Option<String>,
    pane_occupied: bool,
    disconnect_after_remove: bool,
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
                    let slow = state.lock().unwrap().slow;
                    if let Some(delay) = slow {
                        tokio::time::sleep(delay).await;
                    }
                    let body = answer(&mut state.lock().unwrap(), &req);
                    if req["method"] == "worktree.remove"
                        && state.lock().unwrap().disconnect_after_remove
                    {
                        return;
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
        "pane.layout" => {
            let pane = match h.columns {
                Some(_) => p["pane_id"].clone(),
                None => json!("w1:p1"),
            };
            json!({"type": "pane_layout", "layout": {"workspace_id": "w6", "tab_id": "w6:t1",
                "zoomed": h.zoomed, "focused_pane_id": pane, "splits": [], "panes": [
                {"pane_id": "w6:p9", "focused": false, "rect": {"x": 0, "y": 0, "width": 30, "height": 60}},
                {"pane_id": pane, "focused": true, "rect": {"x": 31, "y": 0, "width": h.columns.unwrap_or(80), "height": 60}}]}})
        }
        "agent.prompt" => {
            json!({"type": "agent_prompted", "agent": agent_by_pane(h, p["target"].as_str().unwrap()).unwrap_or(json!({}))})
        }
        "agent.send_keys" | "pane.send_text" | "pane.send_input" | "pane.send_keys"
        | "workspace.close" | "pane.close" => {
            json!({"type": "ok"})
        }
        "pane.get" => {
            let pane = match p["pane_id"].as_str().unwrap() {
                "w9:p1" => json!({"pane_id": "w9:p1", "workspace_id": "w9", "tab_id": "w9:t1",
                    "terminal_id": h.new_pane_terminal.as_deref().unwrap_or("term_new"),
                    "focused": false, "agent": if h.pane_occupied { Some("claude") } else { None },
                    "agent_status": "unknown", "revision": 0}),
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
        "workspace.create" => json!({"type": "workspace_created",
            "workspace": {"workspace_id": "w9", "number": 3, "label": p["label"], "focused": false,
                "pane_count": 1, "tab_count": 1, "active_tab_id": "w9:t1", "agent_status": "unknown"},
            "tab": {"tab_id": "w9:t1", "workspace_id": "w9", "number": 1, "label": "1", "focused": false,
                "pane_count": 1, "agent_status": "unknown"},
            "root_pane": {"pane_id": "w9:p1", "terminal_id": "term_new", "workspace_id": "w9",
                "tab_id": "w9:t1", "focused": false, "cwd": p["cwd"], "agent_status": "unknown", "revision": 0},
        }),
        "worktree.list" => json!({"type": "worktree_list",
            "source": {"repo_key": "repo", "repo_name": "repo", "repo_root": p["cwd"],
                "source_checkout_path": p["cwd"]},
            "worktrees": h.worktree_path.iter().map(|path| json!({
                "path": path, "branch": "feature", "label": "feature", "is_bare": false,
                "is_detached": false, "is_prunable": false, "is_linked_worktree": true,
                "open_workspace_id": if h.worktree_open { h.worktree_workspace_id.as_deref().or(Some("w9")) } else { None }
            })).collect::<Vec<_>>()
        }),
        "worktree.create" | "worktree.open" => {
            if method == "worktree.create" {
                std::fs::create_dir_all(p["path"].as_str().unwrap()).unwrap();
            }
            json!({"type": if method == "worktree.create" { "worktree_created" } else { "worktree_opened" },
                "workspace": {"workspace_id": "w9", "number": 3, "label": "feature", "focused": false,
                    "pane_count": 1, "tab_count": 1, "active_tab_id": "w9:t1", "agent_status": "unknown"},
                "tab": {"tab_id": "w9:t1", "workspace_id": "w9", "number": 1, "label": "1", "focused": false,
                    "pane_count": 1, "agent_status": "unknown"},
                "root_pane": {"pane_id": "w9:p1", "terminal_id": "term_new", "workspace_id": "w9",
                    "tab_id": "w9:t1", "focused": false, "cwd": p["path"],
                    "agent": if h.pane_occupied { Some("claude") } else { None },
                    "agent_status": "unknown", "revision": 0},
                "worktree": {"path": p["path"], "branch": p["branch"].as_str().unwrap_or("feature"), "label": "feature",
                    "is_bare": false, "is_detached": false, "is_prunable": false,
                    "is_linked_worktree": true},
                "already_open": h.worktree_open
            })
        }
        "worktree.remove" => {
            let path = h.worktree_path.as_deref().unwrap_or("/missing").to_owned();
            if h.disconnect_after_remove {
                let repo = h.snapshot["workspaces"][1]["worktree"]["repo_root"]
                    .as_str()
                    .unwrap();
                assert!(
                    std::process::Command::new("git")
                        .args(["-C", repo, "worktree", "remove", "--force", &path])
                        .status()
                        .unwrap()
                        .success()
                );
                h.snapshot["workspaces"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|workspace| workspace["workspace_id"] != p["workspace_id"]);
            }
            json!({"type": "worktree_removed",
                "workspace_id": p["workspace_id"], "force": p["force"],
                "worktree": {"path": path, "branch": "feature", "open_workspace_id": null,
                    "is_bare": false, "is_linked_worktree": true}
            })
        }
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
        new_folder: None,
        worktree: None,
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

    let pane = |terminal: &str| ReadParams {
        terminal_id: tid(terminal),
        source: ReadSource::Visible,
        lines: None,
    };
    assert!(matches!(
        drive.read(pane(CODEX_BLOCKED), false).await,
        Ok(Response::Terminal(_))
    ));
    assert_eq!(
        herdr.params("pane.read"),
        vec![json!({"pane_id": "w7:p1", "source": "visible", "format": "ansi"})]
    );
    assert_eq!(
        code(drive.read(pane(SHELL), false).await),
        ErrorCode::NotFound,
        "a shell is read through terminal.watch, under a grant"
    );
    assert_eq!(herdr.params("pane.read").len(), 1);

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
        vec![
            "agent.list",
            "agent.get",
            "pane.read",
            "pane.read",
            "agent.prompt"
        ]
    );
    assert_eq!(
        herdr.params("pane.read"),
        vec![
            json!({"pane_id": "w6:p1", "source": "detection", "format": "text"}),
            json!({"pane_id": "w6:p1", "source": "visible", "format": "ansi"})
        ]
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
        let draft = screen("❯ one  \n  two\n  three");
        h.screens = [draft.clone(), draft, screen(PLACEHOLDER)].into();
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
        h.screens = [screen("❯ one"), screen("❯ one"), TRUST.to_owned()].into();
    });
    assert_eq!(
        code(drive.prompt(expecting("one"), &yes()).await),
        ErrorCode::DraftNotCleared,
        "a dialog instead of an empty box is not cleared"
    );
    assert_eq!(herdr.mutations(), vec!["agent.send_keys"]);

    herdr.with(|h| {
        h.calls.clear();
        h.screens = [screen("❯ one"), screen("❯ one"), screen(PLACEHOLDER)].into();
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
        h.screens = [screen(&rows), screen(&rows), screen(PLACEHOLDER)].into();
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
        ],
    };
    assert_eq!(
        drive.send_keys(keys(CLAUDE), &yes()).await,
        (Ok(Response::Ok), None)
    );
    assert_eq!(
        herdr.params("agent.send_keys"),
        vec![
            json!({"target": "w6:p1", "keys": ["esc", "enter", "up", "down", "tab", "shift+tab", "ctrl+c"]})
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
async fn scroll_bottom_sends_ctrl_end_only_while_claude_shows_the_banner() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    for screen in [SCROLLED, SCROLLED_DIALOG, NEW_MESSAGE] {
        herdr.with(|h| h.text = screen.into());
        assert_eq!(
            drive.scroll_bottom(&tid(CLAUDE), &yes()).await,
            Ok(Response::Ok)
        );
    }
    assert_eq!(
        herdr.params("pane.send_text"),
        vec![json!({"pane_id": "w6:p1", "text": "\u{1b}[1;5F"}); 3]
    );
    assert_eq!(
        herdr.params("pane.read")[0],
        json!({"pane_id": "w6:p1", "source": "detection", "format": "text"})
    );

    let mut blocked = herdr.with(|h| h.snapshot["agents"][0].clone());
    blocked["agent_status"] = json!("blocked");
    herdr.with(|h| h.gets.push_back(blocked));
    assert_eq!(
        drive.scroll_bottom(&tid(CLAUDE), &yes()).await,
        Ok(Response::Ok)
    );
    assert_eq!(herdr.mutations(), vec!["pane.send_text"; 4]);

    herdr.with(|h| h.text = BOTTOM.into());
    assert_eq!(
        code(drive.scroll_bottom(&tid(CLAUDE), &yes()).await),
        ErrorCode::AgentNotReady
    );
    herdr.with(|h| h.text = SCROLLED.into());
    assert_eq!(
        drive.scroll_bottom(&tid(CODEX_BLOCKED), &yes()).await,
        Err((ErrorCode::AgentNotReady, "not a Claude Code agent".into()))
    );
    let mut other = herdr.with(|h| h.snapshot["agents"][0].clone());
    other["agent_session"]["value"] = json!("11111111-0000-4000-8000-000000000000");
    herdr.with(|h| h.gets.push_back(other));
    assert_eq!(
        code(drive.scroll_bottom(&tid(CLAUDE), &yes()).await),
        ErrorCode::AgentNotReady
    );
    assert_eq!(
        code(drive.scroll_bottom(&tid("term_gone"), &yes()).await),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(drive.scroll_bottom(&tid(CLAUDE), &no()).await),
        ErrorCode::NotPaired
    );
    assert_eq!(herdr.mutations().len(), 4);
}

#[tokio::test]
async fn answer_notice_sends_one_digit_only_while_claude_shows_the_notice() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let answer = |terminal: &str, d: u8, label: &str| AgentAnswerNoticeParams {
        terminal_id: tid(terminal),
        digit: NoticeDigit::new(d).unwrap(),
        label: Label::new(label).unwrap(),
    };
    let filled = screen("❯ /plugin disable cc-plugin-you-should-know@builtin");
    let mut sent = Vec::new();
    for (shown, options) in [
        (
            SURVEY,
            [(1, "Bad"), (2, "Fine"), (3, "Good"), (0, "Dismiss")].as_slice(),
        ),
        (
            HEADS_UP,
            &[(1, "Learn more"), (2, "Knew this already"), (0, "Dismiss")],
        ),
        (
            HEADS_UP_EXPLAINED,
            &[
                (1, "Understood"),
                (2, "Chat in main session"),
                (0, "Dismiss"),
            ],
        ),
        (
            HEADS_UP_DISMISSED,
            &[
                (1, "That was helpful"),
                (2, "Not relevant"),
                (3, "Couldn\u{2019}t understand"),
                (4, "Turn off suggestions"),
                (0, "Dismiss"),
            ],
        ),
        (
            HEADS_UP_EXPLAINED_DISMISSED,
            &[
                (1, "That was helpful"),
                (2, "Didn\u{2019}t understand"),
                (0, "Dismiss"),
            ],
        ),
        (
            HEADS_UP_INTERNAL,
            &[
                (1, "Learn more"),
                (2, "Knew this already"),
                (3, "What is this"),
                (4, "Disable"),
                (0, "Dismiss"),
            ],
        ),
        (
            YOU_SHOULD_KNOW,
            &[(1, "Learn more"), (2, "Knew this already"), (0, "Dismiss")],
        ),
        (
            HEADS_UP_TITLED,
            &[(1, "Learn more"), (2, "Knew this already"), (0, "Dismiss")],
        ),
        (
            HEADS_UP_HINT,
            &[(1, "Learn more"), (2, "Knew this already"), (0, "Dismiss")],
        ),
        (
            HEADS_UP_EXPLAINED_HINT,
            &[
                (1, "Understood"),
                (2, "Chat in main session"),
                (0, "Dismiss"),
            ],
        ),
    ] {
        herdr.with(|h| h.text = shown.into());
        for &(d, label) in options {
            // The call returns once the box is no longer empty.
            herdr.with(|h| {
                h.calls.clear();
                h.screens = [shown.into(), shown.into(), filled.clone()].into();
            });
            assert_eq!(
                drive.answer_notice(&answer(CLAUDE, d, label), &yes()).await,
                Ok(Response::Ok)
            );
            let methods = [
                "agent.list",
                "agent.get",
                "pane.read",
                "pane.read",
                "pane.send_text",
                "pane.read",
            ];
            let mut reads = vec![
                json!({"pane_id": "w6:p1", "source": "detection", "format": "text"}),
                json!({"pane_id": "w6:p1", "source": "visible", "format": "ansi"}),
            ];
            reads.push(reads[1].clone());
            assert_eq!(herdr.methods(), methods, "{label}");
            assert_eq!(herdr.params("pane.read"), reads, "{label}");
            sent.extend(herdr.params("pane.send_text"));
            herdr.with(|h| h.screens.clear());
        }
    }
    let texts: Vec<&str> = sent.iter().map(|p| p["text"].as_str().unwrap()).collect();
    assert_eq!(
        texts,
        [
            "1", "2", "3", "0", "1", "2", "0", "1", "2", "0", "1", "2", "3", "4", "0", "1", "2",
            "0", "1", "2", "3", "4", "0", "1", "2", "0", "1", "2", "0", "1", "2", "0", "1", "2",
            "0"
        ]
    );
    assert!(sent.iter().all(|p| p["pane_id"] == "w6:p1"));

    herdr.with(|h| h.calls.clear());
    let none = "no notice with that option";
    let typed = "the input box is not empty";
    let scrolled = "scrolled up on the machine; jump to the bottom first";
    for (screen, d, label, message) in [
        (HEADS_UP, 3, "What is this", none),
        (HEADS_UP, 2, "Chat in main session", none),
        (HEADS_UP_EXPLAINED, 1, "Learn more", none),
        (HEADS_UP_EXPLAINED, 2, "Knew this already", none),
        (HEADS_UP_EXPLAINED, 3, "Understood", none),
        (HEADS_UP_EXPLAINED_SURVEY, 1, "Understood", none),
        (HEADS_UP, 4, "Turn off suggestions", none),
        (HEADS_UP_INTERNAL, 4, "Turn off suggestions", none),
        (HEADS_UP_DISMISSED, 4, "Disable", none),
        (HEADS_UP_DISMISSED, 3, "Couldn't understand", none),
        (HEADS_UP_EXPLAINED_DISMISSED, 2, "Not relevant", none),
        (
            HEADS_UP_EXPLAINED_DISMISSED,
            3,
            "Couldn\u{2019}t understand",
            none,
        ),
        (YOU_SHOULD_KNOW, 3, "Learn more", none),
        (SURVEY, 1, "Good", none),
        (SURVEY_NARROW, 1, "Bad", none),
        (BOTTOM, 1, "Bad", none),
        (SURVEY_TYPED, 0, "Dismiss", typed),
        (SURVEY_SCROLLED, 1, "Bad", scrolled),
    ] {
        herdr.with(|h| h.text = screen.into());
        assert_eq!(
            drive.answer_notice(&answer(CLAUDE, d, label), &yes()).await,
            Err((ErrorCode::AgentNotReady, message.into())),
            "{d} {label} on {screen}"
        );
    }
    assert!(herdr.mutations().is_empty());

    // The notice and the box come from the visible read, the last before the write.
    let placeholder = SURVEY.replacen("\n❯\u{a0}\n", &format!("\n{PLACEHOLDER}\n"), 1);
    assert_ne!(placeholder, SURVEY);
    herdr.with(|h| h.screens = [SURVEY_STARTUP.to_owned(), placeholder].into());
    assert_eq!(
        drive
            .answer_notice(&answer(CLAUDE, 3, "Good"), &yes())
            .await,
        Ok(Response::Ok)
    );
    let sent = herdr.params("pane.send_text");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["text"], "3");
    herdr.with(|h| {
        h.calls.clear();
        h.screens = [HEADS_UP.to_owned(), HEADS_UP_EXPLAINED.to_owned()].into();
    });
    assert_eq!(
        drive
            .answer_notice(&answer(CLAUDE, 2, "Knew this already"), &yes())
            .await,
        Err((ErrorCode::AgentNotReady, none.into()))
    );
    assert!(herdr.mutations().is_empty());
    herdr.with(|h| h.screens.clear());

    herdr.with(|h| h.text = SURVEY.into());
    let mut blocked = herdr.with(|h| h.snapshot["agents"][0].clone());
    blocked["agent_status"] = json!("blocked");
    herdr.with(|h| h.gets.push_back(blocked));
    assert_eq!(
        code(drive.answer_notice(&answer(CLAUDE, 1, "Bad"), &yes()).await),
        ErrorCode::AgentBlocked
    );
    herdr.with(|h| h.snapshot["agents"][1]["agent_status"] = json!("idle"));
    assert_eq!(
        drive
            .answer_notice(&answer(CODEX_BLOCKED, 1, "Bad"), &yes())
            .await,
        Err((ErrorCode::AgentNotReady, "not a Claude Code agent".into()))
    );
    let mut other = herdr.with(|h| h.snapshot["agents"][0].clone());
    other["agent_session"]["value"] = json!("11111111-0000-4000-8000-000000000000");
    herdr.with(|h| h.gets.push_back(other));
    assert_eq!(
        code(drive.answer_notice(&answer(CLAUDE, 1, "Bad"), &yes()).await),
        ErrorCode::AgentNotReady
    );
    assert_eq!(
        code(
            drive
                .answer_notice(&answer("term_gone", 1, "Bad"), &yes())
                .await
        ),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(drive.answer_notice(&answer(CLAUDE, 1, "Bad"), &no()).await),
        ErrorCode::NotPaired
    );
    assert!(herdr.mutations().is_empty());
}

#[tokio::test]
async fn a_prompt_waits_for_the_box_a_notice_answer_fills() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let off = AgentAnswerNoticeParams {
        terminal_id: tid(CLAUDE),
        digit: NoticeDigit::new(4).unwrap(),
        label: Label::new("Turn off suggestions").unwrap(),
    };
    let command = "/plugin disable cc-plugin-you-should-know@builtin";
    let echo = HEADS_UP_DISMISSED.replacen("\n❯\n", "\n❯\u{a0}4\n", 1);
    assert_ne!(echo, HEADS_UP_DISMISSED);
    // Claude Code shows the digit for 400 ms, clears the box, then fills it; the prompt queued
    // on the lock sees the fill, not the digit or the empty box before it.
    herdr.with(|h| {
        h.screens = [
            HEADS_UP_DISMISSED.into(),
            HEADS_UP_DISMISSED.into(),
            echo.clone(),
            echo.clone(),
            echo.clone(),
            HEADS_UP_DISMISSED.into(),
            screen(&format!("❯\u{a0}{command}")),
        ]
        .into();
    });
    let auth = yes();
    let (answered, prompted) = tokio::join!(
        drive.answer_notice(&off, &auth),
        drive.prompt(expecting(""), &auth)
    );
    assert_eq!(answered, Ok(Response::Ok));
    assert_eq!(prompted, Err((ErrorCode::DraftChanged, command.to_owned())));
    assert_eq!(herdr.mutations(), ["pane.send_text"]);

    // A fill that never shows keeps the lock for a second; the digit stands.
    herdr.with(|h| {
        h.calls.clear();
        h.screens.clear();
        h.text = HEADS_UP_DISMISSED.into();
    });
    assert_eq!(drive.answer_notice(&off, &yes()).await, Ok(Response::Ok));
    assert_eq!(herdr.mutations(), ["pane.send_text"]);
    assert!(herdr.params("pane.read").len() > 3);

    // Nor does a digit no notice took.
    herdr.with(|h| {
        h.calls.clear();
        h.screens = [HEADS_UP_DISMISSED.into(), HEADS_UP_DISMISSED.into(), echo].into();
    });
    assert_eq!(drive.answer_notice(&off, &yes()).await, Ok(Response::Ok));
    assert_eq!(herdr.mutations(), ["pane.send_text"]);
    assert!(herdr.params("pane.read").len() > 3);
}

fn slash(terminal: &str, command: &str, expected: Option<&str>) -> AgentSlashDraftParams {
    AgentSlashDraftParams {
        terminal_id: tid(terminal),
        command: SlashCommand::new(command).unwrap(),
        expected_draft: expected.map(|d| DraftText::new(d).unwrap()),
    }
}

#[tokio::test]
async fn slash_draft_pastes_the_command_into_an_empty_box_only() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    herdr.with(|h| {
        h.screens = [
            screen(PLACEHOLDER),
            screen(PLACEHOLDER),
            screen(PLACEHOLDER),
            screen(PLACEHOLDER),
            screen("❯ /s"),
        ]
        .into()
    });
    assert_eq!(
        drive.slash_draft(slash(CLAUDE, "/s", None), &yes()).await,
        Ok(Response::Ok)
    );
    assert_eq!(
        herdr.methods(),
        [
            "agent.list",
            "agent.get",
            "pane.read",
            "pane.read",
            "agent.list",
            "agent.get",
            "pane.read",
            "pane.read",
            "pane.send_text",
            "pane.read"
        ]
    );
    let reads: Vec<Value> = herdr
        .params("pane.read")
        .into_iter()
        .map(|p| p["source"].clone())
        .collect();
    assert_eq!(
        reads,
        ["detection", "visible", "detection", "visible", "visible"]
    );
    assert_eq!(
        herdr.params("pane.send_text"),
        [json!({"pane_id": "w6:p1", "text": "\u{1b}[200~/s\u{1b}[201~"})]
    );

    // The token on the Mac is cleared, then the new one pasted; an empty command only clears.
    for (command, sent) in [("/sk", Some("\u{1b}[200~/sk\u{1b}[201~")), ("", None)] {
        herdr.with(|h| {
            h.calls.clear();
            h.screens = [
                SLASH_MENU.into(),
                SLASH_MENU.into(),
                screen(PLACEHOLDER),
                screen(PLACEHOLDER),
                screen(PLACEHOLDER),
                screen("❯ /sk"),
            ]
            .into();
        });
        assert_eq!(
            drive
                .slash_draft(slash(CLAUDE, command, Some("/s")), &yes())
                .await,
            Ok(Response::Ok),
            "{command}"
        );
        assert_eq!(
            herdr.params("agent.send_keys"),
            [json!({"target": "w6:p1", "keys": ["ctrl+e", "ctrl+u", "backspace"]})]
        );
        let pasted: Vec<Value> = herdr
            .params("pane.send_text")
            .into_iter()
            .map(|p| p["text"].clone())
            .collect();
        assert_eq!(
            pasted,
            sent.map(|s| json!(s)).into_iter().collect::<Vec<_>>()
        );
        assert!(herdr.params("agent.prompt").is_empty());
    }

    // What Tab completed reads back with its hint; the phone names it to replace it.
    herdr.with(|h| {
        h.calls.clear();
        h.screens = [
            SLASH_TAB_HINT.into(),
            SLASH_TAB_HINT.into(),
            screen(PLACEHOLDER),
        ]
        .into();
    });
    assert_eq!(
        drive
            .slash_draft(slash(CLAUDE, "", Some("/rename  [name]")), &yes())
            .await,
        Ok(Response::Ok)
    );
    assert_eq!(herdr.mutations(), ["agent.send_keys"]);
    herdr.with(|h| h.screens.clear());
}

#[tokio::test]
async fn slash_draft_returns_only_once_the_box_shows_the_command() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let empty = || screen(PLACEHOLDER);
    // Shorter and longer tokens with the menu open; the read right after the paste is stale.
    for (shown, expected, command, after) in [
        (SLASH_STAT, "/stat", "/sta", SLASH_STA),
        (SLASH_STA, "/sta", "/statu", SLASH_STATU),
        (SLASH_STATU, "/statu", "/stat", SLASH_STAT),
    ] {
        herdr.with(|h| {
            h.calls.clear();
            h.screens = [
                shown.into(),
                shown.into(),
                empty(),
                empty(),
                empty(),
                empty(),
                after.into(),
            ]
            .into();
        });
        assert_eq!(
            drive
                .slash_draft(slash(CLAUDE, command, Some(expected)), &yes())
                .await,
            Ok(Response::Ok),
            "{command}"
        );
        assert_eq!(
            herdr.mutations(),
            ["agent.send_keys", "pane.send_text"],
            "{command}"
        );
        let methods = herdr.methods();
        let pasted = methods.iter().position(|m| m == "pane.send_text").unwrap();
        assert_eq!(
            methods[pasted + 1..],
            ["pane.read", "pane.read"],
            "{command}"
        );
        assert!(herdr.with(|h| h.screens.len() == 1), "{command}");
    }

    // Clearing to empty waits for the empty box before it returns.
    herdr.with(|h| {
        h.calls.clear();
        h.screens = [
            SLASH_STAT.into(),
            SLASH_STAT.into(),
            SLASH_STAT.into(),
            empty(),
        ]
        .into();
    });
    assert_eq!(
        drive
            .slash_draft(slash(CLAUDE, "", Some("/stat")), &yes())
            .await,
        Ok(Response::Ok)
    );
    assert_eq!(herdr.mutations(), ["agent.send_keys"]);
    assert_eq!(herdr.params("pane.read").len(), 4);

    // A box that never shows the command fails closed, and nothing more is written.
    for (after, err) in [
        (
            empty(),
            (
                ErrorCode::AgentNotReady,
                "the input box did not show the command".to_owned(),
            ),
        ),
        (
            TRUST.to_owned(),
            (
                ErrorCode::AgentNotReady,
                "the input box did not show the command".to_owned(),
            ),
        ),
        (
            SLASH_STATSTATU.to_owned(),
            (ErrorCode::DraftChanged, "/stat/statu".to_owned()),
        ),
    ] {
        herdr.with(|h| {
            h.calls.clear();
            h.screens = [empty(), empty(), empty(), empty(), after].into();
        });
        assert_eq!(
            drive
                .slash_draft(slash(CLAUDE, "/statu", None), &yes())
                .await,
            Err(err)
        );
        assert_eq!(herdr.mutations(), ["pane.send_text"]);
    }
    herdr.with(|h| h.screens.clear());
}

#[tokio::test]
async fn slash_draft_refusals_write_nothing() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    herdr.with(|h| h.text = SLASH_MENU.into());
    for expected in [None, Some("/x"), Some("")] {
        assert_eq!(
            drive
                .slash_draft(slash(CLAUDE, "/sk", expected), &yes())
                .await,
            Err((ErrorCode::DraftChanged, "/s".to_owned())),
            "{expected:?}"
        );
    }
    for (shown, why) in [
        (screen("! git push"), "bash mode"),
        (screen("❯ /s [Pasted text #1 +40 lines]"), "collapsed paste"),
        (TRUST.to_owned(), "no input box"),
    ] {
        herdr.with(|h| h.text = shown);
        assert_eq!(
            code(
                drive
                    .slash_draft(slash(CLAUDE, "/s", Some("git push")), &yes())
                    .await
            ),
            ErrorCode::DraftNotCleared,
            "{why}"
        );
    }
    let scrolled = "scrolled up on the machine; jump to the bottom first";
    herdr.with(|h| h.text = SCROLLED.into());
    assert_eq!(
        drive.slash_draft(slash(CLAUDE, "/s", None), &yes()).await,
        Err((ErrorCode::AgentNotReady, scrolled.into()))
    );
    herdr.with(|h| h.text = screen(PLACEHOLDER));
    let mut blocked = herdr.with(|h| h.snapshot["agents"][0].clone());
    blocked["agent_status"] = json!("blocked");
    herdr.with(|h| h.gets.push_back(blocked.clone()));
    assert_eq!(
        code(drive.slash_draft(slash(CLAUDE, "/s", None), &yes()).await),
        ErrorCode::AgentBlocked
    );
    herdr.with(|h| h.snapshot["agents"][1]["agent_status"] = json!("idle"));
    assert_eq!(
        drive
            .slash_draft(slash(CODEX_BLOCKED, "/s", None), &yes())
            .await,
        Err((ErrorCode::AgentNotReady, "not a Claude Code agent".into()))
    );
    assert_eq!(
        code(drive.slash_draft(slash(CLAUDE, "/s", None), &no()).await),
        ErrorCode::NotPaired
    );
    assert!(herdr.mutations().is_empty(), "{:?}", herdr.methods());

    // Re-checked after the clear, just before the paste.
    let idle = herdr.with(|h| h.snapshot["agents"][0].clone());
    let typed = "the input box is not empty";
    let cleared = || {
        [
            SLASH_MENU.to_owned(),
            SLASH_MENU.into(),
            screen(PLACEHOLDER),
        ]
    };
    let mut banner = cleared().to_vec();
    banner.push(SCROLLED.into());
    let mut refilled = cleared().to_vec();
    refilled.extend([screen(PLACEHOLDER), screen("❯ /s")]);
    let mut dialog = cleared().to_vec();
    dialog.extend([screen(PLACEHOLDER), TRUST.into()]);
    for (screens, gets, why) in [
        (cleared().to_vec(), vec![idle.clone(), blocked], "blocked"),
        (banner, vec![], scrolled),
        (refilled, vec![], typed),
        (dialog, vec![], "a dialog"),
    ] {
        herdr.with(|h| {
            h.calls.clear();
            h.screens = screens.into();
            h.gets = gets.into();
        });
        assert!(
            drive
                .slash_draft(slash(CLAUDE, "/sk", Some("/s")), &yes())
                .await
                .is_err(),
            "{why}"
        );
        assert_eq!(herdr.mutations(), ["agent.send_keys"], "{why}");
    }
    herdr.with(|h| {
        h.calls.clear();
        h.screens = cleared().into();
    });
    let checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = checks.clone();
    let revoked_after_clear: Authorized =
        Arc::new(move || seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 1);
    assert_eq!(
        code(
            drive
                .slash_draft(slash(CLAUDE, "/sk", Some("/s")), &revoked_after_clear)
                .await
        ),
        ErrorCode::NotPaired
    );
    assert_eq!(herdr.mutations(), ["agent.send_keys"]);
}

#[tokio::test]
async fn enter_never_runs_a_slash_command_in_the_box() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let send = |keys: Vec<Key>| AgentSendKeysParams {
        op_id: op('K'),
        terminal_id: tid(CLAUDE),
        keys,
    };
    herdr.with(|h| h.text = SLASH_MENU.into());
    for enter in [Key::Enter, Key::CtrlEnter] {
        assert_eq!(
            drive.send_keys(send(vec![Key::Down, enter]), &yes()).await,
            (
                Err((
                    ErrorCode::AgentNotReady,
                    "a slash command shows in the input box; send it as a prompt".into()
                )),
                None
            )
        );
    }
    assert!(herdr.mutations().is_empty());
    assert_eq!(
        drive
            .send_keys(send(vec![Key::Down, Key::Tab]), &yes())
            .await,
        (Ok(Response::Ok), None)
    );
    herdr.with(|h| h.text = screen("❯ fix the build"));
    assert_eq!(
        drive.send_keys(send(vec![Key::Enter]), &yes()).await,
        (Ok(Response::Ok), None)
    );
    assert_eq!(herdr.mutations(), ["agent.send_keys", "agent.send_keys"]);
}

#[tokio::test]
async fn keys_and_prompts_are_refused_while_claude_is_scrolled_up() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    for screen in [SCROLLED, SCROLLED_DIALOG, NEW_MESSAGE] {
        herdr.with(|h| h.text = screen.into());
        let (reply, target) = drive.send_keys(keys(CLAUDE), &yes()).await;
        assert_eq!(code(reply), ErrorCode::AgentNotReady);
        assert_eq!(target, None);
    }
    block(&herdr, &format!("{QUESTION}\n   1 new message (click) ↓\n"));
    assert_eq!(
        code(drive.send_keys(keys(CLAUDE), &yes()).await.0),
        ErrorCode::AgentNotReady
    );
    assert!(herdr.mutations().is_empty());
    for screen in [SCROLLED, SCROLLED_DIALOG, NEW_MESSAGE] {
        herdr.with(|h| {
            h.snapshot["agents"][0]["agent_status"] = json!("idle");
            h.text = screen.into();
        });
        assert_eq!(
            drive.prompt(prompt(CLAUDE), &yes()).await,
            Err((
                ErrorCode::AgentNotReady,
                "scrolled up on the machine; jump to the bottom first".into()
            ))
        );
    }
    assert!(herdr.mutations().is_empty());

    herdr.with(|h| {
        h.snapshot["agents"][0]["agent_status"] = json!("idle");
        h.text = BOTTOM.into();
    });
    assert_eq!(
        drive.send_keys(keys(CLAUDE), &yes()).await,
        (Ok(Response::Ok), None)
    );
    assert_eq!(herdr.mutations(), ["agent.send_keys"]);
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
async fn worktree_create_uses_returned_pane_and_keeps_checkout_on_start_failure() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let source = base.join("root/a");
    let path = source.join(".worktree/feature");
    let drive = herdr.driver(&["claude"], &base.join("root"));
    let mut p = task(&source, "claude");
    p.worktree = Some(TaskWorktree::Create {
        branch: "feature".into(),
    });
    herdr.with(|h| {
        h.gets.extend([
            started_agent("idle", true, false),
            started_agent("idle", true, false),
        ])
    });
    let (reply, _) = drive.task_new(p.clone(), &yes()).await;
    assert_eq!(
        reply,
        Ok(Response::TaskStarted {
            workspace_id: WorkspaceId::new("w9").unwrap(),
            terminal_id: tid("term_new"),
        })
    );
    assert_eq!(
        herdr.params("worktree.create")[0]["cwd"],
        source.to_str().unwrap()
    );
    assert_eq!(
        herdr.params("worktree.create")[0]["path"],
        path.to_str().unwrap()
    );
    assert_eq!(herdr.params("agent.start")[0]["pane_id"], "w9:p1");
    assert!(path.exists());

    let failed = source.join(".worktree/failed");
    p.worktree = Some(TaskWorktree::Create {
        branch: "failed".into(),
    });
    herdr.fail_next("agent.start", &["agent_not_ready"]);
    let error = drive.task_new(p, &yes()).await.0.unwrap_err();
    assert!(error.1.contains(failed.to_str().unwrap()));
    assert!(error.1.contains("workspace w9"));
    assert!(failed.exists());
    assert!(!herdr.methods().contains(&"workspace.close".to_owned()));
}

#[tokio::test]
async fn worktree_open_reuses_only_an_empty_root_pane() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let source = base.join("root/a");
    let path = base.join("root/existing");
    std::fs::create_dir(&path).unwrap();
    herdr.with(|h| {
        h.worktree_path = Some(path.to_str().unwrap().into());
        h.worktree_open = true;
        h.pane_occupied = true;
    });
    let drive = herdr.driver(&["claude"], &base.join("root"));
    let listing = drive
        .task_worktrees(
            TaskWorktreesParams {
                cwd: Cwd::new(source.to_str().unwrap()).unwrap(),
            },
            &yes(),
        )
        .await
        .unwrap();
    let Response::TaskWorktrees { worktrees, .. } = listing else {
        panic!("wrong listing")
    };
    assert_eq!(worktrees.len(), 1);
    assert!(worktrees[0].open);
    let mut p = task(&source, "claude");
    p.worktree = Some(TaskWorktree::Open {
        path: Cwd::new(path.to_str().unwrap()).unwrap(),
    });
    let error = drive.task_new(p.clone(), &yes()).await.0.unwrap_err();
    assert!(error.1.contains("occupied"));
    assert!(herdr.params("agent.start").is_empty());
    herdr.with(|h| {
        h.pane_occupied = false;
        h.gets.extend([
            started_agent("idle", true, false),
            started_agent("idle", true, false),
        ]);
    });
    assert!(drive.task_new(p, &yes()).await.0.is_ok());
    assert_eq!(herdr.params("worktree.open").len(), 2);
    assert_eq!(herdr.params("agent.start")[0]["pane_id"], "w9:p1");
}

#[tokio::test]
async fn worktree_rejects_destinations_outside_task_roots() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base.join("root"));
    let source = base.join("root/a");
    let mut p = task(&source, "claude");
    let mut bad_source = task(&base.join("outside"), "claude");
    bad_source.worktree = Some(TaskWorktree::Create {
        branch: "feature".into(),
    });
    assert_eq!(
        drive.task_new(bad_source, &yes()).await.0.unwrap_err().0,
        ErrorCode::InvalidParams
    );
    std::os::unix::fs::symlink(base.join("outside"), source.join(".worktree")).unwrap();
    p.worktree = Some(TaskWorktree::Create {
        branch: "feature".into(),
    });
    assert_eq!(
        drive.task_new(p.clone(), &yes()).await.0.unwrap_err().0,
        ErrorCode::InvalidParams
    );
    assert!(herdr.params("worktree.create").is_empty());
    p.worktree = Some(TaskWorktree::Create {
        branch: "../bad".into(),
    });
    assert_eq!(
        drive.task_new(p, &yes()).await.0.unwrap_err().0,
        ErrorCode::InvalidParams
    );
    assert!(herdr.params("worktree.create").is_empty());
    let outside = base.join("outside/existing");
    std::fs::create_dir(&outside).unwrap();
    herdr.with(|h| h.worktree_path = Some(outside.to_str().unwrap().into()));
    let listing = drive
        .task_worktrees(
            TaskWorktreesParams {
                cwd: Cwd::new(source.to_str().unwrap()).unwrap(),
            },
            &yes(),
        )
        .await
        .unwrap();
    let Response::TaskWorktrees { worktrees, .. } = listing else {
        panic!("wrong listing")
    };
    assert!(worktrees.is_empty());
}

#[tokio::test]
async fn task_new_creates_its_new_folder_once() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base.join("root"));
    let fresh = base.join("root/fresh");
    let mut p = task(&base.join("root/b/.."), "claude");
    p.new_folder = Some(FolderName::new("fresh").unwrap());
    let fp = collied::drive::fingerprint(&Request::TaskNew(p.clone()));
    let attempt = || {
        let (d, p) = (drive.clone(), p.clone());
        async move {
            let (id, run) = (p.op_id.clone(), d.clone());
            d.once(
                "nPHONE",
                &id,
                fp,
                async move { run.task_new(p, &yes()).await.0 },
            )
            .await
        }
    };
    herdr.with(|h| {
        h.gets.extend([
            started_agent("idle", true, false),
            started_agent("idle", true, false),
        ])
    });
    let (first, origin) = attempt().await;
    assert!(first.is_ok(), "{first:?}");
    assert_eq!(origin, Origin::Ran);
    assert!(fresh.is_dir());
    assert_eq!(std::fs::read_dir(&fresh).unwrap().count(), 0);
    assert_eq!(
        herdr.params("workspace.create"),
        vec![json!({"cwd": fresh, "label": "tests", "focus": false})]
    );
    assert_eq!(attempt().await, (first, Origin::Replayed));
    assert_eq!(herdr.mutations().len(), 3);

    let (reply, cwd) = drive.task_new(p.clone(), &yes()).await;
    assert!(cwd.is_none());
    assert_eq!(
        reply,
        Err((ErrorCode::InvalidParams, "folder already exists".into()))
    );

    herdr.with(|h| h.calls.clear());
    p.new_folder = Some(FolderName::new("second").unwrap());
    herdr.fail_next("workspace.create", &["timeout"]);
    let (reply, cwd) = drive.task_new(p.clone(), &yes()).await;
    let second = base.join("root/second");
    assert_eq!(cwd.unwrap().as_str(), second.to_str().unwrap());
    assert_eq!(
        reply.unwrap_err(),
        (
            ErrorCode::AgentNotReady,
            format!(
                "folder {} was created and left in place: the agent did not take the input in time",
                second.display()
            )
        )
    );
    assert!(second.is_dir());

    herdr.with(|h| h.calls.clear());
    p.new_folder = Some(FolderName::new("third").unwrap());
    let refused = [
        (task(&base.join("outside"), "claude"), "outside"),
        (task(&base.join("root/a"), "codex"), "not allowed"),
    ];
    for (mut bad, why) in refused {
        bad.new_folder = p.new_folder.clone();
        let (reply, cwd) = drive.task_new(bad, &yes()).await;
        assert!(cwd.is_none());
        assert!(reply.unwrap_err().1.contains(why));
    }
    let (reply, _) = drive.task_new(p, &no()).await;
    assert_eq!(code(reply), ErrorCode::NotPaired);
    assert!(herdr.methods().is_empty());
    assert!(!base.join("root/third").exists() && !base.join("outside/third").exists());
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

    // Claude Code's folder trust question, numbered or not (2.1.292), is never answered
    // for the user, even when it only shows up after the agent looked ready.
    for trust in [
        TRUST,
        include_str!("fixtures/claude-2.1.292/trust.detection.txt"),
    ] {
        herdr.with(|h| {
            h.calls.clear();
            h.text = trust.into();
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
    }

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

    // A busy pane is retried while it holds the new terminal...
    herdr.with(|h| {
        h.calls.clear();
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
async fn archive_closes_regular_pane_without_removing_its_folder() {
    let herdr = Mock::start();
    let (_dir, base) = root();
    let folder = base.join("root/a");
    herdr.with(|h| h.snapshot["panes"][1]["cwd"] = json!(folder));
    let drive = herdr.driver(&["claude"], &base.join("root"));
    let p = |confirm| TaskArchiveParams {
        op_id: op('A'),
        terminal_id: tid(SHELL),
        confirm,
    };
    assert_eq!(
        code(drive.task_archive(p(false), &yes()).await),
        ErrorCode::ConfirmRequired
    );
    assert_eq!(
        code(drive.task_archive(p(true), &no()).await),
        ErrorCode::NotPaired
    );
    assert!(herdr.mutations().is_empty());
    let Response::TaskArchived { message } = drive.task_archive(p(true), &yes()).await.unwrap()
    else {
        panic!("unexpected response");
    };
    assert!(message.contains("folder was left in place"), "{message}");
    assert!(folder.exists());
    assert_eq!(
        herdr.params("pane.close"),
        vec![json!({"pane_id": "w7:p2"})]
    );
    assert!(herdr.params("worktree.remove").is_empty());
}

#[tokio::test]
async fn archive_force_removes_only_the_linked_worktree_workspace() {
    let herdr = Mock::start();
    let (_dir, base) = root();
    let source = base.join("root/a");
    let checkout = source.join(".worktree/feature");
    std::fs::create_dir_all(&checkout).unwrap();
    herdr.with(|h| {
        h.worktree_path = Some(checkout.to_str().unwrap().into());
        h.worktree_open = true;
        h.worktree_workspace_id = Some("w7".into());
        h.snapshot["workspaces"][1]["worktree"] = json!({
            "repo_root": source, "checkout_path": checkout, "is_linked_worktree": true
        });
        h.snapshot["panes"][1]["cwd"] = json!(checkout);
    });
    let drive = herdr.driver(&["claude"], &base.join("root"));
    let p = TaskArchiveParams {
        op_id: op('A'),
        terminal_id: tid(SHELL),
        confirm: true,
    };
    herdr.fail_next("worktree.remove", &["worktree_not_found"]);
    let error = drive.task_archive(p.clone(), &yes()).await.unwrap_err();
    assert!(error.1.contains("gh poi was skipped"));
    herdr.with(|h| h.calls.clear());
    let Response::TaskArchived { message } = drive.task_archive(p.clone(), &yes()).await.unwrap()
    else {
        panic!("unexpected response");
    };
    assert!(message.contains("Removed worktree checkout"), "{message}");
    assert_eq!(
        herdr.params("worktree.remove"),
        vec![json!({"workspace_id": "w7", "force": true})]
    );
    assert!(herdr.params("pane.close").is_empty());
    assert!(herdr.params("workspace.close").is_empty());

    herdr.with(|h| {
        h.calls.clear();
        h.snapshot["workspaces"][1]["worktree"]["checkout_path"] = json!(base.join("outside"));
    });
    assert_eq!(
        code(drive.task_archive(p, &yes()).await),
        ErrorCode::InvalidParams
    );
    assert!(herdr.mutations().is_empty());
}

#[tokio::test]
async fn archive_reports_removed_checkout_when_herdr_drops_its_response() {
    let herdr = Mock::start();
    let (_dir, base) = root();
    let source = base.join("root/a");
    let checkout = source.join(".worktree/feature");
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&source)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q"]);
    git(&[
        "-c",
        "user.name=Collie Test",
        "-c",
        "user.email=collie@example.invalid",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "initial",
    ]);
    git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "feature",
        checkout.to_str().unwrap(),
    ]);
    herdr.with(|h| {
        h.worktree_path = Some(checkout.to_str().unwrap().into());
        h.worktree_open = true;
        h.worktree_workspace_id = Some("w7".into());
        h.disconnect_after_remove = true;
        h.snapshot["workspaces"][1]["worktree"] = json!({
            "repo_root": source, "checkout_path": checkout, "is_linked_worktree": true
        });
        h.snapshot["panes"][1]["cwd"] = json!(checkout);
    });
    let drive = herdr.driver(&["claude"], &base.join("root"));
    let checks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let auth: Authorized = {
        let checks = checks.clone();
        std::sync::Arc::new(move || checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2)
    };
    let Response::TaskArchived { message } = drive
        .task_archive(
            TaskArchiveParams {
                op_id: op('A'),
                terminal_id: tid(SHELL),
                confirm: true,
            },
            &auth,
        )
        .await
        .unwrap()
    else {
        panic!("unexpected response");
    };
    assert!(message.contains("Removed worktree checkout"), "{message}");
    assert!(message.contains("closed its workspace"), "{message}");
    assert!(message.contains("gh poi was skipped"), "{message}");
    assert!(!checkout.exists());
    assert_eq!(herdr.params("worktree.remove").len(), 1);
}

#[tokio::test]
async fn watch_pushes_changes_only_and_ends_when_the_agent_goes() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    assert!(matches!(
        drive.watch(tid("term_gone"), 200, false).await,
        Err((ErrorCode::NotFound, _))
    ));

    let mut watcher = drive.watch(tid(CLAUDE), 200, false).await.unwrap();
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

    tokio::select! {
        _ = watcher.recv() => panic!("unchanged output was pushed again"),
        reached = reads_reach(&herdr, 3, Duration::from_secs(4)) => {
            assert!(reached, "the watch stopped reading");
        }
    }

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
    assert_eq!((patch.skip, patch.keep, patch.trail), (0, 100, 1));
    assert_eq!(patch.tail, ["b\r"]);
    assert_eq!(patch.apply(&full).unwrap().ansi, format!("{history}b\r\n"));

    herdr.with(|h| {
        h.snapshot["agents"].as_array_mut().unwrap().remove(0);
    });
    assert!(matches!(next(&mut watcher).await, Ok(Some(Watched::Gone))));
    assert!(matches!(next(&mut watcher).await, Ok(None)));

    let watcher = drive.watch(tid(CODEX_BLOCKED), 200, false).await.unwrap();
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

async fn reads_over(herdr: &Mock, span: Duration) -> usize {
    let before = herdr.params("agent.read").len();
    tokio::time::sleep(span).await;
    herdr.params("agent.read").len() - before
}

async fn reads_reach(herdr: &Mock, count: usize, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while herdr.params("agent.read").len() < count {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    true
}

#[tokio::test]
async fn a_quiet_screen_is_read_once_a_second_until_it_changes() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let mut watcher = drive.watch(tid(CLAUDE), 200, false).await.unwrap();
    let Ok(Some(Watched::Output(_))) = next(&mut watcher).await else {
        panic!("no first output");
    };
    let first_at = tokio::time::Instant::now();
    let busy = herdr.params("agent.read").len() + 6;
    assert!(
        reads_reach(&herdr, busy, Duration::from_secs(4)).await,
        "no 6 reads in 4 s before the quiet spell"
    );
    tokio::time::sleep_until(first_at + Duration::from_millis(5100)).await;
    let quiet = reads_over(&herdr, Duration::from_millis(2100)).await;
    assert!((1..=3).contains(&quiet), "{quiet} reads in 2.1 s after it");

    herdr.with(|h| h.text = "next\r\n".into());
    let Ok(Some(Watched::Output(second))) = next(&mut watcher).await else {
        panic!("no output after the change");
    };
    assert_eq!(second.ansi, "next\r\n");
    let woken = herdr.params("agent.read").len() + 6;
    assert!(
        reads_reach(&herdr, woken, Duration::from_secs(4)).await,
        "no 6 reads in 4 s after the change"
    );
}

#[tokio::test]
async fn a_low_data_watch_is_read_once_a_second() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    let mut watcher = drive.watch(tid(CLAUDE), 200, true).await.unwrap();
    let Ok(Some(Watched::Output(_))) = next(&mut watcher).await else {
        panic!("no first output");
    };
    let n = reads_over(&herdr, Duration::from_millis(2100)).await;
    assert!((1..=3).contains(&n), "{n} reads in 2.1 s");
    herdr.with(|h| h.text = "next\r\n".into());
    let Ok(Some(Watched::Output(_))) = next(&mut watcher).await else {
        panic!("no output after the change");
    };
    let n = reads_over(&herdr, Duration::from_millis(2100)).await;
    assert!((1..=3).contains(&n), "{n} reads in 2.1 s after a change");
    drop(watcher);

    let mut watcher = drive
        .watch_terminal(tid(SHELL), 200, true, yes())
        .await
        .unwrap();
    let Ok(Some(Watched::Output(_))) = next(&mut watcher).await else {
        panic!("no shell output");
    };
    let before = herdr.params("pane.read").len();
    tokio::time::sleep(Duration::from_millis(2100)).await;
    let n = herdr.params("pane.read").len() - before;
    assert!((1..=3).contains(&n), "{n} shell reads in 2.1 s");
}

#[tokio::test]
async fn a_reply_over_herdrs_line_limit_is_read_with_fewer_lines() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    // About 2.2 KB of JSON per line, so 500 lines are still over 1 MiB.
    let line = format!("{}\r\n", "\u{1b}[31mx".repeat(200));
    herdr.with(|h| h.text = line.repeat(1000));

    let mut watcher = drive.watch(tid(CLAUDE), 1000, false).await.unwrap();
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

fn run(terminal: &str, text: &str) -> TerminalRunParams {
    TerminalRunParams {
        op_id: op('R'),
        terminal_id: tid(terminal),
        text: PromptText::new(text).unwrap(),
    }
}

#[tokio::test]
async fn terminal_input_reaches_a_shell_pane_only() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);

    assert_eq!(
        drive.terminal_run(run(SHELL, "git pull"), &yes()).await,
        Ok(Response::Ok)
    );
    assert_eq!(herdr.methods(), ["session.snapshot", "pane.send_input"]);
    assert_eq!(
        herdr.params("pane.send_input"),
        vec![json!({"pane_id": "w7:p2", "text": "git pull", "keys": ["enter"]})]
    );
    let keys = AgentSendKeysParams {
        op_id: op('C'),
        terminal_id: tid(SHELL),
        keys: vec![Key::CtrlC, Key::Up, Key::Enter],
    };
    assert_eq!(
        drive.terminal_send_keys(keys.clone(), &yes()).await,
        Ok(Response::Ok)
    );
    assert_eq!(
        herdr.params("pane.send_keys"),
        vec![json!({"pane_id": "w7:p2", "keys": ["ctrl+c", "up", "enter"]})]
    );

    let refused = |reply: Reply, want: ErrorCode, message: Option<&str>| {
        let (got, text) = reply.expect_err("refused");
        assert_eq!(got, want, "{text}");
        if let Some(m) = message {
            assert_eq!(text, m);
        }
    };
    refused(
        drive.terminal_run(run(SHELL, "git pull"), &no()).await,
        ErrorCode::TerminalLocked,
        None,
    );
    refused(
        drive.terminal_send_keys(keys.clone(), &no()).await,
        ErrorCode::TerminalLocked,
        None,
    );
    refused(
        drive.terminal_run(run(CLAUDE, "y"), &yes()).await,
        ErrorCode::NotFound,
        Some(collied::drive::HOSTS_AGENT),
    );
    refused(
        drive.terminal_run(run("term_gone", "ls"), &yes()).await,
        ErrorCode::NotFound,
        None,
    );
    // herdr lists an agent it is still launching before the pane reports it.
    let mut launching = herdr.with(|h| h.snapshot["agents"][0].clone());
    launching["terminal_id"] = json!(SHELL);
    launching["pane_id"] = json!("w7:p2");
    launching["launch_pending"] = json!(true);
    herdr.with(|h| h.snapshot["agents"].as_array_mut().unwrap().push(launching));
    refused(
        drive.terminal_send_keys(keys, &yes()).await,
        ErrorCode::NotFound,
        Some(collied::drive::HOSTS_AGENT),
    );
    assert_eq!(herdr.mutations(), ["pane.send_input", "pane.send_keys"]);

    herdr.fail_next("pane.send_input", &["pane_input_failed"]);
    herdr.with(|h| {
        h.snapshot["agents"].as_array_mut().unwrap().pop();
    });
    refused(
        drive.terminal_run(run(SHELL, "secret"), &yes()).await,
        ErrorCode::Internal,
        Some("herdr refused: pane_input_failed"),
    );
}

/// The op cache is per phone, not per session: a retry from the phone's next session
/// replays the first attempt, which still checks the grant of the session that sent it.
#[tokio::test]
async fn a_retried_terminal_op_writes_only_under_the_session_that_sent_it() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    herdr.with(|h| h.slow = Some(Duration::from_millis(300)));
    let first = Arc::new(AtomicBool::new(true));
    let first_auth: Authorized = {
        let first = first.clone();
        Arc::new(move || first.load(Ordering::SeqCst))
    };
    let sent = tokio::spawn({
        let d = drive.clone();
        async move {
            let write = {
                let d = d.clone();
                async move { d.terminal_run(run(SHELL, "ls"), &first_auth).await }
            };
            d.once("nPHONE", &op('R'), 1, write).await
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    first.store(false, Ordering::SeqCst);
    let retry = {
        let d = drive.clone();
        async move { d.terminal_run(run(SHELL, "ls"), &yes()).await }
    };
    let (reply, origin) = drive.once("nPHONE", &op('R'), 1, retry).await;
    assert_eq!(origin, Origin::Replayed);
    assert_eq!(code(reply), ErrorCode::TerminalLocked);
    assert_eq!(sent.await.unwrap().1, Origin::Ran);
    assert!(herdr.mutations().is_empty(), "{:?}", herdr.methods());
}

#[tokio::test]
async fn claude_prose_wrapped_at_the_pane_width_is_marked() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    herdr.with(|h| {
        h.text = "⏺ The quick brown fox jumps over the\r\n  lazy dog.\r\n".into();
        h.columns = Some(40);
    });
    let recent = |terminal: &str| ReadParams {
        terminal_id: tid(terminal),
        source: ReadSource::Recent,
        lines: None,
    };
    let joins = |reply: Reply| match reply {
        Ok(Response::Terminal(read)) => (read.wraps, read.splits),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        joins(drive.read(recent(CLAUDE), true).await),
        (vec![1], vec![])
    );
    assert_eq!(
        herdr.params("pane.layout"),
        vec![json!({"pane_id": "w6:p1"})]
    );

    let mut watcher = drive.watch(tid(CLAUDE), 200, false).await.unwrap();
    let Ok(Some(Watched::Output(first))) = next(&mut watcher).await else {
        panic!("no first output");
    };
    assert_eq!((first.wraps, first.splits), (vec![1], vec![]));
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(herdr.params("pane.layout").len(), 2, "unchanged ticks");
    herdr.with(|h| {
        h.text.push_str("\r\n");
        h.errors
            .insert("pane.layout".into(), VecDeque::from(["x".to_owned()]));
    });
    let Ok(Some(Watched::Patch(failed))) = next(&mut watcher).await else {
        panic!("no patch");
    };
    assert_eq!((failed.wraps, failed.splits), (Some(vec![]), None));
    let Ok(Some(Watched::Patch(retried))) = next(&mut watcher).await else {
        panic!("a failed width is not read again");
    };
    assert_eq!((retried.wraps, retried.tail), (Some(vec![1]), vec![]));
    drop(watcher);

    herdr.with(|h| h.zoomed = true);
    assert_eq!(
        joins(drive.read(recent(CLAUDE), true).await),
        (vec![], vec![]),
        "a zoomed tab"
    );
    herdr.with(|h| h.zoomed = false);

    herdr.with(|h| h.columns = None);
    assert_eq!(
        joins(drive.read(recent(CLAUDE), true).await),
        (vec![], vec![]),
        "a pane missing from the layout"
    );
    herdr.with(|h| h.columns = Some(40));
    assert_eq!(
        joins(drive.read(recent(CODEX_BLOCKED), true).await),
        (vec![], vec![])
    );
    let layouts = herdr.params("pane.layout").len();
    assert_eq!(
        joins(drive.read(recent(CLAUDE), false).await),
        (vec![], vec![])
    );
    let mut watcher = drive
        .watch_terminal(tid(SHELL), 200, false, yes())
        .await
        .unwrap();
    let Ok(Some(Watched::Output(shell))) = next(&mut watcher).await else {
        panic!("no shell output");
    };
    assert_eq!((shell.wraps, shell.splits), (vec![], vec![]));
    assert_eq!(herdr.params("pane.layout").len(), layouts);
}

#[tokio::test]
async fn codex_and_copilot_reads_and_watches_mark_only_prose_wraps() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude", "codex", "copilot"], &base);
    let codex = include_str!("fixtures/codex-2026-10-09/prose.ansi.txt");
    herdr.with(|h| {
        h.text = codex.into();
        h.columns = Some(186);
    });
    let recent = |terminal: &str| ReadParams {
        terminal_id: tid(terminal),
        source: ReadSource::Recent,
        lines: None,
    };
    let Ok(Response::Terminal(read)) = drive.read(recent(CODEX_BLOCKED), true).await else {
        panic!("no Codex read");
    };
    assert_eq!(
        (read.wraps.as_slice(), read.splits.as_slice()),
        (&[1][..], &[][..])
    );
    assert!(
        read.ansi
            .contains("screens so continuation detection joins prose")
    );
    assert!(
        read.reflowed()
            .unwrap()
            .contains("tables, diffs, and interactive")
    );
    let mut watcher = drive.watch(tid(CODEX_BLOCKED), 200, false).await.unwrap();
    let Ok(Some(Watched::Output(first))) = next(&mut watcher).await else {
        panic!("no Codex watch");
    };
    assert_eq!((first.wraps, first.splits), (vec![1], vec![]));
    drop(watcher);

    let copilot = "term_0102030405060708";
    let prose =
        " ● The quick brown fox jumps over the  ┃\r\n   lazy dog keeps running.              ┃";
    herdr.with(|h| {
        let mut agent = h.snapshot["agents"][1].clone();
        agent["terminal_id"] = json!(copilot);
        agent["pane_id"] = json!("w7:p2");
        agent["agent"] = json!("copilot");
        h.snapshot["agents"].as_array_mut().unwrap().push(agent);
        h.text = prose.into();
        h.columns = Some(40);
    });
    let Ok(Response::Terminal(read)) = drive.read(recent(copilot), true).await else {
        panic!("no Copilot read");
    };
    assert_eq!(
        (read.wraps.as_slice(), read.splits.as_slice()),
        (&[1][..], &[][..])
    );
    assert!(read.ansi.contains("the  ┃\r\n   lazy"));
    assert!(read.reflowed().unwrap().contains("the lazy dog"));
    let mut watcher = drive.watch(tid(copilot), 200, false).await.unwrap();
    let Ok(Some(Watched::Output(first))) = next(&mut watcher).await else {
        panic!("no Copilot watch");
    };
    assert_eq!((first.wraps, first.splits), (vec![1], vec![]));
}

#[tokio::test]
async fn a_terminal_watch_reads_only_under_its_grant_and_ends_when_an_agent_starts() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["claude"], &base);
    assert!(matches!(
        drive.watch_terminal(tid(CLAUDE), 200, false, yes()).await,
        Err((ErrorCode::NotFound, _))
    ));
    let grant = Arc::new(AtomicBool::new(true));
    let auth: Authorized = {
        let grant = grant.clone();
        Arc::new(move || grant.load(Ordering::SeqCst))
    };
    let mut watcher = drive
        .watch_terminal(tid(SHELL), 300, false, auth)
        .await
        .unwrap();
    let Ok(Some(Watched::Output(first))) = next(&mut watcher).await else {
        panic!("no first output");
    };
    assert_eq!(first.terminal_id.as_str(), SHELL);
    assert_eq!(
        herdr.params("pane.read")[0],
        json!({"pane_id": "w7:p2", "source": "recent_unwrapped", "lines": 300, "format": "ansi"})
    );
    assert!(herdr.params("agent.read").is_empty());

    grant.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let reads = herdr.params("pane.read").len();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        herdr.params("pane.read").len(),
        reads,
        "nothing read without a grant"
    );
    grant.store(true, Ordering::SeqCst);

    herdr.with(|h| h.snapshot["panes"][2]["agent"] = json!("claude"));
    assert!(matches!(next(&mut watcher).await, Ok(Some(Watched::Gone))));
    assert!(matches!(next(&mut watcher).await, Ok(None)));
    assert!(herdr.mutations().is_empty());
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
async fn codex_question_opens_and_a_typed_answer_submits_without_approving() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["codex"], &base);
    herdr.with(|h| {
        h.text = CODEX_QUEUED_QUESTION.into();
        h.rule = Some("osc_title_blocked".into());
    });
    let mut open = keys(CODEX_BLOCKED);
    open.keys = vec![Key::ShiftLeft];
    assert_eq!(
        drive.send_keys(open.clone(), &yes()).await,
        (
            Ok(Response::Ok),
            Some(format!("{CODEX_BLOCKED} open question"))
        )
    );
    assert_eq!(
        herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["shift+left"]})]
    );

    let filled = CODEX_QUESTION_OPEN
        .replace("  › 1. SQLite", "    1. SQLite")
        .replace("    3. Other", "  › 3. DuckDB");
    herdr.with(|h| h.screens = [CODEX_QUESTION_OPEN.into(), filled].into());
    assert_eq!(
        drive
            .type_text(typed(CODEX_BLOCKED, "DuckDB"), &yes())
            .await,
        Ok(Response::Ok)
    );
    assert_eq!(
        herdr.params("pane.send_text"),
        [json!({"pane_id": "w7:p1", "text": "\u{1b}[200~DuckDB\u{1b}[201~"})]
    );
    assert_eq!(
        herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["shift+left"]}),
            json!({"target": "w7:p1", "keys": ["enter"]})
        ]
    );

    herdr.with(|h| h.screens = [CODEX_QUESTION_OPEN.into()].into());
    let enters = herdr.params("agent.send_keys").len();
    assert_eq!(
        code(
            drive
                .type_text(typed(CODEX_BLOCKED, "SQLite"), &yes())
                .await
        ),
        ErrorCode::AgentNotReady
    );
    assert_eq!(herdr.params("agent.send_keys").len(), enters);

    let mut changed = herdr.with(|h| h.snapshot["agents"][1].clone());
    changed["state_change_seq"] = json!(999);
    herdr.with(|h| {
        h.screens = [CODEX_QUESTION_OPEN.into()].into();
        h.gets.extend([h.snapshot["agents"][1].clone(), changed]);
    });
    assert_eq!(
        code(
            drive
                .type_text(typed(CODEX_BLOCKED, "DuckDB"), &yes())
                .await
        ),
        ErrorCode::AgentNotReady
    );
    assert_eq!(herdr.params("agent.send_keys").len(), 2);

    let approval = "Would you like to run the following command?\n› 1. Yes, proceed\n  2. No\nPress enter to confirm or esc to cancel\n";
    herdr.with(|h| {
        h.text = approval.into();
        h.screens.clear();
    });
    let writes = herdr.mutations();
    assert_eq!(
        code(drive.send_keys(open, &yes()).await.0),
        ErrorCode::AgentBlocked
    );
    assert_eq!(
        code(drive.type_text(typed(CODEX_BLOCKED, "yes"), &yes()).await),
        ErrorCode::AgentBlocked
    );
    assert_eq!(herdr.mutations(), writes);
}

#[tokio::test]
async fn codex_text_only_question_accepts_a_typed_answer() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["codex"], &base);
    let question = "Queued follow-up inputs\n\n  1 of 2\n\n  Which missing agent do you see in All?\n\n  Type your answer\n\n  enter submit   ⌃] skip   ⇧→ main prompt   ⇧← next question\n";
    let answered = question.replace("Type your answer", "It is in Done");
    herdr.with(|h| {
        h.rule = Some("osc_title_blocked".into());
        h.screens = [question.into(), answered].into();
    });
    assert_eq!(
        drive
            .type_text(typed(CODEX_BLOCKED, "It is in Done"), &yes())
            .await,
        Ok(Response::Ok)
    );
    assert_eq!(
        herdr.params("pane.send_text"),
        [json!({"pane_id": "w7:p1", "text": "\u{1b}[200~It is in Done\u{1b}[201~"})]
    );
    assert_eq!(
        herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["enter"]})]
    );
}

#[tokio::test]
async fn codex_question_options_use_arrows_and_enter_without_approving() {
    let herdr = Mock::start();
    let (_d, base) = root();
    let drive = herdr.driver(&["codex"], &base);
    herdr.with(|h| {
        h.text = CODEX_QUESTION_OPEN.into();
        h.rule = Some("osc_title_blocked".into());
    });
    let mut down = keys(CODEX_BLOCKED);
    down.keys = vec![Key::Down];
    let mut enter = keys(CODEX_BLOCKED);
    enter.keys = vec![Key::Enter];
    assert!(drive.send_keys(down.clone(), &yes()).await.0.is_ok());
    assert!(drive.send_keys(enter.clone(), &yes()).await.0.is_ok());
    assert_eq!(
        herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]})
        ]
    );

    down.keys.push(Key::Enter);
    assert_eq!(
        code(drive.send_keys(down, &yes()).await.0),
        ErrorCode::AgentBlocked
    );
    let approval = "Would you like to run the following command?\n› 1. Yes, proceed\n  2. No\nPress enter to confirm or esc to cancel\n";
    herdr.with(|h| h.text = approval.into());
    assert_eq!(
        code(drive.send_keys(enter, &yes()).await.0),
        ErrorCode::AgentBlocked
    );
    assert_eq!(herdr.params("agent.send_keys").len(), 2);
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

#[tokio::test]
async fn git_changes_use_live_agent_checkout_and_existing_authorization() {
    let herdr = Mock::start();
    let (_dir, base) = root();
    let checkout = base.join("root/a");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-b", "feature"])
            .arg(&checkout)
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::write(checkout.join("test.txt"), "feedback\n").unwrap();
    herdr.with(|h| {
        let agent = h.snapshot["agents"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|a| a["terminal_id"] == CLAUDE)
            .unwrap();
        agent["cwd"] = json!(base.join("root/b"));
        agent["foreground_cwd"] = json!(checkout);
        agent["agent_status"] = json!("blocked");
    });
    let drive = herdr.driver(&["claude"], &base.join("root"));
    let Response::GitChanges(list) = drive.git_changes(tid(CLAUDE), &yes()).await.unwrap() else {
        panic!()
    };
    assert_eq!(
        list.root.as_ref().unwrap().as_str(),
        checkout.to_str().unwrap()
    );
    assert_eq!(list.branch.as_deref(), Some("feature"));
    assert_eq!(list.files[0].path, "test.txt");
    let params = protocol::AgentDiffParams {
        terminal_id: tid(CLAUDE),
        root: list.root.unwrap(),
        path: "test.txt".into(),
        section: protocol::GitSection::Untracked,
    };
    let Response::GitDiff(diff) = drive.git_diff(params.clone(), &yes()).await.unwrap() else {
        panic!()
    };
    assert!(diff.patch.contains("+feedback"));
    assert_eq!(
        drive.git_changes(tid(CLAUDE), &no()).await.unwrap_err().0,
        ErrorCode::NotPaired
    );
    assert_eq!(
        drive.git_diff(params.clone(), &no()).await.unwrap_err().0,
        ErrorCode::NotPaired
    );
    herdr.with(|h| {
        let agent = h.snapshot["agents"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|a| a["terminal_id"] == CLAUDE)
            .unwrap();
        agent["foreground_cwd"] = json!(base.join("outside"));
    });
    assert!(drive.git_changes(tid(CLAUDE), &yes()).await.is_err());
    assert!(drive.git_diff(params, &yes()).await.is_err());
    assert!(herdr.mutations().is_empty());
}
