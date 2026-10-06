use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use protocol::AgentKind;
use serde::Deserialize;

pub use protocol::DEFAULT_PORT;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub herdr: HerdrConfig,
    #[serde(default)]
    pub tailnet: TailnetConfig,
    #[serde(default)]
    pub tasks: TasksConfig,
    #[serde(default)]
    pub terminals: TerminalsConfig,
    pub apns: Option<ApnsConfig>,
}

/// Plain shell panes from the phone. Off unless set here; read at start only.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalsConfig {
    pub enabled: bool,
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

/// `roots` defaults to the user's home directory.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TasksConfig {
    pub agents: Vec<AgentKind>,
    pub roots: Option<Vec<PathBuf>>,
}

impl Default for TasksConfig {
    fn default() -> Self {
        Self {
            agents: ["claude", "codex", "copilot"]
                .into_iter()
                .map(|k| AgentKind::new(k).expect("valid agent kind"))
                .collect(),
            roots: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApnsKey {
    /// macOS: the login Keychain item.
    Keychain,
    /// Linux: a systemd user credential in `<data dir>/apns/<key_id>.cred`.
    SystemdCreds,
    /// The `.p8` file named by `key_path`: legacy on macOS, the fallback on Linux.
    File(PathBuf),
}

impl ApnsKey {
    pub const fn platform_default() -> Self {
        if cfg!(target_os = "macos") {
            ApnsKey::Keychain
        } else {
            ApnsKey::SystemdCreds
        }
    }

    pub fn config_value(&self) -> Option<&'static str> {
        match self {
            ApnsKey::Keychain => Some("keychain"),
            ApnsKey::SystemdCreds => Some("systemd-creds"),
            ApnsKey::File(_) => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "RawApnsConfig")]
pub struct ApnsConfig {
    pub key: ApnsKey,
    pub key_id: String,
    pub team_id: String,
    pub bundle_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum KeyStore {
    Keychain,
    SystemdCreds,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawApnsConfig {
    key: Option<KeyStore>,
    key_path: Option<PathBuf>,
    key_id: String,
    team_id: String,
    bundle_id: String,
}

impl TryFrom<RawApnsConfig> for ApnsConfig {
    type Error = &'static str;

    fn try_from(r: RawApnsConfig) -> Result<Self, Self::Error> {
        let key = match (r.key, r.key_path) {
            (Some(_), Some(_)) => return Err("set either key or key_path, not both"),
            (None, Some(path)) => ApnsKey::File(path),
            (None, None) => ApnsKey::platform_default(),
            (Some(KeyStore::Keychain), None) if cfg!(target_os = "macos") => ApnsKey::Keychain,
            (Some(KeyStore::SystemdCreds), None) if cfg!(target_os = "linux") => {
                ApnsKey::SystemdCreds
            }
            (Some(KeyStore::Keychain), None) => {
                return Err("key = \"keychain\" is macOS only: use key = \"systemd-creds\"");
            }
            (Some(KeyStore::SystemdCreds), None) => {
                return Err("key = \"systemd-creds\" is Linux only: use key = \"keychain\"");
            }
        };
        Ok(Self {
            key,
            key_id: r.key_id,
            team_id: r.team_id,
            bundle_id: r.bundle_id,
        })
    }
}

pub const CONFIG_FILE: &str = "collied.toml";
pub const TSNET_DIR: &str = "tsnet";
pub const PEERS_FILE: &str = "peers.json";
pub const PUSH_FILE: &str = "push.json";
pub const STATUS_FILE: &str = "status.json";
pub const TLS_KEY_FILE: &str = "tls-key.json";
pub const NODE_LOCK: &str = "node.lock";
pub const PEERS_LOCK: &str = "peers.lock";
pub const AUDIT_FILE: &str = "audit.log";
pub const CONTROL_SOCKET: &str = "control.sock";
pub const APNS_DIR: &str = "apns";

pub fn home_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("HOME").context("HOME is not set")?,
    ))
}

#[cfg(target_os = "macos")]
pub fn data_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join("Library/Application Support/collie"))
}

/// No spaces in the path, so an agent reads it back cleanly from a prompt.
#[cfg(target_os = "macos")]
pub fn attachments_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join("Library/Caches/dev.rbstp.collied/attachments"))
}

#[cfg(target_os = "linux")]
pub fn data_dir() -> Result<PathBuf> {
    Ok(xdg_dir("XDG_DATA_HOME", ".local/share")?.join("collie"))
}

#[cfg(target_os = "linux")]
pub fn attachments_dir() -> Result<PathBuf> {
    Ok(xdg_dir("XDG_CACHE_HOME", ".cache")?.join("collied/attachments"))
}

/// The XDG base directory in `var`, or `~/<fallback>` when it is unset, empty or
/// relative (the spec says to ignore a relative value).
#[cfg(target_os = "linux")]
pub fn xdg_dir(var: &str, fallback: &str) -> Result<PathBuf> {
    match std::env::var_os(var).map(PathBuf::from) {
        Some(p) if p.is_absolute() => Ok(p),
        _ => Ok(home_dir()?.join(fallback)),
    }
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

/// `text` with the `[apns]` `key_path` line replaced by `key = "keychain"`, or None
/// unless there is exactly one such line and the result parses to the same `[apns]` with
/// the key in the Keychain.
pub fn use_keychain(text: &str) -> Option<String> {
    set_apns_key(text, &ApnsKey::Keychain, false)
}

/// `text` with the `[apns]` key location set to `key`: its one `key` or `key_path` line
/// replaced, or, with `insert`, a line added under `[apns]` when it has neither. None
/// unless the result parses to the same `[apns]` with `key`.
pub fn set_apns_key(text: &str, key: &ApnsKey, insert: bool) -> Option<String> {
    let line_for = |indent: &str, eol: &str| match key {
        ApnsKey::File(path) => {
            let value = toml::Value::String(path.to_str()?.to_owned());
            Some(format!("{indent}key_path = {value}{eol}"))
        }
        k => Some(format!("{indent}key = \"{}\"{eol}", k.config_value()?)),
    };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut table = "";
    let mut header = None;
    let mut hits = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            table = t.split('#').next().unwrap_or_default().trim();
            if table == "[apns]" {
                header = Some(i);
            }
        } else if table == "[apns]"
            && ["key_path", "key"].iter().any(|k| {
                t.strip_prefix(k)
                    .is_some_and(|rest| rest.trim_start().starts_with('='))
            })
        {
            hits.push(i);
        }
    }
    let (at, replaced) = match hits[..] {
        [i] => {
            let line = lines[i];
            let indent = &line[..line.len() - line.trim_start().len()];
            let eol = &line[line.trim_end().len()..];
            (i, line_for(indent, eol)?)
        }
        [] if insert => {
            let h = header?;
            let eol = if lines[h].ends_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            (h, format!("{}{}", lines[h], line_for("", eol)?))
        }
        _ => return None,
    };
    let mut out = String::with_capacity(text.len() + replaced.len());
    for (j, l) in lines.iter().enumerate() {
        if j == at {
            if hits.is_empty() && !l.ends_with('\n') {
                return None;
            }
            out.push_str(&replaced);
        } else {
            out.push_str(l);
        }
    }
    let before = parse(text).ok()?.apns?;
    let after = parse(&out).ok()?.apns?;
    (after.key == *key
        && before.key != *key
        && after.key_id == before.key_id
        && after.team_id == before.team_id
        && after.bundle_id == before.bundle_id)
        .then_some(out)
}

/// Atomic replace keeping the file's mode; refuses a symlink.
pub fn rewrite(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let meta = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(meta.is_file(), "{} is not a regular file", path.display());
    let tmp = path.with_extension("toml.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.set_permissions(std::fs::Permissions::from_mode(
        meta.permissions().mode() & 0o777,
    ))?;
    file.sync_all()?;
    std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))
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
        let agents: Vec<&str> = c.tasks.agents.iter().map(AgentKind::as_str).collect();
        assert_eq!(agents, ["claude", "codex", "copilot"]);
        assert_eq!(c.tasks.roots, None);
        assert!(!c.terminals.enabled, "terminals are off by default");
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
            [tasks]
            agents = ["claude", "codex"]
            roots = ["/Users/me/src"]
            [terminals]
            enabled = true
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
        assert_eq!(c.tasks.agents.len(), 2);
        assert_eq!(c.tasks.roots, Some(vec![PathBuf::from("/Users/me/src")]));
        assert!(c.terminals.enabled);
        assert_eq!(c.apns.unwrap().key, ApnsKey::File(PathBuf::from("/k.p8")));
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
        assert!(parse("[tasks]\nagent = []").is_err());
        assert!(parse("[tasks]\nagents = [\"Claude\"]").is_err());
        assert!(parse("[terminals]\nenable = true").is_err());
        assert!(parse("[terminals]\nenabled = \"yes\"").is_err());
        assert!(!parse("[terminals]\n").unwrap().terminals.enabled);
        assert!(parse("[apns]\nkey_path = \"/k\"\nkey_id = \"a\"\nteam_id = \"b\"\nbundle_id = \"c\"\nx = 1").is_err());
    }

    #[test]
    fn rejects_incomplete_apns() {
        assert!(parse("[apns]\nkey_path = \"/k\"").is_err());
    }

    const IDS: &str =
        "key_id = \"6Y7FRZ845U\"\nteam_id = \"RM3UT3MMSR\"\nbundle_id = \"dev.rbstp.collie\"\n";

    #[test]
    fn apns_key_modes() {
        let key =
            |extra: &str| parse(&format!("[apns]\n{extra}{IDS}")).map(|c| c.apns.unwrap().key);
        assert_eq!(key("").unwrap(), ApnsKey::platform_default());
        if cfg!(target_os = "macos") {
            assert_eq!(key("key = \"keychain\"\n").unwrap(), ApnsKey::Keychain);
            let linux = key("key = \"systemd-creds\"\n").unwrap_err();
            assert!(linux.message().contains("Linux only"), "{linux}");
        } else {
            assert_eq!(key("").unwrap(), ApnsKey::SystemdCreds);
            assert_eq!(
                key("key = \"systemd-creds\"\n").unwrap(),
                ApnsKey::SystemdCreds
            );
            let mac = key("key = \"keychain\"\n").unwrap_err();
            assert!(mac.message().contains("macOS only"), "{mac}");
        }
        assert_eq!(
            key("key_path = \"/k.p8\"\n").unwrap(),
            ApnsKey::File(PathBuf::from("/k.p8"))
        );
        for store in ["keychain", "systemd-creds"] {
            let both = key(&format!("key = \"{store}\"\nkey_path = \"/k.p8\"\n")).unwrap_err();
            assert!(both.message().contains("not both"), "{both}");
        }
        assert!(key("key = \"file\"\n").is_err());
        assert!(key("key = \"/k.p8\"\n").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn switches_to_keychain() {
        let text = format!(
            "# mine\n[tailnet]\nport = 9000\n\n[apns]\n  key_path = \"/k.p8\" # old\r\n{IDS}"
        );
        let out = use_keychain(&text).unwrap();
        assert_eq!(
            out,
            format!("# mine\n[tailnet]\nport = 9000\n\n[apns]\n  key = \"keychain\"\r\n{IDS}")
        );
        let c = parse(&out).unwrap();
        assert_eq!(c.tailnet.port, 9000);
        assert_eq!(c.apns.unwrap().key, ApnsKey::Keychain);

        assert_eq!(
            use_keychain(&format!("[apns]\n{IDS}")),
            None,
            "already keychain"
        );
        assert_eq!(
            use_keychain(&format!("[other]\nkey_path = 1\n[apns]\n{IDS}")),
            None
        );
        assert_eq!(
            use_keychain(
                "apns = { key_path = \"/k\", key_id = \"6Y7FRZ845U\", team_id = \"RM3UT3MMSR\", bundle_id = \"b\" }\n"
            ),
            None
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn switches_between_credential_and_file() {
        let text = format!(
            "# mine\n[tailnet]\nport = 9000\n\n[apns]\n  key_path = \"/k.p8\" # old\r\n{IDS}"
        );
        let out = set_apns_key(&text, &ApnsKey::SystemdCreds, true).unwrap();
        assert_eq!(
            out,
            format!("# mine\n[tailnet]\nport = 9000\n\n[apns]\n  key = \"systemd-creds\"\r\n{IDS}")
        );
        assert_eq!(
            parse(&out).unwrap().apns.unwrap().key,
            ApnsKey::SystemdCreds
        );

        // Already the default: nothing to change, either way.
        let bare = format!("[apns]\n{IDS}");
        assert_eq!(set_apns_key(&bare, &ApnsKey::SystemdCreds, true), None);

        // Fallback: a key_path added under [apns], or replacing key = "systemd-creds".
        let file = ApnsKey::File(PathBuf::from(
            "/home/me/.local/share/collie/apns/AuthKey_6Y7FRZ845U.p8",
        ));
        let out = set_apns_key(&bare, &file, true).unwrap();
        assert_eq!(
            out,
            format!(
                "[apns]\nkey_path = \"/home/me/.local/share/collie/apns/AuthKey_6Y7FRZ845U.p8\"\n{IDS}"
            )
        );
        assert_eq!(parse(&out).unwrap().apns.unwrap().key, file);
        assert_eq!(set_apns_key(&bare, &file, false), None);
        let creds = format!("[apns]\nkey = \"systemd-creds\"\n{IDS}");
        assert_eq!(
            parse(&set_apns_key(&creds, &file, true).unwrap())
                .unwrap()
                .apns
                .unwrap()
                .key,
            file
        );
        let quoted = ApnsKey::File(PathBuf::from("/a \"b\"\\c.p8"));
        assert_eq!(
            parse(&set_apns_key(&bare, &quoted, true).unwrap())
                .unwrap()
                .apns
                .unwrap()
                .key,
            quoted
        );

        // Never two key lines, never an inline table, never a header without a newline.
        let two = format!("[apns]\nkey_path = \"/a\"\nkey_path = \"/b\"\n{IDS}");
        assert_eq!(set_apns_key(&two, &ApnsKey::SystemdCreds, true), None);
        assert_eq!(
            set_apns_key(
                "apns = { key_path = \"/k\", key_id = \"6Y7FRZ845U\", team_id = \"RM3UT3MMSR\", bundle_id = \"b\" }\n",
                &ApnsKey::SystemdCreds,
                true
            ),
            None
        );
        assert_eq!(set_apns_key(&format!("{IDS}[apns]"), &file, true), None);
    }

    #[test]
    fn rewrite_keeps_mode_and_refuses_links() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("collied.toml");
        std::fs::write(&path, "a").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        rewrite(&path, "b").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "b");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let link = dir.path().join("link.toml");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(rewrite(&link, "c").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "b");
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
