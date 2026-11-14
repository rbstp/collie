use std::path::Path;

use super::{AgentInfo, PaneInfo};

const AGENT_CLIS: [&str; 3] = ["claude", "codex", "copilot"];
const SHELLS: [&str; 9] = [
    "sh", "bash", "zsh", "fish", "dash", "ksh", "tcsh", "csh", "nu",
];

fn file_name(argv0: &str) -> Option<&str> {
    Path::new(argv0).file_name()?.to_str()
}

/// The agent CLI a foreground process's `argv[0]` names.
pub(super) fn cli(argv0: &str) -> Option<&'static str> {
    let name = file_name(argv0)?;
    AGENT_CLIS.into_iter().find(|c| *c == name)
}

/// A login shell's `argv[0]` starts with `-`.
pub(super) fn shell(argv0: &str) -> bool {
    file_name(argv0).is_some_and(|n| SHELLS.contains(&n.trim_start_matches('-')))
}

/// The kind an OSC 7501 report's `app` names, else the agent CLI the terminal runs.
pub(super) fn kind(app: Option<&str>, cli: Option<&str>) -> Option<String> {
    match app {
        Some("claude-code") => Some("claude".to_owned()),
        Some(app) if !app.is_empty() => Some(app.to_owned()),
        _ => cli.map(str::to_owned),
    }
}

/// collie's status for an OSC 7501 state; `error` and states collie does not know are
/// `unknown`.
pub(super) fn status(state: Option<&str>) -> &str {
    state
        .filter(|s| matches!(*s, "idle" | "working" | "blocked" | "done"))
        .unwrap_or("unknown")
}

/// Without the spinner or status glyph an agent puts before its title (Codex redraws one
/// every 100 ms while it works), so a spinner tick changes nothing the phone shows.
pub(super) fn title(raw: &str) -> &str {
    raw.trim_start_matches(|c: char| c.is_whitespace() || (!c.is_ascii() && !c.is_alphanumeric()))
        .trim_end()
}

/// A group's status: its most urgent agent's.
pub(super) fn group_status(agents: &[AgentInfo]) -> &str {
    let rank = |s: &str| match s {
        "blocked" => 4,
        "working" => 3,
        "done" => 2,
        "idle" => 1,
        _ => 0,
    };
    agents
        .iter()
        .map(|a| a.agent_status.as_str())
        .max_by_key(|s| rank(s))
        .unwrap_or("unknown")
}

/// The pane of an agent listed by a runtime with one terminal per pane.
pub fn pane(a: &AgentInfo) -> PaneInfo {
    PaneInfo {
        pane_id: a.pane_id.clone(),
        terminal_id: a.terminal_id.clone(),
        workspace_id: a.workspace_id.clone(),
        tab_id: a.workspace_id.clone(),
        cwd: a.cwd.clone(),
        foreground_cwd: a.foreground_cwd.clone(),
        agent: a.agent.clone(),
        label: a.name.clone(),
    }
}
