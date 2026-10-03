use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use protocol::PushRegisterParams;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct Machine {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub node_id: String,
}

pub struct MachineStore {
    dir: PathBuf,
}

const FILE: &str = "machines.json";
#[cfg(test)]
const TMP: &str = ".machines.json.tmp";
const CORRUPT: &str = "machines.json.corrupt";
const PUSH_FILE: &str = "push.json";

impl MachineStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// An unreadable list is moved aside rather than failing startup: the machines can
    /// be paired again, while a hard error would leave the app unusable.
    pub fn load(&self) -> std::io::Result<Vec<Machine>> {
        let path = self.dir.join(FILE);
        match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(machines) => Ok(machines),
                Err(_) => {
                    fs::rename(&path, self.dir.join(CORRUPT))?;
                    Ok(Vec::new())
                }
            },
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, machines: &[Machine]) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(machines).map_err(std::io::Error::other)?;
        write_atomic(&self.dir, FILE, &bytes)
    }

    /// The APNs token is kept so every reconnect, including one from a background
    /// launch, re-registers it.
    pub fn load_push(&self) -> Option<PushRegisterParams> {
        serde_json::from_slice(&fs::read(self.dir.join(PUSH_FILE)).ok()?).ok()
    }

    pub fn save_push(&self, push: &PushRegisterParams) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(push).map_err(std::io::Error::other)?;
        write_atomic(&self.dir, PUSH_FILE, &bytes)
    }
}

/// Written to a fresh 0600 file, fsynced, then renamed over the old one, so a crash
/// leaves either the previous content or the new one.
pub fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = dir.join(format!(".{name}.tmp"));
    match fs::remove_file(&tmp) {
        Err(e) if e.kind() != ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, dir.join(name))?;
    File::open(dir)?.sync_all()
}

pub fn random_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("system RNG");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn machine(id: &str) -> Machine {
        Machine {
            id: id.into(),
            label: "mac".into(),
            host: "mac.tail1234.ts.net".into(),
            port: 8457,
            node_id: "nMAC".into(),
        }
    }

    #[test]
    fn saves_0600_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachineStore::new(dir.path().to_owned());
        assert_eq!(store.load().unwrap(), Vec::new());
        store.save(&[machine("a"), machine("b")]).unwrap();
        let mode = fs::metadata(dir.path().join(FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(store.load().unwrap(), vec![machine("a"), machine("b")]);
        store.save(&[machine("b")]).unwrap();
        assert_eq!(store.load().unwrap(), vec![machine("b")]);
        assert!(!dir.path().join(TMP).exists());
    }

    #[test]
    fn stale_temp_file_does_not_leak_its_mode() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join(TMP);
        fs::write(&tmp, b"junk").unwrap();
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)).unwrap();
        let store = MachineStore::new(dir.path().to_owned());
        store.save(&[machine("a")]).unwrap();
        let mode = fs::metadata(dir.path().join(FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(FILE), b"{not json").unwrap();
        let store = MachineStore::new(dir.path().to_owned());
        assert_eq!(store.load().unwrap(), Vec::new());
        assert!(!dir.path().join(FILE).exists());
        assert_eq!(fs::read(dir.path().join(CORRUPT)).unwrap(), b"{not json");
        store.save(&[machine("a")]).unwrap();
        assert_eq!(store.load().unwrap(), vec![machine("a")]);
    }

    #[test]
    fn push_token_round_trips_0600() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachineStore::new(dir.path().to_owned());
        assert!(store.load_push().is_none());
        let push = PushRegisterParams {
            apns_token: protocol::PushToken::new("ab".repeat(32)).unwrap(),
            live_activity_push_to_start_token: None,
            environment: protocol::ApnsEnvironment::Sandbox,
        };
        store.save_push(&push).unwrap();
        let mode = fs::metadata(dir.path().join(PUSH_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(store.load_push(), Some(push));
        fs::write(dir.path().join(PUSH_FILE), b"{").unwrap();
        assert!(store.load_push().is_none());
    }

    #[test]
    fn ids_are_random_hex() {
        let (a, b) = (random_id(), random_id());
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
