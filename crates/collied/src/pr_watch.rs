use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use protocol::{
    Agent, AgentPromptParams, AgentStatus, ErrorCode, HistoryMessage, MessageOrigin, OpId, PrWatch,
    PromptText, TerminalId,
};
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

use crate::drive::{Authorized, Driver, Fail, fail};
use crate::herdr::AgentInfo;
use crate::pulls::{self, Check, Outcome, clean};
use crate::{flock, peers};

pub const EVERY_MINUTES: [u32; 4] = [2, 5, 15, 30];
const COMMENT_WAKES: u32 = 10;
const FAILED_READS: u32 = 8;
const LISTED: usize = 10;
const WAKES_KEPT: usize = 20;

/// No body is asked for: review and comment text never reaches collied.
const QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {
  viewer { login }
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      state
      mergeable
      headRefOid
      baseRef {
        branchProtectionRule { requiredStatusChecks { context } }
        rules(first: 100) { nodes { parameters { ... on RequiredStatusChecksParameters { requiredStatusChecks { context } } } } }
      }
      commits(last: 1) { nodes { commit { statusCheckRollup { state contexts(first: 100) { nodes {
        __typename
        ... on CheckRun { name status conclusion isRequired(pullRequestNumber: $number) }
        ... on StatusContext { context state isRequired(pullRequestNumber: $number) }
      } } } } } }
      comments(last: 20) { nodes { author { login } url createdAt } }
      reviews(last: 20) { nodes { author { login } url submittedAt } }
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
    pub every_minutes: u32,
    /// The paired phone that started it: revoking that phone ends the watch.
    pub by: String,
    /// The checkout and branch whose pull request this is: the agent is woken only there.
    pub checkout: String,
    pub branch: String,
    #[serde(default)]
    head: Option<String>,
    #[serde(default)]
    failed: Vec<String>,
    #[serde(default)]
    passed: bool,
    #[serde(default)]
    conflicting: bool,
    /// The newest comment or review time seen, `None` until the first read, which only
    /// records what is already there; `at_since` holds the URLs at that time.
    #[serde(default)]
    since: Option<String>,
    #[serde(default)]
    at_since: Vec<String>,
    #[serde(default)]
    comment_wakes: u32,
    #[serde(default)]
    failed_reads: u32,
    #[serde(default)]
    pending: Vec<Change>,
    /// Never saved: a new, restarted or restored watch is read at once.
    #[serde(skip)]
    read_at: Option<Instant>,
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
    base_ref: Option<BaseRef>,
    commits: Nodes<CommitNode>,
    comments: Nodes<Remark>,
    reviews: Nodes<Remark>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BaseRef {
    branch_protection_rule: Option<RequiredChecks>,
    rules: Option<Nodes<Rule>>,
}

#[derive(Deserialize)]
struct Rule {
    parameters: Option<RequiredChecks>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredChecks {
    #[serde(default)]
    required_status_checks: Option<Vec<RequiredCheck>>,
}

#[derive(Deserialize)]
struct RequiredCheck {
    context: String,
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
    state: Option<String>,
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
#[serde(rename_all = "camelCase")]
struct Remark {
    author: Option<Author>,
    url: String,
    #[serde(alias = "submittedAt")]
    created_at: Option<String>,
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

impl Watch {
    pub fn new(url: &str, number: u64) -> Self {
        Self {
            url: url.to_owned(),
            number,
            every_minutes: EVERY_MINUTES[0],
            ..Self::default()
        }
    }

    pub fn due(&self, now: Instant) -> bool {
        self.read_at.is_none_or(|at| {
            now.saturating_duration_since(at)
                >= Duration::from_secs(u64::from(self.every_minutes) * 60)
        })
    }

    pub fn read(&mut self, read: Option<Read>, now: Instant) -> Option<Stop> {
        self.read_at = Some(now);
        self.observe(read)
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
        let rollups: Vec<Rollup> = pull
            .commits
            .nodes
            .into_iter()
            .flatten()
            .filter_map(|n| n.commit.status_check_rollup)
            .collect();
        let rollup: Vec<Option<String>> = rollups.iter().map(|r| r.state.clone()).collect();
        let mut required: Vec<String> = pull
            .base_ref
            .into_iter()
            .flat_map(|b| {
                let rules = b.rules.into_iter().flat_map(|r| r.nodes).flatten();
                b.branch_protection_rule
                    .into_iter()
                    .chain(rules.filter_map(|r| r.parameters))
            })
            .flat_map(|r| r.required_status_checks.unwrap_or_default())
            .map(|r| clean(&r.context))
            .collect();
        let checks: Vec<(String, bool, Outcome)> = rollups
            .into_iter()
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
            for (name, is_required, _) in &checks {
                if *is_required && !required.contains(name) {
                    required.push(name.clone());
                }
            }
            let waiting = |states: &[&str]| {
                rollup
                    .iter()
                    .any(|s| s.as_deref().is_some_and(|s| states.contains(&s)))
            };
            // A required check that has not reported is missing from `checks`: each required
            // name must be listed and pass. With none required, every listed check must pass
            // and the rollup must not still wait on one.
            let passed = if required.is_empty() {
                !waiting(&["PENDING", "EXPECTED"])
                    && checks
                        .iter()
                        .all(|(_, _, outcome)| *outcome == Outcome::Pass)
            } else {
                !waiting(&["EXPECTED"])
                    && required.iter().all(|name| {
                        let mut runs = checks.iter().filter(|(n, ..)| n == name).peekable();
                        runs.peek().is_some() && runs.all(|(_, _, o)| *o == Outcome::Pass)
                    })
            };
            if passed && !self.passed {
                changes.push(Change::ChecksPassed);
            }
            if !passed && self.pending.contains(&Change::ChecksPassed) {
                self.pending.retain(|c| *c != Change::ChecksPassed);
                self.passed = false;
            }
            self.passed |= passed;
        }
        let remarks: Vec<(bool, Remark, String)> = (pull.reviews.nodes.into_iter().flatten())
            .map(|r| (true, r))
            .chain(
                pull.comments
                    .nodes
                    .into_iter()
                    .flatten()
                    .map(|r| (false, r)),
            )
            .filter(|(_, r)| pulls::valid_url(&r.url))
            .filter_map(|(review, r)| {
                let at = r.created_at.clone()?;
                Some((review, r, at))
            })
            .collect();
        let newest = remarks.iter().map(|(.., at)| at.clone()).max();
        if let Some(since) = &self.since {
            for (review, r, at) in &remarks {
                let own = r
                    .author
                    .as_ref()
                    .is_some_and(|a| a.login.eq_ignore_ascii_case(&viewer));
                let seen = at < since || (at == since && self.at_since.contains(&r.url));
                if own || seen {
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
        let since = self.since.take().unwrap_or_default();
        match newest {
            Some(newest) if newest > since => {
                self.at_since = remarks
                    .iter()
                    .filter(|(.., at)| *at == newest)
                    .map(|(_, r, _)| r.url.clone())
                    .collect();
                self.since = Some(newest);
            }
            _ => {
                for (_, r, at) in &remarks {
                    if *at == since && !self.at_since.contains(&r.url) {
                        self.at_since.push(r.url.clone());
                    }
                }
                self.since = Some(since);
            }
        }
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

    pub fn shown(&self, terminal_id: &str) -> Option<PrWatch> {
        self.watches.get(terminal_id).map(|w| PrWatch {
            number: w.number,
            url: w.url.clone(),
            every_minutes: w.every_minutes,
        })
    }

    /// Watching the same pull request again only changes who asked and how often.
    pub fn start(&mut self, terminal_id: &str, watch: Watch) {
        match self.watches.get_mut(terminal_id) {
            Some(w) if w.url == watch.url => {
                w.every_minutes = watch.every_minutes;
                w.by = watch.by;
            }
            _ => {
                self.watches.insert(terminal_id.to_owned(), watch);
            }
        }
        self.save();
    }

    pub fn stop(&mut self, terminal_id: &str) -> bool {
        let stopped = self.watches.remove(terminal_id).is_some();
        if stopped {
            self.save();
        }
        stopped
    }

    /// Ends the watches of terminals missing from `agents`; a list with none is not trusted,
    /// since herdr answers before it restores its panes.
    pub fn retain_listed(&mut self, agents: &[AgentInfo]) -> bool {
        if agents.is_empty() {
            return false;
        }
        self.retain(|terminal_id, _| agents.iter().any(|a| a.terminal_id == terminal_id))
    }

    pub fn retain_paired(&mut self, paired: impl Fn(&str) -> bool) -> bool {
        self.retain(|_, w| paired(&w.by))
    }

    fn retain(&mut self, keep: impl Fn(&str, &Watch) -> bool) -> bool {
        let before = self.watches.len();
        self.watches.retain(|t, w| keep(t, w));
        let ended = self.watches.len() != before;
        if ended {
            self.save();
        }
        ended
    }

    pub fn forget(&mut self, by: &str) -> bool {
        self.retain_paired(|w| w != by)
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Wake {
    text: String,
    summary: String,
    at_ms: u64,
}

/// The last wake prompts typed into each terminal, kept on disk apart from the watches so
/// Chat still knows them after the watch ends or collied restarts.
#[derive(Default)]
pub struct Wakes {
    wakes: BTreeMap<String, Vec<Wake>>,
    path: Option<PathBuf>,
}

impl Wakes {
    pub fn load(path: PathBuf) -> Self {
        let wakes = peers::load_json(&path).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the pull request wakes");
            BTreeMap::new()
        });
        Self {
            wakes,
            path: Some(path),
        }
    }

    fn save(&self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        if let Err(e) = peers::replace_json(path, &self.wakes) {
            tracing::warn!(error = %e, "could not save the pull request wakes");
        }
    }

    pub fn remember(&mut self, terminal_id: &str, text: &str, summary: String, at_ms: u64) {
        let wakes = self.wakes.entry(terminal_id.to_owned()).or_default();
        wakes.push(Wake {
            text: text.to_owned(),
            summary,
            at_ms,
        });
        if wakes.len() > WAKES_KEPT {
            wakes.drain(..wakes.len() - WAKES_KEPT);
        }
        self.save();
    }

    fn origin(&self, terminal_id: &str, typed: impl Fn(&str) -> bool) -> Option<MessageOrigin> {
        let wake = self
            .wakes
            .get(terminal_id)?
            .iter()
            .rev()
            .find(|w| typed(&w.text))?;
        Some(MessageOrigin {
            kind: "pr_watch".into(),
            summary: wake.summary.clone(),
        })
    }

    pub fn mark(&self, terminal_id: &str, messages: &mut [HistoryMessage]) {
        for m in messages {
            if m.role == "user" && m.tool.is_none() && m.parent_call_id.is_none() {
                m.origin = self.origin(terminal_id, |text| text == m.text);
            }
        }
    }

    /// `last_prompt` is the prompt's first line as the transcript summary cuts it, so a
    /// prompt that differs from a wake only after that cut is marked too.
    pub fn mark_prompt(&self, agent: &mut Agent) {
        agent.last_prompt_origin = agent.last_prompt.as_deref().and_then(|prompt| {
            self.origin(agent.terminal_id.as_str(), |text| {
                crate::transcript::first_line(text).as_deref() == Some(prompt)
            })
        });
    }

    pub fn is_empty(&self) -> bool {
        self.wakes.is_empty()
    }

    /// Like `Watches::retain_listed`: a list with no agents is not trusted.
    pub fn retain_listed(&mut self, agents: &[AgentInfo]) {
        if agents.is_empty() {
            return;
        }
        let before = self.wakes.len();
        self.wakes
            .retain(|t, _| agents.iter().any(|a| a.terminal_id == *t));
        if self.wakes.len() != before {
            self.save();
        }
    }
}

/// Only an idle or done Claude Code agent still on the watched branch is woken, through
/// `Driver::prompt` and every one of its guards: with no `expected_draft`, a box holding
/// text refuses the prompt and is never cleared. The status is read again right before the
/// write, and `auth` is checked then too.
pub async fn wake(
    drive: &Driver,
    agent: &AgentInfo,
    watch: &Watch,
    text: PromptText,
    auth: &Authorized,
) -> Result<(), Fail> {
    if agent.agent.as_deref() != Some("claude")
        || !matches!(
            flock::status(&agent.agent_status),
            AgentStatus::Idle | AgentStatus::Done
        )
    {
        return fail(ErrorCode::AgentNotReady, "the agent is busy");
    }
    let checkout = drive.checkout(agent)?;
    match crate::git::branch(&checkout, drive.roots()).await? {
        Some((root, branch)) if root.as_str() == watch.checkout && branch == watch.branch => {}
        _ => {
            return fail(
                ErrorCode::AgentNotReady,
                "the agent left the watched branch",
            );
        }
    }
    let terminal_id = TerminalId::new(agent.terminal_id.clone())
        .map_err(|_| (ErrorCode::InvalidParams, "invalid terminal id".to_owned()))?;
    let mut op = [0u8; 16];
    getrandom::fill(&mut op).map_err(|_| (ErrorCode::Internal, "no randomness".to_owned()))?;
    let op_id = OpId::new(URL_SAFE_NO_PAD.encode(op))
        .map_err(|_| (ErrorCode::Internal, "invalid op id".to_owned()))?;
    let params = AgentPromptParams {
        op_id,
        terminal_id,
        text,
        expected_draft: None,
    };
    drive.prompt_agent(params, auth, true).await.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://github.com/cli/cli/pull/14673";
    const ROLLUP_PASSED: &str = r#""statusCheckRollup":{"state":"SUCCESS""#;
    const UBUNTU_PASSED: &str =
        r#""build (ubuntu-latest)","status":"COMPLETED","conclusion":"SUCCESS""#;

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

    fn remark(json: &str, list: &str, login: &str, url: &str, at: &str) -> String {
        let field = if list == "reviews" {
            "submittedAt"
        } else {
            "createdAt"
        };
        json.replace(
            &format!(r#""{list}":{{"nodes":["#),
            &format!(
                r#""{list}":{{"nodes":[{{"author":{{"login":"{login}"}},"url":"{url}","{field}":"{at}"}},"#
            ),
        )
        .replace(",]", "]")
    }

    fn review(json: &str, login: &str, url: &str, at: &str) -> String {
        remark(json, "reviews", login, url, at)
    }

    fn failed(name: &str) -> Change {
        Change::CheckFailed { name: name.into() }
    }

    #[test]
    fn a_failing_check_wakes_once_with_its_name_and_the_url() {
        let failing = fixture("watch-failing.json");
        let mut watch = Watch::new(URL, 14673);
        assert_eq!(watch.observe(read(&failing)), None);
        assert_eq!(
            watch.pending,
            vec![failed("govulncheck"), Change::ChecksPassed],
            "govulncheck is not required; the three required builds passed"
        );
        assert_eq!(
            watch.prompt().unwrap().as_str(),
            "Pull request #14673 (https://github.com/cli/cli/pull/14673) changed: check \"govulncheck\" failed; all required checks passed."
        );
        assert_eq!(watch.summary(), "CI failed, checks passed on #14673");
        assert_eq!(watch.woke(), None);
        assert_eq!(watch.observe(read(&failing)), None);
        assert_eq!(watch.prompt(), None, "nothing new on the same head");

        let pushed = failing.replace("866284b5d31b7eed80ae7266cb6e3f764e1db2c3", "0123456");
        watch.observe(read(&pushed));
        assert_eq!(
            watch.pending,
            vec![failed("govulncheck"), Change::ChecksPassed],
            "a new head reports its checks again"
        );
        let required = pushed.replace(
            UBUNTU_PASSED,
            r#""build (ubuntu-latest)","status":"COMPLETED","conclusion":"FAILURE""#,
        );
        watch.observe(read(&required));
        assert_eq!(
            watch.pending,
            vec![failed("govulncheck"), failed("build (ubuntu-latest)")],
            "a queued pass is dropped when a required check then fails"
        );
        let rerun = pushed.replace(r#""conclusion":"FAILURE""#, r#""conclusion":"SUCCESS""#);
        watch.observe(read(&rerun));
        assert_eq!(
            watch.pending,
            vec![Change::ChecksPassed],
            "failures fixed before the agent was woken are not told"
        );
        watch.woke();
        watch.observe(read(&pushed));
        assert_eq!(
            watch.pending,
            vec![failed("govulncheck")],
            "a check that fails again after a rerun wakes again"
        );
    }

    #[test]
    fn checks_pass_once_every_required_check_reported_and_passed() {
        let passing = fixture("watch-passing.json");
        let url = "https://github.com/cli/cli/pull/14566";
        let mut watch = Watch::new(url, 14566);
        watch.observe(read(&fixture("watch-required-missing.json")));
        assert!(
            watch.pending.is_empty(),
            "an expected required check holds the pass"
        );
        let running = passing.replace(
            UBUNTU_PASSED,
            r#""build (ubuntu-latest)","status":"IN_PROGRESS","conclusion":null"#,
        );
        assert_ne!(running, passing);
        watch.observe(read(&running));
        assert!(watch.pending.is_empty());
        let unmarked = passing.replace(r#""isRequired":true"#, r#""isRequired":false"#);
        let waiting = unmarked.replace(ROLLUP_PASSED, r#""statusCheckRollup":{"state":"PENDING""#);
        watch.observe(read(&waiting));
        assert!(
            watch.pending.is_empty(),
            "with none required, a pending rollup waits on an unlisted check"
        );
        watch.observe(read(&passing));
        assert_eq!(watch.pending, vec![Change::ChecksPassed]);
        watch.woke();
        watch.observe(read(&passing));
        assert!(watch.pending.is_empty(), "once per head commit");
        let start = passing.find(r#""contexts":{"nodes":["#).unwrap();
        let end = passing.find(r#"]}}}}]},"comments""#).unwrap();
        let empty = passing.replace(&passing[start..end], r#""contexts":{"nodes":["#);
        watch.observe(read(&empty));
        assert!(watch.pending.is_empty() && watch.passed);
    }

    #[test]
    fn a_required_check_named_by_the_base_branch_must_report_and_pass() {
        let running = fixture("watch-required-running.json");
        let ios = r#"{"__typename":"CheckRun","name":"ios","status":"IN_PROGRESS","conclusion":null,"isRequired":true},"#;
        let go = r#""name":"Analyze (go)","status":"COMPLETED","conclusion":"SUCCESS""#;
        assert!(running.contains(ios) && running.contains(go));
        let url = "https://github.com/rbstp/collie/pull/277";
        let mut watch = Watch::new(url, 277);
        watch.observe(read(&running));
        assert!(watch.pending.is_empty(), "ios is still running");
        let unreported = running.replace(ios, "");
        let optional_failed = unreported
            .replace(
                go,
                r#""name":"Analyze (go)","status":"COMPLETED","conclusion":"FAILURE""#,
            )
            .replace(
                r#""statusCheckRollup":{"state":"PENDING""#,
                r#""statusCheckRollup":{"state":"FAILURE""#,
            );
        watch.observe(read(&optional_failed));
        assert_eq!(
            watch.pending,
            vec![failed("Analyze (go)")],
            "ios is required by the ruleset but has not reported"
        );
        watch.woke();
        let optional_running = unreported.replace(
            go,
            r#""name":"Analyze (go)","status":"IN_PROGRESS","conclusion":null"#,
        );
        watch.observe(read(&optional_running));
        assert!(
            watch.pending.is_empty(),
            "a pending rollup with an optional check running still waits for ios"
        );
        let ios_passed = optional_failed.replace(
            r#"{"__typename":"CheckRun","name":"rust""#,
            r#"{"__typename":"CheckRun","name":"ios","status":"COMPLETED","conclusion":"SUCCESS","isRequired":true},{"__typename":"CheckRun","name":"rust""#,
        );
        assert_ne!(ios_passed, optional_failed);
        watch.observe(read(&ios_passed));
        assert_eq!(
            watch.pending,
            vec![failed("Analyze (go)"), Change::ChecksPassed],
            "an optional check failing does not hold the required checks' pass"
        );
    }

    #[test]
    fn a_new_review_or_comment_from_someone_else_wakes_once_with_its_url() {
        let remarks = fixture("watch-remarks.json");
        let url = "https://github.com/cli/cli/pull/14580";
        let mut watch = Watch::new(url, 14580);
        watch.observe(read(&remarks));
        assert!(
            !watch.pending.iter().any(Change::remark),
            "the first read only records what is there"
        );
        watch.woke();
        let reviewed = review(
            &remarks,
            "octocat",
            &format!("{url}#pullrequestreview-1"),
            "2026-10-14T00:00:01Z",
        );
        let own = review(
            &reviewed,
            "VIEWER",
            &format!("{url}#pullrequestreview-2"),
            "2026-10-14T00:00:01Z",
        );
        watch.observe(read(&own));
        assert_eq!(
            watch.pending,
            vec![Change::Review {
                url: format!("{url}#pullrequestreview-1")
            }]
        );
        assert!(
            watch
                .prompt()
                .unwrap()
                .as_str()
                .ends_with(&format!("new review {url}#pullrequestreview-1."))
        );
        assert_eq!(watch.summary(), "new review on #14580");
        watch.woke();
        watch.observe(read(&own));
        assert_eq!(watch.prompt(), None);
        let same_second = review(
            &own,
            "hubot",
            &format!("{url}#pullrequestreview-3"),
            "2026-10-14T00:00:01Z",
        );
        let commented = remark(
            &same_second,
            "comments",
            "x",
            "javascript:alert(1)",
            "2026-10-14T00:00:02Z",
        )
        .replace(
            r#""comments":{"nodes":["#,
            &format!(
                r#""comments":{{"nodes":[{{"author":null,"url":"{url}#issuecomment-9","createdAt":"2026-10-14T00:00:02Z"}},"#
            ),
        );
        watch.observe(read(&commented));
        assert_eq!(
            watch.pending,
            vec![
                Change::Review {
                    url: format!("{url}#pullrequestreview-3")
                },
                Change::Comment {
                    url: format!("{url}#issuecomment-9")
                }
            ]
        );
        watch.woke();
        let older = remark(
            &own,
            "comments",
            "octocat",
            &format!("{url}#issuecomment-1"),
            "2026-10-01T00:00:00Z",
        );
        watch.observe(read(&older));
        assert!(
            watch.pending.is_empty(),
            "an older comment sliding into the window after a deletion is not new"
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
        let closed = merged.replace(r#""state":"MERGED""#, r#""state":"CLOSED""#);
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
        let add = |json: &str, i: u32| {
            review(
                json,
                "octocat",
                &format!("{URL}#pullrequestreview-{i}"),
                &format!("2026-10-14T00:00:{i:02}Z"),
            )
        };
        for i in 0..9 {
            json = add(&json, i);
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
            json = add(&json, i);
            watch.observe(read(&json));
            assert_eq!(watch.woke(), None);
        }
        json = add(&json, 18);
        watch.observe(read(&json));
        assert_eq!(
            watch.woke(),
            Some(Stop::Comments),
            "the 10th remark-only wake in a row"
        );
    }

    #[test]
    fn a_watch_is_read_on_its_own_interval() {
        let now = Instant::now();
        let mut watch = Watch::new(URL, 14673);
        watch.every_minutes = 15;
        assert!(watch.due(now), "a new or restored watch is read at once");
        watch.read(None, now);
        assert!(!watch.due(now + Duration::from_secs(14 * 60 + 59)));
        assert!(watch.due(now + Duration::from_secs(15 * 60)));
        watch.every_minutes = 2;
        assert!(!watch.due(now + Duration::from_secs(119)));
        assert!(watch.due(now + Duration::from_secs(120)));
        assert_eq!(EVERY_MINUTES, [2, 5, 15, 30]);

        let mut watches = Watches::default();
        watches.start("t1", Watch::new(URL, 14673));
        watches.update("t1", URL, |w| w.read(None, now));
        assert!(!watches.list()[0].1.due(now));
        watches.start("t1", Watch::new(URL, 14673));
        assert!(
            !watches.list()[0].1.due(now),
            "watching the same pull request again keeps its schedule"
        );
        watches.stop("t1");
        watches.start("t1", Watch::new(URL, 14673));
        assert!(
            watches.list()[0].1.due(now),
            "a watch started again is read at once"
        );
        watches.start("t1", Watch::new("https://github.com/cli/cli/pull/1", 1));
        assert!(
            watches.list()[0].1.due(now),
            "a new pull request on the terminal is read at once"
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
    fn watches_survive_a_restart_follow_their_url_and_end_with_their_phone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pr-watches.json");
        let mut watches = Watches::load(path.clone());
        let mut watch = Watch::new(URL, 14673);
        watch.by = "nPHONE".into();
        watches.start("t1", watch.clone());
        assert!(watches.watching("t1", URL));
        assert!(!watches.watching("t1", "https://github.com/cli/cli/pull/1"));
        assert_eq!(
            watches.shown("t1"),
            Some(PrWatch {
                number: 14673,
                url: URL.into(),
                every_minutes: 2
            })
        );
        let failing = fixture("watch-failing.json");
        assert_eq!(
            watches.update("t1", URL, |w| w.observe(read(&failing))),
            None
        );
        watch.every_minutes = 30;
        watches.start("t1", watch.clone());
        assert_eq!(watches.shown("t1").unwrap().every_minutes, 30);
        assert!(
            watches.list()[0].1.prompt().is_some(),
            "a new interval keeps what is queued"
        );
        let restarted = Watches::load(path.clone());
        assert_eq!(restarted.list(), watches.list());
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
        watches.start("t2", watch.clone());
        let mut other = watch.clone();
        other.by = "nOTHER".into();
        watches.start("t3", other);
        assert!(!watches.forget("nNOBODY"));
        assert!(watches.forget("nPHONE"));
        assert_eq!(watches.shown("t2"), None);
        assert!(watches.shown("t3").is_some());
        assert!(watches.stop("t3"));
        assert!(Watches::load(path).list().is_empty());
    }

    #[test]
    fn only_remembered_wake_prompts_are_marked_and_they_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pr-wakes.json");
        let mut wakes = Wakes::load(path.clone());
        let mut watch = Watch::new(URL, 14673);
        watch.observe(read(&fixture("watch-failing.json")));
        let text = watch.prompt().unwrap();
        wakes.remember("t1", text.as_str(), watch.summary(), 1);
        let message = |role: &str, text: &str| HistoryMessage {
            id: "m".into(),
            role: role.into(),
            text: text.into(),
            tool: None,
            call_id: None,
            truncated: false,
            parent_call_id: None,
            origin: None,
        };
        let mut messages = vec![
            message("user", text.as_str()),
            message("user", &format!("{} ", text.as_str())),
            message(
                "user",
                "Pull request #14673 changed: check \"govulncheck\" failed.",
            ),
            message("assistant", text.as_str()),
        ];
        let restarted = Wakes::load(path.clone());
        restarted.mark("t1", &mut messages);
        let origin = MessageOrigin {
            kind: "pr_watch".into(),
            summary: "CI failed, checks passed on #14673".into(),
        };
        assert_eq!(messages[0].origin, Some(origin));
        assert!(
            messages[1..].iter().all(|m| m.origin.is_none()),
            "only the exact text of a user message"
        );
        let mut other = vec![message("user", text.as_str())];
        restarted.mark("t2", &mut other);
        assert_eq!(
            other[0].origin, None,
            "only on the terminal it was typed in"
        );

        for i in 0..WAKES_KEPT {
            wakes.remember("t1", &format!("wake {i}"), String::new(), 2);
        }
        let mut first = vec![message("user", text.as_str()), message("user", "wake 0")];
        Wakes::load(path.clone()).mark("t1", &mut first);
        assert!(first[0].origin.is_none(), "the oldest wake is dropped");
        assert!(first[1].origin.is_some());

        let listed: AgentInfo = serde_json::from_value(serde_json::json!({
            "terminal_id": "t2", "workspace_id": "w1", "pane_id": "w1:p1",
            "agent_status": "idle", "agent": "claude",
        }))
        .unwrap();
        wakes.retain_listed(&[]);
        assert!(Wakes::load(path.clone()).wakes.contains_key("t1"));
        wakes.retain_listed(&[listed]);
        assert!(
            Wakes::load(path).is_empty(),
            "a terminal missing from the agent list loses its wakes"
        );
    }

    #[test]
    fn the_inbox_marks_the_cut_first_line_and_accepts_a_prompt_that_differs_after_the_cut() {
        let mut wakes = Wakes::default();
        let long = format!("Pull request #1 ({URL}) changed: {}.", "x".repeat(200));
        wakes.remember("t1", &long, "CI failed on #1".into(), 1);
        wakes.remember(
            "t1",
            "Pull request #2 changed: CI failed.",
            "CI failed on #2".into(),
            2,
        );
        let cut: String = long.chars().take(159).chain(['…']).collect();
        let prompt = |terminal_id: &str, last_prompt: &str| {
            let mut agent: Agent = serde_json::from_value(serde_json::json!({
                "terminal_id": terminal_id, "workspace_id": "w1", "kind": "claude",
                "name": null, "title": null, "status": "idle", "status_since_ms": 1,
                "cwd": null, "last_line": null, "last_prompt": last_prompt,
            }))
            .unwrap();
            wakes.mark_prompt(&mut agent);
            agent.last_prompt_origin
        };
        assert_eq!(
            prompt("t1", &cut),
            Some(MessageOrigin {
                kind: "pr_watch".into(),
                summary: "CI failed on #1".into()
            })
        );
        assert_eq!(
            prompt("t1", "Pull request #2 changed: CI failed."),
            Some(MessageOrigin {
                kind: "pr_watch".into(),
                summary: "CI failed on #2".into()
            }),
            "a short wake is its own first line"
        );
        assert_eq!(prompt("t1", "fix the build"), None);
        assert_eq!(
            prompt("t2", &cut),
            None,
            "only on the terminal it was typed in"
        );
        let typed = format!("{}, then fix it", &long[..long.len() - 1]);
        let typed_cut: String = typed.chars().take(159).chain(['…']).collect();
        assert!(
            prompt("t1", &typed_cut).is_some(),
            "accepted: the Inbox shows only the cut line, so text after it is not compared"
        );
    }

    #[test]
    fn a_watch_ends_with_its_terminal_or_its_phone() {
        let mut watches = Watches::default();
        for (terminal_id, by) in [("t1", "nA"), ("t2", "nA"), ("t3", "nB")] {
            let mut watch = Watch::new(URL, 14673);
            watch.by = by.into();
            watches.start(terminal_id, watch);
        }
        let agent = |terminal_id: &str| -> AgentInfo {
            serde_json::from_value(serde_json::json!({
                "terminal_id": terminal_id, "workspace_id": "w1", "pane_id": "w1:p1",
                "agent_status": "idle", "agent": "claude",
            }))
            .unwrap()
        };
        assert!(!watches.retain_listed(&[]), "an empty list is not trusted");
        assert!(!watches.retain_listed(&[agent("t1"), agent("t2"), agent("t3")]));
        assert!(watches.retain_listed(&[agent("t1"), agent("t3"), agent("t9")]));
        assert_eq!(watches.shown("t2"), None);
        assert!(!watches.retain_paired(|by| by == "nA" || by == "nB"));
        assert!(
            watches.retain_paired(|by| by == "nB"),
            "a phone revoked offline"
        );
        assert_eq!(watches.shown("t1"), None);
        assert!(watches.shown("t3").is_some());
    }
}
