use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use protocol::PendingTool;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::control::{self, Reply, Request};
use crate::prompt;

const MAX_INPUT: u64 = 1024 * 1024;
const MAX_SESSION_CHARS: usize = 128;
const MAX_NAME_CHARS: usize = 64;
const MAX_SUMMARY_CHARS: usize = prompt::MAX_CONTEXT_CHARS - MAX_NAME_CHARS - 2;
const MAX_SHOWN_CHARS: usize = 4096;
const DEADLINE: Duration = Duration::from_secs(2);

#[derive(Deserialize)]
struct Input {
    #[serde(default)]
    hook_event_name: Option<String>,
    session_id: String,
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_input: Value,
    /// Set when the hook fires inside a subagent.
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    prompt_id: Option<String>,
}

/// `collied hook`: Claude Code's PermissionRequest and PreToolUse hooks and Codex's
/// SessionStart hook. It exits 0 and prints nothing, so the agent goes on as usual, except
/// for a PreToolUse that collied answers with a stop at the plan limit. Any failure, collied
/// not running or not answering within the deadline included, lets the agent go on.
pub async fn run(control_path: &Path) {
    let mut stdin = std::io::stdin();
    let mut raw = Vec::new();
    let read = (&mut stdin).take(MAX_INPUT).read_to_end(&mut raw);
    let _ = std::io::copy(&mut stdin, &mut std::io::sink());
    if read.is_err() {
        return;
    }
    let terminal = std::env::var("HC_TERMINAL_ID").ok();
    if let Some(out) = answer(control_path, &raw, terminal).await {
        println!("{out}");
    }
}

async fn answer(control_path: &Path, raw: &[u8], terminal: Option<String>) -> Option<Value> {
    let request = request(raw, terminal)?;
    let reply = tokio::time::timeout(DEADLINE, control::request(control_path, &request)).await;
    match reply {
        Ok(Ok(Some(Reply::Limit { stop: Some(reason) }))) => Some(stop(&reason)),
        _ => None,
    }
}

/// `continue: false` ends the turn before Claude Code's next request; the denial keeps the
/// call itself from running.
fn stop(reason: &str) -> Value {
    json!({
        "continue": false,
        "stopReason": reason,
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        },
    })
}

/// A SessionStart outside a herdr-core terminal reports nothing: herdr's own hook binds it.
fn request(raw: &[u8], terminal: Option<String>) -> Option<Request> {
    let input = serde_json::from_slice::<Input>(raw).ok()?;
    match input.hook_event_name.as_deref() {
        Some("SessionStart") => Some(Request::Session {
            terminal_id: terminal.filter(|t| !t.is_empty())?,
            session_id: input.session_id,
        }),
        Some("PreToolUse") => Some(Request::Limit {
            session_id: input.session_id,
            subagent: input.agent_id.is_some(),
            prompt_id: input.prompt_id,
        }),
        None | Some("PermissionRequest") => {
            let shown = shown(&input.tool_name, &input.tool_input);
            Report::new(&input.tool_name, shown.clone())?;
            Some(Request::Hook {
                session_id: input.session_id,
                tool_name: input.tool_name,
                shown,
            })
        }
        Some(_) => None,
    }
}

/// What the permission dialog shows of the call: for Bash the command, then Claude's
/// description; else the file, URL, query or pattern. Empty for other tools.
pub fn shown(name: &str, input: &Value) -> Vec<String> {
    let keys: &[&str] = match name {
        "Bash" => &["command", "description"],
        "Edit" | "MultiEdit" | "Write" | "Read" => &["file_path"],
        "NotebookEdit" => &["notebook_path"],
        "WebFetch" => &["url"],
        "WebSearch" => &["query"],
        "Glob" | "Grep" => &["pattern"],
        _ => &[],
    };
    keys.iter()
        .filter_map(|k| input.get(*k).and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

/// A reported call, which names an approval only when its dialog shows it.
pub struct Report {
    pub tool: PendingTool,
    shown: Vec<String>,
}

impl Report {
    /// Checked again by the daemon: any process of the user can reach the control socket.
    pub fn new(name: &str, shown: Vec<String>) -> Option<Self> {
        let name = prompt::clean(name);
        if name.is_empty()
            || shown.is_empty()
            || shown.len() > 2
            || shown.iter().any(|s| s.chars().count() > MAX_SHOWN_CHARS)
        {
            return None;
        }
        // Every line is kept, as in the screen context: the approver acts on it.
        let lines: Vec<String> = shown
            .iter()
            .flat_map(|s| s.lines())
            .map(prompt::clean)
            .filter(|l| !l.is_empty())
            .collect();
        Some(Self {
            tool: PendingTool {
                name: cut(name, MAX_NAME_CHARS),
                summary: cut(lines.join("\n"), MAX_SUMMARY_CHARS),
            },
            shown,
        })
    }

    pub fn on(&self, dialog: &[String]) -> bool {
        self.shown.iter().all(|s| prompt::shows_whole(dialog, s))
    }
}

pub fn valid_session(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_SESSION_CHARS && id.chars().all(|c| c.is_ascii_graphic())
}

/// The alert context for a reported call, in the screen context's `Tool: target` form.
pub fn context(tool: &PendingTool) -> String {
    format!("{}: {}", tool.name, tool.summary)
}

/// Claude Code's user config dir, which honors CLAUDE_CONFIG_DIR.
pub fn claude_dir() -> anyhow::Result<PathBuf> {
    Ok(match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => crate::config::home_dir()?.join(".claude"),
    })
}

pub fn settings_path() -> anyhow::Result<PathBuf> {
    Ok(claude_dir()?.join("settings.json"))
}

/// Whether `collied hook` runs on `event`, in Claude Code's `settings.json` or Codex's
/// `hooks.json`, which share this layout.
pub fn installed(settings: &Value, event: &str) -> bool {
    runs(settings, event, |_| true)
}

/// A matcher naming some tools leaves the others out.
pub fn installed_for_every_tool(settings: &Value, event: &str) -> bool {
    runs(settings, event, |m| {
        m["matcher"]
            .as_str()
            .is_none_or(|m| matches!(m, "" | "*" | ".*"))
    })
}

fn runs(settings: &Value, event: &str, matcher: impl Fn(&Value) -> bool) -> bool {
    settings["hooks"][event]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| matcher(m))
        .flat_map(|m| m["hooks"].as_array().into_iter().flatten())
        .filter_map(|h| h["command"].as_str())
        .any(|c| c.trim_end().ends_with("collied hook"))
}

fn cut(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(name: &str, input: Value) -> Option<Report> {
        Report::new(name, shown(name, &input))
    }

    fn dialog(text: &str) -> Vec<String> {
        prompt::squashed_lines(text)
    }

    #[test]
    fn names_the_call_on_bounded_lines() {
        let r = report(
            "Bash",
            json!({"command": "git push \\\n  --force\u{1b}[2J\n\n", "description": "Force push"}),
        )
        .unwrap();
        assert_eq!(
            context(&r.tool),
            "Bash: git push \\\n--force[2J\nForce push"
        );
        let r = report("Edit", json!({"file_path": "/a/b.rs", "old_string": "x"})).unwrap();
        assert_eq!(context(&r.tool), "Edit: /a/b.rs");
        assert!(report("mcp__github__create_issue", json!({"title": "x"})).is_none());
        assert!(report("Bash", json!({"command": " "})).is_none());
        assert!(report("Bash", json!({"command": "x".repeat(MAX_SHOWN_CHARS + 1)})).is_none());
        let r = report("Bash", json!({"command": "x".repeat(1000)})).unwrap();
        assert_eq!(r.tool.summary.chars().count(), MAX_SUMMARY_CHARS);
        assert!(r.tool.summary.ends_with('…'));
        let r = Report::new(&"n".repeat(500), vec!["ls".into()]).unwrap();
        assert_eq!(r.tool.name.chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn a_report_names_only_a_dialog_that_shows_it() {
        let screen = " Bash command\n\n   touch /tmp/collie-test.txt && ls -l\n   /tmp/collie-test.txt\n   Create an empty test file\n\n Do you want to proceed?\n ❯ 1. Yes\n   2. No";
        let ok = report(
            "Bash",
            json!({"command": "touch /tmp/collie-test.txt && ls -l /tmp/collie-test.txt", "description": "Create an empty test file"}),
        )
        .unwrap();
        assert!(ok.on(&dialog(screen)), "wrapped lines still match");
        for (command, description) in [
            ("ls", "Create an empty test file"),
            ("touch /tmp/collie-test.txt", "Create an empty test file"),
            (
                "touch /tmp/collie-test.txt && ls -l /tmp/collie-test.txt",
                "Other",
            ),
            ("Do you want to", "Create an empty test file"),
        ] {
            let r = report(
                "Bash",
                json!({"command": command, "description": description}),
            )
            .unwrap();
            assert!(!r.on(&dialog(screen)), "{command} / {description}");
        }
    }

    #[test]
    fn the_hook_input_has_no_reason_but_names_the_dialog_that_shows_one() {
        let raw = include_str!("../tests/fixtures/claude-2.1.308/rm-reason.hook.json");
        let fields: Value = serde_json::from_str(raw).unwrap();
        assert!(
            fields
                .as_object()
                .unwrap()
                .keys()
                .all(|k| !k.contains("reason"))
        );
        let input: Input = serde_json::from_str(raw).unwrap();
        let r = report(&input.tool_name, input.tool_input).unwrap();
        let (screen, reason) = prompt::without_reason("claude", prompt::fixtures::RM_REASON);
        assert!(reason.is_some());
        assert!(r.on(&dialog(&screen)));
    }

    #[test]
    fn finds_the_hook_among_others() {
        let other = json!({"type": "command", "command": "node permissionRequest.cjs"});
        let ours = json!({"type": "command", "command": "/Users/me/.cargo/bin/collied hook", "timeout": 5});
        let settings = |hooks: Value| json!({"hooks": {"PermissionRequest": [{"matcher": ".*", "hooks": hooks}]}});
        assert!(installed(
            &settings(json!([other, ours])),
            "PermissionRequest"
        ));
        assert!(!installed(&settings(json!([other])), "PermissionRequest"));
        assert!(!installed(
            &json!({"hooks": {"PreToolUse": [{"hooks": [ours]}]}}),
            "PermissionRequest"
        ));
        assert!(!installed(&json!({}), "PermissionRequest"));
        let pre = |matcher: Value| json!({"hooks": {"PreToolUse": [{"matcher": matcher, "hooks": [ours]}]}});
        for every in [json!(null), json!(""), json!("*"), json!(".*")] {
            assert!(installed_for_every_tool(&pre(every), "PreToolUse"));
        }
        assert!(installed(&pre(json!("Bash")), "PreToolUse"));
        assert!(!installed_for_every_tool(&pre(json!("Bash")), "PreToolUse"));
        let codex = json!({"hooks": {"SessionStart": [
            {"hooks": [{"type": "command", "command": "bash '/h/.codex/herdr-agent-state.sh' session"}]},
            {"hooks": [{"type": "command", "command": "~/.cargo/bin/collied hook", "timeout": 5}]},
        ]}});
        assert!(installed(&codex, "SessionStart"));
        assert!(!installed(&codex, "PermissionRequest"));
    }

    #[test]
    fn codex_session_start_reports_only_from_a_herdr_core_terminal() {
        let start = br#"{"session_id":"01a199ed-7993-7261-8219-300d543d764b","transcript_path":"/x.jsonl","cwd":"/w","hook_event_name":"SessionStart","model":"m","permission_mode":"default","source":"startup"}"#;
        match request(start, Some("t3".into())) {
            Some(Request::Session {
                terminal_id,
                session_id,
            }) => assert_eq!(
                (terminal_id.as_str(), session_id.as_str()),
                ("t3", "01a199ed-7993-7261-8219-300d543d764b")
            ),
            other => panic!("{other:?}"),
        }
        assert!(request(start, None).is_none());
        assert!(request(start, Some(String::new())).is_none());
        let permission = br#"{"hook_event_name":"PermissionRequest","session_id":"s","tool_name":"Bash","tool_input":{"command":"ls"}}"#;
        assert!(matches!(
            request(permission, Some("t3".into())),
            Some(Request::Hook { .. })
        ));
        let stop = br#"{"hook_event_name":"Stop","session_id":"s"}"#;
        assert!(request(stop, Some("t3".into())).is_none());
        assert!(request(b"not json", Some("t3".into())).is_none());
    }

    const PRE_TOOL_USE: &str =
        include_str!("../tests/fixtures/claude-2.1.321/pre-tool-use.hook.json");

    #[test]
    fn pre_tool_use_asks_about_the_limit_with_the_turn_it_belongs_to() {
        match request(PRE_TOOL_USE.as_bytes(), None) {
            Some(Request::Limit {
                session_id,
                subagent: false,
                prompt_id,
            }) => assert_eq!(
                (session_id.as_str(), prompt_id.as_deref()),
                (
                    "85f2d085-49d3-41e6-9f4e-7b6d4be9f21c",
                    Some("f74e6cdc-2837-4093-9962-0b23e94d2b27")
                )
            ),
            other => panic!("{other:?}"),
        }
        // A subagent's call names the parent session and the prompt that started it.
        let sub = include_str!("../tests/fixtures/claude-2.1.321/pre-tool-use-subagent.hook.json");
        match request(sub.as_bytes(), None) {
            Some(Request::Limit {
                session_id,
                subagent: true,
                prompt_id: Some(_),
            }) => assert_eq!(session_id, "85f2d085-49d3-41e6-9f4e-7b6d4be9f21c"),
            other => panic!("{other:?}"),
        }
        let out = stop("at the limit");
        assert_eq!(out["continue"], false);
        assert_eq!(out["stopReason"], "at the limit");
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
    }

    fn one_reply(socket: &Path, line: &'static str) -> tokio::task::JoinHandle<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let _ = std::fs::remove_file(socket);
        let listener = tokio::net::UnixListener::bind(socket).unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).await;
            s.write_all(line.as_bytes()).await.unwrap();
        })
    }

    #[tokio::test]
    async fn the_hook_fails_open() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("control.sock");
        let raw = PRE_TOOL_USE.as_bytes();
        assert_eq!(answer(&socket, raw, None).await, None, "no collied");
        let _hung = tokio::net::UnixListener::bind(&socket).unwrap();
        let start = std::time::Instant::now();
        assert_eq!(
            answer(&socket, raw, None).await,
            None,
            "a collied that never answers"
        );
        assert!(start.elapsed() < DEADLINE + Duration::from_secs(1));
        for line in [
            "not json\n",
            "{\"type\":\"noted\"}\n",
            "{\"type\":\"limit\"}\n",
        ] {
            let server = one_reply(&socket, line);
            assert_eq!(answer(&socket, raw, None).await, None, "{line}");
            server.await.unwrap();
        }
        let server = one_reply(&socket, "{\"type\":\"limit\",\"stop\":\"stopped\"}\n");
        let out = answer(&socket, raw, None).await.unwrap();
        server.await.unwrap();
        assert_eq!(out["stopReason"], "stopped");
        assert_eq!(out["continue"], false);
    }

    #[test]
    fn session_ids_are_short_and_printable() {
        assert!(valid_session("2a1f588e-d959-4394-97f4-15a114db99fe"));
        assert!(!valid_session(""));
        assert!(!valid_session("a b"));
        assert!(!valid_session(&"a".repeat(129)));
    }
}
