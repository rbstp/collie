use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::herdr::{self, AgentInfo, AgentSession, PaneProcessInfo};
use crate::{config, peers, transcript};

const LIMIT: u64 = 65536;
const INPUT_BYTES: u64 = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(2);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    #[serde(alias = "session_id")]
    session_id: String,
    source: String,
    timestamp: u64,
}

impl Input {
    fn valid(&self) -> bool {
        matches!(self.source.as_str(), "startup" | "new" | "resume")
            && transcript::is_session_id(&self.session_id)
            && self.timestamp != 0
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    socket: PathBuf,
    terminal: String,
    session: String,
    pid: u32,
    started: String,
    timestamp: u64,
}

pub fn home() -> anyhow::Result<PathBuf> {
    Ok(match std::env::var_os("COPILOT_HOME") {
        Some(root) => PathBuf::from(root),
        None => config::home_dir()?.join(".copilot"),
    })
}

fn record_path(data: &Path, socket: &Path, terminal: &str) -> PathBuf {
    let mut hash = Sha256::new();
    hash.update(socket.as_os_str().as_bytes());
    hash.update([0]);
    hash.update(terminal.as_bytes());
    let key = hash.finalize();
    let name: String = key.iter().map(|b| format!("{b:02x}")).collect();
    data.join("copilot-sessions").join(format!("{name}.json"))
}

fn read(path: &Path) -> Option<Identity> {
    let dir = std::fs::symlink_metadata(path.parent()?).ok()?;
    if !dir.is_dir() || dir.mode() & 0o077 != 0 || dir.uid() != rustix::process::geteuid().as_raw()
    {
        return None;
    }
    let (file, len, _) = transcript::open(path)?;
    let meta = file.metadata().ok()?;
    if meta.mode() & 0o077 != 0 || meta.uid() != rustix::process::geteuid().as_raw() {
        return None;
    }
    (len <= LIMIT)
        .then(|| serde_json::from_reader(file.take(LIMIT)).ok())
        .flatten()
}

fn foreground(info: &PaneProcessInfo, pane: &str, pid: u32) -> bool {
    info.pane_id == pane
        && info.foreground_process_group_id == Some(pid)
        && info
            .foreground_processes
            .iter()
            .any(|p| p.pid == pid && p.name == "copilot")
}

async fn process(pid: u32, field: &str) -> Option<String> {
    let output = tokio::time::timeout(
        DEADLINE,
        tokio::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", field])
            .env("LC_ALL", "C")
            .env("TZ", "UTC")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())?
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

async fn caller() -> Option<u32> {
    let mut pid = rustix::process::getppid()?.as_raw_nonzero().get() as u32;
    for _ in 0..8 {
        let command = process(pid, "comm=").await?;
        if Path::new(&command).file_name()?.to_str()? == "copilot" {
            return Some(pid);
        }
        pid = process(pid, "ppid=").await?.parse().ok()?;
        if pid <= 1 {
            break;
        }
    }
    None
}

pub async fn hook(data: &Path) {
    let mut input = Vec::new();
    if std::io::stdin()
        .take(INPUT_BYTES + 1)
        .read_to_end(&mut input)
        .is_err()
        || input.len() as u64 > INPUT_BYTES
    {
        return;
    }
    let Ok(input) = serde_json::from_slice::<Input>(&input) else {
        return;
    };
    if !input.valid() || std::env::var("HERDR_ENV").as_deref() != Ok("1") {
        return;
    }
    let (Some(socket), Ok(pane)) = (
        std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from),
        std::env::var("HERDR_PANE_ID"),
    ) else {
        return;
    };
    let _ = tokio::time::timeout(DEADLINE, async {
        register(data, &socket, &pane, &input, caller().await?).await
    })
    .await;
}

async fn register(data: &Path, socket: &Path, pane: &str, input: &Input, pid: u32) -> Option<()> {
    if !input.valid() {
        return None;
    }
    let info = herdr::pane_process_info(socket, pane).await.ok()?;
    if !foreground(&info, pane, pid) {
        return None;
    }
    let current = herdr::pane_get(socket, pane).await.ok()?;
    if current.pane_id != pane {
        return None;
    }
    let identity = Identity {
        socket: socket.into(),
        terminal: current.terminal_id,
        session: input.session_id.clone(),
        pid,
        started: process(pid, "lstart=").await?,
        timestamp: input.timestamp,
    };
    save(data, &identity).await
}

async fn save(data: &Path, identity: &Identity) -> Option<()> {
    crate::ensure_private_dir(data).ok()?;
    let dir = data.join("copilot-sessions");
    crate::ensure_private_dir(&dir).ok()?;
    let path = record_path(data, &identity.socket, &identity.terminal);
    let _lock = loop {
        match peers::lock(&path.with_extension("lock")) {
            Ok(lock) => break lock,
            Err(peers::Error::Locked(_)) => tokio::time::sleep(Duration::from_millis(20)).await,
            Err(_) => return None,
        }
    };
    if read(&path).is_some_and(|old| {
        old.socket == identity.socket
            && old.terminal == identity.terminal
            && old.pid == identity.pid
            && old.started == identity.started
            && old.timestamp >= identity.timestamp
    }) {
        return Some(());
    }
    peers::replace_json(&path, identity).ok()?;
    Some(())
}

pub async fn resolve(data: &Path, socket: &Path, agent: &mut AgentInfo) -> bool {
    if agent.agent.as_deref() != Some("copilot") {
        return false;
    }
    agent.agent_session = None;
    let path = record_path(data, socket, &agent.terminal_id);
    let Some(identity) = read(&path).filter(|r| {
        r.socket == socket
            && r.terminal == agent.terminal_id
            && r.timestamp != 0
            && transcript::is_session_id(&r.session)
    }) else {
        return false;
    };
    let valid = tokio::time::timeout(DEADLINE, async {
        herdr::pane_process_info(socket, &agent.pane_id)
            .await
            .is_ok_and(|info| foreground(&info, &agent.pane_id, identity.pid))
            && process(identity.pid, "lstart=").await.as_ref() == Some(&identity.started)
    })
    .await
    .unwrap_or(false);
    agent.agent_session = valid.then_some(AgentSession {
        value: identity.session,
    });
    valid
}

pub async fn resolve_agents(data: &Path, socket: &Path, agents: &mut [AgentInfo], phones: bool) {
    if phones {
        for agent in agents {
            resolve(data, socket, agent).await;
        }
    }
}

pub fn installed(settings: &Value) -> bool {
    settings["hooks"]["SessionStart"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|h| {
            h["bash"]
                .as_str()
                .is_some_and(|s| s.ends_with(" copilot-hook"))
        })
}

fn settings(settings: &mut Value, executable: &str) -> Option<()> {
    let escaped = executable.replace('\'', "'\\''");
    let command = format!("'{escaped}' copilot-hook");
    let hooks = settings
        .as_object_mut()?
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()?;
    let starts = hooks
        .entry("SessionStart")
        .or_insert_with(|| json!([]))
        .as_array_mut()?;
    starts.retain(|h| {
        !h["bash"]
            .as_str()
            .is_some_and(|s| s.ends_with(" copilot-hook"))
    });
    starts.push(json!({"type":"command","bash":command,"timeoutSec":10}));
    Some(())
}

pub fn install(executable: &Path) -> anyhow::Result<()> {
    install_at(&home()?, executable)
}

fn install_at(root: &Path, executable: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(executable.is_absolute(), "executable path must be absolute");
    match std::fs::symlink_metadata(root) {
        Ok(meta) => anyhow::ensure!(
            meta.is_dir()
                && meta.uid() == rustix::process::geteuid().as_raw()
                && meta.mode() & 0o022 == 0,
            "Copilot home must be a user-owned directory without group or world write access"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => crate::ensure_private_dir(root)?,
        Err(e) => return Err(e.into()),
    }
    let path = root.join("settings.json");
    let _lock = peers::lock(&root.join("collied-hooks.lock"))?;
    let mut settings = match std::fs::symlink_metadata(&path) {
        Ok(_) => {
            let (file, len, _) = transcript::open(&path)
                .ok_or_else(|| anyhow::anyhow!("settings.json is not a regular file"))?;
            anyhow::ensure!(len <= LIMIT, "settings.json exceeds 64 KiB");
            serde_json::from_reader(file)?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.into()),
    };
    anyhow::ensure!(
        self::settings(
            &mut settings,
            executable
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("invalid executable path"))?
        )
        .is_some(),
        "invalid hooks settings"
    );
    peers::save_json(&path, &settings)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    use protocol::{HistoryParams, TerminalId};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    use super::*;

    const OLD: &str = "11111111-2222-4333-8444-555555555555";
    const NEW: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    const LOG: &str = include_str!("../tests/fixtures/transcripts/copilot.jsonl");

    struct Mock {
        socket: PathBuf,
        group: Arc<AtomicU32>,
        calls: Arc<AtomicUsize>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for Mock {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn mock(dir: &Path) -> Mock {
        let socket = dir.join("herdr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let group = Arc::new(AtomicU32::new(std::process::id()));
        let calls = Arc::new(AtomicUsize::new(0));
        let (g, c) = (group.clone(), calls.clone());
        let task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                c.fetch_add(1, Ordering::Relaxed);
                let pane = request["params"]["pane_id"].as_str().unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "pane.process_info" => json!({
                        "type":"pane_process_info",
                        "process_info":{
                            "pane_id":pane, "foreground_process_group_id":g.load(Ordering::Relaxed),
                            "foreground_processes":[{"pid":g.load(Ordering::Relaxed),"name":"copilot","argv":["ignored"]}],
                            "shell_pid":12, "tty":null
                        }
                    }),
                    "pane.get" => json!({
                        "type":"pane_info",
                        "pane":{
                            "pane_id":pane, "terminal_id":if pane.ends_with("p2") {"t2"} else {"t1"},
                            "workspace_id":"w1", "tab_id":"w1:t1", "cwd":"/fictional/counter"
                        }
                    }),
                    method => panic!("unexpected method {method}"),
                };
                let response = json!({"id":request["id"],"result":result}).to_string() + "\n";
                reader
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
            }
        });
        Mock {
            socket,
            group,
            calls,
            task,
        }
    }

    fn input() -> Input {
        serde_json::from_str(include_str!("../tests/fixtures/copilot-session-start.json")).unwrap()
    }

    fn agent() -> AgentInfo {
        serde_json::from_value(json!({
            "pane_id":"w1:p1", "terminal_id":"t1", "workspace_id":"w1",
            "agent":"copilot", "agent_status":"working",
            "cwd":"/fictional/counter", "agent_session":{"value":OLD}
        }))
        .unwrap()
    }

    fn params(after: Option<String>) -> HistoryParams {
        HistoryParams {
            terminal_id: TerminalId::new("t1").unwrap(),
            before: None,
            after,
            query: None,
        }
    }

    fn transcript(root: &Path, id: &str, content: &str) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn lifecycle_input_requires_exact_uuid_source_and_timestamp() {
        let mut report = input();
        assert!(report.valid());
        for source in ["startup", "new", "resume"] {
            report.source = source.into();
            assert!(report.valid());
        }
        for source in ["", "fork", "compact", "SessionStart"] {
            report.source = source.into();
            assert!(!report.valid());
        }
        report = input();
        for id in [
            "../session",
            "11111111-2222-4333-8444-55555555555G",
            "11111111222243338444555555555555",
        ] {
            report.session_id = id.into();
            assert!(!report.valid());
        }
        report = input();
        report.timestamp = 0;
        assert!(!report.valid());
        for malformed in [
            r#"{"source":"new"}"#,
            r#"{"source":"new","sessionId":12,"timestamp":1}"#,
            r#"{"source":"new","sessionId":"11111111-2222-4333-8444-555555555555","timestamp":"now"}"#,
        ] {
            assert!(serde_json::from_str::<Input>(malformed).is_err());
        }
    }

    #[test]
    fn foreground_requires_the_exact_pane_and_copilot_group_leader() {
        let mut info: PaneProcessInfo = serde_json::from_value(json!({
            "pane_id":"w1:p1", "foreground_process_group_id":42,
            "foreground_processes":[{"pid":42,"name":"copilot"}, {"pid":43,"name":"copilot"}]
        }))
        .unwrap();
        assert!(foreground(&info, "w1:p1", 42));
        assert!(!foreground(&info, "w1:p1", 43));
        assert!(!foreground(&info, "w2:p1", 42));
        info.foreground_processes[0].name = "node".into();
        assert!(!foreground(&info, "w1:p1", 42));
        info.foreground_process_group_id = None;
        assert!(!foreground(&info, "w1:p1", 42));
    }

    #[tokio::test]
    async fn initial_new_resume_and_reconnect_resolve_without_herdr_registration() {
        let dir = tempfile::tempdir().unwrap();
        let mock = mock(dir.path());
        let data = dir.path().join("data");
        let root = dir.path().join("session-state");
        let mut transcripts = transcript::Transcripts::new(None, None).with_copilot(root.clone());
        let mut raw = agent();
        raw.agent_session = None;
        let mut current = raw.clone();
        assert!(!resolve(&data, &mock.socket, &mut current).await);
        assert!(!transcripts.history(&current, &params(None)).available);
        let mut report = input();
        assert!(
            register(
                &data,
                &mock.socket,
                &raw.pane_id,
                &report,
                std::process::id()
            )
            .await
            .is_some()
        );
        assert!(resolve(&data, &mock.socket, &mut current).await);
        assert_eq!(current.agent_session.as_ref().unwrap().value, OLD);
        assert!(!transcripts.history(&current, &params(None)).available);
        let path = transcript(&root, OLD, "");
        assert!(transcripts.history(&current, &params(None)).available);
        std::fs::write(&path, LOG).unwrap();
        let old = transcripts.history(&current, &params(None));
        assert_eq!(old.messages.len(), 7);
        assert!(
            transcripts
                .derive(&current, true)
                .unwrap()
                .last_prompt
                .is_some()
        );
        for bad in [
            LOG.replace(OLD, NEW),
            LOG.replace("\"version\":1", "\"version\":2"),
            "{\"type\":\"session.start\"}".into(),
            "{\n".into(),
        ] {
            std::fs::write(&path, bad).unwrap();
            assert!(!transcripts.history(&current, &params(None)).available);
            assert!(transcripts.sent_prompts(&current).is_empty());
        }
        std::fs::write(&path, LOG).unwrap();
        raw.agent_session = Some(AgentSession { value: OLD.into() });
        let updated = LOG
            .replace(OLD, NEW)
            .replace("Fix the café counter test.", "Check the new counter.");
        transcript(&root, NEW, &updated);
        report.source = "new".into();
        report.session_id = NEW.into();
        report.timestamp += 1;
        assert!(
            register(
                &data,
                &mock.socket,
                &raw.pane_id,
                &report,
                std::process::id()
            )
            .await
            .is_some()
        );
        let mut changed = raw.clone();
        assert!(resolve(&data, &mock.socket, &mut changed).await);
        let page = transcripts.history(&changed, &params(old.after));
        assert!(page.reset);
        assert!(page.session.unwrap().contains(NEW));
        assert_eq!(page.messages[0].text, "Check the new counter.");
        assert_eq!(
            transcripts
                .derive(&changed, true)
                .unwrap()
                .last_prompt
                .as_deref(),
            Some("Check the new counter.")
        );
        assert_ne!(
            current.agent_session.unwrap().value,
            changed.agent_session.unwrap().value
        );
        let mut reconnected = raw.clone();
        assert!(resolve(&data, &mock.socket, &mut reconnected).await);
        assert_eq!(reconnected.agent_session.unwrap().value, NEW);
        report.source = "resume".into();
        report.session_id = OLD.into();
        report.timestamp += 1;
        assert!(
            register(
                &data,
                &mock.socket,
                &raw.pane_id,
                &report,
                std::process::id()
            )
            .await
            .is_some()
        );
        assert!(resolve(&data, &mock.socket, &mut raw).await);
        assert_eq!(raw.agent_session.unwrap().value, OLD);
        let record = record_path(&data, &mock.socket, "t1");
        let stored = std::fs::read_to_string(&record).unwrap();
        assert!(!stored.contains("initialPrompt"));
        assert!(!stored.contains("counter"));
        assert_eq!(std::fs::metadata(&record).unwrap().mode() & 0o777, 0o600);
    }

    #[tokio::test]
    async fn inherited_pane_targeting_from_a_different_process_cannot_register() {
        let dir = tempfile::tempdir().unwrap();
        let mock = mock(dir.path());
        let data = dir.path().join("data");
        mock.group.store(std::process::id() + 1, Ordering::Relaxed);
        assert!(
            register(&data, &mock.socket, "w1:p1", &input(), std::process::id())
                .await
                .is_none()
        );
        assert!(!data.exists());
    }

    #[tokio::test]
    async fn concurrent_reports_keep_the_newer_lifecycle_and_panes_never_use_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let mock = mock(dir.path());
        let data = dir.path().join("data");
        register(&data, &mock.socket, "w1:p1", &input(), std::process::id())
            .await
            .unwrap();
        let path = record_path(&data, &mock.socket, "t1");
        let mut older = read(&path).unwrap();
        older.timestamp += 1;
        let mut newer = read(&path).unwrap();
        newer.timestamp += 2;
        newer.session = NEW.into();
        let (a, b) = tokio::join!(save(&data, &newer), save(&data, &older));
        assert!(a.is_some() && b.is_some());
        assert_eq!(read(&path).unwrap().session, NEW);
        register(&data, &mock.socket, "w1:p2", &input(), std::process::id())
            .await
            .unwrap();
        let mut first = agent();
        let mut second = agent();
        second.pane_id = "w1:p2".into();
        second.terminal_id = "t2".into();
        resolve(&data, &mock.socket, &mut first).await;
        resolve(&data, &mock.socket, &mut second).await;
        assert_eq!(first.agent_session.unwrap().value, NEW);
        assert_eq!(second.agent_session.unwrap().value, OLD);
        let mut moved = agent();
        moved.pane_id = "w2:p1".into();
        assert!(resolve(&data, &mock.socket, &mut moved).await);
        assert_eq!(moved.agent_session.unwrap().value, NEW);
    }

    #[tokio::test]
    async fn missing_stale_malformed_and_unsafe_identity_records_fail_closed() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let dir = tempfile::tempdir().unwrap();
        let mock = mock(dir.path());
        let data = dir.path().join("data");
        let mut current = agent();
        assert!(!resolve(&data, &mock.socket, &mut current).await);
        assert!(current.agent_session.is_none());
        register(&data, &mock.socket, "w1:p1", &input(), std::process::id())
            .await
            .unwrap();
        let path = record_path(&data, &mock.socket, "t1");
        let original = std::fs::read(&path).unwrap();
        for (field, value) in [
            ("session", json!("../foreign")),
            ("terminal", json!("t2")),
            ("socket", json!("/fictional/other.sock")),
            ("started", json!("stale")),
            ("unknown", json!(true)),
        ] {
            let mut identity: Value = serde_json::from_slice(&original).unwrap();
            identity[field] = value;
            peers::replace_json(&path, &identity).unwrap();
            assert!(!resolve(&data, &mock.socket, &mut current).await);
            assert!(current.agent_session.is_none());
        }
        for bytes in [vec![b' '; LIMIT as usize + 1], b"{".to_vec()] {
            std::fs::write(&path, bytes).unwrap();
            assert!(!resolve(&data, &mock.socket, &mut current).await);
        }
        std::fs::write(&path, &original).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!resolve(&data, &mock.socket, &mut current).await);
        std::fs::remove_file(&path).unwrap();
        let other = data.join("other.json");
        std::fs::write(&other, &original).unwrap();
        symlink(&other, &path).unwrap();
        assert!(!resolve(&data, &mock.socket, &mut current).await);
        std::fs::remove_file(&path).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        assert!(!resolve(&data, &mock.socket, &mut current).await);
        std::fs::remove_file(&path).unwrap();
        peers::replace_json(&path, &serde_json::from_slice::<Value>(&original).unwrap()).unwrap();
        mock.group.store(std::process::id() + 1, Ordering::Relaxed);
        assert!(!resolve(&data, &mock.socket, &mut current).await);
        mock.group.store(std::process::id(), Ordering::Relaxed);
        let moved = data.join("saved");
        std::fs::rename(path.parent().unwrap(), &moved).unwrap();
        symlink(&moved, path.parent().unwrap()).unwrap();
        assert!(!resolve(&data, &mock.socket, &mut current).await);
    }

    #[tokio::test]
    async fn offline_reconciliation_and_other_agents_do_not_read_identity_or_call_herdr() {
        let dir = tempfile::tempdir().unwrap();
        let mock = mock(dir.path());
        let data = dir.path().join("data");
        let mut agents = vec![agent()];
        resolve_agents(&data, &mock.socket, &mut agents, false).await;
        assert_eq!(agents[0].agent_session.as_ref().unwrap().value, OLD);
        assert_eq!(mock.calls.load(Ordering::Relaxed), 0);
        agents[0].agent = Some("codex".into());
        resolve_agents(&data, &mock.socket, &mut agents, true).await;
        assert_eq!(agents[0].agent_session.as_ref().unwrap().value, OLD);
        assert_eq!(mock.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn installer_preserves_existing_hooks_and_settings_and_is_idempotent() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("copilot");
        crate::ensure_private_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = root.join("settings.json");
        let original = json!({
            "theme":"dark", "hooks":{
                "SessionStart":[{"type":"command","bash":"herdr-hook","timeoutSec":10}],
                "Stop":[{"type":"command","bash":"custom-hook"}]
            }
        });
        peers::save_json(&path, &original).unwrap();
        let executable = Path::new("/fictional/Mac's tools/collied");
        install_at(&root, executable).unwrap();
        let first = std::fs::read(&path).unwrap();
        let settings: Value = serde_json::from_slice(&first).unwrap();
        assert!(installed(&settings));
        assert_eq!(settings["theme"], original["theme"]);
        assert_eq!(settings["hooks"]["Stop"], original["hooks"]["Stop"]);
        assert_eq!(
            settings["hooks"]["SessionStart"][0],
            original["hooks"]["SessionStart"][0]
        );
        assert_eq!(
            settings["hooks"]["SessionStart"][1]["bash"],
            "'/fictional/Mac'\\''s tools/collied' copilot-hook"
        );
        install_at(&root, executable).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), first);
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn installer_refuses_malformed_oversized_or_symlinked_settings_without_overwriting() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let executable = Path::new("/fictional/collied");
        let path = dir.path().join("settings.json");
        for bytes in [
            b"{".to_vec(),
            b"{\"hooks\":[]}".to_vec(),
            b"{\"hooks\":{\"SessionStart\":{}}}".to_vec(),
            vec![b' '; LIMIT as usize + 1],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(install_at(dir.path(), executable).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::remove_file(&path).unwrap();
        let target = dir.path().join("target.json");
        std::fs::write(&target, "{}").unwrap();
        symlink(&target, &path).unwrap();
        assert!(install_at(dir.path(), executable).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"{}");
        std::fs::remove_file(&path).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        assert!(install_at(dir.path(), executable).is_err());
        assert!(install_at(dir.path(), Path::new("relative/collied")).is_err());
    }
}
