use std::fs::Metadata;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement};
#[cfg(target_os = "macos")]
use security_framework::os::macos::keychain::SecKeychain;

#[cfg(target_os = "macos")]
use crate::keychain;

use crate::config::{self, ApnsConfig, ApnsKey, Config};
use crate::control::{self, Reply, Request};
use crate::{herdr, hooks, push};

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
    let config_path_arg = config_path.clone();
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
    let started_ms = check_daemon(&mut r, &data_dir).await;
    let (s, d) = check_service(config_path_arg.as_deref(), &data_dir, started_ms);
    r.line(s, "service", d);
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
    let tls = data_dir.join(config::TLS_KEY_FILE);
    let (s, d) = check_private(&tls, Kind::File, (Status::Warn, "not created yet"));
    r.line(s, "tls key", d);
    let audit = data_dir.join(config::AUDIT_FILE);
    let (s, d) = check_private(&audit, Kind::File, (Status::Warn, "no audit log yet"));
    r.line(s, "audit", d);
    let attachments = config::attachments_dir()?;
    let (s, d) = check_private(&attachments, Kind::Dir, (Status::Warn, "not created yet"));
    let stored = crate::attachments::stored_bytes(&attachments);
    r.line(s, "attachments", format!("{d}, {stored} bytes stored"));
    match config.as_ref().map(|c| &c.apns) {
        Some(Some(apns)) => check_apns(&mut r, apns, &data_dir.join(config::APNS_DIR)),
        Some(None) => r.line(
            Status::Warn,
            "apns",
            "not configured (collied setup offers it)",
        ),
        None => {}
    }
    #[cfg(target_os = "macos")]
    {
        let (s, d) = check_signature();
        r.line(s, "codesign", d);
    }
    let (s, d) = check_hooks();
    r.line(s, "hooks", d);
    let (s, d) = check_usage(&data_dir, crate::now_ms());
    r.line(s, "plan usage", d);

    Ok(!r.failed)
}

fn check_hooks() -> (Status, String) {
    let missing = "Claude Code PermissionRequest hook `collied hook` not installed; approvals name the tool call from the screen only";
    let Ok(path) = hooks::settings_path() else {
        return (Status::Warn, missing.to_owned());
    };
    let settings = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if hooks::installed(&settings) {
        (
            Status::Ok,
            format!("PermissionRequest hook in {}", path.display()),
        )
    } else {
        (Status::Warn, missing.to_owned())
    }
}

fn check_usage(data_dir: &Path, now_ms: u64) -> (Status, String) {
    let Some(recorded) = crate::usage::last_recorded(data_dir) else {
        return (
            Status::Warn,
            "nothing recorded; add `collied statusline` to the Claude Code status line (README)"
                .to_owned(),
        );
    };
    let sessions = recorded.windows.len();
    match recorded.plan {
        Some(plan) => {
            let minutes = now_ms.saturating_sub(plan.recorded_ms) / 60_000;
            let windows: Vec<&str> = [("5h", &plan.five_hour), ("7d", &plan.seven_day)]
                .into_iter()
                .filter_map(|(name, w)| w.as_ref().map(|_| name))
                .collect();
            (
                Status::Ok,
                format!(
                    "recorded {minutes} min ago ({}), windows for {sessions} session(s)",
                    if windows.is_empty() {
                        "no window open".to_owned()
                    } else {
                        windows.join(", ")
                    }
                ),
            )
        }
        None => (
            Status::Warn,
            format!(
                "windows for {sessions} session(s) but no rate limits yet (claude.ai Pro and Max only, after the first reply)"
            ),
        ),
    }
}

#[cfg(target_os = "macos")]
const SIGNING_ID: &str = "dev.rbstp.collied";
// Apple's Developer ID Application requirement: the Developer ID CA intermediate and the
// Developer ID Application leaf marker.
#[cfg(target_os = "macos")]
const DEVELOPER_ID: &str = "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists";

// Interaction is disabled so a Keychain that would prompt fails instead: no dialog, and
// what this binary can read is what the daemon (same binary) can read unattended.
#[cfg(target_os = "macos")]
fn check_keychain(
    r: &mut Report,
    apns: &ApnsConfig,
    ids: &str,
) -> Option<zeroize::Zeroizing<Vec<u8>>> {
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

#[cfg(not(target_os = "macos"))]
fn check_keychain(r: &mut Report, _: &ApnsConfig, _: &str) -> Option<zeroize::Zeroizing<Vec<u8>>> {
    r.line(Status::Fail, "apns", "key = \"keychain\" is macOS only");
    None
}

#[cfg(target_os = "linux")]
fn check_credential(
    r: &mut Report,
    apns: &ApnsConfig,
    ids: &str,
) -> Option<zeroize::Zeroizing<Vec<u8>>> {
    use crate::creds;
    let path = match push::credential_path(&apns.key_id) {
        Ok(p) => p,
        Err(e) => {
            r.line(Status::Fail, "apns", format!("{e:#}"));
            return None;
        }
    };
    let (s, d) = check_private(
        &path,
        Kind::File,
        (
            Status::Fail,
            "missing: run `collied apns import <AuthKey.p8>`",
        ),
    );
    if s == Status::Fail {
        r.line(s, "apns", d);
        return None;
    }
    let credential = match push::read_key(&path) {
        Ok((c, _)) => c,
        Err(e) => {
            r.line(Status::Fail, "apns", format!("{e:#}"));
            return None;
        }
    };
    let seal = creds::seal(&credential);
    match creds::decrypt(&creds::name(&apns.key_id), &credential) {
        Ok(key) => {
            let status = match seal {
                creds::Seal::Null => Status::Fail,
                creds::Seal::Other => Status::Warn,
                _ => s,
            };
            r.line(
                status,
                "apns",
                format!(
                    "{d}: systemd credential, {}, decrypts ({ids})",
                    seal.describe()
                ),
            );
            if seal == creds::Seal::Host && creds::has_tpm2() {
                r.line(
                    Status::Warn,
                    "apns",
                    "a TPM2 is usable now: import the key again to bind it to the TPM2",
                );
            }
            Some(key)
        }
        Err(e) => {
            r.line(
                Status::Fail,
                "apns",
                format!("{d}: does not decrypt: {e:#}"),
            );
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn check_credential(
    r: &mut Report,
    _: &ApnsConfig,
    _: &str,
) -> Option<zeroize::Zeroizing<Vec<u8>>> {
    r.line(
        Status::Fail,
        "apns",
        "key = \"systemd-creds\" is Linux only",
    );
    None
}

#[cfg(target_os = "linux")]
fn creds_usable() -> bool {
    let name = crate::creds::name("doctor-probe");
    crate::creds::encrypt(&name, b"probe")
        .and_then(|c| crate::creds::decrypt(&name, &c))
        .is_ok_and(|p| p.as_slice() == b"probe")
}

#[cfg(not(target_os = "linux"))]
fn creds_usable() -> bool {
    false
}

fn check_key_file(r: &mut Report, path: &Path, ids: &str) -> Option<zeroize::Zeroizing<Vec<u8>>> {
    let (s, d) = check_private(path, Kind::File, (Status::Fail, "key missing"));
    r.line(s, "apns", format!("{d} ({ids})"));
    if cfg!(target_os = "macos") {
        r.line(
            Status::Warn,
            "apns",
            format!(
                "key_path is set: move the key to the Keychain with `collied apns import '{}'`",
                path.display()
            ),
        );
    } else if creds_usable() {
        r.line(
            Status::Warn,
            "apns",
            format!(
                "key_path is set and systemd-creds works: encrypt the key with `collied apns import '{}'`",
                path.display()
            ),
        );
    } else {
        r.line(
            Status::Ok,
            "apns",
            "0600 file fallback (systemd-creds unavailable): protected by its mode only",
        );
    }
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

fn check_apns(r: &mut Report, apns: &ApnsConfig, apns_dir: &Path) {
    let ids = format!(
        "key {}, team {}, bundle {}",
        apns.key_id, apns.team_id, apns.bundle_id
    );
    let key = match &apns.key {
        ApnsKey::Keychain => check_keychain(r, apns, &ids),
        ApnsKey::SystemdCreds => check_credential(r, apns, &ids),
        ApnsKey::File(path) => check_key_file(r, path, &ids),
    };
    if let Some(key) = key {
        match push::check_ids(apns).and_then(|()| push::Apns::with_key(apns, &key)) {
            Ok(_) => r.line(Status::Ok, "apns", "provider token signed (dry run)"),
            Err(e) => r.line(Status::Fail, "apns", format!("{e:#}")),
        }
    }
    let configured = match &apns.key {
        ApnsKey::File(p) => Some(p.as_path()),
        ApnsKey::Keychain | ApnsKey::SystemdCreds => None,
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
#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
fn check_signature() -> (Status, String) {
    match signed_as_collied() {
        Ok(()) => (Status::Ok, format!("Developer ID, identifier {SIGNING_ID}")),
        Err(e) => (
            Status::Warn,
            format!("{e}: install with `just collied-install`"),
        ),
    }
}

fn check_service(
    config: Option<&Path>,
    data_dir: &Path,
    started_ms: Option<u64>,
) -> (Status, String) {
    use crate::service::{self, State};
    match service::state(config, data_dir, started_ms) {
        Ok(State::Current) => (Status::Ok, format!("up to date{}", service::note())),
        Ok(State::Outdated(why)) => (Status::Warn, format!("{why}: run collied setup")),
        Ok(State::Stopped) => (
            Status::Warn,
            "stopped: collied start, or collied setup".to_owned(),
        ),
        Ok(State::Missing) => (Status::Warn, "not installed: run collied setup".to_owned()),
        Err(e) => (Status::Warn, format!("{e:#}")),
    }
}

/// The daemon's start time, when it answers.
async fn check_daemon(r: &mut Report, data_dir: &Path) -> Option<u64> {
    let socket = data_dir.join(config::CONTROL_SOCKET);
    let info = match control::request(&socket, &Request::Status).await {
        Ok(Some(Reply::Status(info))) => info,
        Ok(None) => {
            r.line(Status::Warn, "daemon", "not running");
            return None;
        }
        Ok(Some(other)) => {
            r.line(
                Status::Fail,
                "daemon",
                format!("unexpected reply {other:?}"),
            );
            return None;
        }
        Err(e) => {
            r.line(Status::Fail, "daemon", format!("{}: {e}", socket.display()));
            return None;
        }
    };
    let (s, d) = check_private(&socket, Kind::Socket, (Status::Fail, "missing"));
    r.line(s, "control", d);
    let node_status = if info.backend_state == "Running" {
        Status::Ok
    } else {
        Status::Fail
    };
    let tag = crate::daemon::NODE_TAG;
    match info.tags.as_deref() {
        None => r.line(
            Status::Warn,
            "tag",
            "not reported: the node status failed, or the daemon predates this check (restart it)",
        ),
        Some(tags) if tags.iter().any(|t| t == tag) => {
            r.line(Status::Ok, "tag", format!("node tagged {tag}"))
        }
        Some(tags) => r.line(
            Status::Fail,
            "tag",
            format!(
                "node not tagged {tag} (tags: {}); see collied login",
                if tags.is_empty() {
                    "none".to_owned()
                } else {
                    tags.join(", ")
                }
            ),
        ),
    }
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
    match info.user_peers {
        Some(0) => r.line(
            Status::Warn,
            "reach",
            format!(
                "no device of yours can reach this node: the policy lacks the grant to {tag} on TCP {}, or your phone is not signed in to Tailscale yet (collied setup prints the policy entries)",
                info.port
            ),
        ),
        Some(n) => r.line(
            Status::Ok,
            "reach",
            format!("{n} untagged device(s) can reach this node"),
        ),
        None => {}
    }
    match tailnet::kernel_tcp_listeners(info.pid) {
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
        Err(e) => r.line(Status::Fail, "listen", e.to_string()),
    }
    info.started_ms
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
    fn reports_the_status_line_tap() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("collie");
        let (s, d) = check_usage(&data, 0);
        assert!(s == Status::Warn && d.contains("collied statusline"));
        let input = include_str!("../tests/fixtures/statusline.json");
        crate::usage::record(&data, input.as_bytes(), 60_000).unwrap();
        let (s, d) = check_usage(&data, 240_000);
        assert!(s == Status::Ok);
        assert_eq!(d, "recorded 3 min ago (5h, 7d), windows for 1 session(s)");
    }
}
