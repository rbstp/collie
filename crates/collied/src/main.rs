use std::io::{BufRead, IsTerminal};
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
    /// Set up or update this machine: login, service, doctor, push notifications and
    /// pairing, each skipped when already done (interactive, macOS).
    Setup,
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
        Command::Setup => {
            anyhow::ensure!(
                std::io::stdin().is_terminal(),
                "collied setup is interactive: run it in a terminal"
            );
            #[cfg(target_os = "linux")]
            anyhow::bail!(
                "collied setup is macOS only for now: follow the Linux steps in the README"
            );
            #[cfg(target_os = "macos")]
            {
                // The service runs this path: only the copy `just collied-install` signs
                // stays signed after the next cargo build.
                let installed = config::home_dir()?.join(".cargo/bin/collied");
                let exe = std::env::current_exe()?.canonicalize()?;
                doctor::signed_as_collied()
                    .and_then(|()| match installed.canonicalize() {
                        Ok(p) if p == exe => Ok(()),
                        _ => Err(format!("{} is not {}", exe.display(), installed.display())),
                    })
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "{e}: run `just collied-install`, then ~/.cargo/bin/collied setup"
                        )
                    })?;
                let mut host = SetupHost {
                    config: cli.config.as_deref(),
                    data_dir: &data_dir,
                    control_path: &control_path,
                    auth_key,
                };
                return collied::setup::run(&mut host).await;
            }
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

#[cfg(target_os = "macos")]
struct SetupHost<'a> {
    config: Option<&'a Path>,
    data_dir: &'a Path,
    control_path: &'a Path,
    auth_key: Option<Zeroizing<String>>,
}

#[cfg(target_os = "macos")]
impl SetupHost<'_> {
    async fn running_within(&mut self, secs: u64) -> Option<control::StatusInfo> {
        use collied::setup::Host;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        loop {
            if let Some(s) = self.status().await
                && s.backend_state == "Running"
            {
                return Some(s);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}

#[cfg(target_os = "macos")]
impl collied::setup::Host for SetupHost<'_> {
    async fn status(&mut self) -> Option<control::StatusInfo> {
        match control::request(self.control_path, &Request::Status).await {
            Ok(Some(Reply::Status(s))) => Some(s),
            _ => None,
        }
    }

    fn service(&mut self, started_ms: Option<u64>) -> anyhow::Result<service::State> {
        service::state(self.config, self.data_dir, started_ms)
    }

    async fn login(&mut self) -> anyhow::Result<()> {
        let config = load_config(self.config, self.data_dir)?;
        daemon::login(self.data_dir, &config, self.auth_key.take()).await
    }

    fn unload_service(&mut self) -> anyhow::Result<()> {
        service::unload()
    }

    fn start_service(&mut self) -> anyhow::Result<()> {
        service::start()
    }

    fn install_service(&mut self) -> anyhow::Result<()> {
        service::install(self.config, self.data_dir)
    }

    async fn settle(&mut self) -> Option<control::StatusInfo> {
        self.running_within(20).await
    }

    async fn wait_daemon(&mut self) -> anyhow::Result<control::StatusInfo> {
        if let Some(s) = self.running_within(60).await {
            return Ok(s);
        }
        let log = self.data_dir.join(service::STDERR_LOG);
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        for line in &lines[lines.len().saturating_sub(20)..] {
            println!("  {line}");
        }
        anyhow::bail!(
            "collied did not start within 60 s; the end of {} is above",
            log.display()
        );
    }

    async fn doctor(&mut self) -> anyhow::Result<bool> {
        doctor::run(self.config.map(Path::to_owned)).await
    }

    fn apns_configured(&mut self) -> anyhow::Result<bool> {
        Ok(load_config(self.config, self.data_dir)?.apns.is_some())
    }

    fn setup_apns(&mut self) -> anyhow::Result<bool> {
        use collied::setup;
        let p8 = setup::typed_path(&prompt(
            "Path to the APNs key AuthKey_<KEY_ID>.p8 (mode 0600):",
        )?)?;
        let key_id = match setup::key_id_of(&p8) {
            Some(id) => id.to_owned(),
            None => prompt("Key ID:")?,
        };
        let or = |typed: String, default: &str| {
            if typed.is_empty() {
                default.to_owned()
            } else {
                typed
            }
        };
        let team_id = or(
            prompt(&format!("Team ID [{}]:", setup::TEAM_ID))?,
            setup::TEAM_ID,
        );
        let bundle_id = or(
            prompt(&format!("App bundle ID [{}]:", setup::BUNDLE_ID))?,
            setup::BUNDLE_ID,
        );
        let section = setup::apns_section(&key_id, &team_id, &bundle_id)?;
        let explicit = self.config.is_some();
        let config_path = self
            .config
            .map_or_else(|| self.data_dir.join(config::CONFIG_FILE), Path::to_owned);
        setup::with_apns(&config_path, &section, || {
            push::import(&config_path, explicit, &p8)
        })
    }

    async fn pair(&mut self) -> anyhow::Result<bool> {
        pair(self.control_path, false).await
    }

    fn ask(&mut self, question: &str) -> anyhow::Result<bool> {
        Ok(prompt(question)?.eq_ignore_ascii_case("y"))
    }

    fn say(&mut self, line: &str) {
        println!("{line}");
    }
}

#[cfg(target_os = "macos")]
fn prompt(question: &str) -> anyhow::Result<String> {
    // Discard anything typed before the question was shown.
    let _ = rustix::termios::tcflush(std::io::stdin(), rustix::termios::QueueSelector::IFlush);
    print!("{question} ");
    std::io::Write::flush(&mut std::io::stdout())?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_owned())
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
