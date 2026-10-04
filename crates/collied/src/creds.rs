//! The APNs key as a systemd user credential: encrypted with the host key (and the TPM2
//! when there is one), bound to this user and to the credential name.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;
use zeroize::Zeroizing;

const SYSTEMD_CREDS: &str = "/usr/bin/systemd-creds";
const SYSTEMD_ANALYZE: &str = "/usr/bin/systemd-analyze";
const MAX_CREDENTIAL: u64 = 64 * 1024;

pub fn name(key_id: &str) -> String {
    format!("collied-apns-{key_id}")
}

pub fn path(apns_dir: &Path, key_id: &str) -> PathBuf {
    apns_dir.join(format!("{key_id}.cred"))
}

/// What the credential is sealed with, from the id at the start of its header
/// (systemd src/shared/creds-util.h, v261 and main).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seal {
    Host,
    HostAndTpm2,
    Tpm2,
    /// A fixed zero key: integrity only, no confidentiality.
    Null,
    Other,
}

impl Seal {
    pub fn describe(self) -> &'static str {
        match self {
            Seal::Host => "host key only (no usable TPM2)",
            Seal::HostAndTpm2 => "host key and TPM2",
            Seal::Tpm2 => "TPM2",
            Seal::Null => "null key, not encrypted",
            Seal::Other => "unrecognized seal",
        }
    }
}

const SEALS: [(&str, Seal); 15] = [
    ("5a1c6a86df9d4096b1d5a65e0862f19a", Seal::Host),
    ("55b9ed1d38594d43a8319d2ebb332ac6", Seal::Host),
    ("0c7cc07b117645919c4b0bea08bc20fe", Seal::Tpm2),
    ("faf7eb9341e3412ca1a436f95a29362f", Seal::Tpm2),
    ("d4062dfb71ad4c86804b40ef1180f1fc", Seal::Tpm2),
    ("5e2d5c7603724eaf843c6fb5f64098f5", Seal::Tpm2),
    ("93a894094874449090caf2fc93cab553", Seal::HostAndTpm2),
    ("ef4ac13679a9480ea7db68897f9f165d", Seal::HostAndTpm2),
    ("af4950a849134eb1a73846304ff30c05", Seal::HostAndTpm2),
    ("adbc4ca3efb64201ba881b6f2e4095ea", Seal::HostAndTpm2),
    ("1414258818a240cd900bce862db5c7b9", Seal::HostAndTpm2),
    ("2a1f877a4275431ab3f9ed1f5d8f6601", Seal::HostAndTpm2),
    ("afbfeaaceb6a4a3795419d135c47f37b", Seal::HostAndTpm2),
    ("16e492949f94400286758f94b7c52bc7", Seal::HostAndTpm2),
    ("058469daf6f54324800549da0f8ea2fb", Seal::Null),
];

fn id(hex: &str) -> Option<[u8; 16]> {
    let mut out = [0u8; 16];
    if hex.len() != 32 {
        return None;
    }
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

pub fn seal(credential: &[u8]) -> Seal {
    use base64::Engine;
    let text: Vec<u8> = credential
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .take(24)
        .collect();
    let Ok(head) = base64::engine::general_purpose::STANDARD.decode(&text) else {
        return Seal::Other;
    };
    let Some(head) = head.get(..16) else {
        return Seal::Other;
    };
    SEALS
        .iter()
        .find(|(hex, _)| id(hex).is_some_and(|id| id == head))
        .map_or(Seal::Other, |(_, s)| *s)
}

/// Whether systemd reports a fully usable TPM2 (firmware, driver, subsystem, libraries).
pub fn has_tpm2() -> bool {
    Command::new(SYSTEMD_ANALYZE)
        .args(["has-tpm2", "--quiet"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn run(args: &[&str], input: &[u8]) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    let mut child = Command::new(SYSTEMD_CREDS)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("run {SYSTEMD_CREDS}"))?;
    let mut stdin = child.stdin.take().context("systemd-creds stdin")?;
    let mut stdout = child.stdout.take().context("systemd-creds stdout")?;
    let mut stderr = child.stderr.take().context("systemd-creds stderr")?;
    let writer = std::thread::scope(|s| -> anyhow::Result<_> {
        let w = s.spawn(move || stdin.write_all(input));
        let e = s.spawn(move || {
            let mut err = String::new();
            let _ = stderr.read_to_string(&mut err);
            err
        });
        let mut out = Zeroizing::new(Vec::new());
        (&mut stdout)
            .take(MAX_CREDENTIAL + 1)
            .read_to_end(&mut out)?;
        let wrote = w.join().map_err(|_| anyhow::anyhow!("writer panicked"))?;
        let err = e.join().unwrap_or_default();
        Ok((out, wrote, err))
    })?;
    let (out, wrote, err) = writer;
    let status = child.wait()?;
    anyhow::ensure!(
        status.success(),
        "systemd-creds {}: {} ({status})",
        args.first().copied().unwrap_or_default(),
        err.trim()
    );
    wrote.context("write to systemd-creds")?;
    anyhow::ensure!(
        out.len() as u64 <= MAX_CREDENTIAL,
        "systemd-creds output too large"
    );
    Ok(out)
}

/// Encrypts `secret` for this user under `name`. An unprivileged caller cannot pick the
/// key: systemd seals with the host key and the TPM2 when one is usable, the host key
/// alone otherwise, so [`seal`] tells which it was.
pub fn encrypt(name: &str, secret: &[u8]) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(!name.is_empty(), "a credential needs a name");
    let name = format!("--name={name}");
    let out = run(
        &["--user", "--no-ask-password", "encrypt", &name, "-", "-"],
        secret,
    )?;
    Ok(out.to_vec())
}

/// An empty name, or none, would skip the check of the name sealed in the credential.
pub fn decrypt(name: &str, credential: &[u8]) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    anyhow::ensure!(!name.is_empty(), "a credential needs a name");
    let name = format!("--name={name}");
    run(
        &["--user", "--no-ask-password", "decrypt", &name, "-", "-"],
        credential,
    )
}

/// Writes the credential 0600 at `path` (never through a symlink, never over a file).
pub fn store(path: &Path, credential: &[u8]) -> anyhow::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        crate::ensure_private_dir(dir)?;
    }
    let tmp = path.with_extension("cred.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(&tmp)
        .with_context(|| format!("create {}", tmp.display()))?;
    file.write_all(credential)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_from_header() {
        // A --user --with-key=host credential made on a machine without a usable TPM2.
        let host =
            b"VbntHThZTUOoMZ0uuzMqxiAAAAABAAAADAAAABAAAAAWAkvUACl2iDhZlswAAAAABwAAAAAAAAAGrar";
        assert_eq!(seal(host), Seal::Host);
        assert_eq!(seal(b""), Seal::Other);
        assert_eq!(seal(b"not base64 at all!!!!!!!!!!"), Seal::Other);
        for (hex, _) in SEALS {
            assert!(id(hex).is_some(), "{hex}");
        }
        assert!(decrypt("", b"x").is_err());
    }

    // Skipped where systemd-creds cannot encrypt for this user (no systemd, no varlink).
    #[test]
    fn round_trip_when_available() {
        let secret = crate::push::tests::TEST_KEY.as_bytes();
        let Ok(credential) = encrypt(&name("ABCDE12345"), secret) else {
            println!("skipped: systemd-creds --user encrypt is unavailable");
            return;
        };
        assert!(!credential.windows(secret.len()).any(|w| w == secret));
        assert!(matches!(seal(&credential), Seal::Host | Seal::HostAndTpm2));
        assert_eq!(
            decrypt(&name("ABCDE12345"), &credential)
                .unwrap()
                .as_slice(),
            secret
        );
        assert!(decrypt(&name("ZZZZZ99999"), &credential).is_err());
        let mut tampered = credential.clone();
        let i = tampered.len() / 2;
        tampered[i] = if tampered[i] == b'A' { b'B' } else { b'A' };
        assert!(decrypt(&name("ABCDE12345"), &tampered).is_err());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apns").join("ABCDE12345.cred");
        store(&path, &credential).unwrap();
        let (read, st) = crate::push::read_key(&path).unwrap();
        assert_eq!(read.as_slice(), credential.as_slice());
        assert_eq!(st.st_mode & 0o777, 0o600);
    }

    #[test]
    fn names_and_paths() {
        assert_eq!(name("ABCDE12345"), "collied-apns-ABCDE12345");
        assert_eq!(
            path(Path::new("/d/apns"), "ABCDE12345"),
            Path::new("/d/apns/ABCDE12345.cred")
        );
    }
}
