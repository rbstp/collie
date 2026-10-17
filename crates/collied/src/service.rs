use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
mod launchd;
#[cfg(target_os = "macos")]
pub use launchd::{install, log_tail, note, start, state, stop, uninstall, unload};

#[cfg(target_os = "linux")]
mod systemd;
#[cfg(target_os = "linux")]
pub use systemd::{install, log_tail, note, start, state, stop, uninstall, unload};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Missing,
    Stopped,
    Outdated(&'static str),
    Current,
}

/// The service runs this path: a build output would be replaced by the next cargo build
/// (and on macOS left unsigned), and a symlink could point at one. Linux packages own
/// the regular /usr/bin/collied instead.
pub fn installed_exe() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    #[cfg(target_os = "linux")]
    {
        let packaged = Path::new("/usr/bin/collied");
        if matches_installed(packaged, &exe) {
            return Ok(exe);
        }
    }
    let installed = crate::config::home_dir()?.join(".cargo/bin/collied");
    anyhow::ensure!(
        matches_installed(&installed, &exe),
        "{} is not {}",
        exe.display(),
        installed.display()
    );
    Ok(exe)
}

fn matches_installed(installed: &Path, exe: &Path) -> bool {
    installed.symlink_metadata().is_ok_and(|m| m.is_file())
        && installed.canonicalize().ok().as_deref() == Some(exe)
}

fn built_ms(exe: &Path) -> anyhow::Result<u64> {
    Ok(std::fs::metadata(exe)?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64))
}

fn decide(
    on_disk: Option<&str>,
    want: &str,
    loaded: bool,
    started_ms: Option<u64>,
    built_ms: u64,
) -> State {
    match (on_disk, started_ms) {
        (None, _) => State::Missing,
        _ if !loaded => State::Stopped,
        (Some(p), _) if p != want => State::Outdated("the service runs another binary or config"),
        (_, None) => State::Outdated("the daemon is not answering or is older than this collied"),
        (_, Some(started)) if started < built_ms => {
            State::Outdated("a newer collied binary is installed")
        }
        _ => State::Current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_executable_must_be_the_same_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("collied");
        let other = dir.path().join("other");
        let link = dir.path().join("link");
        std::fs::write(&installed, b"installed").unwrap();
        std::fs::write(&other, b"other").unwrap();
        std::os::unix::fs::symlink(&installed, &link).unwrap();
        let exe = installed.canonicalize().unwrap();
        assert!(matches_installed(&installed, &exe));
        assert!(!matches_installed(&other, &exe));
        assert!(!matches_installed(&link, &exe));
        assert!(!matches_installed(&dir.path().join("missing"), &exe));
    }

    #[test]
    fn state_follows_service_job_and_binary() {
        let want = "service";
        assert_eq!(decide(None, want, true, Some(9), 5), State::Missing);
        assert!(matches!(
            decide(Some("other"), want, true, Some(9), 5),
            State::Outdated(_)
        ));
        assert_eq!(decide(Some("other"), want, false, None, 5), State::Stopped);
        assert_eq!(decide(Some(want), want, false, None, 5), State::Stopped);
        assert_eq!(
            decide(Some(want), want, true, None, 5),
            State::Outdated("the daemon is not answering or is older than this collied")
        );
        assert_eq!(
            decide(Some(want), want, true, Some(4), 5),
            State::Outdated("a newer collied binary is installed")
        );
        assert_eq!(decide(Some(want), want, true, Some(5), 5), State::Current);
    }
}
