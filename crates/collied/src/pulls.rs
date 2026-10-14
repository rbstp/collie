use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use protocol::{Agent, Cwd, PullChecks, PullRequest, PullState};
use serde::Deserialize;
use tokio::process::Command;
use tokio::time::timeout;

use crate::pr_watch::Watches;

pub const REFRESH: Duration = Duration::from_secs(120);
pub(crate) const GH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_URL_BYTES: usize = 512;

pub type Key = (String, String);

/// Never prompts and never uses a token from collied's environment: only the user's
/// `gh auth login`.
pub fn gh_command(gh: &str) -> Command {
    let mut command = Command::new(gh);
    command
        .env("PATH", crate::drive::tool_path())
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_ENTERPRISE_TOKEN")
        .env_remove("GITHUB_ENTERPRISE_TOKEN")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}

pub async fn signed_in(gh: &str) -> bool {
    let mut command = gh_command(gh);
    command.args(["auth", "status"]);
    matches!(
        timeout(Duration::from_secs(15), command.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

/// With no argument gh finds the current branch itself, so no branch name reaches its
/// command line.
pub async fn fetch(gh: &str, checkout: &Cwd) -> Result<Option<PullRequest>, &'static str> {
    let mut command = gh_command(gh);
    command
        .args([
            "pr",
            "view",
            "--json",
            "number,state,isDraft,url,statusCheckRollup",
        ])
        .current_dir(checkout.as_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = timeout(GH_TIMEOUT, command.output())
        .await
        .map_err(|_| "timed out")?
        .map_err(|_| "unavailable")?;
    if output.status.success() {
        return parse(&output.stdout).map(Some).ok_or("unreadable output");
    }
    if String::from_utf8_lossy(&output.stderr).contains("no pull requests found") {
        Ok(None)
    } else {
        Err("command failed")
    }
}

/// Read again right before an automatic archive: the number and head commit of the
/// current branch's pull request, only when it merged.
pub async fn merged_head(gh: &str, checkout: &Cwd) -> Option<(u64, String)> {
    let mut command = gh_command(gh);
    command
        .args(["pr", "view", "--json", "number,state,headRefOid"])
        .current_dir(checkout.as_str())
        .stdout(Stdio::piped());
    let output = timeout(GH_TIMEOUT, command.output()).await.ok()?.ok()?;
    if !output.status.success() {
        return None;
    }
    merged(&output.stdout)
}

fn merged(json: &[u8]) -> Option<(u64, String)> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Merged {
        number: u64,
        state: String,
        head_ref_oid: String,
    }
    let view: Merged = serde_json::from_slice(json).ok()?;
    (view.state == "MERGED").then_some((view.number, view.head_ref_oid))
}

/// The paired phones that turned on "Archive when the PR merges", by `StableID`, kept on
/// disk so the setting holds with no phone connected and across a restart.
#[derive(Default)]
pub struct ArchiveOnMerge {
    by: BTreeSet<String>,
    path: Option<PathBuf>,
}

impl ArchiveOnMerge {
    pub fn load(path: PathBuf) -> Self {
        let by = crate::peers::load_json(&path).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the archive on merge setting");
            BTreeSet::new()
        });
        Self {
            by,
            path: Some(path),
        }
    }

    pub fn set(&mut self, by: &str, enabled: bool) {
        let changed = if enabled {
            self.by.insert(by.to_owned())
        } else {
            self.by.remove(by)
        };
        if let (true, Some(path)) = (changed, &self.path)
            && let Err(e) = crate::peers::replace_json(path, &self.by)
        {
            tracing::warn!(error = %e, "could not save the archive on merge setting");
        }
    }

    pub fn enabled(&self, paired: impl Fn(&str) -> bool) -> bool {
        self.by.iter().any(|by| paired(by))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct View {
    number: u64,
    state: String,
    is_draft: bool,
    url: String,
    #[serde(default)]
    status_check_rollup: Option<Vec<Check>>,
}

#[derive(Deserialize)]
pub(crate) struct Check {
    status: Option<String>,
    conclusion: Option<String>,
    state: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Outcome {
    Pass,
    Pending,
    Fail,
}

fn parse(json: &[u8]) -> Option<PullRequest> {
    let view: View = serde_json::from_slice(json).ok()?;
    let state = match (view.state.as_str(), view.is_draft) {
        ("OPEN", true) => PullState::Draft,
        ("OPEN", false) => PullState::Open,
        ("MERGED", _) => PullState::Merged,
        ("CLOSED", _) => PullState::Closed,
        _ => return None,
    };
    if !valid_url(&view.url) {
        return None;
    }
    Some(PullRequest {
        number: view.number,
        state,
        checks: checks(&view.status_check_rollup.unwrap_or_default()),
        url: view.url,
    })
}

pub(crate) fn valid_url(url: &str) -> bool {
    url.starts_with("https://")
        && url.len() <= MAX_URL_BYTES
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// A status context has a `state`; a check run a `status`, and a `conclusion` once it
/// completed.
pub(crate) fn outcome(check: &Check) -> Outcome {
    match (
        check.state.as_deref(),
        check.status.as_deref(),
        check.conclusion.as_deref(),
    ) {
        (Some("SUCCESS"), ..) => Outcome::Pass,
        (Some("PENDING" | "EXPECTED"), ..) => Outcome::Pending,
        (Some(_), ..) => Outcome::Fail,
        (None, Some(status), _) if status != "COMPLETED" => Outcome::Pending,
        (None, _, Some("SUCCESS" | "NEUTRAL" | "SKIPPED")) => Outcome::Pass,
        (None, _, None | Some("")) => Outcome::Pending,
        (None, _, Some(_)) => Outcome::Fail,
    }
}

fn checks(rollup: &[Check]) -> PullChecks {
    match rollup.iter().map(outcome).max() {
        None => PullChecks::None,
        Some(Outcome::Pass) => PullChecks::Passing,
        Some(Outcome::Pending) => PullChecks::Pending,
        Some(Outcome::Fail) => PullChecks::Failing,
    }
}

#[derive(Default)]
pub struct Pulls {
    cache: HashMap<Key, (Instant, Option<PullRequest>, bool)>,
    linked: HashMap<String, PullRequest>,
    pub watches: Watches,
    pub archive: ArchiveOnMerge,
}

impl Pulls {
    pub fn new(watches: Watches, archive: ArchiveOnMerge) -> Self {
        Self {
            watches,
            archive,
            ..Self::default()
        }
    }

    pub fn get(&self, terminal_id: &str) -> Option<PullRequest> {
        self.linked.get(terminal_id).cloned()
    }

    pub fn fill(&self, agent: &mut Agent) {
        agent.pull_request = self.get(agent.terminal_id.as_str());
        agent.pr_watch = self.watches.shown(agent.terminal_id.as_str());
    }

    pub fn cached(&self, key: &Key, now: Instant) -> Option<Option<PullRequest>> {
        self.cache
            .get(key)
            .filter(|(at, ..)| now.saturating_duration_since(*at) < REFRESH)
            .map(|(_, pr, _)| pr.clone())
    }

    /// A failed read keeps what the last good one found for one refresh, then shows none.
    pub fn store(
        &mut self,
        key: Key,
        now: Instant,
        read: Result<Option<PullRequest>, &str>,
    ) -> Option<PullRequest> {
        let failed = read.is_err();
        let pr = match read {
            Ok(pr) => pr,
            Err(_) => match self.cache.get(&key) {
                Some((_, pr, false)) => pr.clone(),
                _ => None,
            },
        };
        self.cache.insert(key, (now, pr.clone(), failed));
        pr
    }

    pub fn link(&mut self, linked: HashMap<String, PullRequest>, seen: &HashSet<Key>) -> bool {
        self.cache.retain(|key, _| seen.contains(key));
        let changed = linked != self.linked;
        self.linked = linked;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/gh")
                .join(name),
        )
        .unwrap()
    }

    fn summary(name: &str) -> (u64, PullState, PullChecks) {
        let pr = parse(&fixture(name)).unwrap();
        (pr.number, pr.state, pr.checks)
    }

    #[test]
    fn parses_recorded_gh_pr_view_output() {
        assert_eq!(
            summary("pr-view-failing.json"),
            (14673, PullState::Open, PullChecks::Failing)
        );
        assert_eq!(
            summary("pr-view-draft.json"),
            (14355, PullState::Draft, PullChecks::Passing)
        );
        assert_eq!(
            summary("pr-view-merged.json"),
            (275, PullState::Merged, PullChecks::Passing)
        );
        assert_eq!(
            summary("pr-view-no-checks.json"),
            (14665, PullState::Open, PullChecks::None)
        );
        assert_eq!(
            summary("pr-view-pending.json"),
            (276, PullState::Open, PullChecks::Pending)
        );
        assert_eq!(
            parse(&fixture("pr-view-merged.json")).unwrap().url,
            "https://github.com/rbstp/collie/pull/275"
        );
    }

    #[test]
    fn only_a_merged_pull_request_gives_its_head() {
        let json = r#"{"number":275,"state":"MERGED","headRefOid":"0123456789abcdef0123456789abcdef01234567"}"#;
        assert_eq!(
            merged(json.as_bytes()),
            Some((275, "0123456789abcdef0123456789abcdef01234567".to_owned()))
        );
        for state in ["OPEN", "CLOSED"] {
            let json = json.replace("MERGED", state);
            assert_eq!(merged(json.as_bytes()), None, "{state}");
        }
        assert_eq!(merged(br#"{"number":275,"state":"MERGED"}"#), None);
    }

    #[test]
    fn archive_on_merge_is_per_phone_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive-on-merge.json");
        let mut setting = ArchiveOnMerge::load(path.clone());
        assert!(!setting.enabled(|_| true));
        setting.set("a", true);
        setting.set("b", true);
        setting.set("b", false);
        let setting = ArchiveOnMerge::load(path);
        assert!(setting.enabled(|by| by == "a"));
        assert!(!setting.enabled(|by| by == "b"));
        assert!(!setting.enabled(|_| false));
    }

    #[test]
    fn closed_state_status_contexts_and_bad_output() {
        let merged = String::from_utf8(fixture("pr-view-merged.json")).unwrap();
        let closed = merged.replace(r#""state":"MERGED""#, r#""state":"CLOSED""#);
        assert_eq!(parse(closed.as_bytes()).unwrap().state, PullState::Closed);
        let error = r#"{"number":1,"state":"OPEN","isDraft":false,"url":"https://github.com/o/r/pull/1","statusCheckRollup":[{"__typename":"StatusContext","state":"PENDING"},{"__typename":"StatusContext","state":"ERROR"}]}"#;
        assert_eq!(parse(error.as_bytes()).unwrap().checks, PullChecks::Failing);
        let null = error.replace(r#"[{"__typename":"StatusContext","state":"PENDING"},{"__typename":"StatusContext","state":"ERROR"}]"#, "null");
        assert_eq!(parse(null.as_bytes()).unwrap().checks, PullChecks::None);
        for bad in [
            merged.replace(r#""state":"MERGED""#, r#""state":"LOCKED""#),
            merged.replace("https://github.com", "http://github.com"),
            merged.replace("https://github.com", "javascript:alert(1)//"),
            merged.replace("/pull/275", "/pull/275 x"),
            merged.replace("/pull/275", &format!("/{}", "a".repeat(MAX_URL_BYTES))),
            "no pull requests found".to_owned(),
        ] {
            assert_eq!(parse(bad.as_bytes()), None, "{bad}");
        }
    }

    fn pr(number: u64) -> PullRequest {
        PullRequest {
            number,
            state: PullState::Open,
            checks: PullChecks::Pending,
            url: format!("https://github.com/o/r/pull/{number}"),
        }
    }

    #[test]
    fn cache_refreshes_every_two_minutes_and_keeps_the_last_read_through_one_failure() {
        let mut pulls = Pulls::default();
        let key = ("/repo".to_owned(), "feature".to_owned());
        let start = Instant::now();
        assert_eq!(pulls.cached(&key, start), None);
        assert_eq!(
            pulls.store(key.clone(), start, Ok(Some(pr(1)))),
            Some(pr(1))
        );
        assert_eq!(
            pulls.cached(&key, start + REFRESH - Duration::from_secs(1)),
            Some(Some(pr(1)))
        );
        assert_eq!(pulls.cached(&key, start + REFRESH), None);
        let later = start + REFRESH;
        assert_eq!(
            pulls.store(key.clone(), later, Err("command failed")),
            Some(pr(1))
        );
        assert_eq!(pulls.cached(&key, later), Some(Some(pr(1))));
        let again = later + REFRESH;
        assert_eq!(pulls.store(key.clone(), again, Err("command failed")), None);
        assert_eq!(pulls.store(key.clone(), again, Err("command failed")), None);
        assert_eq!(pulls.cached(&key, again), Some(None));
        assert_eq!(
            pulls.store(key.clone(), later, Ok(Some(pr(1)))),
            Some(pr(1))
        );
        assert_eq!(pulls.store(key.clone(), later, Ok(None)), None);
        assert_eq!(pulls.cached(&key, later), Some(None));
        assert_eq!(pulls.store(key.clone(), later, Err("command failed")), None);
    }

    #[test]
    fn link_reports_changes_and_forgets_branches_no_agent_is_on() {
        let mut pulls = Pulls::default();
        let now = Instant::now();
        let a = ("/a".to_owned(), "x".to_owned());
        let b = ("/b".to_owned(), "y".to_owned());
        pulls.store(a.clone(), now, Ok(Some(pr(1))));
        pulls.store(b.clone(), now, Ok(None));
        let linked = HashMap::from([("t1".to_owned(), pr(1))]);
        let seen = HashSet::from([a.clone()]);
        assert!(pulls.link(linked.clone(), &seen));
        assert_eq!(pulls.get("t1"), Some(pr(1)));
        assert_eq!(pulls.get("t2"), None);
        assert_eq!(pulls.cached(&b, now), None);
        assert!(!pulls.link(linked, &seen));
        assert!(pulls.link(HashMap::new(), &HashSet::new()));
        assert_eq!(pulls.get("t1"), None);
        assert_eq!(pulls.cached(&a, now), None);
    }
}
