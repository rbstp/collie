use std::path::{Path, PathBuf};

use super::Rex;
use super::model::{Model, PREFIX};
use crate::runtime::agents;

/// Claude Code's session for each Claude Code agent whose process or status changed: the
/// `sessionId` of the `sessions/<pid>.json` its foreground process keeps, as Rex names that
/// process, through the lookup herdr-core's agents use. Read on those events only, never per
/// listing, so a session changed without a report (`/clear`) shows at the next report.
pub(super) fn claude(model: &mut Model, sessions: Option<&Path>) {
    for (session, block, pid) in model.touched() {
        if let Some(found) = sessions.and_then(|d| crate::transcript::claude_session(d, pid)) {
            model.bind(&session, &block, pid, found);
        }
    }
}

impl Rex {
    /// Claude Code's `sessions` folder, where each process keeps `<pid>.json`.
    pub fn with_claude_sessions(mut self, dir: Option<PathBuf>) -> Self {
        self.claude_sessions = dir;
        self
    }

    /// A Codex session reported by collied's SessionStart hook from `reporter` (the hook's
    /// pid, from the control socket) for the block named by its `REX_BLOCK`. Kept only when
    /// the followed server lists that block as a Codex agent and the hook runs under the
    /// Codex process that leads it: any process of the user can name any block.
    pub fn bind(&self, id: &str, session: &str, reporter: u32) -> bool {
        let Some(block) = id.strip_prefix(PREFIX) else {
            return false;
        };
        if !crate::transcript::is_session_id(session) {
            return false;
        }
        let mut inner = self.lock();
        let Some(live) = inner.live.as_mut() else {
            return false;
        };
        let Some((rex_session, pid)) = live.model.codex(block) else {
            return false;
        };
        if !agents::reported_by(pid, reporter, agents::process) {
            return false;
        }
        live.model
            .bind(&rex_session, block, pid, session.to_owned());
        true
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::{Child, Command};
    use std::sync::Arc;

    use protocol::{AgentKind, Cwd, Response, TaskSessionsParams};
    use serde_json::{Value, json};

    use super::super::fake::{self, Fake};
    use super::super::tests::{eventually, follow, id};
    use super::*;
    use crate::drive::Driver;
    use crate::transcript::Transcripts;

    const S: &str = "session:019a7d2e-0000-7000-8000-0000000000a1";
    const CLAUDE: &str = "block:019a7d2e-0000-7000-8000-0000000000c1";
    const CODEX: &str = "block:019a7d2e-0000-7000-8000-0000000000c2";
    const OTHER: &str = "block:019a7d2e-0000-7000-8000-0000000000c3";
    const STALE: &str = "block:019a7d2e-0000-7000-8000-0000000000c4";
    /// The session in the Claude Code transcript fixture.
    const FIRST: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    const CLEARED: &str = "0a0a0a0a-0000-4000-8000-000000000002";
    const T1: &str = "2026-11-12T16:47:02.000001Z";

    struct Running(Child);

    impl Running {
        fn pid(&self) -> u32 {
            self.0.id()
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// A child of this test named `name`: `sleep` copied under that name.
    fn running(dir: &Path, name: &str) -> Running {
        let exe = dir.join(name);
        std::fs::copy("/bin/sleep", &exe).unwrap();
        Running(Command::new(&exe).arg("600").spawn().unwrap())
    }

    /// Claude Code's `<file>.json`, naming `pid` and its `session`, started at `started`.
    fn pid_file(dir: &Path, file: u32, pid: u32, session: &str, started: u64) {
        let claim =
            json!({"pid": pid, "sessionId": session, "cwd": "/work/app", "startedAt": started});
        std::fs::write(dir.join(format!("{file}.json")), claim.to_string()).unwrap();
    }

    fn session_of(rex: &Rex, block: &str) -> Option<String> {
        rex.get(&id(block)).ok()?.agent_session.map(|s| s.value)
    }

    fn add(fake: &Fake, block: &str, process: Value) {
        let mut s = fake.state();
        if s.sessions.is_empty() {
            s.sessions.push((S.to_owned(), "work".to_owned()));
        }
        s.add(S, block, process, Vec::new());
    }

    fn process_changed(block: &str, pid: u32, argv0: &str, name: &str) -> Value {
        let foreground = fake::process(pid, argv0, name)["foreground"].clone();
        fake::block_event(
            S,
            block,
            "process_changed",
            json!({ "foreground": foreground }),
        )
    }

    fn rex(fake: &Fake, sessions: &Path) -> Arc<Rex> {
        Arc::new(Rex::new(fake.socket.clone()).with_claude_sessions(Some(sessions.to_owned())))
    }

    #[tokio::test]
    async fn a_claude_code_session_comes_from_the_pid_file_of_the_blocks_foreground_process() {
        let dir = tempfile::tempdir().unwrap();
        let (me, now) = (std::process::id(), crate::now_ms());
        let other = running(dir.path(), "claude");
        let reused = running(dir.path(), "claude-reused");
        pid_file(dir.path(), me, me, FIRST, now);
        pid_file(dir.path(), other.pid(), me, FIRST, now);
        // Left by a process that ended before this pid started.
        pid_file(dir.path(), reused.pid(), reused.pid(), FIRST, 1_000);
        let fake = Fake::start().await;
        add(&fake, CLAUDE, fake::process(me, "claude", "2.1.322"));
        add(
            &fake,
            OTHER,
            fake::process(other.pid(), "claude", "2.1.322"),
        );
        add(
            &fake,
            STALE,
            fake::process(reused.pid(), "claude", "2.1.322"),
        );
        let rex = rex(&fake, dir.path());
        let _task = follow(&rex).await;
        assert_eq!(session_of(&rex, CLAUDE).as_deref(), Some(FIRST));
        assert_eq!(session_of(&rex, OTHER), None, "a file naming another pid");
        assert_eq!(session_of(&rex, STALE), None, "a stale pid file");

        // A session changed without a report (`/clear`): listings read no file, so it shows
        // at the next report.
        pid_file(dir.path(), me, me, CLEARED, now);
        assert_eq!(session_of(&rex, CLAUDE).as_deref(), Some(FIRST));
        fake.emit(move |_| {
            vec![fake::status_changed(
                S,
                CLAUDE,
                fake::record("working", me, "2.1.322", T1),
            )]
        });
        eventually("the session after the report", || {
            session_of(&rex, CLAUDE).as_deref() == Some(CLEARED)
        })
        .await;

        // A new foreground process: its own file, from its process_changed.
        fake.emit(move |_| vec![process_changed(STALE, me, "claude", "2.1.322")]);
        eventually("the session of the new foreground process", || {
            session_of(&rex, STALE).as_deref() == Some(CLEARED)
        })
        .await;
    }

    #[tokio::test]
    async fn a_codex_session_is_kept_only_from_under_the_codex_that_leads_the_block() {
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id();
        let codex = running(dir.path(), "codex");
        let fake = Fake::start().await;
        add(&fake, CODEX, fake::process(codex.pid(), "codex", "codex"));
        add(&fake, CLAUDE, fake::process(me, "claude", "2.1.322"));
        let rex = rex(&fake, dir.path());
        let _task = follow(&rex).await;

        assert!(!rex.bind(&id(OTHER), FIRST, codex.pid()), "no such block");
        assert!(!rex.bind(CODEX, FIRST, codex.pid()), "not a Rex id");
        assert!(!rex.bind(&id(CODEX), "../x", codex.pid()));
        assert!(
            !rex.bind(&id(CODEX), FIRST, me),
            "a process the block's Codex does not run"
        );
        // This test leads CLAUDE and runs the Codex: only a Codex agent takes a hook's report.
        assert!(!rex.bind(&id(CLAUDE), FIRST, codex.pid()));
        assert_eq!(session_of(&rex, CLAUDE), None);
        assert_eq!(session_of(&rex, CODEX), None);

        assert!(rex.bind(&id(CODEX), FIRST, codex.pid()));
        assert_eq!(session_of(&rex, CODEX).as_deref(), Some(FIRST));
        fake.emit(move |_| vec![process_changed(CODEX, me, "codex", "codex")]);
        eventually("the report to lapse with its process", || {
            rex.get(&id(CODEX)).is_ok() && session_of(&rex, CODEX).is_none()
        })
        .await;
    }

    #[tokio::test]
    async fn task_sessions_leaves_out_the_session_a_rex_agent_runs() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let (me, now) = (std::process::id(), crate::now_ms());
        let sessions = root.join("sessions");
        std::fs::create_dir(&sessions).unwrap();
        pid_file(&sessions, me, me, FIRST, now);
        let cwd = root.to_str().unwrap();
        let project: String = (cwd.chars())
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let projects = root.join("projects");
        std::fs::create_dir_all(projects.join(&project)).unwrap();
        let claude = include_str!("../../../tests/fixtures/transcripts/claude.jsonl");
        for (id, text) in [
            (FIRST, claude.to_owned()),
            (CLEARED, claude.replace(FIRST, CLEARED)),
        ] {
            std::fs::write(projects.join(&project).join(format!("{id}.jsonl")), text).unwrap();
        }
        let fake = Fake::start().await;
        add(&fake, CLAUDE, fake::process(me, "claude", "2.1.322"));
        let rex = rex(&fake, &sessions);
        let _task = follow(&rex).await;
        let drive = Driver::new(
            root.join("herdr.sock"),
            vec![AgentKind::new("claude").unwrap()],
            std::slice::from_ref(&root),
        )
        .unwrap()
        .with_transcripts(Transcripts::new(Some(projects), None))
        .with_rex(Some(rex));
        let auth: crate::drive::Authorized = Arc::new(|| true);
        let p = TaskSessionsParams {
            cwd: Cwd::new(cwd).unwrap(),
        };
        let Ok(Response::TaskSessions { sessions }) = drive.task_sessions(p, &auth).await else {
            panic!("no sessions");
        };
        let listed: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(listed, [CLEARED]);
    }
}
