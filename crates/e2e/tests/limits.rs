#[allow(dead_code)]
mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use collie_core::{CollieCore, MachineFlock};
use collied::config::TasksConfig;
use collied::control::{Client, Reply, Request};
use collied::limits::CONTINUE;
use collied::server::{self, ServerConfig};
use common::*;

const RESUMED: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
const PROMPTED: &str = "0a0a0a0a-0000-4000-8000-000000000002";
const STOPPED_TURN: &str = "11111111-1111-4111-8111-111111111111";
const WAKEUP_TURN: &str = "22222222-2222-4222-8222-222222222222";

/// A stand-in Claude Code: it writes its `sessions/<pid>.json`, reports idle with an empty
/// input box as Claude Code does, and prints each prompt it reads.
fn fake_claude(sessions: &Path, session: &str) -> String {
    let sessions = sessions.display();
    format!(
        r#"mkdir -p '{sessions}'
printf '{{"pid":%d,"sessionId":"{session}","cwd":"%s","startedAt":%s000}}' $$ "$PWD" "$(date +%s)" > '{sessions}'/$$.json
st() {{ printf '\033]7501;state=%s:app=claude-code\033\\' "$1"; }}
cols=$(stty size | cut -d' ' -f2)
rule() {{ i=0; while [ $i -lt $cols ]; do printf '\342\224\200'; i=$((i+1)); done; printf '\n'; }}
box() {{ rule; printf '\342\235\257 \033[2mTry "fix lint errors"\033[0m\n'; rule; }}
st idle; box
while IFS= read -r p; do st working; printf '\033[2J\033[Hgot %s\n' "$p"; st idle; box; done
exec sleep 600"#
    )
}

/// collied stops two Claude Code agents at the plan limit when their PreToolUse hook asks:
/// one resumes on its own once the limit resets; the other's resume is cancelled from the
/// phone, a wakeup is held again, and a phone prompt releases it. Everything runs in a
/// herdr-core of the test's own.
#[test]
fn live_limit_resume() {
    if !hc_installed() {
        return;
    }
    // Named once, in the parent: the child must see the same values to run the test.
    let agents = std::env::var("COLLIE_E2E_AGENTS")
        .unwrap_or_else(|_| format!("/tmp/hcl-{}", std::process::id()));
    let claude = format!("{agents}/claude");
    let codex = format!("{agents}/codex");
    let env = [
        ("COLLIE_E2E_AGENTS", agents.as_str()),
        ("CLAUDE_CONFIG_DIR", claude.as_str()),
        ("CODEX_HOME", codex.as_str()),
    ];
    if !in_child_with("live_limit_resume", &env) {
        let _ = std::fs::remove_dir_all(&agents);
        return;
    }
    let mut hc = CoreSession::start("hcl");
    let root = TempDir::new("e2e-limits");
    let net = Net::start(&root.0);
    let (rt, phone) = phone_up(&root.0, &net);
    rt.block_on(scenario(&root.0, &net, &phone, &hc, Path::new(&claude)));
    drop(phone);
    drop(rt);
    assert!(hc.stop(), "dedicated herdr-core did not stop");
}

fn run(hc: &CoreSession, name: &str, work: &Path, script: &str) -> String {
    let started = hc.hc(&[
        "run",
        "--bg",
        "--json",
        "--name",
        name,
        "--cwd",
        work.to_str().unwrap(),
        "/bin/bash",
        "-c",
        r#"exec -a collie-fake /bin/bash -c "$1""#,
        "fake",
        script,
    ]);
    started["address"].as_str().unwrap().to_owned()
}

async fn ask(control: &Path, session: &str, prompt: &str) -> Option<String> {
    let ask = Request::Limit {
        session_id: session.into(),
        subagent: false,
        prompt_id: Some(prompt.into()),
    };
    match Client::connect(control)
        .await
        .unwrap()
        .call(&ask)
        .await
        .unwrap()
    {
        Reply::Limit { stop } => stop,
        other => panic!("{other:?}"),
    }
}

async fn flock_until(
    phone: &CollieCore,
    m: &str,
    what: &str,
    ok: impl Fn(&MachineFlock) -> bool,
) -> MachineFlock {
    for _ in 0..200 {
        let f = phone.flock(m.into()).await.unwrap();
        if ok(&f) {
            return f;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the phone never saw {what}");
}

async fn shows(hc: &CoreSession, address: &str, text: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "{text} never reached the pane");
        let screen = hc.hc(&["read", "--json", address])["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if screen.contains(text) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn scenario(
    root: &Path,
    net: &Net,
    phone: &Arc<CollieCore>,
    hc: &CoreSession,
    claude: &Path,
) {
    let work = std::fs::canonicalize(&hc.work).unwrap();
    let sessions = claude.join("sessions");
    let resumed = run(
        hc,
        "collie-resumed",
        &work,
        &fake_claude(&sessions, RESUMED),
    );
    let prompted = run(
        hc,
        "collie-prompted",
        &work,
        &fake_claude(&sessions, PROMPTED),
    );
    let data_dir = root.join("collied");
    let handle = server::start_with(
        net.mac.clone(),
        ServerConfig {
            herdr_core: Some(hc.socket.clone()),
            resume_grace: Duration::from_secs(1),
            ..server_config(&data_dir, "e2e")
        },
        root.join("no-herdr.sock"),
        &TasksConfig {
            roots: Some(vec![work.clone()]),
            ..TasksConfig::default()
        },
        None,
    )
    .await
    .unwrap();
    let control = handle.control_path();
    let (machine, _) = pair(&control, phone, LABEL).await;
    let m = machine.id.clone();
    let f = flock_until(phone, &m, "both agents", |f| {
        f.agents
            .iter()
            .filter(|a| a.kind.as_deref() == Some("claude"))
            .count()
            == 2
    })
    .await;
    let id = |address: &str| {
        f.agents
            .iter()
            .find(|a| a.terminal_id.ends_with(&format!(":{address}")))
            .unwrap()
            .terminal_id
            .clone()
    };
    let (resumed_id, prompted_id) = (id(&resumed), id(&prompted));
    phone.set_stop_at_limit(m.clone(), true).await.unwrap();
    flock_until(phone, &m, "the switch on", |f| f.stop_at_limit).await;

    // Each session's status line reports the 5-hour limit at 99%: one resets in seconds.
    let now = collied::now_ms();
    let soon = now / 1000 + 3;
    for (session, resets) in [(RESUMED, soon), (PROMPTED, soon + 3_600)] {
        let status_line = serde_json::json!({
            "session_id": session,
            "rate_limits": {
                "five_hour": {"used_percentage": 99.3, "resets_at": resets},
                "seven_day": {"used_percentage": 40, "resets_at": resets + 86_400},
            },
        });
        collied::usage::record(&data_dir, status_line.to_string().as_bytes(), now).unwrap();
    }

    // The hook's question, until collied lists each session and stops it.
    let deadline = Instant::now() + Duration::from_secs(20);
    for session in [RESUMED, PROMPTED] {
        let reason = loop {
            assert!(Instant::now() < deadline, "{session} was never stopped");
            match ask(&control, session, STOPPED_TURN).await {
                Some(reason) => break reason,
                None => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        };
        assert!(reason.contains("5-hour plan limit"), "{reason}");
    }
    match Client::connect(&control)
        .await
        .unwrap()
        .call(&Request::Watch)
        .await
        .unwrap()
    {
        Reply::Watch { limited, .. } => assert_eq!(limited.len(), 2),
        other => panic!("{other:?}"),
    }

    // Cancel from the phone: still held, and a wakeup is stopped again.
    let armed = |f: &MachineFlock, id: &str| {
        f.agents
            .iter()
            .find(|a| a.terminal_id == id)
            .and_then(|a| a.plan_limit.as_ref())
            .map(|l| l.resume_at_ms.is_some())
    };
    flock_until(phone, &m, "the resume armed", |f| {
        armed(f, &prompted_id) == Some(true)
    })
    .await;
    phone
        .cancel_resume(m.clone(), prompted_id.clone())
        .await
        .unwrap();
    flock_until(phone, &m, "the resume cancelled", |f| {
        armed(f, &prompted_id) == Some(false)
    })
    .await;
    assert!(
        phone
            .cancel_resume(m.clone(), prompted_id.clone())
            .await
            .is_err()
    );
    assert!(
        ask(&control, PROMPTED, WAKEUP_TURN).await.is_some(),
        "a wakeup is held"
    );

    // A phone prompt releases it.
    phone
        .prompt(m.clone(), prompted_id.clone(), CONTINUE.into(), None)
        .await
        .unwrap();
    shows(hc, &prompted, &format!("got {CONTINUE}")).await;
    flock_until(phone, &m, "the prompted agent released", |f| {
        armed(f, &prompted_id).is_none()
    })
    .await;
    assert_eq!(ask(&control, PROMPTED, WAKEUP_TURN).await, None);

    // The other resumes on its own at the reset.
    shows(hc, &resumed, &format!("got {CONTINUE}")).await;
    flock_until(phone, &m, "the resumed agent released", |f| {
        armed(f, &resumed_id).is_none()
    })
    .await;

    let lines = audit_lines(&data_dir.join("audit.log"));
    let limit_lines: Vec<(String, String, String)> = lines
        .iter()
        .filter(|l| {
            l["method"].as_str().is_some_and(|m| {
                m.starts_with("limit.")
                    || m.contains("stop_at_limit")
                    || m.contains("cancel_resume")
            })
        })
        .map(|l| {
            (
                l["method"].as_str().unwrap().to_owned(),
                l["target"].as_str().unwrap_or_default().to_owned(),
                l["result"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let has = |method: &str, target: &str, result: &str| {
        limit_lines
            .iter()
            .filter(|(m, t, r)| m == method && t == target && r.starts_with(result))
            .count()
    };
    assert_eq!(
        has("plan.stop_at_limit", "enabled=true", "ok"),
        1,
        "{limit_lines:?}"
    );
    assert_eq!(
        has("limit.stop", &resumed_id, "5-hour limit"),
        1,
        "{limit_lines:?}"
    );
    assert_eq!(
        has("limit.stop", &prompted_id, "5-hour limit"),
        1,
        "one stop, one alert"
    );
    assert_eq!(
        has("agent.cancel_resume", &prompted_id, "ok"),
        1,
        "{limit_lines:?}"
    );
    assert_eq!(has("limit.resume", &resumed_id, "ok"), 1, "{limit_lines:?}");
    assert_eq!(has("limit.cancel", &resumed_id, ""), 0, "{limit_lines:?}");
    handle.shutdown().await;
}
