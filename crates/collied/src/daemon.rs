use std::os::fd::OwnedFd;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use tailnet::{BackendState, Node, Status};
use zeroize::Zeroizing;

use crate::config::{self, Config};
use crate::control;
use crate::server::{self, ServerConfig};
use crate::{approvals, herdr, push};

pub const MAC_TAG: &str = "tag:collie-mac";
const POLL: Duration = Duration::from_millis(250);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const RUN_START_TIMEOUT: Duration = Duration::from_secs(60);

/// The returned lock must outlive the node: two processes on one tsnet state dir
/// would fight over the node keys, and a starting daemon has no control socket yet.
fn mac_node(
    data_dir: &Path,
    config: &Config,
    auth_key: Option<Zeroizing<String>>,
) -> anyhow::Result<(Node, OwnedFd)> {
    crate::ensure_private_dir(data_dir)?;
    let lock = crate::peers::lock(&data_dir.join(config::NODE_LOCK))
        .context("another collied (login or run) is using this data dir")?;
    let tsnet = data_dir.join(config::TSNET_DIR);
    crate::ensure_private_dir(&tsnet)?;
    let node = Node::new(&tailnet::Config {
        state_dir: tsnet,
        hostname: config.tailnet.hostname(),
        auth_key,
        control_url: None,
        advertise_tags: vec![MAC_TAG.to_owned()],
        log_to_stderr: false,
    })?;
    Ok((node, lock))
}

fn status(node: &Node) -> anyhow::Result<Status> {
    Ok(node.status()?)
}

pub async fn login(
    data_dir: &Path,
    config: &Config,
    auth_key: Option<Zeroizing<String>>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !control::daemon_running(&data_dir.join(config::CONTROL_SOCKET)).await,
        "collied is running; stop it (collied service uninstall) before logging in"
    );
    let (node, _lock) = mac_node(data_dir, config, auth_key)?;
    node.start()?;
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    let mut shown_url = String::new();
    let mut warned_approval = false;
    loop {
        let st = status(&node)?;
        match st.backend_state {
            BackendState::Running if st.self_node.is_some() => {
                print_identity(&st);
                return Ok(());
            }
            BackendState::NeedsMachineAuth if !warned_approval => {
                println!("This Mac needs approval by a tailnet admin; waiting.");
                warned_approval = true;
            }
            _ => {}
        }
        if !st.auth_url.is_empty() && st.auth_url != shown_url {
            println!("Log in to Tailscale to add this Mac to your tailnet:\n");
            println!("{}", crate::qr_text(&st.auth_url)?);
            println!("{}\n", st.auth_url);
            shown_url = st.auth_url;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "login did not complete within 10 minutes (state {:?})",
            st.backend_state
        );
        tokio::time::sleep(POLL).await;
    }
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
    let (node, _lock) = mac_node(data_dir, config, None)?;
    node.start()?;
    let deadline = Instant::now() + RUN_START_TIMEOUT;
    loop {
        let st = status(&node)?;
        if st.backend_state == BackendState::Running {
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
    let apns: Option<Arc<dyn push::Sender>> = match &config.apns {
        Some(apns) => Some(Arc::new(push::Apns::new(apns).context("[apns]")?)),
        None => None,
    };
    let mut handle = server::start_with(
        node,
        ServerConfig {
            data_dir: data_dir.to_owned(),
            port: config.tailnet.port,
            owner_user_id: config.tailnet.owner_user_id,
            herdr_session: herdr::session_label(config.herdr.session.as_deref(), &env),
            machine_name: config::machine_name(),
            approval_ttl: approvals::TTL,
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
        "tailnet listener failed; exiting so launchd restarts collied"
    );
    Ok(())
}
