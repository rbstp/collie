use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::client::{Client, Method, TERMINAL, TIMEOUT};
use super::model::{self, Listed, Listing, PREFIX};
use super::wire::{Attached, Blocks, Created, Owners, Process, Sessions, Tagged};
use super::{CLOSE_TIMEOUT, Inner, Rex, gone};
use crate::runtime::{Error, Occupant};

/// The terminal flavor that runs a command through the user's login shell, which reads the
/// account's profile first, so the agent gets the `PATH` the user's own terminals get.
pub(super) const FLAVOR: &str = "com.superlogical.terminal.shell";

/// A task collied started, kept by block id in its data folder: Rex holds no label of
/// collied's own on a block, and its ids are never reused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Task {
    pub name: String,
    /// The checkout collied created or opened for it: only that checkout is ever removed by
    /// an archive.
    #[serde(default)]
    pub worktree: Option<String>,
    /// The session collied created for it: only that session is ever removed, by its archive.
    #[serde(default)]
    pub session: Option<String>,
}

pub(super) type Tasks = BTreeMap<String, Task>;

impl Rex {
    pub fn with_tasks(mut self, path: PathBuf) -> Self {
        let tasks = crate::peers::load_json(&path).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the Rex tasks");
            Tasks::new()
        });
        self.lock().tasks = tasks;
        self.tasks = Some(path);
        self
    }

    fn save_tasks(&self, tasks: &Tasks) {
        if let Some(path) = &self.tasks
            && let Err(e) = crate::peers::replace_json(path, tasks)
        {
            tracing::warn!(error = %e, "could not save the Rex tasks");
        }
    }

    /// A session of the task's own whose one block runs `argv` in `cwd`, as an argument
    /// vector Rex hands to the program unchanged through the login shell. The block is
    /// recorded, given up, then sized as the tallest terminal Rex holds; returns its id.
    pub async fn start_task(
        &self,
        cwd: &str,
        name: &str,
        label: Option<&str>,
        worktree: Option<&str>,
        argv: Vec<String>,
    ) -> Result<String, Error> {
        let client = (self.live(|l| l.synced.then(|| l.client.clone()))?).ok_or(Error::Closed)?;
        let block = json!({
            "flavor": FLAVOR, "label": name, "options": { "command": argv, "cwd": cwd },
        });
        let payload = json!({
            "label": label.unwrap_or(name),
            "initial_windows": [{ "layout": { "block": block } }],
        });
        let (_, created): (_, Created) = (client.call(Method::SessionCreate, payload, TIMEOUT))
            .await
            .map_err(|e| match e {
                Error::Rex { .. } => e,
                e => Error::Unsupported(format!(
                    "Rex's answer to session.create was lost ({e}); an agent may have been started in a new Rex session"
                )),
            })?;
        let session = created.session_id;
        let block = (created.initial_windows.into_iter().flatten())
            .find_map(|w| w.block_ids?.into_iter().next())
            .filter(|b| model::valid(b, "block") && model::valid(&session, "session"))
            .ok_or_else(|| {
                Error::Unsupported(format!(
                    "Rex created session {} without a block collied can name",
                    crate::printable(&session)
                ))
            })?;
        let id = format!("{PREFIX}{block}");
        {
            let mut inner = self.lock();
            let task = Task {
                name: name.to_owned(),
                worktree: worktree.map(str::to_owned),
                session: Some(session.clone()),
            };
            inner.tasks.insert(block.clone(), task);
            self.save_tasks(&inner.tasks);
        }
        disown(&client, &session, &block).await.map_err(|e| {
            Error::Unsupported(format!(
                "Rex started agent {id}, which was left running, but did not let collied give it up: {e}"
            ))
        })?;
        let tallest = self.live(|l| l.model.tallest()).ok().flatten();
        resize(&client, &session, &block, tallest).await;
        Ok(id)
    }

    /// Archive's close of the task in `id`: its block, then the session collied created for
    /// that task, once nothing is left in it. What became of that session, for the archive's
    /// answer; `None` when collied has no record of creating the block's session for this
    /// task, and that session then stays.
    pub async fn close_task(&self, id: &str) -> Result<Option<String>, Error> {
        let (client, session, block, ..) = self.target(id)?;
        let own = (self.lock().tasks.get(&block))
            .is_some_and(|t| t.session.as_deref() == Some(session.as_str()));
        self.close(id).await?;
        if !own {
            return Ok(None);
        }
        Ok(Some(match remove_empty(&client, &session).await {
            Ok(true) => "Removed its Rex session.".to_owned(),
            Ok(false) => {
                "Kept its Rex session, which still holds something or could not be read.".to_owned()
            }
            Err(e) => format!("Could not remove its Rex session ({e})."),
        }))
    }

    /// The checkout recorded for the worktree task collied started in `id`, while Rex lists
    /// its block.
    pub fn worktree(&self, id: &str) -> Option<String> {
        let block = id.strip_prefix(PREFIX)?;
        let inner = self.lock();
        let live = inner.live.as_ref().filter(|l| l.synced)?;
        if !live.model.has(block) {
            return None;
        }
        inner.tasks.get(block)?.worktree.clone()
    }

    /// A block no client owns closes when its program exits; one a client took stays.
    pub fn finished(&self, id: &str) -> bool {
        let inner = self.lock();
        (id.strip_prefix(PREFIX)).is_some_and(|block| {
            inner.ended.contains(block)
                || (inner.live.as_ref()).is_some_and(|live| live.model.exited(block))
        })
    }

    /// Every terminal block on this machine, shells included, read afresh: Rex sends no event
    /// when a shell changes folder, so what the events keep could be stale.
    pub async fn occupants(&self) -> Result<Vec<Occupant>, Error> {
        let (client, host) = (self
            .live(|l| (l.synced).then(|| (l.client.clone(), l.model.host().map(str::to_owned))))?)
        .ok_or(Error::Closed)?;
        let (_, list): (_, Sessions) = client
            .call(Method::SessionList, Value::Null, TIMEOUT)
            .await?;
        let mut found = Vec::new();
        for s in list.sessions.unwrap_or_default() {
            let at = json!({ "session_id": s.session_id });
            let blocks: Blocks = match client.call(Method::SessionListBlocks, at, TIMEOUT).await {
                Ok((_, blocks)) => blocks,
                Err(e) if gone(&e) => continue,
                Err(e) => return Err(e),
            };
            for b in blocks.blocks.unwrap_or_default() {
                let Some(block) = b.block_id else { continue };
                if (host.is_some() && b.host_id != host)
                    || b.creator_name.as_deref() != Some(TERMINAL)
                {
                    continue;
                }
                let process: Process = match client
                    .terminal(Method::Process, &s.session_id, &block, json!({}))
                    .await
                {
                    Ok((_, p)) => p,
                    Err(e) if gone(&e) => continue,
                    Err(e) => return Err(e),
                };
                found.push((block, process));
            }
        }
        let inner = self.lock();
        Ok((found.into_iter())
            .map(|(block, p)| {
                let task = inner.tasks.get(&block);
                Occupant {
                    id: format!("{PREFIX}{block}"),
                    task: task.is_some(),
                    worktree: task.and_then(|t| t.worktree.clone()),
                    cwd: p.child.and_then(|c| c.cwd).unwrap_or_default(),
                    foreground_cwd: p.foreground.and_then(|f| f.cwd),
                }
            })
            .collect())
    }

    pub(super) async fn size_tasks(&self, client: &Client) {
        let (tallest, small) = {
            let inner = self.lock();
            let Some(live) = inner.live.as_ref() else {
                return;
            };
            let small: Vec<(String, String)> = (inner.tasks.keys())
                .filter_map(|b| live.model.target(&format!("{PREFIX}{b}")))
                .filter(|&(_, _, size, listed)| size == UNSIZED && listed == Listed::Agent)
                .map(|(session, block, ..)| (session, block))
                .collect();
            (live.model.tallest(), small)
        };
        for (session, block) in small {
            resize(client, &session, &block, tallest).await;
        }
    }

    pub(super) fn move_task(&self, from: &str, to: &str) {
        let mut inner = self.lock();
        if let Some(task) = inner.tasks.remove(from) {
            inner.tasks.insert(to.to_owned(), task);
            self.save_tasks(&inner.tasks);
        }
    }

    /// A task block that closed or whose program exited, from the event pump: a closed one's
    /// record goes at once, so the file does not grow over one long connection.
    pub(super) fn task_ended(&self, inner: &mut Inner, e: &Tagged) {
        let Some((block, closed)) = ended(e).filter(|(b, _)| inner.tasks.contains_key(b)) else {
            return;
        };
        if closed {
            inner.tasks.remove(&block);
            self.save_tasks(&inner.tasks);
        }
        inner.ended.insert(block);
    }

    /// Once every session is read: a task whose block is gone from the server is forgotten.
    pub(super) fn prune_tasks(&self) {
        let mut inner = self.lock();
        let Inner {
            live, tasks, ended, ..
        } = &mut *inner;
        let Some(live) = live.as_ref() else { return };
        let before = tasks.len();
        tasks.retain(|block, _| live.model.holds(block));
        ended.retain(|block| tasks.contains_key(block));
        if tasks.len() != before {
            self.save_tasks(tasks);
        }
    }
}

/// The block an event says closed (`true`) or whose program exited (`false`).
pub(super) fn ended(e: &Tagged) -> Option<(String, bool)> {
    if e.kind != "session_event" {
        return None;
    }
    let event = &e.payload["event"];
    let closed = match event["type"].as_str()? {
        "block_closed" => true,
        "block_event" if event["payload"]["name"] == "child_exited" => false,
        _ => return None,
    };
    let block = event["payload"]["block_id"].as_str()?;
    Some((block.to_owned(), closed))
}

/// A task's agent shows its recorded name, as herdr's and herdr-core's do.
pub(super) fn name(listing: &mut Listing, tasks: &Tasks) {
    for a in &mut listing.agents {
        let block = a.terminal_id.strip_prefix(PREFIX);
        a.name = block.and_then(|b| tasks.get(b)).map(|t| t.name.clone());
    }
}

/// Removes `session` when it holds no block, detached ones included, and no window; `false`
/// when it holds one or Rex's answer leaves it unclear. Rex has no conditional destroy: a
/// block or window added after `session.list_blocks` answers and before `session.destroy`
/// arrives, one round trip, would end with the session.
async fn remove_empty(client: &Client, session: &str) -> Result<bool, Error> {
    let at = json!({ "session_id": session });
    let (_, attached): (_, Attached) = client
        .call(Method::SessionAttach, at.clone(), TIMEOUT)
        .await?;
    let (_, blocks): (_, Blocks) = client
        .call(Method::SessionListBlocks, at.clone(), TIMEOUT)
        .await?;
    let windowless = attached
        .view
        .is_some_and(|v| v.windows.unwrap_or_default().is_empty());
    if !windowless || blocks.blocks.is_none_or(|b| !b.is_empty()) {
        return Ok(false);
    }
    client
        .call::<Value>(Method::SessionDestroy, at, CLOSE_TIMEOUT)
        .await?;
    Ok(true)
}

const UNSIZED: (u16, u16) = (80, 24);

async fn resize(client: &Client, session: &str, block: &str, tallest: Option<(u16, u16)>) {
    let Some((columns, rows)) = tallest.filter(|&(c, r)| (r, c) > (UNSIZED.1, UNSIZED.0)) else {
        return;
    };
    let size = json!({ "columns": columns, "rows": rows });
    if let Err(e) = (client.terminal::<Value>(Method::Resize, session, block, size)).await {
        tracing::warn!(error = %e, "could not size a Rex task's block");
    }
}

/// `session.create` makes collied the block's owner, whose size (80x24) the block would keep.
/// Giving it up needs the session attached; attaching twice is harmless.
pub(super) async fn disown(client: &Client, session: &str, block: &str) -> Result<(), Error> {
    let at = json!({ "session_id": session });
    let (_, attached): (_, Attached) = client.call(Method::SessionAttach, at, TIMEOUT).await?;
    let payload = json!({ "session_id": session, "block_ids": [block] });
    let (_, owners): (_, Owners) = client
        .call(Method::SessionDisownBlock, payload, TIMEOUT)
        .await?;
    let Some(me) = attached.client_id else {
        return Err(Error::Unsupported(
            "session.attach did not name collied's client".to_owned(),
        ));
    };
    match (owners.blocks.into_iter().flatten()).find(|o| o.block_id.as_deref() == Some(block)) {
        Some(o) if o.owner.as_deref() != Some(me.as_str()) => Ok(()),
        _ => Err(Error::Unsupported(
            "the block is still collied's after session.disown_block".to_owned(),
        )),
    }
}
