#[allow(dead_code)]
mod common;

use std::sync::Arc;
use std::time::Duration;

use collied::runtime::rex::{GROUP_CLOSE, Rex};
use collied::runtime::{AgentInfo, Error, Runtime};
use common::*;
use tokio::sync::watch;

/// A stand-in agent: it reports OSC 7501 as Claude Code does, from the process that leads the
/// terminal, and moves on once the test creates `go`, then `block`, in its folder. One
/// process with no child, so the reporter is always the foreground process.
const AGENT: &str = r#"$| = 1;
sub st { print "\e]7501;$_[0]\e\\" }
sub wait_for { select(undef, undef, undef, 0.05) until -e $_[0] }
print "collie screen probe\r\n";
st("state=idle:app=claude-code");
wait_for("go");
st("state=working:app=claude-code");
wait_for("block");
st("state=blocked:app=claude-code:kind=permission:msg=approve Bash");
select(undef, undef, undef, 600);
"#;

/// An agent below 100,000 lines of scrollback.
const LONG: &str = r#"$| = 1;
print "line $_\r\n" for 1 .. 100000;
print "\e]7501;state=idle:app=claude-code\e\\";
select(undef, undef, undef, 600);
"#;

/// A shell that reported a status and leads its terminal.
const SHELL: &str = r#"printf '\033]7501;state=idle:app=claude-code\033\\'; read -r -t 600 _"#;

#[test]
fn live_rex() {
    let Some(cli) = rex_cli() else {
        return;
    };
    let mut server = RexServer::start(&cli, "x");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(scenario(&server));
    drop(rt);
    assert!(server.stop(), "dedicated Rex server did not stop");
}

fn new_session(server: &RexServer, label: &str, command: &[&str]) -> (String, String) {
    let work = server.work.to_str().unwrap();
    let mut args = vec!["new", label, "--shell=none", "-c", work, "--json", "--"];
    args.extend(command);
    let created = server.rex(&args);
    let session = created["session_id"].as_str();
    let block = created["initial_windows"][0]["block_ids"][0].as_str();
    match (session, block) {
        (Some(s), Some(b)) => (s.to_owned(), b.to_owned()),
        _ => panic!("rex new: {created}"),
    }
}

async fn until<T>(what: &str, mut ready: impl AsyncFnMut() -> Option<T>) -> T {
    for _ in 0..100 {
        if let Some(v) = ready().await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn scenario(server: &RexServer) {
    let (session, block) = new_session(server, "collie-live", &["/usr/bin/perl", "-e", AGENT]);
    let (_, shell) = new_session(server, "collie-shell", &["/bin/bash", "-c", SHELL]);
    let agent = format!("rex:{block}");

    let rex = Arc::new(Rex::new(server.socket.clone()));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(rex.clone().run(stopped));
    tokio::time::timeout(Duration::from_secs(5), rex.settled())
        .await
        .unwrap();
    let report = rex.report();
    assert!(
        report.version.is_some() && report.refused.is_none(),
        "{report:?}"
    );
    assert!(
        report.api_revision >= Some(13) && report.remote.is_empty(),
        "{report:?}"
    );

    let rt = Runtime::new(server.root.join("no-herdr.sock")).with_rex(Some(rex.clone()));
    let find = async |status: &str| -> Option<AgentInfo> {
        (rt.agent_list().await.ok()?.into_iter())
            .find(|a| a.terminal_id == agent && a.agent_status == status)
    };
    let idle = until("the agent's idle report", async || find("idle").await).await;
    assert_eq!(idle.agent.as_deref(), Some("claude"));
    assert_eq!(idle.workspace_id, format!("rex:{session}"));
    let poll = rt.poll().await.unwrap();
    let ids: Vec<&str> = poll.agents.iter().map(|a| a.terminal_id.as_str()).collect();
    assert_eq!(ids, [agent.as_str()], "a shell in front is never an agent");
    assert!(!ids.contains(&format!("rex:{shell}").as_str()));
    let groups: Vec<(&str, &str)> = (poll.workspaces.iter())
        .map(|w| (w.workspace_id.as_str(), w.label.as_str()))
        .collect();
    assert_eq!(groups, [(idle.workspace_id.as_str(), "collie-live")]);

    let size = || server.rex(&["block", "call", "-b", &block, "size"]);
    let before = size();
    assert!(before["columns"].as_u64().is_some(), "{before}");
    let visible = rt.agent_read(&agent, "visible", None).await.unwrap().text;
    assert!(visible.contains("collie screen probe"), "{visible:?}");
    let detected = rt.detection_text(&agent).await.unwrap();
    assert!(detected.starts_with("collie screen probe"), "{detected:?}");
    let columns = rt.pane_columns(&agent).await.unwrap();
    assert_eq!(columns.map(u64::from), before["columns"].as_u64());
    assert_eq!(
        size(),
        before,
        "a read changed the terminal's size or owner"
    );

    let (_, long) = new_session(server, "collie-long", &["/usr/bin/perl", "-e", LONG]);
    let long = format!("rex:{long}");
    let read = until("the long terminal's agent", async || {
        rt.agent_get(&long).await.ok()?;
        let read = rt
            .agent_read(&long, "recent_unwrapped", Some(50))
            .await
            .ok()?;
        read.text.contains("line 100000").then_some(read.text)
    })
    .await;
    let rows: Vec<&str> = read.split("\r\n").collect();
    assert_eq!(rows.len(), 50, "{read:?}");
    assert_eq!(rows[0], "line 99951");
    assert_eq!(rows[49], "line 100000");

    std::fs::write(server.work.join("go"), "").unwrap();
    let working = until("the working report", async || find("working").await).await;
    std::fs::write(server.work.join("block"), "").unwrap();
    let blocked = until("the blocked report", async || find("blocked").await).await;
    let seqs = [
        idle.state_change_seq,
        working.state_change_seq,
        blocked.state_change_seq,
    ];
    assert!(seqs[0] != seqs[1] && seqs[1] != seqs[2], "{seqs:?}");
    let explained = rt.agent_explain(&agent).await.unwrap();
    assert_eq!(
        explained.matched_rule.map(|r| r.id).as_deref(),
        Some("permission")
    );

    assert!(matches!(
        rt.workspace_close(&idle.workspace_id).await,
        Err(Error::Unsupported(why)) if why == GROUP_CLOSE
    ));
    rt.pane_close(&agent).await.unwrap();
    until("the closed agent to leave", async || {
        rt.agent_list()
            .await
            .ok()
            .filter(|l| l.iter().all(|a| a.terminal_id != agent))
    })
    .await;
    let sessions = server.rex(&["ls", "--json"]);
    let listed = sessions.to_string();
    assert!(
        listed.contains(&session) && listed.contains("collie-shell"),
        "{listed}"
    );

    stop.send_replace(true);
    task.await.unwrap();
}
