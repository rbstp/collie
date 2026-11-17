use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::client::Method;
use super::fake::{self, Fake};
use super::model::Model;
use super::wire::{BlockRef, Process, Record, SessionRef};
use super::*;
use crate::runtime::{Owner, Runtime};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

mod approvals;
mod input;
mod stream;
mod tasks;

pub(super) const S1: &str = "session:019a7d2e-0000-7000-8000-000000000001";
const S2: &str = "session:019a7d2e-0000-7000-8000-000000000002";
pub(super) const B1: &str = "block:019a7d2e-0000-7000-8000-0000000000b1";
pub(super) const B2: &str = "block:019a7d2e-0000-7000-8000-0000000000b2";
const B3: &str = "block:019a7d2e-0000-7000-8000-0000000000b3";
pub(super) const T0: &str = "2026-11-12T16:47:01.000001Z";
const T1: &str = "2026-11-12T16:47:02.000001Z";
const T2: &str = "2026-11-12T16:47:03.000001Z";

pub(super) fn id(block: &str) -> String {
    format!("rex:{block}")
}

/// A Claude Code in `block` of `session`, pid `pid`, that reported `state` at `at`.
pub(super) fn claude(fake: &Fake, session: &str, block: &str, pid: u32, state: &str, at: &str) {
    let mut s = fake.state();
    if !s.sessions.iter().any(|(id, _)| id == session) {
        s.sessions.push((session.to_owned(), "work".to_owned()));
    }
    s.add(
        session,
        block,
        fake::process(pid, "claude", "2.1.322"),
        vec![fake::record(state, pid, "2.1.322", at)],
    );
}

pub(super) async fn eventually(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Follows until the first reads are done; the task ends with the connection.
pub(super) async fn follow(rex: &Arc<Rex>) -> tokio::task::JoinHandle<(Error, bool)> {
    let task = tokio::spawn({
        let rex = rex.clone();
        async move { rex.attempt().await }
    });
    eventually("the first reads", || {
        rex.listing().is_some() || task.is_finished()
    })
    .await;
    task
}

fn status_of(rex: &Rex, block: &str) -> Option<(String, u64)> {
    let a = rex.get(&id(block)).ok()?;
    Some((a.agent_status, a.state_change_seq))
}

#[test]
fn the_allowlist_is_these_methods_by_these_wire_names() {
    let names: Vec<&str> = Method::ALL.into_iter().map(Method::name).collect();
    assert_eq!(
        names,
        [
            "help.all",
            "server.version",
            "server.status",
            "block_creator.list",
            "session.attach_all",
            "session.list",
            "session.attach",
            "session.list_blocks",
            "block.close",
            "com.superlogical.terminal.process",
            "com.superlogical.terminal.program_status",
            "com.superlogical.terminal.title",
            "com.superlogical.terminal.size",
            "com.superlogical.terminal.format",
            "com.superlogical.terminal.blob_read",
            "block.event",
            "session.create",
            "session.disown_block",
            "session.destroy",
            "data.connect",
        ]
    );
    let terminal: Vec<&str> = Method::ALL
        .into_iter()
        .filter_map(Method::terminal)
        .collect();
    assert_eq!(
        terminal,
        [
            "process",
            "program_status",
            "title",
            "size",
            "format",
            "blob_read"
        ]
    );
}

/// Why `attempt` and `probe` do not follow a server that closes right after the hello.
async fn refused_after_hello(code: CloseCode, reason: &'static str) -> (String, Report) {
    let fake = Fake::start().await;
    fake.state().close = Some((code, reason));
    let rex = Rex::new(fake.socket.clone());
    let (ended, followed) = rex.attempt().await;
    assert!(!followed);
    let Error::Unsupported(why) = ended else {
        panic!("{ended:?}")
    };
    let hello = fake.state().hello.clone();
    assert_eq!(
        (
            hello["api_revision"].as_u64(),
            hello["api_min_revision"].as_u64()
        ),
        (Some(13), Some(13))
    );
    assert!(matches!(probe(&fake.socket).await, Err(Error::Unsupported(w)) if w == why));
    (why, rex.report())
}

#[tokio::test]
async fn an_older_api_revision_is_refused_in_plain_words_whatever_the_server_says() {
    for reason in [
        "no common API revision: the client needs 13 or newer, this server speaks 0 through 12",
        "client too new\u{7}",
    ] {
        let (why, report) = refused_after_hello(CloseCode::Policy, reason).await;
        assert!(
            why.contains("API revision 13") && why.contains("so restart the Rex server"),
            "{why}"
        );
        assert!(why.contains(&crate::printable(reason)), "{why}");
        assert!(!why.contains('\u{7}'));
        assert_eq!(report.refused.as_deref(), Some(why.as_str()));
    }
}

#[tokio::test]
async fn a_server_that_answers_then_fails_is_running_and_not_followed() {
    let (why, report) = refused_after_hello(CloseCode::Normal, "bye").await;
    assert!(
        why.starts_with("accepted the connection, then failed"),
        "{why}"
    );
    assert!(!why.contains("API revision"), "{why}");
    assert_eq!(report.refused.as_deref(), Some(why.as_str()));
}

#[tokio::test]
async fn a_missing_method_stops_the_follow_before_any_other_call_and_is_named() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    {
        let mut s = fake.state();
        s.methods.retain(|m| *m != "session.list_blocks");
        s.terminal.retain(|m| *m != "title");
    }
    let rex = Rex::new(fake.socket.clone());
    let (ended, followed) = rex.attempt().await;
    assert!(!followed);
    let missing = "session.list_blocks, com.superlogical.terminal.title";
    assert!(
        matches!(&ended, Error::Unsupported(w) if *w == format!("lacks {missing}")),
        "{ended:?}"
    );
    assert_eq!(
        rex.report().missing,
        missing.split(", ").collect::<Vec<_>>()
    );
    assert_eq!(
        fake.calls(),
        [
            "hello",
            "help.all",
            "server.version",
            "server.status",
            "block_creator.list"
        ]
    );
    assert!(rex.listing().is_none());
}

#[tokio::test]
async fn a_listener_admitting_anonymous_callers_is_refused_and_tailnet_access_warned() {
    let fake = Fake::start().await;
    fake.state().listeners = json!([
        {"endpoint": "unix:///r.sock", "transport": "unix", "admission": "peer_credentials"},
        {"endpoint": "127.0.0.1:7297", "transport": "tcp", "admission": "anonymous"},
    ]);
    let rex = Rex::new(fake.socket.clone());
    let (ended, _) = rex.attempt().await;
    assert!(
        matches!(&ended, Error::Unsupported(w) if w.contains("anonymous callers on 127.0.0.1:7297")),
        "{ended:?}"
    );
    assert!(!fake.calls().iter().any(|c| c.starts_with("session.")));

    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    {
        let mut s = fake.state();
        s.tailscale = "running";
        s.listeners = json!([{"endpoint": "tailnet://:7297", "transport": "tailscale", "admission": "tailscale_owner"}]);
    }
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _task = follow(&rex).await;
    assert!(rex.listing().is_some());
    assert_eq!(
        rex.report().remote,
        ["built-in Tailscale", "tailnet listener tailnet://:7297"]
    );
}

#[tokio::test]
async fn it_subscribes_before_it_reads_and_keeps_what_changed_in_between() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    claude(&fake, S1, B2, 20, "idle", T0);
    {
        let mut s = fake.state();
        // A session created once the list was read: only its event names it.
        s.hook(
            Method::SessionList,
            None,
            true,
            Box::new(|s| {
                s.sessions.push((S2.to_owned(), "later".to_owned()));
                s.add(
                    S2,
                    B3,
                    fake::process(30, "claude", "2.1.322"),
                    vec![fake::record("working", 30, "2.1.322", T0)],
                );
                vec![fake::session_created(S2, "later")]
            }),
        );
        // B1 reported before its read began: the read reflects it and wins.
        s.hook(
            Method::ProgramStatus,
            Some(B1),
            false,
            Box::new(|s| {
                s.block(B1).records = vec![fake::record("done", 10, "2.1.322", T2)];
                vec![fake::status_changed(
                    S1,
                    B1,
                    fake::record("working", 10, "2.1.322", T1),
                )]
            }),
        );
        // B2 reported once its read was answered: the report is newer and wins.
        s.hook(
            Method::ProgramStatus,
            Some(B2),
            true,
            Box::new(|_| {
                vec![fake::status_changed(
                    S1,
                    B2,
                    fake::record("blocked", 20, "2.1.322", T1),
                )]
            }),
        );
    }
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _task = follow(&rex).await;
    let calls = fake.calls();
    let at = |m: &str| calls.iter().position(|c| c == m).unwrap();
    assert!(at("session.attach_all") < at("session.list"));
    assert!(at("session.list") < at("session.attach"));
    assert!(at("session.attach") < at("session.list_blocks"));
    eventually("the later session", || rex.get(&id(B3)).is_ok()).await;
    assert_eq!(status_of(&rex, B1).unwrap().0, "done");
    assert_eq!(status_of(&rex, B2).unwrap().0, "blocked");
    assert_eq!(status_of(&rex, B3).unwrap().0, "working");
    let groups = rex.listing().unwrap().groups(3);
    let shown: Vec<(&str, u32, &str)> = (groups.iter())
        .map(|g| (g.workspace_id.as_str(), g.number, g.label.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            (&*format!("rex:{S1}"), 4, "work"),
            (&*format!("rex:{S2}"), 5, "later")
        ]
    );
}

#[tokio::test]
async fn reports_move_the_state_change_seq_and_a_reconnect_reads_everything_again() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "blocked", T0);
    claude(&fake, S1, B2, 20, "idle", T0);
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let task = follow(&rex).await;
    let (_, first) = status_of(&rex, B1).unwrap();
    let (_, kept) = status_of(&rex, B2).unwrap();

    // Two identical dialogs in a row: each report its own seq.
    fake.emit(|_| {
        vec![fake::status_changed(
            S1,
            B1,
            fake::record("blocked", 10, "2.1.322", T1),
        )]
    });
    eventually("the second report", || {
        status_of(&rex, B1).unwrap().1 != first
    })
    .await;
    let (status, second) = status_of(&rex, B1).unwrap();
    assert_eq!(status, "blocked");

    fake.drop_connection();
    let (_, followed) = task.await.unwrap();
    assert!(followed);
    assert!(rex.listing().is_none());
    {
        let mut s = fake.state();
        s.block(B1).records = vec![fake::record("working", 10, "2.1.322", T2)];
        s.add(S1, B3, fake::process(30, "claude", "2.1.322"), vec![]);
    }
    let _task = follow(&rex).await;
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|c| *c == "session.attach_all")
            .count(),
        2
    );
    let (status, third) = status_of(&rex, B1).unwrap();
    assert_eq!(status, "working");
    assert!(third != second && third != first);
    assert_eq!(status_of(&rex, B2).unwrap().1, kept);
    assert_eq!(status_of(&rex, B3).unwrap(), ("unknown".to_owned(), 0));
    assert!(
        rex.get(&id(B3)).unwrap().read_only,
        "no status, so no input"
    );
}

#[tokio::test]
async fn listing_answers_from_the_model_and_rex_ids_never_reach_herdr() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    fake.state()
        .add(S1, B2, fake::process(20, "/usr/bin/top", "top"), vec![]);
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _task = follow(&rex).await;
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::new(dir.path().join("no-herdr.sock")).with_rex(Some(rex.clone()));
    let before = fake.calls().len();

    let poll = rt.poll().await.unwrap();
    assert!(!poll.herdr);
    let ids: Vec<&str> = poll.agents.iter().map(|a| a.terminal_id.as_str()).collect();
    assert_eq!(ids, [id(B1)]);
    assert_eq!(poll.agents[0].agent.as_deref(), Some("claude"));
    assert_eq!(poll.workspaces[0].number, 1);
    let snapshot = rt.session_snapshot().await.unwrap();
    assert_eq!(snapshot.panes[0].workspace_id, format!("rex:{S1}"));
    let listed = crate::flock::map_agents(&snapshot.agents, &mut Default::default(), 0, None);
    assert_eq!(listed[0].runtime, Some(protocol::TaskRuntime::Rex));
    assert!(rt.agent_list().await.unwrap().len() == 1);
    assert!(!rt.agent_get(&id(B1)).await.unwrap().read_only);
    assert_eq!(fake.calls().len(), before, "listing called the server");

    assert_eq!(Owner::of(&id(B1)), Owner::Rex);
    assert!(matches!(rt.agent_focus(&id(B1)).await, Err(Error::Unsupported(w)) if w == FOCUS));
    let group = format!("rex:{S1}");
    assert!(
        matches!(rt.workspace_close(&group).await, Err(Error::Unsupported(w)) if w == GROUP_CLOSE)
    );
    assert!(
        matches!(rt.pane_close(&id(B2)).await, Err(Error::Rex { code, .. }) if code == "not_found")
    );
    assert_eq!(
        fake.calls().len(),
        before,
        "a refused call reached the server"
    );

    rt.pane_close(&id(B1)).await.unwrap();
    assert_eq!(fake.calls().last().map(String::as_str), Some("block.close"));
    assert!(fake.state().blocks.iter().all(|b| b.id != B1));
    assert!(fake.state().blocks.iter().any(|b| b.id == B2));
    eventually("the closed block to leave", || rex.get(&id(B1)).is_err()).await;
}

#[tokio::test]
async fn a_missing_rex_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let rex = Arc::new(Rex::new(dir.path().join("rex/server.sock")));
    let (ended, followed) = rex.attempt().await;
    assert!(!followed);
    assert!(matches!(ended, Error::Io(e) if e.kind() == std::io::ErrorKind::NotFound));
    let report = rex.report();
    assert_eq!((report.version, report.refused), (None, None));
    let rt = Runtime::new(dir.path().join("no-herdr.sock")).with_rex(Some(rex));
    assert!(matches!(rt.agent_list().await, Err(Error::Io(_))));
}

#[tokio::test]
async fn a_message_over_the_limit_ends_the_connection() {
    let fake = Fake::start().await;
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let task = follow(&rex).await;
    let big = "x".repeat(2 << 20);
    fake.emit(move |_| vec![json!({ "type": "bell", "payload": { "pad": big } })]);
    let (ended, _) = task.await.unwrap();
    assert!(matches!(ended, Error::LineTooLong), "{ended:?}");
}

fn read(model: &mut Model, block: &str, host: &str, creator: &str, p: Value, records: Value) {
    let refs = vec![BlockRef {
        block_id: Some(block.to_owned()),
        creator_name: Some(creator.to_owned()),
        host_id: Some(host.to_owned()),
    }];
    for b in model.blocks(S1, refs) {
        let p: Process = serde_json::from_value(p.clone()).unwrap();
        model.process(S1, &b, 1, p);
        let records: Vec<Record> = serde_json::from_value(records.clone()).unwrap();
        model.status(S1, &b, 1, records);
    }
}

#[test]
fn a_stale_record_or_a_shell_in_front_lists_no_agent_and_other_hosts_are_skipped() {
    let mut model = Model::new(Some(fake::HOST.to_owned()));
    let session = SessionRef {
        session_id: S1.to_owned(),
        label: "work".to_owned(),
    };
    model.sessions(1, vec![session]);
    let other = "0e9d6c1f-4a3b-4a52-9c85-2f6d3f6f2f10";
    let t = super::client::TERMINAL;
    let claude = |pid| fake::process(pid, "claude", "2.1.322");
    let reported = |pid, name| json!([fake::record("working", pid, name, T0)]);
    let blocks = [
        (
            "block:0000000000000000000000000000000000a1",
            claude(10),
            reported(10, "2.1.322"),
        ),
        // The record of a program that exited: its pid, or its name for a reused pid.
        (
            "block:0000000000000000000000000000000000a2",
            fake::process(11, "vim", "vim"),
            reported(9, "vim"),
        ),
        (
            "block:0000000000000000000000000000000000a3",
            fake::process(12, "vim", "vim"),
            reported(12, "2.1.322"),
        ),
        // Stale, but an agent CLI leads the terminal: listed, with no status.
        (
            "block:0000000000000000000000000000000000a4",
            claude(13),
            reported(99, "2.1.322"),
        ),
        // A shell in front, whatever it reported.
        (
            "block:0000000000000000000000000000000000a5",
            fake::process(14, "-zsh", "zsh"),
            reported(14, "zsh"),
        ),
    ];
    for (block, p, records) in blocks {
        read(&mut model, block, fake::HOST, t, p, records);
    }
    read(
        &mut model,
        "block:0000000000000000000000000000000000a6",
        other,
        t,
        claude(15),
        reported(15, "2.1.322"),
    );
    read(
        &mut model,
        "block:0000000000000000000000000000000000a7",
        fake::HOST,
        "com.superlogical.onboarding",
        claude(16),
        reported(16, "2.1.322"),
    );
    read(
        &mut model,
        "block:../a8",
        fake::HOST,
        t,
        claude(17),
        reported(17, "2.1.322"),
    );
    let shown: Vec<(String, String, u64)> = (model.listing().agents.into_iter())
        .map(|a| (a.terminal_id, a.agent_status, a.state_change_seq))
        .collect();
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert_eq!(shown[0].0, "rex:block:0000000000000000000000000000000000a1");
    assert_eq!(shown[0].1, "working");
    assert_ne!(shown[0].2, 0);
    assert_eq!(
        shown[1],
        (
            "rex:block:0000000000000000000000000000000000a4".to_owned(),
            "unknown".to_owned(),
            0
        )
    );
}

#[test]
fn a_shell_or_what_it_runs_is_a_terminal_and_a_program_rex_started_alone_is_not() {
    let mut model = Model::new(Some(fake::HOST.to_owned()));
    let session = SessionRef {
        session_id: S1.to_owned(),
        label: "work".to_owned(),
    };
    model.sessions(1, vec![session]);
    let t = super::client::TERMINAL;
    let [zsh, vim, claude, top, away] = [
        "block:0000000000000000000000000000000000c1",
        "block:0000000000000000000000000000000000c2",
        "block:0000000000000000000000000000000000c3",
        "block:0000000000000000000000000000000000c4",
        "block:0000000000000000000000000000000000c5",
    ];
    let none = json!([]);
    // A shell in front is no agent, whatever a program that left reported.
    let reported = json!([fake::record("idle", 9, "claude", T0)]);
    read(
        &mut model,
        zsh,
        fake::HOST,
        t,
        fake::process(10, "-zsh", "zsh"),
        reported,
    );
    read(
        &mut model,
        vim,
        fake::HOST,
        t,
        fake::under_shell(11, "vim", "vim"),
        none.clone(),
    );
    let cli = fake::under_shell(12, "claude", "2.1.322");
    read(&mut model, claude, fake::HOST, t, cli, none.clone());
    let alone = fake::process(13, "/usr/bin/top", "top");
    read(&mut model, top, fake::HOST, t, alone, none.clone());
    let other = "0e9d6c1f-4a3b-4a52-9c85-2f6d3f6f2f10";
    read(
        &mut model,
        away,
        other,
        t,
        fake::process(14, "-zsh", "zsh"),
        none,
    );
    let listed = |model: &Model| {
        let l = model.listing();
        let terminals: Vec<String> = l.terminals.into_iter().map(|p| p.terminal_id).collect();
        let agents: Vec<String> = l.agents.into_iter().map(|a| a.terminal_id).collect();
        (terminals, agents)
    };
    assert_eq!(listed(&model), (vec![id(zsh), id(vim)], vec![id(claude)]));
    let pane = &model.listing().terminals[1];
    assert_eq!(
        (
            pane.workspace_id.as_str(),
            pane.agent.as_deref(),
            pane.label.as_deref()
        ),
        (format!("rex:{S1}").as_str(), None, None)
    );
    assert_eq!(pane.foreground_cwd.as_deref(), Some("/work/app"));
    assert_eq!(
        model.target(&id(zsh)).map(|t| t.3),
        Some(super::model::Listed::Terminal)
    );
    assert_eq!(model.target(&id(top)), None);
    assert_eq!(model.target(&id(away)), None);

    let event = |model: &mut Model, seq, block: &str, p: Value| {
        let e = fake::block_event(S1, block, "process_changed", p);
        model.event(seq, serde_json::from_value(e).unwrap());
    };
    // A command the phone runs keeps the terminal listed, so its watch goes on; an agent
    // started there takes it, and a program the shell `exec`s leaves the list.
    event(&mut model, 2, zsh, fake::under_shell(20, "make", "make"));
    event(&mut model, 3, vim, fake::under_shell(21, "codex", "codex"));
    assert_eq!(listed(&model), (vec![id(zsh)], vec![id(vim), id(claude)]));
    event(&mut model, 4, zsh, fake::process(10, "/usr/bin/top", "top"));
    assert_eq!(listed(&model).0, Vec::<String>::new());
}

/// `sh -c 'cd x && claude'` keeps `sh` in front, and Rex names it as the owner of the agent's
/// reports: shell input there would reach the agent past every agent check.
#[test]
fn a_shell_keeping_an_agent_under_it_is_no_terminal() {
    let mut model = Model::new(Some(fake::HOST.to_owned()));
    let session = SessionRef {
        session_id: S1.to_owned(),
        label: "work".to_owned(),
    };
    model.sessions(1, vec![session]);
    let me = std::process::id();
    let gone = {
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        child.wait().unwrap();
        child.id()
    };
    let wrapper = "block:0000000000000000000000000000000000d1";
    let idle = json!([fake::record("idle", me, "bash", T0)]);
    let sh = fake::under_shell(me, "sh", "bash");
    read(
        &mut model,
        wrapper,
        fake::HOST,
        super::client::TERMINAL,
        sh,
        idle,
    );
    // Terminals, a pane hosting an agent, and whether input can reach it.
    let listed = |model: &Model| {
        let panes = model.listing().terminals;
        let terminals = panes.iter().filter(|p| p.agent.is_none()).count();
        let hosting = panes.iter().filter(|p| p.agent.is_some()).count();
        assert!(model.listing().agents.is_empty());
        (terminals, hosting, model.target(&id(wrapper)).is_some())
    };
    // The shell in front owns the record: whatever it says, an agent may be at a dialog.
    assert_eq!(listed(&model), (0, 1, false));
    assert_eq!(
        model.listing().terminals[0].agent.as_deref(),
        Some("claude")
    );

    let report = |model: &mut Model, seq, state: &str, pid| {
        let r = fake::record(state, pid, "bash", &format!("2026-11-12T16:48:0{seq}Z"));
        let e = fake::status_changed(S1, wrapper, r);
        model.event(seq, serde_json::from_value(e).unwrap());
    };
    let front = |model: &mut Model, seq, pid, argv0: &str, name: &str| {
        let e = fake::block_event(
            S1,
            wrapper,
            "process_changed",
            fake::process(pid, argv0, name),
        );
        model.event(seq, serde_json::from_value(e).unwrap());
    };
    report(&mut model, 2, "blocked", me);
    assert_eq!(listed(&model), (0, 1, false));
    // Suspended with Ctrl+Z, the login shell leads: idle or done, it is listed for `fg`;
    // working or blocked under a live owner, it is not.
    front(&mut model, 3, 2, "-zsh", "zsh");
    report(&mut model, 4, "idle", me);
    assert_eq!(listed(&model), (1, 0, true));
    report(&mut model, 5, "working", me);
    assert_eq!(listed(&model), (0, 1, false));
    // A record whose owner is gone is a program that left.
    report(&mut model, 6, "working", gone);
    assert_eq!(listed(&model), (1, 0, true));

    // A report credited to the shell in front: a login shell, as Rex.app starts one, stays
    // listed at idle or done; a shell that is not one is hidden.
    front(&mut model, 7, me, "-zsh", "zsh");
    for (seq, state) in [(8, "idle"), (9, "done")] {
        let r = fake::record(state, me, "zsh", &format!("2026-11-12T16:48:0{seq}Z"));
        let e = fake::status_changed(S1, wrapper, r);
        model.event(seq, serde_json::from_value(e).unwrap());
        assert_eq!(listed(&model), (1, 0, true), "{state}");
    }
    front(&mut model, 10, me, "sh", "zsh");
    assert_eq!(listed(&model), (0, 1, false));
}

#[tokio::test]
async fn terminals_are_listed_per_session_from_the_model_without_reading_a_screen() {
    const S3: &str = "session:019a7d2e-0000-7000-8000-000000000003";
    const B4: &str = "block:019a7d2e-0000-7000-8000-0000000000b4";
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    {
        let mut s = fake.state();
        s.sessions.push((S2.to_owned(), "logs".to_owned()));
        s.sessions.push((S3.to_owned(), "tools".to_owned()));
        s.add(S1, B2, fake::process(20, "-zsh", "zsh"), vec![]);
        s.add(S2, B3, fake::under_shell(30, "tail", "tail"), vec![]);
        s.add(S3, B4, fake::process(40, "/usr/bin/top", "top"), vec![]);
    }
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _task = follow(&rex).await;
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::new(dir.path().join("no-herdr.sock")).with_rex(Some(rex.clone()));
    let before = fake.calls().len();

    let snap = rt.session_snapshot().await.unwrap();
    let terminals: Vec<(String, String)> = (crate::flock::map_terminals(&snap).into_iter())
        .map(|t| {
            (
                t.terminal_id.as_str().to_owned(),
                t.workspace_id.as_str().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        terminals,
        [(id(B2), format!("rex:{S1}")), (id(B3), format!("rex:{S2}"))]
    );
    let groups: Vec<(String, String, u32)> = (snap.workspaces.into_iter())
        .map(|w| (w.workspace_id, w.label, w.number))
        .collect();
    assert_eq!(
        groups,
        [
            (format!("rex:{S1}"), "work".to_owned(), 1),
            (format!("rex:{S2}"), "logs".to_owned(), 2)
        ]
    );
    let poll = rt.poll().await.unwrap();
    let agents: Vec<&str> = poll.agents.iter().map(|a| a.terminal_id.as_str()).collect();
    assert_eq!(agents, [id(B1)]);
    assert_eq!(poll.workspaces.len(), 2);
    assert!(rt.agent_get(&id(B2)).await.is_err());
    assert_eq!(fake.calls().len(), before, "listing called the server");
}

fn group(rex: &Rex, session: &str) -> Option<String> {
    let groups = rex.listing()?.groups(0);
    let id = format!("rex:{session}");
    groups
        .into_iter()
        .find(|g| g.workspace_id == id)
        .map(|g| g.label)
}

#[tokio::test]
async fn events_after_the_reads_keep_agents_and_groups_current() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    claude(&fake, S2, B2, 20, "idle", T0);
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _task = follow(&rex).await;
    let title = |rex: &Rex| rex.get(&id(B1)).ok().and_then(|a| a.title);

    fake.emit(|_| {
        let title = json!({ "title": "\u{2733} Fix the flaky test" });
        vec![fake::block_event(S1, B1, "title_changed", title)]
    });
    eventually("the title", || {
        title(&rex).as_deref() == Some("Fix the flaky test")
    })
    .await;

    fake.emit(|_| {
        let renamed = json!({ "type": "session_label_changed", "payload": { "label": "renamed" } });
        vec![fake::session_event(S1, renamed)]
    });
    eventually("the label", || {
        group(&rex, S1).as_deref() == Some("renamed")
    })
    .await;

    // A shell takes the terminal back: the record Claude Code left is not an agent's.
    fake.emit(|_| {
        let zsh = fake::process(50, "-zsh", "zsh");
        vec![fake::block_event(S1, B1, "process_changed", zsh)]
    });
    eventually("the agent to leave", || rex.get(&id(B1)).is_err()).await;

    fake.emit(|_| {
        let removed = json!({ "ids": [""], "reason": "foreground" });
        vec![
            fake::block_event(S1, B1, "program_status_removed", removed),
            fake::block_event(
                S1,
                B1,
                "process_changed",
                fake::process(10, "claude", "2.1.322"),
            ),
        ]
    });
    eventually("the agent back", || rex.get(&id(B1)).is_ok()).await;
    let back = rex.get(&id(B1)).unwrap();
    assert_eq!(
        (back.agent_status.as_str(), back.read_only),
        ("unknown", true)
    );

    fake.emit(|s| {
        s.add(
            S1,
            B3,
            fake::process(30, "claude", "2.1.322"),
            vec![fake::record("working", 30, "2.1.322", T0)],
        );
        vec![fake::session_event(
            S1,
            json!({ "type": "block_created", "payload": { "block_id": B3 } }),
        )]
    });
    eventually("the created block", || rex.get(&id(B3)).is_ok()).await;

    assert_eq!(group(&rex, S2).as_deref(), Some("work"));
    fake.emit(|s| {
        s.sessions.retain(|(id, _)| id != S2);
        vec![json!({ "type": "session_destroyed", "payload": { "session_id": S2 } })]
    });
    eventually("the destroyed session's group to leave", || {
        group(&rex, S2).is_none()
    })
    .await;
    assert!(rex.get(&id(B2)).is_err());
}

#[tokio::test]
async fn a_block_closed_while_its_session_is_read_is_never_added_back() {
    const B4: &str = "block:019a7d2e-0000-7000-8000-0000000000b4";
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _task = follow(&rex).await;
    // The list is read with B3 in it, and B3 closes before the answer arrives. The fake
    // keeps answering B3's reads, so only the closed rule keeps it out.
    fake.state().hook(
        Method::SessionListBlocks,
        None,
        true,
        Box::new(|_| vec![fake::block_closed(S1, B3)]),
    );
    fake.emit(|s| {
        for b in [B3, B4] {
            s.add(
                S1,
                b,
                fake::process(30, "claude", "2.1.322"),
                vec![fake::record("idle", 30, "2.1.322", T0)],
            );
        }
        vec![fake::session_event(
            S1,
            json!({ "type": "block_created", "payload": { "block_id": B4 } }),
        )]
    });
    eventually("the block created beside it", || rex.get(&id(B4)).is_ok()).await;
    assert!(rex.get(&id(B3)).is_err());
}

#[tokio::test]
async fn a_block_whose_reads_fail_is_skipped_beside_a_good_one() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    {
        let mut s = fake.state();
        s.add(
            S1,
            B2,
            json!({ "foreground": { "pid": 20, "argv0": "claude" } }),
            vec![],
        );
        let undated = json!({ "state": "idle", "owner": { "pid": 30, "name": "2.1.322" } });
        s.add(
            S1,
            B3,
            fake::process(30, "claude", "2.1.322"),
            vec![undated],
        );
    }
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _task = follow(&rex).await;
    let ids: Vec<String> = (rex.listing().unwrap().agents.into_iter())
        .map(|a| a.terminal_id)
        .collect();
    assert_eq!(ids, [id(B1)]);
    assert_eq!(rex.report().refused, None);
}

#[tokio::test]
async fn a_session_renamed_before_it_is_read_shows_its_new_label() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    // The rename's event arrives before the list, with no session to apply to yet.
    fake.state().hook(
        Method::SessionList,
        None,
        true,
        Box::new(|s| {
            s.sessions[0].1 = "renamed".to_owned();
            let renamed =
                json!({ "type": "session_label_changed", "payload": { "label": "renamed" } });
            vec![fake::session_event(S1, renamed)]
        }),
    );
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _task = follow(&rex).await;
    assert_eq!(group(&rex, S1).as_deref(), Some("renamed"));
}

#[derive(Clone, Default)]
pub(super) struct Logs(pub(super) Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        crate::lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn herdr_lost_and_back_is_logged_once_when_the_core_stops_while_herdr_is_down() {
    use crate::runtime::HerdrSeen;
    use crate::runtime::herdr_core::{Core, tests as core};
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let count = |line: &str| {
        String::from_utf8_lossy(&crate::lock(&logs.0))
            .matches(line)
            .count()
    };

    let dir = tempfile::tempdir().unwrap();
    let herdr = dir.path().join("herdr.sock");
    let answer = |method: &str| match method {
        "agent.list" => Some(json!({ "type": "agent_list", "agents": [] })),
        "workspace.list" => Some(json!({ "type": "workspace_list", "workspaces": [] })),
        _ => None,
    };
    core::fake_herdr(&herdr, answer);
    let seen = Arc::new(HerdrSeen::default());
    let followed = Core::new(dir.path().join("core.sock")).with_herdr_seen(seen.clone());
    let followed = core::following(followed, json!([]));
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    let rex = Arc::new(Rex::new(fake.socket.clone()).with_herdr_seen(seen));
    let _task = follow(&rex).await;
    let rt = Runtime::new(herdr.clone())
        .with_core(Some(followed.clone()))
        .with_rex(Some(rex));

    assert!(rt.poll().await.unwrap().herdr);
    std::fs::remove_file(&herdr).unwrap();
    assert!(!rt.poll().await.unwrap().herdr);
    core::unfollow(&followed);
    assert!(!rt.poll().await.unwrap().herdr);
    core::fake_herdr(&herdr, answer);
    assert!(rt.poll().await.unwrap().herdr);
    assert_eq!(count("herdr unavailable"), 1);
    assert_eq!(count("herdr reachable again"), 1);
}
