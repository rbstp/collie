use std::path::PathBuf;

use super::model::{Model, PREFIX, Read};
use super::{Inner, Rex};
use crate::runtime::agents;

impl Rex {
    /// Claude Code's `sessions` folder, where each process keeps `<pid>.json`.
    pub fn with_claude_sessions(mut self, dir: Option<PathBuf>) -> Self {
        self.claude_sessions = dir;
        self
    }

    /// A session reported by collied's hook from `reporter` (the hook's pid, from the control
    /// socket) for the block named by its `REX_BLOCK`. Kept only when the followed server
    /// lists that block as a Codex or Antigravity agent and the hook runs under the process
    /// that leads it: any process of the user can name any block.
    pub fn bind(&self, id: &str, session: &str, reporter: u32) -> bool {
        let Some(block) = id.strip_prefix(PREFIX) else {
            return false;
        };
        if !crate::transcript::is_session_id(session) {
            return false;
        }
        let mut inner = self.lock();
        let Inner { live, codex, .. } = &mut *inner;
        let Some(live) = live.as_mut() else {
            return false;
        };
        let Some((rex_session, pid, kind)) = live.model.hook_agent(block) else {
            return false;
        };
        if !agents::reported_by(pid, reporter, kind, agents::process) {
            return false;
        }
        if live.synced {
            codex.retain(|b, (bound, _)| {
                live.model
                    .hook_agent(b)
                    .is_some_and(|(_, p, _)| p == *bound)
            });
        }
        codex.insert(block.to_owned(), (pid, session.to_owned()));
        if kind == "agy" {
            drop(inner);
            let block = block.to_owned();
            self.read_files(|m| m.read_of(rex_session, block).into_iter().collect());
        } else {
            live.model
                .bind(&rex_session, block, pid, Some(session.to_owned()));
        }
        true
    }

    /// After a block's reads: the Codex session reported before a reconnect, and Claude
    /// Code's from its pid file.
    pub(super) fn read_sessions(&self, session: &str, block: &str) {
        {
            let mut inner = self.lock();
            let Inner { live, codex, .. } = &mut *inner;
            if let (Some(live), Some((pid, value))) = (live.as_mut(), codex.get(block)) {
                live.model.bind(session, block, *pid, Some(value.clone()));
            }
        }
        self.read_files(Model::touched);
    }

    /// Once per reconcile tick, never per listing: `/clear` rewrites Claude Code's pid file
    /// with no report and no block event. At most one small file read per Claude Code agent.
    pub fn reread_sessions(&self) {
        self.read_files(|m| m.reads());
    }

    pub(super) fn read_files(&self, which: impl FnOnce(&mut Model) -> Vec<Read>) {
        let wanted: Vec<_> = {
            let mut inner = self.lock();
            let Inner { live, codex, .. } = &mut *inner;
            let Some(live) = live.as_mut() else {
                return;
            };
            (which(&mut live.model).into_iter())
                .map(|r| {
                    let hook = (codex.get(&r.1))
                        .filter(|(pid, _)| *pid == r.2)
                        .map(|(_, s)| s.clone());
                    (r, hook)
                })
                .collect()
        };
        let roots = &self.agent_roots;
        let found: Vec<_> = (wanted.into_iter())
            .filter_map(|((session, block, pid, kind, cwd), hook)| {
                let value = match kind {
                    "claude" => Some(crate::transcript::claude_session(
                        self.claude_sessions.as_deref()?,
                        pid,
                    )?),
                    "copilot" => (roots.copilot.as_deref())
                        .zip(cwd.as_deref())
                        .and_then(|(root, cwd)| crate::transcript::copilot_session(root, pid, cwd)),
                    _ => agents::agy_session(
                        hook.as_deref(),
                        &(self.presence_dir())
                            .and_then(|d| agents::presence(&d, &agents::open_files(pid)))
                            .unwrap_or_default(),
                    ),
                };
                Some((session, block, pid, value))
            })
            .collect();
        if found.is_empty() {
            return;
        }
        if let Some(live) = self.lock().live.as_mut() {
            for (session, block, pid, value) in found {
                live.model.bind(&session, &block, pid, value);
            }
        }
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
    use super::super::wire::{BlockRef, Process, SessionRef};
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

        // A session changed without a report (`/clear`): listings read no file, and the
        // reconcile tick's read finds it.
        pid_file(dir.path(), me, me, CLEARED, now);
        assert_eq!(session_of(&rex, CLAUDE).as_deref(), Some(FIRST));
        rex.reread_sessions();
        assert_eq!(session_of(&rex, CLAUDE).as_deref(), Some(CLEARED));

        // And a report finds it.
        pid_file(dir.path(), me, me, FIRST, now);
        fake.emit(move |_| {
            vec![fake::status_changed(
                S,
                CLAUDE,
                fake::record("working", me, "2.1.322", T1),
            )]
        });
        eventually("the session after the report", || {
            session_of(&rex, CLAUDE).as_deref() == Some(FIRST)
        })
        .await;

        // A new foreground process: its own file, from its process_changed.
        fake.emit(move |_| vec![process_changed(STALE, me, "claude", "2.1.322")]);
        eventually("the session of the new foreground process", || {
            session_of(&rex, STALE).as_deref() == Some(FIRST)
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
        let task = follow(&rex).await;

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

        let agy = running(dir.path(), "agy");
        let agy_block = "block:019a7d2e-0000-7000-8000-0000000000c5";
        {
            let mut s = fake.state();
            s.add(
                S,
                agy_block,
                fake::process(agy.pid(), "agy", "agy"),
                Vec::new(),
            );
        }
        fake.emit(|_| {
            vec![fake::session_event(
                S,
                json!({ "type": "block_created", "payload": {} }),
            )]
        });
        eventually("the agy block to be listed", || {
            rex.get(&id(agy_block)).is_ok()
        })
        .await;
        assert!(!rex.bind(&id(agy_block), FIRST, me));
        assert!(rex.bind(&id(agy_block), FIRST, agy.pid()));
        assert_eq!(session_of(&rex, agy_block).as_deref(), Some(FIRST));

        // A reconnect reads the block again and keeps the report.
        fake.drop_connection();
        let (_, followed) = task.await.unwrap();
        assert!(followed);
        let _task = follow(&rex).await;
        assert_eq!(session_of(&rex, CODEX).as_deref(), Some(FIRST));

        fake.emit(move |_| vec![process_changed(CODEX, me, "codex", "codex")]);
        eventually("the report to lapse with its process", || {
            rex.get(&id(CODEX)).is_ok() && session_of(&rex, CODEX).is_none()
        })
        .await;
    }

    #[tokio::test]
    async fn copilot_and_agy_sessions_come_from_what_their_process_holds_and_never_from_two() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id();
        let fake = Fake::start().await;
        add(&fake, OTHER, fake::process(me, "copilot", "copilot"));
        add(&fake, STALE, fake::process(me, "agy", "agy"));
        let root = dir.path().join("copilot/session-state");
        let roots = crate::costs::Roots {
            copilot: Some(root.clone()),
            agy: Some(dir.path().join("agy/brain")),
            ..crate::costs::Roots::default()
        };
        let rex = Arc::new(Rex::new(fake.socket.clone()).with_agent_roots(roots));
        let _task = follow(&rex).await;
        let read = |block: &str| {
            rex.reread_sessions();
            session_of(&rex, block)
        };
        assert_eq!(read(OTHER), None);

        let marked = |id: &str| {
            let session = root.join(id);
            std::fs::create_dir_all(&session).unwrap();
            let workspace = format!("id: {id}\ncwd: /work/app\n");
            std::fs::write(session.join("workspace.yaml"), workspace).unwrap();
            let start = json!({"type": "session.start", "data": {"sessionId": id}});
            std::fs::write(session.join("events.jsonl"), format!("{start}\n")).unwrap();
            std::fs::write(session.join(format!("inuse.{me}.lock")), me.to_string()).unwrap();
            let hold = std::fs::File::create(session.join(format!("inuse.{me}.hold"))).unwrap();
            hold.lock().unwrap();
            hold
        };
        let first = marked(FIRST);
        assert_eq!(read(OTHER).as_deref(), Some(FIRST));
        let second = marked(CLEARED);
        assert_eq!(read(OTHER), None, "after /new it marks both");
        drop(second);
        assert_eq!(read(OTHER).as_deref(), Some(FIRST));
        let unreadable = root.join("0a0a0a0a-0000-4000-8000-000000000009");
        std::fs::create_dir(&unreadable).unwrap();
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
        let found = read(OTHER);
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(found, None, "a session folder it cannot read");
        drop(first);

        let presence = dir.path().join("agy/presence");
        std::fs::create_dir_all(&presence).unwrap();
        let lock = |id: &str| std::fs::File::create(presence.join(format!("{id}.lock"))).unwrap();
        let own = lock(FIRST);
        assert_eq!(read(STALE).as_deref(), Some(FIRST));
        let block = STALE.to_owned();
        rex.lock()
            .codex
            .insert(block.clone(), (me, CLEARED.to_owned()));
        assert_eq!(read(STALE), None, "the hook names another conversation");
        rex.lock()
            .codex
            .insert(block.clone(), (me, FIRST.to_owned()));
        let other = lock(CLEARED);
        assert_eq!(
            read(STALE).as_deref(),
            Some(FIRST),
            "the hook names an open lock"
        );
        rex.lock().codex.remove(&block);
        assert_eq!(read(STALE), None, "two locks and no hook");
        drop((own, other));
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

    #[test]
    fn a_session_found_for_a_process_that_no_longer_leads_the_block_is_refused() {
        let mut model = Model::new(Some(fake::HOST.to_owned()));
        let session = SessionRef {
            session_id: S.to_owned(),
            label: "work".to_owned(),
        };
        model.sessions(1, vec![session]);
        let refs = vec![BlockRef {
            block_id: Some(CLAUDE.to_owned()),
            creator_name: Some(super::super::client::TERMINAL.to_owned()),
            host_id: Some(fake::HOST.to_owned()),
        }];
        model.blocks(S, refs);
        let claude = |pid| -> Process {
            serde_json::from_value(fake::process(pid, "claude", "2.1.322")).unwrap()
        };
        model.process(S, CLAUDE, 1, claude(10));
        let cwd = Some("/work/app".to_owned());
        assert_eq!(
            model.touched(),
            [(S.to_owned(), CLAUDE.to_owned(), 10, "claude", cwd)]
        );
        // Its pid file is read outside the lock; meanwhile another process leads the block
        // and its own session is found first.
        model.process(S, CLAUDE, 2, claude(20));
        model.bind(S, CLAUDE, 20, Some(CLEARED.to_owned()));
        model.bind(S, CLAUDE, 10, Some(FIRST.to_owned()));
        let session = |m: &Model| m.listing().agents[0].agent_session.clone().map(|s| s.value);
        assert_eq!(session(&model).as_deref(), Some(CLEARED));
    }
}
