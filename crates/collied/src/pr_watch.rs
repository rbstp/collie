use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use protocol::{AgentPromptParams, AgentStatus, ErrorCode, OpId, PromptText, TerminalId};
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

use crate::drive::{Authorized, Driver, Fail, fail};
use crate::herdr::AgentInfo;
use crate::pulls::{self, Check, Outcome};
use crate::{flock, peers};

pub const POLL: Duration = Duration::from_secs(120);
const COMMENT_WAKES: u32 = 10;
const FAILED_READS: u32 = 8;
const LISTED: usize = 10;
const MAX_NAME_CHARS: usize = 100;

/// No body is asked for: review and comment text never reaches collied.
const QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {
  viewer { login }
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      state
      mergeable
      headRefOid
      commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes {
        __typename
        ... on CheckRun { name status conclusion isRequired(pullRequestNumber: $number) }
        ... on StatusContext { context state isRequired(pullRequestNumber: $number) }
      } } } } } }
      comments(last: 20) { nodes { author { login } url } }
      reviews(last: 20) { nodes { author { login } url } }
    }
  }
}";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    CheckFailed { name: String },
    ChecksPassed,
    Review { url: String },
    Comment { url: String },
    Conflict,
}

impl Change {
    fn remark(&self) -> bool {
        matches!(self, Self::Review { .. } | Self::Comment { .. })
    }

    fn summary(&self) -> &'static str {
        match self {
            Self::CheckFailed { .. } => "CI failed",
            Self::ChecksPassed => "checks passed",
            Self::Review { .. } => "new review",
            Self::Comment { .. } => "new comment",
            Self::Conflict => "merge conflict",
        }
    }

    fn line(&self) -> String {
        match self {
            Self::CheckFailed { name } if name.is_empty() => "a check failed".to_owned(),
            Self::CheckFailed { name } => format!("check \"{name}\" failed"),
            Self::ChecksPassed => "all required checks passed".to_owned(),
            Self::Review { url } => format!("new review {url}"),
            Self::Comment { url } => format!("new comment {url}"),
            Self::Conflict => "the branch now conflicts with its base".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Merged,
    Closed,
    Comments,
    FailedReads,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Watch {
    pub url: String,
    pub number: u64,
    #[serde(default)]
    head: Option<String>,
    #[serde(default)]
    failed: Vec<String>,
    #[serde(default)]
    passed: bool,
    #[serde(default)]
    conflicting: bool,
    /// `None` until the first read, which only records what is already there.
    #[serde(default)]
    seen: Option<Vec<String>>,
    #[serde(default)]
    comment_wakes: u32,
    #[serde(default)]
    failed_reads: u32,
    #[serde(default)]
    pending: Vec<Change>,
}

#[derive(Deserialize)]
struct Answer {
    data: Option<Data>,
}

#[derive(Deserialize)]
struct Data {
    viewer: Viewer,
    repository: Option<Repository>,
}

#[derive(Deserialize)]
struct Viewer {
    login: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    pull_request: Option<Pull>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<Option<T>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pull {
    state: String,
    mergeable: String,
    head_ref_oid: String,
    commits: Nodes<CommitNode>,
    comments: Nodes<Remark>,
    reviews: Nodes<Remark>,
}

#[derive(Deserialize)]
struct CommitNode {
    commit: Commit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Commit {
    status_check_rollup: Option<Rollup>,
}

#[derive(Deserialize)]
struct Rollup {
    contexts: Nodes<Context>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Context {
    name: Option<String>,
    context: Option<String>,
    #[serde(default)]
    is_required: bool,
    #[serde(flatten)]
    check: Check,
}

#[derive(Deserialize)]
struct Remark {
    author: Option<Author>,
    url: String,
}

#[derive(Deserialize)]
struct Author {
    login: String,
}

pub struct Read {
    viewer: String,
    pull: Pull,
}

pub fn parse(json: &[u8]) -> Option<Read> {
    let data = serde_json::from_slice::<Answer>(json).ok()?.data?;
    Some(Read {
        viewer: data.viewer.login,
        pull: data.repository?.pull_request?,
    })
}

/// `host`, `owner`, `name` and `number` of `https://host/owner/name/pull/number`, each
/// limited to the characters GitHub allows and never starting with `-`.
fn target(url: &str) -> Option<(&str, &str, &str, u64)> {
    let mut parts = url.strip_prefix("https://")?.split('/');
    let (host, owner, name, pull, number) = (
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    );
    let allowed = |s: &str, extra: &[u8]| {
        !s.is_empty()
            && !s.starts_with('-')
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || extra.contains(&b))
    };
    if parts.next().is_some()
        || pull != "pull"
        || !allowed(host, b".-")
        || !allowed(owner, b"-")
        || !allowed(name, b"._-")
        || !allowed(number, b"")
    {
        return None;
    }
    Some((host, owner, name, number.parse().ok()?))
}

pub fn supported(url: &str) -> bool {
    target(url).is_some()
}

/// Run outside any repository: the query names its repository and pull request, passed as
/// GraphQL variables with `-f`, which never reads a file.
pub async fn fetch(gh: &str, url: &str, dir: &Path) -> Option<Read> {
    let (host, owner, name, number) = target(url)?;
    let mut command = pulls::gh_command(gh);
    command
        .args(["api", "graphql", "--hostname", host, "-f"])
        .arg(format!("owner={owner}"))
        .arg("-f")
        .arg(format!("name={name}"))
        .arg("-F")
        .arg(format!("number={number}"))
        .arg("-f")
        .arg(format!("query={QUERY}"))
        .current_dir(dir)
        .stdout(Stdio::piped());
    let output = timeout(pulls::GH_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse(&output.stdout)
}

fn clean(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|&c| !c.is_control() && !protocol::is_format(c) && c != '"')
        .take(MAX_NAME_CHARS)
        .collect();
    name.trim().to_owned()
}

impl Watch {
    pub fn new(url: &str, number: u64) -> Self {
        Self {
            url: url.to_owned(),
            number,
            ..Self::default()
        }
    }

    pub fn observe(&mut self, read: Option<Read>) -> Option<Stop> {
        let Some(Read { viewer, pull }) = read else {
            self.failed_reads += 1;
            return (self.failed_reads >= FAILED_READS).then_some(Stop::FailedReads);
        };
        self.failed_reads = 0;
        match pull.state.as_str() {
            "MERGED" => return Some(Stop::Merged),
            "CLOSED" => return Some(Stop::Closed),
            _ => {}
        }
        let mut changes = Vec::new();
        if self.head.as_deref() != Some(&pull.head_ref_oid) {
            self.head = Some(pull.head_ref_oid);
            self.failed.clear();
            self.passed = false;
            self.pending
                .retain(|c| !matches!(c, Change::CheckFailed { .. } | Change::ChecksPassed));
        }
        let checks: Vec<(String, bool, Outcome)> = pull
            .commits
            .nodes
            .into_iter()
            .flatten()
            .filter_map(|n| n.commit.status_check_rollup)
            .flat_map(|r| r.contexts.nodes.into_iter().flatten())
            .map(|c| {
                let name = clean(c.name.or(c.context).as_deref().unwrap_or_default());
                (name, c.is_required, pulls::outcome(&c.check))
            })
            .collect();
        // An empty rollup keeps the last state: GitHub answers with one while it is busy.
        if !checks.is_empty() {
            let failed: Vec<String> = checks
                .iter()
                .filter(|(_, _, outcome)| *outcome == Outcome::Fail)
                .map(|(name, ..)| name.clone())
                .collect();
            for name in failed.iter().filter(|n| !self.failed.contains(n)) {
                changes.push(Change::CheckFailed { name: name.clone() });
            }
            self.pending
                .retain(|c| !matches!(c, Change::CheckFailed { name } if !failed.contains(name)));
            self.failed = failed;
            let required = checks.iter().any(|(_, required, _)| *required);
            let passed = checks
                .iter()
                .filter(|(_, r, _)| *r || !required)
                .all(|(_, _, outcome)| *outcome == Outcome::Pass);
            if passed && !self.passed {
                changes.push(Change::ChecksPassed);
            }
            self.passed |= passed;
        }
        let remarks: Vec<(bool, Remark)> = (pull.reviews.nodes.into_iter().flatten())
            .map(|r| (true, r))
            .chain(
                pull.comments
                    .nodes
                    .into_iter()
                    .flatten()
                    .map(|r| (false, r)),
            )
            .filter(|(_, r)| pulls::valid_url(&r.url))
            .collect();
        if let Some(seen) = &self.seen {
            for (review, r) in &remarks {
                let own = r
                    .author
                    .as_ref()
                    .is_some_and(|a| a.login.eq_ignore_ascii_case(&viewer));
                if own || seen.contains(&r.url) {
                    continue;
                }
                let url = r.url.clone();
                changes.push(if *review {
                    Change::Review { url }
                } else {
                    Change::Comment { url }
                });
            }
        }
        self.seen = Some(remarks.into_iter().map(|(_, r)| r.url).collect());
        match pull.mergeable.as_str() {
            "CONFLICTING" => {
                if !self.conflicting {
                    changes.push(Change::Conflict);
                }
                self.conflicting = true;
            }
            "MERGEABLE" => {
                self.conflicting = false;
                self.pending.retain(|c| *c != Change::Conflict);
            }
            _ => {}
        }
        for change in changes {
            if !self.pending.contains(&change) {
                self.pending.push(change);
            }
        }
        None
    }

    pub fn prompt(&self) -> Option<PromptText> {
        if self.pending.is_empty() {
            return None;
        }
        let mut lines: Vec<String> = self.pending.iter().take(LISTED).map(Change::line).collect();
        if self.pending.len() > LISTED {
            lines.push(format!("and {} more", self.pending.len() - LISTED));
        }
        PromptText::new(format!(
            "Pull request #{} ({}) changed: {}.",
            self.number,
            self.url,
            lines.join("; ")
        ))
        .ok()
    }

    pub fn summary(&self) -> String {
        let mut said: Vec<&str> = Vec::new();
        for change in &self.pending {
            if !said.contains(&change.summary()) {
                said.push(change.summary());
            }
        }
        format!("{} on #{}", said.join(", "), self.number)
    }

    pub fn woke(&mut self) -> Option<Stop> {
        let remarks = self.pending.iter().all(Change::remark);
        self.comment_wakes = if remarks { self.comment_wakes + 1 } else { 0 };
        self.pending.clear();
        (remarks && self.comment_wakes >= COMMENT_WAKES).then_some(Stop::Comments)
    }
}

/// Watched pull requests by terminal id, kept on disk so a watch survives a restart.
#[derive(Default)]
pub struct Watches {
    watches: BTreeMap<String, Watch>,
    path: Option<PathBuf>,
}

impl Watches {
    pub fn load(path: PathBuf) -> Self {
        let watches = peers::load_json(&path).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the watched pull requests");
            BTreeMap::new()
        });
        Self {
            watches,
            path: Some(path),
        }
    }

    fn save(&self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        if let Err(e) = peers::replace_json(path, &self.watches) {
            tracing::warn!(error = %e, "could not save the watched pull requests");
        }
    }

    pub fn watching(&self, terminal_id: &str, url: &str) -> bool {
        self.watches.get(terminal_id).is_some_and(|w| w.url == url)
    }

    pub fn start(&mut self, terminal_id: &str, url: &str, number: u64) {
        if !self.watching(terminal_id, url) {
            self.watches
                .insert(terminal_id.to_owned(), Watch::new(url, number));
            self.save();
        }
    }

    pub fn stop(&mut self, terminal_id: &str) {
        if self.watches.remove(terminal_id).is_some() {
            self.save();
        }
    }

    pub fn list(&self) -> Vec<(String, Watch)> {
        self.watches
            .iter()
            .map(|(t, w)| (t.clone(), w.clone()))
            .collect()
    }

    /// Applies `f` only while the terminal still watches `url`, and ends the watch on a stop.
    pub fn update(
        &mut self,
        terminal_id: &str,
        url: &str,
        f: impl FnOnce(&mut Watch) -> Option<Stop>,
    ) -> Option<Stop> {
        let watch = self.watches.get_mut(terminal_id).filter(|w| w.url == url)?;
        let stop = f(watch);
        if let Some(stop) = stop {
            tracing::info!(terminal_id, ?stop, "stopped watching a pull request");
            self.watches.remove(terminal_id);
        }
        self.save();
        stop
    }
}

/// Only an idle or done agent is woken, through `Driver::prompt` and every one of its
/// guards: with no `expected_draft`, a box holding text refuses the prompt and is never
/// cleared.
pub async fn wake(drive: &Driver, agent: &AgentInfo, text: PromptText) -> Result<(), Fail> {
    if !matches!(
        flock::status(&agent.agent_status),
        AgentStatus::Idle | AgentStatus::Done
    ) {
        return fail(ErrorCode::AgentNotReady, "the agent is busy");
    }
    let terminal_id = TerminalId::new(agent.terminal_id.clone())
        .map_err(|_| (ErrorCode::InvalidParams, "invalid terminal id".to_owned()))?;
    let mut op = [0u8; 16];
    getrandom::fill(&mut op).map_err(|_| (ErrorCode::Internal, "no randomness".to_owned()))?;
    let op_id = OpId::new(URL_SAFE_NO_PAD.encode(op))
        .map_err(|_| (ErrorCode::Internal, "invalid op id".to_owned()))?;
    let always: Authorized = Arc::new(|| true);
    let params = AgentPromptParams {
        op_id,
        terminal_id,
        text,
        expected_draft: None,
    };
    drive.prompt(params, &always).await.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://github.com/cli/cli/pull/14673";

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/gh")
                .join(name),
        )
        .unwrap()
    }

    fn read(json: &str) -> Option<Read> {
        Some(parse(json.as_bytes()).unwrap())
    }

    fn review(json: &str, login: &str, url: &str) -> String {
        json.replace(
            r#""reviews":{"nodes":["#,
            &format!(r#""reviews":{{"nodes":[{{"author":{{"login":"{login}"}},"url":"{url}"}},"#),
        )
        .replace(",]", "]")
    }

    #[test]
    fn a_failing_check_wakes_once_with_its_name_and_the_url() {
        let failing = fixture("watch-failing.json");
        let mut watch = Watch::new(URL, 14673);
        assert_eq!(watch.observe(read(&failing)), None);
        assert_eq!(
            watch.pending,
            vec![
                Change::CheckFailed {
                    name: "govulncheck".into()
                },
                Change::ChecksPassed
            ],
            "govulncheck is not required; the three required builds passed"
        );
        let prompt = watch.prompt().unwrap();
        assert_eq!(
            prompt.as_str(),
            "Pull request #14673 (https://github.com/cli/cli/pull/14673) changed: check \"govulncheck\" failed; all required checks passed."
        );
        assert_eq!(watch.summary(), "CI failed, checks passed on #14673");
        assert_eq!(watch.woke(), None);
        assert_eq!(watch.observe(read(&failing)), None);
        assert_eq!(watch.prompt(), None, "nothing new on the same head");

        let pushed = failing.replace("866284b5d31b7eed80ae7266cb6e3f764e1db2c3", "0123456");
        watch.observe(read(&pushed));
        assert_eq!(
            watch.pending.len(),
            2,
            "a new head reports its checks again"
        );
        let rerun = pushed.replace(r#""conclusion":"FAILURE""#, r#""conclusion":"SUCCESS""#);
        watch.observe(read(&rerun));
        assert_eq!(
            watch.pending,
            vec![Change::ChecksPassed],
            "a failure fixed before the agent was woken is not told"
        );
        watch.woke();
        watch.observe(read(&pushed));
        assert_eq!(
            watch.pending,
            vec![Change::CheckFailed {
                name: "govulncheck".into()
            }],
            "a check that fails again after a rerun wakes again"
        );
    }

    #[test]
    fn pending_and_required_checks_hold_the_passed_wake() {
        let failing = fixture("watch-failing.json");
        let pending = failing.replace(
            r#""name":"build (macos-latest)","status":"COMPLETED","conclusion":"SUCCESS""#,
            r#""name":"build (macos-latest)","status":"IN_PROGRESS","conclusion":null"#,
        );
        assert_ne!(pending, failing);
        let mut watch = Watch::new(URL, 14673);
        watch.observe(read(&pending));
        assert_eq!(
            watch.pending,
            vec![Change::CheckFailed {
                name: "govulncheck".into()
            }]
        );
        watch.woke();
        watch.observe(read(&failing));
        assert_eq!(watch.pending, vec![Change::ChecksPassed]);
        let empty = failing.replace(
            &failing[failing.find(r#""contexts":{"nodes":["#).unwrap()
                ..failing.find(r#"]}}}}]},"comments""#).unwrap()],
            r#""contexts":{"nodes":["#,
        );
        watch.woke();
        watch.observe(read(&empty));
        assert!(watch.pending.is_empty() && watch.passed && watch.failed.len() == 1);
    }

    #[test]
    fn a_new_review_or_comment_from_someone_else_wakes_once_with_its_url() {
        let remarks = fixture("watch-remarks.json");
        let mut watch = Watch::new("https://github.com/cli/cli/pull/14580", 14580);
        watch.observe(read(&remarks));
        assert!(
            !watch.pending.iter().any(Change::remark),
            "the first read only records what is there"
        );
        watch.woke();
        let reviewed = review(
            &remarks,
            "octocat",
            "https://github.com/cli/cli/pull/14580#pullrequestreview-1",
        );
        let own = review(
            &reviewed,
            "RBSTP",
            "https://github.com/cli/cli/pull/14580#pullrequestreview-2",
        );
        watch.observe(read(&own));
        assert_eq!(
            watch.pending,
            vec![Change::Review {
                url: "https://github.com/cli/cli/pull/14580#pullrequestreview-1".into()
            }]
        );
        assert!(
            watch
                .prompt()
                .unwrap()
                .as_str()
                .ends_with("new review https://github.com/cli/cli/pull/14580#pullrequestreview-1.")
        );
        assert_eq!(watch.summary(), "new review on #14580");
        watch.woke();
        watch.observe(read(&own));
        assert_eq!(watch.prompt(), None);
        let commented = own.replace(
            r#""comments":{"nodes":["#,
            r#""comments":{"nodes":[{"author":null,"url":"https://github.com/cli/cli/pull/14580#issuecomment-9"},{"author":{"login":"x"},"url":"javascript:alert(1)"},"#,
        );
        watch.observe(read(&commented));
        assert_eq!(
            watch.pending,
            vec![Change::Comment {
                url: "https://github.com/cli/cli/pull/14580#issuecomment-9".into()
            }]
        );
    }

    #[test]
    fn a_conflict_wakes_when_it_starts() {
        let conflicting = fixture("watch-conflicting.json");
        let mut watch = Watch::new("https://github.com/cli/cli/pull/14450", 14450);
        watch.observe(read(&conflicting));
        assert!(watch.pending.contains(&Change::Conflict));
        watch.woke();
        watch.observe(read(&conflicting));
        let unknown = conflicting.replace("CONFLICTING", "UNKNOWN");
        watch.observe(read(&unknown));
        watch.observe(read(&conflicting));
        assert!(!watch.pending.contains(&Change::Conflict));
        watch.observe(read(&conflicting.replace("CONFLICTING", "MERGEABLE")));
        watch.observe(read(&conflicting));
        assert_eq!(watch.pending, vec![Change::Conflict]);
        watch.observe(read(&conflicting.replace("CONFLICTING", "MERGEABLE")));
        assert!(
            watch.pending.is_empty(),
            "a conflict solved before the wake is not told"
        );
    }

    #[test]
    fn stop_rules() {
        let mut watch = Watch::new("https://github.com/rbstp/collie/pull/275", 275);
        let merged = fixture("watch-merged.json");
        assert_eq!(watch.observe(read(&merged)), Some(Stop::Merged));
        let closed = merged.replace("MERGED", "CLOSED");
        assert_eq!(watch.observe(read(&closed)), Some(Stop::Closed));

        let open = fixture("watch-failing.json");
        let mut watch = Watch::new(URL, 14673);
        for _ in 0..7 {
            assert_eq!(watch.observe(None), None);
        }
        assert_eq!(
            watch.observe(read(&open)),
            None,
            "a good read resets the count"
        );
        for _ in 0..7 {
            assert_eq!(watch.observe(None), None);
        }
        assert_eq!(watch.observe(None), Some(Stop::FailedReads));

        let mut watch = Watch::new(URL, 14673);
        watch.observe(read(&open));
        assert_eq!(watch.woke(), None);
        let mut json = open.clone();
        for i in 0..9 {
            json = review(&json, "octocat", &format!("{URL}#pullrequestreview-{i}"));
            watch.observe(read(&json));
            assert_eq!(watch.woke(), None);
        }
        let conflict = json.replace("MERGEABLE", "CONFLICTING");
        watch.observe(read(&conflict));
        assert_eq!(
            watch.woke(),
            None,
            "a wake that is not only remarks resets the count"
        );
        for i in 9..18 {
            json = review(&json, "octocat", &format!("{URL}#pullrequestreview-{i}"));
            watch.observe(read(&json));
            assert_eq!(watch.woke(), None);
        }
        json = review(&json, "octocat", &format!("{URL}#pullrequestreview-18"));
        watch.observe(read(&json));
        assert_eq!(
            watch.woke(),
            Some(Stop::Comments),
            "the 10th remark-only wake in a row"
        );
    }

    #[test]
    fn only_github_shaped_pull_request_urls_are_read() {
        assert_eq!(target(URL), Some(("github.com", "cli", "cli", 14673)));
        assert!(supported("https://ghe.example.com/my-org/my.repo_1/pull/3"));
        for url in [
            "http://github.com/cli/cli/pull/1",
            "https://github.com/cli/cli/issues/1",
            "https://github.com/cli/cli/pull/1/files",
            "https://github.com/-cli/cli/pull/1",
            "https://github.com/cli/@cli/pull/1",
            "https://github.com/cli/cli/pull/1x",
            "https://github.com/cli/cli/pull/",
            "https://user@github.com/cli/cli/pull/1",
            "https://github.com/cli/cli/pull/1?x=1",
        ] {
            assert!(!supported(url), "{url}");
        }
    }

    #[test]
    fn watches_survive_a_restart_and_follow_their_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pr-watches.json");
        let mut watches = Watches::load(path.clone());
        watches.start("t1", URL, 14673);
        assert!(watches.watching("t1", URL));
        assert!(!watches.watching("t1", "https://github.com/cli/cli/pull/1"));
        let failing = fixture("watch-failing.json");
        assert_eq!(
            watches.update("t1", URL, |w| w.observe(read(&failing))),
            None
        );
        let restarted = Watches::load(path.clone());
        assert_eq!(restarted.list(), watches.list());
        assert!(restarted.list()[0].1.prompt().is_some());
        assert_eq!(
            watches.update("t1", "https://github.com/cli/cli/pull/1", |_| Some(
                Stop::Closed
            )),
            None
        );
        assert!(watches.watching("t1", URL));
        assert_eq!(
            watches.update("t1", URL, |_| Some(Stop::Merged)),
            Some(Stop::Merged)
        );
        assert!(Watches::load(path.clone()).list().is_empty());
        watches.start("t2", URL, 14673);
        watches.stop("t2");
        assert!(Watches::load(path).list().is_empty());
    }
}
