use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: must be a regular file with mode 0600 owned by the current user")]
    Insecure(PathBuf),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("user {got} is not the owner {owner}")]
    NotOwner { owner: i64, got: i64 },
    #[error("label {0:?} is already used by another phone")]
    LabelTaken(String),
    #[error("no paired phone matches {0:?}")]
    NotFound(String),
    #[error("{0:?} matches several phones; use the stable id")]
    Ambiguous(String),
    #[error("{0}: a paired phone does not belong to owner_user_id")]
    ForeignPeer(PathBuf),
    #[error("{0} is held by a running collied or another peers command")]
    Locked(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub stable_id: String,
    pub user_id: i64,
    pub login: String,
    pub label: String,
    pub paired_at: u64,
    /// None for a phone paired before mutual TLS, which pairs again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_key: Option<protocol::KeyPin>,
    /// Signs terminal grants. None for a phone that sent none at pairing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_key: Option<protocol::TerminalKey>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Store {
    pub owner_user_id: Option<i64>,
    pub peers: Vec<Peer>,
}

impl Store {
    pub fn get(&self, stable_id: &str) -> Option<&Peer> {
        self.peers.iter().find(|p| p.stable_id == stable_id)
    }

    /// The first confirmed user becomes the owner; every later peer must be that user.
    /// Re-pairing the same node replaces its entry.
    pub fn add(&mut self, peer: Peer) -> Result<(), Error> {
        if let Some(owner) = self.owner_user_id
            && owner != peer.user_id
        {
            return Err(Error::NotOwner {
                owner,
                got: peer.user_id,
            });
        }
        if self.peers.iter().any(|p| {
            (p.label == peer.label || p.stable_id == peer.label) && p.stable_id != peer.stable_id
        }) {
            return Err(Error::LabelTaken(peer.label));
        }
        self.owner_user_id = Some(peer.user_id);
        self.peers.retain(|p| p.stable_id != peer.stable_id);
        self.peers.push(peer);
        Ok(())
    }

    pub fn remove_node(&mut self, stable_id: &str) -> Result<Peer, Error> {
        let i = self
            .peers
            .iter()
            .position(|p| p.stable_id == stable_id)
            .ok_or_else(|| Error::NotFound(stable_id.to_owned()))?;
        Ok(self.peers.remove(i))
    }

    pub fn remove(&mut self, target: &str) -> Result<Peer, Error> {
        let matches: Vec<usize> = self
            .peers
            .iter()
            .enumerate()
            .filter(|(_, p)| p.label == target || p.stable_id == target)
            .map(|(i, _)| i)
            .collect();
        match matches.as_slice() {
            [] => Err(Error::NotFound(target.to_owned())),
            [i] => Ok(self.peers.remove(*i)),
            _ => Err(Error::Ambiguous(target.to_owned())),
        }
    }
}

// The daemon holds this for its whole life, taken before it loads the store, so an
// offline edit can never be overwritten by a daemon's stale in-memory copy.
pub fn lock(path: &Path) -> Result<OwnedFd, Error> {
    use rustix::fs::{FlockOperation, Mode, OFlags};
    let fd = rustix::fs::open(
        path,
        OFlags::CREATE | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|e| Error::Io {
        path: path.to_owned(),
        source: e.into(),
    })?;
    match rustix::fs::flock(&fd, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(fd),
        Err(rustix::io::Errno::WOULDBLOCK) => Err(Error::Locked(path.to_owned())),
        Err(e) => Err(Error::Io {
            path: path.to_owned(),
            source: e.into(),
        }),
    }
}

pub fn load(path: &Path) -> Result<Store, Error> {
    let store: Store = load_json(path)?;
    // The owner rule in `add` only holds if every stored peer already belongs to the owner.
    if store
        .peers
        .iter()
        .any(|p| Some(p.user_id) != store.owner_user_id)
    {
        return Err(Error::ForeignPeer(path.to_owned()));
    }
    Ok(store)
}

pub fn save(path: &Path, store: &Store) -> Result<(), Error> {
    save_json(path, store)
}

/// A missing file is the default value; anything but a 0600 regular file owned by the
/// current user is refused.
pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T, Error> {
    let io = |source| Error::Io {
        path: path.to_owned(),
        source,
    };
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(e) => return Err(io(e)),
    };
    if !meta.is_file()
        || meta.mode() & 0o077 != 0
        || meta.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(Error::Insecure(path.to_owned()));
    }
    let text = std::fs::read(path).map_err(io)?;
    serde_json::from_slice(&text).map_err(|source| Error::Json {
        path: path.to_owned(),
        source,
    })
}

pub fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    write_json(path, value, true)
}

/// For a file whose loss to a power cut costs only display state: replaced atomically but
/// never flushed, since `sync_all` is a full drive cache flush on macOS.
pub fn replace_json<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    write_json(path, value, false)
}

fn write_json<T: Serialize>(path: &Path, value: &T, durable: bool) -> Result<(), Error> {
    let tmp = path.with_extension("json.tmp");
    let io = |source| Error::Io {
        path: tmp.clone(),
        source,
    };
    let json = serde_json::to_vec_pretty(value).map_err(|source| Error::Json {
        path: path.to_owned(),
        source,
    })?;
    match std::fs::remove_file(&tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io(e)),
        _ => {}
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(io)?;
    file.write_all(&json).map_err(io)?;
    file.write_all(b"\n").map_err(io)?;
    if durable {
        file.sync_all().map_err(io)?;
    }
    drop(file);
    std::fs::rename(&tmp, path).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    if durable && let Some(dir) = path.parent() {
        std::fs::File::open(dir)
            .and_then(|d| d.sync_all())
            .map_err(|source| Error::Io {
                path: dir.to_owned(),
                source,
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn peer(id: &str, user: i64, label: &str) -> Peer {
        Peer {
            stable_id: id.into(),
            user_id: user,
            login: "me@example.com".into(),
            label: label.into(),
            paired_at: 1,
            tls_key: None,
            terminal_key: None,
        }
    }

    #[test]
    fn owner_rule() {
        let mut s = Store::default();
        s.add(peer("n1", 7, "a")).unwrap();
        assert_eq!(s.owner_user_id, Some(7));
        assert!(matches!(
            s.add(peer("n2", 8, "b")),
            Err(Error::NotOwner { owner: 7, got: 8 })
        ));
        s.add(peer("n2", 7, "b")).unwrap();
        assert!(matches!(
            s.add(peer("n3", 7, "a")),
            Err(Error::LabelTaken(_))
        ));
        s.add(peer("n1", 7, "a2")).unwrap();
        assert_eq!(s.peers.len(), 2);
        assert_eq!(s.get("n1").unwrap().label, "a2");
    }

    #[test]
    fn remove_by_label_or_id() {
        let mut s = Store::default();
        s.add(peer("n1", 7, "a")).unwrap();
        s.add(peer("n2", 7, "b")).unwrap();
        assert_eq!(s.remove("a").unwrap().stable_id, "n1");
        assert_eq!(s.remove("n2").unwrap().label, "b");
        assert!(matches!(s.remove("a"), Err(Error::NotFound(_))));
        s.peers = vec![peer("n1", 7, "x"), peer("x", 7, "y")];
        assert!(matches!(s.remove("x"), Err(Error::Ambiguous(_))));
        assert_eq!(s.owner_user_id, Some(7));
    }

    #[test]
    fn save_is_private_and_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        assert_eq!(load(&path).unwrap(), Store::default());
        let mut s = Store::default();
        s.add(peer("n1", 7, "a")).unwrap();
        std::fs::write(path.with_extension("json.tmp"), "stale").unwrap();
        save(&path, &s).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(!path.with_extension("json.tmp").exists());
        assert_eq!(load(&path).unwrap(), s);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec!["peers.json"]);
    }

    #[test]
    fn load_refuses_open_or_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        save(&path, &Store::default()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(load(&path), Err(Error::Insecure(_))));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&path, r#"{"owner_user_id":null,"peers":[],"x":1}"#).unwrap();
        assert!(matches!(load(&path), Err(Error::Json { .. })));
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(matches!(load(&link), Err(Error::Insecure(_))));
    }

    #[test]
    fn lock_is_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.lock");
        let held = lock(&path).unwrap();
        assert!(matches!(lock(&path), Err(Error::Locked(_))));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        drop(held);
        lock(&path).unwrap();
    }

    #[test]
    fn load_refuses_peer_outside_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        for store in [
            Store {
                owner_user_id: None,
                peers: vec![peer("n1", 7, "a")],
            },
            Store {
                owner_user_id: Some(8),
                peers: vec![peer("n1", 7, "a")],
            },
        ] {
            save(&path, &store).unwrap();
            assert!(matches!(load(&path), Err(Error::ForeignPeer(_))));
        }
    }
}
