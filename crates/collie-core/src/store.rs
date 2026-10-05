use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct Machine {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub node_id: String,
    /// The tag seen at pairing, required on every later connection. Entries written
    /// before Linux support are Macs: the pin accepted only tag:collie-mac then.
    #[serde(default)]
    pub kind: MachineKind,
    /// Empty for a pairing made before mutual TLS, which must pair again.
    #[serde(default)]
    pub key: String,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum,
)]
#[serde(rename_all = "lowercase")]
pub enum MachineKind {
    #[default]
    Mac,
    Linux,
}

impl MachineKind {
    pub const ALL: [Self; 2] = [Self::Mac, Self::Linux];

    pub fn tag(self) -> &'static str {
        match self {
            Self::Mac => "tag:collie-mac",
            Self::Linux => "tag:collie-linux",
        }
    }
}

pub struct MachineStore {
    dir: PathBuf,
}

const FILE: &str = "machines.json";
#[cfg(test)]
const TMP: &str = ".machines.json.tmp";
const CORRUPT: &str = "machines.json.corrupt";

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
            kind: MachineKind::Mac,
            key: String::new(),
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
    fn entries_from_before_linux_support_are_macs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(FILE),
            br#"[{"id":"a","label":"mac","host":"mac.tail1234.ts.net","port":8457,"node_id":"nMAC"}]"#,
        )
        .unwrap();
        let store = MachineStore::new(dir.path().to_owned());
        assert_eq!(store.load().unwrap(), vec![machine("a")]);
        let linux = Machine {
            kind: MachineKind::Linux,
            ..machine("b")
        };
        store.save(std::slice::from_ref(&linux)).unwrap();
        assert!(
            fs::read_to_string(dir.path().join(FILE))
                .unwrap()
                .contains(r#""kind": "linux""#)
        );
        assert_eq!(store.load().unwrap(), vec![linux]);
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
    fn ids_are_random_hex() {
        let (a, b) = (random_id(), random_id());
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
