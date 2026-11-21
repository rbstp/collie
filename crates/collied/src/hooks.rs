use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::control::{self, Request};

const MAX_INPUT: u64 = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(2);

#[derive(Deserialize)]
struct Input {
    #[serde(default)]
    hook_event_name: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default, rename = "hookEventName")]
    agy_event_name: Option<String>,
    #[serde(default, rename = "conversationId")]
    conversation_id: Option<String>,
}

// ADR-0009
pub async fn run(control_path: &Path) {
    let mut stdin = std::io::stdin();
    let mut raw = Vec::new();
    let read = (&mut stdin).take(MAX_INPUT).read_to_end(&mut raw);
    let _ = std::io::copy(&mut stdin, &mut std::io::sink());
    if read.is_err() {
        return;
    }
    let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
    let terminals: Vec<String> = [
        var("HC_TERMINAL_ID"),
        var("REX_BLOCK").map(|b| format!("{}{b}", crate::runtime::rex::PREFIX)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let (reqs, agy) = parse(&raw, &terminals);
    let send = async {
        for request in reqs {
            let _ = control::request(control_path, &request).await;
        }
    };
    let _ = tokio::time::timeout(DEADLINE, send).await;
    if agy {
        println!("{{}}");
    }
}

/// A SessionStart/PreInvocation outside a herdr-core terminal or a Rex block reports nothing:
/// herdr's own hook binds it. A terminal can inherit the other runtime's variable from the
/// server that runs it, so each is reported and collied keeps only the one whose terminal the
/// agent leads. Returns `(requests, is_antigravity_payload)`.
fn parse(raw: &[u8], terminals: &[String]) -> (Vec<Request>, bool) {
    let Ok(input) = serde_json::from_slice::<Input>(raw) else {
        return (Vec::new(), false);
    };
    if let Some(event) = input.agy_event_name.as_deref() {
        let reqs = match (event, input.conversation_id) {
            ("SessionStart" | "PreInvocation", Some(session_id)) => terminals
                .iter()
                .map(|t| Request::Session {
                    terminal_id: t.clone(),
                    session_id: session_id.clone(),
                })
                .collect(),
            _ => Vec::new(),
        };
        return (reqs, true);
    }
    let reqs = match (input.hook_event_name.as_deref(), input.session_id) {
        (Some("SessionStart"), Some(session_id)) => terminals
            .iter()
            .map(|t| Request::Session {
                terminal_id: t.clone(),
                session_id: session_id.clone(),
            })
            .collect(),
        _ => Vec::new(),
    };
    (reqs, false)
}

/// Claude Code's user config dir, which honors CLAUDE_CONFIG_DIR.
pub fn claude_dir() -> anyhow::Result<PathBuf> {
    Ok(match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => crate::config::home_dir()?.join(".claude"),
    })
}

pub fn agy_config_dir() -> anyhow::Result<PathBuf> {
    Ok(match std::env::var_os("ANTIGRAVITY_CLI_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => crate::config::home_dir()?.join(".gemini/config"),
    })
}

pub fn agy_data_dir() -> anyhow::Result<PathBuf> {
    Ok(match std::env::var_os("ANTIGRAVITY_APP_DATA_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => crate::config::home_dir()?.join(".gemini/antigravity-cli"),
    })
}

/// Whether `collied hook` runs on `event` in Codex's `hooks.json`.
pub fn installed(settings: &Value, event: &str) -> bool {
    settings["hooks"][event]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["hooks"].as_array().into_iter().flatten())
        .filter_map(|h| h["command"].as_str())
        .any(|c| c.trim_end().ends_with("collied hook"))
}

pub fn agy_installed(settings: &Value, events: &[&str]) -> bool {
    settings
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, spec)| spec["enabled"] != false)
        .flat_map(|(_, spec)| {
            events
                .iter()
                .filter_map(|e| spec.get(*e).and_then(Value::as_array))
                .flatten()
        })
        .filter_map(|h| h["command"].as_str())
        .any(|c| c.trim_end().ends_with("collied hook"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let codex = json!({"hooks": {"SessionStart": [
            {"hooks": [{"type": "command", "command": "bash '/h/.codex/herdr-agent-state.sh' session"}]},
            {"hooks": [{"type": "command", "command": "~/.cargo/bin/collied hook", "timeout": 5}]},
        ]}});
        assert!(installed(&codex, "SessionStart"));
        assert!(!installed(&codex, "PermissionRequest"));
    }

    #[test]
    fn codex_session_start_reports_only_from_a_herdr_core_terminal_or_a_rex_block() {
        let start = br#"{"session_id":"01a199ed-7993-7261-8219-300d543d764b","transcript_path":"/x.jsonl","cwd":"/w","hook_event_name":"SessionStart","model":"m","permission_mode":"default","source":"startup"}"#;
        let both = ["t3".to_owned(), "rex:block:01a1dd68".to_owned()];
        let reported = |raw: &[u8], terms: &[String]| -> (Vec<(String, String)>, bool) {
            let (reqs, agy) = parse(raw, terms);
            (
                reqs.into_iter()
                    .map(|r| match r {
                        Request::Session {
                            terminal_id,
                            session_id,
                        } => (terminal_id, session_id),
                        other => panic!("{other:?}"),
                    })
                    .collect(),
                agy,
            )
        };
        let session = "01a199ed-7993-7261-8219-300d543d764b".to_owned();
        assert_eq!(
            reported(start, &both),
            (
                vec![
                    ("t3".to_owned(), session.clone()),
                    ("rex:block:01a1dd68".to_owned(), session.clone())
                ],
                false
            )
        );
        assert!(parse(start, &[]).0.is_empty());
        let permission = br#"{"hook_event_name":"PermissionRequest","session_id":"s","tool_name":"Bash","tool_input":{"command":"ls"}}"#;
        assert_eq!(reported(permission, &both), (vec![], false));
        let unnamed = br#"{"session_id":"s","tool_name":"Bash","tool_input":{"command":"ls"}}"#;
        assert_eq!(reported(unnamed, &both), (vec![], false));
        let stop = br#"{"hook_event_name":"Stop","session_id":"s"}"#;
        assert_eq!(reported(stop, &both), (vec![], false));
        assert_eq!(reported(b"not json", &both), (vec![], false));

        for event in ["SessionStart", "PreInvocation"] {
            let agy =
                format!(r#"{{"conversationId":"{session}","hookEventName":"{event}","cwd":"/w"}}"#);
            assert_eq!(
                reported(agy.as_bytes(), &both),
                (
                    vec![
                        ("t3".to_owned(), session.clone()),
                        ("rex:block:01a1dd68".to_owned(), session.clone())
                    ],
                    true
                )
            );
        }
        let agy_stop =
            br#"{"conversationId":"fe70ebd6-1592-44a6-9aca-8ae830e7b181","hookEventName":"Stop"}"#;
        assert_eq!(reported(agy_stop, &both), (vec![], true));
    }

    #[test]
    fn finds_the_agy_hook_when_enabled() {
        let ours = json!({"type": "command", "command": "~/.cargo/bin/collied hook", "timeout": 5});
        let herdr = json!({"type": "command", "command": "bash '/h/.gemini/config/hooks/herdr-agent-state.sh' session"});
        let cfg = json!({
            "herdr": { "PreInvocation": [herdr] },
            "collie": { "PreInvocation": [ours] }
        });
        assert!(agy_installed(&cfg, &["SessionStart", "PreInvocation"]));
        let disabled = json!({
            "collie": { "enabled": false, "PreInvocation": [ours] }
        });
        assert!(!agy_installed(
            &disabled,
            &["SessionStart", "PreInvocation"]
        ));
        assert!(!agy_installed(
            &json!({}),
            &["SessionStart", "PreInvocation"]
        ));
    }
}
