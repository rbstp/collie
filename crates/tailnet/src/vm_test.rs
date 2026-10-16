use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::{Error, Result};

pub const CONTROL_URL: &str = "http://127.0.0.1:8789";

pub fn enabled() -> bool {
    std::env::var_os("COLLIE_VM_TEST").is_some_and(|value| value == "1")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    auth_key: String,
    run_id: String,
}

pub fn load(directory: &Path) -> Result<Option<Zeroizing<String>>> {
    if !enabled() {
        return Ok(None);
    }
    read(directory).map(Some)
}

fn read(directory: &Path) -> Result<Zeroizing<String>> {
    let invalid = || Error::InvalidArgument("private VM test marker");
    if directory.file_name().and_then(|name| name.to_str()) != Some("collie-vm-test") {
        return Err(invalid());
    }
    let metadata = std::fs::symlink_metadata(directory)?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_dir() || metadata.uid() != uid || metadata.permissions().mode() & 0o077 != 0 {
        return Err(invalid());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join("vm-tailnet.json"))?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.len() > 4096
    {
        return Err(invalid());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(invalid());
    }
    let marker: Marker = serde_json::from_slice(&bytes)?;
    if !marker.auth_key.starts_with("test-authkey-collievm")
        || !(40..=96).contains(&marker.auth_key.len())
        || !marker
            .auth_key
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || value == b'-')
        || marker.run_id.len() != 36
        || !marker
            .run_id
            .bytes()
            .all(|value| value.is_ascii_hexdigit() || value == b'-')
    {
        return Err(invalid());
    }
    Ok(Zeroizing::new(marker.auth_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, symlink};

    #[test]
    fn marker_requires_isolated_private_state_and_a_synthetic_key() {
        let root = std::env::temp_dir().join(format!(
            "collie-vm-marker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let directory = root.join("collie-vm-test");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let file = directory.join("vm-tailnet.json");
        let key = format!("test-authkey-collievm{}", "a".repeat(32));
        let marker =
            serde_json::json!({"auth_key": key, "run_id": "00000000-0000-0000-0000-000000000000"});
        std::fs::write(&file, marker.to_string()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read(&directory).is_ok());
        std::fs::write(&file, format!("{} {{}}", marker)).unwrap();
        assert!(read(&directory).is_err());
        std::fs::write(&file, "x".repeat(4097)).unwrap();
        assert!(read(&directory).is_err());
        std::fs::write(&file, marker.to_string()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read(&directory).is_err());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let moved = root.join("marker");
        std::fs::rename(&file, &moved).unwrap();
        symlink(&moved, &file).unwrap();
        assert!(read(&directory).is_err());
        std::fs::remove_file(&file).unwrap();
        std::fs::write(
            &file,
            serde_json::json!({"auth_key":"tskey-not-a-test-key", "run_id":marker["run_id"]})
                .to_string(),
        )
        .unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read(&directory).is_err());
        assert!(read(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
