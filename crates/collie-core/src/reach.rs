use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::session::lock;
use crate::store::write_atomic;

pub const FILE: &str = "reachability.json";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    pub last_ok_ms: Option<u64>,
    pub last_fail_ms: Option<u64>,
}

/// Last connection outcome per Mac node ID, mirrored to `reachability.json` in the App
/// Group container for the Notification Service Extension. Holds no secrets: node IDs
/// and timestamps only.
#[derive(Default)]
pub struct Reachability(Mutex<State>);

#[derive(Default)]
struct State {
    dir: Option<PathBuf>,
    seen: BTreeMap<String, Seen>,
}

impl Reachability {
    pub fn set_dir(&self, dir: PathBuf) -> std::io::Result<()> {
        let mut state = lock(&self.0);
        let on_disk: BTreeMap<String, Seen> = std::fs::read(dir.join(FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        for (node_id, disk) in on_disk {
            let seen = state.seen.entry(node_id).or_default();
            seen.last_ok_ms = seen.last_ok_ms.max(disk.last_ok_ms);
            seen.last_fail_ms = seen.last_fail_ms.max(disk.last_fail_ms);
        }
        state.dir = Some(dir);
        state.write()
    }

    /// Best effort: the file is only a hint, so a failed write is not an error.
    pub fn record(&self, node_id: &str, ok: bool) {
        let now = Some(now_ms());
        let mut state = lock(&self.0);
        let seen = state.seen.entry(node_id.to_owned()).or_default();
        if ok {
            seen.last_ok_ms = now;
        } else {
            seen.last_fail_ms = now;
        }
        let _ = state.write();
    }
}

impl State {
    fn write(&self) -> std::io::Result<()> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(&self.seen).map_err(std::io::Error::other)?;
        write_atomic(dir, FILE, &bytes)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(dir: &std::path::Path) -> BTreeMap<String, Seen> {
        serde_json::from_slice(&std::fs::read(dir.join(FILE)).unwrap()).unwrap()
    }

    #[test]
    fn records_are_written_once_a_dir_is_set_and_merged_with_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(FILE),
            r#"{"nOLD":{"last_ok_ms":5,"last_fail_ms":null},"nMAC":{"last_ok_ms":1,"last_fail_ms":9999999999999}}"#,
        )
        .unwrap();
        let reach = Reachability::default();
        reach.record("nMAC", true);
        assert_eq!(
            read(dir.path())["nMAC"].last_ok_ms,
            Some(1),
            "nothing is written before a dir is set"
        );
        reach.set_dir(dir.path().to_owned()).unwrap();
        let seen = read(dir.path());
        assert_eq!(seen["nOLD"].last_ok_ms, Some(5));
        assert!(seen["nMAC"].last_ok_ms.unwrap() > 1);
        assert_eq!(seen["nMAC"].last_fail_ms, Some(9999999999999));

        reach.record("nOTHER", false);
        let seen = read(dir.path());
        assert!(seen["nOTHER"].last_fail_ms.is_some());
        assert_eq!(seen["nOTHER"].last_ok_ms, None);
        let raw = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(
            raw.contains(r#""nOTHER":{"last_ok_ms":null,"last_fail_ms":"#),
            "{raw}"
        );
        assert!(!dir.path().join(format!(".{FILE}.tmp")).exists());
    }

    #[test]
    fn missing_dir_is_not_fatal_for_records() {
        let reach = Reachability::default();
        reach
            .set_dir(PathBuf::from("/nonexistent/collie"))
            .unwrap_err();
        reach.record("nMAC", false);
    }
}
