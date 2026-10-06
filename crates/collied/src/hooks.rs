use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use protocol::PendingTool;
use serde::Deserialize;
use serde_json::Value;

use crate::control::{self, Request};
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
    tool_name: String,
    #[serde(default)]
    tool_input: Value,
}

/// `collied hook`, Claude Code's PermissionRequest hook. It only reports the call to the
/// daemon and never answers it: it prints nothing and exits 0, so Claude Code shows its
/// dialog as usual, even when collied is not running.
pub async fn run(control_path: &Path) {
    let mut stdin = std::io::stdin();
    let mut raw = Vec::new();
    let read = (&mut stdin).take(MAX_INPUT).read_to_end(&mut raw);
    let _ = std::io::copy(&mut stdin, &mut std::io::sink());
    if read.is_err() {
        return;
    }
    let Ok(input) = serde_json::from_slice::<Input>(&raw) else {
        return;
    };
    if input
        .hook_event_name
        .is_some_and(|e| e != "PermissionRequest")
    {
        return;
    }
    let shown = shown(&input.tool_name, &input.tool_input);
    if Report::new(&input.tool_name, shown.clone()).is_none() {
        return;
    }
    let request = Request::Hook {
        session_id: input.session_id,
        tool_name: input.tool_name,
        shown,
    };
    let _ = tokio::time::timeout(DEADLINE, control::request(control_path, &request)).await;
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

pub fn installed(settings: &Value) -> bool {
    settings["hooks"]["PermissionRequest"]
        .as_array()
        .into_iter()
        .flatten()
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
    fn finds_the_hook_among_others() {
        let other = json!({"type": "command", "command": "node permissionRequest.cjs"});
        let ours = json!({"type": "command", "command": "/Users/me/.cargo/bin/collied hook", "timeout": 5});
        let settings = |hooks: Value| json!({"hooks": {"PermissionRequest": [{"matcher": ".*", "hooks": hooks}]}});
        assert!(installed(&settings(json!([other, ours]))));
        assert!(!installed(&settings(json!([other]))));
        assert!(!installed(
            &json!({"hooks": {"PreToolUse": [{"hooks": [ours]}]}})
        ));
        assert!(!installed(&json!({})));
    }

    #[test]
    fn session_ids_are_short_and_printable() {
        assert!(valid_session("2a1f588e-d959-4394-97f4-15a114db99fe"));
        assert!(!valid_session(""));
        assert!(!valid_session("a b"));
        assert!(!valid_session(&"a".repeat(129)));
    }
}
