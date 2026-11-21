//! Plan usage and context window sizes from Claude Code's status line input.
//! `collied statusline` records them; the daemon reads them back for the phone. Nothing
//! else from the input is kept.

use std::collections::BTreeMap;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::SystemTime;

use protocol::{AgyQuotaBucket, AgyUsage, CodexUsage, CopilotUsage, PlanUsage, UsageWindow};
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
const MAX_AGY_BUCKETS: usize = 16;
const MAX_AGY_BUCKET_ID: usize = 64;
const MAX_AGY_RESET_SPAN_MS: u64 = 366 * 86_400_000;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recorded {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot: Option<CopilotUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agy: Option<AgyUsage>,
    /// Context window size and last reported limits per Claude Code or Antigravity session id.
    #[serde(default)]
    pub windows: BTreeMap<String, SessionWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionWindow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub seen_ms: u64,
    /// None until the session reports limits: an entry from before they were kept, or a
    /// session before its first reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<SessionLimits>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<UsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<UsageWindow>,
}

#[derive(Clone, Copy, PartialEq)]
enum Report {
    Unnamed,
    /// A session with no limits on record: not told from a quiet session's old figures.
    First,
    Repeat,
    /// Differs from the session's last report: the session received a reply.
    New,
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
    let limits = v
        .get("rate_limits")
        .filter(|r| r.is_object())
        .map(|r| SessionLimits {
            five_hour: window(&r["five_hour"]),
            seven_day: window(&r["seven_day"]),
        });
    let agy = parse_agy_usage(&v, now_ms);
    let id = v["session_id"]
        .as_str()
        .filter(|id| crate::transcript::is_session_id(id));
    let size = v["context_window"]["context_window_size"]
        .as_u64()
        .filter(|n| WINDOW_RANGE.contains(n));
    if limits.is_none() && agy.is_none() && (id.is_none() || size.is_none()) {
        return Ok(());
    }
    update(data_dir, |recorded| {
        let before = recorded.clone();
        let session = id.and_then(|id| recorded.windows.get(id));
        if let Some(new) = &limits {
            let report = match (id, session.and_then(|s| s.limits.as_ref())) {
                (None, _) => Report::Unnamed,
                (_, None) => Report::First,
                // A window the report leaves out or that is past its reset is no change:
                // every session's status line drops a window when it resets.
                (_, Some(last))
                    if [
                        (&new.five_hour, &last.five_hour),
                        (&new.seven_day, &last.seven_day),
                    ]
                    .iter()
                    .any(|(n, l)| {
                        n != l && n.as_ref().is_some_and(|w| w.resets_at_ms > now_ms)
                    }) =>
                {
                    Report::New
                }
                _ => Report::Repeat,
            };
            recorded.plan = Some(merge_plan(recorded.plan.take(), new, report, now_ms));
        }
        if let Some(new) = agy
            && !recorded.agy.as_ref().is_some_and(|old| {
                old.buckets == new.buckets && new.recorded_ms < old.recorded_ms + RESTAMP_MS
            })
        {
            recorded.agy = Some(new);
        }
        let size = size.or(session.and_then(|s| s.size));
        let limits = limits.or_else(|| session.and_then(|s| s.limits.clone()));
        if let Some(id) = id
            && !session.is_some_and(|s| {
                s.size == size && s.limits == limits && now_ms < s.seen_ms + RESTAMP_MS
            })
        {
            recorded.windows.insert(
                id.to_owned(),
                SessionWindow {
                    size,
                    seen_ms: now_ms,
                    limits,
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
        *recorded != before
    })
}

/// Each session reports the limits from its own last reply, so a quiet session's older
/// figures must not replace a newer one's. A plan's usage can be reset before its window
/// ends, so a lower figure or an earlier reset is not older: only a change in a session's
/// own report is a new reading.
fn merge_plan(
    old: Option<PlanUsage>,
    new: &SessionLimits,
    report: Report,
    now_ms: u64,
) -> PlanUsage {
    let Some(old) = old else {
        return PlanUsage {
            five_hour: new.five_hour.clone(),
            seven_day: new.seven_day.clone(),
            recorded_ms: now_ms,
            codex: None,
            copilot: None,
            agy: None,
        };
    };
    let five_hour = merge_window(old.five_hour.clone(), new.five_hour.clone(), report, now_ms);
    let seven_day = merge_window(old.seven_day.clone(), new.seven_day.clone(), report, now_ms);
    let current = (new.five_hour.is_some() || new.seven_day.is_some())
        && [(&new.five_hour, &five_hour), (&new.seven_day, &seven_day)]
            .iter()
            .all(|(n, m)| n.is_none() || n == m);
    let changed = five_hour != old.five_hour || seven_day != old.seven_day;
    let recorded_ms = if report == Report::New
        || current && (changed || now_ms >= old.recorded_ms + RESTAMP_MS)
    {
        now_ms
    } else {
        old.recorded_ms
    };
    PlanUsage {
        five_hour,
        seven_day,
        recorded_ms,
        codex: None,
        copilot: None,
        agy: None,
    }
}

pub fn record_codex(data_dir: &Path, usage: Option<CodexUsage>) -> anyhow::Result<()> {
    update(data_dir, |recorded| {
        if recorded.codex.is_none() && usage.is_none() {
            return false;
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
            return false;
        }
        recorded.codex = usage;
        true
    })
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

/// `name` in collied's `PATH`, then where installers put it, as an absolute path.
pub(crate) fn program(name: &str) -> Option<PathBuf> {
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
    dirs.into_iter()
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

pub async fn fetch_codex_usage() -> Result<Option<CodexUsage>, CodexUsageError> {
    let bin = program("codex").ok_or(CodexUsageError)?;
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

pub fn record_copilot(data_dir: &Path, usage: Option<CopilotUsage>) -> anyhow::Result<()> {
    update(data_dir, |recorded| {
        if recorded.copilot.is_none() && usage.is_none() {
            return false;
        }
        if recorded
            .copilot
            .as_ref()
            .zip(usage.as_ref())
            .is_some_and(|(old, new)| {
                *old == CopilotUsage {
                    recorded_ms: old.recorded_ms,
                    ..new.clone()
                } && new.recorded_ms < old.recorded_ms + RESTAMP_MS
            })
        {
            return false;
        }
        recorded.copilot = usage;
        true
    })
}

/// Reduced inside `gh` by `--jq`, so the login, ids, organizations and endpoints in the
/// answer never reach collied.
const COPILOT_FIELDS: &str = "{token_based_billing, quota_reset_date, quota_reset_date_utc, quota_snapshots: {premium_interactions: .quota_snapshots.premium_interactions}}";

/// github.com only. No `gh`, no login (exit code 4) and no Copilot seat (404) are `None`.
pub async fn fetch_copilot_usage() -> Result<Option<CopilotUsage>, &'static str> {
    let Some(gh) = crate::drive::gh() else {
        return Ok(None);
    };
    let mut command = crate::pulls::gh_command(&gh);
    command
        .args([
            "api",
            "--hostname",
            "github.com",
            "copilot_internal/user",
            "--jq",
            COPILOT_FIELDS,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = tokio::time::timeout(crate::pulls::GH_TIMEOUT, command.output())
        .await
        .map_err(|_| "timed out")?
        .map_err(|_| "unavailable")?;
    if output.status.success() {
        return parse_copilot_usage(&output.stdout, crate::now_ms());
    }
    if output.status.code() == Some(4)
        || String::from_utf8_lossy(&output.stderr).contains("(HTTP 404)")
    {
        return Ok(None);
    }
    Err("request failed")
}

pub fn parse_copilot_usage(
    response: &[u8],
    now_ms: u64,
) -> Result<Option<CopilotUsage>, &'static str> {
    const BAD: &str = "unreadable response";
    let root: Value = serde_json::from_slice(response).map_err(|_| BAD)?;
    root.as_object().ok_or(BAD)?;
    let premium = &root["quota_snapshots"]["premium_interactions"];
    if premium.is_null() {
        return Ok(None);
    }
    premium.as_object().ok_or(BAD)?;
    let resets_at_ms = match root["quota_reset_date_utc"].as_str() {
        Some(at) => crate::transcript::timestamp_ms(at),
        None => root["quota_reset_date"]
            .as_str()
            .and_then(|day| crate::transcript::timestamp_ms(&format!("{day}T00:00:00Z"))),
    }
    .filter(|ms| *ms > now_ms)
    .ok_or(BAD)?;
    let token_based_billing = root["token_based_billing"].as_bool().unwrap_or(false);
    if premium["unlimited"].as_bool().ok_or(BAD)? {
        return Ok(Some(CopilotUsage {
            used: 0,
            limit: 0,
            unlimited: true,
            token_based_billing,
            resets_at_ms,
            recorded_ms: now_ms,
        }));
    }
    let number = |v: &Value| v.as_f64().filter(|n| n.is_finite() && n.abs() < 1e9);
    let entitlement = number(&premium["entitlement"])
        .filter(|n| *n >= 0.0)
        .ok_or(BAD)?;
    if entitlement < 1.0 {
        return Ok(None);
    }
    // Past the allowance, a seat that permits overage reports a negative `remaining` or
    // a `remaining` held at 0 with the excess in `overage_count`.
    let remaining = number(&premium["remaining"]).ok_or(BAD)?.max(0.0);
    let overage = match premium.get("overage_count") {
        None => 0.0,
        Some(v) => number(v).filter(|n| *n >= 0.0).ok_or(BAD)?,
    };
    Ok(Some(CopilotUsage {
        used: (entitlement - remaining + overage).max(0.0).round() as u64,
        limit: entitlement.round() as u64,
        unlimited: false,
        token_based_billing,
        resets_at_ms,
        recorded_ms: now_ms,
    }))
}

fn merge_window(
    old: Option<UsageWindow>,
    new: Option<UsageWindow>,
    report: Report,
    now_ms: u64,
) -> Option<UsageWindow> {
    let live = |w: &UsageWindow| w.resets_at_ms > now_ms;
    match (report, old.filter(live), new.filter(live)) {
        (Report::Repeat, o, _) => o,
        (Report::First, o, n) => o.or(n),
        (Report::Unnamed, Some(o), Some(n)) => Some(match o.resets_at_ms.cmp(&n.resets_at_ms) {
            std::cmp::Ordering::Less => n,
            std::cmp::Ordering::Greater => o,
            std::cmp::Ordering::Equal if n.used_percent >= o.used_percent => n,
            std::cmp::Ordering::Equal => o,
        }),
        (_, o, n) => n.or(o),
    }
}

fn parse_agy_usage(v: &Value, now_ms: u64) -> Option<AgyUsage> {
    let map = v.get("quota")?.as_object()?;
    let mut buckets = Vec::new();
    for (id, entry) in map {
        if id.is_empty()
            || id.len() > MAX_AGY_BUCKET_ID
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            continue;
        }
        if entry["disabled"].as_bool() == Some(true) {
            continue;
        }
        let Some(remaining) = entry["remaining_fraction"]
            .as_f64()
            .filter(|f| f.is_finite() && (0.0..=1.0).contains(f))
        else {
            continue;
        };
        let resets_at_ms = entry["reset_time"]
            .as_str()
            .and_then(crate::transcript::timestamp_ms)
            .or_else(|| {
                entry["reset_in_seconds"]
                    .as_u64()
                    .filter(|&s| s > 0)
                    .and_then(|s| s.checked_mul(1000))
                    .and_then(|ms| now_ms.checked_add(ms))
            })
            .filter(|&ms| ms > now_ms && ms <= now_ms.saturating_add(MAX_AGY_RESET_SPAN_MS));
        let Some(resets_at_ms) = resets_at_ms else {
            continue;
        };
        let window_seconds = if id.ends_with("-5h") {
            Some(5 * 3600)
        } else if id.ends_with("-weekly") {
            Some(7 * 86_400)
        } else {
            None
        };
        buckets.push(AgyQuotaBucket {
            id: id.clone(),
            used_percent: ((1.0 - remaining) * 100.0).round().clamp(0.0, 100.0) as u8,
            resets_at_ms,
            window_seconds,
        });
    }
    if buckets.is_empty() {
        return None;
    }
    buckets.sort_by(|a, b| agy_bucket_order(&a.id).cmp(&agy_bucket_order(&b.id)));
    buckets.truncate(MAX_AGY_BUCKETS);
    Some(AgyUsage {
        buckets,
        recorded_ms: now_ms,
    })
}

fn agy_bucket_order(id: &str) -> (u8, &str) {
    let rank = match id {
        "gemini-5h" => 0,
        "gemini-weekly" => 1,
        "3p-5h" => 2,
        "3p-weekly" => 3,
        _ => 4,
    };
    (rank, id)
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

fn update(data_dir: &Path, change: impl FnOnce(&mut Recorded) -> bool) -> anyhow::Result<()> {
    crate::ensure_private_dir(data_dir)?;
    let _lock = lock(&data_dir.join(USAGE_LOCK))?;
    let path = data_dir.join(USAGE_FILE);
    let mut recorded = match peers::load_json::<Recorded>(&path) {
        Err(peers::Error::Json { .. }) => Recorded::default(),
        other => other?,
    };
    if change(&mut recorded) {
        peers::replace_json(&path, &recorded)?;
    }
    Ok(())
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
            copilot: None,
            agy: None,
        });
        plan.codex = self.recorded.codex.clone();
        plan.copilot = self.recorded.copilot.clone();
        plan.agy = self.recorded.agy.clone();
        (self.recorded.plan.is_some()
            || plan.codex.is_some()
            || plan.copilot.is_some()
            || plan.agy.is_some())
        .then_some(plan)
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
            .and_then(|w| w.size)
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
                copilot: None,
                agy: None,
            })
        );
        assert_eq!(r.windows[SESSION].size, Some(1_000_000));
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

    fn copilot(name: &str, now_ms: u64) -> Result<Option<CopilotUsage>, &'static str> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/gh")
            .join(name);
        parse_copilot_usage(&std::fs::read(path).unwrap(), now_ms)
    }

    #[test]
    fn copilot_premium_usage_keeps_only_the_quota() {
        let resets_at_ms = 1_793_491_200_000;
        let usage = |used, limit, unlimited, token_based_billing| {
            Ok(Some(CopilotUsage {
                used,
                limit,
                unlimited,
                token_based_billing,
                resets_at_ms,
                recorded_ms: 1_000,
            }))
        };
        assert_eq!(
            copilot("copilot-enterprise.json", 1_000),
            usage(239_413, 400_000, false, true)
        );
        assert_eq!(
            copilot("copilot-requests.json", 1_000),
            usage(179, 300, false, false)
        );
        assert_eq!(
            copilot("copilot-unlimited.json", 1_000),
            usage(0, 0, true, false)
        );
        assert_eq!(
            copilot("copilot-overage.json", 1_000),
            usage(412_500, 400_000, false, true)
        );
        assert_eq!(
            copilot("copilot-overage-clamped.json", 1_000),
            usage(412_500, 400_000, false, true)
        );
        assert_eq!(copilot("copilot-no-premium.json", 1_000), Ok(None));
        for bad in [
            &b"not json"[..],
            b"[]",
            br#"{"quota_snapshots":{"premium_interactions":"lots"}}"#,
            br#"{"quota_reset_date":"2026-11-01","quota_snapshots":{"premium_interactions":{"unlimited":false,"entitlement":"300","remaining":1}}}"#,
            br#"{"quota_snapshots":{"premium_interactions":{"unlimited":false,"entitlement":300,"remaining":1}}}"#,
        ] {
            assert!(parse_copilot_usage(bad, 1_000).is_err());
        }
        assert!(copilot("copilot-enterprise.json", resets_at_ms).is_err());
    }

    #[test]
    fn copilot_usage_preserves_the_others_and_clears() {
        let (_dir, data) = data();
        record(&data, INPUT.as_bytes(), 1_000).unwrap();
        let reading = copilot("copilot-enterprise.json", 1_000).unwrap();
        record_copilot(&data, reading.clone()).unwrap();
        let mut usage = Usage::new(data.join(USAGE_FILE));
        usage.refresh();
        let plan = usage.plan().unwrap();
        assert_eq!(plan.five_hour.unwrap().used_percent, 24);
        assert_eq!(plan.copilot, reading);
        let text = std::fs::read_to_string(data.join(USAGE_FILE)).unwrap();
        for kept_out in ["enterprise", "overage", "credits_used", "percent_remaining"] {
            assert!(!text.contains(kept_out), "{kept_out} leaked into the file");
        }
        record_copilot(&data, None).unwrap();
        usage.refresh();
        assert!(usage.plan().unwrap().copilot.is_none());
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
        assert_eq!(plan.seven_day.unwrap().used_percent, 41);
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

    const A: &str = "0000000a-0000-4000-8000-000000000000";
    const B: &str = "0000000b-0000-4000-8000-000000000000";
    const FIVE: u64 = 1_738_425_600;
    const WEEK: u64 = 1_738_857_600;

    fn report(session: &str, five: (f64, u64), seven: (f64, u64)) -> Vec<u8> {
        serde_json::json!({"session_id": session, "rate_limits": {
            "five_hour": {"used_percentage": five.0, "resets_at": five.1},
            "seven_day": {"used_percentage": seven.0, "resets_at": seven.1},
        }})
        .to_string()
        .into_bytes()
    }

    fn figures(data: &Path) -> (Option<UsageWindow>, Option<UsageWindow>, u64) {
        let p = read(data).plan.unwrap();
        (p.five_hour, p.seven_day, p.recorded_ms)
    }

    fn at(used_percent: u8, resets: u64) -> Option<UsageWindow> {
        Some(UsageWindow {
            used_percent,
            resets_at_ms: resets * 1000,
        })
    }

    /// Two sessions known and the weekly at 99, then B's reply after the plan's usage was
    /// reset, with the weekly resetting at `resets`.
    fn a_reply_after_a_reset(resets: u64) {
        let (_dir, data) = data();
        record(&data, &report(A, (50.0, FIVE), (98.0, WEEK)), 100_000).unwrap();
        record(&data, &report(B, (50.0, FIVE), (98.0, WEEK)), 110_000).unwrap();
        record(&data, &report(A, (60.0, FIVE), (99.0, WEEK)), 120_000).unwrap();
        assert_eq!(figures(&data), (at(60, FIVE), at(99, WEEK), 120_000));
        record(&data, &report(B, (2.0, FIVE), (1.0, resets)), 150_000).unwrap();
        let reset = (at(2, FIVE), at(1, resets), 150_000);
        assert_eq!(figures(&data), reset);
        // While the reply streams, the same report is no new reading.
        record(&data, &report(B, (2.0, FIVE), (1.0, resets)), 150_001).unwrap();
        assert_eq!(figures(&data), reset);
        // A is quiet and repeats the figures of its last reply.
        record(&data, &report(A, (60.0, FIVE), (99.0, WEEK)), 300_000).unwrap();
        assert_eq!(figures(&data), reset);
        // The five-hour window resets and Claude Code drops it from A's input.
        let weekly = serde_json::json!({"session_id": A, "rate_limits": {
            "seven_day": {"used_percentage": 99.0, "resets_at": WEEK},
        }});
        record(&data, weekly.to_string().as_bytes(), FIVE * 1000).unwrap();
        assert_eq!(figures(&data), (None, at(1, resets), 150_000));
    }

    #[test]
    fn a_reply_after_an_early_reset_replaces_the_figures() {
        a_reply_after_a_reset(WEEK);
    }

    #[test]
    fn a_reply_after_an_early_reset_replaces_a_later_reset_time() {
        a_reply_after_a_reset(WEEK - 86_400);
    }

    #[test]
    fn a_first_report_fills_only_a_window_with_no_live_value() {
        let (_dir, data) = data();
        record(&data, &report(A, (60.0, FIVE), (99.0, WEEK)), 100_000).unwrap();
        assert_eq!(figures(&data), (at(60, FIVE), at(99, WEEK), 100_000));
        // Not told from a quiet session's old figures, later reset times or not.
        let later = report(B, (10.0, FIVE + 18_000), (5.0, WEEK + 604_800));
        record(&data, &later, 110_000).unwrap();
        assert_eq!(figures(&data), (at(60, FIVE), at(99, WEEK), 100_000));
        let unseen = report(SESSION, (10.0, FIVE + 18_000), (5.0, WEEK + 604_800));
        record(&data, &unseen, FIVE * 1000).unwrap();
        assert_eq!(
            figures(&data),
            (at(10, FIVE + 18_000), at(99, WEEK), 100_000)
        );
    }

    #[test]
    fn a_repeat_of_the_recorded_figures_restamps_once_a_minute() {
        let (_dir, data) = data();
        record(&data, &report(A, (60.0, FIVE), (99.0, WEEK)), 100_000).unwrap();
        record(&data, &report(B, (60.0, FIVE), (99.0, WEEK)), 110_000).unwrap();
        let reset = report(B, (2.0, FIVE), (1.0, WEEK));
        record(&data, &reset, 200_000).unwrap();
        // A's reply agrees: a new reading is fresh even when no figure moves.
        record(&data, &report(A, (2.0, FIVE), (1.0, WEEK)), 201_000).unwrap();
        assert_eq!(figures(&data), (at(2, FIVE), at(1, WEEK), 201_000));
        record(&data, &reset, 201_000 + RESTAMP_MS - 1).unwrap();
        assert_eq!(figures(&data), (at(2, FIVE), at(1, WEEK), 201_000));
        record(&data, &reset, 201_000 + RESTAMP_MS).unwrap();
        assert_eq!(
            figures(&data),
            (at(2, FIVE), at(1, WEEK), 201_000 + RESTAMP_MS)
        );
    }

    #[test]
    fn a_file_without_session_limits_loads_and_heals_at_a_new_reading() {
        let (_dir, data) = data();
        crate::ensure_private_dir(&data).unwrap();
        let path = data.join(USAGE_FILE);
        let old = serde_json::json!({
            "plan": {
                "five_hour": {"used_percent": 50, "resets_at_ms": FIVE * 1000},
                "seven_day": {"used_percent": 99, "resets_at_ms": WEEK * 1000},
                "recorded_ms": 100_000,
            },
            "windows": {A: {"size": 200_000, "seen_ms": 100_000}},
        });
        std::fs::write(&path, old.to_string()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut usage = Usage::new(path);
        usage.refresh();
        assert_eq!(usage.window(A), Some(200_000));
        assert_eq!(usage.plan().unwrap().seven_day, at(99, WEEK));
        record(&data, &report(A, (2.0, FIVE), (1.0, WEEK)), 200_000).unwrap();
        assert_eq!(figures(&data), (at(50, FIVE), at(99, WEEK), 100_000));
        record(&data, &report(A, (3.0, FIVE), (1.0, WEEK)), 210_000).unwrap();
        assert_eq!(figures(&data), (at(3, FIVE), at(1, WEEK), 210_000));
        usage.refresh();
        assert_eq!(usage.window(A), Some(200_000));
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
                r#"{{"session_id":"00000000-0000-4000-8000-{i:012}","context_window":{{"context_window_size":100000000}},"rate_limits":{{"five_hour":{{"used_percentage":100,"resets_at":99999999999}},"seven_day":{{"used_percentage":100,"resets_at":99999999999}}}}}}"#
            );
            record(&data, input.as_bytes(), 1_700_000_000_000 + i).unwrap();
        }
        // Full, the file stays within what the daemon reads.
        let mut usage = Usage::new(data.join(USAGE_FILE));
        usage.refresh();
        assert!(usage.plan().is_some());
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

    #[test]
    fn records_antigravity_quota_and_window_without_leaking_account_fields() {
        let (_dir, data) = data();
        let agy_input = include_str!("../tests/fixtures/statusline-agy.json");
        let now_ms = crate::transcript::timestamp_ms("2026-11-20T21:17:32Z").unwrap();
        record(&data, INPUT.as_bytes(), now_ms - 1_000).unwrap();
        record(&data, agy_input.as_bytes(), now_ms).unwrap();

        let mut usage = Usage::new(data.join(USAGE_FILE));
        usage.refresh();
        let plan = usage.plan().unwrap();
        assert_eq!(plan.five_hour.as_ref().unwrap().used_percent, 24);
        let agy = plan.agy.expect("agy quota recorded");
        assert_eq!(agy.recorded_ms, now_ms);
        let ids: Vec<&str> = agy.buckets.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "gemini-5h",
                "gemini-weekly",
                "3p-5h",
                "3p-weekly",
                "gemini-argon-wtu-weekly",
            ]
        );
        assert_eq!(agy.buckets[0].used_percent, 65);
        assert_eq!(agy.buckets[0].window_seconds, Some(5 * 3600));
        assert_eq!(
            agy.buckets[0].resets_at_ms,
            crate::transcript::timestamp_ms("2026-11-21T00:46:42Z").unwrap()
        );
        assert_eq!(agy.buckets[1].used_percent, 19);
        assert_eq!(agy.buckets[1].window_seconds, Some(7 * 86_400));
        assert_eq!(agy.buckets[2].used_percent, 0);
        assert_eq!(usage.window(SESSION), Some(1_048_576));

        let text = std::fs::read_to_string(data.join(USAGE_FILE)).unwrap();
        for kept_out in ["email", "example.com", "cwd", "Google AI Pro", "transcript"] {
            assert!(!text.contains(kept_out), "{kept_out} leaked into the file");
        }

        record(&data, agy_input.as_bytes(), now_ms + 5_000).unwrap();
        assert_eq!(read(&data).agy.unwrap().recorded_ms, now_ms);
        record(&data, agy_input.as_bytes(), now_ms + RESTAMP_MS).unwrap();
        assert_eq!(read(&data).agy.unwrap().recorded_ms, now_ms + RESTAMP_MS);

        let filtered = r#"{"product":"antigravity","quota":{"gemini-5h":{"remaining_fraction":0.5,"reset_in_seconds":3600},"off":{"remaining_fraction":0.1,"reset_in_seconds":3600,"disabled":true},"old":{"remaining_fraction":0.1,"reset_time":"2020-01-01T00:00:00Z"},"bad key":{"remaining_fraction":0.1,"reset_in_seconds":3600}}}"#;
        record(&data, filtered.as_bytes(), now_ms + RESTAMP_MS + 1).unwrap();
        let only = read(&data).agy.unwrap();
        assert_eq!(only.buckets.len(), 1);
        assert_eq!(only.buckets[0].id, "gemini-5h");
        assert_eq!(only.buckets[0].used_percent, 50);
        assert_eq!(
            only.buckets[0].resets_at_ms,
            now_ms + RESTAMP_MS + 1 + 3_600_000
        );
    }
}
