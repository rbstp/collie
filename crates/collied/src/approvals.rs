use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::Engine;
use protocol::{
    AgentStatus, Approval, ApprovalDecideParams, ApprovalId, ApprovalOutcome, Decision, ErrorCode,
    Event, Nonce, Response, TerminalId,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::broadcast;
use zeroize::Zeroizing;

use crate::audit::Audit;
use crate::drive::{Authorized, Reply, herdr_fail};
use crate::flock;
use crate::herdr::{self, AgentInfo, WorkspaceInfo};
use crate::prompt::{self, Menu};
use crate::push::{self, Push};

pub const TTL: Duration = Duration::from_secs(600);
pub const SETTLE: Duration = Duration::from_secs(3);
const SETTLE_POLL: Duration = Duration::from_millis(250);
const NAV_PAUSE: Duration = Duration::from_millis(150);
const RESOLVED_KEPT: usize = 256;
const DECIDE_PER_SEC: f64 = 1.0;
const DECIDE_BURST: f64 = 5.0;
const MAX_LABEL_CHARS: usize = 64;
// Keeps a lock-screen decision inside iOS's background budget: no key is sent once it has
// passed, and the settle wait after the last key adds at most SETTLE, so a decision takes
// under 10 s.
const DECIDE_BUDGET: Duration = Duration::from_secs(6);
// Reissues of a prompt (after a burned nonce) alert at most this often per terminal, so
// bad attempts cannot flood the phone; the reissued approval still reaches live sessions.
const ALERT_GAP: Duration = Duration::from_secs(30);

/// What collied saw on an agent's screen. Never leaves collied except as the snippet and
/// the sealed alert context.
#[derive(Clone)]
struct Screen {
    pane_id: String,
    seq: u64,
    blocked: bool,
    menu: Option<Menu>,
    offered: Vec<(Decision, usize)>,
    fingerprint: [u8; 32],
    /// The fingerprint with the menu cursor on each option in turn.
    cursor_at: Vec<[u8; 32]>,
    snippet: String,
    context: String,
}

struct Pending {
    approval: Approval,
    screen: Screen,
    deciding: bool,
}

struct Alerted {
    at: Instant,
    seq: u64,
    fingerprint: [u8; 32],
    expires_at_ms: u64,
}

#[derive(Default)]
struct Inner {
    pending: HashMap<String, Pending>,
    resolved: VecDeque<ApprovalId>,
    buckets: HashMap<String, (f64, Instant)>,
    alerted: HashMap<String, Alerted>,
}

impl Inner {
    fn remember(&mut self, id: ApprovalId) {
        if self.resolved.len() == RESOLVED_KEPT {
            self.resolved.pop_front();
        }
        self.resolved.push_back(id);
    }

    /// A new blocked episode, a changed screen or an expired previous alert always
    /// alerts; the same prompt reissued within ALERT_GAP does not.
    fn alert_due(&mut self, terminal: &str, screen: &Screen, a: &Approval) -> bool {
        let now = Instant::now();
        self.alerted
            .retain(|_, last| now.duration_since(last.at) < ALERT_GAP);
        if self.alerted.get(terminal).is_some_and(|last| {
            last.seq == screen.seq
                && last.fingerprint == screen.fingerprint
                && a.created_at_ms < last.expires_at_ms
        }) {
            return false;
        }
        self.alerted.insert(
            terminal.to_owned(),
            Alerted {
                at: now,
                seq: screen.seq,
                fingerprint: screen.fingerprint,
                expires_at_ms: a.expires_at_ms,
            },
        );
        true
    }
}

pub struct Approvals {
    herdr: PathBuf,
    node_id: String,
    events: broadcast::Sender<Event>,
    audit: Arc<Audit>,
    push: Option<Arc<Push>>,
    ttl: Duration,
    settle: Duration,
    budget: Duration,
    inner: Mutex<Inner>,
}

fn fail<T>(code: ErrorCode, message: &str) -> Result<T, (ErrorCode, String)> {
    Err((code, message.to_owned()))
}

fn is_blocked(a: &AgentInfo) -> bool {
    flock::status(&a.agent_status) == AgentStatus::Blocked
}

pub fn fingerprint(
    kind: &str,
    rule: Option<&str>,
    terminal_id: &str,
    session: Option<&str>,
    region: &str,
) -> [u8; 32] {
    let mut h = Sha256::new();
    for part in [
        kind,
        rule.unwrap_or(""),
        terminal_id,
        session.unwrap_or(""),
        region,
    ] {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().into()
}

pub fn nonce_matches(expected: &Nonce, given: &Nonce) -> bool {
    expected
        .as_str()
        .as_bytes()
        .ct_eq(given.as_str().as_bytes())
        .into()
}

fn random(n: usize) -> anyhow::Result<String> {
    let mut bytes = Zeroizing::new(vec![0u8; n]);
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes.as_slice()))
}

fn label(s: &str) -> String {
    prompt::clean(s).chars().take(MAX_LABEL_CHARS).collect()
}

fn agent_label(a: &AgentInfo) -> String {
    [&a.name, &a.terminal_title_stripped, &a.title, &a.agent]
        .into_iter()
        .flatten()
        .map(|s| label(s))
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| "agent".to_owned())
}

/// The APNs title goes through Apple, so it never comes from the terminal title or any
/// title the pane's program reports: the herdr agent name, else the agent kind.
fn alert_title(a: &AgentInfo) -> String {
    [&a.name, &a.agent]
        .into_iter()
        .flatten()
        .map(|s| label(s))
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| "agent".to_owned())
}

fn decision_name(d: Decision) -> &'static str {
    match d {
        Decision::Approve => "approve",
        Decision::ApproveAlways => "approve_always",
        Decision::Deny => "deny",
    }
}

impl Approvals {
    pub fn new(
        herdr: PathBuf,
        node_id: String,
        events: broadcast::Sender<Event>,
        audit: Arc<Audit>,
        push: Option<Arc<Push>>,
    ) -> Self {
        Self {
            herdr,
            node_id,
            events,
            audit,
            push,
            ttl: TTL,
            settle: SETTLE,
            budget: DECIDE_BUDGET,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn with_timing(mut self, ttl: Duration, settle: Duration) -> Self {
        self.ttl = ttl;
        self.settle = settle;
        self
    }

    pub fn with_budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Pending approvals with their nonces: for paired sessions only.
    pub fn pending(&self) -> Vec<Approval> {
        let mut out: Vec<Approval> = self
            .lock()
            .pending
            .values()
            .filter(|p| !p.deciding)
            .map(|p| p.approval.clone())
            .collect();
        out.sort_by_key(|a| a.created_at_ms);
        out
    }

    pub async fn observe(&self, agents: &[AgentInfo], workspaces: &[WorkspaceInfo]) {
        let now = crate::now_ms();
        let mut ended = Vec::new();
        let fresh: Vec<&AgentInfo> = {
            let mut inner = self.lock();
            let done: Vec<(String, ApprovalOutcome)> = inner
                .pending
                .iter()
                .filter(|(_, p)| !p.deciding)
                .filter_map(|(t, p)| {
                    let outcome = match agents.iter().find(|a| a.terminal_id == *t) {
                        Some(a) if is_blocked(a) && a.state_change_seq == p.screen.seq => {
                            if now < p.approval.expires_at_ms {
                                return None;
                            }
                            ApprovalOutcome::Expired
                        }
                        _ => ApprovalOutcome::Superseded,
                    };
                    Some((t.clone(), outcome))
                })
                .collect();
            for (t, outcome) in done {
                if let Some(p) = inner.pending.remove(&t) {
                    inner.remember(p.approval.approval_id.clone());
                    ended.push((p.approval.approval_id, outcome));
                }
            }
            agents
                .iter()
                .filter(|a| is_blocked(a) && !inner.pending.contains_key(&a.terminal_id))
                .collect()
        };
        for (approval_id, outcome) in ended {
            let _ = self.events.send(Event::ApprovalResolved {
                approval_id,
                outcome,
            });
        }
        for a in fresh {
            if let Err(e) = self.create(a, workspaces).await {
                tracing::debug!(terminal = %a.terminal_id, error = %e, "no approval yet");
            }
        }
    }

    async fn screen(&self, a: &AgentInfo) -> Result<Screen, herdr::Error> {
        let kind = a.agent.as_deref().unwrap_or_default();
        let explain = herdr::agent_explain(&self.herdr, &a.pane_id).await?;
        let text = herdr::detection_text(&self.herdr, &a.pane_id).await?;
        let rule = explain.matched_rule.map(|r| r.id);
        let menu = prompt::uses_menu(kind, rule.as_deref())
            .then(|| Menu::parse(&text))
            .flatten();
        let (region, snippet, offered) = match &menu {
            Some(m) => (
                m.region(),
                prompt::snippet(m.tail().iter().map(String::as_str)),
                m.decisions(),
            ),
            None => (
                text.clone(),
                prompt::snippet(prompt::after_last_rule(&text).into_iter()),
                Vec::new(),
            ),
        };
        let session = a.agent_session.as_ref().map(|s| s.value.as_str());
        let print =
            |region: &str| fingerprint(kind, rule.as_deref(), &a.terminal_id, session, region);
        Ok(Screen {
            pane_id: a.pane_id.clone(),
            seq: a.state_change_seq,
            blocked: is_blocked(a),
            fingerprint: print(&region),
            cursor_at: menu.as_ref().map_or_else(Vec::new, |m| {
                (0..m.options.len())
                    .map(|i| print(&m.region_at(i)))
                    .collect()
            }),
            menu,
            offered,
            snippet,
            context: prompt::context(&text),
        })
    }

    async fn create(&self, a: &AgentInfo, workspaces: &[WorkspaceInfo]) -> anyhow::Result<()> {
        let terminal_id = TerminalId::new(a.terminal_id.clone())?;
        let title = alert_title(a);
        let screen = self.screen(a).await?;
        if !screen.blocked {
            return Ok(());
        }
        let now = crate::now_ms();
        let approval = Approval {
            approval_id: ApprovalId::new(random(16)?)?,
            terminal_id,
            agent_label: agent_label(a),
            workspace_label: workspaces
                .iter()
                .find(|w| w.workspace_id == a.workspace_id)
                .map(|w| label(&w.label))
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "workspace".to_owned()),
            snippet: screen.snippet.clone(),
            tool: None,
            options: screen.offered.iter().map(|(d, _)| *d).collect(),
            nonce: Nonce::new(random(protocol::limits::NONCE_BYTES)?)?,
            created_at_ms: now,
            expires_at_ms: now + self.ttl.as_millis() as u64,
        };
        let context = screen.context.clone();
        let alert = {
            let mut inner = self.lock();
            if inner.pending.contains_key(&a.terminal_id) {
                return Ok(());
            }
            let alert = inner.alert_due(&a.terminal_id, &screen, &approval);
            inner.pending.insert(
                a.terminal_id.clone(),
                Pending {
                    approval: approval.clone(),
                    screen,
                    deciding: false,
                },
            );
            alert
        };
        // A reissued alert replaces the dead one, whose approval_id no longer works.
        if let Some(push) = self.push.as_ref().filter(|_| alert) {
            push.notify(push::approval_alert(
                &approval,
                &title,
                &self.node_id,
                &context,
            ));
        }
        let _ = self.events.send(Event::ApprovalNeeded { approval });
        Ok(())
    }

    fn take_token(&self, peer: &str) -> bool {
        let now = Instant::now();
        let mut inner = self.lock();
        let (tokens, at) = inner
            .buckets
            .entry(peer.to_owned())
            .or_insert((DECIDE_BURST, now));
        *tokens =
            (*tokens + now.duration_since(*at).as_secs_f64() * DECIDE_PER_SEC).min(DECIDE_BURST);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn finish(&self, terminal: &str, id: &ApprovalId, outcome: ApprovalOutcome) {
        {
            let mut inner = self.lock();
            if inner
                .pending
                .get(terminal)
                .is_some_and(|p| p.approval.approval_id == *id)
            {
                inner.pending.remove(terminal);
            }
            inner.remember(id.clone());
        }
        let _ = self.events.send(Event::ApprovalResolved {
            approval_id: id.clone(),
            outcome,
        });
    }

    async fn current(&self, terminal: &str) -> Result<Option<Screen>, herdr::Error> {
        let agents = herdr::agent_list(&self.herdr).await?;
        match agents.iter().find(|a| a.terminal_id == terminal) {
            Some(a) => Ok(Some(self.screen(a).await?)),
            None => Ok(None),
        }
    }

    async fn settled(&self, terminal: &str, seq: u64) -> bool {
        let poll = async {
            loop {
                tokio::time::sleep(SETTLE_POLL).await;
                let Ok(agents) = herdr::agent_list(&self.herdr).await else {
                    continue;
                };
                match agents.iter().find(|a| a.terminal_id == terminal) {
                    Some(a) if is_blocked(a) && a.state_change_seq == seq => {}
                    _ => return,
                }
            }
        };
        tokio::time::timeout(self.settle, poll).await.is_ok()
    }

    /// Any attempt on a pending approval burns its nonce, whatever the outcome; an
    /// approval that cannot be used any more is resolved, and the next reconcile issues
    /// a fresh one if the agent is still blocked.
    pub async fn decide(
        &self,
        peer: &str,
        stable_id: &str,
        p: ApprovalDecideParams,
        auth: &Authorized,
    ) -> Reply {
        let id = p.approval_id.clone();
        let decision = decision_name(p.decision);
        let audit = |result: &str| {
            self.audit.log(
                peer,
                "approval.decide",
                Some(id.as_str()),
                &format!("{decision}: {result}"),
            )
        };
        if !self.take_token(stable_id) {
            audit("rejected: rate limited");
            return fail(ErrorCode::RateLimited, "slow down");
        }
        let claimed = {
            let mut inner = self.lock();
            let used = inner.resolved.contains(&id);
            match inner
                .pending
                .values_mut()
                .find(|e| e.approval.approval_id == id)
            {
                Some(e) if !e.deciding => {
                    e.deciding = true;
                    Some((e.approval.clone(), e.screen.clone()))
                }
                Some(_) => None,
                None if used => None,
                None => {
                    drop(inner);
                    audit("rejected: unknown approval");
                    return fail(ErrorCode::ApprovalNotFound, "no such approval");
                }
            }
        };
        let budget = tokio::time::Instant::now() + self.budget;
        let Some((approval, screen)) = claimed else {
            audit("rejected: replayed");
            return fail(ErrorCode::ApprovalAlreadyResolved, "approval already used");
        };
        let terminal = approval.terminal_id.as_str();
        let reject = |outcome, code, why: &str, message: &str| {
            self.finish(terminal, &id, outcome);
            audit(why);
            fail(code, message)
        };
        if crate::now_ms() >= approval.expires_at_ms {
            return reject(
                ApprovalOutcome::Expired,
                ErrorCode::ApprovalExpired,
                "rejected: expired",
                "approval expired",
            );
        }
        if !nonce_matches(&approval.nonce, &p.nonce) {
            return reject(
                ApprovalOutcome::Superseded,
                ErrorCode::ApprovalNonceMismatch,
                "rejected: bad nonce",
                "nonce mismatch",
            );
        }
        let Some(&(_, target)) = screen.offered.iter().find(|(d, _)| *d == p.decision) else {
            return reject(
                ApprovalOutcome::Superseded,
                ErrorCode::InvalidParams,
                "rejected: decision not offered",
                "decision not offered",
            );
        };
        let superseded = |why: &str| {
            self.finish(terminal, &id, ApprovalOutcome::Superseded);
            audit(why);
            Ok(Response::ApprovalResolved {
                approval_id: id.clone(),
                outcome: ApprovalOutcome::Superseded,
            })
        };
        let now = match tokio::time::timeout_at(budget, self.current(terminal)).await {
            Ok(Ok(now)) => now,
            Ok(Err(e)) => {
                self.finish(terminal, &id, ApprovalOutcome::Superseded);
                audit("failed: herdr unavailable");
                return Err(herdr_fail(e));
            }
            Err(_) => return superseded("superseded: herdr too slow"),
        };
        let now =
            now.filter(|s| s.blocked && s.seq == screen.seq && s.fingerprint == screen.fingerprint);
        let (Some(now), Some(menu)) = (now.as_ref(), screen.menu.as_ref()) else {
            return superseded("superseded: fingerprint mismatch");
        };
        if tokio::time::Instant::now() >= budget {
            return superseded("superseded: herdr too slow");
        }
        if !auth() {
            return reject(
                ApprovalOutcome::Superseded,
                ErrorCode::NotPaired,
                "rejected: peer no longer authorized",
                "peer is no longer authorized",
            );
        }
        let (nav, confirm) = menu.keys(target);
        let pane = now.pane_id.as_str();
        let sent = |keys: &[&str]| format!("terminal={terminal} keys={}", keys.join(","));
        if !nav.is_empty() {
            match tokio::time::timeout_at(budget, herdr::agent_send_keys(&self.herdr, pane, &nav))
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    self.finish(terminal, &id, ApprovalOutcome::Superseded);
                    audit("failed: keys not sent");
                    return Err(herdr_fail(e));
                }
                Err(_) => return superseded(&format!("superseded: herdr too slow {}", sent(&nav))),
            }
            // Never one call with the arrows and Enter: arrows that were lost or landed on
            // another option would confirm that option.
            tokio::time::sleep(NAV_PAUSE).await;
            let moved = tokio::time::timeout_at(budget, self.current(terminal)).await;
            let on_target = matches!(&moved, Ok(Ok(Some(s)))
                if s.blocked && s.seq == screen.seq
                    && screen.cursor_at.get(target) == Some(&s.fingerprint));
            if !on_target {
                return superseded(&format!("superseded: cursor not on target {}", sent(&nav)));
            }
            if !auth() {
                return reject(
                    ApprovalOutcome::Superseded,
                    ErrorCode::NotPaired,
                    "rejected: peer no longer authorized",
                    "peer is no longer authorized",
                );
            }
            // An elapsed timeout_at still polls its future once, which can send the key.
            if tokio::time::Instant::now() >= budget {
                return superseded(&format!("superseded: herdr too slow {}", sent(&nav)));
            }
        }
        let keys: Vec<&str> = nav.iter().copied().chain([confirm]).collect();
        let settled = match tokio::time::timeout_at(
            budget,
            herdr::agent_send_keys(&self.herdr, pane, &[confirm]),
        )
        .await
        {
            Ok(Ok(())) => Some(self.settled(terminal, screen.seq).await),
            Ok(Err(e)) => {
                self.finish(terminal, &id, ApprovalOutcome::Superseded);
                if nav.is_empty() {
                    audit("failed: keys not sent");
                } else {
                    audit(&format!("failed: {confirm} not sent {}", sent(&nav)));
                }
                return Err(herdr_fail(e));
            }
            // The key may still have reached the agent.
            Err(_) => None,
        };
        let (decision, by) = (p.decision, peer.to_owned());
        let (outcome, result) = match settled {
            Some(true) => (ApprovalOutcome::Applied { decision, by }, "applied"),
            Some(false) => (ApprovalOutcome::Unconfirmed { decision, by }, "unconfirmed"),
            None => (
                ApprovalOutcome::Unconfirmed { decision, by },
                "unconfirmed: herdr too slow",
            ),
        };
        self.finish(terminal, &id, outcome.clone());
        audit(&format!("{result} {}", sent(&keys)));
        Ok(Response::ApprovalResolved {
            approval_id: id.clone(),
            outcome,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_compare() {
        let a = Nonce::new("A".repeat(43)).unwrap();
        let b = Nonce::new(format!("{}B", "A".repeat(42))).unwrap();
        assert!(nonce_matches(&a, &a.clone()));
        assert!(!nonce_matches(&a, &b));
    }

    #[test]
    fn fresh_ids_and_nonces_are_valid_and_distinct() {
        let n1 = Nonce::new(random(32).unwrap()).unwrap();
        let n2 = Nonce::new(random(32).unwrap()).unwrap();
        assert_ne!(n1, n2);
        assert!(ApprovalId::new(random(16).unwrap()).is_ok());
    }

    #[test]
    fn fingerprint_covers_every_field() {
        let base = fingerprint("claude", Some("r"), "t", Some("s"), "q");
        assert_eq!(base, fingerprint("claude", Some("r"), "t", Some("s"), "q"));
        for other in [
            fingerprint("codex", Some("r"), "t", Some("s"), "q"),
            fingerprint("claude", Some("r2"), "t", Some("s"), "q"),
            fingerprint("claude", None, "t", Some("s"), "q"),
            fingerprint("claude", Some("r"), "t2", Some("s"), "q"),
            fingerprint("claude", Some("r"), "t", None, "q"),
            fingerprint("claude", Some("r"), "t", Some("s"), "q2"),
            fingerprint("claude", Some("rt"), "", Some("s"), "q"),
        ] {
            assert_ne!(base, other);
        }
    }

    #[test]
    fn labels_are_clean_and_bounded() {
        let a: AgentInfo = serde_json::from_value(serde_json::json!({
            "terminal_id": "t", "workspace_id": "w1", "pane_id": "w1:p1", "agent_status": "blocked",
            "agent": "claude", "terminal_title_stripped": format!("x\u{202e}{}", "y".repeat(100)),
        }))
        .unwrap();
        let l = agent_label(&a);
        assert_eq!(l.chars().count(), MAX_LABEL_CHARS);
        assert!(l.starts_with("xy"));
        let bare: AgentInfo = serde_json::from_value(serde_json::json!({
            "terminal_id": "t", "workspace_id": "w1", "pane_id": "w1:p1", "agent_status": "blocked",
        }))
        .unwrap();
        assert_eq!(agent_label(&bare), "agent");
        assert_eq!(alert_title(&bare), "agent");
    }

    #[test]
    fn alert_title_is_never_terminal_text() {
        let a: AgentInfo = serde_json::from_value(serde_json::json!({
            "terminal_id": "t", "workspace_id": "w1", "pane_id": "w1:p1", "agent_status": "blocked",
            "agent": "claude", "title": "Reported title",
            "terminal_title_stripped": "Collie iOS remote control",
        }))
        .unwrap();
        assert_eq!(agent_label(&a), "Collie iOS remote control");
        assert_eq!(alert_title(&a), "claude");
        let named = AgentInfo {
            name: Some("api-fixer".into()),
            ..a
        };
        assert_eq!(alert_title(&named), "api-fixer");
    }
}
