use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;

pub struct Audit {
    file: Mutex<File>,
}

#[derive(Serialize)]
struct Entry<'a> {
    ts: u64,
    peer: &'a str,
    method: &'a str,
    target: Option<&'a str>,
    result: &'a str,
}

impl Audit {
    /// Append-only and 0600: an existing file with a wider mode is narrowed, one owned by
    /// someone else is refused.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.uid() != rustix::process::geteuid().as_raw() {
            return Err(std::io::Error::other(format!(
                "{}: not a regular file owned by the current user",
                path.display()
            )));
        }
        if meta.mode() & 0o777 != 0o600 {
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    pub fn log(&self, peer: &str, method: &str, target: Option<&str>, result: &str) {
        let entry = Entry {
            ts: crate::now_ms(),
            peer,
            method,
            target,
            result,
        };
        let Ok(mut line) = serde_json::to_vec(&entry) else {
            return;
        };
        line.push(b'\n');
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = file.write_all(&line) {
            tracing::error!(error = %e, "audit log write failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_append_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        std::fs::write(&path, "{\"old\":1}\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let audit = Audit::open(&path).unwrap();
        audit.log("100.64.0.2:1234", "connect", None, "rejected: tagged");
        audit.log("phone", "pair.complete", Some("nX"), "paired");
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "{\"old\":1}");
        let v: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(v["method"], "pair.complete");
        assert_eq!(v["target"], "nX");
        assert!(v["ts"].as_u64().unwrap() > 0);

        let fresh = dir.path().join("fresh.log");
        Audit::open(&fresh).unwrap();
        let mode = std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let link = dir.path().join("link.log");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(Audit::open(&link).is_err());
    }
}
