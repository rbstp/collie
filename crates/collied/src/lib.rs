#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("collied runs on macOS and Linux only");

pub mod activity;
pub mod approvals;
pub mod attachments;
pub mod audit;
pub mod config;
pub mod control;
#[cfg(target_os = "linux")]
pub mod creds;
pub mod daemon;
pub mod doctor;
pub mod draft;
pub mod drive;
pub mod flock;
pub mod gate;
pub mod herdr;
pub mod hooks;
#[cfg(target_os = "macos")]
pub mod keychain;
pub mod pairing;
pub mod peers;
pub mod prompt;
pub mod push;
pub mod reflow;
pub mod server;
pub mod service;
pub mod setup;
pub mod terminal;
pub mod transcript;
pub mod usage;

use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::Path;

#[cfg(target_os = "linux")]
use anyhow::Context;

/// A systemd tool by absolute path (never PATH): `/usr/bin`, then `/bin` (Debian
/// without merged /usr), then NixOS's system profile.
#[cfg(target_os = "linux")]
pub(crate) fn system_bin(name: &str) -> std::path::PathBuf {
    let candidates =
        ["/usr/bin", "/bin", "/run/current-system/sw/bin"].map(|d| Path::new(d).join(name));
    candidates
        .iter()
        .find(|p| p.is_file())
        .unwrap_or(&candidates[0])
        .clone()
}

pub(crate) fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

// Refuses a symlink, a foreign owner or any group/world bit.
pub fn ensure_private_dir(dir: &Path) -> anyhow::Result<()> {
    // A fresh account may have no XDG base directory yet; the spec creates it 0700.
    #[cfg(target_os = "linux")]
    if let Some(parent) = dir.parent().filter(|p| !p.exists()) {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => {
            return Err(anyhow::Error::new(e).context(format!("create {}", dir.display())));
        }
        _ => {}
    }
    let meta = std::fs::symlink_metadata(dir)?;
    let mode = meta.permissions().mode() & 0o777;
    anyhow::ensure!(
        meta.is_dir() && meta.uid() == rustix::process::geteuid().as_raw() && mode & 0o077 == 0,
        "{} must be a directory owned by the current user with mode 0700 (is {mode:04o})",
        dir.display()
    );
    Ok(())
}

/// For the Mac pairing prompt: format characters (bidi overrides, zero-width) and other
/// non-printing characters are shown escaped so a phone-supplied label cannot disguise
/// itself.
pub fn printable(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let escaped = c.escape_debug();
        if escaped.len() == 1 || matches!(c, '\'' | '"' | '\\') {
            out.push(c);
        } else {
            out.extend(escaped);
        }
    }
    out
}

pub fn qr_text(data: &str) -> anyhow::Result<String> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::new(data.as_bytes())?;
    Ok(code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_dir() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("collie");
        ensure_private_dir(&dir).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o750)).unwrap();
        assert!(ensure_private_dir(&dir).is_err());
        let link = root.path().join("link");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(ensure_private_dir(&link).is_err());
    }

    #[test]
    fn printable_escapes_format_characters() {
        assert_eq!(
            printable("Rich\u{2019}s \"iPhone\" \\ 15"),
            "Rich\u{2019}s \"iPhone\" \\ 15"
        );
        assert_eq!(printable("ab\u{202e}cd"), "ab\\u{202e}cd");
        assert_eq!(printable("a\u{200b}b\u{2066}"), "a\\u{200b}b\\u{2066}");
        assert_eq!(printable("a\nb"), "a\\nb");
    }

    #[test]
    fn qr_renders() {
        let qr = qr_text("collie://pair#v=1").unwrap();
        assert!(qr.lines().count() > 10);
    }
}
