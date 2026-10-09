use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::config::{self, ApnsConfig, ApnsKey};
use crate::control::StatusInfo;
use crate::service::State;

pub const TEAM_ID: &str = "RM3UT3MMSR";
pub const BUNDLE_ID: &str = "dev.rbstp.collie";

#[allow(async_fn_in_trait)]
pub trait Host {
    async fn status(&mut self) -> Option<StatusInfo>;
    fn service(&mut self, started_ms: Option<u64>) -> anyhow::Result<State>;
    async fn login(&mut self) -> anyhow::Result<()>;
    fn unload_service(&mut self) -> anyhow::Result<()>;
    fn start_service(&mut self) -> anyhow::Result<()>;
    fn install_service(&mut self) -> anyhow::Result<()>;
    /// Up to 20 s for a daemon that is starting, for example after `just collied-install`.
    async fn settle(&mut self) -> Option<StatusInfo>;
    async fn wait_daemon(&mut self) -> anyhow::Result<StatusInfo>;
    async fn doctor(&mut self) -> anyhow::Result<bool>;
    fn apns_configured(&mut self) -> anyhow::Result<bool>;
    fn setup_apns(&mut self) -> anyhow::Result<bool>;
    async fn pair(&mut self) -> anyhow::Result<bool>;
    fn ask(&mut self, question: &str) -> anyhow::Result<bool>;
    fn say(&mut self, line: &str);
}

/// Each step checks its own done state, so a rerun resumes where the last one stopped.
/// Pairing still needs its own y/N on this machine: nothing here confirms a phone.
pub async fn run(host: &mut impl Host) -> anyhow::Result<bool> {
    let running = |s: &Option<StatusInfo>| s.as_ref().is_some_and(|s| s.backend_state == "Running");
    let mut status = host.status().await;
    let mut loaded = false;
    if !running(&status) {
        loaded = !matches!(host.service(None)?, State::Missing | State::Stopped);
        if loaded && status.is_none() {
            status = host.settle().await;
        }
    }
    if running(&status) {
        host.say("login: done");
    } else {
        // A crash-looping or logged-out daemon holds the node lock or the control socket.
        if loaded {
            host.say("service: stopping collied for the login; setup starts it again (collied start if you cancel)");
            host.unload_service()?;
        }
        host.say("login:");
        if let Err(e) = host.login().await {
            if loaded && let Err(s) = host.start_service() {
                host.say(&format!(
                    "could not start collied again ({s:#}): run collied start"
                ));
            }
            return Err(e);
        }
        status = None;
    }

    let started_ms = status.as_ref().and_then(|s| s.started_ms);
    let status = match (host.service(started_ms)?, status) {
        (State::Current, Some(s)) => {
            host.say("service: up to date");
            s
        }
        (State::Missing | State::Stopped, Some(_)) => anyhow::bail!(
            "collied is running outside the service (collied run?): stop it, then run collied setup again"
        ),
        (state, _) => {
            host.say(&match state {
                State::Outdated(why) => format!("service: {why}; installing it again"),
                State::Stopped => "service: stopped; installing it again".to_owned(),
                _ => "service: installing".to_owned(),
            });
            host.install_service()?;
            host.wait_daemon().await?
        }
    };

    host.say("doctor:");
    if !host.doctor().await? {
        host.say("Fix the fail lines above, then run collied setup again.");
        return Ok(false);
    }
    if status.user_peers == Some(0) {
        host.say(&format!(
            "\nNo device of yours can reach this machine yet. If your phone is signed in to Tailscale, the policy lacks the grant. {}",
            crate::daemon::policy_help(status.port)
        ));
    }

    if host.apns_configured()? {
        host.say("push notifications: configured");
    } else if host
        .ask("Set up push notifications now (needs this machine's APNs key, a .p8)? [y/N]")?
        && host.setup_apns()?
    {
        // The daemon reads [apns] only when it starts.
        host.install_service()?;
        host.wait_daemon().await?;
    }

    let question = match status.peers {
        0 => "Pair a phone now? [y/N]".to_owned(),
        n => format!("{n} phone(s) paired. Pair another, or pair one again after an update? [y/N]"),
    };
    if host.ask(&question)? {
        return host.pair().await;
    }
    host.say("Pair a phone later with collied pair.");
    Ok(true)
}

pub fn apns_section(key_id: &str, team_id: &str, bundle_id: &str) -> anyhow::Result<String> {
    let cfg = ApnsConfig {
        key: ApnsKey::platform_default(),
        key_id: key_id.to_owned(),
        team_id: team_id.to_owned(),
        bundle_id: bundle_id.to_owned(),
    };
    crate::push::check_ids(&cfg)?;
    Ok(format!(
        "[apns]\nkey = \"{}\"\nkey_id = \"{key_id}\"\nteam_id = \"{team_id}\"\nbundle_id = \"{bundle_id}\"\n",
        cfg.key.config_value().unwrap_or_default()
    ))
}

/// Adds `section` to the config for `import`, and puts the old text back when it fails so
/// the next run does not stop at a doctor fail.
pub fn with_apns(
    config_path: &Path,
    section: &str,
    import: impl FnOnce() -> anyhow::Result<bool>,
) -> anyhow::Result<bool> {
    let old = match std::fs::read_to_string(config_path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("read {}", config_path.display())),
    };
    let new = match old.as_deref() {
        None | Some("") => section.to_owned(),
        Some(t) if t.ends_with('\n') => format!("{t}\n{section}"),
        Some(t) => format!("{t}\n\n{section}"),
    };
    anyhow::ensure!(
        config::parse(&new).is_ok_and(|c| c.apns.is_some()),
        "{}: could not add [apns]; add it by hand (docs/release.md)",
        config_path.display()
    );
    match old {
        Some(_) => config::rewrite(config_path, &new)?,
        None => create_private(config_path, &new)?,
    }
    let result = import();
    if result.is_err() {
        let restored = match old.as_deref() {
            Some(t) => config::rewrite(config_path, t),
            None => std::fs::remove_file(config_path).map_err(Into::into),
        };
        if let Err(e) = restored {
            eprintln!(
                "could not restore {}: {e:#}; remove its [apns] section",
                config_path.display()
            );
        }
    }
    result
}

fn create_private(path: &Path, text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

/// A path as typed or dropped into a terminal: quoted, with escaped spaces, or under `~/`.
pub fn typed_path(typed: &str) -> anyhow::Result<PathBuf> {
    let t = typed.trim();
    let t = ["'", "\""]
        .iter()
        .find_map(|q| t.strip_prefix(q)?.strip_suffix(q))
        .map_or_else(|| t.replace("\\ ", " "), str::to_owned);
    Ok(match t.strip_prefix("~/") {
        Some(rest) => config::home_dir()?.join(rest),
        None => PathBuf::from(t),
    })
}

/// The key ID in an `AuthKey_<KEY_ID>.p8` file name.
pub fn key_id_of(path: &Path) -> Option<&str> {
    path.file_name()?
        .to_str()?
        .strip_prefix("AuthKey_")?
        .strip_suffix(".p8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        log: Vec<String>,
        status: Option<StatusInfo>,
        service: Vec<State>,
        doctor: bool,
        apns: bool,
        answers: Vec<bool>,
        said: String,
        settled: Option<StatusInfo>,
        login_fails: bool,
    }

    fn info(peers: usize, user_peers: Option<usize>) -> StatusInfo {
        StatusInfo {
            pid: 1,
            backend_state: "Running".into(),
            dns_name: "m".into(),
            node_id: "n".into(),
            port: 8457,
            sessions: 0,
            peers,
            herdr_version: None,
            tags: None,
            flock: None,
            flock_too_large: false,
            started_ms: Some(1),
            user_peers,
        }
    }

    impl Host for Fake {
        async fn status(&mut self) -> Option<StatusInfo> {
            self.log.push("status".into());
            self.status.clone()
        }
        fn service(&mut self, started_ms: Option<u64>) -> anyhow::Result<State> {
            self.log.push(format!("service {started_ms:?}"));
            Ok(self.service.remove(0))
        }
        async fn login(&mut self) -> anyhow::Result<()> {
            self.log.push("login".into());
            anyhow::ensure!(!self.login_fails, "login failed");
            Ok(())
        }
        fn unload_service(&mut self) -> anyhow::Result<()> {
            self.log.push("unload".into());
            Ok(())
        }
        fn start_service(&mut self) -> anyhow::Result<()> {
            self.log.push("start".into());
            Ok(())
        }
        async fn settle(&mut self) -> Option<StatusInfo> {
            self.log.push("settle".into());
            self.settled.clone()
        }
        fn install_service(&mut self) -> anyhow::Result<()> {
            self.log.push("install".into());
            Ok(())
        }
        async fn wait_daemon(&mut self) -> anyhow::Result<StatusInfo> {
            self.log.push("wait".into());
            Ok(self.status.clone().unwrap_or_else(|| info(0, Some(1))))
        }
        async fn doctor(&mut self) -> anyhow::Result<bool> {
            self.log.push("doctor".into());
            Ok(self.doctor)
        }
        fn apns_configured(&mut self) -> anyhow::Result<bool> {
            Ok(self.apns)
        }
        fn setup_apns(&mut self) -> anyhow::Result<bool> {
            self.log.push("apns".into());
            Ok(true)
        }
        async fn pair(&mut self) -> anyhow::Result<bool> {
            self.log.push("pair".into());
            Ok(true)
        }
        fn ask(&mut self, question: &str) -> anyhow::Result<bool> {
            self.log
                .push(format!("ask {}", question.split(' ').next().unwrap_or("")));
            Ok(self.answers.remove(0))
        }
        fn say(&mut self, line: &str) {
            self.said.push_str(line);
            self.said.push('\n');
        }
    }

    async fn run_fake(mut fake: Fake) -> (anyhow::Result<bool>, Fake) {
        let r = run(&mut fake).await;
        (r, fake)
    }

    #[tokio::test]
    async fn fresh_machine_runs_every_step() {
        let (r, f) = run_fake(Fake {
            service: vec![State::Missing, State::Missing],
            doctor: true,
            answers: vec![true, true],
            ..Fake::default()
        })
        .await;
        assert!(r.unwrap());
        assert_eq!(
            f.log,
            [
                "status",
                "service None",
                "login",
                "service None",
                "install",
                "wait",
                "doctor",
                "ask Set",
                "apns",
                "install",
                "wait",
                "ask Pair",
                "pair"
            ]
        );
    }

    #[tokio::test]
    async fn set_up_machine_only_checks_and_offers_pairing() {
        let (r, f) = run_fake(Fake {
            status: Some(info(1, Some(2))),
            service: vec![State::Current],
            doctor: true,
            apns: true,
            answers: vec![false],
            ..Fake::default()
        })
        .await;
        assert!(r.unwrap());
        assert_eq!(f.log, ["status", "service Some(1)", "doctor", "ask 1"]);
        assert!(!f.said.contains("tagOwners"));
    }

    #[tokio::test]
    async fn stops_a_crash_looping_service_before_login() {
        let (_, f) = run_fake(Fake {
            service: vec![
                State::Outdated("the daemon is not answering"),
                State::Stopped,
            ],
            doctor: true,
            apns: true,
            answers: vec![false],
            ..Fake::default()
        })
        .await;
        assert_eq!(
            f.log[..7],
            [
                "status",
                "service None",
                "settle",
                "unload",
                "login",
                "service None",
                "install"
            ]
        );
    }

    #[tokio::test]
    async fn waits_for_a_restarting_daemon_instead_of_logging_in() {
        let (r, f) = run_fake(Fake {
            settled: Some(info(1, Some(1))),
            service: vec![
                State::Outdated("the daemon is not answering"),
                State::Current,
            ],
            doctor: true,
            apns: true,
            answers: vec![false],
            ..Fake::default()
        })
        .await;
        assert!(r.unwrap());
        assert_eq!(
            f.log,
            [
                "status",
                "service None",
                "settle",
                "service Some(1)",
                "doctor",
                "ask 1"
            ]
        );
    }

    #[tokio::test]
    async fn failed_login_starts_the_service_again() {
        let (r, f) = run_fake(Fake {
            service: vec![State::Outdated("the daemon is not answering")],
            login_fails: true,
            ..Fake::default()
        })
        .await;
        assert!(r.is_err());
        assert_eq!(f.log[3..], ["unload", "login", "start"]);
    }

    #[tokio::test]
    async fn leaves_a_stopped_service_stopped_when_login_fails() {
        let (r, f) = run_fake(Fake {
            service: vec![State::Stopped],
            login_fails: true,
            ..Fake::default()
        })
        .await;
        assert!(r.is_err());
        assert_eq!(f.log, ["status", "service None", "login"]);
    }

    #[tokio::test]
    async fn newer_binary_reinstalls_without_login() {
        let (_, f) = run_fake(Fake {
            status: Some(info(1, Some(1))),
            service: vec![State::Outdated("a newer collied binary is installed")],
            doctor: true,
            apns: true,
            answers: vec![false],
            ..Fake::default()
        })
        .await;
        assert_eq!(f.log[..4], ["status", "service Some(1)", "install", "wait"]);
        assert!(!f.log.contains(&"login".to_owned()));
    }

    #[tokio::test]
    async fn doctor_failure_stops_before_apns_and_pairing() {
        let (r, f) = run_fake(Fake {
            status: Some(info(0, Some(1))),
            service: vec![State::Current],
            ..Fake::default()
        })
        .await;
        assert!(!r.unwrap());
        assert_eq!(f.log.last().unwrap(), "doctor");
        assert!(f.said.contains("run collied setup again"));
    }

    #[tokio::test]
    async fn declined_steps_are_skipped() {
        let (r, f) = run_fake(Fake {
            status: Some(info(0, Some(1))),
            service: vec![State::Current],
            doctor: true,
            answers: vec![false, false],
            ..Fake::default()
        })
        .await;
        assert!(r.unwrap());
        assert_eq!(f.log[3..], ["ask Set", "ask Pair"]);
        assert!(f.said.contains("collied pair"));
    }

    #[tokio::test]
    async fn prints_the_grant_when_no_device_can_reach_the_node() {
        let (_, f) = run_fake(Fake {
            status: Some(info(0, Some(0))),
            service: vec![State::Current],
            doctor: true,
            apns: true,
            answers: vec![false],
            ..Fake::default()
        })
        .await;
        assert!(f.said.contains(&crate::daemon::policy_snippet(8457)));
    }

    #[tokio::test]
    async fn refuses_a_daemon_outside_the_service() {
        for service in [State::Missing, State::Stopped] {
            let (r, f) = run_fake(Fake {
                status: Some(info(0, Some(1))),
                service: vec![service],
                ..Fake::default()
            })
            .await;
            assert!(r.is_err());
            assert!(!f.log.contains(&"install".to_owned()));
        }
    }

    #[test]
    fn apns_section_parses_and_rejects_bad_ids() {
        let s = apns_section("ABCDE12345", TEAM_ID, BUNDLE_ID).unwrap();
        let apns = config::parse(&s).unwrap().apns.unwrap();
        assert_eq!(apns.key, ApnsKey::platform_default());
        assert_eq!(
            (apns.key_id.as_str(), apns.bundle_id.as_str()),
            ("ABCDE12345", BUNDLE_ID)
        );
        assert!(apns_section("ABCDE12345\"\nx = 1", TEAM_ID, BUNDLE_ID).is_err());
        assert!(apns_section("ABCDE12345", TEAM_ID, "a\"b").is_err());
    }

    #[test]
    fn failed_import_restores_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("collied.toml");
        let section = apns_section("ABCDE12345", TEAM_ID, BUNDLE_ID).unwrap();

        let r = with_apns(&path, &section, || {
            assert!(std::fs::read_to_string(&path).unwrap().contains("[apns]"));
            anyhow::bail!("import failed")
        });
        assert!(r.is_err() && !path.exists());

        let old = "[tailnet]\nport = 8457";
        std::fs::write(&path, old).unwrap();
        assert!(with_apns(&path, &section, || anyhow::bail!("no")).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), old);

        assert!(with_apns(&path, &section, || Ok(true)).unwrap());
        let config = config::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(config.apns.is_some() && config.tailnet.port == 8457);
    }

    #[test]
    fn new_config_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("collied.toml");
        let section = apns_section("ABCDE12345", TEAM_ID, BUNDLE_ID).unwrap();
        assert!(with_apns(&path, &section, || Ok(true)).unwrap());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn reads_typed_paths_and_key_ids() {
        assert_eq!(
            typed_path("'/tmp/a b/AuthKey_ABCDE12345.p8'\n").unwrap(),
            PathBuf::from("/tmp/a b/AuthKey_ABCDE12345.p8")
        );
        assert_eq!(
            typed_path("/tmp/a\\ b/k.p8 ").unwrap(),
            PathBuf::from("/tmp/a b/k.p8")
        );
        assert_eq!(
            key_id_of(Path::new("/x/AuthKey_ABCDE12345.p8")),
            Some("ABCDE12345")
        );
        assert_eq!(key_id_of(Path::new("/x/key.p8")), None);
    }
}
