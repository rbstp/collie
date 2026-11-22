use std::path::Path;
use std::sync::Arc;

use protocol::{
    AgentKind, Cwd, ErrorCode, OpId, PromptText, Response, TaskArchiveParams, TaskNewParams,
    TaskWorktreesParams, TerminalId,
};
use serde_json::{Value, json};
use tokio::io::AsyncBufReadExt;

use super::{B1, S1, T0, claude, eventually, follow, id};
use crate::drive::{Authorized, Driver, ELSEWHERE, STARTUP_PROMPT};
use crate::runtime::herdr_core::tests::{
    ID, answered, codex_in, core_of, fake_core, fake_herdr, terminal, worktree, worktree_task,
};
use crate::runtime::rex::Rex;
use crate::runtime::rex::fake::{self, Fake};
use crate::runtime::rex::tasks::Task;
use crate::runtime::{Error, Owner, Runtime, STRIPPED, TaskHost, herdr, pane};

const NAME: &str = "collie-0a1b2c3d";
const SESSION: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

async fn followed(fake: &Fake, root: &Path) -> (Arc<Rex>, Runtime) {
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _following = follow(&rex).await;
    let rt = Runtime::new(root.join("no-herdr.sock")).with_rex(Some(rex.clone()));
    (rex, rt)
}

async fn host(rt: &Runtime) -> TaskHost {
    let host = rt.task_host(None).await.unwrap().expect("a task host");
    assert_eq!(host.owner(), Owner::Rex);
    host
}

/// `env` with every stripped name, then the CLI and its arguments.
fn through_env(cli: &str, args: &[&str]) -> Vec<String> {
    let mut argv = vec!["/usr/bin/env".to_owned()];
    for name in STRIPPED {
        argv.extend(["-u".to_owned(), name.to_owned()]);
    }
    argv.push(cli.to_owned());
    argv.extend(args.iter().map(|a| (*a).to_owned()));
    argv
}

fn block_of(id: &str) -> &str {
    id.strip_prefix("rex:").unwrap()
}

#[tokio::test]
async fn a_task_starts_in_a_session_of_its_own_through_env_and_its_block_is_given_up() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (rex, rt) = followed(&fake, dir.path()).await;
    let host = host(&rt).await;
    let args = herdr::agent_args("claude", None, false, None);
    let started = host
        .start(
            "/w",
            NAME,
            Some("fix it"),
            "claude",
            None,
            "/opt/bin/claude",
            &args,
            None,
        )
        .await
        .unwrap();

    let created = fake.state().created.clone();
    assert_eq!(created.len(), 1);
    assert_eq!(
        created[0],
        json!({
            "label": "fix it",
            "initial_windows": [{ "layout": { "block": {
                "flavor": "com.superlogical.terminal.shell",
                "label": NAME,
                "options": { "command": through_env("/opt/bin/claude", &[]), "cwd": "/w" },
            }}}],
        }),
        "the user's login shell runs the command: no `shell` or `login` option"
    );
    assert_eq!(STRIPPED.len(), 17);
    let block = block_of(&started).to_owned();
    assert_eq!(
        fake.state().block(&block).owner,
        None,
        "the block was given up"
    );
    let disowned = fake
        .calls()
        .iter()
        .filter(|c| *c == "session.disown_block")
        .count();
    assert_eq!(disowned, 1);

    eventually("the task listed", || rex.get(&started).is_ok()).await;
    let a = rex.get(&started).unwrap();
    assert_eq!(a.name.as_deref(), Some(NAME));
    assert_eq!(a.agent.as_deref(), Some("claude"));
    let session = fake.state().block(&block).session.clone();
    assert_eq!(a.workspace_id, format!("rex:{session}"));
    assert_eq!(
        rex.lock().tasks.get(&block),
        Some(&Task {
            name: NAME.to_owned(),
            worktree: None,
            session: Some(session.clone()),
        })
    );

    // Without a label the session is named after the task.
    let codex = host
        .start(
            "/w",
            "collie-0a1b2c3e",
            None,
            "codex",
            None,
            "/opt/bin/codex",
            &herdr::agent_args("codex", None, false, None),
            None,
        )
        .await
        .unwrap();
    assert_eq!(fake.state().created[1]["label"], "collie-0a1b2c3e");
    eventually("Codex listed", || rex.get(&codex).is_ok()).await;
    assert_eq!(rex.get(&codex).unwrap().agent.as_deref(), Some("codex"));
}

#[tokio::test]
async fn each_kind_and_resume_runs_as_one_argument_vector_whatever_its_values_hold() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (_rex, rt) = followed(&fake, dir.path()).await;
    let host = host(&rt).await;
    let hostile = "a b'\"; $(touch /tmp/x) `id`\n-n";
    let cases: [(&str, Option<&str>, bool, &[&str]); 5] = [
        ("claude", None, false, &[]),
        ("claude", Some(SESSION), false, &["--resume", SESSION]),
        (
            "claude",
            None,
            true,
            &["--settings", r#"{"ultracode":true}"#],
        ),
        ("codex", None, false, &["--no-daemon"]),
        (
            "codex",
            Some(SESSION),
            false,
            &["resume", SESSION, "--no-daemon"],
        ),
    ];
    for (n, (kind, resume, ultracode, expected)) in cases.into_iter().enumerate() {
        let cli = format!("/opt/my tools/{kind}");
        let args = herdr::agent_args(kind, resume, ultracode, None);
        host.start(hostile, NAME, None, kind, None, &cli, &args, None)
            .await
            .unwrap();
        let options =
            fake.state().created[n]["initial_windows"][0]["layout"]["block"]["options"].clone();
        assert_eq!(
            options["command"],
            json!(through_env(&cli, expected)),
            "{kind}"
        );
        assert_eq!(
            options["cwd"], hostile,
            "the folder is a value, never a word"
        );
    }
    // A hostile argument reaches the program as one element, unchanged.
    host.start(
        "/w",
        NAME,
        None,
        "claude",
        None,
        "/opt/bin/claude",
        &[hostile.to_owned(), "-x".to_owned()],
        None,
    )
    .await
    .unwrap();
    let command =
        fake.state().created[5]["initial_windows"][0]["layout"]["block"]["options"]["command"]
            .clone();
    assert_eq!(
        command,
        json!(through_env("/opt/bin/claude", &[hostile, "-x"]))
    );
    for (n, on) in [(6, true), (7, false)] {
        host.start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            Some(on),
        )
        .await
        .unwrap();
        let mut expected = through_env("/opt/bin/claude", &[]);
        expected.insert(
            expected.len() - 1,
            format!("CLAUDE_CODE_EXPERIMENTAL_CODE_MODE={}", u8::from(on)),
        );
        assert_eq!(
            fake.state().created[n]["initial_windows"][0]["layout"]["block"]["options"]["command"],
            json!(expected)
        );
    }
}

#[tokio::test]
async fn tasks_start_in_herdr_then_herdr_core_then_rex() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let (rex, rt) = followed(&fake, &root).await;
    assert!(matches!(
        rt.task_host(None).await,
        Ok(Some(TaskHost::Rex(_)))
    ));

    let core = core_of(&root, json!([]));
    let both = Runtime::new(root.join("no-herdr.sock"))
        .with_core(Some(core))
        .with_rex(Some(rex.clone()));
    assert!(matches!(
        both.task_host(None).await,
        Ok(Some(TaskHost::Core(_)))
    ));

    // A core that answered and lost its connection is there but not answering: an error, not
    // a reason to start in Rex. One collied refused to follow is not waited for.
    let waiting = Runtime::new(root.join("no-herdr.sock"))
        .with_core(Some(answered(&root, false)))
        .with_rex(Some(rex.clone()));
    assert!(matches!(
        waiting.task_host(None).await,
        Err(Error::Unsupported(m)) if m.starts_with("Herdr Core is reconnecting")
    ));
    let refused = Runtime::new(root.join("no-herdr.sock"))
        .with_core(Some(answered(&root, true)))
        .with_rex(Some(rex.clone()));
    assert!(matches!(
        refused.task_host(None).await,
        Ok(Some(TaskHost::Rex(_)))
    ));

    let unfollowed = Runtime::new(root.join("no-herdr.sock"))
        .with_rex(Some(Arc::new(Rex::new(fake.socket.clone()))));
    assert!(matches!(unfollowed.task_host(None).await, Ok(None)));

    fake_herdr(&root.join("herdr.sock"), |_| None);
    let with_herdr = Runtime::new(root.join("herdr.sock")).with_rex(Some(rex));
    assert!(
        matches!(with_herdr.task_host(None).await, Ok(None)),
        "herdr answers, so the task starts there"
    );
    assert!(fake.state().created.is_empty());
}

#[tokio::test]
async fn a_chosen_runtime_is_the_only_place_a_task_starts() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let (rex, _) = followed(&fake, &root).await;
    let refused = |host: Result<Option<TaskHost>, Error>| match host {
        Err(Error::Unavailable(why)) => why,
        Err(e) => panic!("not a refusal: {e}"),
        Ok(host) => panic!("started in {:?}", host.map(|h| h.owner())),
    };
    let core = || Some(core_of(&root, json!([])));

    fake_herdr(&root.join("herdr.sock"), |_| None);
    let all = Runtime::new(root.join("herdr.sock"))
        .with_core(core())
        .with_rex(Some(rex.clone()));
    assert!(matches!(all.task_host(Some(Owner::Herdr)).await, Ok(None)));
    assert!(matches!(
        all.task_host(Some(Owner::Core)).await,
        Ok(Some(TaskHost::Core(_)))
    ));
    assert!(matches!(
        all.task_host(Some(Owner::Rex)).await,
        Ok(Some(TaskHost::Rex(_)))
    ));

    // Nothing there, a core collied refused, a Rex that lacks a method, a Rex lost.
    let bare = Runtime::new(root.join("herdr.sock"))
        .with_core(Some(Arc::new(crate::runtime::herdr_core::Core::new(
            root.join("none.sock"),
        ))))
        .with_rex(Some(Arc::new(Rex::new(root.join("no-rex.sock")))));
    assert_eq!(
        refused(bare.task_host(Some(Owner::Core)).await),
        "Herdr Core is not running on this computer"
    );
    assert_eq!(
        refused(bare.task_host(Some(Owner::Rex)).await),
        "Rex is not running on this computer"
    );
    let lacking = Fake::start().await;
    lacking
        .state()
        .methods
        .retain(|m| *m != "session.list_blocks");
    let unfollowed = Arc::new(Rex::new(lacking.socket.clone()));
    assert!(!unfollowed.attempt().await.1);
    let refusing = Runtime::new(root.join("herdr.sock"))
        .with_core(Some(answered(&root, true)))
        .with_rex(Some(unfollowed));
    let why = refused(refusing.task_host(Some(Owner::Core)).await);
    assert!(
        why.starts_with(
            "collied does not follow Herdr Core on this computer: speaks protocol generation"
        ),
        "{why}"
    );
    assert_eq!(
        refused(refusing.task_host(Some(Owner::Rex)).await),
        "collied does not follow Rex on this computer: lacks session.list_blocks"
    );
    let dropping = Fake::start().await;
    let lost = Arc::new(Rex::new(dropping.socket.clone()));
    let attempt = follow(&lost).await;
    dropping.drop_connection();
    assert!(attempt.await.unwrap().1, "followed before it was lost");
    let reconnecting = Runtime::new(root.join("herdr.sock")).with_rex(Some(lost));
    assert_eq!(
        refused(reconnecting.task_host(Some(Owner::Rex)).await),
        "Rex is reconnecting; try again in a moment"
    );

    // A reconnecting core holds Automatic back, not a task asked of Rex.
    let waiting = Runtime::new(root.join("no-herdr.sock"))
        .with_core(Some(answered(&root, false)))
        .with_rex(Some(rex.clone()));
    assert!(matches!(
        waiting.task_host(None).await,
        Err(Error::Unsupported(_))
    ));
    assert_eq!(
        refused(waiting.task_host(Some(Owner::Herdr)).await),
        "Herdr is not running on this computer"
    );
    assert_eq!(
        refused(waiting.task_host(Some(Owner::Core)).await),
        "Herdr Core is reconnecting; try again in a moment"
    );
    assert!(matches!(
        waiting.task_host(Some(Owner::Rex)).await,
        Ok(Some(TaskHost::Rex(_)))
    ));

    // A herdr that runs but does not answer fails Herdr with its own error, not a refusal.
    let closing = root.join("closing.sock");
    let listener = tokio::net::UnixListener::bind(&closing).unwrap();
    let closer = tokio::spawn(async move {
        // Read first: Linux resets a socket closed with data unread.
        while let Ok((stream, _)) = listener.accept().await {
            let mut stream = tokio::io::BufReader::new(stream);
            let _ = stream.read_until(b'\n', &mut Vec::new()).await;
        }
    });
    assert!(matches!(
        Runtime::new(closing).task_host(Some(Owner::Herdr)).await,
        Err(Error::Closed)
    ));
    closer.abort();

    // One that never answers would hold a task for its 5 s timeout: one asked of the core or
    // Rex never waits on herdr.
    let busy = root.join("busy.sock");
    let listener = tokio::net::UnixListener::bind(&busy).unwrap();
    let held = tokio::spawn(async move {
        let mut open = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            open.push(stream);
        }
    });
    let stuck = Runtime::new(busy).with_core(core()).with_rex(Some(rex));
    let quick = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        let core = stuck.task_host(Some(Owner::Core)).await;
        let rex = stuck.task_host(Some(Owner::Rex)).await;
        (core, rex)
    });
    let (core, rex) = quick.await.expect("a chosen runtime waited on herdr");
    assert!(matches!(core, Ok(Some(TaskHost::Core(_)))));
    assert!(matches!(rex, Ok(Some(TaskHost::Rex(_)))));
    held.abort();
    assert!(fake.state().created.is_empty());
}

#[tokio::test]
async fn a_task_its_chosen_runtime_refuses_starts_nowhere_and_creates_no_folder() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    fake_herdr(&root.join("herdr.sock"), |_| None);
    let drive = Driver::new(
        root.join("herdr.sock"),
        vec![claude_kind()],
        std::slice::from_ref(&root),
    )
    .unwrap()
    .with_rex(Some(Arc::new(Rex::new(root.join("no-rex.sock")))));
    let p = TaskNewParams {
        op_id: OpId::new("n".repeat(22)).unwrap(),
        cwd: Cwd::new(root.to_str().unwrap()).unwrap(),
        agent: claude_kind(),
        prompt: Some(PromptText::new("go".to_owned()).unwrap()),
        label: None,
        new_folder: Some(protocol::FolderName::new("made").unwrap()),
        worktree: None,
        clone: None,
        resume: None,
        ultracode: false,
        code_mode: None,
        runtime: Some(protocol::TaskRuntime::Rex),
    };
    let yes: Authorized = Arc::new(|| true);
    let unknown = TaskNewParams {
        runtime: Some(protocol::TaskRuntime::Unknown),
        ..p.clone()
    };
    let (reply, cwd) = drive.task_new(p, &yes).await;
    assert_eq!(
        reply.unwrap_err(),
        (
            ErrorCode::InvalidParams,
            "Rex is not running on this computer".to_owned()
        )
    );
    assert!(cwd.is_none());
    let (reply, cwd) = drive.task_new(unknown, &yes).await;
    assert_eq!(
        reply.unwrap_err(),
        (
            ErrorCode::InvalidParams,
            "collied on this computer does not know that runtime".to_owned()
        )
    );
    assert!(cwd.is_none());
    assert!(!root.join("made").exists());
}

// A task asked of Rex must be possible to set up while herdr or herdr-core is the problem;
// Automatic still refuses the start as it did. The test runs again with a stand-in Claude Code
// first on its `PATH`, so `task_cli` finds one on every runner.
#[test]
fn options_and_worktrees_answer_from_a_followed_rex_while_herdr_does_not() {
    const STAND_IN: &str = "COLLIED_TEST_STAND_IN_CLAUDE";
    if std::env::var_os(STAND_IN).is_none() {
        use std::os::unix::fs::PermissionsExt;
        let tools = tempfile::tempdir().unwrap();
        let claude = tools.path().join("claude");
        std::fs::write(&claude, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = std::env::join_paths(std::iter::once(tools.path().to_path_buf()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .unwrap();
        let test = format!(
            "{}::options_and_worktrees_answer_from_a_followed_rex_while_herdr_does_not",
            module_path!().split_once("::").unwrap().1
        );
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([test.as_str(), "--exact", "--nocapture"])
            .env("PATH", path)
            .env(STAND_IN, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        print!("{stdout}");
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "child run failed: {}", out.status);
        assert!(stdout.contains("1 passed"), "child did not run the test");
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(answer_from_a_followed_rex_while_herdr_does_not());
}

async fn answer_from_a_followed_rex_while_herdr_does_not() {
    let fake = Fake::start().await;
    let (_dir, root, repo, _) = worktree();
    let (rex, _) = followed(&fake, &root).await;
    let busy = root.join("busy.sock");
    let listener = tokio::net::UnixListener::bind(&busy).unwrap();
    let closer = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
    let busy_herdr = Driver::new(busy, vec![claude_kind()], std::slice::from_ref(&root))
        .unwrap()
        .with_rex(Some(rex.clone()));
    let reconnecting = Driver::new(
        root.join("no-herdr.sock"),
        vec![claude_kind()],
        std::slice::from_ref(&root),
    )
    .unwrap()
    .with_core(Some(answered(&root, false)))
    .with_rex(Some(rex));
    let yes: Authorized = Arc::new(|| true);
    let hosts = [
        (&busy_herdr, &[protocol::TaskRuntime::Rex][..]),
        (
            &reconnecting,
            &[protocol::TaskRuntime::HerdrCore, protocol::TaskRuntime::Rex],
        ),
    ];
    for (drive, hosts) in hosts {
        let Ok(Response::TaskOptions(o)) = drive.task_options().await else {
            panic!("no task options");
        };
        assert_eq!(o.agents, [claude_kind()]);
        assert_eq!(o.runtimes, Some(vec![protocol::TaskRuntime::Rex]));
        // Without herdr's answer, no kinds for herdr.
        let starts: Vec<_> = (hosts.iter())
            .map(|&runtime| protocol::RuntimeAgents {
                runtime,
                agents: vec![claude_kind()],
            })
            .collect();
        assert_eq!(o.runtime_agents, starts);
        let listed = TaskWorktreesParams {
            cwd: Cwd::new(repo.to_str().unwrap()).unwrap(),
        };
        assert!(matches!(
            drive.task_worktrees(listed, &yes).await,
            Ok(Response::TaskWorktrees { .. })
        ));
    }
    let automatic = TaskNewParams {
        op_id: OpId::new("n".repeat(22)).unwrap(),
        cwd: Cwd::new(repo.to_str().unwrap()).unwrap(),
        agent: claude_kind(),
        prompt: Some(PromptText::new("go".to_owned()).unwrap()),
        label: None,
        new_folder: None,
        worktree: None,
        clone: None,
        resume: None,
        ultracode: false,
        code_mode: None,
        runtime: None,
    };
    let (reply, _) = busy_herdr.task_new(automatic.clone(), &yes).await;
    assert_eq!(reply.unwrap_err().0, ErrorCode::HerdrUnavailable);
    let (reply, _) = reconnecting.task_new(automatic, &yes).await;
    assert_eq!(
        reply.unwrap_err(),
        (
            ErrorCode::Internal,
            "Herdr Core is reconnecting; try again in a moment".to_owned()
        )
    );
    closer.abort();
    assert!(fake.state().created.is_empty());
}

/// Rex's calls that end sessions, windows or the server, or take a block from Rex.app.
fn archive(id: &str) -> TaskArchiveParams {
    TaskArchiveParams {
        op_id: OpId::new("a".repeat(22)).unwrap(),
        terminal_id: TerminalId::new(id.to_owned()).unwrap(),
        confirm: true,
    }
}

fn driver(root: &Path, rex: Arc<Rex>) -> Driver {
    Driver::new(root.join("no-herdr.sock"), vec![], &[root.to_owned()])
        .unwrap()
        .with_rex(Some(rex))
}

/// A shell in `session`, in `cwd`: not an agent, and not collied's.
fn shell(fake: &Fake, session: &str, block: &str, cwd: &Path) {
    let p = json!({ "pid": 40, "user_id": 501, "name": "zsh", "argv0": "-zsh", "cwd": cwd });
    fake.state().add(
        session,
        block,
        json!({ "last_exit": null, "child": p, "foreground": p }),
        vec![],
    );
}

const SHELL: &str = "block:019a7d2e-0000-7000-8000-0000000000e1";

#[tokio::test]
async fn archive_closes_only_the_tasks_block_and_removes_its_recorded_worktree() {
    let (_dir, root, repo, checkout) = worktree();
    let fake = Fake::start().await;
    let (rex, rt) = followed(&fake, &root).await;
    let host = host(&rt).await;
    let at = checkout.to_str().unwrap();
    let task = host
        .start(
            at,
            NAME,
            None,
            "codex",
            Some(at),
            "/opt/bin/codex",
            &[],
            None,
        )
        .await
        .unwrap();
    let block = block_of(&task).to_owned();
    let session = fake.state().block(&block).session.clone();
    // Another block in the task's own session, working elsewhere, stays.
    shell(&fake, &session, SHELL, &repo);
    fake.emit({
        let session = session.clone();
        move |_| {
            vec![fake::session_event(
                &session,
                json!({ "type": "block_created", "payload": {} }),
            )]
        }
    });
    eventually("the task listed", || rex.get(&task).is_ok()).await;
    let drive = driver(&root, rex.clone());

    // Authorized while the checkout exists, so `gh poi` is skipped after the removal.
    let held = checkout.clone();
    let auth: Authorized = Arc::new(move || held.exists());
    let Ok(Response::TaskArchived { message }) = drive.task_archive(archive(&task), &auth).await
    else {
        panic!("not archived");
    };
    assert!(
        message.starts_with(&format!(
            "Removed worktree checkout {at} and closed {task}. Kept its Rex session, which still holds something or could not be read."
        )),
        "{message}"
    );
    assert!(!checkout.exists());
    let state = fake.state();
    assert!(!state.blocks.iter().any(|b| b.id == block));
    assert!(
        state.blocks.iter().any(|b| b.id == SHELL),
        "the other block stays"
    );
    assert!(
        state.sessions.iter().any(|(s, _)| *s == session),
        "the session stays"
    );
    assert!(!state.calls.iter().any(|c| c.contains("destroy")));
    assert_eq!(
        state.calls.iter().filter(|c| *c == "block.close").count(),
        1
    );
}

#[tokio::test]
async fn archive_closes_nothing_while_anything_else_works_in_the_checkout() {
    let (_dir, root, repo, checkout) = worktree();
    let fake = Fake::start().await;
    let (rex, rt) = followed(&fake, &root).await;
    let host = host(&rt).await;
    let at = checkout.to_str().unwrap();
    let task = host
        .start(
            at,
            NAME,
            None,
            "codex",
            Some(at),
            "/opt/bin/codex",
            &[],
            None,
        )
        .await
        .unwrap();
    // A shell of the user's own, in a session of its own, now working in the checkout. The
    // archive reads where it works afresh, not from what the events kept.
    fake.state()
        .sessions
        .push((S1.to_owned(), "mine".to_owned()));
    // As Rex.app runs it: under `login`, which reports no folder.
    let zsh = json!({ "pid": 40, "user_id": 501, "name": "zsh", "argv0": "-zsh", "cwd": repo });
    let login = json!({ "pid": 39, "user_id": 0, "name": "login" });
    let p = json!({ "last_exit": null, "child": login, "foreground": zsh });
    fake.state().add(S1, SHELL, p, vec![]);
    fake.emit(|_| vec![fake::session_created(S1, "mine")]);
    eventually("the task listed", || rex.get(&task).is_ok()).await;
    fake.state().block(SHELL).process["foreground"]["cwd"] = json!(checkout.join("src"));
    let drive = driver(&root, rex.clone());
    let yes: Authorized = Arc::new(|| true);

    let (code, message) = drive.task_archive(archive(&task), &yes).await.unwrap_err();
    assert_eq!(code, ErrorCode::InvalidParams);
    assert!(message.contains("collie did not start"), "{message}");
    assert!(checkout.exists());
    assert!(!fake.calls().iter().any(|c| c == "block.close"));

    // An id Rex does not list is never closed.
    let (code, _) = drive
        .task_archive(archive(&id(B1)), &yes)
        .await
        .unwrap_err();
    assert_eq!(code, ErrorCode::NotFound);
    assert!(!fake.calls().iter().any(|c| c == "block.close"));

    // Archiving the user's shell, a terminal, closes its block alone: the task's block, its
    // checkout and its session stay. The cleanup's repository is the shell's folder, as the
    // listing last saw it, since `login` has none; authorized only while the shell's block
    // exists, so `gh poi` is skipped after the close.
    let session = fake.state().block(block_of(&task)).session.clone();
    let state = fake.state.clone();
    let open: Authorized =
        Arc::new(move || crate::lock(&state).blocks.iter().any(|b| b.id == SHELL));
    let Ok(Response::TaskArchived { message }) =
        drive.task_archive(archive(&id(SHELL)), &open).await
    else {
        panic!("the shell was not archived");
    };
    assert!(
        message.starts_with("Closed pane. Its folder was left in place.")
            && message.ends_with("Authorization ended; gh poi was skipped."),
        "{message}"
    );
    assert!(checkout.exists());
    let state = fake.state();
    assert!(!state.blocks.iter().any(|b| b.id == SHELL));
    assert!(state.blocks.iter().any(|b| b.id == block_of(&task)));
    assert!(state.sessions.iter().any(|(s, _)| *s == session));
    assert!(!state.calls.iter().any(|c| c.contains("destroy")));
    assert_eq!(
        state.calls.iter().filter(|c| *c == "block.close").count(),
        1
    );
}

#[tokio::test]
async fn a_task_without_a_worktree_is_archived_with_the_session_collied_created_for_it() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let file = dir.path().join("rex-tasks.json");
    let rex = Arc::new(Rex::new(fake.socket.clone()).with_tasks(file.clone()));
    let following = follow(&rex).await;
    let rt = Runtime::new(root.join("no-herdr.sock")).with_rex(Some(rex.clone()));
    let task = host(&rt)
        .await
        .start(
            root.to_str().unwrap(),
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap();
    let session = fake.state().block(block_of(&task)).session.clone();

    // A collied started again knows the session as its own from the file.
    fake.drop_connection();
    let _ = following.await;
    let again = Arc::new(Rex::new(fake.socket.clone()).with_tasks(file));
    let _following = follow(&again).await;
    eventually("the task listed", || again.get(&task).is_ok()).await;
    let drive = driver(&root, again.clone());
    let yes: Authorized = Arc::new(|| true);
    let Ok(Response::TaskArchived { message }) = drive.task_archive(archive(&task), &yes).await
    else {
        panic!("not archived");
    };
    assert!(
        message.starts_with("Closed pane. Its folder was left in place. Removed its Rex session."),
        "{message}"
    );
    {
        let state = fake.state();
        assert!(!state.blocks.iter().any(|b| b.id == block_of(&task)));
        assert!(!state.sessions.iter().any(|(s, _)| *s == session));
        assert!(
            state.blocks.iter().any(|b| b.id == B1),
            "another agent stays"
        );
    }
    eventually("the session's group to leave", || {
        !again
            .listing()
            .unwrap()
            .agents
            .iter()
            .any(|a| a.workspace_id == format!("rex:{session}"))
    })
    .await;

    // An agent collied did not start ends alone: its session, now empty, stays.
    let Ok(Response::TaskArchived { message }) = drive.task_archive(archive(&id(B1)), &yes).await
    else {
        panic!("not archived");
    };
    assert!(!message.contains("Rex session"), "{message}");
    let state = fake.state();
    assert!(!state.blocks.iter().any(|b| b.id == B1));
    assert_eq!(state.sessions.len(), 1, "{:?}", state.sessions);
    assert_eq!(state.sessions[0].0, S1);
    assert_eq!(
        state
            .calls
            .iter()
            .filter(|c| *c == "session.destroy")
            .count(),
        1
    );
}

#[tokio::test]
async fn a_tasks_session_with_a_detached_block_or_a_window_left_is_kept() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let (rex, rt) = followed(&fake, &root).await;
    let host = host(&rt).await;
    let drive = driver(&root, rex.clone());
    let yes: Authorized = Arc::new(|| true);
    for name in [NAME, "collie-0a1b2c3e"] {
        let task = host
            .start(
                root.to_str().unwrap(),
                name,
                None,
                "claude",
                None,
                "/opt/bin/claude",
                &[],
                None,
            )
            .await
            .unwrap();
        eventually("the task listed", || rex.get(&task).is_ok()).await;
        let session = fake.state().block(block_of(&task)).session.clone();
        if name == NAME {
            // Only `session.list_blocks` shows a detached block.
            shell(&fake, &session, SHELL, &root);
            fake.state().block(SHELL).detached = true;
        } else {
            // Only the view shows this window.
            fake.state().bare_windows.push(session.clone());
        }
        let Ok(Response::TaskArchived { message }) = drive.task_archive(archive(&task), &yes).await
        else {
            panic!("not archived");
        };
        assert!(
            message.contains("Kept its Rex session"),
            "{name}: {message}"
        );
        assert!(
            fake.state().sessions.iter().any(|(s, _)| *s == session),
            "{name}"
        );
    }
    assert!(!fake.calls().iter().any(|c| c == "session.destroy"));
}

#[tokio::test]
async fn records_are_kept_on_disk_and_forgotten_once_their_block_is_gone() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rex-tasks.json");
    let rex = Arc::new(Rex::new(fake.socket.clone()).with_tasks(file.clone()));
    let task = follow(&rex).await;
    let rt = Runtime::new(dir.path().join("no-herdr.sock")).with_rex(Some(rex.clone()));
    let started = host(&rt)
        .await
        .start(
            "/w",
            NAME,
            None,
            "codex",
            Some("/w/.worktree/x"),
            "/opt/bin/codex",
            &[],
            None,
        )
        .await
        .unwrap();
    let block = block_of(&started).to_owned();
    let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(
        saved,
        json!({ block.clone(): {
            "name": NAME, "worktree": "/w/.worktree/x", "session": fake.state().block(&block).session,
        }})
    );
    eventually("the task listed", || rex.get(&started).is_ok()).await;
    assert_eq!(rex.worktree(&started).as_deref(), Some("/w/.worktree/x"));

    // A collied started again knows the task from the file.
    let again = Arc::new(Rex::new(fake.socket.clone()).with_tasks(file.clone()));
    fake.drop_connection();
    let _ = task.await;
    let task = follow(&again).await;
    assert_eq!(again.worktree(&started).as_deref(), Some("/w/.worktree/x"));
    assert_eq!(again.get(&started).unwrap().name.as_deref(), Some(NAME));

    // Closed while collied was away: forgotten at the next sync.
    fake.state().blocks.retain(|b| b.id != block);
    fake.drop_connection();
    let _ = task.await;
    let _task = follow(&again).await;
    assert_eq!(again.worktree(&started), None);
    assert!(again.lock().tasks.is_empty());
    let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved, json!({}));
}

#[tokio::test]
async fn an_agent_that_exits_is_finished_and_a_block_kept_by_collied_fails_the_start() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (rex, rt) = followed(&fake, dir.path()).await;
    let host = host(&rt).await;
    fake.state().start = None;
    let started = host
        .start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap();
    let block = block_of(&started).to_owned();
    eventually("the block read", || {
        rex.lock()
            .live
            .as_ref()
            .is_some_and(|l| l.model.has(&block))
    })
    .await;
    assert!(!host.finished(&started));
    let session = fake.state().block(&block).session.clone();
    fake.emit(move |s| {
        s.block(&block).process = json!({ "last_exit": { "exit_code": 1 } });
        vec![fake::block_event(
            &session,
            &block,
            "child_exited",
            json!({ "exit_code": 1 }),
        )]
    });
    eventually("the exit", || host.finished(&started)).await;

    // A block no client owns closes with its program, which ends the task as well.
    let closing = host
        .start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap();
    let block = block_of(&closing).to_owned();
    eventually("the block read", || {
        rex.lock()
            .live
            .as_ref()
            .is_some_and(|l| l.model.has(&block))
    })
    .await;
    assert!(!host.finished(&closing));
    fake.emit(move |s| {
        let i = s.blocks.iter().position(|b| b.id == block).unwrap();
        let b = s.blocks.remove(i);
        vec![fake::block_closed(&b.session, &b.id)]
    });
    eventually("the close", || host.finished(&closing)).await;

    // A server that keeps the block collied's: the start fails and names the agent it left.
    fake.state().keep_owner = true;
    let failed = host
        .start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap_err();
    assert!(failed.to_string().contains("was left running"), "{failed}");
}

const VT: &str =
    include_str!("../../../../tests/fixtures/rex-0.1.0-preview.19/claude-2.1.325-permission.vt");

fn claude_kind() -> AgentKind {
    AgentKind::new("claude").unwrap()
}

fn drive_in(root: &Path, rex: Arc<Rex>) -> Driver {
    Driver::new(
        root.join("no-herdr.sock"),
        vec![claude_kind()],
        &[root.to_owned()],
    )
    .unwrap()
    .with_rex(Some(rex))
}

#[tokio::test]
async fn a_rex_agent_running_in_another_folder_takes_no_prompt() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (rex, rt) = followed(&fake, dir.path()).await;
    let host = host(&rt).await;
    let drive = drive_in(dir.path(), rex.clone());
    let w = Cwd::new("/w").unwrap();
    // Rex starts a block whose folder is gone in the home folder.
    fake.state().runs_in = Some("/Users/me".to_owned());
    let home = host
        .start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap();
    let failed = drive
        .host_agent_ready(&host, &home, NAME, &claude_kind(), &w)
        .await
        .unwrap_err();
    assert_eq!(failed, (ErrorCode::AgentNotReady, ELSEWHERE.to_owned()));

    fake.state().runs_in = None;
    let there = host
        .start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap();
    let (pane, startup_blocked) = drive
        .host_agent_ready(&host, &there, NAME, &claude_kind(), &w)
        .await
        .unwrap();
    assert!(!startup_blocked);
    assert_eq!(pane.foreground_cwd.as_deref(), Some("/w"));

    fake.state().start = Some("blocked");
    let blocked = host
        .start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap();
    let (pane, startup_blocked) = drive
        .host_agent_ready(&host, &blocked, NAME, &claude_kind(), &w)
        .await
        .unwrap();
    assert!(startup_blocked);
    assert!(pane.workspace_id.starts_with("rex:session:"));
    assert_eq!(pane.terminal_id, blocked);
}

#[tokio::test]
async fn no_prompt_reaches_an_idle_rex_claude_code_that_shows_a_menu() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (rex, rt) = followed(&fake, dir.path()).await;
    let task = host(&rt)
        .await
        .start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap();
    let block = block_of(&task).to_owned();
    let session = fake.state().block(&block).session.clone();
    fake.emit(move |s| {
        s.block(&block).screen = VT.to_owned();
        s.block(&block).size = (198, 68);
        let size = json!({ "columns": 198, "rows": 68 });
        vec![fake::block_event(&session, &block, "size_changed", size)]
    });
    eventually("the dialog's size", || {
        rex.columns(&task).ok().flatten() == Some(198)
    })
    .await;
    let a = rex.get(&task).unwrap();
    assert_eq!(a.agent_status, "idle");
    let p = TaskNewParams {
        op_id: OpId::new("n".repeat(22)).unwrap(),
        cwd: Cwd::new("/w").unwrap(),
        agent: claude_kind(),
        prompt: Some(PromptText::new("go".to_owned()).unwrap()),
        label: None,
        new_folder: None,
        worktree: None,
        clone: None,
        resume: None,
        ultracode: false,
        code_mode: None,
        runtime: None,
    };
    let yes: Authorized = Arc::new(|| true);
    let drive = drive_in(dir.path(), rex.clone());
    let failed = drive
        .prompt_started(&p, &pane(&a), NAME, &task, &yes)
        .await
        .unwrap_err();
    assert_eq!(failed, (ErrorCode::AgentBlocked, STARTUP_PROMPT.to_owned()));
    let reply = drive
        .finish_host_task(&host(&rt).await, &p, &task, NAME, &p.cwd, &yes)
        .await;
    assert_eq!(
        reply,
        Ok(protocol::Response::TaskStarted {
            workspace_id: protocol::WorkspaceId::new(a.workspace_id).unwrap(),
            terminal_id: protocol::TerminalId::new(task).unwrap(),
            startup_blocked: true,
        })
    );
    assert!(!fake.calls().iter().any(|c| c == "block.event"));
}

#[tokio::test]
async fn a_rex_agent_that_exits_early_fails_its_start_at_once() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (rex, rt) = followed(&fake, dir.path()).await;
    let host = host(&rt).await;
    fake.state().start = None;
    let task = host
        .start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap();
    let block = block_of(&task).to_owned();
    fake.emit(move |s| {
        let i = s.blocks.iter().position(|b| b.id == block).unwrap();
        let b = s.blocks.remove(i);
        vec![fake::block_closed(&b.session, &b.id)]
    });
    let (drive, kind, w) = (
        drive_in(dir.path(), rex),
        claude_kind(),
        Cwd::new("/w").unwrap(),
    );
    let ready = drive.host_agent_ready(&host, &task, NAME, &kind, &w);
    let failed = tokio::time::timeout(std::time::Duration::from_secs(5), ready)
        .await
        .expect("the start ends at once, not after 30 s")
        .unwrap_err();
    assert_eq!(failed.1, "agent exited before becoming interactive");
}

fn kill_and_rm(method: &str) -> Value {
    match method {
        "terminal.kill" => json!({"address": "t1"}),
        "terminal.rm" => json!({"removed": "t1"}),
        _ => Value::Null,
    }
}

#[tokio::test]
async fn a_rex_block_in_the_checkout_refuses_a_herdr_core_tasks_archive() {
    let (_dir, root, _repo, checkout) = worktree();
    let seen = fake_core(&root.join("core.sock"), kill_and_rm).await;
    let core = core_of(
        &root,
        json!([codex_in(&checkout, worktree_task(&checkout))]),
    );
    let fake = Fake::start().await;
    fake.state()
        .sessions
        .push((S1.to_owned(), "mine".to_owned()));
    shell(&fake, S1, SHELL, &checkout);
    let (rex, _) = followed(&fake, &root).await;
    let drive = driver(&root, rex).with_core(Some(core));
    let yes: Authorized = Arc::new(|| true);
    let (code, message) = drive
        .task_archive(archive(&format!("hc:{ID}:t1")), &yes)
        .await
        .unwrap_err();
    assert_eq!(code, ErrorCode::InvalidParams);
    assert!(message.contains("collie did not start"), "{message}");
    assert!(checkout.exists());
    assert!(
        crate::lock(&seen).calls.is_empty(),
        "no terminal was closed"
    );
}

#[tokio::test]
async fn a_herdr_core_terminal_in_the_checkout_refuses_a_rex_tasks_archive() {
    let (_dir, root, repo, checkout) = worktree();
    let fake = Fake::start().await;
    let (rex, rt) = followed(&fake, &root).await;
    let at = checkout.to_str().unwrap();
    let task = host(&rt)
        .await
        .start(
            at,
            NAME,
            None,
            "codex",
            Some(at),
            "/opt/bin/codex",
            &[],
            None,
        )
        .await
        .unwrap();
    eventually("the task listed", || rex.get(&task).is_ok()).await;
    let mut shell = terminal("t2", Value::Null, "-zsh");
    shell["cwd"] = json!(repo);
    shell["foreground"]["cwd"] = json!(checkout);
    let drive = driver(&root, rex).with_core(Some(core_of(&root, json!([shell]))));
    let yes: Authorized = Arc::new(|| true);
    let (code, message) = drive.task_archive(archive(&task), &yes).await.unwrap_err();
    assert_eq!(code, ErrorCode::InvalidParams);
    assert!(message.contains("collie did not start"), "{message}");
    assert!(checkout.exists());
    assert!(!fake.calls().iter().any(|c| c == "block.close"));
}

#[tokio::test]
async fn a_lost_create_answer_or_an_unnamed_client_fails_the_start_and_says_so() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (_rex, rt) = followed(&fake, dir.path()).await;
    let host = host(&rt).await;
    // `session.attach` naming no client: nothing says the block is no longer collied's.
    {
        let mut s = fake.state();
        s.anonymous = true;
        s.keep_owner = true;
    }
    let failed = host
        .start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap_err();
    assert!(failed.to_string().contains("was left running"), "{failed}");

    fake.state().hang_up = Some("session.create");
    let failed = host
        .start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap_err();
    assert!(
        failed
            .to_string()
            .contains("an agent may have been started"),
        "{failed}"
    );
}

#[tokio::test]
async fn a_tasks_record_goes_when_its_block_closes() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rex-tasks.json");
    let rex = Arc::new(Rex::new(fake.socket.clone()).with_tasks(file.clone()));
    let _task = follow(&rex).await;
    let rt = Runtime::new(dir.path().join("no-herdr.sock")).with_rex(Some(rex.clone()));
    let started = host(&rt)
        .await
        .start(
            "/w",
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
            None,
        )
        .await
        .unwrap();
    eventually("the task listed", || rex.get(&started).is_ok()).await;
    let block = block_of(&started).to_owned();
    fake.emit(move |s| {
        let i = s.blocks.iter().position(|b| b.id == block).unwrap();
        let b = s.blocks.remove(i);
        vec![fake::block_closed(&b.session, &b.id)]
    });
    eventually("the record gone", || rex.lock().tasks.is_empty()).await;
    let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved, json!({}));
    assert!(rex.finished(&started));
}

/// herdr with one workspace, `w1`, open on the linked checkout `LINKED` names, whose pane
/// `term_1` is herdr's task.
static LINKED: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();

fn linked_herdr(root: &Path, repo: &Path, checkout: &Path) {
    LINKED
        .set((repo.display().to_string(), checkout.display().to_string()))
        .unwrap();
    fake_herdr(&root.join("herdr.sock"), |method| {
        let (repo, checkout) = LINKED.get().unwrap();
        match method {
            "worktree.list" => Some(json!({"type": "worktree_list",
                "source": {"source_checkout_path": repo},
                "worktrees": [{"path": checkout, "branch": "feat", "open_workspace_id": "w1",
                    "is_bare": false, "is_linked_worktree": true}]})),
            "session.snapshot" => Some(json!({"type": "session_snapshot", "snapshot": {
                "agents": [],
                "workspaces": [{"workspace_id": "w1", "number": 1, "label": "feat",
                    "agent_status": "idle", "worktree": {"repo_root": repo,
                    "checkout_path": checkout, "is_linked_worktree": true}}],
                "panes": [{"pane_id": "w1:p1", "terminal_id": "term_1", "workspace_id": "w1",
                    "tab_id": "w1:t1", "cwd": checkout}]}})),
            // Another path, so the archive ends there and runs no cleanup.
            "worktree.remove" => Some(json!({"type": "worktree_removed", "workspace_id": "w1",
                "force": true, "worktree": {"path": "/elsewhere", "branch": "feat",
                "open_workspace_id": null, "is_bare": false, "is_linked_worktree": true}})),
            _ => None,
        }
    });
}

fn herdr_beside(root: &Path, rex: Arc<Rex>) -> Driver {
    Driver::new(
        root.join("herdr.sock"),
        vec![claude_kind()],
        &[root.to_owned()],
    )
    .unwrap()
    .with_rex(Some(rex))
}

#[tokio::test]
async fn a_rex_block_in_the_checkout_refuses_herdrs_open_and_archive() {
    let (_dir, root, repo, checkout) = worktree();
    let fake = Fake::start().await;
    fake.state()
        .sessions
        .push((S1.to_owned(), "mine".to_owned()));
    shell(&fake, S1, SHELL, &checkout);
    let (rex, _) = followed(&fake, &root).await;
    linked_herdr(&root, &repo, &checkout);
    let drive = herdr_beside(&root, rex);
    let yes: Authorized = Arc::new(|| true);
    let p = TaskNewParams {
        op_id: OpId::new("o".repeat(22)).unwrap(),
        cwd: Cwd::new(repo.to_str().unwrap()).unwrap(),
        agent: claude_kind(),
        prompt: Some(PromptText::new("go".to_owned()).unwrap()),
        label: None,
        new_folder: None,
        worktree: Some(protocol::TaskWorktree::Open {
            path: Cwd::new(checkout.to_str().unwrap()).unwrap(),
        }),
        clone: None,
        resume: None,
        ultracode: false,
        code_mode: None,
        runtime: None,
    };
    let (reply, _) = drive.task_new(p, &yes).await;
    let why = "a Rex block works in this checkout";
    assert_eq!(
        reply.unwrap_err(),
        (ErrorCode::InvalidParams, why.to_owned())
    );
    let refused = drive.task_archive(archive("term_1"), &yes).await;
    assert_eq!(
        refused.unwrap_err(),
        (
            ErrorCode::InvalidParams,
            format!("{why}; nothing was closed")
        )
    );
    assert!(checkout.exists());
}

// herdr asked Rex nothing before tasks could start beside it: Rex failing never holds it back.
#[tokio::test]
async fn a_rex_that_does_not_list_its_blocks_never_holds_herdrs_archive_back() {
    let (_dir, root, repo, checkout) = worktree();
    let fake = Fake::start().await;
    fake.state()
        .sessions
        .push((S1.to_owned(), "mine".to_owned()));
    shell(&fake, S1, SHELL, &checkout);
    let (rex, _) = followed(&fake, &root).await;
    fake.state().hang_up = Some("session.list");
    linked_herdr(&root, &repo, &checkout);
    let yes: Authorized = Arc::new(|| true);
    let archived = herdr_beside(&root, rex)
        .task_archive(archive("term_1"), &yes)
        .await;
    let Ok(Response::TaskArchived { message }) = archived else {
        panic!("held back: {archived:?}");
    };
    assert!(message.starts_with("Herdr removed a checkout"), "{message}");
}

#[tokio::test]
async fn a_herdr_core_archive_names_rex_when_rex_does_not_list_its_blocks() {
    let (_dir, root, _repo, checkout) = worktree();
    let seen = fake_core(&root.join("core.sock"), kill_and_rm).await;
    let core = core_of(
        &root,
        json!([codex_in(&checkout, worktree_task(&checkout))]),
    );
    let fake = Fake::start().await;
    let (rex, _) = followed(&fake, &root).await;
    fake.state().hang_up = Some("session.list");
    let drive = driver(&root, rex).with_core(Some(core));
    let yes: Authorized = Arc::new(|| true);
    let (_, message) = drive
        .task_archive(archive(&format!("hc:{ID}:t1")), &yes)
        .await
        .unwrap_err();
    assert!(
        message.starts_with("Rex is not followed right now"),
        "{message}"
    );
    assert!(checkout.exists());
    assert!(
        crate::lock(&seen).calls.is_empty(),
        "no terminal was closed"
    );
}
