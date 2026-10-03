use std::fs::Metadata;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement};
use security_framework::os::macos::keychain::SecKeychain;

use crate::config::{self, ApnsConfig, ApnsKey, Config};
use crate::control::{self, Reply, Request};
use crate::{herdr, keychain, push};

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

struct Report {
    failed: bool,
}

impl Report {
    fn line(&mut self, status: Status, check: &str, detail: impl AsRef<str>) {
        let label = match status {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
        };
        self.failed |= status == Status::Fail;
        println!("{label:<4}  {check:<11}  {}", detail.as_ref());
    }
}

pub async fn run(config_path: Option<PathBuf>) -> anyhow::Result<bool> {
    let mut r = Report { failed: false };
    let data_dir = config::data_dir()?;
    let explicit = config_path.is_some();
    let config_path = config_path.unwrap_or_else(|| data_dir.join(config::CONFIG_FILE));

    let config = match config::load(&config_path, explicit) {
        Ok(Some(c)) => {
            r.line(Status::Ok, "config", config_path.display().to_string());
            Some(c)
        }
        Ok(None) => {
            r.line(
                Status::Ok,
                "config",
                format!("{} absent, using defaults", config_path.display()),
            );
            Some(Config::default())
        }
        Err(e) => {
            r.line(Status::Fail, "config", format!("{e:#}"));
            None
        }
    };

    let (s, d) = check_private(&data_dir, Kind::Dir, (Status::Warn, "missing"));
    r.line(s, "data dir", d);
    check_daemon(&mut r, &data_dir).await;
    match &config {
        Some(config) => {
            check_herdr(&mut r, config).await;
            check_tailnet(&mut r, config, &data_dir.join(config::TSNET_DIR));
        }
        None => {
            for check in ["herdr", "tailnet", "apns"] {
                r.line(Status::Warn, check, "skipped: config did not parse");
            }
        }
    }
    let peers = data_dir.join(config::PEERS_FILE);
    let (s, d) = check_private(&peers, Kind::File, (Status::Warn, "no paired phones"));
    r.line(s, "peers", d);
    let audit = data_dir.join(config::AUDIT_FILE);
    let (s, d) = check_private(&audit, Kind::File, (Status::Warn, "no audit log yet"));
    r.line(s, "audit", d);
    let attachments = config::attachments_dir()?;
    let (s, d) = check_private(&attachments, Kind::Dir, (Status::Warn, "not created yet"));
    let stored = crate::attachments::stored_bytes(&attachments);
    r.line(s, "attachments", format!("{d}, {stored} bytes stored"));
    match config.as_ref().map(|c| &c.apns) {
        Some(Some(apns)) => check_apns(&mut r, apns, &data_dir.join(APNS_DIR)),
        Some(None) => r.line(Status::Warn, "apns", "not configured"),
        None => {}
    }
    let (s, d) = check_signature();
    r.line(s, "codesign", d);
    r.line(
        Status::Warn,
        "hooks",
        "Claude Code hooks not installed (phase 5)",
    );

    Ok(!r.failed)
}

const APNS_DIR: &str = "apns";
const SIGNING_ID: &str = "dev.rbstp.collied";
// Apple's Developer ID Application requirement: the Developer ID CA intermediate and the
// Developer ID Application leaf marker.
const DEVELOPER_ID: &str = "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists";

// Interaction is disabled so a Keychain that would prompt fails instead: no dialog, and
// what this binary can read is what the daemon (same binary) can read unattended.
fn check_apns(r: &mut Report, apns: &ApnsConfig, apns_dir: &Path) {
    let ids = format!(
        "key {}, team {}, bundle {}",
        apns.key_id, apns.team_id, apns.bundle_id
    );
    let key = match &apns.key {
        ApnsKey::Keychain => {
            let item = push::keychain_item(&apns.key_id);
            let read = SecKeychain::disable_user_interaction()
                .map_err(anyhow::Error::from)
                .and_then(|_lock| Ok(keychain::read(None, &apns.key_id)?));
            match read {
                Ok(Some(key)) => {
                    r.line(
                        Status::Ok,
                        "apns",
                        format!("{item} readable by collied ({ids})"),
                    );
                    Some(key)
                }
                Ok(None) => {
                    r.line(
                        Status::Fail,
                        "apns",
                        format!("{item} missing: run `collied apns import <AuthKey.p8>`"),
                    );
                    None
                }
                Err(e) => {
                    r.line(
                        Status::Fail,
                        "apns",
                        format!("{item} not readable by this binary without a prompt: {e}"),
                    );
                    None
                }
            }
        }
        ApnsKey::File(path) => {
            let (s, d) = check_private(path, Kind::File, (Status::Fail, "key missing"));
            r.line(s, "apns", format!("{d} ({ids})"));
            r.line(
                Status::Warn,
                "apns",
                format!(
                    "key_path is set: move the key to the Keychain with `collied apns import '{}'`",
                    path.display()
                ),
            );
            if s == Status::Fail {
                None
            } else {
                match push::read_key(path) {
                    Ok((key, _)) => Some(key),
                    Err(e) => {
                        r.line(Status::Fail, "apns", format!("{e:#}"));
                        None
                    }
                }
            }
        }
    };
    if let Some(key) = key {
        match push::check_ids(apns).and_then(|()| push::Apns::with_key(apns, &key)) {
            Ok(_) => r.line(Status::Ok, "apns", "provider token signed (dry run)"),
            Err(e) => r.line(Status::Fail, "apns", format!("{e:#}")),
        }
    }
    let configured = match &apns.key {
        ApnsKey::File(p) => Some(p.as_path()),
        ApnsKey::Keychain => None,
    };
    if let Ok(entries) = std::fs::read_dir(apns_dir) {
        for path in entries.flatten().map(|e| e.path()) {
            if path.extension().is_some_and(|x| x == "p8") && Some(path.as_path()) != configured {
                r.line(
                    Status::Warn,
                    "apns",
                    format!("{} still on disk", path.display()),
                );
            }
        }
    }
}

/// Whether the running binary is Developer ID signed with collied's identifier.
pub fn signed_as_collied() -> Result<(), String> {
    let check = |requirement: &str| -> Result<(), security_framework::base::Error> {
        let requirement: SecRequirement = requirement.parse()?;
        SecCode::for_self(Flags::NONE)?.check_validity(Flags::NONE, &requirement)
    };
    check(DEVELOPER_ID)
        .map_err(|e| format!("not signed with a Developer ID Application identity ({e})"))?;
    check(&format!("identifier \"{SIGNING_ID}\""))
        .map_err(|_| format!("Developer ID, but the identifier is not {SIGNING_ID}"))
}

fn check_signature() -> (Status, String) {
    match signed_as_collied() {
        Ok(()) => (Status::Ok, format!("Developer ID, identifier {SIGNING_ID}")),
        Err(e) => (
            Status::Warn,
            format!("{e}: install with `just collied-install`"),
        ),
    }
}

async fn check_daemon(r: &mut Report, data_dir: &Path) {
    let socket = data_dir.join(config::CONTROL_SOCKET);
    let info = match control::request(&socket, &Request::Status).await {
        Ok(Some(Reply::Status(info))) => info,
        Ok(None) => return r.line(Status::Warn, "daemon", "not running"),
        Ok(Some(other)) => {
            return r.line(
                Status::Fail,
                "daemon",
                format!("unexpected reply {other:?}"),
            );
        }
        Err(e) => return r.line(Status::Fail, "daemon", format!("{}: {e}", socket.display())),
    };
    let (s, d) = check_private(&socket, Kind::Socket, (Status::Fail, "missing"));
    r.line(s, "control", d);
    let node_status = if info.backend_state == "Running" {
        Status::Ok
    } else {
        Status::Fail
    };
    r.line(
        node_status,
        "daemon",
        format!(
            "pid {}, node {} ({}), {}:{}, {} session(s), {} peer(s)",
            info.pid,
            info.backend_state,
            info.node_id,
            info.dns_name,
            info.port,
            info.sessions,
            info.peers
        ),
    );
    match kernel_listeners(info.pid) {
        Ok(lines) if lines.is_empty() => r.line(
            Status::Ok,
            "listen",
            "no kernel TCP listener (tailnet only)",
        ),
        Ok(lines) => r.line(
            Status::Fail,
            "listen",
            format!("kernel TCP listener(s): {}", lines.join("; ")),
        ),
        Err(e) => r.line(Status::Fail, "listen", format!("lsof: {e}")),
    }
}

// lsof prints nothing and exits 1 both when nothing matches and when the pid is gone, so
// an empty result only counts once the pid is known to be alive and lsof stayed silent.
fn kernel_listeners(pid: u32) -> std::io::Result<Vec<String>> {
    let pid_ok = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .is_some_and(|p| rustix::process::test_kill_process(p).is_ok());
    if !pid_ok {
        return Err(std::io::Error::other(format!(
            "daemon pid {pid} is not running"
        )));
    }
    let out = std::process::Command::new("/usr/sbin/lsof")
        .args(["-nP", "-a", "-p", &pid.to_string(), "-iTCP", "-sTCP:LISTEN"])
        .output()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() || !matches!(out.status.code(), Some(0 | 1)) {
        return Err(std::io::Error::other(format!(
            "{} ({})",
            stderr.trim(),
            out.status
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .map(str::to_owned)
        .collect())
}

#[derive(Clone, Copy)]
enum Kind {
    Dir,
    File,
    Socket,
}

fn check_private(
    path: &Path,
    kind: Kind,
    (missing_status, missing): (Status, &str),
) -> (Status, String) {
    let shown = path.display();
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (missing_status, format!("{missing}: {shown}"));
        }
        Err(e) => return (Status::Fail, format!("{shown}: {e}")),
    };
    let (is_kind, want) = match kind {
        Kind::Dir => (meta.is_dir(), 0o700),
        Kind::File => (meta.is_file(), 0o600),
        Kind::Socket => (meta.file_type().is_socket(), 0o600),
    };
    if !is_kind {
        return (Status::Fail, format!("{shown}: unexpected file type"));
    }
    let (status, detail) = ownership(&meta, want);
    (status, format!("{shown}: {detail}"))
}

fn ownership(meta: &Metadata, want: u32) -> (Status, String) {
    let uid = rustix::process::getuid().as_raw();
    let mode = meta.mode() & 0o777;
    if meta.uid() != uid {
        (
            Status::Fail,
            format!("owned by uid {}, not {uid}", meta.uid()),
        )
    } else if mode & !want != 0 {
        (
            Status::Fail,
            format!("mode {mode:04o} too open, want {want:04o}"),
        )
    } else if mode != want {
        (Status::Warn, format!("mode {mode:04o}, want {want:04o}"))
    } else {
        (Status::Ok, format!("mode {mode:04o}"))
    }
}

async fn check_herdr(r: &mut Report, config: &Config) {
    let env = herdr::SocketEnv::from_process();
    let socket = match herdr::resolve_socket_path(config.herdr.session.as_deref(), &env) {
        Ok(p) => p,
        Err(e) => return r.line(Status::Fail, "herdr", e.to_string()),
    };
    let shown = socket.display();
    let meta = match std::fs::symlink_metadata(&socket) {
        Ok(m) => m,
        Err(e) => return r.line(Status::Fail, "herdr", format!("{shown}: {e}")),
    };
    if !meta.file_type().is_socket() {
        return r.line(Status::Fail, "herdr", format!("{shown}: not a socket"));
    }
    let (status, detail) = ownership(&meta, 0o600);
    if status != Status::Ok {
        return r.line(status, "herdr", format!("{shown}: {detail}"));
    }
    match herdr::ping(&socket).await {
        Ok(pong) => r.line(
            Status::Ok,
            "herdr",
            format!(
                "{shown}: herdr {} (protocol {})",
                pong.version, pong.protocol
            ),
        ),
        Err(e) => r.line(Status::Fail, "herdr", format!("{shown}: ping: {e}")),
    }
}

// Read-only: the node is never started here because a running collied may own the state.
fn check_tailnet(r: &mut Report, config: &Config, tsnet: &Path) {
    let node = format!("{}:{}", config.tailnet.hostname(), config.tailnet.port);
    let shown = tsnet.display();
    let meta = match std::fs::symlink_metadata(tsnet) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return r.line(
                Status::Warn,
                "tailnet",
                format!("{node}: no node state ({shown} missing)"),
            );
        }
        Err(e) => return r.line(Status::Fail, "tailnet", format!("{shown}: {e}")),
    };
    if !meta.is_dir() {
        return r.line(Status::Fail, "tailnet", format!("{shown}: not a directory"));
    }
    let (status, detail) = ownership(&meta, 0o700);
    if status != Status::Ok {
        return r.line(status, "tailnet", format!("{shown}: {detail}"));
    }
    if tsnet.join("tailscaled.state").is_file() {
        r.line(Status::Ok, "tailnet", format!("{node}: node state present"));
    } else {
        r.line(
            Status::Warn,
            "tailnet",
            format!("{node}: no node state (not logged in)"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listener_check_needs_a_live_pid() {
        assert_eq!(
            kernel_listeners(std::process::id()).unwrap(),
            Vec::<String>::new()
        );
        assert!(kernel_listeners(999_999).is_err());
        assert!(kernel_listeners(u32::MAX).is_err());
    }
}
