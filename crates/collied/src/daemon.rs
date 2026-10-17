use std::os::fd::OwnedFd;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Context;
use tailnet::{BackendState, Node, Status};
use zeroize::Zeroizing;

use crate::config::{self, Config};
use crate::control;
use crate::server::{self, ServerConfig};
use crate::{approvals, herdr, push, terminal};

const POLL: Duration = Duration::from_millis(250);
const LOGIN_ERROR: &str = "You are logged out. The last login error was: ";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const RUN_START_TIMEOUT: Duration = Duration::from_secs(60);

/// The returned lock must outlive the node: two processes on one tsnet state dir
/// would fight over the node keys, and a starting daemon has no control socket yet.
fn mac_node(
    data_dir: &Path,
    config: &Config,
    auth_key: Option<Zeroizing<String>>,
) -> anyhow::Result<(Node, OwnedFd, bool)> {
    crate::ensure_private_dir(data_dir)?;
    let lock = crate::peers::lock(&data_dir.join(config::NODE_LOCK))
        .context("another collied (login or run) is using this data dir")?;
    let tsnet = data_dir.join(config::TSNET_DIR);
    crate::ensure_private_dir(&tsnet)?;
    #[cfg(all(debug_assertions, target_os = "macos"))]
    let test_key = tailnet::vm_test::load(data_dir)?;
    #[cfg(all(debug_assertions, target_os = "macos"))]
    let control_url = test_key
        .as_ref()
        .map(|_| tailnet::vm_test::CONTROL_URL.to_owned());
    #[cfg(not(all(debug_assertions, target_os = "macos")))]
    let control_url = None;
    #[cfg(all(debug_assertions, target_os = "macos"))]
    let auth_key = test_key.or(auth_key);
    let custom_control = control_url.is_some();
    let node = Node::new(&tailnet::Config {
        state_dir: tsnet,
        hostname: config.tailnet.hostname(),
        auth_key,
        control_url,
        advertise_tags: Vec::new(),
        log_to_stderr: false,
    })?;
    Ok((node, lock, custom_control))
}

pub async fn login(
    data_dir: &Path,
    config: &Config,
    auth_key: Option<Zeroizing<String>>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !control::daemon_running(&data_dir.join(config::CONTROL_SOCKET)).await,
        "collied is running; stop it (collied stop) before logging in"
    );
    #[cfg(target_os = "macos")]
    let has_auth_key = auth_key.is_some();
    let (node, _lock, _custom_control) = mac_node(data_dir, config, auth_key)?;
    #[cfg(target_os = "macos")]
    let mut browser = {
        use std::io::IsTerminal;
        LoginBrowser::new(
            std::io::stdin().is_terminal(),
            has_auth_key,
            _custom_control,
        )
    };
    node.start()?;
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    let mut shown_url = String::new();
    let mut shown_error = String::new();
    let mut warned_approval = false;
    loop {
        let st = node.status()?;
        match st.backend_state {
            BackendState::Running if st.self_node.is_some() => {
                print_identity(&st);
                return check_identity(&node, &st, data_dir, config.tailnet.owner_user_id)
                    .map(|_| ());
            }
            BackendState::NeedsMachineAuth if !warned_approval => {
                println!("This machine needs approval by a tailnet admin; waiting.");
                warned_approval = true;
            }
            _ => {}
        }
        if !st.auth_url.is_empty() && st.auth_url != shown_url {
            println!("Log in to Tailscale to add this machine to your tailnet:\n");
            println!("{}", crate::qr_text(&st.auth_url)?);
            println!("{}\n", st.auth_url);
            #[cfg(target_os = "macos")]
            if browser.open(&st.auth_url, |url| {
                std::process::Command::new("/usr/bin/open")
                    .arg(url)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
            }) == Some(false)
            {
                println!("Could not open the browser. Open the link above to continue.");
            }
            shown_url = st.auth_url;
        }
        if let Some(e) = login_error(st.health.as_deref().unwrap_or_default())
            && e != shown_error
        {
            println!("Tailscale login error: {e}");
            shown_error = e.to_owned();
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "login did not complete within 10 minutes (state {:?}); sign in to Collie with the same personal Tailscale account on this machine and the phone",
            st.backend_state
        );
        tokio::time::sleep(POLL).await;
    }
}

fn check_identity(
    node: &Node,
    status: &Status,
    data_dir: &Path,
    configured_owner: Option<i64>,
) -> anyhow::Result<i64> {
    let owner = node.owned_user(status).with_context(|| {
        if status.self_node.as_ref().is_some_and(|me| me.tags.as_ref().is_some_and(|tags| !tags.is_empty())) {
            format!(
                "This Collie node is tagged and is not converted automatically: stop collied, remove only Collie's node in the Tailscale admin console, delete {}, then run collied setup and pair phones again. Leave the system Tailscale VPN and its state unchanged",
                data_dir.join(config::TSNET_DIR).display()
            )
        } else {
            "Could not verify Collie's embedded Tailscale ownership. Check its sign-in and retry collied setup with the same personal account; do not delete node state for a transient status or whois failure".to_owned()
        }
    })?;
    anyhow::ensure!(
        configured_owner.is_none_or(|configured| configured == owner),
        "owner_user_id in collied.toml differs from Collie's embedded Tailscale user; not serving"
    );
    Ok(owner)
}

#[cfg(any(target_os = "macos", test))]
struct LoginBrowser {
    enabled: bool,
    attempted: bool,
}

#[cfg(any(target_os = "macos", test))]
impl LoginBrowser {
    fn new(interactive: bool, auth_key: bool, custom_control: bool) -> Self {
        Self {
            enabled: interactive && !auth_key && !custom_control,
            attempted: false,
        }
    }

    fn open(&mut self, url: &str, opener: impl FnOnce(&str) -> bool) -> Option<bool> {
        if !self.enabled || self.attempted {
            return None;
        }
        let uri = url.parse::<hyper::Uri>().ok()?;
        if uri.scheme_str() != Some("https")
            || !matches!(
                uri.authority().map(|a| a.as_str()),
                Some("login.tailscale.com" | "login.tailscale.com:443")
            )
        {
            return None;
        }
        self.attempted = true;
        Some(opener(url))
    }
}

fn login_error(health: &[String]) -> Option<&str> {
    health.iter().find_map(|h| h.strip_prefix(LOGIN_ERROR))
}

pub fn policy_snippet(port: u16) -> String {
    format!(
        r#"  "grants": [
    {{
      "src": ["autogroup:member"],
      "dst": ["autogroup:self"],
      "ip": ["tcp:{port}"],
    }},
  ],"#
    )
}

pub fn policy_help(port: u16) -> String {
    format!(
        "Sign in inside Collie on the phone and this machine with the same personal Tailscale account. No system VPN profile or standalone Tailscale app is needed. Existing policies that allow your own devices to connect need no changes. A restrictive policy may need an administrator to allow TCP {port} between your user-owned devices; merge these entries into the existing policy, never replace it. See docs/tailnet.md:\n\n{}\n",
        policy_snippet(port)
    )
}

fn print_identity(st: &Status) {
    let Some(me) = &st.self_node else { return };
    println!("Logged in.");
    println!("  name:      {}", me.dns_name.trim_end_matches('.'));
    println!("  stable id: {}", me.stable_id);
    if let Some(t) = &st.current_tailnet {
        println!("  tailnet:   {}", t.name);
    }
    let tags = me.tags.as_deref().unwrap_or_default();
    println!(
        "  tags:      {}",
        if tags.is_empty() {
            "(none)".to_owned()
        } else {
            tags.join(", ")
        }
    );
}

pub async fn run(data_dir: &Path, config: &Config) -> anyhow::Result<()> {
    let control_path = data_dir.join(config::CONTROL_SOCKET);
    anyhow::ensure!(
        !control::daemon_running(&control_path).await,
        "collied is already running"
    );
    let (node, _lock, _) = mac_node(data_dir, config, None)?;
    node.start()?;
    let deadline = Instant::now() + RUN_START_TIMEOUT;
    loop {
        let st = node.status()?;
        if st.backend_state == BackendState::Running && st.self_node.is_some() {
            check_identity(&node, &st, data_dir, config.tailnet.owner_user_id)?;
            break;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "tailnet node is {:?} after 60 s; not listening (run `collied login`)",
            st.backend_state
        );
        tokio::time::sleep(POLL).await;
    }
    let env = herdr::SocketEnv::from_process();
    let herdr_socket = herdr::resolve_socket_path(config.herdr.session.as_deref(), &env)?;
    let apns = push::sender(config)?;
    let mut handle = server::start_with(
        node,
        ServerConfig {
            data_dir: data_dir.to_owned(),
            port: config.tailnet.port,
            owner_user_id: config.tailnet.owner_user_id,
            herdr_session: herdr::session_label(config.herdr.session.as_deref(), &env),
            machine_name: config::machine_name(),
            approval_ttl: approvals::TTL,
            attachments_dir: config::attachments_dir()?,
            terminals: config.terminals.enabled,
            terminal_grant_ttl: terminal::GRANT_TTL,
        },
        herdr_socket,
        &config.tasks,
        apns,
    )
    .await?;
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install SIGTERM handler")?;
    let listener_failed = tokio::select! {
        _ = tokio::signal::ctrl_c() => false,
        _ = term.recv() => false,
        _ = handle.listener_failed() => true,
    };
    tracing::info!("shutting down");
    handle.shutdown().await;
    anyhow::ensure!(
        !listener_failed,
        "tailnet listener failed; exiting so the service manager restarts collied"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_browser_opens_once_even_when_open_fails() {
        for success in [true, false] {
            let mut browser = LoginBrowser::new(true, false, false);
            assert_eq!(
                browser.open("https://login.tailscale.com/a/test", |_| success),
                Some(success)
            );
            assert_eq!(
                browser.open("https://login.tailscale.com/a/another", |_| panic!(
                    "opened twice"
                )),
                None
            );
        }
    }

    #[test]
    fn login_browser_requires_interactive_login_without_an_auth_key() {
        for (interactive, auth_key) in [(false, false), (false, true), (true, true)] {
            assert_eq!(
                LoginBrowser::new(interactive, auth_key, false)
                    .open("https://login.tailscale.com/a/test", |_| panic!(
                        "unexpected browser"
                    )),
                None
            );
        }
    }

    #[test]
    fn custom_control_never_opens_even_a_trusted_login_url() {
        assert_eq!(
            LoginBrowser::new(true, false, true)
                .open("https://login.tailscale.com/a/test", |_| panic!(
                    "custom control opened the browser"
                )),
            None
        );
    }

    #[test]
    fn login_browser_only_opens_the_trusted_https_authority() {
        let mut browser = LoginBrowser::new(true, false, false);
        for url in [
            "http://login.tailscale.com/a/test",
            "https://control.example/a/test",
            "https://login.tailscale.com.evil.example/a/test",
            "https://user@login.tailscale.com/a/test",
            "https://login.tailscale.com:444/a/test",
            "https://login.tailscale.com:0443/a/test",
            "file:///tmp/test",
            "not a URL",
        ] {
            assert_eq!(browser.open(url, |_| panic!("untrusted browser URL")), None);
        }
        assert_eq!(
            browser.open("https://login.tailscale.com:443/a/test", |_| true),
            Some(true)
        );
    }

    #[test]
    fn policy_snippet_matches_the_docs() {
        let docs = include_str!("../../../docs/tailnet.md");
        let snippet = policy_snippet(8457);
        for line in snippet.lines().map(str::trim).filter(|l| l.len() > 3) {
            assert!(docs.contains(line), "docs/tailnet.md lacks {line}");
        }
        assert!(snippet.contains("autogroup:self"));
    }

    #[test]
    fn finds_the_last_login_error() {
        let health = |h: &[&str]| h.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(login_error(&health(&["You are logged out."])), None);
        assert_eq!(
            login_error(&health(&[
                "Unable to connect to the Tailscale coordination server.",
                "You are logged out. The last login error was: requested tags [tag:collie-mac] are invalid or not permitted",
            ])),
            Some("requested tags [tag:collie-mac] are invalid or not permitted")
        );
    }
}
