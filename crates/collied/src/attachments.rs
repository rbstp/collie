use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use protocol::{AttachmentName, ErrorCode, Sha256Hex, UploadId, limits};
use sha2::{Digest, Sha256};

use crate::drive::Fail;

pub const MAX_PER_SESSION: usize = 2;
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_STORED_BYTES: u64 = 200 * 1024 * 1024;
pub const MAX_STORED_ENTRIES: usize = 1000;
pub const RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
pub const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);
const PART: &str = ".part";
const FALLBACK: &str = "attachment";

fn fail<T>(code: ErrorCode, message: impl Into<String>) -> Result<T, Fail> {
    Err((code, message.into()))
}

fn internal(e: std::io::Error) -> Fail {
    tracing::error!(error = %e, "attachment storage");
    (
        ErrorCode::Internal,
        "could not store the attachment".to_owned(),
    )
}

/// Keeps `[A-Za-z0-9._-]`, turns each run of anything else into `-`, drops leading dots
/// and dashes (no hidden files, nothing that reads as an option) and caps the result at
/// 64 characters, keeping a short extension.
pub fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let (stem, ext) = match out.rsplit_once('.') {
        Some((stem, ext))
            if !ext.is_empty()
                && ext.len() <= 16
                && ext.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            (stem, Some(ext))
        }
        _ => (out.as_str(), None),
    };
    let stem = stem.trim_start_matches(['.', '-']);
    let stem = if stem.is_empty() { FALLBACK } else { stem };
    let max = limits::MAX_ATTACHMENT_NAME_CHARS;
    match ext {
        Some(ext) => {
            let keep = max - 1 - ext.len();
            format!("{}.{ext}", &stem[..stem.len().min(keep)])
        }
        None => stem[..stem.len().min(max)].to_owned(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).expect("system RNG");
    hex(&bytes)
}

/// Creates the root (0700) if missing; refuses a symlink, a foreign owner or any mode
/// other than 0700 on an existing one.
pub fn prepare_root(root: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        root.is_absolute(),
        "attachments dir {} is not absolute",
        root.display()
    );
    if let Some(parent) = root.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(root) {
        Ok(()) => std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(anyhow::Error::new(e).context(format!("create {}", root.display()))),
    }
    let meta = std::fs::symlink_metadata(root)?;
    let mode = meta.mode() & 0o777;
    anyhow::ensure!(
        meta.file_type().is_dir()
            && meta.uid() == rustix::process::geteuid().as_raw()
            && mode == 0o700,
        "{} must be a directory owned by the current user with mode 0700 (is {mode:04o})",
        root.display()
    );
    Ok(())
}

pub fn stored_bytes(root: &Path) -> u64 {
    stored_except(root, &[]).bytes
}

pub struct Committed {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

pub struct Upload {
    dir: PathBuf,
    file: Option<File>,
    name: String,
    size: u64,
    sha256: Sha256Hex,
    hasher: Sha256,
    received: u64,
    done: bool,
}

impl Drop for Upload {
    fn drop(&mut self) {
        if !self.done {
            self.file = None;
            let _ = std::fs::remove_file(self.dir.join(PART));
            let _ = std::fs::remove_dir(&self.dir);
        }
    }
}

impl Upload {
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), Fail> {
        if offset != self.received {
            return fail(
                ErrorCode::InvalidParams,
                format!("chunk out of order: expected offset {}", self.received),
            );
        }
        if self.received + data.len() as u64 > self.size {
            return fail(ErrorCode::TooLarge, "more data than the declared size");
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| (ErrorCode::Internal, "upload already closed".to_owned()))?;
        file.write_all(data).map_err(internal)?;
        self.hasher.update(data);
        self.received += data.len() as u64;
        Ok(())
    }

    /// On a size or checksum mismatch the upload is dropped by the caller, which removes
    /// the partial file.
    pub fn commit(&mut self) -> Result<Committed, Fail> {
        if self.received != self.size {
            return fail(
                ErrorCode::InvalidParams,
                format!(
                    "incomplete upload: {} of {} bytes",
                    self.received, self.size
                ),
            );
        }
        let digest = hex(&std::mem::take(&mut self.hasher).finalize());
        if digest != self.sha256.as_str() {
            return fail(
                ErrorCode::ChecksumMismatch,
                "sha256 does not match the data",
            );
        }
        let file = self
            .file
            .take()
            .ok_or_else(|| (ErrorCode::Internal, "upload already closed".to_owned()))?;
        file.sync_all().map_err(internal)?;
        drop(file);
        let path = self.dir.join(&self.name);
        std::fs::rename(self.dir.join(PART), &path).map_err(internal)?;
        if let Ok(dir) = File::open(&self.dir) {
            let _ = dir.sync_all();
        }
        self.done = true;
        Ok(Committed {
            path,
            name: self.name.clone(),
            size: self.size,
            sha256: digest,
        })
    }
}

struct Entry {
    session: u64,
    peer: String,
    size: u64,
    dir: PathBuf,
    last: Instant,
    upload: Arc<Mutex<Upload>>,
}

/// In-flight uploads, each bound to the session (and peer `StableID`) that began it.
pub struct Attachments {
    root: PathBuf,
    uploads: Mutex<HashMap<UploadId, Entry>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Attachments {
    /// Partial files left by a previous run cannot be resumed and are removed at once.
    pub fn open(root: PathBuf) -> anyhow::Result<Self> {
        prepare_root(&root)?;
        let store = Self {
            root,
            uploads: Mutex::default(),
        };
        store.sweep(SystemTime::now(), true);
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn begin(
        &self,
        session: u64,
        peer: &str,
        name: &AttachmentName,
        size: u64,
        sha256: &Sha256Hex,
    ) -> Result<UploadId, Fail> {
        if !(1..=limits::MAX_ATTACHMENT_BYTES).contains(&size) {
            return fail(ErrorCode::TooLarge, "attachments are limited to 20 MiB");
        }
        let mut uploads = lock(&self.uploads);
        if uploads.values().filter(|e| e.session == session).count() >= MAX_PER_SESSION {
            return fail(
                ErrorCode::RateLimited,
                "at most 2 uploads at a time per connection",
            );
        }
        let reserved: u64 = uploads.values().map(|e| e.size).sum();
        let in_flight: Vec<&Path> = uploads.values().map(|e| e.dir.as_path()).collect();
        let stored = stored_except(&self.root, &in_flight);
        if stored.entries + uploads.len() >= MAX_STORED_ENTRIES {
            return fail(
                ErrorCode::TooLarge,
                "too many attachments on the machine (1000); files are removed after 24 h",
            );
        }
        if stored.bytes + reserved + size > MAX_STORED_BYTES {
            return fail(
                ErrorCode::TooLarge,
                "attachment storage on the machine is full (200 MiB); files are removed after 24 h",
            );
        }
        let dir = self.root.join(random_hex(8));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(internal)?;
        let opened = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .and_then(|()| {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                    .open(dir.join(PART))
            })
            .and_then(|f| {
                f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                Ok(f)
            });
        let file = match opened {
            Ok(f) => f,
            Err(e) => {
                let _ = std::fs::remove_dir(&dir);
                return Err(internal(e));
            }
        };
        let id = UploadId::new(random_hex(16)).expect("32 hex chars");
        let upload = Upload {
            dir: dir.clone(),
            file: Some(file),
            name: sanitize(name.as_str()),
            size,
            sha256: sha256.clone(),
            hasher: Sha256::new(),
            received: 0,
            done: false,
        };
        uploads.insert(
            id.clone(),
            Entry {
                session,
                peer: peer.to_owned(),
                size,
                dir,
                last: Instant::now(),
                upload: Arc::new(Mutex::new(upload)),
            },
        );
        Ok(id)
    }

    fn owned(&self, session: u64, peer: &str, id: &UploadId) -> Option<Arc<Mutex<Upload>>> {
        let mut uploads = lock(&self.uploads);
        let e = uploads
            .get_mut(id)
            .filter(|e| e.session == session && e.peer == peer)?;
        e.last = Instant::now();
        Some(e.upload.clone())
    }

    pub fn chunk(
        &self,
        session: u64,
        peer: &str,
        id: &UploadId,
        offset: u64,
        data: &[u8],
    ) -> Result<(), Fail> {
        let upload = self
            .owned(session, peer, id)
            .ok_or_else(|| (ErrorCode::NotFound, "no such upload".to_owned()))?;
        let res = lock(&upload).write(offset, data);
        if matches!(&res, Err((ErrorCode::Internal, _))) {
            self.take(session, peer, id);
        }
        res
    }

    /// Removes the upload from the session; the caller commits or drops it.
    pub fn take(&self, session: u64, peer: &str, id: &UploadId) -> Option<Arc<Mutex<Upload>>> {
        let mut uploads = lock(&self.uploads);
        if !uploads
            .get(id)
            .is_some_and(|e| e.session == session && e.peer == peer)
        {
            return None;
        }
        uploads.remove(id).map(|e| e.upload)
    }

    pub fn end_session(&self, session: u64) {
        let gone: Vec<Entry> = {
            let mut uploads = lock(&self.uploads);
            let ids: Vec<UploadId> = uploads
                .iter()
                .filter(|(_, e)| e.session == session)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter().filter_map(|id| uploads.remove(id)).collect()
        };
        drop(gone);
    }

    /// Drops uploads that received nothing for `IDLE_TIMEOUT`.
    pub fn reap(&self, now: Instant) {
        let gone: Vec<Entry> = {
            let mut uploads = lock(&self.uploads);
            let ids: Vec<UploadId> = uploads
                .iter()
                .filter(|(_, e)| now.duration_since(e.last) >= IDLE_TIMEOUT)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter().filter_map(|id| uploads.remove(id)).collect()
        };
        drop(gone);
    }

    /// Removes entries of the root older than `RETENTION` (and, with `partials`, any dir
    /// still holding a partial file), skipping uploads in flight. Never follows symlinks:
    /// a symlink is removed as a link.
    pub fn sweep(&self, now: SystemTime, partials: bool) {
        let in_flight: Vec<PathBuf> = lock(&self.uploads)
            .values()
            .map(|e| e.dir.clone())
            .collect();
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if in_flight.contains(&path) {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let old = meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age >= RETENTION);
            let is_dir = meta.file_type().is_dir();
            let partial = partials && is_dir && path.join(PART).symlink_metadata().is_ok();
            if !old && !partial {
                continue;
            }
            let res = if is_dir {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if let Err(e) = res {
                tracing::warn!(error = %e, path = %path.display(), "attachment cleanup");
            }
        }
    }
}

struct Stored {
    entries: usize,
    bytes: u64,
}

/// Every entry of the root, and the regular files one level below it, symlinks not
/// followed. A file counts for the larger of its length and its allocated blocks, so tiny
/// files still pay for the disk they use.
fn stored_except(root: &Path, skip: &[&Path]) -> Stored {
    let mut stored = Stored {
        entries: 0,
        bytes: 0,
    };
    let Ok(dirs) = std::fs::read_dir(root) else {
        return stored;
    };
    for d in dirs.flatten() {
        let path = d.path();
        if skip.contains(&path.as_path()) {
            continue;
        }
        stored.entries += 1;
        if !d.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        stored.bytes += files
            .flatten()
            .filter_map(|f| f.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len().max(m.blocks() * 512))
            .sum::<u64>();
    }
    stored
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: &str = "nPhone";

    fn sha(data: &[u8]) -> Sha256Hex {
        Sha256Hex::new(hex(&Sha256::digest(data))).unwrap()
    }

    fn name(s: &str) -> AttachmentName {
        AttachmentName::new(s).unwrap()
    }

    fn store() -> (tempfile::TempDir, Attachments) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Attachments::open(tmp.path().join("cache/attachments")).unwrap();
        (tmp, store)
    }

    fn mode(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).unwrap().mode() & 0o777
    }

    fn entries(root: &Path) -> usize {
        std::fs::read_dir(root).unwrap().count()
    }

    #[test]
    fn sanitizes_names() {
        for (raw, want) in [
            ("photo-20261003-101500.jpg", "photo-20261003-101500.jpg"),
            ("Rapport final (v2).pdf", "Rapport-final-v2-.pdf"),
            ("..hidden", "attachment.hidden"),
            (".env", "attachment.env"),
            ("..", "attachment"),
            ("--rf", "rf"),
            ("日本.png", "attachment.png"),
            ("日本", "attachment"),
            ("a b  c", "a-b-c"),
            ("archive.tar.gz", "archive.tar.gz"),
            ("noext.", "noext."),
        ] {
            assert_eq!(sanitize(raw), want, "{raw:?}");
        }
        let long = format!("{}.jpeg", "x".repeat(70));
        let s = sanitize(&long);
        assert_eq!(s.len(), 64);
        assert!(s.ends_with("x.jpeg"));
        assert_eq!(sanitize(&"y".repeat(80)).len(), 64);
        assert_eq!(sanitize(&format!("a.{}", "b".repeat(20))).len(), 22);
    }

    #[test]
    fn root_must_be_private() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("a/b/attachments");
        prepare_root(&root).unwrap();
        assert_eq!(mode(&root), 0o700);
        assert!(prepare_root(Path::new("relative")).is_err());
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            prepare_root(&root).is_err(),
            "existing root with a wider mode"
        );
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        assert!(prepare_root(&link).is_err(), "symlinked root");
    }

    #[test]
    fn upload_commits_private_file() {
        let (_tmp, store) = store();
        let data: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
        let id = store
            .begin(
                1,
                PEER,
                &name("Screen Shot.png"),
                data.len() as u64,
                &sha(&data),
            )
            .unwrap();
        let mut offset = 0;
        for chunk in data.chunks(limits::MAX_ATTACHMENT_CHUNK_BYTES) {
            store.chunk(1, PEER, &id, offset, chunk).unwrap();
            offset += chunk.len() as u64;
        }
        let upload = store.take(1, PEER, &id).unwrap();
        let done = lock(&upload).commit().unwrap();
        drop(upload);
        assert_eq!(done.name, "Screen-Shot.png");
        assert_eq!(done.path.file_name().unwrap(), "Screen-Shot.png");
        assert_eq!(std::fs::read(&done.path).unwrap(), data);
        assert_eq!(mode(&done.path), 0o600);
        assert_eq!(mode(done.path.parent().unwrap()), 0o700);
        assert_eq!(done.path.parent().unwrap().parent().unwrap(), store.root());
        assert!(!done.path.with_file_name(PART).exists());
        assert!(stored_bytes(store.root()) >= data.len() as u64);
    }

    #[test]
    fn checksum_or_size_mismatch_leaves_nothing() {
        let (_tmp, store) = store();
        let id = store
            .begin(1, PEER, &name("a.txt"), 3, &sha(b"abd"))
            .unwrap();
        store.chunk(1, PEER, &id, 0, b"abc").unwrap();
        let upload = store.take(1, PEER, &id).unwrap();
        let err = lock(&upload).commit().err().unwrap();
        assert_eq!(err.0, ErrorCode::ChecksumMismatch);
        drop(upload);
        assert_eq!(entries(store.root()), 0);

        let id = store
            .begin(1, PEER, &name("a.txt"), 4, &sha(b"abcd"))
            .unwrap();
        store.chunk(1, PEER, &id, 0, b"abc").unwrap();
        let upload = store.take(1, PEER, &id).unwrap();
        assert_eq!(
            lock(&upload).commit().err().unwrap().0,
            ErrorCode::InvalidParams
        );
        drop(upload);
        assert_eq!(entries(store.root()), 0);
    }

    #[test]
    fn chunks_in_order_and_within_size() {
        let (_tmp, store) = store();
        let id = store.begin(1, PEER, &name("a"), 4, &sha(b"abcd")).unwrap();
        let err = store.chunk(1, PEER, &id, 1, b"a").unwrap_err();
        assert_eq!(err.0, ErrorCode::InvalidParams);
        store.chunk(1, PEER, &id, 0, b"ab").unwrap();
        assert_eq!(
            store.chunk(1, PEER, &id, 0, b"ab").unwrap_err().0,
            ErrorCode::InvalidParams
        );
        assert_eq!(
            store.chunk(1, PEER, &id, 2, b"cde").unwrap_err().0,
            ErrorCode::TooLarge
        );
        store.chunk(1, PEER, &id, 2, b"cd").unwrap();
    }

    #[test]
    fn uploads_are_bound_to_their_session_and_peer() {
        let (_tmp, store) = store();
        let id = store.begin(1, PEER, &name("a"), 1, &sha(b"x")).unwrap();
        for (session, peer) in [(2, PEER), (1, "nOther")] {
            assert_eq!(
                store.chunk(session, peer, &id, 0, b"x").unwrap_err().0,
                ErrorCode::NotFound
            );
            assert!(store.take(session, peer, &id).is_none());
        }
        store.end_session(2);
        store.chunk(1, PEER, &id, 0, b"x").unwrap();
        store.end_session(1);
        assert_eq!(
            store.chunk(1, PEER, &id, 1, b"x").unwrap_err().0,
            ErrorCode::NotFound
        );
        assert_eq!(entries(store.root()), 0, "session close drops its uploads");
    }

    #[test]
    fn limits_per_session_and_total() {
        let (_tmp, store) = store();
        let max = limits::MAX_ATTACHMENT_BYTES;
        let digest = sha(b"");
        assert_eq!(
            store
                .begin(1, PEER, &name("a"), max + 1, &digest)
                .unwrap_err()
                .0,
            ErrorCode::TooLarge
        );
        store.begin(1, PEER, &name("a"), max, &digest).unwrap();
        store.begin(1, PEER, &name("b"), max, &digest).unwrap();
        assert_eq!(
            store.begin(1, PEER, &name("c"), 1, &digest).unwrap_err().0,
            ErrorCode::RateLimited
        );
        for session in 2..=5 {
            store
                .begin(session, PEER, &name("d"), max, &digest)
                .unwrap();
            store
                .begin(session, PEER, &name("e"), max, &digest)
                .unwrap();
        }
        let err = store.begin(6, PEER, &name("f"), 1, &digest).unwrap_err();
        assert_eq!(
            err.0,
            ErrorCode::TooLarge,
            "200 MiB reserved by uploads in flight"
        );
        store.end_session(5);
        let big = store.root().join("0123456789abcdef");
        std::fs::create_dir(&big).unwrap();
        File::create(big.join("old.bin"))
            .unwrap()
            .set_len(2 * max)
            .unwrap();
        assert_eq!(
            store.begin(6, PEER, &name("f"), 1, &digest).unwrap_err().0,
            ErrorCode::TooLarge,
            "committed files count too"
        );
        std::fs::remove_dir_all(&big).unwrap();
        store.begin(6, PEER, &name("f"), 1, &digest).unwrap();
    }

    #[test]
    fn caps_stored_entries_and_counts_allocated_blocks() {
        let (_tmp, store) = store();
        let digest = sha(b"x");
        store.begin(1, PEER, &name("a"), 1, &digest).unwrap();
        for i in 0..MAX_STORED_ENTRIES - 2 {
            let dir = store.root().join(format!("{i:016x}"));
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("f"), b"x").unwrap();
        }
        assert!(stored_bytes(store.root()) >= 512 * (MAX_STORED_ENTRIES as u64 - 2));
        store.begin(2, PEER, &name("b"), 1, &digest).unwrap();
        assert_eq!(
            store.begin(3, PEER, &name("c"), 1, &digest).unwrap_err().0,
            ErrorCode::TooLarge,
            "1000 entries including uploads in flight"
        );
        store.end_session(2);
        store.begin(3, PEER, &name("c"), 1, &digest).unwrap();
    }

    #[test]
    fn idle_uploads_are_reaped() {
        let (_tmp, store) = store();
        let id = store.begin(1, PEER, &name("a"), 2, &sha(b"ab")).unwrap();
        store.reap(Instant::now());
        store.chunk(1, PEER, &id, 0, b"a").unwrap();
        store.reap(Instant::now() + IDLE_TIMEOUT);
        assert_eq!(
            store.chunk(1, PEER, &id, 1, b"b").unwrap_err().0,
            ErrorCode::NotFound
        );
        assert_eq!(entries(store.root()), 0);
    }

    #[test]
    fn sweep_removes_old_entries_and_stale_partials() {
        let (tmp, store) = store();
        let id = store.begin(1, PEER, &name("a"), 1, &sha(b"x")).unwrap();
        store.chunk(1, PEER, &id, 0, b"x").unwrap();
        let kept = lock(&store.take(1, PEER, &id).unwrap()).commit().unwrap();
        let live = store.begin(2, PEER, &name("b"), 1, &sha(b"y")).unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep"), b"k").unwrap();
        std::os::unix::fs::symlink(&outside, store.root().join("link")).unwrap();
        let stale = store.root().join("fedcba9876543210");
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(stale.join(PART), b"half").unwrap();

        store.sweep(SystemTime::now(), false);
        assert_eq!(entries(store.root()), 4, "nothing is old yet");
        store.sweep(SystemTime::now(), true);
        assert!(!stale.exists(), "partial from a previous run");
        assert!(kept.path.exists());
        store.sweep(SystemTime::now() + RETENTION, false);
        assert!(!kept.path.exists());
        assert!(!store.root().join("link").exists());
        assert!(
            outside.join("keep").exists(),
            "the symlink target is untouched"
        );
        store.chunk(2, PEER, &live, 0, b"y").unwrap();
        assert_eq!(entries(store.root()), 1, "the upload in flight stays");
    }
}
