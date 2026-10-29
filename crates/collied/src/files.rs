use std::collections::HashMap;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use protocol::{Cwd, DownloadId, ErrorCode, FileKind, Sha256Hex, hex, limits};
use rustix::fs::{FileType, Mode, OFlags};
use sha2::{Digest, Sha256};

use crate::drive::{Fail, fail};
use crate::lock;

pub const MAX_PER_SESSION: usize = 2;
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const SNIFF_BYTES: usize = 8192;
const CHANGED: &str = "The file changed while it was read. Try again.";

fn refused<T>(message: &str) -> Result<T, Fail> {
    fail(ErrorCode::InvalidParams, message)
}

pub struct Opened {
    pub file: File,
    pub name: String,
    pub size: u64,
    pub kind: FileKind,
    pub sha256: Sha256Hex,
}

/// `path` is absolute or relative to `cwd` (a canonical checkout inside the roots). It is
/// resolved without `..` and opened one component at a time from its root, so no symlink
/// is followed and no hidden file or folder is opened.
pub fn open(cwd: &Cwd, path: &str, roots: &[PathBuf]) -> Result<Opened, Fail> {
    let mut abs = PathBuf::new();
    for part in Path::new(cwd.as_str()).join(path).components() {
        match part {
            Component::RootDir => abs.push("/"),
            Component::CurDir => {}
            Component::Normal(p) => abs.push(p),
            _ => return refused("Paths with .. are not opened."),
        }
    }
    let Some(root) = roots.iter().find(|r| abs.starts_with(r)) else {
        return refused("This file is outside the task folders.");
    };
    let (Some(parent), Some(name)) = (abs.parent(), abs.file_name()) else {
        return refused("Folders are not opened.");
    };
    if !parent.starts_with(root) {
        return refused("Folders are not opened.");
    }
    if name.as_encoded_bytes().starts_with(b".") {
        return refused("Hidden files are not opened.");
    }
    let dir = crate::drive::open_beneath(root, parent)?;
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let fd = rustix::fs::openat(&dir, name, flags, Mode::empty()).map_err(|e| match e {
        rustix::io::Errno::LOOP => (
            ErrorCode::InvalidParams,
            "Symlinks are not opened.".to_owned(),
        ),
        rustix::io::Errno::NOENT => (ErrorCode::InvalidParams, "No such file.".to_owned()),
        e => (
            ErrorCode::InvalidParams,
            format!("The file cannot be opened: {}", std::io::Error::from(e)),
        ),
    })?;
    let st = rustix::fs::fstat(&fd).map_err(|e| {
        (
            ErrorCode::InvalidParams,
            format!("The file cannot be opened: {}", std::io::Error::from(e)),
        )
    })?;
    #[cfg(target_os = "macos")]
    if st.st_flags & crate::drive::UF_HIDDEN != 0 {
        return refused("Hidden files are not opened.");
    }
    match FileType::from_raw_mode(st.st_mode) {
        FileType::RegularFile => {}
        FileType::Directory => return refused("Folders are not opened."),
        _ => return refused("Only regular files are opened."),
    }
    let file = File::from(fd);
    let name = name.to_string_lossy().into_owned();
    let size = st.st_size as u64;
    let kind = kind(&file, &name)?;
    let (cap, what) = match kind {
        FileKind::Text => (limits::MAX_TEXT_FILE_BYTES, "text files"),
        _ => (limits::MAX_FILE_BYTES, "files"),
    };
    if size > cap {
        return fail(
            ErrorCode::TooLarge,
            format!(
                "{name} is {:.1} MB; {what} open up to {} MB.",
                size as f64 / 1_048_576.0,
                cap >> 20
            ),
        );
    }
    let sha256 = digest(&file, size)?;
    Ok(Opened {
        file,
        name,
        size,
        kind,
        sha256,
    })
}

/// Images and PDF by extension; anything else is text when its first 8 KiB are UTF-8
/// without a NUL.
fn kind(file: &File, name: &str) -> Result<FileKind, Fail> {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("png" | "jpg" | "jpeg" | "gif" | "heic" | "webp") => return Ok(FileKind::Image),
        Some("pdf") => return Ok(FileKind::Pdf),
        _ => {}
    }
    let mut head = vec![0; SNIFF_BYTES];
    let n = file
        .read_at(&mut head, 0)
        .map_err(|_| (ErrorCode::Internal, "could not read the file".to_owned()))?;
    let head = &head[..n];
    let utf8 = match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    };
    Ok(if utf8 && !head.contains(&0) {
        FileKind::Text
    } else {
        FileKind::Other
    })
}

fn digest(file: &File, size: u64) -> Result<Sha256Hex, Fail> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 256 * 1024];
    let mut offset = 0;
    while offset < size {
        let want = buf.len().min((size - offset) as usize);
        file.read_exact_at(&mut buf[..want], offset)
            .map_err(|_| (ErrorCode::InvalidParams, CHANGED.to_owned()))?;
        hasher.update(&buf[..want]);
        offset += want as u64;
    }
    Ok(Sha256Hex::new(hex(&hasher.finalize())).expect("64 lowercase hex chars"))
}

fn random_id() -> DownloadId {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("system RNG");
    DownloadId::new(hex(&bytes)).expect("32 hex chars")
}

struct Entry {
    session: u64,
    peer: String,
    file: Arc<File>,
    size: u64,
    last: Instant,
}

/// Open files, each bound to the session (and peer `StableID`) that opened it. Nothing is
/// copied or cached: chunks are read from the open file.
#[derive(Default)]
pub struct Downloads {
    open: Mutex<HashMap<DownloadId, Entry>>,
}

impl Downloads {
    pub fn begin(
        &self,
        session: u64,
        peer: &str,
        file: File,
        size: u64,
    ) -> Result<DownloadId, Fail> {
        let mut open = lock(&self.open);
        if open.values().filter(|e| e.session == session).count() >= MAX_PER_SESSION {
            return fail(
                ErrorCode::RateLimited,
                "at most 2 files open at a time per connection",
            );
        }
        let id = random_id();
        open.insert(
            id.clone(),
            Entry {
                session,
                peer: peer.to_owned(),
                file: Arc::new(file),
                size,
                last: Instant::now(),
            },
        );
        Ok(id)
    }

    pub fn chunk(
        &self,
        session: u64,
        peer: &str,
        id: &DownloadId,
        offset: u64,
    ) -> Result<Vec<u8>, Fail> {
        let (file, size) = {
            let mut open = lock(&self.open);
            let e = open
                .get_mut(id)
                .filter(|e| e.session == session && e.peer == peer)
                .ok_or_else(|| (ErrorCode::NotFound, "no such file transfer".to_owned()))?;
            e.last = Instant::now();
            (e.file.clone(), e.size)
        };
        if offset >= size {
            return fail(
                ErrorCode::InvalidParams,
                "offset is past the end of the file",
            );
        }
        let len = (limits::MAX_ATTACHMENT_CHUNK_BYTES as u64).min(size - offset) as usize;
        let mut data = vec![0; len];
        file.read_exact_at(&mut data, offset)
            .map_err(|_| (ErrorCode::InvalidParams, CHANGED.to_owned()))?;
        Ok(data)
    }

    pub fn close(&self, session: u64, peer: &str, id: &DownloadId) {
        let mut open = lock(&self.open);
        if open
            .get(id)
            .is_some_and(|e| e.session == session && e.peer == peer)
        {
            open.remove(id);
        }
    }

    pub fn end_session(&self, session: u64) {
        lock(&self.open).retain(|_, e| e.session != session);
    }

    /// Closes files that served nothing for `IDLE_TIMEOUT`.
    pub fn reap(&self, now: Instant) {
        lock(&self.open).retain(|_, e| now.duration_since(e.last) < IDLE_TIMEOUT);
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    struct Tree {
        _dir: tempfile::TempDir,
        root: PathBuf,
        cwd: Cwd,
    }

    fn tree() -> Tree {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap().join("root");
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(repo.join("shot.png"), [0x89, b'P', b'N', b'G', 0, 1]).unwrap();
        std::fs::write(repo.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
        std::fs::write(repo.join(".env"), "SECRET=1").unwrap();
        std::fs::write(repo.join(".git/config"), "[core]").unwrap();
        std::fs::write(dir.path().join("outside.txt"), "outside").unwrap();
        symlink(repo.join("src/main.rs"), repo.join("link.rs")).unwrap();
        symlink(repo.join("src"), repo.join("linked")).unwrap();
        let cwd = Cwd::new(repo.to_str().unwrap()).unwrap();
        Tree {
            _dir: dir,
            root,
            cwd,
        }
    }

    fn open_in(t: &Tree, path: &str) -> Result<Opened, Fail> {
        open(&t.cwd, path, std::slice::from_ref(&t.root))
    }

    fn message(r: Result<Opened, Fail>) -> String {
        match r {
            Ok(o) => panic!("{} opened", o.name),
            Err((_, m)) => m,
        }
    }

    #[test]
    fn opens_relative_and_absolute_paths_inside_the_roots() {
        let t = tree();
        let abs = format!("{}/src/main.rs", t.cwd.as_str());
        for path in ["src/main.rs", "./src/main.rs", abs.as_str(), "src//main.rs"] {
            let o = open_in(&t, path).unwrap();
            assert_eq!(o.name, "main.rs");
            assert_eq!(o.size, 13);
            assert_eq!(o.kind, FileKind::Text);
            assert_eq!(
                o.sha256.as_str(),
                hex(&Sha256::digest(b"fn main() {}\n")),
                "{path}"
            );
        }
        assert_eq!(open_in(&t, "shot.png").unwrap().kind, FileKind::Image);
        assert_eq!(open_in(&t, "blob.bin").unwrap().kind, FileKind::Other);
    }

    #[test]
    fn refuses_hidden_files_symlinks_folders_and_paths_outside() {
        let t = tree();
        let outside = format!("{}/outside.txt", t.root.parent().unwrap().display());
        for (path, expected) in [
            (".env", "Hidden files are not opened."),
            (".git/config", "folder is hidden"),
            ("link.rs", "Symlinks are not opened."),
            ("src", "Folders are not opened."),
            ("missing.txt", "No such file."),
            ("../outside.txt", "Paths with .. are not opened."),
            (outside.as_str(), "This file is outside the task folders."),
            ("/etc/hosts", "This file is outside the task folders."),
        ] {
            assert_eq!(message(open_in(&t, path)), expected, "{path}");
        }
        assert!(message(open_in(&t, "linked/main.rs")).starts_with("folder cannot be opened"));
        let root = t.root.to_str().unwrap();
        assert_eq!(message(open_in(&t, root)), "Folders are not opened.");
    }

    #[test]
    fn refuses_files_over_the_cap_before_reading_them() {
        let t = tree();
        let text = Path::new(t.cwd.as_str()).join("big.log");
        std::fs::write(&text, "a".repeat(limits::MAX_TEXT_FILE_BYTES as usize + 1)).unwrap();
        let err = open_in(&t, "big.log").err().unwrap();
        assert_eq!(err.0, ErrorCode::TooLarge);
        assert!(err.1.contains("1.0 MB"), "{}", err.1);
        let image = Path::new(t.cwd.as_str()).join("big.png");
        File::create(&image)
            .unwrap()
            .set_len(limits::MAX_FILE_BYTES)
            .unwrap();
        assert_eq!(open_in(&t, "big.png").unwrap().kind, FileKind::Image);
        File::create(&image)
            .unwrap()
            .set_len(limits::MAX_FILE_BYTES + 1)
            .unwrap();
        assert_eq!(open_in(&t, "big.png").err().unwrap().0, ErrorCode::TooLarge);
    }

    #[test]
    fn chunks_cover_the_file_for_its_session_only() {
        let t = tree();
        let path = Path::new(t.cwd.as_str()).join("data.bin");
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let opened = open_in(&t, "data.bin").unwrap();
        assert_eq!(opened.sha256.as_str(), hex(&Sha256::digest(&data)));
        let downloads = Downloads::default();
        let id = downloads
            .begin(1, "peer", opened.file, opened.size)
            .unwrap();
        assert_eq!(
            downloads.chunk(2, "peer", &id, 0).unwrap_err().0,
            ErrorCode::NotFound
        );
        assert_eq!(
            downloads.chunk(1, "other", &id, 0).unwrap_err().0,
            ErrorCode::NotFound
        );
        let mut received = Vec::new();
        while (received.len() as u64) < opened.size {
            let chunk = downloads
                .chunk(1, "peer", &id, received.len() as u64)
                .unwrap();
            assert!(chunk.len() <= limits::MAX_ATTACHMENT_CHUNK_BYTES);
            received.extend(chunk);
        }
        assert_eq!(received, data);
        assert_eq!(
            downloads.chunk(1, "peer", &id, opened.size).unwrap_err().0,
            ErrorCode::InvalidParams
        );
        downloads.close(2, "peer", &id);
        assert!(downloads.chunk(1, "peer", &id, 0).is_ok());
        std::fs::File::create(&path).unwrap().set_len(10).unwrap();
        assert_eq!(downloads.chunk(1, "peer", &id, 0).unwrap_err().1, CHANGED);
        downloads.close(1, "peer", &id);
        assert_eq!(
            downloads.chunk(1, "peer", &id, 0).unwrap_err().0,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn sessions_hold_two_files_until_closed_ended_or_idle() {
        let t = tree();
        let file = || open_in(&t, "src/main.rs").unwrap().file;
        let downloads = Downloads::default();
        let a = downloads.begin(1, "peer", file(), 13).unwrap();
        downloads.begin(1, "peer", file(), 13).unwrap();
        assert_eq!(
            downloads.begin(1, "peer", file(), 13).unwrap_err().0,
            ErrorCode::RateLimited
        );
        let other = downloads.begin(2, "peer", file(), 13).unwrap();
        downloads.end_session(1);
        assert_eq!(
            downloads.chunk(1, "peer", &a, 0).unwrap_err().0,
            ErrorCode::NotFound
        );
        downloads.reap(Instant::now() + IDLE_TIMEOUT);
        assert_eq!(
            downloads.chunk(2, "peer", &other, 0).unwrap_err().0,
            ErrorCode::NotFound
        );
    }
}
