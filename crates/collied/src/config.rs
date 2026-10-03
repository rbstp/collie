use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

pub use protocol::DEFAULT_PORT;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub herdr: HerdrConfig,
    #[serde(default)]
    pub tailnet: TailnetConfig,
    pub apns: Option<ApnsConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HerdrConfig {
    pub session: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TailnetConfig {
    pub hostname: Option<String>,
    pub port: u16,
    pub owner_user_id: Option<i64>,
}

impl Default for TailnetConfig {
    fn default() -> Self {
        Self {
            hostname: None,
            port: DEFAULT_PORT,
            owner_user_id: None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApnsConfig {
    pub key_path: PathBuf,
    pub key_id: String,
    pub team_id: String,
    pub bundle_id: String,
}

pub const CONFIG_FILE: &str = "collied.toml";
pub const TSNET_DIR: &str = "tsnet";
pub const PEERS_FILE: &str = "peers.json";
pub const NODE_LOCK: &str = "node.lock";
pub const PEERS_LOCK: &str = "peers.lock";
pub const AUDIT_FILE: &str = "audit.log";
pub const CONTROL_SOCKET: &str = "control.sock";

pub fn data_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Application Support/collie"))
}

pub fn parse(text: &str) -> Result<Config, toml::de::Error> {
    toml::from_str(text)
}

pub fn load(path: &Path, explicit: bool) -> Result<Option<Config>> {
    match std::fs::read_to_string(path) {
        Ok(text) => match parse(&text) {
            Ok(c) => Ok(Some(c)),
            Err(e) => {
                let line = e
                    .span()
                    .map_or(0, |s| text[..s.start].matches('\n').count() + 1);
                anyhow::bail!("{}:{line}: {}", path.display(), e.message().trim())
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

impl TailnetConfig {
    pub fn hostname(&self) -> String {
        match &self.hostname {
            Some(h) => h.clone(),
            None => default_hostname(&machine_name()),
        }
    }
}

pub fn machine_name() -> String {
    let local = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "LocalHostName"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty());
    local.unwrap_or_else(|| {
        let node = rustix::system::uname()
            .nodename()
            .to_string_lossy()
            .into_owned();
        node.split('.').next().unwrap_or_default().to_owned()
    })
}

fn default_hostname(name: &str) -> String {
    let short = name.split('.').next().unwrap_or_default();
    let label: String = short
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let mut name = format!("collie-{}", label.trim_matches('-'));
    name.truncate(63);
    name.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_defaults() {
        let c = parse("").unwrap();
        assert_eq!(c.herdr.session, None);
        assert_eq!(c.tailnet.port, DEFAULT_PORT);
        assert_eq!(c.tailnet.hostname, None);
        assert_eq!(c.tailnet.owner_user_id, None);
        assert!(c.apns.is_none());
    }

    #[test]
    fn full() {
        let c = parse(
            r#"
            [herdr]
            session = "work"
            [tailnet]
            hostname = "mac"
            port = 9000
            owner_user_id = 123456789012
            [apns]
            key_path = "/k.p8"
            key_id = "ABC"
            team_id = "RM3UT3MMSR"
            bundle_id = "dev.rbstp.collie"
            "#,
        )
        .unwrap();
        assert_eq!(c.herdr.session.as_deref(), Some("work"));
        assert_eq!(c.tailnet.hostname(), "mac");
        assert_eq!(c.tailnet.port, 9000);
        assert_eq!(c.tailnet.owner_user_id, Some(123456789012));
        assert_eq!(c.apns.unwrap().key_path, PathBuf::from("/k.p8"));
    }

    #[test]
    fn partial_tailnet_keeps_default_port() {
        let c = parse("[tailnet]\nhostname = \"x\"\n").unwrap();
        assert_eq!(c.tailnet.port, DEFAULT_PORT);
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(parse("bogus = 1").is_err());
        assert!(parse("[herdr]\nsocket = \"/x\"").is_err());
        assert!(parse("[tailnet]\nprt = 1").is_err());
        assert!(parse("[apns]\nkey_path = \"/k\"\nkey_id = \"a\"\nteam_id = \"b\"\nbundle_id = \"c\"\nx = 1").is_err());
    }

    #[test]
    fn rejects_incomplete_apns() {
        assert!(parse("[apns]\nkey_path = \"/k\"").is_err());
    }

    #[test]
    fn load_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("collied.toml");
        assert!(load(&path, false).unwrap().is_none());
        assert!(load(&path, true).is_err());
        std::fs::write(&path, "[tailnet]\nport = 1\n").unwrap();
        assert_eq!(load(&path, true).unwrap().unwrap().tailnet.port, 1);
        std::fs::write(&path, "[tailnet]\nport = \"x\"\n").unwrap();
        assert!(load(&path, false).is_err());
    }

    #[test]
    fn hostname_sanitized() {
        assert_eq!(
            default_hostname("Richards-MacBook-Pro.local"),
            "collie-richards-macbook-pro"
        );
        assert_eq!(default_hostname("Mac Studio_2"), "collie-mac-studio-2");
        assert_eq!(default_hostname("-x-"), "collie-x");
        assert_eq!(default_hostname(&"a".repeat(100)).len(), 63);
    }
}
