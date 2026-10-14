use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use protocol::{
    AgentPromptParams, AgentStatus, DraftText, ErrorCode, PromptText, QueuedPrompt, TerminalId,
};
use serde::{Deserialize, Serialize};

use crate::drive::{self, Authorized, Driver, Fail, Reply, fail};
use crate::herdr::{self, AgentInfo, PaneInfo};
use crate::{attachments, flock, lock, peers};

pub const MAX_ITEMS: usize = 10;
/// Across every queue: the snapshot carries them all to the phone.
pub const MAX_BYTES: usize = 256 * 1024;
/// After a send, how long an agent that is never seen working holds the next item.
pub const NO_TURN: Duration = Duration::from_secs(20);
const GONE: &str = "the prompt is no longer queued";

/// Whether a phone, by `StableID`, is still paired.
pub type Paired = Arc<dyn Fn(&str) -> bool + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<String>,
    /// The paired phone that queued it: revoking that phone drops it.
    pub by: String,
}

impl Item {
    /// Attachment paths, then the text, as the phone builds a prompt it sends now.
    fn composed(&self) -> String {
        let text = self.text.trim();
        let parts: Vec<&str> = (self.attachments.iter().map(String::as_str))
            .chain((!text.is_empty()).then_some(text))
            .collect();
        parts.join(" ")
    }

    pub fn prompt(&self) -> Option<PromptText> {
        PromptText::new(self.composed()).ok()
    }

    fn bytes(&self) -> usize {
        self.prompt().map_or(0, |p| p.as_str().len())
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    next: u64,
    queues: BTreeMap<String, Vec<Item>>,
    /// The agent session each queue is for: another session in the pane drops it.
    #[serde(default)]
    sessions: BTreeMap<String, String>,
}

/// Queued prompts by terminal id, kept on disk so a queue survives a restart.
#[derive(Default)]
pub struct Queues {
    saved: Saved,
    path: Option<PathBuf>,
    /// When an item was last sent to each terminal, and whether the agent has been seen
    /// working since.
    sent: HashMap<String, (Instant, bool)>,
    /// Terminals whose pane showed no agent on the last tick.
    missing: HashSet<String>,
}

impl Queues {
    pub fn load(path: PathBuf) -> Self {
        let saved = peers::load_json(&path).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the queued prompts");
            Saved::default()
        });
        Self {
            saved,
            path: Some(path),
            ..Self::default()
        }
    }

    fn save(&mut self) -> Result<(), peers::Error> {
        self.saved.queues.retain(|_, q| !q.is_empty());
        let queues = &self.saved.queues;
        self.saved.sessions.retain(|t, _| queues.contains_key(t));
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        peers::replace_json(path, &self.saved).inspect_err(|e| {
            tracing::warn!(error = %e, "could not save the queued prompts");
        })
    }

    pub fn is_empty(&self) -> bool {
        self.saved.queues.is_empty()
    }

    pub fn shown(&self, terminal_id: &str) -> Vec<QueuedPrompt> {
        self.saved
            .queues
            .get(terminal_id)
            .into_iter()
            .flatten()
            .map(|i| QueuedPrompt {
                id: i.id,
                text: i.text.clone(),
                attachments: i.attachments.clone(),
            })
            .collect()
    }

    fn bytes(&self) -> usize {
        self.saved.queues.values().flatten().map(Item::bytes).sum()
    }

    fn position(&self, terminal_id: &str, id: u64) -> Result<usize, Fail> {
        self.saved
            .queues
            .get(terminal_id)
            .and_then(|q| q.iter().position(|i| i.id == id))
            .ok_or_else(|| (ErrorCode::NotFound, GONE.to_owned()))
    }

    fn checked(&self, item: &Item, replacing: usize) -> Result<(), Fail> {
        let composed = item.composed();
        if composed.trim().is_empty() {
            return fail(ErrorCode::InvalidParams, "the prompt is empty");
        }
        if composed.len() > protocol::limits::MAX_PROMPT_BYTES {
            return fail(
                ErrorCode::TooLarge,
                "The prompt would be longer than 32 KiB.",
            );
        }
        let Ok(prompt) = PromptText::new(composed) else {
            return fail(
                ErrorCode::InvalidParams,
                "the prompt holds a control character",
            );
        };
        if self.bytes() - replacing + prompt.as_str().len() > MAX_BYTES {
            return fail(
                ErrorCode::TooLarge,
                "the queued prompts are too long; send or remove some first",
            );
        }
        Ok(())
    }

    /// `session` is the agent's session now: a queue left by another session is dropped.
    pub fn add(
        &mut self,
        terminal_id: &str,
        text: &str,
        attachments: Vec<String>,
        (by, session): (&str, Option<&str>),
    ) -> Result<(), Fail> {
        if let (Some(old), Some(now)) = (self.saved.sessions.get(terminal_id), session)
            && old != now
        {
            self.saved.queues.remove(terminal_id);
            self.saved.sessions.remove(terminal_id);
        }
        if self
            .saved
            .queues
            .get(terminal_id)
            .is_some_and(|q| q.len() >= MAX_ITEMS)
        {
            return fail(ErrorCode::TooLarge, "at most 10 prompts can wait per agent");
        }
        let item = Item {
            id: self.saved.next,
            text: text.to_owned(),
            attachments,
            by: by.to_owned(),
        };
        self.checked(&item, 0)?;
        self.saved.next += 1;
        self.saved
            .queues
            .entry(terminal_id.to_owned())
            .or_default()
            .push(item);
        if let Some(session) = session {
            (self.saved.sessions.entry(terminal_id.to_owned()))
                .or_insert_with(|| session.to_owned());
        }
        let _ = self.save();
        Ok(())
    }

    pub fn edit(&mut self, terminal_id: &str, id: u64, text: &str) -> Result<(), Fail> {
        let at = self.position(terminal_id, id)?;
        let mut item = self.saved.queues[terminal_id][at].clone();
        let old = item.bytes();
        item.text = text.to_owned();
        self.checked(&item, old)?;
        self.saved.queues.entry(terminal_id.to_owned()).or_default()[at] = item;
        let _ = self.save();
        Ok(())
    }

    pub fn move_item(&mut self, terminal_id: &str, id: u64, up: bool) -> Result<(), Fail> {
        let at = self.position(terminal_id, id)?;
        let queue = self.saved.queues.entry(terminal_id.to_owned()).or_default();
        let to = if up { at.checked_sub(1) } else { Some(at + 1) };
        if let Some(to) = to.filter(|&to| to < queue.len()) {
            queue.swap(at, to);
            let _ = self.save();
        }
        Ok(())
    }

    pub fn remove(&mut self, terminal_id: &str, id: u64) -> Result<(), Fail> {
        let at = self.position(terminal_id, id)?;
        self.saved
            .queues
            .entry(terminal_id.to_owned())
            .or_default()
            .remove(at);
        let _ = self.save();
        Ok(())
    }

    pub fn get(&self, terminal_id: &str, id: u64) -> Result<Item, Fail> {
        let at = self.position(terminal_id, id)?;
        Ok(self.saved.queues[terminal_id][at].clone())
    }

    /// The first item of each queue.
    pub fn heads(&self) -> Vec<(String, Item)> {
        self.saved
            .queues
            .iter()
            .filter_map(|(t, q)| Some((t.clone(), q.first()?.clone())))
            .collect()
    }

    /// Takes `item` off the queue, only while it is there unchanged (and first, with
    /// `head`), and returns where it was. Only once that is saved: a crash after the write
    /// never sends it twice.
    pub fn claim(&mut self, terminal_id: &str, item: &Item, head: bool) -> Option<usize> {
        let at = self
            .saved
            .queues
            .get(terminal_id)?
            .iter()
            .position(|i| i == item)
            .filter(|&at| !head || at == 0)?;
        let queue = self.saved.queues.entry(terminal_id.to_owned()).or_default();
        let taken = queue.remove(at);
        if self.save().is_err() {
            let queue = self.saved.queues.entry(terminal_id.to_owned()).or_default();
            queue.insert(at, taken);
            return None;
        }
        Some(at)
    }

    /// Puts back an item whose write failed after it was claimed.
    pub fn restore(&mut self, terminal_id: &str, at: usize, item: Item) {
        let queue = self.saved.queues.entry(terminal_id.to_owned()).or_default();
        queue.insert(at.min(queue.len()), item);
        let _ = self.save();
    }

    pub fn sent(&mut self, terminal_id: &str, now: Instant) {
        self.sent.insert(terminal_id.to_owned(), (now, false));
    }

    /// Whether the next item may go to an agent listed with `status`: only idle or done,
    /// and after a send only once the agent has been seen working and is idle again, or
    /// after `NO_TURN` with no turn seen.
    pub fn ready(&mut self, terminal_id: &str, status: AgentStatus, now: Instant) -> bool {
        match status {
            AgentStatus::Working | AgentStatus::Blocked => {
                if let Some((_, started)) = self.sent.get_mut(terminal_id) {
                    *started = true;
                }
                false
            }
            AgentStatus::Idle | AgentStatus::Done => match self.sent.get(terminal_id) {
                Some(&(at, false)) if now.saturating_duration_since(at) < NO_TURN => false,
                _ => {
                    self.sent.remove(terminal_id);
                    true
                }
            },
            AgentStatus::Unknown => false,
        }
    }

    /// Drops the queue of a pane that closed, that holds another agent session than the
    /// one the queue was started for, or that showed no agent on two ticks in a row. A
    /// snapshot with no pane is not trusted, since herdr answers before it restores them.
    pub fn retain_agents(&mut self, panes: &[PaneInfo], agents: &[AgentInfo]) -> bool {
        if panes.is_empty() {
            return false;
        }
        let missing = std::mem::take(&mut self.missing);
        let (mut gone, mut adopted) = (Vec::new(), Vec::new());
        for t in self.saved.queues.keys() {
            let agent = agents.iter().find(|a| &a.terminal_id == t);
            let session = agent.and_then(|a| a.agent_session.as_ref());
            if !panes.iter().any(|p| &p.terminal_id == t) {
                gone.push(t.clone());
            } else if agent.is_none() {
                if missing.contains(t) {
                    gone.push(t.clone());
                } else {
                    self.missing.insert(t.clone());
                }
            } else if let Some(now) = session {
                match self.saved.sessions.get(t) {
                    Some(old) if *old != now.value => gone.push(t.clone()),
                    Some(_) => {}
                    None => adopted.push((t.clone(), now.value.clone())),
                }
            }
        }
        self.sent
            .retain(|t, _| agents.iter().any(|a| &a.terminal_id == t));
        for t in &gone {
            self.saved.queues.remove(t);
            self.sent.remove(t);
        }
        if gone.is_empty() && adopted.is_empty() {
            return false;
        }
        self.saved.sessions.extend(adopted);
        let _ = self.save();
        !gone.is_empty()
    }

    pub fn retain_paired(&mut self, paired: impl Fn(&str) -> bool) -> bool {
        self.retain(|_, i| paired(&i.by))
    }

    pub fn forget(&mut self, by: &str) -> bool {
        self.retain_paired(|b| b != by)
    }

    fn retain(&mut self, keep: impl Fn(&str, &Item) -> bool) -> bool {
        let mut dropped = false;
        for (t, queue) in &mut self.saved.queues {
            let before = queue.len();
            queue.retain(|i| keep(t, i));
            dropped |= queue.len() != before;
        }
        if dropped {
            let _ = self.save();
        }
        dropped
    }

    /// The upload directories of queued attachments, kept past the attachment retention.
    pub fn kept(&self) -> Vec<PathBuf> {
        self.saved
            .queues
            .values()
            .flatten()
            .flat_map(|i| &i.attachments)
            .filter_map(|a| Path::new(a).parent().map(Path::to_path_buf))
            .collect()
    }
}

/// One pass of the queue task: drops the queues whose agent is gone and the items of
/// phones no longer paired, then sends the first item of each queue whose Claude Code agent
/// is ready. Answers whether a queue changed, and each send that reached the claim.
pub async fn tick(
    drive: &Driver,
    herdr_socket: &Path,
    queues: &Arc<Mutex<Queues>>,
    paired: &Paired,
) -> (bool, Vec<(String, Reply)>) {
    if lock(queues).is_empty() {
        return (false, Vec::new());
    }
    let Ok(snap) = herdr::session_snapshot(herdr_socket).await else {
        return (false, Vec::new());
    };
    let mut changed = {
        let mut queues = lock(queues);
        queues.retain_agents(&snap.panes, &snap.agents) | queues.retain_paired(|by| paired(by))
    };
    let mut sent = Vec::new();
    let heads = lock(queues).heads();
    for (terminal_id, item) in heads {
        let Some(agent) = snap.agents.iter().find(|a| a.terminal_id == terminal_id) else {
            continue;
        };
        let status = flock::status(&agent.agent_status);
        if agent.agent.as_deref() != Some("claude")
            || !lock(queues).ready(&terminal_id, status, Instant::now())
        {
            continue;
        }
        let Ok(t) = TerminalId::new(terminal_id.clone()) else {
            continue;
        };
        let owner: Authorized = {
            let (paired, by) = (paired.clone(), item.by.clone());
            Arc::new(move || paired(&by))
        };
        let (reply, claimed) = send(drive, queues, &owner, None, (t, item), None).await;
        if claimed {
            changed = true;
            sent.push((terminal_id, reply));
        }
    }
    (changed, sent)
}

/// Through `Driver::prompt_agent` and every one of its guards, with `idle_only` unless
/// `session` is a phone sending the item now: a box holding text, a blocked or busy agent
/// or a scrolled screen refuses it and nothing is cleared. Right before the write, once
/// `owner` (the phone that queued it) and `session` pass, the item is claimed, which needs
/// it still queued unchanged, and first for a queued send. A write herdr refuses after the
/// claim puts it back; one whose answer was lost does not, since herdr may have typed it.
/// Also answers whether it was claimed, which a deferral never is.
pub async fn send(
    drive: &Driver,
    queues: &Arc<Mutex<Queues>>,
    owner: &Authorized,
    session: Option<&Authorized>,
    (terminal_id, item): (TerminalId, Item),
    expected_draft: Option<DraftText>,
) -> (Reply, bool) {
    let (Some(text), Ok(op_id)) = (item.prompt(), drive::new_op_id()) else {
        return (fail(ErrorCode::InvalidParams, "the prompt is empty"), false);
    };
    let idle_only = session.is_none();
    let claimed = Arc::new(Mutex::new(None));
    let auth: Authorized = {
        let (queues, owner, claimed) = (queues.clone(), owner.clone(), claimed.clone());
        let (session, t, item) = (session.cloned(), terminal_id.clone(), item.clone());
        Arc::new(move || {
            if !session.as_ref().is_none_or(|a| a()) || !owner() {
                return false;
            }
            let mut claimed = lock(&claimed);
            if claimed.is_none() {
                *claimed = lock(&queues).claim(t.as_str(), &item, idle_only);
            }
            claimed.is_some()
        })
    };
    let params = AgentPromptParams {
        op_id,
        terminal_id: terminal_id.clone(),
        text,
        expected_draft,
    };
    let reply = drive.prompt_agent(params, &auth, idle_only).await;
    let Some(at) = lock(&claimed).take() else {
        if matches!(reply, Err((ErrorCode::NotPaired, _))) && session.is_none_or(|a| a()) {
            return (fail(ErrorCode::NotFound, GONE), false);
        }
        return (reply, false);
    };
    match &reply {
        Ok(_) => {
            lock(queues).sent(terminal_id.as_str(), Instant::now());
            for dir in item
                .attachments
                .iter()
                .filter_map(|a| Path::new(a).parent())
            {
                attachments::touch(dir);
            }
        }
        Err((ErrorCode::HerdrUnavailable, _)) => {}
        Err(_) => lock(queues).restore(terminal_id.as_str(), at, item),
    }
    (reply, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(terminal_id: &str, session: &str) -> AgentInfo {
        serde_json::from_value(serde_json::json!({
            "terminal_id": terminal_id, "workspace_id": "w1", "pane_id": "w1:p1",
            "agent_status": "working", "agent_session": {"value": session},
        }))
        .unwrap()
    }

    fn pane(terminal_id: &str) -> PaneInfo {
        serde_json::from_value(serde_json::json!({
            "terminal_id": terminal_id, "workspace_id": "w1", "pane_id": "w1:p1", "tab_id": "w1:t1",
            "cwd": null, "foreground_cwd": null,
        }))
        .unwrap()
    }

    fn texts(q: &Queues, t: &str) -> Vec<String> {
        q.shown(t).into_iter().map(|p| p.text).collect()
    }

    #[test]
    fn items_keep_their_order_and_can_be_edited_moved_and_removed() {
        let mut q = Queues::default();
        for text in ["one", "two", "three"] {
            q.add("t1", text, Vec::new(), ("phone", None)).unwrap();
        }
        q.add("t2", "other", Vec::new(), ("phone", None)).unwrap();
        assert_eq!(texts(&q, "t1"), ["one", "two", "three"]);
        let ids: Vec<u64> = q.shown("t1").iter().map(|p| p.id).collect();
        assert_eq!(ids, [0, 1, 2]);
        q.move_item("t1", 2, true).unwrap();
        assert_eq!(texts(&q, "t1"), ["one", "three", "two"]);
        q.move_item("t1", 0, true).unwrap();
        q.move_item("t1", 1, false).unwrap();
        assert_eq!(
            texts(&q, "t1"),
            ["one", "three", "two"],
            "no move past an end"
        );
        q.edit("t1", 1, "deux").unwrap();
        q.remove("t1", 0).unwrap();
        assert_eq!(texts(&q, "t1"), ["three", "deux"]);
        assert_eq!(q.remove("t1", 0).unwrap_err().0, ErrorCode::NotFound);
        assert_eq!(q.edit("t2", 2, "x").unwrap_err().0, ErrorCode::NotFound);
        let heads: Vec<(String, String)> =
            q.heads().into_iter().map(|(t, i)| (t, i.text)).collect();
        assert_eq!(
            heads,
            [("t1".into(), "three".into()), ("t2".into(), "other".into())]
        );
    }

    #[test]
    fn attachments_come_first_and_the_queue_is_bounded() {
        let mut q = Queues::default();
        let file = "/cache/attachments/0123456789abcdef/shot.png".to_owned();
        q.add("t1", "  look  ", vec![file.clone()], ("phone", None))
            .unwrap();
        q.add("t1", "", vec![file.clone()], ("phone", None))
            .unwrap();
        let heads = q.heads();
        assert_eq!(
            heads[0].1.prompt().unwrap().as_str(),
            format!("{file} look")
        );
        assert_eq!(q.get("t1", 1).unwrap().prompt().unwrap().as_str(), file);
        assert_eq!(
            q.add("t1", " \n", Vec::new(), ("phone", None))
                .unwrap_err()
                .0,
            ErrorCode::InvalidParams
        );
        let full = "a".repeat(protocol::limits::MAX_PROMPT_BYTES);
        assert_eq!(
            q.add("t1", &full, vec![file.clone()], ("phone", None))
                .unwrap_err(),
            (
                ErrorCode::TooLarge,
                "The prompt would be longer than 32 KiB.".to_owned()
            )
        );
        assert_eq!(
            q.kept(),
            vec![PathBuf::from("/cache/attachments/0123456789abcdef"); 2]
        );
        for _ in 2..MAX_ITEMS {
            q.add("t1", "more", Vec::new(), ("phone", None)).unwrap();
        }
        assert_eq!(
            q.add("t1", "one too many", Vec::new(), ("phone", None))
                .unwrap_err()
                .0,
            ErrorCode::TooLarge
        );
        let mut q = Queues::default();
        let long = "a".repeat(protocol::limits::MAX_PROMPT_BYTES);
        for t in 0..MAX_BYTES / long.len() {
            q.add(&format!("t{t}"), &long, Vec::new(), ("phone", None))
                .unwrap();
        }
        assert_eq!(
            q.add("t1", "a", Vec::new(), ("phone", None)).unwrap_err().0,
            ErrorCode::TooLarge
        );
        q.edit("t0", 0, "short").unwrap();
        assert_eq!(
            q.edit("t0", 0, &long).map_err(|e| e.0),
            Ok(()),
            "an edit counts the text it replaces"
        );
    }

    #[test]
    fn a_claimed_item_leaves_the_queue_only_while_unchanged() {
        let mut q = Queues::default();
        q.add("t1", "one", Vec::new(), ("phone", None)).unwrap();
        q.add("t1", "two", Vec::new(), ("phone", None)).unwrap();
        let (_, head) = q.heads().remove(0);
        q.edit("t1", head.id, "edited").unwrap();
        assert_eq!(q.claim("t1", &head, true), None, "edited since it was read");
        let (_, head) = q.heads().remove(0);
        q.move_item("t1", 1, true).unwrap();
        assert_eq!(q.claim("t1", &head, true), None, "no longer first");
        q.move_item("t1", 1, false).unwrap();
        assert_eq!(q.claim("t1", &head, true), Some(0));
        assert_eq!(texts(&q, "t1"), ["two"]);
        q.restore("t1", 0, head);
        assert_eq!(texts(&q, "t1"), ["edited", "two"]);
        let second = q.get("t1", 1).unwrap();
        assert_eq!(q.claim("t1", &second, false), Some(1), "sent now");
        q.restore("t1", 1, second);
        let (_, head) = q.heads().remove(0);
        q.remove("t1", head.id).unwrap();
        assert_eq!(
            q.claim("t1", &head, true),
            None,
            "removed since it was read"
        );
    }

    #[test]
    fn the_next_item_waits_for_the_turn_the_last_one_started() {
        let mut q = Queues::default();
        let start = Instant::now();
        assert!(!q.ready("t1", AgentStatus::Working, start));
        assert!(!q.ready("t1", AgentStatus::Blocked, start));
        assert!(!q.ready("t1", AgentStatus::Unknown, start));
        assert!(q.ready("t1", AgentStatus::Idle, start));
        assert!(q.ready("t1", AgentStatus::Done, start));
        q.sent("t1", start);
        assert!(
            !q.ready("t1", AgentStatus::Idle, start + Duration::from_secs(2)),
            "still idle: the turn has not started"
        );
        assert!(!q.ready("t1", AgentStatus::Working, start + Duration::from_secs(4)));
        assert!(!q.ready("t1", AgentStatus::Blocked, start + Duration::from_secs(6)));
        assert!(q.ready("t1", AgentStatus::Done, start + Duration::from_secs(8)));
        q.sent("t1", start);
        assert!(!q.ready(
            "t1",
            AgentStatus::Idle,
            start + NO_TURN - Duration::from_secs(1)
        ));
        assert!(
            q.ready("t1", AgentStatus::Idle, start + NO_TURN),
            "a turn too short to be seen"
        );
    }

    #[test]
    fn a_closed_pane_or_an_unpaired_phone_drops_its_items() {
        let mut q = Queues::default();
        q.add("t1", "one", Vec::new(), ("a", Some("s1"))).unwrap();
        q.add("t1", "two", Vec::new(), ("b", Some("s1"))).unwrap();
        q.add("t2", "three", Vec::new(), ("a", None)).unwrap();
        q.sent("t2", Instant::now());
        assert!(
            !q.retain_agents(&[], &[]),
            "a snapshot with no pane is not trusted"
        );
        assert!(q.retain_agents(&[pane("t1")], &[agent("t1", "s1")]));
        assert_eq!(texts(&q, "t2"), Vec::<String>::new(), "its pane closed");
        assert!(q.sent.is_empty());
        assert!(q.forget("a"));
        assert_eq!(texts(&q, "t1"), ["two"]);
        assert!(!q.retain_paired(|by| by == "b"));
        assert!(q.retain_paired(|_| false));
        assert!(q.is_empty());
    }

    #[test]
    fn a_queue_ends_with_its_agent_session() {
        let mut q = Queues::default();
        let panes = [pane("t1"), pane("t2")];
        q.add("t1", "one", Vec::new(), ("a", Some("s1"))).unwrap();
        q.add("t2", "two", Vec::new(), ("a", None)).unwrap();
        assert!(!q.retain_agents(&panes, &[agent("t1", "s1"), agent("t2", "s7")]));
        assert_eq!(
            q.saved.sessions["t2"], "s7",
            "a queue started with no session takes the first seen"
        );
        assert!(q.retain_agents(&panes, &[agent("t1", "s1"), agent("t2", "s8")]));
        assert_eq!(
            texts(&q, "t2"),
            Vec::<String>::new(),
            "another session in the pane"
        );
        assert!(!q.retain_agents(&panes, &[]), "no agent for one tick");
        assert!(!q.retain_agents(&panes, &[agent("t1", "s1")]), "back");
        assert!(!q.retain_agents(&panes, &[]));
        assert!(
            q.retain_agents(&panes, &[]),
            "no agent on two ticks in a row"
        );
        assert!(q.is_empty());
        q.add("t1", "old", Vec::new(), ("a", Some("s1"))).unwrap();
        q.add("t1", "new", Vec::new(), ("a", Some("s2"))).unwrap();
        assert_eq!(
            texts(&q, "t1"),
            ["new"],
            "an add for another session drops the old queue"
        );
        assert_eq!(q.saved.sessions["t1"], "s2");
    }

    #[test]
    fn an_unsaved_claim_takes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prompt-queue.json");
        let mut q = Queues::load(path);
        q.add("t1", "one", Vec::new(), ("phone", None)).unwrap();
        let (_, head) = q.heads().remove(0);
        use std::os::unix::fs::PermissionsExt;
        let mode = |m| std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(m));
        mode(0o500).unwrap();
        let claimed = q.claim("t1", &head, true);
        mode(0o700).unwrap();
        assert_eq!(claimed, None);
        assert_eq!(texts(&q, "t1"), ["one"]);
    }

    #[test]
    fn queues_survive_a_restart_in_a_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prompt-queue.json");
        let mut q = Queues::load(path.clone());
        q.add("t1", "one", Vec::new(), ("phone", None)).unwrap();
        q.add("t1", "two", Vec::new(), ("phone", None)).unwrap();
        q.remove("t1", 0).unwrap();
        let mut q = Queues::load(path.clone());
        assert_eq!(texts(&q, "t1"), ["two"]);
        q.add("t1", "three", Vec::new(), ("phone", None)).unwrap();
        assert_eq!(q.shown("t1")[1].id, 2, "ids are never reused");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
