//! Plan usage and context window sizes from Claude Code's status line input.
//! `collied statusline` records them; the daemon reads them back for the phone. Nothing
//! else from the input is kept.

use std::collections::BTreeMap;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::SystemTime;

use protocol::{CodexUsage, PlanUsage, UsageWindow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::peers;

pub const USAGE_FILE: &str = "usage.json";
const USAGE_LOCK: &str = "usage.lock";
const MAX_INPUT: u64 = 1 << 20;
const MAX_FILE: u64 = 64 << 10;
const MAX_SESSIONS: usize = 64;
/// The status line runs several times a second while a reply streams; an unchanged value
/// is stamped again (and the file rewritten) at most this often.
const RESTAMP_MS: u64 = 60_000;
/// Claude Code's windows are 200K and 1M; anything far outside is not a window size.
const WINDOW_RANGE: std::ops::RangeInclusive<u64> = 1_000..=100_000_000;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recorded {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexUsage>,
    /// Context window size per Claude Code session id.
    #[serde(default)]
    pub windows: BTreeMap<String, SessionWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionWindow {
    pub size: u64,
    pub seen_ms: u64,
}

/// The status line command's stdin; an input with nothing to record is not an error.
pub fn record_stdin(data_dir: &Path) -> anyhow::Result<()> {
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take(MAX_INPUT)
        .read_to_end(&mut input)?;
    record(data_dir, &input, crate::now_ms())
}

pub fn record(data_dir: &Path, input: &[u8], now_ms: u64) -> anyhow::Result<()> {
    let v: Value = serde_json::from_slice(input)?;
    let plan = v
        .get("rate_limits")
        .filter(|r| r.is_object())
        .map(|r| PlanUsage {
            five_hour: window(&r["five_hour"]),
            seven_day: window(&r["seven_day"]),
            recorded_ms: now_ms,
            codex: None,
        });
    let session = v["session_id"]
        .as_str()
        .filter(|id| crate::transcript::is_session_id(id))
        .zip(
            v["context_window"]["context_window_size"]
                .as_u64()
                .filter(|n| WINDOW_RANGE.contains(n)),
        );
    if plan.is_none() && session.is_none() {
        return Ok(());
    }
    crate::ensure_private_dir(data_dir)?;
    let _lock = lock(&data_dir.join(USAGE_LOCK))?;
    let path = data_dir.join(USAGE_FILE);
    let mut recorded = match peers::load_json::<Recorded>(&path) {
        Err(peers::Error::Json { .. }) => Recorded::default(),
        other => other?,
    };
    let before = recorded.clone();
    if let Some(plan) = plan {
        recorded.plan = Some(merge_plan(recorded.plan.take(), plan, now_ms));
    }
    if let Some((id, size)) = session
        && !recorded
            .windows
            .get(id)
            .is_some_and(|w| w.size == size && now_ms < w.seen_ms + RESTAMP_MS)
    {
        recorded.windows.insert(
            id.to_owned(),
            SessionWindow {
                size,
                seen_ms: now_ms,
            },
        );
        while recorded.windows.len() > MAX_SESSIONS {
            let Some(oldest) = recorded
                .windows
                .iter()
                .min_by_key(|(_, w)| w.seen_ms)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            recorded.windows.remove(&oldest);
        }
    }
    if recorded != before {
        peers::replace_json(&path, &recorded)?;
    }
    Ok(())
}

/// Each session reports the limits from its own last reply, so a quiet session's older
/// figures must not replace a newer one's: within a window usage only grows, and a later
/// reset is a later window.
fn merge_plan(old: Option<PlanUsage>, new: PlanUsage, now_ms: u64) -> PlanUsage {
    let Some(old) = old else { return new };
    let five_hour = merge_window(old.five_hour.clone(), new.five_hour.clone(), now_ms);
    let seven_day = merge_window(old.seven_day.clone(), new.seven_day.clone(), now_ms);
    let current = (new.five_hour.is_some() || new.seven_day.is_some())
        && [(&new.five_hour, &five_hour), (&new.seven_day, &seven_day)]
            .iter()
            .all(|(n, m)| n.is_none() || n == m);
    let changed = five_hour != old.five_hour || seven_day != old.seven_day;
    let recorded_ms = if current && (changed || now_ms >= old.recorded_ms + RESTAMP_MS) {
        now_ms
    } else {
        old.recorded_ms
    };
    PlanUsage {
        five_hour,
        seven_day,
        recorded_ms,
        codex: None,
    }
}

pub fn record_codex(data_dir: &Path, usage: Option<CodexUsage>) -> anyhow::Result<()> {
    crate::ensure_private_dir(data_dir)?;
    let _lock = lock(&data_dir.join(USAGE_LOCK))?;
    let path = data_dir.join(USAGE_FILE);
    let mut recorded = match peers::load_json::<Recorded>(&path) {
        Err(peers::Error::Json { .. }) => Recorded::default(),
        other => other?,
    };
    if recorded.codex.is_none() && usage.is_none() {
        return Ok(());
    }
    if recorded
        .codex
        .as_ref()
        .zip(usage.as_ref())
        .is_some_and(|(old, new)| {
            old.used == new.used
                && old.limit == new.limit
                && old.resets_at_ms == new.resets_at_ms
                && new.recorded_ms < old.recorded_ms + RESTAMP_MS
        })
    {
        return Ok(());
    }
    recorded.codex = usage;
    peers::replace_json(&path, &recorded)?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub struct CodexUsageError;

pub fn parse_codex_usage(
    response: &[u8],
    now_ms: u64,
) -> Result<Option<CodexUsage>, CodexUsageError> {
    let root: Value = serde_json::from_slice(response).map_err(|_| CodexUsageError)?;
    let limits = root["result"].get("rateLimits").ok_or(CodexUsageError)?;
    if limits.is_null() {
        return Ok(None);
    }
    let limits = limits.as_object().ok_or(CodexUsageError)?;
    let Some(limit) = limits.get("individualLimit").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let number = |v: &Value| v.as_f64().or_else(|| v.as_str()?.parse().ok());
    let allowance = number(&limit["limit"])
        .filter(|n| n.is_finite() && *n >= 1.0 && *n < 1e9)
        .ok_or(CodexUsageError)?;
    let used = number(&limit["used"])
        .filter(|n| n.is_finite() && *n >= 0.0 && *n < 1e9)
        .ok_or(CodexUsageError)?;
    let resets = limit["resetsAt"].as_u64().ok_or(CodexUsageError)?;
    let resets_at_ms = resets.checked_mul(1000).ok_or(CodexUsageError)?;
    if resets_at_ms <= now_ms {
        return Err(CodexUsageError);
    }
    Ok(Some(CodexUsage {
        used: used.round() as u64,
        limit: allowance.round() as u64,
        resets_at_ms,
        recorded_ms: now_ms,
    }))
}

fn codex_account(response: &Value) -> Result<bool, CodexUsageError> {
    let account = response["result"].get("account").ok_or(CodexUsageError)?;
    if account.is_null() {
        Ok(false)
    } else {
        account.as_object().map(|_| true).ok_or(CodexUsageError)
    }
}

pub async fn fetch_codex_usage() -> Result<Option<CodexUsage>, CodexUsageError> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .filter(|path| path.is_absolute())
        .collect();
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    if let Ok(home) = crate::config::home_dir() {
        dirs.extend([home.join(".cargo/bin"), home.join(".local/bin")]);
    }
    let bin = dirs
        .into_iter()
        .map(|dir| dir.join("codex"))
        .find(|path| path.is_file())
        .ok_or(CodexUsageError)?;
    let mut child = tokio::process::Command::new(bin)
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| CodexUsageError)?;
    let mut stdin = child.stdin.take().ok_or(CodexUsageError)?;
    stdin
        .write_all(b"{\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"collie\",\"version\":\"0.1\"}}}\n")
        .await
        .map_err(|_| CodexUsageError)?;
    stdin.flush().await.map_err(|_| CodexUsageError)?;
    let stdout = child.stdout.take().ok_or(CodexUsageError)?;
    let mut lines = BufReader::new(stdout).lines();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let line = lines.next_line().await.map_err(|_| CodexUsageError)?.ok_or(CodexUsageError)?;
            let response: Value = serde_json::from_str(&line).map_err(|_| CodexUsageError)?;
            if response["id"] == 1 {
                response["result"].as_object().ok_or(CodexUsageError)?;
                break;
            }
        }
        stdin
            .write_all(b"{\"method\":\"initialized\",\"params\":{}}\n{\"id\":2,\"method\":\"account/read\",\"params\":{}}\n")
            .await
            .map_err(|_| CodexUsageError)?;
        stdin.flush().await.map_err(|_| CodexUsageError)?;
        loop {
            let line = lines.next_line().await.map_err(|_| CodexUsageError)?.ok_or(CodexUsageError)?;
            let response: Value = serde_json::from_str(&line).map_err(|_| CodexUsageError)?;
            if response["id"] == 2 {
                if !codex_account(&response)? {
                    return Ok(None);
                }
                break;
            }
        }
        stdin
            .write_all(b"{\"id\":3,\"method\":\"account/rateLimits/read\",\"params\":{}}\n")
            .await
            .map_err(|_| CodexUsageError)?;
        stdin.flush().await.map_err(|_| CodexUsageError)?;
        loop {
            let line = lines.next_line().await.map_err(|_| CodexUsageError)?.ok_or(CodexUsageError)?;
            let response: Value = serde_json::from_str(&line).map_err(|_| CodexUsageError)?;
            if response["id"] == 3 {
                return parse_codex_usage(line.as_bytes(), crate::now_ms());
            }
        }
    })
    .await
    .map_err(|_| CodexUsageError)?
}

fn merge_window(
    old: Option<UsageWindow>,
    new: Option<UsageWindow>,
    now_ms: u64,
) -> Option<UsageWindow> {
    let live = |w: &UsageWindow| w.resets_at_ms > now_ms;
    match (old.filter(live), new.filter(live)) {
        (Some(o), Some(n)) => Some(match o.resets_at_ms.cmp(&n.resets_at_ms) {
            std::cmp::Ordering::Less => n,
            std::cmp::Ordering::Greater => o,
            std::cmp::Ordering::Equal if n.used_percent >= o.used_percent => n,
            std::cmp::Ordering::Equal => o,
        }),
        (o, n) => n.or(o),
    }
}

fn window(v: &Value) -> Option<UsageWindow> {
    let used = v["used_percentage"].as_f64().filter(|p| p.is_finite())?;
    let resets = v["resets_at"]
        .as_f64()
        .filter(|s| s.is_finite() && *s > 0.0 && *s < 1e11)?;
    Some(UsageWindow {
        used_percent: used.round().clamp(0.0, 100.0) as u8,
        resets_at_ms: (resets * 1000.0) as u64,
    })
}

/// Concurrent sessions' status lines merge into the same file.
fn lock(path: &Path) -> anyhow::Result<OwnedFd> {
    use rustix::fs::{FlockOperation, Mode, OFlags};
    let fd = rustix::fs::open(
        path,
        OFlags::CREATE | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )?;
    rustix::fs::flock(&fd, FlockOperation::LockExclusive)?;
    Ok(fd)
}

/// The daemon's view of the file, read again only when its size or mtime changes.
#[derive(Default)]
pub struct Usage {
    path: Option<PathBuf>,
    stamp: Option<(u64, SystemTime)>,
    recorded: Recorded,
    announced: Option<PlanUsage>,
}

impl Usage {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            ..Self::default()
        }
    }

    pub fn refresh(&mut self) {
        let Some((file, len, modified)) = self
            .path
            .as_deref()
            .and_then(crate::transcript::open)
            .filter(|(_, len, _)| *len <= MAX_FILE)
        else {
            self.stamp = None;
            self.recorded = Recorded::default();
            return;
        };
        if self.stamp == Some((len, modified)) {
            return;
        }
        let mut text = Vec::new();
        self.recorded = file
            .take(MAX_FILE)
            .read_to_end(&mut text)
            .ok()
            .and_then(|_| serde_json::from_slice(&text).ok())
            .unwrap_or_default();
        self.stamp = Some((len, modified));
    }

    pub fn plan(&self) -> Option<PlanUsage> {
        let mut plan = self.recorded.plan.clone().unwrap_or(PlanUsage {
            five_hour: None,
            seven_day: None,
            recorded_ms: 0,
            codex: None,
        });
        plan.codex = self.recorded.codex.clone();
        (self.recorded.plan.is_some() || plan.codex.is_some()).then_some(plan)
    }

    /// True once per change of the recorded plan, its stamp included.
    pub fn plan_moved(&mut self) -> bool {
        self.refresh();
        let plan = self.plan();
        let moved = self.announced != plan;
        if moved {
            self.announced = plan;
        }
        moved
    }

    pub fn window(&self, session_id: &str) -> Option<u64> {
        self.recorded
            .windows
            .get(session_id)
            .map(|w| w.size)
            .filter(|n| WINDOW_RANGE.contains(n))
    }
}

/// For `collied doctor`: when the tap last recorded plan usage, if it has.
pub fn last_recorded(data_dir: &Path) -> Option<Recorded> {
    let mut usage = Usage::new(data_dir.join(USAGE_FILE));
    usage.refresh();
    usage.stamp.map(|_| usage.recorded)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    const INPUT: &str = include_str!("../tests/fixtures/statusline.json");
    const SESSION: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

    fn data() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("collie");
        (dir, data)
    }

    fn read(dir: &Path) -> Recorded {
        peers::load_json(&dir.join(USAGE_FILE)).unwrap()
    }

    #[test]
    fn records_rate_limits_and_the_window() {
        let (_dir, data) = data();
        record(&data, INPUT.as_bytes(), 1_000).unwrap();
        let r = read(&data);
        assert_eq!(
            r.plan,
            Some(PlanUsage {
                five_hour: Some(UsageWindow {
                    used_percent: 24,
                    resets_at_ms: 1_738_425_600_000
                }),
                seven_day: Some(UsageWindow {
                    used_percent: 41,
                    resets_at_ms: 1_738_857_600_000
                }),
                recorded_ms: 1_000,
                codex: None,
            })
        );
        assert_eq!(r.windows[SESSION].size, 1_000_000);
        let meta = std::fs::metadata(data.join(USAGE_FILE)).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let text = std::fs::read_to_string(data.join(USAGE_FILE)).unwrap();
        for kept_out in ["cwd", "cost", "transcript", "model", "spend", "314"] {
            assert!(!text.contains(kept_out), "{kept_out} leaked into the file");
        }
    }

    #[test]
    fn codex_monthly_limit_preserves_claude_usage() {
        let (_dir, data) = data();
        record(&data, INPUT.as_bytes(), 1_000).unwrap();
        let response = br#"{"id":2,"result":{"rateLimits":{"primary":null,"secondary":null,"individualLimit":{"limit":"10000","used":"1531.7076","remainingPercent":85,"resetsAt":1793491200}}}}"#;
        let codex = parse_codex_usage(response, 1_000).unwrap().unwrap();
        assert_eq!(codex.used, 1532);
        assert_eq!(codex.limit, 10_000);
        record_codex(&data, Some(codex.clone())).unwrap();
        let mut usage = Usage::new(data.join(USAGE_FILE));
        usage.refresh();
        let plan = usage.plan().unwrap();
        assert_eq!(plan.five_hour.unwrap().used_percent, 24);
        assert_eq!(plan.codex, Some(codex));
        assert!(
            parse_codex_usage(
                br#"{"id":2,"result":{"rateLimits":{"individualLimit":null}}}"#,
                1_000
            )
            .unwrap()
            .is_none()
        );
        assert!(parse_codex_usage(br#"{"id":2,"error":{"message":"offline"}}"#, 1_000).is_err());
        record_codex(&data, None).unwrap();
        usage.refresh();
        assert!(usage.plan().unwrap().codex.is_none());
        assert_eq!(usage.plan().unwrap().five_hour.unwrap().used_percent, 24);
    }

    #[test]
    fn codex_only_usage_clears_after_sign_out() {
        let (_dir, data) = data();
        let response: Value =
            serde_json::from_str(r#"{"id":2,"result":{"account":null}}"#).unwrap();
        assert_eq!(codex_account(&response), Ok(false));
        let failed: Value =
            serde_json::from_str(r#"{"id":2,"error":{"message":"offline"}}"#).unwrap();
        assert!(codex_account(&failed).is_err());
        record_codex(
            &data,
            Some(CodexUsage {
                used: 100,
                limit: 1000,
                resets_at_ms: 1_000_000,
                recorded_ms: 10,
            }),
        )
        .unwrap();
        let mut usage = Usage::new(data.join(USAGE_FILE));
        usage.refresh();
        assert!(usage.plan_moved());
        assert!(usage.plan().is_some());
        record_codex(&data, None).unwrap();
        assert!(usage.plan_moved());
        assert!(usage.plan().is_none());
    }

    #[test]
    fn input_without_limits_keeps_the_last_ones() {
        let (_dir, data) = data();
        record(&data, INPUT.as_bytes(), 1_000).unwrap();
        let mut v: Value = serde_json::from_str(INPUT).unwrap();
        v.as_object_mut().unwrap().remove("rate_limits");
        v["session_id"] = "01a10f07-b5ed-79a3-989d-fa10b96c5cab".into();
        v["context_window"]["context_window_size"] = 200_000.into();
        record(&data, v.to_string().as_bytes(), 2_000).unwrap();
        let r = read(&data);
        assert_eq!(r.plan.unwrap().recorded_ms, 1_000);
        assert_eq!(r.windows.len(), 2);
        // Claude Code drops a window once it resets; one still open is kept.
        v["rate_limits"] =
            serde_json::json!({"seven_day": {"used_percentage": 0.4, "resets_at": 1838857600}});
        record(&data, v.to_string().as_bytes(), 3_000).unwrap();
        assert_eq!(
            read(&data).plan.unwrap().five_hour.unwrap().used_percent,
            24
        );
        record(&data, v.to_string().as_bytes(), 1_738_425_600_000).unwrap();
        let plan = read(&data).plan.unwrap();
        assert_eq!(plan.five_hour, None);
        assert_eq!(plan.seven_day.unwrap().used_percent, 0);
    }

    fn limits(five: f64, five_resets: u64, seven: f64) -> Vec<u8> {
        serde_json::json!({"rate_limits": {
            "five_hour": {"used_percentage": five, "resets_at": five_resets},
            "seven_day": {"used_percentage": seven, "resets_at": 1738857600},
        }})
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_quiet_session_does_not_replace_newer_figures() {
        let (_dir, data) = data();
        let plan = || {
            let p = read(&data).plan.unwrap();
            (
                p.five_hour.unwrap().used_percent,
                p.seven_day.unwrap().used_percent,
                p.recorded_ms,
            )
        };
        // Session B's reply at 10:00, then session A re-runs with its 09:40 figures.
        record(&data, &limits(92.0, 1738425600, 45.0), 100_000).unwrap();
        record(&data, &limits(70.0, 1738425600, 41.0), 160_000).unwrap();
        assert_eq!(plan(), (92, 45, 100_000));
        // A later window wins even with less used.
        record(&data, &limits(3.0, 1738443600, 46.0), 170_000).unwrap();
        assert_eq!(plan(), (3, 46, 170_000));
        record(&data, &limits(92.0, 1738425600, 46.0), 400_000).unwrap();
        assert_eq!(plan(), (3, 46, 170_000));
    }

    #[test]
    fn an_unchanged_refresh_leaves_the_file_alone() {
        let (_dir, data) = data();
        let path = data.join(USAGE_FILE);
        record(&data, INPUT.as_bytes(), 1_000).unwrap();
        let written = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();
        record(&data, INPUT.as_bytes(), 1_000 + RESTAMP_MS - 1).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.modified().unwrap(), SystemTime::UNIX_EPOCH);
        assert_ne!(written, SystemTime::UNIX_EPOCH);
        record(&data, INPUT.as_bytes(), 1_000 + RESTAMP_MS).unwrap();
        let r = read(&data);
        assert_eq!(r.plan.unwrap().recorded_ms, 1_000 + RESTAMP_MS);
        assert_eq!(r.windows[SESSION].seen_ms, 1_000 + RESTAMP_MS);
        assert!(!data.join("usage.json.tmp").exists());
    }

    #[test]
    fn refuses_what_is_not_status_line_data() {
        let (_dir, data) = data();
        assert!(record(&data, b"not json", 1).is_err());
        record(
            &data,
            br#"{"session_id":"../../etc","context_window":{"context_window_size":200000}}"#,
            1,
        )
        .unwrap();
        record(&data, br#"{"session_id":"3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d","context_window":{"context_window_size":7}}"#, 1).unwrap();
        assert!(!data.join(USAGE_FILE).exists());
        let bad = br#"{"rate_limits":{"five_hour":{"used_percentage":"lots","resets_at":1},"seven_day":{"used_percentage":250.0,"resets_at":1738857600}}}"#;
        record(&data, bad, 1).unwrap();
        let plan = read(&data).plan.unwrap();
        assert_eq!(plan.five_hour, None);
        assert_eq!(plan.seven_day.unwrap().used_percent, 100);
    }

    #[test]
    fn keeps_the_latest_sessions() {
        let (_dir, data) = data();
        for i in 0..(MAX_SESSIONS as u64 + 3) {
            let input = format!(
                r#"{{"session_id":"00000000-0000-4000-8000-{i:012}","context_window":{{"context_window_size":200000}}}}"#
            );
            record(&data, input.as_bytes(), i).unwrap();
        }
        let r = read(&data);
        assert_eq!(r.windows.len(), MAX_SESSIONS);
        assert!(
            !r.windows
                .contains_key("00000000-0000-4000-8000-000000000002")
        );
        assert!(
            r.windows
                .contains_key("00000000-0000-4000-8000-000000000003")
        );
    }

    #[test]
    fn the_daemon_reads_it_back_by_mtime() {
        let (_dir, data) = data();
        let mut usage = Usage::new(data.join(USAGE_FILE));
        usage.refresh();
        assert_eq!((usage.plan(), usage.window(SESSION)), (None, None));
        assert!(last_recorded(&data).is_none());
        record(&data, INPUT.as_bytes(), 5).unwrap();
        usage.refresh();
        assert_eq!(usage.plan().unwrap().recorded_ms, 5);
        assert_eq!(usage.window(SESSION), Some(1_000_000));
        assert_eq!(last_recorded(&data).unwrap().plan.unwrap().recorded_ms, 5);
        assert!(usage.plan_moved());
        assert!(!usage.plan_moved());

        // Same size and mtime: not read again.
        let path = data.join(USAGE_FILE);
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let edited = text.replace("\"used_percent\": 24", "\"used_percent\": 25");
        assert_eq!(edited.len(), text.len());
        std::fs::write(&path, &edited).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(modified).unwrap();
        usage.refresh();
        assert_eq!(usage.plan().unwrap().five_hour.unwrap().used_percent, 24);
        assert!(!usage.plan_moved());
        file.set_modified(modified + std::time::Duration::from_secs(1))
            .unwrap();
        assert!(usage.plan_moved());
        assert_eq!(usage.plan().unwrap().five_hour.unwrap().used_percent, 25);

        std::fs::remove_file(data.join(USAGE_FILE)).unwrap();
        std::os::unix::fs::symlink(data.join(USAGE_LOCK), data.join(USAGE_FILE)).unwrap();
        usage.refresh();
        assert_eq!(usage.plan(), None);
    }
}
