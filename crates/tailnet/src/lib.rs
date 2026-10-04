use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_int};
use std::net::{IpAddr, SocketAddr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use tailscale_sys as sys;
use tokio::io::unix::AsyncFd;
use zeroize::Zeroizing;

mod listeners;
pub use listeners::kernel_tcp_listeners;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("tailscale: {0}")]
    Tailscale(String),
    #[error("tailscale call failed with errno {0}")]
    Errno(c_int),
    #[error("state dir {0} must be a directory with mode 0700 owned by the current user")]
    InsecureStateDir(PathBuf),
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("decode: {0}")]
    Decode(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Config {
    pub state_dir: PathBuf,
    pub hostname: String,
    pub auth_key: Option<Zeroizing<String>>,
    pub control_url: Option<String>,
    pub advertise_tags: Vec<String>,
    pub log_to_stderr: bool,
}

// libtailscale keeps one last-error string per node, so a failing call and the
// errmsg read that follows hold this lock to keep another call from replacing it.
struct Handle(c_int, Mutex<()>);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { sys::tailscale_close(self.0) };
    }
}

#[derive(Clone)]
pub struct Node {
    handle: Arc<Handle>,
}

impl Node {
    /// Creates the state dir 0700 if missing and refuses one that is group/world accessible.
    pub fn new(config: &Config) -> Result<Self> {
        ensure_private_dir(&config.state_dir)?;
        let handle = Arc::new(Handle(unsafe { sys::tailscale_new() }, Mutex::new(())));
        let node = Self { handle };
        let dir = cstring(
            config
                .state_dir
                .to_str()
                .ok_or(Error::InvalidArgument("state_dir"))?,
        )?;
        node.check(|| unsafe { sys::tailscale_set_dir(node.sd(), dir.as_ptr()) })?;
        let hostname = cstring(&config.hostname)?;
        node.check(|| unsafe { sys::tailscale_set_hostname(node.sd(), hostname.as_ptr()) })?;
        if let Some(key) = &config.auth_key {
            let key = Zeroizing::new(cstring(key)?.into_bytes_with_nul());
            node.check(|| unsafe {
                sys::tailscale_set_authkey(node.sd(), key.as_ptr() as *const c_char)
            })?;
        }
        if !config.advertise_tags.is_empty() {
            if !config.advertise_tags.iter().all(|t| is_tag(t)) {
                return Err(Error::InvalidArgument("advertise_tags"));
            }
            let tags = cstring(&config.advertise_tags.join(","))?;
            node.check(|| unsafe { sys::tailscale_set_advertise_tags(node.sd(), tags.as_ptr()) })?;
        }
        if let Some(url) = &config.control_url {
            let url = cstring(url)?;
            node.check(|| unsafe { sys::tailscale_set_control_url(node.sd(), url.as_ptr()) })?;
        }
        let log_fd = if config.log_to_stderr {
            // libtailscale wraps the fd in an os.File it may close, so hand it a dup.
            let fd = unsafe { libc::dup(libc::STDERR_FILENO) };
            if fd < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            fd
        } else {
            -1
        };
        node.check(|| unsafe { sys::tailscale_set_logfd(node.sd(), log_fd) })?;
        Ok(node)
    }

    fn sd(&self) -> c_int {
        self.handle.0
    }

    /// Non-blocking: brings the backend up and starts interactive login if needed.
    /// Progress (`BackendState`, `AuthURL`) is observed through [`Node::status`].
    pub fn start(&self) -> Result<()> {
        self.check(|| unsafe { sys::tailscale_start(self.sd()) })
    }

    pub fn status(&self) -> Result<Status> {
        let json = self.json_call(|out| unsafe { sys::tailscale_status_json(self.sd(), out) })?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn whois(&self, addr: &str) -> Result<WhoIs> {
        let addr = cstring(addr)?;
        let json = self
            .json_call(|out| unsafe { sys::tailscale_whois_json(self.sd(), addr.as_ptr(), out) })?;
        Ok(serde_json::from_str(&json)?)
    }

    /// Blocking; call from a blocking context.
    pub fn dial(&self, network: &str, addr: &str) -> Result<UnixStream> {
        let network = cstring(network)?;
        let addr = cstring(addr)?;
        let mut fd: c_int = -1;
        self.check_unlocked(unsafe {
            sys::tailscale_dial(self.sd(), network.as_ptr(), addr.as_ptr(), &mut fd)
        })?;
        Ok(unsafe { UnixStream::from_raw_fd(fd) })
    }

    /// Blocking; the libtailscale dial itself is bounded, so it is safe on a blocking pool.
    pub fn dial_timeout(&self, network: &str, addr: &str, timeout: Duration) -> Result<UnixStream> {
        let ms = c_int::try_from(timeout.as_millis())
            .ok()
            .filter(|ms| *ms > 0)
            .ok_or(Error::InvalidArgument("timeout"))?;
        let network = cstring(network)?;
        let addr = cstring(addr)?;
        let mut fd: c_int = -1;
        self.check_unlocked(unsafe {
            sys::tailscale_dial_timeout(self.sd(), network.as_ptr(), addr.as_ptr(), ms, &mut fd)
        })?;
        Ok(unsafe { UnixStream::from_raw_fd(fd) })
    }

    pub fn listen(&self, network: &str, addr: &str) -> Result<Listener> {
        let network = cstring(network)?;
        let addr = cstring(addr)?;
        let mut fd: c_int = -1;
        self.check(|| unsafe {
            sys::tailscale_listen(self.sd(), network.as_ptr(), addr.as_ptr(), &mut fd)
        })?;
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Listener {
            node: self.clone(),
            fd: AsyncFd::new(fd)?,
        })
    }

    fn json_call(&self, call: impl FnOnce(*mut *mut c_char) -> c_int) -> Result<String> {
        let mut out: *mut c_char = std::ptr::null_mut();
        let _call = self.lock();
        let rc = call(&mut out);
        if rc != 0 || out.is_null() {
            return Err(self.error(rc));
        }
        let json = unsafe { CStr::from_ptr(out) }
            .to_string_lossy()
            .into_owned();
        unsafe { libc::free(out.cast()) };
        Ok(json)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.handle.1.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn check(&self, call: impl FnOnce() -> c_int) -> Result<()> {
        let _call = self.lock();
        self.check_unlocked(call())
    }

    // Dials block for seconds and would stall every other call (the whois gate
    // included) behind the lock. Their message can still be replaced by a
    // concurrent failure; it is diagnostic text only and nothing matches on it.
    fn check_unlocked(&self, rc: c_int) -> Result<()> {
        if rc == 0 { Ok(()) } else { Err(self.error(rc)) }
    }

    fn error(&self, rc: c_int) -> Error {
        if rc != -1 {
            return Error::Errno(rc);
        }
        let mut buf = [0 as c_char; 1024];
        unsafe { sys::tailscale_errmsg(self.sd(), buf.as_mut_ptr(), buf.len()) };
        let msg = unsafe { CStr::from_ptr(buf.as_ptr()) };
        Error::Tailscale(msg.to_string_lossy().into_owned())
    }
}

pub struct Listener {
    node: Node,
    fd: AsyncFd<OwnedFd>,
}

pub struct Accepted {
    pub stream: tokio::net::UnixStream,
    pub peer: SocketAddr,
}

impl Listener {
    pub async fn accept(&self) -> Result<Accepted> {
        loop {
            let mut guard = self.fd.readable().await?;
            if !poll_readable(self.fd.get_ref().as_raw_fd())? {
                guard.clear_ready();
                continue;
            }
            let mut fd: c_int = -1;
            let mut addr = [0 as c_char; 128];
            let res = self.node.check(|| unsafe {
                sys::tailscale_accept_with_addr(
                    self.fd.get_ref().as_raw_fd(),
                    &mut fd,
                    addr.as_mut_ptr(),
                    addr.len(),
                )
            });
            if !poll_readable(self.fd.get_ref().as_raw_fd())? {
                guard.clear_ready();
            }
            res?;
            let conn = unsafe { UnixStream::from_raw_fd(fd) };
            let peer = unsafe { CStr::from_ptr(addr.as_ptr()) }
                .to_str()
                .ok()
                .and_then(|s| s.parse::<SocketAddr>().ok())
                .ok_or(Error::Tailscale("accept: unparseable peer address".into()))?;
            conn.set_nonblocking(true)?;
            return Ok(Accepted {
                stream: tokio::net::UnixStream::from_std(conn)?,
                peer,
            });
        }
    }
}

fn poll_readable(fd: RawFd) -> std::io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // poll(2) is never restarted on Darwin, and the Go runtime signals threads
    // that have called into it (SIGURG preemption), so EINTR does happen here.
    loop {
        let n = unsafe { libc::poll(&mut pfd, 1, 0) };
        if n >= 0 {
            return Ok(n > 0 && pfd.revents & (libc::POLLIN | libc::POLLHUP) != 0);
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

fn is_tag(t: &str) -> bool {
    t.strip_prefix("tag:").is_some_and(|name| {
        name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

fn cstring(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::InvalidArgument("interior NUL"))
}

fn ensure_private_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let meta = std::fs::symlink_metadata(dir)?;
    let uid = unsafe { libc::getuid() };
    use std::os::unix::fs::MetadataExt;
    if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 || meta.uid() != uid {
        return Err(Error::InsecureStateDir(dir.to_owned()));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendState {
    NoState,
    NeedsLogin,
    NeedsMachineAuth,
    Stopped,
    Starting,
    Running,
    Other,
}

impl<'de> Deserialize<'de> for BackendState {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(match String::deserialize(d)?.as_str() {
            "NoState" => Self::NoState,
            "NeedsLogin" => Self::NeedsLogin,
            "NeedsMachineAuth" => Self::NeedsMachineAuth,
            "Stopped" => Self::Stopped,
            "Starting" => Self::Starting,
            "Running" => Self::Running,
            _ => Self::Other,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Status {
    pub backend_state: BackendState,
    #[serde(rename = "AuthURL", default)]
    pub auth_url: String,
    #[serde(rename = "Self")]
    pub self_node: Option<PeerStatus>,
    #[serde(default)]
    pub peer: Option<BTreeMap<String, PeerStatus>>,
    pub current_tailnet: Option<TailnetStatus>,
    #[serde(default)]
    pub health: Option<Vec<String>>,
}

impl Status {
    /// Phone side: the node that owns `ip` according to our netmap.
    pub fn peer_by_ip(&self, ip: IpAddr) -> Option<&PeerStatus> {
        self.peer
            .as_ref()?
            .values()
            .find(|p| p.tailscale_ips.iter().flatten().any(|a| *a == ip))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PeerStatus {
    #[serde(rename = "ID")]
    pub stable_id: String,
    #[serde(default)]
    pub host_name: String,
    #[serde(rename = "DNSName", default)]
    pub dns_name: String,
    #[serde(rename = "UserID", default)]
    pub user_id: i64,
    #[serde(rename = "TailscaleIPs", default)]
    pub tailscale_ips: Option<Vec<IpAddr>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub relay: String,
    #[serde(default)]
    pub cur_addr: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TailnetStatus {
    pub name: String,
    #[serde(rename = "MagicDNSSuffix", default)]
    pub magic_dns_suffix: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WhoIs {
    pub node: WhoIsNode,
    pub user_profile: Option<UserProfile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WhoIsNode {
    #[serde(rename = "StableID")]
    pub stable_id: String,
    pub name: String,
    pub user: i64,
    #[serde(default)]
    pub sharer: i64,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

impl WhoIsNode {
    /// Owned by `owner` directly: not tagged (tags replace user ownership) and not shared in.
    pub fn is_owned_by(&self, owner: i64) -> bool {
        self.tags.as_ref().is_none_or(Vec::is_empty) && self.sharer == 0 && self.user == owner
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserProfile {
    #[serde(rename = "ID")]
    pub id: i64,
    pub login_name: String,
    #[serde(default)]
    pub display_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_grammar() {
        assert!(is_tag("tag:collie-mac"));
        assert!(!is_tag("collie-mac"));
        assert!(!is_tag("tag:"));
        assert!(!is_tag("tag:a,tag:b"));
        assert!(!is_tag("tag:1abc"));
    }

    #[test]
    fn ownership_rule() {
        let node = |user, sharer, tags: Option<Vec<String>>| WhoIsNode {
            stable_id: "n1".into(),
            name: "phone.tailnet.ts.net.".into(),
            user,
            sharer,
            tags,
        };
        assert!(node(7, 0, None).is_owned_by(7));
        assert!(node(7, 0, Some(vec![])).is_owned_by(7));
        assert!(!node(8, 0, None).is_owned_by(7));
        assert!(!node(7, 9, None).is_owned_by(7));
        assert!(!node(7, 0, Some(vec!["tag:collie-phone".into()])).is_owned_by(7));
    }

    #[test]
    fn decodes_status_and_whois() {
        let st: Status = serde_json::from_str(
            r#"{"BackendState":"NeedsLogin","AuthURL":"https://login.tailscale.com/a/x","Self":{"ID":"nSELF","HostName":"collie-mac","DNSName":"","UserID":1,"TailscaleIPs":null,"Online":false},"Peer":null,"CurrentTailnet":null,"Health":["x"],"Version":"1.104.0"}"#,
        )
        .unwrap();
        assert_eq!(st.backend_state, BackendState::NeedsLogin);
        assert!(st.auth_url.starts_with("https://"));
        let who: WhoIs = serde_json::from_str(
            r#"{"Node":{"ID":5,"StableID":"nPHONE","Name":"phone.example.ts.net.","User":7,"Addresses":["100.64.0.2/32"]},"UserProfile":{"ID":7,"LoginName":"me@example.com","DisplayName":"Me"},"CapMap":{}}"#,
        )
        .unwrap();
        assert!(who.node.is_owned_by(7));
        assert_eq!(who.node.stable_id, "nPHONE");
    }

    #[test]
    fn refuses_malformed_advertise_tags() {
        let dir = std::env::temp_dir().join(format!("tailnet-tags-{}", std::process::id()));
        for tags in [
            vec!["collie-mac"],
            vec!["tag:a,tag:b"],
            vec!["tag:ok", "tag:"],
        ] {
            let res = Node::new(&Config {
                state_dir: dir.clone(),
                hostname: "t".into(),
                auth_key: None,
                control_url: None,
                advertise_tags: tags.iter().map(|t| t.to_string()).collect(),
                log_to_stderr: false,
            });
            assert!(
                matches!(res, Err(Error::InvalidArgument("advertise_tags"))),
                "{tags:?}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_open_state_dir() {
        let dir = std::env::temp_dir().join(format!("tailnet-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            ensure_private_dir(&dir),
            Err(Error::InsecureStateDir(_))
        ));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(ensure_private_dir(&dir).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
