use std::sync::Arc;
use std::time::{Duration, Instant};

use protocol::{
    Approval, ApprovalDecideParams, ApprovalId, ApprovalOutcome, Decision, Empty, ErrorCode, Event,
    Nonce, PromptText, Request, Response,
};
use tailnet::{BackendState, Node};
use tokio::net::UnixStream;

use crate::conn::{self, LinkPhase, blocking};
use crate::pin;
use crate::session::{FlockState, Session, SessionError, lock, unexpected};
use crate::store::Machine;
use crate::{Inner, POLL_INTERVAL, ms};

/// `Choose` picks `PendingApproval.choices[choice]`: collied takes it only for an
/// approval with no `options`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ApprovalDecision {
    Approve,
    ApproveAlways,
    Deny,
    Choose { choice: u8 },
}

impl ApprovalDecision {
    fn wire(self) -> (Decision, Option<u8>) {
        match self {
            Self::Approve => (Decision::Approve, None),
            Self::ApproveAlways => (Decision::ApproveAlways, None),
            Self::Deny => (Decision::Deny, None),
            Self::Choose { choice } => (Decision::Choose, Some(choice)),
        }
    }

    fn received(decision: Decision, choice: Option<u8>) -> Option<Self> {
        Some(match (decision, choice) {
            (Decision::Approve, _) => Self::Approve,
            (Decision::ApproveAlways, _) => Self::ApproveAlways,
            (Decision::Deny, _) => Self::Deny,
            (Decision::Choose, Some(choice)) => Self::Choose { choice },
            (Decision::Choose, None) => return None,
        })
    }
}

pub(crate) fn decide_params(
    approval_id: ApprovalId,
    decision: ApprovalDecision,
    nonce: Nonce,
    note: Option<PromptText>,
) -> ApprovalDecideParams {
    let (decision, choice) = decision.wire();
    ApprovalDecideParams {
        approval_id,
        decision,
        choice,
        nonce,
        note,
    }
}

/// One option of the menu on the Mac's screen; `current` is under its cursor.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ApprovalChoice {
    pub index: u8,
    pub label: String,
    pub current: bool,
}

/// An approval as Swift sees it: the nonce never leaves collie-core.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PendingApproval {
    pub approval_id: String,
    pub terminal_id: String,
    pub agent_label: String,
    pub workspace_label: String,
    pub snippet: String,
    pub tool_name: Option<String>,
    pub tool_summary: Option<String>,
    pub options: Vec<ApprovalDecision>,
    /// The menu as collied read it, also when it offers no `options`.
    pub choices: Vec<ApprovalChoice>,
    /// collied takes keys and typed text on this prompt; false from an older collied.
    pub accepts_input: bool,
    /// The menu's free-text option can be filled with `type_text`: a question's, or a
    /// plan's "Tell Claude what to change" (then without `accepts_input`).
    pub has_text_field: bool,
    /// `decide` takes a note with Approve and Deny; false from an older collied.
    pub supports_note: bool,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

impl From<&Approval> for PendingApproval {
    fn from(a: &Approval) -> Self {
        Self {
            approval_id: a.approval_id.as_str().into(),
            terminal_id: a.terminal_id.as_str().into(),
            agent_label: a.agent_label.clone(),
            workspace_label: a.workspace_label.clone(),
            snippet: a.snippet.clone(),
            tool_name: a.tool.as_ref().map(|t| t.name.clone()),
            tool_summary: a.tool.as_ref().map(|t| t.summary.clone()),
            options: a
                .options
                .iter()
                .filter_map(|d| ApprovalDecision::received(*d, None))
                .collect(),
            choices: a
                .choices
                .iter()
                .map(|c| ApprovalChoice {
                    index: c.index,
                    label: c.label.clone(),
                    current: c.current,
                })
                .collect(),
            accepts_input: a.accepts_input,
            has_text_field: a.has_text_field,
            supports_note: a.supports_note,
            created_at_ms: a.created_at_ms,
            expires_at_ms: a.expires_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum DecisionOutcome {
    /// Keys were sent and the agent left `blocked`.
    Applied {
        decision: ApprovalDecision,
        by: String,
    },
    /// Keys were sent but the agent was still `blocked`: never report this as success.
    Unconfirmed {
        decision: ApprovalDecision,
        by: String,
    },
    Expired,
    Superseded,
    Unknown,
}

impl From<ApprovalOutcome> for DecisionOutcome {
    fn from(o: ApprovalOutcome) -> Self {
        match o {
            ApprovalOutcome::Applied { decision, by } => {
                match ApprovalDecision::received(decision, None) {
                    Some(decision) => Self::Applied { decision, by },
                    None => Self::Unknown,
                }
            }
            ApprovalOutcome::Unconfirmed { decision, by } => {
                match ApprovalDecision::received(decision, None) {
                    Some(decision) => Self::Unconfirmed { decision, by },
                    None => Self::Unknown,
                }
            }
            ApprovalOutcome::Chosen { choice, by } => Self::Applied {
                decision: ApprovalDecision::Choose { choice },
                by,
            },
            ApprovalOutcome::ChosenUnconfirmed { choice, by } => Self::Unconfirmed {
                decision: ApprovalDecision::Choose { choice },
                by,
            },
            ApprovalOutcome::Expired => Self::Expired,
            ApprovalOutcome::Superseded => Self::Superseded,
            ApprovalOutcome::Other => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum ApprovalEvent {
    Needed {
        approval: PendingApproval,
    },
    Resolved {
        approval_id: String,
        outcome: DecisionOutcome,
    },
}

/// `events` are those after the caller's `after_revision`. `missed` means older events
/// were dropped, so the caller should diff `pending` instead. Approvals raised while the
/// link was down arrive through the snapshot only, so they show up in `pending` without
/// an event.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ApprovalFeed {
    pub link: LinkPhase,
    pub revision: u64,
    pub missed: bool,
    pub events: Vec<ApprovalEvent>,
    pub pending: Vec<PendingApproval>,
}

pub(crate) fn feed(link: LinkPhase, state: &FlockState, after_revision: u64) -> ApprovalFeed {
    ApprovalFeed {
        link,
        revision: state.approval_revision,
        missed: state
            .approval_events
            .front()
            .is_some_and(|(rev, _)| *rev > after_revision.saturating_add(1)),
        events: state
            .approval_events
            .iter()
            .filter(|(rev, _)| *rev > after_revision)
            .filter_map(|(_, e)| match e {
                Event::ApprovalNeeded { approval } => Some(ApprovalEvent::Needed {
                    approval: approval.into(),
                }),
                Event::ApprovalResolved {
                    approval_id,
                    outcome,
                } => Some(ApprovalEvent::Resolved {
                    approval_id: approval_id.as_str().into(),
                    outcome: outcome.clone().into(),
                }),
                _ => None,
            })
            .collect(),
        pending: state
            .flock
            .iter()
            .flat_map(|f| &f.approvals)
            .map(Into::into)
            .collect(),
    }
}

pub(crate) fn cached_nonce(state: &FlockState, id: &ApprovalId) -> Option<Nonce> {
    state
        .flock
        .iter()
        .flat_map(|f| &f.approvals)
        .find(|a| a.approval_id == *id)
        .map(|a| a.nonce.clone())
}

pub(crate) fn listed_nonce(
    response: Response,
    id: &ApprovalId,
) -> Result<Option<Nonce>, SessionError> {
    match response {
        Response::Approvals { approvals } => Ok(approvals
            .into_iter()
            .find(|a| a.approval_id == *id)
            .map(|a| a.nonce)),
        other => Err(unexpected(&other)),
    }
}

pub(crate) fn expect_resolved(response: Response) -> Result<ApprovalOutcome, SessionError> {
    match response {
        Response::ApprovalResolved { outcome, .. } => Ok(outcome),
        other => Err(unexpected(&other)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DecideStage {
    NodeUp,
    Connect,
    Lookup,
    Decide,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum BackgroundOutcome {
    Applied {
        decision: ApprovalDecision,
    },
    /// Keys were sent but the agent was still `blocked`.
    Unconfirmed {
        decision: ApprovalDecision,
    },
    Expired,
    /// The prompt changed or was answered on the Mac.
    Superseded,
    AlreadyResolved,
    /// Not pending on the Mac any more (answered, expired, or never existed).
    NotFound,
    UnknownMachine,
    /// At `Decide` the request may have reached collied: the outcome is unknown.
    Unreachable {
        stage: DecideStage,
        message: String,
    },
    /// Pin violation, unpaired, or the tailnet needs an interactive login.
    Unauthorized {
        message: String,
    },
    Failed {
        message: String,
    },
}

/// Step timings are set for the steps that completed.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct BackgroundDecideReport {
    pub outcome: BackgroundOutcome,
    pub node_was_running: bool,
    pub node_up_ms: Option<u64>,
    pub connect_ms: Option<u64>,
    pub lookup_ms: Option<u64>,
    pub decide_ms: Option<u64>,
    pub total_ms: u64,
}

impl BackgroundDecideReport {
    pub(crate) fn failed(message: String) -> Self {
        Self {
            outcome: BackgroundOutcome::Failed { message },
            node_was_running: false,
            node_up_ms: None,
            connect_ms: None,
            lookup_ms: None,
            decide_ms: None,
            total_ms: 0,
        }
    }
}

/// One-shot session for the lock-screen action, independent of the connection
/// supervisor (whose sockets iOS may have killed while suspended). Never retries past
/// the budget.
pub(crate) async fn decide_in_background(
    inner: Arc<Inner>,
    node_id: String,
    approval_id: String,
    decision: ApprovalDecision,
    budget: Duration,
) -> BackgroundDecideReport {
    let t0 = Instant::now();
    let mut report = BackgroundDecideReport {
        outcome: BackgroundOutcome::NotFound,
        node_was_running: false,
        node_up_ms: None,
        connect_ms: None,
        lookup_ms: None,
        decide_ms: None,
        total_ms: 0,
    };
    let result = attempt(
        &inner,
        &mut report,
        &node_id,
        approval_id,
        decision,
        t0 + budget,
    )
    .await;
    report.outcome = result.unwrap_or_else(|o| o);
    if matches!(report.outcome, BackgroundOutcome::Unreachable { .. }) {
        inner.reach.record(&node_id, false);
    }
    report.total_ms = ms(t0.elapsed());
    report
}

async fn attempt(
    inner: &Arc<Inner>,
    report: &mut BackgroundDecideReport,
    node_id: &str,
    approval_id: String,
    decision: ApprovalDecision,
    deadline: Instant,
) -> Result<BackgroundOutcome, BackgroundOutcome> {
    let approval_id = ApprovalId::new(approval_id).map_err(|_| BackgroundOutcome::NotFound)?;
    let machine = lock(&inner.machines)
        .iter()
        .find(|m| m.node_id == node_id)
        .cloned()
        .ok_or(BackgroundOutcome::UnknownMachine)?;

    let step = Instant::now();
    report.node_was_running = lock(&inner.node).is_some();
    let node = node_up(inner, &machine, deadline).await?;
    report.node_up_ms = Some(ms(step.elapsed()));

    let step = Instant::now();
    let opened = within(
        deadline,
        DecideStage::Connect,
        conn::open(
            node,
            &machine.host,
            machine.port,
            &machine.node_id,
            Some(machine.kind),
            true,
        ),
    )
    .await?;
    let mut session = match opened {
        Ok((_, hello, _)) if !hello.paired => {
            return Err(BackgroundOutcome::Unauthorized {
                message: SessionError::NotPaired.to_string(),
            });
        }
        Ok((session, _, _)) => session,
        Err(e) if e.is_auth() => {
            return Err(BackgroundOutcome::Unauthorized {
                message: e.to_string(),
            });
        }
        Err(e) => {
            return Err(BackgroundOutcome::Unreachable {
                stage: DecideStage::Connect,
                message: e.to_string(),
            });
        }
    };
    inner.reach.record(&machine.node_id, true);
    report.connect_ms = Some(ms(step.elapsed()));

    let step = Instant::now();
    let conn = lock(&inner.conns).get(&machine.id).cloned();
    let cached = conn.and_then(|c| cached_nonce(&lock(&c.shared.flock), &approval_id));
    let nonce = match cached {
        Some(nonce) => nonce,
        None => {
            let response = call(
                &mut session,
                Request::ApprovalList(Empty {}),
                deadline,
                DecideStage::Lookup,
            )
            .await?;
            listed_nonce(response, &approval_id)
                .map_err(|e| failed(e, DecideStage::Lookup))?
                .ok_or(BackgroundOutcome::NotFound)?
        }
    };
    report.lookup_ms = Some(ms(step.elapsed()));

    let step = Instant::now();
    let request = Request::ApprovalDecide(decide_params(approval_id, decision, nonce, None));
    let response = call(&mut session, request, deadline, DecideStage::Decide).await?;
    report.decide_ms = Some(ms(step.elapsed()));
    tokio::spawn(session.close());
    let outcome = expect_resolved(response).map_err(|e| failed(e, DecideStage::Decide))?;
    Ok(match DecisionOutcome::from(outcome) {
        DecisionOutcome::Applied { decision, .. } => BackgroundOutcome::Applied { decision },
        DecisionOutcome::Unconfirmed { decision, .. } => {
            BackgroundOutcome::Unconfirmed { decision }
        }
        DecisionOutcome::Expired => BackgroundOutcome::Expired,
        DecisionOutcome::Superseded => BackgroundOutcome::Superseded,
        DecisionOutcome::Unknown => BackgroundOutcome::Failed {
            message: "unrecognized outcome, open Collie".into(),
        },
    })
}

/// Starts the node from its cached state (never a login) and waits until the pinned
/// Mac is in the netmap.
async fn node_up(
    inner: &Arc<Inner>,
    machine: &Machine,
    deadline: Instant,
) -> Result<Arc<Node>, BackgroundOutcome> {
    if lock(&inner.node).is_none() {
        if !inner.tailnet_configured() {
            return Err(BackgroundOutcome::Unauthorized {
                message: "Tailscale is not set up on this phone".into(),
            });
        }
        let i = inner.clone();
        within(
            deadline,
            DecideStage::NodeUp,
            blocking(move || i.node_start(None)),
        )
        .await?
        .map_err(|e| BackgroundOutcome::Failed {
            message: e.to_string(),
        })?
        .map_err(|e| BackgroundOutcome::Failed {
            message: e.to_string(),
        })?;
    }
    loop {
        let current = lock(&inner.node).clone();
        if let Some(node) = current {
            let n = node.clone();
            if let Ok(Ok(status)) =
                within(deadline, DecideStage::NodeUp, blocking(move || n.status())).await?
            {
                match status.backend_state {
                    BackendState::Running => match pin::resolve(
                        &status,
                        &machine.host,
                        &machine.node_id,
                        Some(machine.kind),
                    ) {
                        Ok(_) => return Ok(node),
                        Err(e) if e.is_violation() => {
                            return Err(BackgroundOutcome::Unauthorized {
                                message: e.to_string(),
                            });
                        }
                        Err(_) => {}
                    },
                    BackendState::NeedsLogin | BackendState::NeedsMachineAuth => {
                        return Err(BackgroundOutcome::Unauthorized {
                            message: "Tailscale needs to sign in again, open Collie".into(),
                        });
                    }
                    _ => {}
                }
            }
        }
        if Instant::now() + POLL_INTERVAL >= deadline {
            return Err(BackgroundOutcome::Unreachable {
                stage: DecideStage::NodeUp,
                message: "the tailnet did not come up in time".into(),
            });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn call(
    session: &mut Session<UnixStream>,
    request: Request,
    deadline: Instant,
    stage: DecideStage,
) -> Result<Response, BackgroundOutcome> {
    session
        .call(request, remaining(deadline))
        .await
        .map_err(|e| failed(e, stage))
}

fn failed(e: SessionError, stage: DecideStage) -> BackgroundOutcome {
    let message = e.to_string();
    match e {
        _ if e.is_auth() => BackgroundOutcome::Unauthorized { message },
        _ if e.is_transport() => BackgroundOutcome::Unreachable { stage, message },
        SessionError::Server { code, .. } => match code {
            ErrorCode::ApprovalExpired => BackgroundOutcome::Expired,
            ErrorCode::ApprovalAlreadyResolved => BackgroundOutcome::AlreadyResolved,
            ErrorCode::ApprovalNotFound => BackgroundOutcome::NotFound,
            _ => BackgroundOutcome::Failed { message },
        },
        _ => BackgroundOutcome::Failed { message },
    }
}

async fn within<T>(
    deadline: Instant,
    stage: DecideStage,
    fut: impl Future<Output = T>,
) -> Result<T, BackgroundOutcome> {
    tokio::time::timeout(remaining(deadline), fut)
        .await
        .map_err(|_| BackgroundOutcome::Unreachable {
            stage,
            message: "out of time".into(),
        })
}

fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

#[cfg(test)]
mod tests {
    use protocol::{Flock, MachineInfo};

    use super::*;

    const NONCE: &str = "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";

    fn approval(id: &str) -> Approval {
        serde_json::from_value(serde_json::json!({
            "approval_id": id, "terminal_id": "t1", "agent_label": "claude",
            "workspace_label": "collie", "snippet": "Run cargo test?",
            "tool": {"name": "Bash", "summary": "cargo test"},
            "options": ["approve", "approve_always", "deny"], "nonce": NONCE,
            "created_at_ms": 1, "expires_at_ms": 2
        }))
        .unwrap()
    }

    fn state() -> FlockState {
        let mut s = FlockState::default();
        s.apply_snapshot(Flock {
            seq: 0,
            machine: MachineInfo {
                name: "mac".into(),
                node_id: "nMAC".into(),
                herdr_session: "default".into(),
            },
            workspaces: Vec::new(),
            agents: Vec::new(),
            approvals: vec![approval("a0")],
        });
        s
    }

    #[test]
    fn choices_map_both_ways() {
        let id = || ApprovalId::new("a1").unwrap();
        let nonce = || Nonce::new(NONCE).unwrap();
        let p = decide_params(id(), ApprovalDecision::Choose { choice: 2 }, nonce(), None);
        assert_eq!((p.decision, p.choice), (Decision::Choose, Some(2)));
        let p = decide_params(id(), ApprovalDecision::Deny, nonce(), None);
        assert_eq!((p.decision, p.choice), (Decision::Deny, None));
        assert_eq!(
            DecisionOutcome::from(ApprovalOutcome::ChosenUnconfirmed {
                choice: 2,
                by: "mac".into()
            }),
            DecisionOutcome::Unconfirmed {
                decision: ApprovalDecision::Choose { choice: 2 },
                by: "mac".into()
            }
        );
        assert_eq!(
            DecisionOutcome::from(ApprovalOutcome::Applied {
                decision: Decision::Choose,
                by: "mac".into()
            }),
            DecisionOutcome::Unknown,
            "a choice without its index"
        );
        let mut a = approval("a1");
        a.options.push(Decision::Choose);
        a.choices = vec![protocol::ApprovalChoice {
            index: 0,
            label: "Yes".into(),
            current: true,
        }];
        let pending = PendingApproval::from(&a);
        assert_eq!(pending.options.len(), 3);
        assert_eq!(
            pending.choices,
            [ApprovalChoice {
                index: 0,
                label: "Yes".into(),
                current: true
            }]
        );
    }

    #[test]
    fn feed_reports_events_after_the_revision_without_nonces() {
        let mut s = state();
        s.apply_event(
            1,
            Event::ApprovalNeeded {
                approval: approval("a1"),
            },
        );
        s.apply_event(
            2,
            Event::ApprovalResolved {
                approval_id: ApprovalId::new("a0").unwrap(),
                outcome: ApprovalOutcome::Applied {
                    decision: Decision::Deny,
                    by: "mac".into(),
                },
            },
        );
        let all = feed(LinkPhase::Connected, &s, 0);
        assert_eq!((all.revision, all.missed, all.events.len()), (2, false, 2));
        assert_eq!(all.pending.len(), 1);
        assert_eq!(all.pending[0].approval_id, "a1");
        assert_eq!(all.pending[0].tool_name.as_deref(), Some("Bash"));
        assert_eq!(
            all.pending[0].options,
            vec![
                ApprovalDecision::Approve,
                ApprovalDecision::ApproveAlways,
                ApprovalDecision::Deny
            ]
        );
        let debug = format!("{all:?}");
        assert!(
            !debug.contains(NONCE) && !debug.contains("Nonce"),
            "{debug}"
        );
        let later = feed(LinkPhase::Connected, &s, 1);
        assert_eq!(
            later.events,
            vec![ApprovalEvent::Resolved {
                approval_id: "a0".into(),
                outcome: DecisionOutcome::Applied {
                    decision: ApprovalDecision::Deny,
                    by: "mac".into()
                }
            }]
        );
        assert!(feed(LinkPhase::Connected, &s, 2).events.is_empty());
        s.approval_events.pop_front();
        assert!(feed(LinkPhase::Connected, &s, 0).missed);
        assert!(!feed(LinkPhase::Connected, &s, 1).missed);
    }

    #[test]
    fn nonce_lookup() {
        let s = state();
        let a0 = ApprovalId::new("a0").unwrap();
        assert_eq!(cached_nonce(&s, &a0).unwrap().as_str(), NONCE);
        assert!(cached_nonce(&s, &ApprovalId::new("a9").unwrap()).is_none());
        let list = Response::Approvals {
            approvals: vec![approval("a0")],
        };
        assert_eq!(listed_nonce(list, &a0).unwrap().unwrap().as_str(), NONCE);
        assert!(listed_nonce(Response::Ok, &a0).is_err());
    }

    #[test]
    fn errors_map_to_background_outcomes() {
        let server = |code| SessionError::Server {
            code,
            message: "m".into(),
        };
        let stage = DecideStage::Decide;
        assert_eq!(
            failed(server(ErrorCode::ApprovalExpired), stage),
            BackgroundOutcome::Expired
        );
        assert_eq!(
            failed(server(ErrorCode::ApprovalAlreadyResolved), stage),
            BackgroundOutcome::AlreadyResolved
        );
        assert_eq!(
            failed(server(ErrorCode::ApprovalNotFound), stage),
            BackgroundOutcome::NotFound
        );
        assert!(matches!(
            failed(server(ErrorCode::NotPaired), stage),
            BackgroundOutcome::Unauthorized { .. }
        ));
        assert!(matches!(
            failed(server(ErrorCode::ApprovalNonceMismatch), stage),
            BackgroundOutcome::Failed { .. }
        ));
        assert_eq!(
            failed(SessionError::Timeout, stage),
            BackgroundOutcome::Unreachable {
                stage,
                message: SessionError::Timeout.to_string()
            }
        );
    }
}
