use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use collied::config::{self, Config};
use collied::control::{self, Client, Reply, Request};
use collied::{daemon, doctor, hooks, push, service};
use tracing_subscriber::EnvFilter;
use zeroize::Zeroizing;

const AUTHKEY_ENV: &str = "COLLIE_TS_AUTHKEY";

#[derive(Parser)]
#[command(version, about = "collie daemon")]
struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Join the tailnet as tag:collie-mac on macOS, tag:collie-linux on Linux
    /// (interactive, or COLLIE_TS_AUTHKEY).
    Login,
    /// Run the daemon.
    Run,
    /// Open a pairing window and show its QR code.
    Pair {
        /// Also print the invite as text (contains the one-time code; for the simulator).
        #[arg(long)]
        show_uri: bool,
    },
    /// Manage paired phones.
    Peers {
        #[command(subcommand)]
        command: PeersCommand,
    },
    /// Show the running daemon's state.
    Status,
    /// Stop the service and keep it off, across reboots, until `collied start`.
    Stop,
    /// Start the service again after `collied stop`.
    Start,
    /// Install or remove the service (launchd agent on macOS, systemd user unit on Linux).
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Check configuration, permissions and herdr reachability.
    Doctor,
    /// Apple push notifications.
    Apns {
        #[command(subcommand)]
        command: ApnsCommand,
    },
    /// Claude Code PermissionRequest hook: reports the pending tool call to collied.
    Hook,
    /// Claude Code status line tap: records the plan usage and context window from the
    /// status line input on stdin. Prints nothing.
    Statusline,
}

#[derive(Subcommand)]
enum ApnsCommand {
    /// Send a test alert to every registered device of a paired phone.
    Test,
    /// Move a .p8 key into the login Keychain, then offer to delete the file.
    Import { path: PathBuf },
}

#[derive(Subcommand)]
enum PeersCommand {
    List,
    /// Revoke by label or stable id; closes its live sessions.
    Revoke {
        target: String,
    },
}

#[derive(Subcommand)]
enum ServiceCommand {
    Install,
    Uninstall,
}

fn main() -> anyhow::Result<ExitCode> {
    let auth_key = std::env::var(AUTHKEY_ENV).ok().map(Zeroizing::new);
    // SAFETY: no other Rust thread exists yet. The embedded Go runtime (libtailscale
    // c-archive) initializes on its own thread from a load-time constructor and copies
    // envp there, possibly concurrently with this call; it never reads the C environment
    // again. So Go's copy may still hold the key for the life of the process, and only
    // child processes and later Rust reads are guaranteed not to see it.
    unsafe { std::env::remove_var(AUTHKEY_ENV) };

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    // Runs on every status line refresh: no runtime, no output.
    if let Command::Statusline = cli.command {
        let recorded = config::data_dir().and_then(|d| collied::usage::record_stdin(&d));
        return Ok(if recorded.is_ok() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let ok = rt.block_on(dispatch(cli, auth_key));
    // A pending stdin read in `pair` must not keep the process alive.
    rt.shutdown_background();
    let ok = ok?;
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn load_config(path: Option<&Path>, data_dir: &Path) -> anyhow::Result<Config> {
    let explicit = path.is_some();
    let path = path.map_or_else(|| data_dir.join(config::CONFIG_FILE), Path::to_owned);
    Ok(config::load(&path, explicit)?.unwrap_or_default())
}

async fn dispatch(cli: Cli, auth_key: Option<Zeroizing<String>>) -> anyhow::Result<bool> {
    let data_dir = config::data_dir()?;
    let control_path = data_dir.join(config::CONTROL_SOCKET);
    match cli.command {
        Command::Doctor => return doctor::run(cli.config).await,
        Command::Apns {
            command: ApnsCommand::Test,
        } => {
            let config = load_config(cli.config.as_deref(), &data_dir)?;
            let apns = config
                .apns
                .as_ref()
                .context("no [apns] section in collied.toml")?;
            return push::send_test(&data_dir, apns).await;
        }
        Command::Apns {
            command: ApnsCommand::Import { path },
        } => {
            let explicit = cli.config.is_some();
            let config_path = cli
                .config
                .unwrap_or_else(|| data_dir.join(config::CONFIG_FILE));
            return push::import(&config_path, explicit, &path);
        }
        Command::Login => {
            let config = load_config(cli.config.as_deref(), &data_dir)?;
            daemon::login(&data_dir, &config, auth_key).await?;
        }
        Command::Run => {
            drop(auth_key);
            let config = load_config(cli.config.as_deref(), &data_dir)?;
            daemon::run(&data_dir, &config).await?;
        }
        Command::Pair { show_uri } => return pair(&control_path, show_uri).await,
        Command::Peers { command } => peers(command, &control_path, &data_dir).await?,
        Command::Status => status(&control_path).await?,
        Command::Hook => hooks::run(&control_path).await,
        Command::Statusline => unreachable!("handled before the runtime starts"),
        Command::Stop => service::stop()?,
        Command::Start => service::start()?,
        Command::Service { command } => match command {
            ServiceCommand::Install => service::install(cli.config.as_deref(), &data_dir)?,
            ServiceCommand::Uninstall => service::uninstall()?,
        },
    }
    Ok(true)
}

fn not_running() -> anyhow::Error {
    anyhow::anyhow!("collied is not running")
}

async fn status(control_path: &Path) -> anyhow::Result<()> {
    match control::request(control_path, &Request::Status).await? {
        Some(Reply::Status(s)) => {
            println!("pid:       {}", s.pid);
            println!("node:      {} ({})", s.backend_state, s.node_id);
            println!("name:      {}", s.dns_name);
            println!("port:      {}", s.port);
            println!("sessions:  {}", s.sessions);
            println!("peers:     {}", s.peers);
            println!(
                "tags:      {}",
                match s.tags.as_deref() {
                    None => "(not reported)".to_owned(),
                    Some([]) => "(none)".to_owned(),
                    Some(t) => t.join(", "),
                }
            );
            println!(
                "herdr:     {}",
                s.herdr_version.as_deref().unwrap_or("unreachable")
            );
            if let Some(flock) = s.flock {
                print_agents(&flock);
            } else if s.flock_too_large {
                println!("agents:    too many to list here");
            }
            Ok(())
        }
        Some(other) => anyhow::bail!("unexpected reply: {other:?}"),
        None => Err(not_running()),
    }
}

fn print_agents(flock: &control::StatusFlock) {
    let p = collied::printable;
    println!("agents:    {}", flock.agents.len());
    for a in &flock.agents {
        let workspace = flock
            .workspaces
            .iter()
            .find(|w| w.workspace_id == a.workspace_id)
            .map_or("?", |w| w.label.as_str());
        let name = a.name.as_deref().or(a.kind.as_deref()).unwrap_or("agent");
        println!(
            "  {:<8} {}\t{}\t{}",
            format!("{:?}", a.status).to_lowercase(),
            p(name),
            p(workspace),
            p(a.terminal_id.as_str())
        );
    }
}

async fn peers(command: PeersCommand, control_path: &Path, data_dir: &Path) -> anyhow::Result<()> {
    match command {
        PeersCommand::List => {
            let (owner, peers) = match control::request(control_path, &Request::PeersList).await? {
                Some(Reply::Peers {
                    owner_user_id,
                    peers,
                }) => (owner_user_id, peers),
                Some(other) => anyhow::bail!("unexpected reply: {other:?}"),
                None => {
                    let store = collied::peers::load(&data_dir.join(config::PEERS_FILE))?;
                    (store.owner_user_id, store.peers)
                }
            };
            match owner {
                Some(o) => println!("owner user id: {o}"),
                None => println!("no owner yet"),
            }
            let e = collied::printable;
            for p in peers {
                println!(
                    "{}\t{}\t{} ({})\tpaired_at_ms={}",
                    e(&p.label),
                    e(&p.stable_id),
                    e(&p.login),
                    p.user_id,
                    p.paired_at
                );
            }
        }
        PeersCommand::Revoke { target } => {
            let req = Request::PeersRevoke {
                target: target.clone(),
            };
            match control::request(control_path, &req).await? {
                Some(Reply::Revoked {
                    peer,
                    closed_sessions,
                }) => println!(
                    "revoked {} ({}), closed {closed_sessions} session(s)",
                    collied::printable(&peer.label),
                    collied::printable(&peer.stable_id)
                ),
                Some(Reply::Error { message }) => anyhow::bail!(message),
                Some(other) => anyhow::bail!("unexpected reply: {other:?}"),
                None => {
                    let peer = control::revoke_offline(data_dir, &target)?;
                    println!(
                        "revoked {} ({}) with the daemon stopped",
                        collied::printable(&peer.label),
                        collied::printable(&peer.stable_id)
                    );
                }
            }
        }
    }
    Ok(())
}

async fn pair(control_path: &Path, show_uri: bool) -> anyhow::Result<bool> {
    let mut client = Client::connect(control_path)
        .await
        .map_err(|_| not_running())?;
    match client.call(&Request::Pair).await? {
        Reply::Invite {
            uri,
            expires_in_secs,
        } => {
            println!("Scan with the Collie app:\n");
            println!("{}", collied::qr_text(&uri)?);
            if show_uri {
                println!("{uri}\n");
            }
            println!("Waiting up to {expires_in_secs} s for the phone (Ctrl-C cancels).");
        }
        Reply::Error { message } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
    loop {
        match client.recv().await.context("daemon closed the pairing")? {
            Reply::Confirm(c) => {
                println!("\nA phone presented the pairing code:");
                let p = collied::printable;
                println!("  device:    {}", p(&c.device_label));
                println!("  node:      {} ({})", p(&c.node_name), p(&c.stable_id));
                println!("  user:      {} ({})", p(&c.login), c.user_id);
                println!("  terminal key: {}", c.terminal_key_change());
                if c.replaces {
                    println!("  replaces an existing pairing of this node");
                }
                // Discard anything typed before the candidate was shown, so a stray
                // "y" cannot approve a phone the human never saw.
                let _ = rustix::termios::tcflush(
                    std::io::stdin(),
                    rustix::termios::QueueSelector::IFlush,
                );
                print!("Pair this phone? [y/N] ");
                std::io::Write::flush(&mut std::io::stdout())?;
                let answer = tokio::task::spawn_blocking(|| {
                    let mut line = String::new();
                    std::io::stdin().lock().read_line(&mut line).map(|_| line)
                });
                tokio::select! {
                    line = answer => {
                        let accept = line??.trim().eq_ignore_ascii_case("y");
                        client.send(&Request::Confirm { accept }).await?;
                    }
                    reply = client.recv() => {
                        if let Reply::PairDone { paired, detail } = reply? {
                            println!("\n{}", collied::printable(&detail));
                            return Ok(paired);
                        }
                    }
                }
            }
            Reply::PairDone { paired, detail } => {
                println!("{}", collied::printable(&detail));
                return Ok(paired);
            }
            other => anyhow::bail!("unexpected reply: {other:?}"),
        }
    }
}
