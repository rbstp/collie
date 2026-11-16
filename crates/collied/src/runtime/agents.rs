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

/// `reported` is the kind a status's `app` names. Claude Code's and Codex's reports were
/// checked: they report `blocked` at their permission and question dialogs. Copilot CLI
/// 1.0.106 and later reports `idle` at its folder trust prompt and pickers (#741), and other
/// programs were never checked. A report naming another CLI than the one leading the
/// terminal is a previous program's.
pub(super) fn read_only(reported: Option<&str>, cli: Option<&str>) -> bool {
    !reported.is_some_and(|k| TASK_KINDS.contains(&k) && cli.is_none_or(|c| c == k))
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

/// The hook runs under the Codex process that started the session, which must lead the
/// terminal. Codex without `--no-daemon` runs it under its shared app-server, a second
/// `codex` whose environment names whichever terminal started it, so that report is refused.
pub(super) fn reported_by(
    foreground: u32,
    reporter: u32,
    process: impl Fn(u32) -> Option<(u32, String)>,
) -> bool {
    let mut pid = reporter;
    let mut codex = 0;
    for _ in 0..16 {
        let Some((parent, name)) = process(pid) else {
            return false;
        };
        codex += usize::from(name == "codex");
        if pid == foreground {
            return codex == 1;
        }
        if parent <= 1 {
            return false;
        }
        pid = parent;
    }
    false
}

/// A process's parent and short name.
#[cfg(target_os = "macos")]
pub(super) fn process(pid: u32) -> Option<(u32, String)> {
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    // SAFETY: `info` is a writable `proc_bsdinfo` of `size` bytes, what PROC_PIDTBSDINFO fills.
    let n = unsafe {
        libc::proc_pidinfo(
            i32::try_from(pid).ok()?,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            i32::try_from(size).ok()?,
        )
    };
    if usize::try_from(n).ok()? != size {
        return None;
    }
    // SAFETY: zeroed, then filled whole by the call above.
    let info = unsafe { info.assume_init() };
    let comm: Vec<u8> = info
        .pbi_comm
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    Some((info.pbi_ppid, String::from_utf8_lossy(&comm).into_owned()))
}

#[cfg(target_os = "linux")]
pub(super) fn process(pid: u32) -> Option<(u32, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (head, rest) = stat.rsplit_once(')')?;
    let name = head.split_once('(')?.1.to_owned();
    let parent = rest.split_whitespace().nth(1)?.parse().ok()?;
    Some((parent, name))
}

/// herdr's and Claude Code's session variables, which a herdr-core core or a Rex server passes
/// from whatever started it to every terminal: herdr's hooks would report a task agent's
/// session against the starter's herdr pane, and Claude Code with `CLAUDE_CODE_CHILD_SESSION`
/// saves no transcript. Named, not matched by pattern, so variables the user chose
/// (`CLAUDE_CONFIG_DIR`) stay.
pub const STRIPPED: [&str; 16] = [
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
/// The kinds a task starts in herdr-core or Rex: those that report OSC 7501, without which
/// nothing says the agent is ready or what dialog it shows.
pub const TASK_KINDS: [&str; 2] = ["claude", "codex"];

/// A task kind's CLI by absolute path: `terminal.run` starts a template for a bare name that
/// matches one. A path holding `=` would be read as a variable by `env`.
pub fn task_cli(kind: &str) -> Option<String> {
    if !TASK_KINDS.contains(&kind) {
        return None;
    }
    // `program` joins folders under `$HOME`, which may be relative; `env` would resolve a
    // relative path against the task's folder.
    crate::usage::program(kind)
        .filter(|p| p.is_absolute())
        .and_then(|p| p.into_os_string().into_string().ok())
        .filter(|p| !p.contains('='))
}

/// The argv a task agent runs as: `env` drops each of `STRIPPED` first, since neither
/// `terminal.run`'s `env` nor Rex's options can remove a variable.
pub(super) fn task_argv(cli: &str, args: &[&str]) -> Vec<String> {
    let mut argv = vec!["/usr/bin/env".to_owned()];
    for name in STRIPPED {
        argv.extend(["-u".to_owned(), name.to_owned()]);
    }
    argv.push(cli.to_owned());
    argv.extend(args.iter().map(|a| (*a).to_owned()));
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_report_holds_only_from_the_codex_that_leads_the_terminal() {
        let table = |rows: &'static [(u32, u32, &'static str)]| {
            move |pid: u32| {
                rows.iter()
                    .find(|r| r.0 == pid)
                    .map(|r| (r.1, r.2.to_owned()))
            }
        };
        // collied hook, under sh, under the terminal's Codex (`--no-daemon`).
        let own = table(&[
            (30, 20, "collied"),
            (20, 10, "sh"),
            (10, 5, "codex"),
            (5, 1, "hc"),
        ]);
        assert!(reported_by(10, 30, own));
        // Under the shared app-server that terminal 10's Codex started.
        let shared = table(&[
            (30, 25, "collied"),
            (25, 10, "codex"),
            (10, 5, "codex"),
            (5, 1, "hc"),
        ]);
        assert!(!reported_by(10, 30, shared));
        // An npm install leads with node, which runs the native codex.
        let npm = table(&[
            (30, 15, "collied"),
            (15, 10, "codex"),
            (10, 5, "node"),
            (5, 1, "hc"),
        ]);
        assert!(reported_by(10, 30, npm));
        // From another terminal, or a vanished process.
        let other = table(&[(30, 40, "collied"), (40, 1, "codex")]);
        assert!(!reported_by(10, 30, other));
        assert!(!reported_by(10, 30, table(&[])));
    }
}
