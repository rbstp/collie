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
const DEADLINE: Duration = Duration::from_secs(2);

#[derive(Deserialize)]
struct Input {
    session_id: String,
    tool_name: String,
    #[serde(default)]
    tool_input: Value,
}

/// `collied hook`, Claude Code's PermissionRequest hook. It only reports the call to the
/// daemon and never answers it: it prints nothing and exits 0, so Claude Code shows its
/// dialog as usual, even when collied is not running.
pub async fn run(control_path: &Path) {
    let mut raw = Vec::new();
    if std::io::stdin()
        .take(MAX_INPUT)
        .read_to_end(&mut raw)
        .is_err()
    {
        return;
    }
    let Ok(input) = serde_json::from_slice::<Input>(&raw) else {
        return;
    };
    let request = Request::Hook {
        session_id: input.session_id,
        tool: pending_tool(&input.tool_name, &input.tool_input),
    };
    let _ = tokio::time::timeout(DEADLINE, control::request(control_path, &request)).await;
}

pub fn pending_tool(name: &str, input: &Value) -> PendingTool {
    let field = |k: &str| input.get(k).and_then(Value::as_str);
    let summary = match name {
        "Bash" => [field("command"), field("description")]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n"),
        "Edit" | "MultiEdit" | "Write" | "Read" => field("file_path").unwrap_or_default().into(),
        "NotebookEdit" => field("notebook_path").unwrap_or_default().into(),
        "WebFetch" => field("url").unwrap_or_default().into(),
        "WebSearch" => field("query").unwrap_or_default().into(),
        "Glob" | "Grep" => field("pattern").unwrap_or_default().into(),
        _ => String::new(),
    };
    bounded(PendingTool {
        name: name.to_owned(),
        summary,
    })
}

/// Applied again by the daemon: any process of the user can reach the control socket.
/// Every line of the summary is kept, as in the screen context: the approver acts on it.
pub fn bounded(tool: PendingTool) -> PendingTool {
    let lines: Vec<String> = tool
        .summary
        .lines()
        .map(prompt::clean)
        .filter(|l| !l.is_empty())
        .collect();
    PendingTool {
        name: cut(prompt::clean(&tool.name), MAX_NAME_CHARS),
        summary: cut(lines.join("\n"), MAX_SUMMARY_CHARS),
    }
}

pub fn valid_session(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_SESSION_CHARS && id.chars().all(|c| c.is_ascii_graphic())
}

/// The alert context for a hook-reported call, in the screen context's `Tool: target` form.
pub fn context(tool: &PendingTool) -> String {
    if tool.summary.is_empty() {
        tool.name.clone()
    } else {
        format!("{}: {}", tool.name, tool.summary)
    }
}

/// Claude Code's user settings file, which honors CLAUDE_CONFIG_DIR.
pub fn settings_path() -> anyhow::Result<PathBuf> {
    Ok(match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => crate::config::home_dir()?.join(".claude"),
    }
    .join("settings.json"))
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

    #[test]
    fn names_the_call_on_one_bounded_line() {
        let t = pending_tool(
            "Bash",
            &json!({"command": "git push \\\n  --force\u{1b}[2J\n\n", "description": "Force push"}),
        );
        assert_eq!(context(&t), "Bash: git push \\\n--force[2J\nForce push");
        let t = pending_tool("Edit", &json!({"file_path": "/a/b.rs", "old_string": "x"}));
        assert_eq!(context(&t), "Edit: /a/b.rs");
        let t = pending_tool("mcp__github__create_issue", &json!({"title": "x"}));
        assert_eq!(context(&t), "mcp__github__create_issue");
        let t = pending_tool("Bash", &json!({"command": "x".repeat(1000)}));
        assert_eq!(t.summary.chars().count(), MAX_SUMMARY_CHARS);
        assert!(t.summary.ends_with('…'));
        let t = bounded(PendingTool {
            name: "n".repeat(500),
            summary: String::new(),
        });
        assert_eq!(t.name.chars().count(), MAX_NAME_CHARS);
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
