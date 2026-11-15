use std::path::Path;
use std::sync::Arc;

use protocol::{ErrorCode, OpId, Response, TaskArchiveParams, TerminalId};
use serde_json::{Value, json};

use super::{B1, S1, T0, claude, eventually, follow, id};
use crate::drive::{Authorized, Driver};
use crate::runtime::herdr_core::tests::{core_of, fake_herdr, worktree};
use crate::runtime::rex::Rex;
use crate::runtime::rex::fake::{self, Fake};
use crate::runtime::rex::tasks::Task;
use crate::runtime::{Owner, Runtime, STRIPPED, TaskHost, herdr};

const NAME: &str = "collie-0a1b2c3d";
const SESSION: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

async fn followed(fake: &Fake, root: &Path) -> (Arc<Rex>, Runtime) {
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _following = follow(&rex).await;
    let rt = Runtime::new(root.join("no-herdr.sock")).with_rex(Some(rex.clone()));
    (rex, rt)
}

async fn host(rt: &Runtime) -> TaskHost {
    let host = rt.task_host().await.unwrap().expect("a task host");
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
    let args = herdr::agent_args("claude", None, false);
    let started = host
        .start(
            "/w",
            NAME,
            Some("fix it"),
            "claude",
            None,
            "/opt/bin/claude",
            &args,
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
    assert_eq!(STRIPPED.len(), 16);
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
            &herdr::agent_args("codex", None, false),
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
        let args = herdr::agent_args(kind, resume, ultracode);
        host.start(hostile, NAME, None, kind, None, &cli, &args)
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
        &[hostile, "-x"],
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
}

#[tokio::test]
async fn tasks_start_in_herdr_then_herdr_core_then_rex() {
    let fake = Fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let (rex, rt) = followed(&fake, &root).await;
    assert!(matches!(rt.task_host().await, Ok(Some(TaskHost::Rex(_)))));

    let core = core_of(&root, json!([]));
    let both = Runtime::new(root.join("no-herdr.sock"))
        .with_core(Some(core))
        .with_rex(Some(rex.clone()));
    assert!(matches!(
        both.task_host().await,
        Ok(Some(TaskHost::Core(_)))
    ));

    let unfollowed = Runtime::new(root.join("no-herdr.sock"))
        .with_rex(Some(Arc::new(Rex::new(fake.socket.clone()))));
    assert!(matches!(unfollowed.task_host().await, Ok(None)));

    fake_herdr(&root.join("herdr.sock"), |_| None);
    let with_herdr = Runtime::new(root.join("herdr.sock")).with_rex(Some(rex));
    assert!(
        matches!(with_herdr.task_host().await, Ok(None)),
        "herdr answers, so the task starts there"
    );
    assert!(fake.state().created.is_empty());
}

/// Rex's calls that end sessions, windows or the server, or take a block from Rex.app.
#[test]
fn no_call_destroys_a_session_or_ends_anything_but_a_listed_block() {
    use crate::runtime::rex::client::Method;
    let names: Vec<&str> = Method::ALL.into_iter().map(Method::name).collect();
    for never in [
        "session.destroy",
        "session.close_window",
        "session.close_layer",
        "session.detach_block",
        "session.own_block",
        "server.stop",
        "com.superlogical.terminal.write",
    ] {
        assert!(!names.contains(&never), "{never}");
    }
}

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
        .start(at, NAME, None, "codex", Some(at), "/opt/bin/codex", &[])
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
            "Removed worktree checkout {at} and closed {task}."
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
        .start(at, NAME, None, "codex", Some(at), "/opt/bin/codex", &[])
        .await
        .unwrap();
    // A shell of the user's own, in a session of its own, now working in the checkout. The
    // archive reads where it works afresh, not from what the events kept.
    fake.state()
        .sessions
        .push((S1.to_owned(), "mine".to_owned()));
    shell(&fake, S1, SHELL, &repo);
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

    // Neither the shell, which is no agent, nor an id Rex does not list is ever closed.
    for other in [id(SHELL), id(B1)] {
        let (code, _) = drive.task_archive(archive(&other), &yes).await.unwrap_err();
        assert_eq!(code, ErrorCode::NotFound, "{other}");
    }
    assert!(!fake.calls().iter().any(|c| c == "block.close"));
}

#[tokio::test]
async fn a_task_without_a_worktree_is_archived_by_closing_its_block_alone() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let (rex, rt) = followed(&fake, &root).await;
    let host = host(&rt).await;
    let task = host
        .start(
            root.to_str().unwrap(),
            NAME,
            None,
            "claude",
            None,
            "/opt/bin/claude",
            &[],
        )
        .await
        .unwrap();
    eventually("the task listed", || rex.get(&task).is_ok()).await;
    let drive = driver(&root, rex);
    let yes: Authorized = Arc::new(|| true);
    let Ok(Response::TaskArchived { message }) = drive.task_archive(archive(&task), &yes).await
    else {
        panic!("not archived");
    };
    assert!(message.starts_with("Closed pane."), "{message}");
    let state = fake.state();
    assert!(!state.blocks.iter().any(|b| b.id == block_of(&task)));
    assert!(
        state.blocks.iter().any(|b| b.id == B1),
        "another agent stays"
    );
    assert_eq!(state.sessions.len(), 2, "no session is destroyed");
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
        )
        .await
        .unwrap();
    let block = block_of(&started).to_owned();
    let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(
        saved,
        json!({ block.clone(): { "name": NAME, "worktree": "/w/.worktree/x" } })
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
        .start("/w", NAME, None, "claude", None, "/opt/bin/claude", &[])
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
        .start("/w", NAME, None, "claude", None, "/opt/bin/claude", &[])
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
        .start("/w", NAME, None, "claude", None, "/opt/bin/claude", &[])
        .await
        .unwrap_err();
    assert!(failed.to_string().contains("was left running"), "{failed}");
}
