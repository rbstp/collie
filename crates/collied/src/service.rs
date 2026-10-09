use std::path::Path;

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
