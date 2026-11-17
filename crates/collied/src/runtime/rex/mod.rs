use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};

use self::client::{Client, Method, TIMEOUT};
use self::model::Model;
use self::screen::Screens;
use self::wire::{
    Attached, Blocks, Creators, Help, Process, ServerStatus, Sessions, Size, Status, Tagged, Title,
};
use super::herdr::{Explain, MatchedRule};
use super::{AgentInfo, Error, HerdrSeen, PaneRead};

mod client;
mod discovery;
mod input;
mod model;
mod screen;
mod sessions;
mod stream;
mod tasks;
mod wire;

#[cfg(test)]
mod fake;
#[cfg(test)]
mod screen_tests;
#[cfg(test)]
mod tests;

pub use discovery::socket_path;
pub use input::{STATUS_CHANGED, Seen};
pub use model::{Listing, PREFIX};
pub use stream::Watch;

pub const NAME: &str = "Rex";
pub const NOT_YET: &str = "not available for Rex agents yet";
/// `session.focus_block` changes only the server's view: Rex.app 0.1.0-preview.19 neither
/// switched to the session nor came forward (#709).
pub const FOCUS: &str = "a Rex agent cannot be focused from the phone";
pub const GROUP_CLOSE: &str =
    "a Rex session cannot be closed from the phone; close its agents one at a time";
const CLOSE_TIMEOUT: Duration = Duration::from_secs(15);
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

pub fn owns(id: &str) -> bool {
    id.starts_with(model::PREFIX)
}

/// What `collied status` shows of the Rex collied looks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub socket: String,
    #[serde(default)]
    pub folder_mode: Option<u32>,
    /// None while no server answers.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub api_revision: Option<u32>,
    #[serde(default)]
    pub missing: Vec<String>,
    /// Why the server that answers is not followed.
    #[serde(default)]
    pub refused: Option<String>,
    /// How other machines reach the same terminals: built-in Tailscale, tailnet listeners.
    #[serde(default)]
    pub remote: Vec<String>,
}

/// What a server says of itself when collied connects.
#[derive(Debug, Clone, Default)]
pub struct Server {
    pub version: Option<String>,
    /// The newest revision the server speaks.
    pub api_revision: u32,
    pub pid: Option<u32>,
    host: Option<String>,
    pub missing: Vec<&'static str>,
    anonymous: Vec<String>,
    tailnet: Vec<String>,
    tailscale: bool,
}

impl Server {
    /// Why collied does not follow this server.
    pub fn refused(&self) -> Option<String> {
        if !self.missing.is_empty() {
            Some(format!("lacks {}", self.missing.join(", ")))
        } else if !self.anonymous.is_empty() {
            Some(format!(
                "admits anonymous callers on {}, who could type into the same agents",
                self.anonymous.join(", ")
            ))
        } else {
            None
        }
    }

    /// How other machines reach the same terminals, outside collie's allowlist and audit.
    pub fn remote(&self) -> Vec<String> {
        let mut remote = Vec::new();
        if self.tailscale {
            remote.push("built-in Tailscale".to_owned());
        }
        remote.extend(self.tailnet.iter().map(|l| format!("tailnet listener {l}")));
        remote
    }
}

/// The methods this phase calls must be the server's: server methods in `help.all`, terminal
/// methods in `block_creator.list`. Nothing is called before it is checked.
async fn introduce(client: &Client, server_revision: u32) -> Result<Server, Error> {
    let (_, help): (_, Help) = client.call(Method::HelpAll, Value::Null, TIMEOUT).await?;
    let has = |m: Method| help.methods.iter().any(|n| n.name == m.name());
    let mut server = Server {
        api_revision: server_revision,
        missing: (Method::ALL.into_iter())
            .filter(|m| m.terminal().is_none() && !has(*m))
            .map(Method::name)
            .collect(),
        ..Server::default()
    };
    if has(Method::ServerVersion) {
        let (_, v): (_, wire::Version) = client
            .call(Method::ServerVersion, Value::Null, TIMEOUT)
            .await?;
        server.version = Some(crate::printable(&v.rex_version));
    }
    if has(Method::ServerStatus) {
        let (_, s): (_, ServerStatus) = client
            .call(Method::ServerStatus, Value::Null, TIMEOUT)
            .await?;
        server.pid = Some(s.pid);
        server.host = s.host_id;
        server.tailscale = s.tailscale.is_some_and(|t| t.state == "running");
        for l in s.listeners.unwrap_or_default() {
            let endpoint = crate::printable(&l.endpoint);
            if l.admission.as_deref() == Some("anonymous") {
                server.anonymous.push(endpoint.clone());
            }
            if l.transport == "tailscale" || l.endpoint.starts_with("tailnet://") {
                server.tailnet.push(endpoint);
            }
        }
    }
    let terminal = if has(Method::BlockCreatorList) {
        let (_, c): (_, Creators) = client
            .call(Method::BlockCreatorList, Value::Null, TIMEOUT)
            .await?;
        (c.creators.unwrap_or_default().into_iter())
            .find(|c| c.name == client::TERMINAL)
            .map(|c| c.api.methods)
            .unwrap_or_default()
    } else {
        serde_json::Map::new()
    };
    server.missing.extend(
        (Method::ALL.into_iter())
            .filter(|m| m.terminal().is_some_and(|t| !terminal.contains_key(t)))
            .map(Method::name),
    );
    Ok(server)
}

/// A server that took the connection and then failed is running: never "not running".
fn answered(e: Error) -> Error {
    match e {
        Error::Unsupported(_) => e,
        e => Error::Unsupported(format!("accepted the connection, then failed: {e}")),
    }
}

/// Connects and asks the server about itself, without following it; for `collied doctor`.
pub async fn probe(socket: &Path) -> Result<Server, Error> {
    let stream = discovery::connect(socket).await?;
    let answer = async {
        let (client, pump, welcome) = client::open(stream).await?;
        tokio::select! {
            e = pump.run(&client, |_, _| {}) => Err(e),
            server = introduce(&client, welcome.api_server_revision) => server,
        }
    };
    answer.await.map_err(answered)
}

#[derive(Default)]
struct Inner {
    server: Option<Server>,
    refused: Option<String>,
    live: Option<Live>,
    /// Codex sessions by block id, as collied's SessionStart hook reported them, with the
    /// foreground pid they were reported under. Kept outside the model, which a reconnect
    /// drops, and applied when the block is read again.
    codex: BTreeMap<String, (u32, String)>,
    tasks: tasks::Tasks,
    /// Task blocks whose program exited or that closed, as their events said.
    ended: BTreeSet<String>,
}

struct Live {
    client: Arc<Client>,
    model: Model,
    /// The first reads are done: until then, nothing is listed.
    synced: bool,
    /// Sessions whose blocks are to be read.
    dirty: BTreeSet<String>,
}

/// One personal Rex server, followed on one control connection. collied never starts a
/// server and never runs the `rex` CLI: an absent socket means none runs.
pub struct Rex {
    socket: PathBuf,
    claude_sessions: Option<PathBuf>,
    /// Where the tasks collied starts here are recorded.
    tasks: Option<PathBuf>,
    inner: Mutex<Inner>,
    wake: Notify,
    settled: watch::Sender<bool>,
    pub(super) herdr: Arc<HerdrSeen>,
    screens: Screens,
    /// One call's input at a time, so another call's key never lands among its keys.
    input: tokio::sync::Mutex<()>,
    streams: stream::Streams,
}

/// The connection, session, block and size of a listed agent.
type Target = (Arc<Client>, String, String, (u16, u16));

fn gone(e: &Error) -> bool {
    matches!(e, Error::Rex { code, .. } if code == "not_found" || code == "session_not_found")
}

fn not_found() -> Error {
    Error::Rex {
        code: "not_found".to_owned(),
        message: "no such agent".to_owned(),
    }
}

impl Rex {
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            claude_sessions: None,
            tasks: None,
            inner: Mutex::default(),
            wake: Notify::new(),
            settled: watch::channel(false).0,
            herdr: Arc::default(),
            screens: Screens::default(),
            input: tokio::sync::Mutex::default(),
            streams: stream::Streams::default(),
        }
    }

    /// Shared with the followed core, so herdr's loss and return are logged once.
    pub fn with_herdr_seen(mut self, seen: Arc<HerdrSeen>) -> Self {
        self.herdr = seen;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        crate::lock(&self.inner)
    }

    /// From the model the events keep current: no call to the server.
    pub fn listing(&self) -> Option<Listing> {
        let inner = self.lock();
        let live = inner.live.as_ref().filter(|l| l.synced)?;
        let mut listing = live.model.listing();
        tasks::name(&mut listing, &inner.tasks);
        Some(listing)
    }

    pub fn following(&self) -> bool {
        self.lock().live.as_ref().is_some_and(|l| l.synced)
    }

    /// Answered and not refused, but not followed: not read yet, or lost and not found gone
    /// since.
    pub fn reconnecting(&self) -> bool {
        let inner = self.lock();
        inner.server.is_some()
            && inner.refused.is_none()
            && !inner.live.as_ref().is_some_and(|l| l.synced)
    }

    /// Why collied does not follow the server that answered last.
    pub fn refused(&self) -> Option<String> {
        self.lock().refused.clone()
    }

    pub fn get(&self, id: &str) -> Result<AgentInfo, Error> {
        (self.listing())
            .and_then(|l| l.agents.into_iter().find(|a| a.terminal_id == id))
            .ok_or_else(not_found)
    }

    fn target(&self, id: &str) -> Result<Target, Error> {
        let inner = self.lock();
        let live = inner
            .live
            .as_ref()
            .filter(|l| l.synced)
            .ok_or_else(not_found)?;
        let (session, block, size) = live.model.target(id).ok_or_else(not_found)?;
        self.screens.retain(|b| live.model.has(b));
        Ok((live.client.clone(), session, block, size))
    }

    pub fn columns(&self, id: &str) -> Result<Option<u16>, Error> {
        Ok(Some(self.target(id)?.3.0))
    }

    /// Follows the terminal as a stream until the last watch of it is dropped, unless
    /// streaming is off for this run.
    pub fn watch(self: &Arc<Self>, id: &str) -> Result<Option<Watch>, Error> {
        self.target(id)?;
        Ok(self.streams.watch(self, id))
    }

    /// Where a stream in progress is marked, in collied's data folder.
    pub fn with_stream_marker(mut self, marker: PathBuf) -> Self {
        self.streams = stream::Streams::marked(marker);
        self
    }

    /// From the stream while a watch holds it live, else polled, as herdr's reads are.
    pub async fn read(
        &self,
        id: &str,
        source: &str,
        lines: Option<u32>,
    ) -> Result<PaneRead, Error> {
        let (client, session, block, size) = self.target(id)?;
        if let Some(text) = self.streams.read(id, source, lines) {
            return Ok(PaneRead {
                text,
                truncated: false,
            });
        }
        let text = (self.screens)
            .read(&client, &session, &block, size, source, lines)
            .await?;
        Ok(PaneRead {
            text,
            truncated: false,
        })
    }

    /// The visible screen as text, as herdr's `detection` source reads it.
    pub async fn screen_text(&self, id: &str) -> Result<String, Error> {
        Ok(self.read(id, screen::DETECTION, None).await?.text)
    }

    pub fn explain(&self, id: &str) -> Result<Explain, Error> {
        let inner = self.lock();
        let kind = (inner.live.as_ref().filter(|l| l.synced))
            .and_then(|live| live.model.blocked_kind(id))
            .ok_or_else(not_found)?;
        Ok(Explain {
            matched_rule: kind.map(|id| MatchedRule { id }),
        })
    }

    /// Only a block collied lists as an agent. `block.close` ends its program; a window
    /// whose last block closes closes too, and the session stays.
    pub async fn close(&self, id: &str) -> Result<(), Error> {
        let (client, session, block, _) = self.target(id)?;
        let payload = json!({ "session_id": session, "block_id": block });
        client
            .call::<Value>(Method::BlockClose, payload, CLOSE_TIMEOUT)
            .await?;
        Ok(())
    }

    /// Returns once the first connection attempt is over, so a server that is running when
    /// collied starts is listed from the first reconcile.
    pub async fn settled(&self) {
        let _ = self.settled.subscribe().wait_for(|s| *s).await;
    }

    pub fn report(&self) -> Report {
        let inner = self.lock();
        let server = inner.server.as_ref();
        Report {
            socket: self.socket.display().to_string(),
            folder_mode: (self.socket.parent())
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777),
            version: server.and_then(|s| s.version.clone()),
            api_revision: server.map(|s| s.api_revision),
            missing: server.map_or_else(Vec::new, |s| {
                s.missing.iter().map(|m| (*m).to_owned()).collect()
            }),
            refused: inner.refused.clone(),
            remote: server.map_or_else(Vec::new, Server::remote),
        }
    }

    /// A server that never answered stays quiet; one that stops after answering logs once.
    pub async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        let mut delay = RETRY_FIRST;
        let mut logged: Option<String> = None;
        loop {
            let (ended, followed) = tokio::select! {
                ended = self.attempt() => ended,
                _ = shutdown.changed() => return self.streams.unmark(),
            };
            match ended {
                e if followed => {
                    tracing::warn!(socket = %self.socket.display(), error = %e, "Rex unavailable, retrying with backoff");
                    logged = None;
                    delay = RETRY_FIRST;
                }
                Error::Unsupported(why) if logged.as_ref() != Some(&why) => {
                    tracing::warn!(socket = %self.socket.display(), "Rex {why}; not following it");
                    logged = Some(why);
                }
                _ => {}
            }
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = shutdown.changed() => return self.streams.unmark(),
            }
            delay = (delay * 2).min(RETRY_MAX);
        }
    }

    /// One connection, until it ends: why, and whether it was followed. A server that was
    /// never followed keeps why in the report.
    async fn attempt(&self) -> (Error, bool) {
        let ended = self.follow().await;
        let followed = {
            let mut inner = self.lock();
            let followed = inner.live.take().is_some_and(|l| l.synced);
            match &ended {
                _ if followed => {}
                Error::Unsupported(why) => inner.refused = Some(why.clone()),
                _ => (inner.server, inner.refused) = (None, None),
            }
            followed
        };
        self.settled.send_replace(true);
        (ended, followed)
    }

    async fn follow(&self) -> Error {
        let stream = match discovery::connect(&self.socket).await {
            Ok(s) => s,
            Err(e) => return e,
        };
        let (client, pump, welcome) = match client::open(stream).await {
            Ok(opened) => opened,
            Err(e) => return answered(e),
        };
        let client = Arc::new(client);
        let ended = tokio::select! {
            e = pump.run(&client, |seq, e| self.event(seq, e)) => e,
            e = self.sync(&client, welcome.api_server_revision) => e,
        };
        let followed = self.lock().live.as_ref().is_some_and(|l| l.synced);
        if followed { ended } else { answered(ended) }
    }

    /// Never waits: the pump calls it for every event.
    fn event(&self, seq: u64, e: Tagged) {
        let mut inner = self.lock();
        if e.kind == "tailscale_status_changed" {
            let running = e.payload["state"].as_str() == Some("running");
            if let Some(server) = inner.server.as_mut()
                && server.tailscale != running
            {
                server.tailscale = running;
                if running {
                    tracing::warn!(socket = %self.socket.display(), "Rex turned on built-in Tailscale: other machines reach the agents collie follows");
                }
            }
            return;
        }
        self.task_ended(&mut inner, &e);
        let Some(live) = inner.live.as_mut() else {
            return;
        };
        let read = live.model.event(seq, e);
        let dirty = read.map(|session| live.dirty.insert(session)).is_some();
        drop(inner);
        if dirty {
            self.wake.notify_one();
        }
        self.claude(Model::touched);
    }

    /// Subscribes before it reads, so a session or report in between is not missed: events
    /// for every session first, then the list, then each session's events and reads.
    async fn sync(&self, client: &Arc<Client>, server_revision: u32) -> Error {
        if let Err(e) = self.start(client, server_revision).await {
            return e;
        }
        loop {
            self.wake.notified().await;
            if let Err(e) = self.read_dirty(client).await {
                return e;
            }
        }
    }

    async fn start(&self, client: &Arc<Client>, server_revision: u32) -> Result<(), Error> {
        let server = introduce(client, server_revision).await?;
        let refused = server.refused();
        let remote = server.remote();
        let host = server.host.clone();
        let version = server.version.clone().unwrap_or_default();
        {
            let mut inner = self.lock();
            inner.server = Some(server);
            inner.refused.clone_from(&refused);
        }
        if let Some(why) = refused {
            return Err(Error::Unsupported(why));
        }
        if !remote.is_empty() {
            tracing::warn!(socket = %self.socket.display(), remote = %remote.join(", "), "Rex is reachable from other machines: they reach the agents collie follows outside its allowlist and audit");
        }
        self.lock().live = Some(Live {
            client: client.clone(),
            model: Model::new(host),
            synced: false,
            dirty: BTreeSet::new(),
        });
        client
            .call::<Value>(
                Method::SessionAttachAll,
                json!({ "event_scope": "session" }),
                TIMEOUT,
            )
            .await?;
        let (seq, list): (_, Sessions) = client
            .call(Method::SessionList, Value::Null, TIMEOUT)
            .await?;
        self.live(|live| {
            let listed = live.model.sessions(seq, list.sessions.unwrap_or_default());
            live.dirty.extend(listed);
        })?;
        self.read_dirty(client).await?;
        self.live(|live| live.synced = true)?;
        self.prune_tasks();
        self.settled.send_replace(true);
        tracing::info!(socket = %self.socket.display(), version, "following Rex");
        Ok(())
    }

    fn live<T>(&self, f: impl FnOnce(&mut Live) -> T) -> Result<T, Error> {
        self.lock().live.as_mut().map(f).ok_or(Error::Closed)
    }

    /// Reads every session marked, its blocks and theirs, until none is left. A session or
    /// block that went away meanwhile is skipped: its own event removes it.
    async fn read_dirty(&self, client: &Client) -> Result<(), Error> {
        loop {
            let next = self.live(|live| match live.dirty.pop_first() {
                Some(session) => {
                    let attach = live.model.attach(&session);
                    Some((session, attach))
                }
                None => {
                    live.model.settle();
                    None
                }
            })?;
            let Some((session, attach)) = next else {
                return Ok(());
            };
            match self.read_session(client, &session, attach).await {
                Err(e) if !gone(&e) => return Err(e),
                _ => {}
            }
        }
    }

    async fn read_session(
        &self,
        client: &Client,
        session: &str,
        attach: bool,
    ) -> Result<(), Error> {
        let at = json!({ "session_id": session });
        if attach {
            let (seq, attached): (_, Attached) = client
                .call(Method::SessionAttach, at.clone(), TIMEOUT)
                .await?;
            if let Some(label) = attached.view.and_then(|v| v.label) {
                self.live(|live| live.model.relabel(session, seq, label))?;
            }
        }
        let (_, blocks): (_, Blocks) = client.call(Method::SessionListBlocks, at, TIMEOUT).await?;
        let added = self.live(|live| {
            if !live.model.known(session) {
                return Vec::new();
            }
            live.model
                .blocks(session, blocks.blocks.unwrap_or_default())
        })?;
        for block in added {
            match self.read_block(client, session, &block).await {
                Ok(()) => self.live(|live| live.model.recovered(&block))?,
                Err(e @ (Error::Json(_) | Error::Timeout | Error::Rex { .. })) => {
                    let failed = !gone(&e);
                    if self.live(|live| live.model.forget(session, &block, failed))? {
                        tracing::warn!(socket = %self.socket.display(), block, error = %e, "Rex block skipped until it reports again: its reads failed");
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// The size first: once `process` names an agent it is listed, and read at that size.
    async fn read_block(&self, client: &Client, session: &str, block: &str) -> Result<(), Error> {
        let (seq, size): (_, Size) = client
            .terminal(Method::Size, session, block, json!({}))
            .await?;
        self.live(|live| live.model.size(session, block, seq, size))?;
        let (seq, process): (_, Process) = client
            .terminal(Method::Process, session, block, json!({}))
            .await?;
        self.live(|live| live.model.process(session, block, seq, process))?;
        let (seq, status): (_, Status) = client
            .terminal(Method::ProgramStatus, session, block, json!({}))
            .await?;
        self.live(|live| live.model.status(session, block, seq, status.records))?;
        let (seq, title): (_, Title) = client
            .terminal(Method::Title, session, block, json!({}))
            .await?;
        self.live(|live| live.model.title(session, block, seq, title.title))?;
        self.read_sessions(session, block);
        Ok(())
    }
}
