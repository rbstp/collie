use std::sync::Arc;

use protocol::{
    AgentPromptParams, AgentSendKeysParams, AgentTypeTextParams, ErrorCode, Key, OpId, PromptText,
    SendTextParams, TerminalId, TypedText,
};
use serde_json::{Value, json};

use super::{B1, B2, B3, S1, T0, claude, follow, id};
use crate::drive::{Authorized, Driver};
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

/// Copilot CLI reports `idle` at its folder trust prompt and with a picker open (#741).
#[tokio::test]
async fn an_agent_with_no_status_or_copilot_takes_no_prompt_answer_or_key() {
    let fake = Fake::start().await;
    let copilot = |state: &str, pid| {
        json!({
            "state": state, "app": "copilot", "updated_at": T0,
            "owner": { "pid": pid, "name": "copilot" },
        })
    };
    let process = |pid| fake::process(pid, "/opt/homebrew/bin/copilot", "copilot");
    {
        let mut s = fake.state();
        s.sessions.push((S1.to_owned(), "work".to_owned()));
        s.add(S1, B1, process(10), vec![copilot("idle", 10)]);
        s.add(S1, B2, process(20), vec![copilot("blocked", 20)]);
        s.add(S1, B3, fake::process(30, "claude", "2.1.322"), vec![]);
    }
    let (rex, _rt, dir) = followed(&fake).await;
    let shown = |b: &str| {
        let a = rex.get(&id(b)).unwrap();
        (a.agent, a.agent_status, a.read_only)
    };
    assert_eq!(shown(B1), (Some("copilot".into()), "idle".into(), true));
    assert_eq!(shown(B2), (Some("copilot".into()), "blocked".into(), true));
    assert_eq!(shown(B3), (Some("claude".into()), "unknown".into(), true));
    let drive = Driver::new(dir.path().join("no-herdr.sock"), vec![], &[])
        .unwrap()
        .with_rex(Some(rex));
    let yes: Authorized = Arc::new(|| true);
    let op = || OpId::new("P".repeat(22)).unwrap();
    let refused = |r: Result<protocol::Response, (ErrorCode, String)>| r.unwrap_err().0;

    for (block, prompt_error) in [
        (B1, ErrorCode::AgentNotReady),
        (B2, ErrorCode::AgentBlocked),
        (B3, ErrorCode::AgentNotReady),
    ] {
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
            refused(drive.send_text(text, &yes).await),
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
