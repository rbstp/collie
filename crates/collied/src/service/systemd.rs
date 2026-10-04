use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;

use crate::config;

pub const UNIT: &str = "collied.service";

/// The variables collied resolves its directories from, pinned to what this CLI sees,
/// so the daemon uses the same data dir whatever the user manager's environment holds.
const PINNED_ENV: [(&str, &str); 3] = [
    ("XDG_DATA_HOME", ".local/share"),
    ("XDG_CACHE_HOME", ".cache"),
    ("XDG_CONFIG_HOME", ".config"),
];

fn unit_path() -> anyhow::Result<PathBuf> {
    Ok(config::xdg_dir("XDG_CONFIG_HOME", ".config")?
        .join("systemd/user")
        .join(UNIT))
}

fn utf8(p: &Path) -> anyhow::Result<&str> {
    p.to_str()
        .with_context(|| format!("{}: not valid UTF-8", p.display()))
}

fn no_controls(s: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !s.chars().any(char::is_control),
        "{s:?}: control characters cannot go into a unit file"
    );
    Ok(())
}

/// Quoted, with C escapes for `"` and `\`, and `%` doubled so no specifier expands.
fn quoted(s: &str, dollar: bool) -> anyhow::Result<String> {
    no_controls(s)?;
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '%' => out.push_str("%%"),
            '$' if dollar => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

/// systemd refuses an executable path with quotes or backslashes and does not expand `$`
/// in it.
fn quote_exe(exe: &Path) -> anyhow::Result<String> {
    let s = utf8(exe)?;
    anyhow::ensure!(
        exe.is_absolute() && !s.contains(['"', '\'', '\\']),
        "{s:?}: systemd cannot run an executable at this path; install collied elsewhere"
    );
    quoted(s, false)
}

/// Arguments also expand `$VAR`, so `$` is doubled.
fn quote_arg(s: &str) -> anyhow::Result<String> {
    quoted(s, true)
}

/// `$` has no special meaning in `Environment=`.
fn quote_env(k: &str, v: &Path) -> anyhow::Result<String> {
    quoted(&format!("{k}={}", utf8(v)?), false)
}

pub fn unit(exe: &Path, config: Option<&Path>, env: &[(&str, PathBuf)]) -> anyhow::Result<String> {
    let mut exec = vec![quote_exe(exe)?];
    if let Some(c) = config {
        exec.push(quote_arg("--config")?);
        exec.push(quote_arg(utf8(c)?)?);
    }
    exec.push(quote_arg("run")?);
    let exec = exec.join(" ");
    let mut environment = String::new();
    for (k, v) in env {
        environment.push_str(&format!("Environment={}\n", quote_env(k, v)?));
    }
    // Units started with the user manager do not get XDG_*_HOME from the session, hence the
    // pinned Environment= lines. Restart=always with a growing delay is launchd's KeepAlive
    // without a hot loop. The sandboxing is best effort: in a user manager it relies on
    // unprivileged user namespaces and is skipped where they are unavailable.
    Ok(format!(
        "# Written by `collied service install`; rewritten on every install.
[Unit]
Description=collie daemon: herdr agents over the tailnet
Documentation=https://github.com/rbstp/collie
StartLimitIntervalSec=0

[Service]
Type=exec
ExecStart={exec}
{environment}Restart=always
RestartSec=2s
RestartSteps=5
RestartMaxDelaySec=1min
UMask=0077
NoNewPrivileges=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectSystem=strict
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
RestrictSUIDSGID=yes
RestrictRealtime=yes
RestrictNamespaces=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK

[Install]
WantedBy=default.target
"
    ))
}

fn systemctl(args: &[&str]) -> anyhow::Result<bool> {
    let status = Command::new(crate::system_bin("systemctl"))
        .arg("--user")
        .args(args)
        .status()
        .context("run systemctl --user")?;
    Ok(status.success())
}

fn quiet(args: &[&str]) -> anyhow::Result<bool> {
    let status = Command::new(crate::system_bin("systemctl"))
        .arg("--user")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("run systemctl --user")?;
    Ok(status.success())
}

fn active() -> anyhow::Result<bool> {
    quiet(&["is-active", "--quiet", UNIT])
}

/// Where the user manager loads the unit from, so a unit written where the manager does
/// not look fails here instead of silently never starting.
fn fragment() -> anyhow::Result<String> {
    let out = Command::new(crate::system_bin("systemctl"))
        .args(["--user", "show", "--property=FragmentPath", "--value", UNIT])
        .stderr(Stdio::null())
        .output()
        .context("run systemctl --user show")?;
    anyhow::ensure!(out.status.success(), "systemctl --user show {UNIT} failed");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn installed() -> anyhow::Result<PathBuf> {
    let path = unit_path()?;
    anyhow::ensure!(path.exists(), "not installed: run collied service install");
    Ok(path)
}

pub fn install(config: Option<&Path>, data_dir: &Path) -> anyhow::Result<()> {
    crate::ensure_private_dir(data_dir)?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let config = config.map(Path::canonicalize).transpose()?;
    let env = PINNED_ENV
        .iter()
        .map(|(k, fallback)| Ok((*k, config::xdg_dir(k, fallback)?)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let text = unit(&exe, config.as_deref(), &env)?;
    let path = unit_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("service.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
    file.sync_all()?;
    std::fs::rename(&tmp, &path)?;
    anyhow::ensure!(
        systemctl(&["daemon-reload"])?,
        "systemctl --user daemon-reload failed"
    );
    let loaded = fragment()?;
    anyhow::ensure!(
        Path::new(&loaded) == path,
        "the user manager loads {UNIT} from {loaded:?}, not {}: remove that unit or align XDG_CONFIG_HOME",
        path.display()
    );
    anyhow::ensure!(
        systemctl(&["enable", UNIT])?,
        "systemctl --user enable {UNIT} failed"
    );
    anyhow::ensure!(
        systemctl(&["restart", UNIT])?,
        "systemctl --user restart {UNIT} failed"
    );
    println!("installed {} ({})", path.display(), exe.display());
    Ok(())
}

/// Stops the unit and disables it, so it also stays off after logout or a reboot until
/// `start`; the unit file stays in place.
pub fn stop() -> anyhow::Result<()> {
    installed()?;
    let was_active = active()?;
    anyhow::ensure!(
        systemctl(&["disable", "--now", UNIT])?,
        "systemctl --user disable --now {UNIT} failed"
    );
    let _ = quiet(&["reset-failed", UNIT]);
    println!(
        "{}; stays off until collied start",
        if was_active {
            "stopped"
        } else {
            "already stopped"
        }
    );
    Ok(())
}

pub fn start() -> anyhow::Result<()> {
    installed()?;
    anyhow::ensure!(
        systemctl(&["enable", UNIT])?,
        "systemctl --user enable {UNIT} failed"
    );
    if active()? {
        println!("already running");
        return Ok(());
    }
    anyhow::ensure!(
        systemctl(&["start", UNIT])?,
        "systemctl --user start {UNIT} failed"
    );
    println!("started");
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let path = unit_path()?;
    let was_active = active()?;
    if path.exists() {
        anyhow::ensure!(
            systemctl(&["disable", "--now", UNIT])?,
            "systemctl --user disable --now {UNIT} failed; {} left in place",
            path.display()
        );
    } else if was_active {
        anyhow::bail!(
            "{UNIT} is running but not installed at {}: the user manager loads it from {:?}",
            path.display(),
            fragment().unwrap_or_default()
        );
    }
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    let _ = systemctl(&["daemon-reload"]);
    let _ = quiet(&["reset-failed", UNIT]);
    println!(
        "{} {}",
        if was_active {
            "stopped and removed"
        } else {
            "removed"
        },
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NASTY: &str = "/home/me/a b\"$HOME%h\\;.toml";

    #[test]
    fn unit_runs_absolute_exe_with_pinned_dirs() {
        let u = unit(
            Path::new("/opt/collie $x%/bin/collied"),
            Some(Path::new(NASTY)),
            &[("XDG_DATA_HOME", PathBuf::from("/home/me/$data%"))],
        )
        .unwrap();
        assert!(u.contains(
            "ExecStart=\"/opt/collie $x%%/bin/collied\" \"--config\" \"/home/me/a b\\\"$$HOME%%h\\\\;.toml\" \"run\"\n"
        ), "{u}");
        assert!(
            u.contains("Environment=\"XDG_DATA_HOME=/home/me/$data%%\"\n"),
            "{u}"
        );
        assert!(u.contains("Restart=always\n"));
        assert!(u.contains("UMask=0077\n"));
        assert!(u.contains("WantedBy=default.target\n"));
        assert!(!u.contains("User=") && !u.contains("ListenStream") && !u.contains("Socket"));
    }

    #[test]
    fn unsafe_paths_are_refused() {
        assert!(unit(Path::new("/x\n[Service]"), None, &[]).is_err());
        assert!(unit(Path::new("/a\"b/collied"), None, &[]).is_err());
        assert!(unit(Path::new("/a\\b/collied"), None, &[]).is_err());
        assert!(unit(Path::new("relative/collied"), None, &[]).is_err());
        assert!(unit(Path::new("/c"), Some(Path::new("/a\tb")), &[]).is_err());
        use std::os::unix::ffi::OsStrExt;
        let latin1 = Path::new(std::ffi::OsStr::from_bytes(b"/caf\xe9.toml"));
        assert!(unit(Path::new("/c"), Some(latin1), &[]).is_err());
    }

    // What systemd itself makes of the unit, where systemd-analyze exists.
    #[test]
    fn systemd_parses_the_unit() {
        let analyze = Path::new("/usr/bin/systemd-analyze");
        if !analyze.exists() {
            println!("skipped: no systemd-analyze");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("collied $x%");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.path().join(UNIT);
        let text = unit(
            &exe,
            Some(Path::new(NASTY)),
            &[("XDG_DATA_HOME", PathBuf::from("/home/me/$data%"))],
        )
        .unwrap();
        std::fs::write(&path, text).unwrap();
        let out = Command::new(analyze)
            .args(["--user", "verify"])
            .arg(&path)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        if [
            "Failed to connect",
            "bus",
            "Failed to initialize manager",
            "RuntimeDirectory",
        ]
        .iter()
        .any(|m| stderr.contains(m))
        {
            println!("skipped: no user manager ({stderr})");
            return;
        }
        assert!(out.status.success(), "{stderr}");
        assert!(!stderr.contains(UNIT), "{stderr}");
    }
}
