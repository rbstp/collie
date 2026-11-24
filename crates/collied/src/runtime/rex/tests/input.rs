use std::sync::Arc;

use protocol::{
    AgentPromptParams, AgentSendKeysParams, AgentTypeTextParams, ErrorCode, Key, KeyCombo, OpId,
    PromptText, SendTextParams, TerminalId, TerminalRunParams, TypedText,
};
use serde_json::{Value, json};

use super::{B1, B2, B3, S1, T0, claude, follow, id};
use crate::drive::{Authorized, Driver, HOSTS_AGENT};
use crate::runtime::rex::fake::{self, Fake};
use crate::runtime::rex::{Rex, input};
use crate::runtime::{Error, Owner, Runtime};

fn events(fake: &Fake, block: &str) -> Vec<Value> {
    std::mem::take(&mut fake.state().block(block).events)
}

fn paste(text: &str) -> Value {
    json!({ "type": "paste", "payload": { "text": text } })
}

fn key(code: &str, mods: &str, text: &str, codepoint: u32) -> Value {
    json!({ "type": "key", "payload": {
        "key": code, "mods": mods, "consumed_mods": "", "text": text,
        "unshifted_codepoint": codepoint, "composing": false, "action": "press",
    }})
}

async fn followed(fake: &Fake) -> (Arc<Rex>, Runtime, tempfile::TempDir) {
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let _following = follow(&rex).await;
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::new(dir.path().join("no-herdr.sock")).with_rex(Some(rex.clone()));
    (rex, rt, dir)
}

#[tokio::test]
async fn prompts_and_answers_are_pastes_then_enter_and_keys_are_key_events_on_one_connection() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    let (_rex, rt, _dir) = followed(&fake).await;
    let agent = id(B1);
    assert_eq!(Owner::of(&agent), Owner::Rex);
    let enter = key("enter", "", "\r", 13);

    rt.agent_prompt(&agent, "fix the build\nthen test", "idle")
        .await
        .unwrap();
    assert_eq!(
        events(&fake, B1),
        [paste("fix the build\nthen test"), enter.clone()]
    );
    rt.type_text(&agent, "yes", "idle", 0).await.unwrap();
    assert_eq!(events(&fake, B1), [paste("yes"), enter]);

    // Rex brackets a paste itself; text is pasted as it is.
    rt.pane_paste(&agent, "/model").await.unwrap();
    rt.pane_send_text(&agent, "a note").await.unwrap();
    assert_eq!(events(&fake, B1), [paste("/model"), paste("a note")]);

    rt.agent_send_keys(&agent, &["down", "ctrl+c", "shift+tab", "ctrl+alt+x"])
        .await
        .unwrap();
    rt.scroll_bottom(&agent).await.unwrap();
    rt.notice_digit(&agent, "2").await.unwrap();
    assert_eq!(
        events(&fake, B1),
        [
            key("arrow_down", "", "", 0),
            key("key_c", "ctrl", "", 99),
            key("tab", "shift", "\t", 9),
            key("key_x", "ctrl+alt", "", 120),
            key("end", "ctrl", "", 0),
            key("digit_2", "", "2", 50),
        ]
    );
    let calls = fake.calls();
    assert_eq!(calls.iter().filter(|c| *c == "hello").count(), 1);
    assert_eq!(calls.iter().filter(|c| *c == "block.event").count(), 12);
}

#[tokio::test]
async fn keys_rex_cannot_send_as_herdr_does_send_nothing_at_all() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "blocked", T0);
    let (_rex, rt, _dir) = followed(&fake).await;
    for name in [
        "cmd+k",
        "ctrl+[",
        "ctrl+alt+[",
        "ctrl+shift+a",
        "ctrl+alt+shift+cmd+backspace",
    ] {
        assert!(protocol::KeyCombo::new(name).is_ok(), "{name}");
        let sent = rt.agent_send_keys(&id(B1), &["down", name]).await;
        assert!(
            matches!(&sent, Err(Error::Unsupported(w)) if w == input::REFUSED),
            "{name}: {sent:?}"
        );
    }
    assert!(events(&fake, B1).is_empty());
    assert!(!fake.calls().iter().any(|c| c == "block.event"));
}

#[tokio::test]
async fn an_agent_with_no_status_takes_no_prompt_answer_or_key() {
    let fake = Fake::start().await;
    {
        let mut s = fake.state();
        s.sessions.push((S1.to_owned(), "work".to_owned()));
        s.add(S1, B3, fake::process(30, "claude", "2.1.322"), vec![]);
    }
    let (rex, _rt, dir) = followed(&fake).await;
    let a = rex.get(&id(B3)).unwrap();
    assert_eq!(
        (a.agent, a.agent_status, a.read_only),
        (Some("claude".into()), "unknown".into(), true)
    );
    let drive = Driver::new(dir.path().join("no-herdr.sock"), vec![], &[])
        .unwrap()
        .with_rex(Some(rex));
    let yes: Authorized = Arc::new(|| true);
    let op = || OpId::new("P".repeat(22)).unwrap();
    let refused = |r: Result<protocol::Response, (ErrorCode, String)>| r.unwrap_err().0;

    for (block, prompt_error) in [(B3, ErrorCode::AgentNotReady)] {
        let terminal = TerminalId::new(id(block)).unwrap();
        let prompt = AgentPromptParams {
            op_id: op(),
            terminal_id: terminal.clone(),
            text: PromptText::new("hi".to_owned()).unwrap(),
            expected_draft: None,
        };
        assert_eq!(refused(drive.prompt(prompt, &yes).await), prompt_error);
        let typed = AgentTypeTextParams {
            op_id: op(),
            terminal_id: terminal.clone(),
            text: PromptText::new("yes".to_owned()).unwrap(),
        };
        assert_eq!(
            refused(drive.type_text(typed, &yes).await),
            ErrorCode::AgentNotReady
        );
        let keys = AgentSendKeysParams {
            op_id: op(),
            terminal_id: terminal.clone(),
            keys: vec![Key::Enter],
        };
        assert_eq!(
            refused(drive.send_keys(keys, &yes).await.0),
            ErrorCode::AgentNotReady
        );
        let text = SendTextParams {
            op_id: op(),
            terminal_id: terminal.clone(),
            text: TypedText::new("/x".to_owned()).unwrap(),
        };
        assert_eq!(
            refused(drive.send_text(text, &yes).await.0),
            ErrorCode::AgentNotReady
        );
        assert_eq!(
            refused(drive.scroll_bottom(&terminal, &yes).await),
            ErrorCode::AgentNotReady
        );
    }
    assert!(!fake.calls().iter().any(|c| c == "block.event"));
}

#[tokio::test]
async fn input_reaches_only_a_listed_agent_and_fails_at_once_without_rex() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    fake.state()
        .add(S1, B2, fake::process(20, "-zsh", "zsh"), vec![]);
    let rex = Arc::new(Rex::new(fake.socket.clone()));
    let task = follow(&rex).await;
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::new(dir.path().join("no-herdr.sock")).with_rex(Some(rex.clone()));
    let not_found =
        |r: Result<(), Error>| matches!(r, Err(Error::Rex { code, .. }) if code == "not_found");

    assert!(not_found(rt.agent_send_keys(&id(B2), &["enter"]).await));
    assert!(not_found(rt.agent_prompt(&id(B2), "ls", "idle").await));
    assert!(!fake.calls().iter().any(|c| c == "block.event"));

    fake.drop_connection();
    let (_, followed) = task.await.unwrap();
    assert!(followed);
    let started = tokio::time::Instant::now();
    assert!(not_found(rt.agent_prompt(&id(B1), "hi", "idle").await));
    assert!(not_found(rt.agent_send_keys(&id(B1), &["enter"]).await));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

fn blocked_at(at: &str) -> Value {
    let mut r = fake::record("blocked", 10, "2.1.322", at);
    r["kind"] = json!("permission");
    r
}

/// The program reports `record` as Rex takes the next `block.event`, before answering it.
fn reports_on_next_event(fake: &Fake, record: Value) {
    fake.state().hook(
        crate::runtime::rex::client::Method::BlockEvent,
        Some(B1),
        false,
        Box::new(move |s| {
            s.block(B1).records = vec![record.clone()];
            vec![fake::status_changed(S1, B1, record)]
        }),
    );
}

fn status_changed(r: Result<(), Error>) -> bool {
    matches!(r, Err(Error::Rex { code, .. }) if code == input::STATUS_CHANGED)
}

#[tokio::test]
async fn a_dialog_that_opens_during_a_prompt_never_gets_its_enter() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    let (rex, rt, _dir) = followed(&fake).await;
    reports_on_next_event(&fake, blocked_at("2026-11-12T16:47:05.000001Z"));
    assert!(status_changed(rt.agent_prompt(&id(B1), "hi", "idle").await));
    assert_eq!(events(&fake, B1), [paste("hi")]);
    assert_eq!(rex.get(&id(B1)).unwrap().agent_status, "blocked");

    // Nor a prompt whose agent changed before its first event.
    assert!(status_changed(rt.agent_prompt(&id(B1), "hi", "idle").await));
    assert!(events(&fake, B1).is_empty());
}

#[tokio::test]
async fn a_typed_answer_never_lands_in_the_next_dialog() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "blocked", T0);
    fake.state().block(B1).records = vec![blocked_at(T0)];
    let (rex, rt, _dir) = followed(&fake).await;
    let seq = rex.get(&id(B1)).unwrap().state_change_seq;

    // The same dialog: the answer and its Enter.
    rt.type_text(&id(B1), "yes", "blocked", seq).await.unwrap();
    assert_eq!(
        events(&fake, B1),
        [paste("yes"), key("enter", "", "\r", 13)]
    );

    // Another dialog, still `blocked`, reported while the answer was pasted.
    reports_on_next_event(&fake, blocked_at("2026-11-12T16:47:05.000001Z"));
    assert!(status_changed(
        rt.type_text(&id(B1), "yes", "blocked", seq).await
    ));
    assert_eq!(events(&fake, B1), [paste("yes")]);
}

#[tokio::test]
async fn one_calls_keys_reach_the_block_together() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "blocked", T0);
    let (_rex, rt, _dir) = followed(&fake).await;
    let rt = Arc::new(rt);
    fake.state().stall = true;
    let deny = tokio::spawn({
        let rt = rt.clone();
        async move {
            rt.agent_send_keys(&id(B1), &["down", "down", "enter"])
                .await
        }
    });
    super::eventually("the first arrow", || {
        fake.state().block(B1).events.len() == 1
    })
    .await;
    let other = tokio::spawn({
        let rt = rt.clone();
        async move { rt.agent_send_keys(&id(B1), &["enter"]).await }
    });
    // Without one call at a time, the other Enter reaches Rex here, between the arrows.
    let between = tokio::time::timeout(std::time::Duration::from_millis(200), async {
        while fake.state().block(B1).events.len() < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await;
    fake.emit(|s| {
        s.stall = false;
        Vec::new()
    });
    deny.await.unwrap().unwrap();
    other.await.unwrap().unwrap();
    assert!(
        between.is_err(),
        "another call's key was sent between the arrows"
    );
    let codes: Vec<String> = events(&fake, B1)
        .iter()
        .map(|e| e["payload"]["key"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(codes, ["arrow_down", "arrow_down", "enter", "enter"]);
}

fn not_found(r: Result<(), Error>) -> bool {
    matches!(r, Err(Error::Rex { code, .. }) if code == "not_found")
}

#[tokio::test]
async fn terminal_input_reaches_only_a_listed_terminal_of_this_host() {
    let fake = Fake::start().await;
    claude(&fake, S1, B1, 10, "idle", T0);
    {
        let mut s = fake.state();
        s.add(S1, B2, fake::process(20, "-zsh", "zsh"), vec![]);
        s.add(S1, B3, fake::process(30, "-zsh", "zsh"), vec![]);
        s.block(B3).host = "0e9d6c1f-4a3b-4a52-9c85-2f6d3f6f2f10".to_owned();
    }
    let (rex, rt, dir) = followed(&fake).await;
    let (agent, shell, away) = (id(B1), id(B2), id(B3));
    let enter = key("enter", "", "\r", 13);

    rt.pane_send_input(&shell, "ls -la", &["enter"])
        .await
        .unwrap();
    rt.pane_send_keys(&shell, &["ctrl+c", "up"]).await.unwrap();
    rt.terminal_send_text(&shell, "echo").await.unwrap();
    assert_eq!(
        events(&fake, B2),
        [
            paste("ls -la"),
            enter.clone(),
            key("key_c", "ctrl", "", 99),
            key("arrow_up", "", "", 0),
            paste("echo"),
        ]
    );

    // An agent takes no terminal input, a terminal no agent input, and a block of another
    // host neither.
    assert!(not_found(
        rt.pane_send_input(&agent, "ls", &["enter"]).await
    ));
    assert!(not_found(rt.pane_send_keys(&agent, &["enter"]).await));
    assert!(not_found(rt.terminal_send_text(&agent, "ls").await));
    assert!(not_found(rt.agent_prompt(&shell, "ls", "idle").await));
    assert!(not_found(rt.agent_send_keys(&shell, &["enter"]).await));
    assert!(not_found(rt.pane_send_text(&shell, "ls").await));
    assert!(not_found(rt.pane_send_input(&away, "ls", &["enter"]).await));
    assert!(not_found(rt.pane_send_keys(&away, &["enter"]).await));
    assert!(not_found(rt.terminal_send_text(&away, "ls").await));
    let refused = rt.pane_send_keys(&shell, &["up", "cmd+k"]).await;
    assert!(matches!(&refused, Err(Error::Unsupported(w)) if w == input::REFUSED));
    assert!(events(&fake, B1).is_empty() && events(&fake, B2).is_empty());
    assert!(events(&fake, B3).is_empty());

    // The phone's calls: a grant, and a terminal of this host with no agent in it.
    let drive = Driver::new(dir.path().join("no-herdr.sock"), vec![], &[])
        .unwrap()
        .with_rex(Some(rex));
    let yes: Authorized = Arc::new(|| true);
    let no: Authorized = Arc::new(|| false);
    let op = || OpId::new("T".repeat(22)).unwrap();
    let run = |t: &str| TerminalRunParams {
        op_id: op(),
        terminal_id: TerminalId::new(t).unwrap(),
        text: PromptText::new("make".to_owned()).unwrap(),
    };
    drive.terminal_run(run(&shell), &yes).await.unwrap();
    let keys = AgentSendKeysParams {
        op_id: op(),
        terminal_id: TerminalId::new(&shell).unwrap(),
        keys: vec![Key::Combo(KeyCombo::new("ctrl+c").unwrap())],
    };
    drive.terminal_send_keys(keys, &yes).await.unwrap();
    let text = SendTextParams {
        op_id: op(),
        terminal_id: TerminalId::new(&shell).unwrap(),
        text: TypedText::new("q".to_owned()).unwrap(),
    };
    drive.terminal_send_text(text, &yes).await.unwrap();
    assert_eq!(
        events(&fake, B2),
        [
            paste("make"),
            enter,
            key("key_c", "ctrl", "", 99),
            paste("q")
        ]
    );
    let code = |r: Result<protocol::Response, (ErrorCode, String)>| r.unwrap_err();
    assert_eq!(
        code(drive.terminal_run(run(&shell), &no).await).0,
        ErrorCode::TerminalLocked
    );
    assert_eq!(
        code(drive.terminal_run(run(&away), &yes).await).0,
        ErrorCode::NotFound
    );
    assert_eq!(
        code(drive.terminal_run(run(&agent), &yes).await),
        (ErrorCode::NotFound, HOSTS_AGENT.to_owned())
    );
    assert!(events(&fake, B1).is_empty() && events(&fake, B2).is_empty());
    assert!(events(&fake, B3).is_empty());
}

#[tokio::test]
async fn an_agent_started_in_a_terminal_during_a_run_never_gets_its_enter() {
    let fake = Fake::start().await;
    fake.state()
        .sessions
        .push((S1.to_owned(), "work".to_owned()));
    fake.state()
        .add(S1, B2, fake::process(20, "-zsh", "zsh"), vec![]);
    let (_rex, rt, _dir) = followed(&fake).await;
    fake.state().hook(
        crate::runtime::rex::client::Method::BlockEvent,
        Some(B2),
        false,
        Box::new(|s| {
            let claude = fake::under_shell(21, "claude", "2.1.322");
            s.block(B2).process = claude.clone();
            vec![fake::block_event(S1, B2, "process_changed", claude)]
        }),
    );
    assert!(not_found(
        rt.pane_send_input(&id(B2), "ls", &["enter"]).await
    ));
    assert_eq!(events(&fake, B2), [paste("ls")]);
}

#[tokio::test]
async fn a_shell_keeping_an_agent_under_it_takes_no_terminal_input_and_drops_its_grant() {
    let fake = Fake::start().await;
    let me = std::process::id();
    {
        let mut s = fake.state();
        s.sessions.push((S1.to_owned(), "work".to_owned()));
        let sh = fake::under_shell(me, "sh", "bash");
        s.add(S1, B2, sh, vec![fake::record("idle", me, "bash", T0)]);
    }
    let (rex, rt, dir) = followed(&fake).await;
    let shell = id(B2);
    assert!(not_found(
        rt.pane_send_input(&shell, "yes", &["enter"]).await
    ));
    assert!(not_found(rt.pane_send_keys(&shell, &["enter"]).await));
    let drive = Driver::new(dir.path().join("no-herdr.sock"), vec![], &[])
        .unwrap()
        .with_rex(Some(rex.clone()));
    let yes: Authorized = Arc::new(|| true);
    let run = TerminalRunParams {
        op_id: OpId::new("W".repeat(22)).unwrap(),
        terminal_id: TerminalId::new(&shell).unwrap(),
        text: PromptText::new("yes".to_owned()).unwrap(),
    };
    // The answer that makes the server drop the terminal's grants, as for a herdr pane.
    assert_eq!(
        drive.terminal_run(run.clone(), &yes).await.unwrap_err(),
        (ErrorCode::NotFound, HOSTS_AGENT.to_owned())
    );
    assert!(events(&fake, B2).is_empty());

    // The agent and its `sh` exit: the user's shell leads again.
    fake.emit(|s| {
        let zsh = fake::process(30, "-zsh", "zsh");
        s.block(B2).process = zsh.clone();
        vec![fake::block_event(S1, B2, "process_changed", zsh)]
    });
    super::eventually("the terminal listed", || {
        rex.listing()
            .is_some_and(|l| l.terminals.iter().any(|p| p.agent.is_none()))
    })
    .await;
    drive.terminal_run(run, &yes).await.unwrap();
    assert_eq!(
        events(&fake, B2),
        [paste("yes"), key("enter", "", "\r", 13)]
    );
}

#[tokio::test]
async fn a_silent_codex_takes_only_its_startup_dialogs_own_keys_up_to_the_answer() {
    let trust = include_str!("../../../../tests/fixtures/codex-0.171.0/trust.detection.txt");
    let narrow =
        include_str!("../../../../tests/fixtures/codex-0.171.0/trust-narrow.detection.txt");
    let composer = include_str!("../../../../tests/fixtures/codex-0.171.0/idle.detection.txt");
    let fake = Fake::start().await;
    {
        let mut s = fake.state();
        s.sessions.push((S1.to_owned(), "work".to_owned()));
        let codex = fake::process(10, "/opt/homebrew/bin/codex", "codex");
        s.add(S1, B1, codex, vec![]);
    }
    let show = |screen: &str| fake.state().block(B1).screen = screen.replace('\n', "\r\n");
    show(trust);
    let (rex, _rt, dir) = followed(&fake).await;
    let drive = Driver::new(dir.path().join("no-herdr.sock"), vec![], &[])
        .unwrap()
        .with_rex(Some(rex));
    let yes: Authorized = Arc::new(|| true);
    let op = || OpId::new("K".repeat(22)).unwrap();
    let terminal = TerminalId::new(id(B1)).unwrap();
    let combo = |k: &str| Key::Combo(KeyCombo::new(k).unwrap());
    let keys = |keys: Vec<Key>| AgentSendKeysParams {
        op_id: op(),
        terminal_id: terminal.clone(),
        keys,
    };
    let text = |t: &str| SendTextParams {
        op_id: op(),
        terminal_id: terminal.clone(),
        text: TypedText::new(t.to_owned()).unwrap(),
    };
    let refused = |r: (
        Result<protocol::Response, (ErrorCode, String)>,
        Option<String>,
    )| {
        assert_eq!(r.1, None);
        r.0.unwrap_err().0
    };
    for batch in [
        vec![combo("1"), combo("h"), combo("i"), Key::Enter],
        vec![Key::Enter, Key::Down],
        vec![combo("1"), combo("2")],
        vec![Key::Tab],
        vec![combo("h")],
        vec![combo("shift+1")],
    ] {
        assert_eq!(
            refused(drive.send_keys(keys(batch), &yes).await),
            ErrorCode::AgentNotReady
        );
    }
    for t in ["hi", "12", "1h", "x"] {
        assert_eq!(
            refused(drive.send_text(text(t), &yes).await),
            ErrorCode::AgentNotReady
        );
    }
    assert_eq!(events(&fake, B1), Vec::<Value>::new());

    let target = |what: &str| Some(format!("{} blocked {what}", id(B1)));
    let (sent, audited) = drive
        .send_keys(keys(vec![Key::Down, Key::Up, Key::Enter]), &yes)
        .await;
    assert_eq!(
        (sent, audited),
        (Ok(protocol::Response::Ok), target("keys=down up enter"))
    );
    let enter = key("enter", "", "\r", 13);
    assert_eq!(
        events(&fake, B1),
        [
            key("arrow_down", "", "", 0),
            key("arrow_up", "", "", 0),
            enter
        ]
    );
    assert_eq!(
        refused(drive.send_keys(keys(vec![Key::Down]), &yes).await),
        ErrorCode::AgentNotReady
    );
    assert_eq!(
        refused(drive.send_text(text("1"), &yes).await),
        ErrorCode::AgentNotReady
    );
    show(composer);
    assert_eq!(
        refused(drive.send_keys(keys(vec![Key::Down]), &yes).await),
        ErrorCode::AgentNotReady
    );
    show("Update available\n1. Update now\n› Ask Codex to do anything\n");
    assert_eq!(
        refused(drive.send_text(text("1"), &yes).await),
        ErrorCode::AgentNotReady
    );
    assert_eq!(events(&fake, B1), Vec::<Value>::new());

    show(narrow);
    let (sent, audited) = drive.send_text(text("2"), &yes).await;
    assert_eq!(
        (sent, audited),
        (Ok(protocol::Response::Ok), target("text"))
    );
    assert_eq!(events(&fake, B1), [key("digit_2", "", "2", 50)]);
}

#[tokio::test]
async fn a_copilot_trust_box_takes_its_keys_and_a_blocked_copilot_takes_any() {
    let wide =
        include_str!("../../../../tests/fixtures/copilot-1.0.112/trust-wide-char.detection.txt");
    let fake = Fake::start().await;
    let record = |state: &str| {
        json!({
            "state": state, "app": "copilot", "updated_at": T0,
            "owner": { "pid": 10, "name": "copilot" },
        })
    };
    {
        let mut s = fake.state();
        s.sessions.push((S1.to_owned(), "work".to_owned()));
        let copilot = fake::process(10, "/opt/homebrew/bin/copilot", "copilot");
        s.add(S1, B1, copilot, vec![record("idle")]);
        s.block(B1).screen = wide.trim_end().replace('\n', "\r\n");
    }
    let (rex, _rt, dir) = followed(&fake).await;
    let drive = Driver::new(dir.path().join("no-herdr.sock"), vec![], &[])
        .unwrap()
        .with_rex(Some(rex.clone()));
    let yes: Authorized = Arc::new(|| true);
    let keys = |keys: Vec<Key>| AgentSendKeysParams {
        op_id: OpId::new("K".repeat(22)).unwrap(),
        terminal_id: TerminalId::new(id(B1)).unwrap(),
        keys,
    };
    let (sent, audited) = drive.send_keys(keys(vec![Key::Down]), &yes).await;
    assert_eq!(sent, Ok(protocol::Response::Ok));
    assert_eq!(audited, Some(format!("{} blocked keys=down", id(B1))));
    assert_eq!(events(&fake, B1), [key("arrow_down", "", "", 0)]);
    fake.emit(move |_| vec![fake::status_changed(S1, B1, record("blocked"))]);
    super::eventually("the blocked report", || {
        rex.get(&id(B1)).is_ok_and(|a| a.agent_status == "blocked")
    })
    .await;
    let (sent, audited) = drive.send_keys(keys(vec![Key::Tab]), &yes).await;
    assert_eq!(sent, Ok(protocol::Response::Ok));
    assert_eq!(audited, Some(format!("{} blocked keys=tab", id(B1))));
    assert_eq!(events(&fake, B1), [key("tab", "", "\t", 9)]);
}

#[tokio::test]
async fn a_copilot_takes_input_only_while_its_input_box_shows() {
    let fixture = |name: &str| {
        let dir = env!("CARGO_MANIFEST_DIR");
        let path = format!("{dir}/tests/fixtures/copilot-1.0.115/{name}.detection.txt");
        std::fs::read_to_string(path).unwrap().replace('\n', "\r\n")
    };
    let fake = Fake::start().await;
    {
        let mut s = fake.state();
        s.sessions.push((S1.to_owned(), "work".to_owned()));
        let copilot = fake::process(10, "/opt/homebrew/bin/copilot", "copilot");
        let record = json!({
            "state": "idle", "app": "copilot", "updated_at": T0,
            "owner": { "pid": 10, "name": "copilot" },
        });
        s.add(S1, B1, copilot, vec![record]);
    }
    let show = |name: &str| fake.state().block(B1).screen = fixture(name);
    show("model");
    let (rex, _rt, dir) = followed(&fake).await;
    assert!(!rex.get(&id(B1)).unwrap().read_only);
    let drive = Driver::new(dir.path().join("no-herdr.sock"), vec![], &[])
        .unwrap()
        .with_rex(Some(rex));
    let yes: Authorized = Arc::new(|| true);
    let op = || OpId::new("C".repeat(22)).unwrap();
    let terminal = TerminalId::new(id(B1)).unwrap();
    let prompt = || AgentPromptParams {
        op_id: op(),
        terminal_id: terminal.clone(),
        text: PromptText::new("hi".to_owned()).unwrap(),
        expected_draft: None,
    };
    let typed = || AgentTypeTextParams {
        op_id: op(),
        terminal_id: terminal.clone(),
        text: PromptText::new("yes".to_owned()).unwrap(),
    };
    let keys = || AgentSendKeysParams {
        op_id: op(),
        terminal_id: terminal.clone(),
        keys: vec![Key::Down],
    };
    let text = || SendTextParams {
        op_id: op(),
        terminal_id: terminal.clone(),
        text: TypedText::new("x".to_owned()).unwrap(),
    };
    let dialog = (
        ErrorCode::AgentNotReady,
        "Copilot CLI does not show its input box; answer its dialog in the terminal".to_owned(),
    );
    for name in ["model", "theme", "permissions", "resume"] {
        show(name);
        assert_eq!(
            drive.prompt(prompt(), &yes).await,
            Err(dialog.clone()),
            "{name}"
        );
        assert_eq!(
            drive.type_text(typed(), &yes).await,
            Err(dialog.clone()),
            "{name}"
        );
        assert_eq!(
            drive.send_keys(keys(), &yes).await.0,
            Err(dialog.clone()),
            "{name}"
        );
        assert_eq!(
            drive.send_text(text(), &yes).await.0,
            Err(dialog.clone()),
            "{name}"
        );
    }
    assert_eq!(events(&fake, B1), Vec::<Value>::new());

    show("trust");
    assert_eq!(drive.prompt(prompt(), &yes).await, Err(dialog.clone()));
    let (sent, audited) = drive.send_keys(keys(), &yes).await;
    assert_eq!(sent, Ok(protocol::Response::Ok));
    assert_eq!(audited, Some(format!("{} blocked keys=down", id(B1))));
    assert_eq!(events(&fake, B1), [key("arrow_down", "", "", 0)]);

    show("draft");
    let busy = (
        ErrorCode::AgentNotReady,
        "the input box is not empty".to_owned(),
    );
    assert_eq!(drive.prompt(prompt(), &yes).await, Err(busy.clone()));
    assert_eq!(drive.type_text(typed(), &yes).await, Err(busy));
    assert_eq!(
        drive.send_keys(keys(), &yes).await.0,
        Ok(protocol::Response::Ok)
    );
    assert_eq!(
        drive.send_text(text(), &yes).await.0,
        Ok(protocol::Response::Ok)
    );
    assert_eq!(
        events(&fake, B1),
        [key("arrow_down", "", "", 0), paste("x")]
    );

    show("idle");
    assert_eq!(
        drive.prompt(prompt(), &yes).await,
        Ok(protocol::Response::Ok)
    );
    assert_eq!(events(&fake, B1), [paste("hi"), key("enter", "", "\r", 13)]);
}

#[tokio::test]
async fn agy_prompt_splits_leading_media_paths_into_their_own_paste_before_text_and_enter() {
    let fake = Fake::start().await;
    let agy_record = json!({
        "state": "idle", "app": "antigravity", "updated_at": T0,
        "owner": { "pid": 10, "name": "agy" },
    });
    {
        let mut s = fake.state();
        s.sessions.push((S1.to_owned(), "work".to_owned()));
        s.add(
            S1,
            B1,
            fake::process(10, "/Users/me/.local/bin/agy", "agy"),
            vec![agy_record],
        );
    }
    let (_rex, rt, dir) = followed(&fake).await;
    let shot1 = dir.path().join("screen1.png");
    let shot2 = dir.path().join("screen2.jpg");
    std::fs::write(&shot1, b"png").unwrap();
    std::fs::write(&shot2, b"jpg").unwrap();
    let enter = key("enter", "", "\r", 13);

    let combined = format!("{} {} fix the bug", shot1.display(), shot2.display());
    rt.agent_prompt(&id(B1), &combined, "idle").await.unwrap();
    assert_eq!(
        events(&fake, B1),
        [
            paste(&format!("{} {}", shot1.display(), shot2.display())),
            paste("fix the bug"),
            enter.clone(),
        ]
    );

    let only_media = shot1.display().to_string();
    rt.agent_prompt(&id(B1), &only_media, "idle").await.unwrap();
    assert_eq!(events(&fake, B1), [paste(&only_media), enter.clone()]);

    let plain = "just text without attachments";
    rt.agent_prompt(&id(B1), plain, "idle").await.unwrap();
    assert_eq!(events(&fake, B1), [paste(plain), enter]);
}

#[tokio::test]
async fn stale_foreground_refreshes_process_when_status_owner_disagrees() {
    let fake = Fake::start().await;
    let agy_record = |pid: u32, state: &str, at: &str| {
        json!({
            "state": state, "app": "antigravity", "updated_at": at,
            "owner": { "pid": pid, "name": "agy" },
        })
    };
    {
        let mut s = fake.state();
        s.sessions.push((S1.to_owned(), "work".to_owned()));
        s.add(
            S1,
            B1,
            fake::process(10, "/Users/me/.local/bin/agy", "agy"),
            vec![agy_record(10, "idle", T0)],
        );
    }
    let (rex, rt, dir) = followed(&fake).await;
    assert!(!rex.get(&id(B1)).unwrap().read_only);

    let updated_process = fake::process(20, "/Users/me/.local/bin/agy", "agy");
    let updated_record = agy_record(20, "idle", "2026-11-12T16:47:05.000001Z");
    fake.emit(move |s| {
        let block = s.block(B1);
        block.process = updated_process;
        block.records = vec![updated_record.clone()];
        vec![fake::status_changed(S1, B1, updated_record)]
    });
    super::eventually("stale foreground refreshed from Rex", || {
        rex.get(&id(B1))
            .is_ok_and(|a| !a.read_only && a.agent_status == "idle")
    })
    .await;

    let drive = Driver::new(dir.path().join("no-herdr.sock"), vec![], &[])
        .unwrap()
        .with_rex(Some(rex.clone()));
    let yes: Authorized = Arc::new(|| true);
    let send = SendTextParams {
        op_id: OpId::new("T".repeat(22)).unwrap(),
        terminal_id: TerminalId::new(id(B1)).unwrap(),
        text: TypedText::new("hello".to_owned()).unwrap(),
    };
    assert_eq!(
        drive.send_text(send, &yes).await.0,
        Ok(protocol::Response::Ok)
    );
    assert_eq!(events(&fake, B1), [paste("hello")]);
    assert!(!rt.agent_get(&id(B1)).await.unwrap().read_only);
}
