use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

pub const LABEL: &str = "dev.rbstp.collied";
pub const STDOUT_LOG: &str = "collied.out.log";
pub const STDERR_LOG: &str = "collied.err.log";

fn plist_path() -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

fn domain() -> String {
    format!("gui/{}", rustix::process::getuid().as_raw())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn plist(exe: &Path, config: Option<&Path>, data_dir: &Path) -> String {
    let mut args = vec![exe.display().to_string()];
    if let Some(c) = config {
        args.push("--config".into());
        args.push(c.display().to_string());
    }
    args.push("run".into());
    let args: String = args
        .iter()
        .map(|a| format!("        <string>{}</string>\n", xml_escape(a)))
        .collect();
    let out = xml_escape(&data_dir.join(STDOUT_LOG).display().to_string());
    let err = xml_escape(&data_dir.join(STDERR_LOG).display().to_string());
    // Umask 63 (077) keeps every file the daemon creates private.
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
{args}    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>Umask</key>
    <integer>63</integer>
    <key>StandardOutPath</key>
    <string>{out}</string>
    <key>StandardErrorPath</key>
    <string>{err}</string>
</dict>
</plist>
"#
    )
}

fn launchctl(args: &[&str]) -> anyhow::Result<bool> {
    let status = Command::new("/bin/launchctl")
        .args(args)
        .status()
        .context("run launchctl")?;
    Ok(status.success())
}

pub fn install(config: Option<&Path>, data_dir: &Path) -> anyhow::Result<()> {
    crate::ensure_private_dir(data_dir)?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let config = config.map(Path::canonicalize).transpose()?;
    for log in [STDOUT_LOG, STDERR_LOG] {
        let path = data_dir.join(log);
        OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&path)?
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    let path = plist_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let target = format!("{}/{LABEL}", domain());
    let _ = launchctl(&["bootout", &target]);
    let tmp = path.with_extension("plist.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&tmp)?;
    file.write_all(plist(&exe, config.as_deref(), data_dir).as_bytes())?;
    file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
    file.sync_all()?;
    std::fs::rename(&tmp, &path)?;
    anyhow::ensure!(
        launchctl(&["bootstrap", &domain(), &path.display().to_string()])?,
        "launchctl bootstrap {} failed",
        path.display()
    );
    println!("installed {} ({})", path.display(), exe.display());
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let path = plist_path()?;
    let target = format!("{}/{LABEL}", domain());
    let unloaded = launchctl(&["bootout", &target])?;
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    println!(
        "{} {}",
        if unloaded {
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
    fn plist_runs_absolute_exe() {
        let p = plist(
            Path::new("/opt/collie/bin/collied"),
            Some(Path::new("/Users/me/a&b.toml")),
            Path::new("/Users/me/Library/Application Support/collie"),
        );
        assert!(p.contains("<string>/opt/collie/bin/collied</string>"));
        assert!(p.contains("<string>/Users/me/a&amp;b.toml</string>"));
        assert!(p.contains("<string>run</string>"));
        assert!(p.contains("<key>KeepAlive</key>\n    <true/>"));
        assert!(p.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(p.contains("Application Support/collie/collied.err.log"));
        assert!(!p.contains("UserName") && !p.contains("Sockets"));
    }
}
