use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use super::subagents::alive;
use super::{Kind, Transcripts, is_file, is_session_id, open, project_dir, session_id};
use crate::herdr::{AgentInfo, PaneProcesses};

const SESSION_BYTES: u64 = 64 * 1024;
const POINTER_BYTES: u64 = 64 * 1024;
const MAX_HOPS: usize = 4;
const MAX_PIDS: usize = 16;
const MAX_SESSION_FILES: usize = 256;
/// Linux gives a process start to the second.
const START_SLACK_MS: u64 = 2_000;
const PANE_EVERY: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(super) struct Pane {
    pane_id: String,
    at: Instant,
    pids: Vec<i32>,
    attach: Option<String>,
}

/// Rebuilt when herdr's id or the pane's pids are refreshed; followed again when the
/// starting transcript's size or mtime moves.
pub(super) struct Resolved {
    herdr: Option<String>,
    pane_at: Option<Instant>,
    start: Option<String>,
    path: Option<PathBuf>,
    stamp: Option<(u64, SystemTime)>,
    id: Option<String>,
}

struct Claim {
    session: String,
    cwd: String,
    kind: Option<String>,
    job: Option<String>,
    name: Option<String>,
}

enum Pointer {
    Continued(String),
    Attached(String),
}

impl Transcripts {
    pub fn stale_panes(&self, agents: &[AgentInfo]) -> Vec<(String, String)> {
        agents
            .iter()
            .filter(|a| a.agent.as_deref() == Some("claude"))
            .filter(|a| {
                self.panes
                    .get(&a.terminal_id)
                    .is_none_or(|p| p.pane_id != a.pane_id || p.at.elapsed() >= PANE_EVERY)
            })
            .map(|a| (a.terminal_id.clone(), a.pane_id.clone()))
            .collect()
    }

    /// `None` when herdr could not list them: the processes last listed for this pane stand.
    pub fn set_pane(&mut self, terminal_id: String, pane_id: String, procs: Option<PaneProcesses>) {
        let kept = self
            .panes
            .remove(&terminal_id)
            .filter(|p| p.pane_id == pane_id)
            .map(|p| (p.pids, p.attach));
        let (pids, attach) = match procs {
            Some(procs) => (
                procs
                    .pids
                    .into_iter()
                    .filter_map(|p| i32::try_from(p).ok())
                    .take(MAX_PIDS)
                    .collect(),
                procs.attach,
            ),
            None => kept.unwrap_or_default(),
        };
        self.panes.insert(
            terminal_id,
            Pane {
                pane_id,
                at: Instant::now(),
                pids,
                attach,
            },
        );
    }

    /// For Claude Code, the session a process in the pane reports, else herdr's, then where
    /// that session's own transcript says the conversation went.
    pub(super) fn live_id(&self, a: &AgentInfo) -> Option<String> {
        if a.agent.as_deref() != Some("claude") {
            return session_id(a).map(str::to_owned);
        }
        let (start, path) = self.start(a, true);
        self.follow(start, path.as_deref())
    }

    /// `live_id`, cached per terminal.
    pub(super) fn resolve(&mut self, a: &AgentInfo, relocate: bool) -> Option<String> {
        let herdr = session_id(a).map(str::to_owned);
        if a.agent.as_deref() != Some("claude") {
            return herdr;
        }
        let pane_at = self
            .panes
            .get(&a.terminal_id)
            .filter(|p| p.pane_id == a.pane_id)
            .map(|p| p.at);
        let (mut r, fresh) = match self.resolved.remove(&a.terminal_id) {
            Some(r)
                if r.herdr == herdr && r.pane_at == pane_at && (r.path.is_some() || !relocate) =>
            {
                (r, false)
            }
            _ => {
                let (start, path) = self.start(a, relocate);
                let r = Resolved {
                    herdr,
                    pane_at,
                    start,
                    path,
                    stamp: None,
                    id: None,
                };
                (r, true)
            }
        };
        let stamp = r.path.as_deref().and_then(stamp);
        if fresh || stamp != r.stamp {
            r.stamp = stamp;
            r.id = self.follow(r.start.clone(), r.path.as_deref());
        }
        let id = r.id.clone();
        self.resolved.insert(a.terminal_id.clone(), r);
        id
    }

    fn start(&self, a: &AgentInfo, relocate: bool) -> (Option<String>, Option<PathBuf>) {
        let sessions = self.sessions_dir();
        let start = sessions
            .as_deref()
            .and_then(|s| self.pane_session(s, a).or_else(|| self.attached_to(s, a)))
            .or_else(|| session_id(a).map(str::to_owned));
        let path = start
            .as_deref()
            .filter(|id| relocate || !self.missed.contains(*id))
            .and_then(|id| self.locate(Kind::Claude, id, a));
        (start, path)
    }

    fn follow(&self, start: Option<String>, path: Option<&Path>) -> Option<String> {
        match path {
            Some(path) => Some(follow(path, start?, self.sessions_dir().as_deref())),
            None => start,
        }
    }

    pub(super) fn sessions_dir(&self) -> Option<PathBuf> {
        let dir = self.claude_projects.as_deref()?.parent()?.join("sessions");
        std::fs::symlink_metadata(&dir)
            .is_ok_and(|m| m.is_dir())
            .then_some(dir)
    }

    /// Only when every trusted process in the pane reports the same session in the pane's cwd.
    fn pane_session(&self, sessions: &Path, a: &AgentInfo) -> Option<String> {
        let pane = self
            .panes
            .get(&a.terminal_id)
            .filter(|p| p.pane_id == a.pane_id)?;
        let cwds = [&a.foreground_cwd, &a.cwd];
        let mut found = pane
            .pids
            .iter()
            .filter_map(|pid| claim(sessions, *pid))
            .filter(|c| cwds.iter().any(|d| d.as_deref() == Some(c.cwd.as_str())))
            .map(|c| c.session);
        let first = found.next()?;
        found.all(|s| s == first).then_some(first)
    }

    /// `claude attach` takes a session id prefix or a name; only one live `bg` process may match.
    fn attached_to(&self, sessions: &Path, a: &AgentInfo) -> Option<String> {
        let target = self
            .panes
            .get(&a.terminal_id)
            .filter(|p| p.pane_id == a.pane_id)?
            .attach
            .as_deref()
            .filter(|t| !t.is_empty())?;
        unique(
            background(sessions)?
                .filter(|c| c.session.starts_with(target) || c.name.as_deref() == Some(target)),
        )
    }
}

fn unique(mut found: impl Iterator<Item = Claim>) -> Option<String> {
    let first = found.next()?.session;
    found.all(|c| c.session == first).then_some(first)
}

fn background(sessions: &Path) -> Option<impl Iterator<Item = Claim>> {
    let files = std::fs::read_dir(sessions).ok()?;
    Some(
        files
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()?
                    .strip_suffix(".json")?
                    .parse::<i32>()
                    .ok()
            })
            .take(MAX_SESSION_FILES)
            .filter_map(move |pid| claim(sessions, pid))
            .filter(|c| c.kind.as_deref() == Some("bg")),
    )
}

fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// Trusted only while that pid is alive and started before the file says it did, so a file
/// left by a process whose pid was reused is refused.
fn claim(sessions: &Path, pid: i32) -> Option<Claim> {
    if pid <= 0 {
        return None;
    }
    let (file, _, _) = open(&sessions.join(format!("{pid}.json")))?;
    let mut bytes = Vec::new();
    file.take(SESSION_BYTES).read_to_end(&mut bytes).ok()?;
    let v = serde_json::from_slice::<Value>(&bytes).ok()?;
    if v["pid"] != pid {
        return None;
    }
    let session = v["sessionId"].as_str().filter(|s| is_session_id(s))?;
    let started_at = v["startedAt"].as_u64()?;
    let claim = Claim {
        session: session.to_owned(),
        cwd: v["cwd"].as_str()?.to_owned(),
        kind: v["kind"].as_str().map(str::to_owned),
        job: v["jobId"].as_str().map(str::to_owned),
        name: v["name"].as_str().map(str::to_owned),
    };
    (alive(pid) && started_ms(pid)? <= started_at.saturating_add(START_SLACK_MS)).then_some(claim)
}

/// Bounded and within one project directory: a loop or a longer chain keeps `start`.
fn follow(path: &Path, start: String, sessions: Option<&Path>) -> String {
    let Some(dir) = path.parent() else {
        return start;
    };
    let mut chain = vec![start];
    let mut path = path.to_owned();
    loop {
        let id = &chain[chain.len() - 1];
        let Some(next) = moved_to(&path, id, dir, sessions) else {
            return chain.pop().unwrap_or_default();
        };
        if chain.contains(&next) || chain.len() > MAX_HOPS {
            return chain.swap_remove(0);
        }
        path = dir.join(format!("{next}.jsonl"));
        chain.push(next);
    }
}

/// The latest hand-off since the session's last turn that resolves: a `continued-in`
/// record whose target's transcript is in `dir`, or `/resume` opening a background session
/// that a live `bg` process of `dir` runs.
fn moved_to(path: &Path, id: &str, dir: &Path, sessions: Option<&Path>) -> Option<String> {
    let (mut file, len, _) = open(path)?;
    let start = len.saturating_sub(POINTER_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(POINTER_BYTES).read_to_end(&mut bytes).ok()?;
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    if !has(b"continued-in") && !has(b"commandRun") {
        return None;
    }
    let mut pointers = Vec::new();
    for line in bytes.split(|b| *b == b'\n').skip(usize::from(start > 0)) {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if v["sessionId"] != id {
            continue;
        }
        match v["type"].as_str() {
            Some("assistant") => pointers.clear(),
            Some("user") if v["origin"]["kind"] == "human" => pointers.clear(),
            Some("continued-in") => {
                if let Some(to) = v["continuedInSessionId"].as_str() {
                    pointers.push(Pointer::Continued(to.to_owned()));
                }
            }
            Some("system")
                if v["subtype"] == "local_command" && v["commandRun"]["command"] == "resume" =>
            {
                if let Some(job) = v["content"].as_str().and_then(background_job) {
                    pointers.push(Pointer::Attached(job.to_owned()));
                }
            }
            _ => {}
        }
    }
    pointers.into_iter().rev().find_map(|p| match p {
        Pointer::Continued(to) => {
            (is_session_id(&to) && to != id && is_file(&dir.join(format!("{to}.jsonl"))))
                .then_some(to)
        }
        Pointer::Attached(job) => attached(sessions?, dir, &job),
    })
}

/// The job id at the end of `/resume`'s output, `… running in the background (<job>)`.
fn background_job(content: &str) -> Option<&str> {
    let rest = content.strip_suffix(")</local-command-stdout>")?;
    let job = rest.get(rest.len().checked_sub(8)?..)?;
    (rest[..rest.len() - 8].ends_with("running in the background (")
        && job
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(job)
}

fn attached(sessions: &Path, dir: &Path, job: &str) -> Option<String> {
    let name = dir.file_name()?.to_str()?;
    unique(background(sessions)?.filter(|c| {
        c.job.as_deref() == Some(job)
            && c.session.starts_with(job)
            && project_dir(&c.cwd) == name
            && is_file(&dir.join(format!("{}.jsonl", c.session)))
    }))
}

#[cfg(target_os = "macos")]
fn started_ms(pid: i32) -> Option<u64> {
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    // SAFETY: `info` is a writable `proc_bsdinfo` of `size` bytes, what PROC_PIDTBSDINFO fills.
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            i32::try_from(size).ok()?,
        )
    };
    if usize::try_from(n).ok()? != size {
        return None;
    }
    // SAFETY: zeroed, then filled whole by the call above.
    let info = unsafe { info.assume_init() };
    Some(info.pbi_start_tvsec * 1000 + info.pbi_start_tvusec / 1000)
}

#[cfg(target_os = "linux")]
fn started_ms(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ticks: u64 = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()?;
    let boot: u64 = std::fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    let hz = rustix::param::clock_ticks_per_second();
    Some(boot * 1000 + ticks * 1000 / hz.max(1))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use protocol::{HistoryParams, TerminalId};

    use super::*;
    use crate::herdr::AgentSession;

    const CLAUDE: &str = include_str!("../../tests/fixtures/transcripts/claude.jsonl");
    const LIVE: &str = "3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    const OLD: &str = "0a0a0a0a-0000-4000-8000-000000000001";
    const CLIENT: &str = "0c0c0c0c-0000-4000-8000-000000000002";
    const PARKED: &str = "0d0d0d0d-0000-4000-8000-000000000003";
    const CWD: &str = "/Users/me/src/collie";
    const DEAD: i32 = 0x7fff_fff0;

    fn setup() -> (tempfile::TempDir, Transcripts, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        let t = Transcripts::new(Some(claude.join("projects")), None);
        let project = claude.join("projects").join(project_dir(CWD));
        let sessions = claude.join("sessions");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&sessions).unwrap();
        (dir, t, project, sessions)
    }

    fn agent(id: &str) -> AgentInfo {
        AgentInfo {
            terminal_id: "term_1".into(),
            workspace_id: "w1".into(),
            pane_id: "w1:p1".into(),
            agent: Some("claude".into()),
            name: None,
            title: None,
            terminal_title_stripped: None,
            agent_status: "idle".into(),
            state_change_seq: 1,
            cwd: Some(CWD.into()),
            foreground_cwd: Some(CWD.into()),
            agent_session: Some(AgentSession { value: id.into() }),
            interactive_ready: true,
            launch_pending: false,
        }
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn transcript(project: &Path, id: &str, text: &str) {
        write(&project.join(format!("{id}.jsonl")), text);
    }

    fn reply(id: &str, text: &str) -> String {
        format!(
            r#"{{"isSidechain":false,"type":"assistant","message":{{"model":"claude-opus-5-5","role":"assistant","content":[{{"type":"text","text":"{text}"}}],"stop_reason":"end_turn","usage":{{"input_tokens":1000}}}},"sessionId":"{id}"}}"#
        ) + "\n"
    }

    fn continued(id: &str, to: &str) -> String {
        format!(r#"{{"type":"continued-in","sessionId":"{id}","continuedInSessionId":"{to}"}}"#)
            + "\n"
    }

    /// What a fresh client writes when `/resume` opens a background session.
    fn resumed(job: &str) -> String {
        [
            format!(
                r#"{{"type":"system","subtype":"local_command","content":"<command-name>/resume</command-name>","isMeta":false,"cwd":"{CWD}","sessionId":"{CLIENT}"}}"#
            ),
            format!(
                r#"{{"type":"system","subtype":"local_command","content":"<local-command-stdout>Opening \"Fix (deadbeef) things\", running in the background ({job})</local-command-stdout>","commandRun":{{"command":"resume","args":""}},"isMeta":false,"cwd":"{CWD}","sessionId":"{CLIENT}"}}"#
            ),
            continued(CLIENT, PARKED),
        ]
        .join("\n")
    }

    fn pid_file(sessions: &Path, pid: i32, id: &str, cwd: &str, started_at: u64, extra: &str) {
        write(
            &sessions.join(format!("{pid}.json")),
            &format!(
                r#"{{"pid":{pid},"sessionId":"{id}","cwd":"{cwd}","startedAt":{started_at}{extra}}}"#
            ),
        );
    }

    fn own() -> (i32, u64) {
        let pid = std::process::id() as i32;
        (pid, started_ms(pid).unwrap())
    }

    fn parent() -> (i32, u64) {
        let pid = std::os::unix::process::parent_id() as i32;
        (pid, started_ms(pid).unwrap())
    }

    fn pane(t: &mut Transcripts, pids: &[i32]) {
        attach(t, pids, None);
    }

    fn attach(t: &mut Transcripts, pids: &[i32], target: Option<&str>) {
        t.set_pane(
            "term_1".into(),
            "w1:p1".into(),
            Some(PaneProcesses {
                pids: pids.iter().map(|p| *p as u32).collect(),
                attach: target.map(str::to_owned),
            }),
        );
    }

    #[test]
    fn the_pane_process_replaces_a_stale_herdr_id() {
        let (_dir, mut t, project, sessions) = setup();
        transcript(&project, OLD, &reply(OLD, "Old reply."));
        transcript(&project, LIVE, CLAUDE);
        let a = agent(OLD);
        let (pid, started) = own();
        pid_file(&sessions, pid, LIVE, CWD, started, "");
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD));

        pane(&mut t, &[DEAD, pid]);
        assert_eq!(t.stale_panes(std::slice::from_ref(&a)), []);
        let d = t.derive(&a, true).unwrap();
        assert_eq!(
            (d.context_left, d.last_line.as_deref()),
            (Some(94), Some("**Fixed** the flaky test."))
        );
        let params = HistoryParams {
            terminal_id: TerminalId::new("term_1").unwrap(),
            before: None,
            after: None,
            query: None,
        };
        let page = t.history_reader().history(&a, &params);
        assert!(
            page.session
                .unwrap()
                .starts_with(&format!("claude:{LIVE}:"))
        );

        let mut moved = a.clone();
        moved.pane_id = "w1:p2".into();
        assert_eq!(t.live_id(&moved).as_deref(), Some(OLD));
        assert_eq!(t.stale_panes(&[moved]).len(), 1);
        let mut none = a.clone();
        none.agent_session = None;
        assert_eq!(t.live_id(&none).as_deref(), Some(LIVE));
    }

    #[test]
    fn untrusted_pid_files_keep_herdr_id() {
        let (dir, mut t, project, sessions) = setup();
        transcript(&project, OLD, &reply(OLD, "Old reply."));
        transcript(&project, LIVE, CLAUDE);
        let a = agent(OLD);
        let (pid, started) = own();

        pid_file(&sessions, pid, LIVE, "/Users/me/src/other", started, "");
        pane(&mut t, &[pid]);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "cwd mismatch");

        pid_file(&sessions, pid, LIVE, CWD, started - 60_000, "");
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "reused pid");

        pid_file(&sessions, DEAD, LIVE, CWD, started, "");
        pane(&mut t, &[DEAD]);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "dead pid");

        write(
            &sessions.join(format!("{pid}.json")),
            &format!(r#"{{"pid":1,"sessionId":"{LIVE}","cwd":"{CWD}","startedAt":{started}}}"#),
        );
        pane(&mut t, &[pid]);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "pid not its own");

        let (other, other_started) = parent();
        pid_file(&sessions, pid, LIVE, CWD, started, "");
        pid_file(&sessions, other, CLIENT, CWD, other_started, "");
        pane(&mut t, &[pid, other]);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "two sessions");

        let elsewhere = dir.path().join("elsewhere.json");
        pid_file(dir.path(), pid, LIVE, CWD, started, "");
        std::fs::rename(dir.path().join(format!("{pid}.json")), &elsewhere).unwrap();
        std::fs::remove_file(sessions.join(format!("{pid}.json"))).unwrap();
        std::os::unix::fs::symlink(&elsewhere, sessions.join(format!("{pid}.json"))).unwrap();
        pane(&mut t, &[pid]);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "symlink");
        std::fs::remove_file(sessions.join(format!("{pid}.json"))).unwrap();
        std::fs::copy(&elsewhere, sessions.join(format!("{pid}.json"))).unwrap();
        assert_eq!(
            t.live_id(&a).as_deref(),
            Some(LIVE),
            "the same file in place"
        );
    }

    #[test]
    fn resolution_is_cached_until_the_pids_or_the_transcript_move() {
        let (_dir, mut t, project, sessions) = setup();
        transcript(&project, OLD, &reply(OLD, "Old reply."));
        transcript(&project, LIVE, CLAUDE);
        let a = agent(OLD);
        let (pid, started) = own();
        pid_file(&sessions, pid, LIVE, CWD, started, "");
        pane(&mut t, &[pid]);
        assert_eq!(t.resolve(&a, false).as_deref(), Some(LIVE));

        let file = sessions.join(format!("{pid}.json"));
        std::fs::remove_file(&file).unwrap();
        assert_eq!(t.resolve(&a, false).as_deref(), Some(LIVE), "cached");

        t.set_pane("term_1".into(), "w1:p1".into(), None);
        assert_eq!(t.panes["term_1"].pids, [pid], "kept on a herdr error");
        assert_eq!(t.resolve(&a, false).as_deref(), Some(OLD), "refreshed");
        pid_file(&sessions, pid, LIVE, CWD, started, "");
        t.set_pane("term_1".into(), "w1:p1".into(), None);
        assert_eq!(t.resolve(&a, false).as_deref(), Some(LIVE));

        transcript(&project, LIVE, &(CLAUDE.to_owned() + &continued(LIVE, OLD)));
        assert_eq!(
            t.resolve(&a, false).as_deref(),
            Some(OLD),
            "transcript moved"
        );
    }

    #[test]
    fn continued_in_is_followed_within_the_project_only() {
        let (dir, t, project, _sessions) = setup();
        let a = agent(OLD);
        transcript(
            &project,
            OLD,
            &(reply(OLD, "Backgrounding.") + &continued(OLD, LIVE)),
        );
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "no target transcript");

        let elsewhere = dir.path().join("claude/projects/-Users-me-src-other");
        transcript(&elsewhere, LIVE, CLAUDE);
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "other project");

        transcript(&project, LIVE, CLAUDE);
        assert_eq!(t.live_id(&a).as_deref(), Some(LIVE));

        transcript(
            &project,
            OLD,
            &(continued(OLD, LIVE) + &reply(OLD, "Back in the foreground.")),
        );
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "a later turn");

        transcript(&project, OLD, &continued(OLD, OLD));
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "self");

        transcript(&project, OLD, &continued(OLD, LIVE));
        transcript(&project, LIVE, &continued(LIVE, OLD));
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "loop");

        let ids: Vec<String> = (1..=6)
            .map(|n| format!("0b0b0b0b-0000-4000-8000-00000000000{n}"))
            .collect();
        transcript(&project, OLD, &continued(OLD, &ids[0]));
        for w in ids.windows(2) {
            transcript(&project, &w[0], &continued(&w[0], &w[1]));
        }
        transcript(&project, &ids[5], "");
        assert_eq!(t.live_id(&a).as_deref(), Some(OLD), "too long");
        transcript(&project, OLD, &continued(OLD, &ids[3]));
        assert_eq!(t.live_id(&a).as_deref(), Some(ids[5].as_str()));
    }

    #[test]
    fn resume_into_a_background_session_follows_it() {
        let (_dir, mut t, project, sessions) = setup();
        let a = agent(CLIENT);
        let (pid, started) = own();
        let (bg, bg_started) = parent();
        transcript(&project, CLIENT, &resumed("3f1b2c4d"));
        transcript(&project, LIVE, CLAUDE);
        pid_file(&sessions, pid, CLIENT, CWD, started, "");
        pane(&mut t, &[pid]);
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "no live bg process");

        let job = r#","jobId":"3f1b2c4d""#;
        let interactive = format!(r#"{job},"kind":"interactive""#);
        pid_file(&sessions, bg, LIVE, CWD, bg_started, &interactive);
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "not kind bg");

        let bg_kind = format!(r#"{job},"kind":"bg""#);
        pid_file(
            &sessions,
            bg,
            LIVE,
            "/Users/me/src/other",
            bg_started,
            &bg_kind,
        );
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "other project");

        pid_file(&sessions, bg, LIVE, CWD, bg_started - 60_000, &bg_kind);
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "reused pid");

        pid_file(&sessions, bg, LIVE, CWD, bg_started, &bg_kind);
        transcript(&project, CLIENT, &resumed("0a0a0a0a"));
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "another job");

        transcript(&project, CLIENT, &resumed("3f1b2c4d"));
        assert_eq!(t.live_id(&a).as_deref(), Some(LIVE));

        let sub = project.join(LIVE).join("subagents");
        write(
            &sub.join("agent-a1.meta.json"),
            include_str!("../../tests/fixtures/transcripts/claude-live-agent.meta.json"),
        );
        let lines = include_str!("../../tests/fixtures/transcripts/claude-live-agent.jsonl")
            .split_inclusive('\n')
            .collect::<Vec<_>>();
        write(&sub.join("agent-a1.jsonl"), &lines[..4].concat());
        let d = t.derive(&a, true).unwrap();
        assert_eq!(
            (d.context_left, d.last_line.as_deref(), d.subagents.len()),
            (Some(94), Some("**Fixed** the flaky test."), 1)
        );

        transcript(
            &project,
            CLIENT,
            &(resumed("3f1b2c4d") + &reply(CLIENT, "Back in the client.")),
        );
        assert_eq!(t.live_id(&a).as_deref(), Some(CLIENT), "a later turn");
    }

    #[test]
    fn claude_attach_follows_the_background_session() {
        let (_dir, mut t, project, sessions) = setup();
        let mut a = agent(OLD);
        a.agent_session = None;
        let (pid, started) = own();
        let (bg, bg_started) = parent();
        transcript(&project, LIVE, CLAUDE);
        let named = r#","kind":"bg","jobId":"3f1b2c4d","name":"collie app""#;
        pid_file(&sessions, bg, LIVE, CWD, bg_started, named);

        attach(&mut t, &[pid], None);
        assert_eq!(t.live_id(&a), None, "not an attach");
        for unknown in ["0a0a0a0a", "collie", "", "collie app 2"] {
            attach(&mut t, &[pid], Some(unknown));
            assert_eq!(t.live_id(&a), None, "{unknown:?}");
        }
        for target in ["3f1b2c4d", "3f1b", LIVE, "collie app"] {
            attach(&mut t, &[pid], Some(target));
            assert_eq!(t.live_id(&a).as_deref(), Some(LIVE), "{target}");
        }

        let sub = project.join(LIVE).join("subagents");
        write(
            &sub.join("agent-a1.meta.json"),
            include_str!("../../tests/fixtures/transcripts/claude-live-agent.meta.json"),
        );
        let lines = include_str!("../../tests/fixtures/transcripts/claude-live-agent.jsonl")
            .split_inclusive('\n')
            .collect::<Vec<_>>();
        write(&sub.join("agent-a1.jsonl"), &lines[..4].concat());
        attach(&mut t, &[pid], Some("3f1b2c4d"));
        let d = t.derive(&a, true).unwrap();
        assert_eq!(
            (d.context_left, d.last_line.as_deref(), d.subagents.len()),
            (Some(94), Some("**Fixed** the flaky test."), 1)
        );
        assert!(d.last_prompt.is_some());
        let params = HistoryParams {
            terminal_id: TerminalId::new("term_1").unwrap(),
            before: None,
            after: None,
            query: None,
        };
        let page = t.history_reader().history(&a, &params);
        assert!(
            page.session
                .unwrap()
                .starts_with(&format!("claude:{LIVE}:"))
        );

        assert_eq!(
            t.live_id(&agent(OLD)).as_deref(),
            Some(LIVE),
            "over a stale herdr id"
        );

        let other = "3f1b0000-0000-4000-8000-000000000004";
        pid_file(
            &sessions,
            pid,
            other,
            CWD,
            started,
            r#","kind":"bg","name":"3f1b2c4d""#,
        );
        attach(&mut t, &[DEAD], Some("3f1b"));
        assert_eq!(t.live_id(&a), None, "two prefixes");
        attach(&mut t, &[DEAD], Some("3f1b2c4d"));
        assert_eq!(t.live_id(&a), None, "a prefix and a name");
        attach(&mut t, &[DEAD], Some("3f1b2c"));
        assert_eq!(t.live_id(&a).as_deref(), Some(LIVE), "unique prefix");
        std::fs::remove_file(sessions.join(format!("{pid}.json"))).unwrap();

        pid_file(
            &sessions,
            bg,
            LIVE,
            CWD,
            bg_started,
            r#","kind":"interactive""#,
        );
        assert_eq!(t.live_id(&a), None, "not kind bg");
        pid_file(&sessions, bg, LIVE, CWD, bg_started - 60_000, named);
        assert_eq!(t.live_id(&a), None, "reused pid");
        std::fs::remove_file(sessions.join(format!("{bg}.json"))).unwrap();
        pid_file(&sessions, DEAD, LIVE, CWD, bg_started, named);
        assert_eq!(t.live_id(&a), None, "dead pid");
    }

    #[test]
    fn background_job_is_read_from_the_end_of_the_output() {
        assert_eq!(
            background_job(
                "<local-command-stdout>Opening \"x (deadbeef)\", running in the background (1a2b3c4d)</local-command-stdout>"
            ),
            Some("1a2b3c4d")
        );
        for bad in [
            "<local-command-stdout>Opening \"x\", running in the background (1a2b3c4d)!</local-command-stdout>",
            "<local-command-stdout>running in the background (1A2B3C4D)</local-command-stdout>",
            "<local-command-stdout>running in the background (1a2b3c4)</local-command-stdout>",
            "<local-command-stdout>running in the background(1a2b3c4d)</local-command-stdout>",
            "<local-command-stdout>Opening \"x (1a2b3c4d)\"</local-command-stdout>",
            "<local-command-stdout>Resumed \"x\" (1a2b3c4d)</local-command-stdout>",
            "(1a2b3c4d)</local-command-stdout>",
        ] {
            assert_eq!(background_job(bad), None, "{bad}");
        }
    }
}
