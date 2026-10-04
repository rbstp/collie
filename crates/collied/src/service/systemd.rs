use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;

use crate::config;

pub const UNIT: &str = "collied.service";
const SYSTEMCTL: &str = "/usr/bin/systemctl";

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

/// One word for `ExecStart=` or one `Environment=` assignment: double quoted, C escapes
/// for quotes and backslashes, `%` and `$` doubled so neither specifiers nor variables
/// expand. Control characters cannot be written into a unit file safely.
fn quote(s: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        !s.chars().any(char::is_control),
        "{s:?}: control characters cannot go into a unit file"
    );
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '%' => out.push_str("%%"),
            '$' => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

pub fn unit(exe: &Path, config: Option<&Path>, env: &[(&str, PathBuf)]) -> anyhow::Result<String> {
    let mut args = vec![exe.display().to_string()];
    if let Some(c) = config {
        args.push("--config".into());
        args.push(c.display().to_string());
    }
    args.push("run".into());
    let exec = args
        .iter()
        .map(|a| quote(a))
        .collect::<anyhow::Result<Vec<_>>>()?
        .join(" ");
    let mut environment = String::new();
    for (k, v) in env {
        environment.push_str(&format!(
            "Environment={}\n",
            quote(&format!("{k}={}", v.display()))?
        ));
    }
    // UMask 0077 keeps every file the daemon creates private. Restart=always with a delay
    // matches launchd's KeepAlive without tripping the start rate limit.
    Ok(format!(
        "[Unit]
Description=collie daemon: herdr agents over the tailnet
Documentation=https://github.com/rbstp/collie

[Service]
Type=exec
ExecStart={exec}
{environment}Restart=always
RestartSec=5
UMask=0077
NoNewPrivileges=yes

[Install]
WantedBy=default.target
"
    ))
}

fn systemctl(args: &[&str]) -> anyhow::Result<bool> {
    let status = Command::new(SYSTEMCTL)
        .arg("--user")
        .args(args)
        .status()
        .context("run systemctl --user")?;
    Ok(status.success())
}

fn quiet(args: &[&str]) -> anyhow::Result<bool> {
    let status = Command::new(SYSTEMCTL)
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
    let out = Command::new(SYSTEMCTL)
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
        let _ = systemctl(&["disable", "--now", UNIT]);
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

    #[test]
    fn unit_runs_absolute_exe_with_pinned_dirs() {
        let u = unit(
            Path::new("/opt/collie/bin/collied"),
            Some(Path::new("/home/me/a b\"$HOME%h\\.toml")),
            &[("XDG_DATA_HOME", PathBuf::from("/home/me/.local/share"))],
        )
        .unwrap();
        assert!(u.contains(
            "ExecStart=\"/opt/collie/bin/collied\" \"--config\" \"/home/me/a b\\\"$$HOME%%h\\\\.toml\" \"run\"\n"
        ));
        assert!(u.contains("Environment=\"XDG_DATA_HOME=/home/me/.local/share\"\n"));
        assert!(u.contains("Restart=always\n"));
        assert!(u.contains("UMask=0077\n"));
        assert!(u.contains("WantedBy=default.target\n"));
        assert!(!u.contains("User=") && !u.contains("ListenStream"));
    }

    #[test]
    fn control_characters_are_refused() {
        assert!(unit(Path::new("/x\n[Service]"), None, &[]).is_err());
        assert!(quote("a\tb").is_err());
    }
}
