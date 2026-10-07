use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot, watch};

use crate::pairing::{self, WINDOW_TTL};
use crate::peers::{self, Peer};
use crate::server::State;

pub(crate) const MAX_LINE: u64 = 64 * 1024;
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status,
    Pair,
    PeersList,
    PeersRevoke {
        target: String,
    },
    Confirm {
        accept: bool,
    },
    /// Holds the connection open: the pending approval count now, then on every change.
    Watch,
    Hook {
        session_id: String,
        tool_name: String,
        shown: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusInfo {
    pub pid: u32,
    pub backend_state: String,
    pub dns_name: String,
    pub node_id: String,
    pub port: u16,
    pub sessions: usize,
    pub peers: usize,
    pub herdr_version: Option<String>,
    /// The node's own tags, as the tailnet reports them; None when the node status failed
    /// or the daemon predates this field.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// What the phone would see; None when herdr is unreachable or it would not fit.
    #[serde(default)]
    pub flock: Option<StatusFlock>,
    #[serde(default)]
    pub flock_too_large: bool,
}

impl StatusInfo {
    /// Drops the flock when the reply would not fit on one control line, so the status
    /// request, also the liveness probe, never fails on a large herdr session.
    pub(crate) fn fit(mut self) -> Self {
        let len = serde_json::to_vec(&self).map_or(usize::MAX, |v| v.len());
        if len as u64 >= MAX_LINE - 1024 {
            self.flock = None;
            self.flock_too_large = true;
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusFlock {
    pub workspaces: Vec<protocol::Workspace>,
    pub agents: Vec<protocol::Agent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub device_label: String,
    pub node_name: String,
    pub stable_id: String,
    pub login: String,
    pub user_id: i64,
    pub tls_key: protocol::KeyPin,
    #[serde(default)]
    pub terminal_key: Option<protocol::TerminalKey>,
    /// This node is paired already: confirming replaces its record.
    #[serde(default)]
    pub replaces: bool,
    #[serde(default)]
    pub previous_terminal_key: Option<protocol::TerminalKey>,
}

impl Candidate {
    /// For the y/N prompt: a compromised app can only swap the terminal key through a
    /// pairing confirmed on the machine.
    pub fn terminal_key_change(&self) -> &'static str {
        match (&self.terminal_key, &self.previous_terminal_key) {
            (None, _) => "none",
            (Some(_), None) => "new",
            (Some(k), Some(p)) if k == p => "unchanged",
            (Some(_), Some(_)) => "replaces the existing one",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Status(StatusInfo),
    Invite {
        uri: String,
        expires_in_secs: u64,
    },
    Confirm(Candidate),
    PairDone {
        paired: bool,
        detail: String,
    },
    Peers {
        owner_user_id: Option<i64>,
        peers: Vec<Peer>,
    },
    Revoked {
        peer: Peer,
        closed_sessions: usize,
    },
    Noted,
    Watch {
        pending_approvals: usize,
    },
    Error {
        message: String,
    },
}

pub struct PairAttempt {
    pub candidate: Candidate,
    pub code_ok: bool,
    pub reply: oneshot::Sender<bool>,
}

pub struct Client {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Client {
    pub async fn connect(path: &Path) -> std::io::Result<Self> {
        let (r, w) = UnixStream::connect(path).await?.into_split();
        Ok(Self {
            reader: BufReader::new(r),
            writer: w,
        })
    }

    pub async fn send(&mut self, req: &Request) -> std::io::Result<()> {
        write_msg(&mut self.writer, req).await
    }

    pub async fn recv(&mut self) -> std::io::Result<Reply> {
        read_msg(&mut self.reader)
            .await?
            .ok_or_else(|| std::io::ErrorKind::UnexpectedEof.into())
    }

    pub async fn call(&mut self, req: &Request) -> std::io::Result<Reply> {
        self.send(req).await?;
        self.recv().await
    }
}

pub async fn request(path: &Path, req: &Request) -> std::io::Result<Option<Reply>> {
    let mut client = match Client::connect(path).await {
        Ok(c) => c,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    tokio::time::timeout(Duration::from_secs(10), client.call(req))
        .await
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::TimedOut))?
        .map(Some)
}

pub async fn daemon_running(path: &Path) -> bool {
    matches!(
        request(path, &Request::Status).await,
        Ok(Some(Reply::Status(_)))
    )
}

async fn write_msg<T: Serialize>(w: &mut OwnedWriteHalf, msg: &T) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(msg)?;
    line.push(b'\n');
    w.write_all(&line).await
}

async fn read_msg<T: DeserializeOwned>(
    r: &mut BufReader<OwnedReadHalf>,
) -> std::io::Result<Option<T>> {
    let mut buf = Vec::new();
    r.take(MAX_LINE + 1).read_until(b'\n', &mut buf).await?;
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.last() != Some(&b'\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "control line too long or truncated",
        ));
    }
    Ok(Some(serde_json::from_slice(&buf)?))
}

/// Binds `<data dir>/control.sock` 0600. The data dir is already 0700, and every
/// connection is additionally checked with getpeereid.
pub async fn bind(path: &Path) -> anyhow::Result<UnixListener> {
    if std::fs::symlink_metadata(path).is_ok() {
        anyhow::ensure!(
            !daemon_running(path).await,
            "another collied is already running ({} answers)",
            path.display()
        );
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub fn same_user(stream: &UnixStream) -> bool {
    stream
        .peer_cred()
        .is_ok_and(|c| uid_allowed(c.uid(), rustix::process::geteuid().as_raw()))
}

fn uid_allowed(peer: u32, euid: u32) -> bool {
    peer == euid
}

pub async fn serve(listener: UnixListener, state: Arc<State>, mut shutdown: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            res = listener.accept() => match res {
                Ok((stream, _)) => {
                    if !same_user(&stream) {
                        state.audit.log("control", "control.connect", None, "rejected: foreign uid");
                        continue;
                    }
                    tokio::spawn(handle(stream, state.clone(), shutdown.clone()));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "control accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            _ = shutdown.changed() => return,
        }
    }
}

async fn handle(stream: UnixStream, state: Arc<State>, shutdown: watch::Receiver<bool>) {
    let (r, mut w) = stream.into_split();
    let mut r = BufReader::new(r);
    let req = match read_msg::<Request>(&mut r).await {
        Ok(Some(req)) => req,
        Ok(None) => return,
        Err(e) => {
            let _ = write_msg(
                &mut w,
                &Reply::Error {
                    message: e.to_string(),
                },
            )
            .await;
            return;
        }
    };
    let reply = match req {
        Request::Status => Reply::Status(state.status_info().await),
        Request::PeersList => {
            let store = state.peers();
            Reply::Peers {
                owner_user_id: store.owner_user_id,
                peers: store.peers,
            }
        }
        Request::PeersRevoke { target } => match state.revoke(&target) {
            Ok((peer, closed_sessions)) => Reply::Revoked {
                peer,
                closed_sessions,
            },
            Err(e) => Reply::Error {
                message: e.to_string(),
            },
        },
        Request::Hook {
            session_id,
            tool_name,
            shown,
        } => {
            if let Some(report) = crate::hooks::Report::new(&tool_name, shown) {
                state.approvals.hook(session_id, report);
            }
            Reply::Noted
        }
        Request::Pair => return pair(&state, &mut r, &mut w).await,
        Request::Watch => return watch_pending(&state, &mut r, &mut w, shutdown).await,
        Request::Confirm { .. } => Reply::Error {
            message: "no pairing in progress".into(),
        },
    };
    let _ = write_msg(&mut w, &reply).await;
}

async fn pair(state: &State, r: &mut BufReader<OwnedReadHalf>, w: &mut OwnedWriteHalf) {
    let code = match pairing::new_code() {
        Ok(c) => c,
        Err(e) => {
            let _ = write_msg(
                w,
                &Reply::Error {
                    message: e.to_string(),
                },
            )
            .await;
            return;
        }
    };
    let (tx, mut rx) = mpsc::channel::<PairAttempt>(1);
    let now = Instant::now();
    let Ok(window) = state.lock_pairing().open(code.clone(), now, tx) else {
        let _ = write_msg(
            w,
            &Reply::Error {
                message: "a pairing window is already open".into(),
            },
        )
        .await;
        return;
    };
    state
        .audit
        .log("control", "pair.open", None, "window opened");
    let invite = Reply::Invite {
        uri: state.invite(code).to_uri(),
        expires_in_secs: WINDOW_TTL.as_secs(),
    };
    if write_msg(w, &invite).await.is_err() {
        state.end_window(window);
        return;
    }
    let deadline = tokio::time::Instant::from_std(now + WINDOW_TTL);
    let attempt = tokio::select! {
        a = rx.recv() => a,
        _ = tokio::time::sleep_until(deadline) => None,
        _ = read_msg::<Request>(r) => {
            state.end_window(window);
            state.audit.log("control", "pair.close", None, "cancelled");
            return;
        }
    };
    state.end_window(window);
    let Some(attempt) = attempt else {
        state.audit.log("control", "pair.close", None, "expired");
        let _ = write_msg(w, &done(false, "pairing window expired")).await;
        return;
    };
    let c = attempt.candidate;
    if !attempt.code_ok {
        let _ = attempt.reply.send(false);
        let _ = write_msg(w, &done(false, &format!("wrong code from {}", c.node_name))).await;
        return;
    }
    let accept = write_msg(w, &Reply::Confirm(c.clone())).await.is_ok()
        && matches!(
            tokio::time::timeout(CONFIRM_TIMEOUT, read_msg::<Request>(r)).await,
            Ok(Ok(Some(Request::Confirm { accept: true })))
        );
    let outcome = if accept {
        state.add_peer(Peer {
            stable_id: c.stable_id.clone(),
            user_id: c.user_id,
            login: c.login.clone(),
            label: c.device_label.clone(),
            paired_at: crate::now_ms(),
            tls_key: Some(c.tls_key.clone()),
            terminal_key: c.terminal_key.clone(),
        })
    } else {
        Err(anyhow::anyhow!("not confirmed on the machine"))
    };
    let (paired, detail) = match outcome {
        Ok(()) => (true, format!("paired {} ({})", c.device_label, c.stable_id)),
        Err(e) => (false, format!("{e:#}")),
    };
    state.audit.log(
        &c.stable_id,
        "pair.confirm",
        Some(&c.device_label),
        if paired { "paired" } else { "refused" },
    );
    let _ = attempt.reply.send(paired);
    let _ = write_msg(w, &done(paired, &detail)).await;
}

/// Read-only like status, and not audited: it mutates nothing. Any input or EOF from the
/// client, or the daemon's shutdown, ends it.
async fn watch_pending(
    state: &State,
    r: &mut BufReader<OwnedReadHalf>,
    w: &mut OwnedWriteHalf,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut rx = state.approvals.watch_pending();
    let closed = read_msg::<Request>(r);
    tokio::pin!(closed);
    loop {
        let n = *rx.borrow_and_update();
        if write_msg(
            w,
            &Reply::Watch {
                pending_approvals: n,
            },
        )
        .await
        .is_err()
        {
            return;
        }
        tokio::select! {
            res = rx.changed() => if res.is_err() { return },
            _ = &mut closed => return,
            _ = shutdown.changed() => return,
        }
    }
}

fn done(paired: bool, detail: &str) -> Reply {
    Reply::PairDone {
        paired,
        detail: detail.to_owned(),
    }
}

pub fn revoke_offline(data_dir: &Path, target: &str) -> anyhow::Result<Peer> {
    let _lock = peers::lock(&data_dir.join(crate::config::PEERS_LOCK))?;
    let path = data_dir.join(crate::config::PEERS_FILE);
    let mut store = peers::load(&path)?;
    let peer = store.remove(target)?;
    peers::save(&path, &store)?;
    crate::audit::Audit::open(&data_dir.join(crate::config::AUDIT_FILE))?.log(
        "control",
        "peers.revoke",
        Some(&peer.stable_id),
        "revoked (offline)",
    );
    Ok(peer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn uid_check_and_socket_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let listener = bind(&path).await.unwrap();
        let mode = std::fs::symlink_metadata(&path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let client = tokio::spawn({
            let path = path.clone();
            async move { UnixStream::connect(&path).await.unwrap() }
        });
        let (server_side, _) = listener.accept().await.unwrap();
        let _client = client.await.unwrap();
        assert!(same_user(&server_side));
        let cred = server_side.peer_cred().unwrap();
        assert_eq!(cred.uid(), rustix::process::geteuid().as_raw());
        assert!(uid_allowed(cred.uid(), cred.uid()));
        assert!(!uid_allowed(0, cred.uid().max(1)));
        assert!(!uid_allowed(cred.uid() + 1, cred.uid()));

        drop(listener);
        assert!(!daemon_running(&path).await);
        let again = bind(&path).await;
        assert!(again.is_ok(), "stale socket is replaced");
    }

    #[tokio::test]
    async fn refuses_second_daemon() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let listener = bind(&path).await.unwrap();
        tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            let (r, mut w) = s.into_split();
            let mut r = BufReader::new(r);
            let _ = read_msg::<Request>(&mut r).await;
            let info = StatusInfo {
                pid: 1,
                backend_state: "Running".into(),
                dns_name: "m".into(),
                node_id: "n".into(),
                port: 1,
                sessions: 0,
                peers: 0,
                herdr_version: None,
                tags: None,
                flock: None,
                flock_too_large: false,
            };
            write_msg(&mut w, &Reply::Status(info)).await.unwrap();
        });
        assert!(bind(&path).await.is_err());
    }

    #[test]
    fn status_drops_a_flock_that_would_not_fit() {
        let workspace = |i: usize| {
            serde_json::json!({"workspace_id": format!("w{i}"), "label": "x".repeat(200),
                "number": i, "status": "idle", "cwd": null})
        };
        let flock = |n: usize| -> StatusFlock {
            serde_json::from_value(serde_json::json!({
                "workspaces": (0..n).map(workspace).collect::<Vec<_>>(), "agents": []
            }))
            .unwrap()
        };
        let info = |n| StatusInfo {
            pid: 1,
            backend_state: "Running".into(),
            dns_name: "m".into(),
            node_id: "n".into(),
            port: 1,
            sessions: 0,
            peers: 0,
            herdr_version: None,
            tags: Some(vec!["tag:collie-linux".into()]),
            flock: Some(flock(n)),
            flock_too_large: false,
        };
        let small = info(3).fit();
        assert!(small.flock.is_some() && !small.flock_too_large);
        let big = info(1000).fit();
        assert!(big.flock.is_none() && big.flock_too_large);
        assert!(serde_json::to_vec(&big).unwrap().len() < MAX_LINE as usize);
        // A reply from a daemon that predates tags and flock still decodes.
        let old: StatusInfo = serde_json::from_str(
            r#"{"pid":1,"backend_state":"Running","dns_name":"m","node_id":"n","port":1,"sessions":0,"peers":0,"herdr_version":null}"#,
        )
        .unwrap();
        assert_eq!(old.tags, None);
    }

    #[test]
    fn request_wire_format() {
        let req: Request =
            serde_json::from_str(r#"{"cmd":"peers_revoke","target":"phone"}"#).unwrap();
        assert!(matches!(req, Request::PeersRevoke { target } if target == "phone"));
        assert!(
            serde_json::from_str::<Request>(r#"{"cmd":"peers_revoke","target":"a","x":1}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"shell"}"#).is_err());
        // The menu bar app sends these exact lines.
        assert!(matches!(
            serde_json::from_str(r#"{"cmd":"watch"}"#).unwrap(),
            Request::Watch
        ));
        for (line, want) in [
            (r#"{"cmd":"pair"}"#, Request::Pair),
            (r#"{"cmd":"peers_list"}"#, Request::PeersList),
            (r#"{"cmd":"watch"}"#, Request::Watch),
            (
                r#"{"cmd":"confirm","accept":true}"#,
                Request::Confirm { accept: true },
            ),
        ] {
            assert_eq!(serde_json::to_string(&want).unwrap(), line);
        }
        assert_eq!(
            serde_json::to_string(&Reply::Watch {
                pending_approvals: 1
            })
            .unwrap(),
            r#"{"type":"watch","pending_approvals":1}"#
        );
    }
}
