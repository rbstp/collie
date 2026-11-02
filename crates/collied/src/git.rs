use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures_util::StreamExt;
use protocol::{
    AgentDiffParams, Cwd, ErrorCode, GitChanges, GitDiff, GitFile, GitPullRequest, GitScope,
    GitSection, GitWorktree, PullState, hex,
};
use rustix::fs::{Mode, OFlags};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::drive::{Fail, resolve_cwd};
use crate::runtime::herdr::{WorktreeInfo, WorktreeList, WorktreeSource};

const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const FILE_LIMIT: u64 = 1024 * 1024;
const RESPONSE_LIMIT: usize = 48 * 1024;
const MAX_FILES: usize = 200;
const MAX_WORKTREES: usize = 20;
const PULL_PAGE: usize = 100;

fn error(message: &str) -> Fail {
    (ErrorCode::InvalidParams, message.into())
}

async fn git(cwd: &Cwd, args: &[&str], limit: usize) -> Result<(Vec<u8>, bool, bool), Fail> {
    let mut overrides = Vec::new();
    if matches!(args.first(), Some(&"diff" | &"status")) {
        let (out, _, truncated) = git_command(
            cwd,
            &[
                "config",
                "--null",
                "--name-only",
                "--get-regexp",
                "^filter\\..*\\.(clean|process|required)$",
            ],
            64 * 1024,
            &[],
        )
        .await?;
        if truncated {
            return Err(error("Too many Git filters to safely read changes"));
        }
        for key in out.split(|b| *b == 0).filter(|s| !s.is_empty()) {
            let key =
                std::str::from_utf8(key).map_err(|_| error("Invalid Git filter configuration"))?;
            overrides.push(format!(
                "{key}={}",
                if key.ends_with(".required") {
                    "false"
                } else {
                    ""
                }
            ));
        }
    }
    git_command(cwd, args, limit, &overrides).await
}

async fn git_command(
    cwd: &Cwd,
    args: &[&str],
    limit: usize,
    overrides: &[String],
) -> Result<(Vec<u8>, bool, bool), Fail> {
    let mut command = Command::new("git");
    command.current_dir(cwd.as_str()).args([
        "--no-pager",
        "--literal-pathspecs",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "fetch.bundleURI=",
        "-c",
        "diff.external=",
        "-c",
        "diff.submodule=short",
        "-c",
        "diff.renames=true",
        "-c",
        "diff.renameLimit=1000",
    ]);
    for value in overrides {
        command.args(["-c", value]);
    }
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    // A partial clone's lazy fetch runs repository-configured programs (uploadpack,
    // sshCommand, credential helpers). GIT_NO_LAZY_FETCH needs git 2.45 or a 2.39.4 to
    // 2.44.1 security release; for older git, an empty GIT_ALLOW_PROTOCOL refuses every
    // transport and outranks repository config, and the blank fetch.bundleURI above stops
    // the fetch from starting git-remote-https before that check.
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_ALLOW_PROTOCOL", "");
    tokio::time::timeout(Duration::from_secs(8), async {
        let mut child = command.spawn().map_err(|_| error("Git is unavailable"))?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| error("Git output unavailable"))?
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| error("Could not read Git output"))?;
        if bytes.len() > limit {
            child
                .kill()
                .await
                .map_err(|_| error("Could not stop Git"))?;
            bytes.truncate(limit);
            return Ok((bytes, true, true));
        }
        let status = child.wait().await.map_err(|_| error("Git failed"))?;
        Ok((bytes, status.success(), false))
    })
    .await
    .map_err(|_| error("Git took too long; try again"))?
}

/// The first line of a short answer, or none when git fails.
pub(crate) async fn line(cwd: &Cwd, args: &[&str]) -> Result<Option<String>, Fail> {
    let (out, ok, truncated) = git_command(cwd, args, 4096, &[]).await?;
    Ok((ok && !truncated).then(|| {
        String::from_utf8_lossy(&out)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
    }))
}

async fn root(cwd: &Cwd, roots: &[PathBuf]) -> Result<Option<Cwd>, Fail> {
    let (out, ok, truncated) = git(cwd, &["rev-parse", "--show-toplevel"], 4096).await?;
    if !ok {
        return Ok(None);
    }
    if truncated {
        return Err(error("Git checkout path is too long"));
    }
    let path = std::str::from_utf8(&out)
        .map_err(|_| error("Git checkout path is not UTF-8"))?
        .trim_end_matches('\n');
    let root = resolve_cwd(path, roots).map_err(error)?;
    for option in ["--absolute-git-dir", "--git-common-dir"] {
        metadata(&root, option, roots).await?;
    }
    Ok(Some(root))
}

async fn metadata(root: &Cwd, option: &str, roots: &[PathBuf]) -> Result<Cwd, Fail> {
    let (out, ok, truncated) = git(root, &["rev-parse", option], 4096).await?;
    if !ok || truncated {
        return Err(error("Git metadata is unavailable"));
    }
    let path = std::str::from_utf8(&out)
        .map_err(|_| error("Invalid Git metadata path"))?
        .trim_end_matches('\n');
    let path = Path::new(root.as_str()).join(path);
    resolve_cwd(
        path.to_str()
            .ok_or_else(|| error("Invalid Git metadata path"))?,
        roots,
    )
    .map_err(error)
}

async fn worktrees(checkout: &Cwd, roots: &[PathBuf]) -> Result<Vec<GitWorktree>, Fail> {
    let (out, ok, truncated) = git(
        checkout,
        &["worktree", "list", "--porcelain", "-z"],
        64 * 1024,
    )
    .await?;
    if !ok || truncated {
        return Ok(Vec::new());
    }
    let lines: Vec<&[u8]> = out.split(|b| *b == 0).collect();
    let mut list = Vec::new();
    for record in lines.split(|line| line.is_empty()) {
        let mut root = None;
        let mut branch = None;
        for line in record {
            if let Some(path) = line.strip_prefix(b"worktree ") {
                root = std::str::from_utf8(path)
                    .ok()
                    .and_then(|p| resolve_cwd(p, roots).ok());
            } else if let Some(name) = line.strip_prefix(b"branch refs/heads/") {
                branch = std::str::from_utf8(name).ok().map(str::to_owned);
            } else if *line == b"bare" || line.starts_with(b"prunable") {
                root = None;
                break;
            }
        }
        if let Some(root) = root
            && (list.len() < MAX_WORKTREES || &root == checkout)
        {
            list.push(GitWorktree {
                root,
                branch,
                subject: None,
                committed_at_ms: None,
                ahead: None,
                uncommitted: None,
                additions: None,
                deletions: None,
                pull_request: None,
            });
        }
    }
    Ok(list)
}

/// What herdr's `worktree.list` answers, for a herdr-core task: the main checkout is the
/// first entry and the source, every other entry a linked worktree.
pub(crate) async fn worktree_list(cwd: &Cwd) -> Result<WorktreeList, Fail> {
    let (out, ok, truncated) = git_command(
        cwd,
        &["worktree", "list", "--porcelain", "-z"],
        256 * 1024,
        &[],
    )
    .await?;
    if !ok || truncated {
        return Err(error("Git could not list the worktrees"));
    }
    let lines: Vec<&[u8]> = out.split(|b| *b == 0).collect();
    let mut worktrees: Vec<WorktreeInfo> = Vec::new();
    for (index, record) in lines
        .split(|line| line.is_empty())
        .filter(|record| !record.is_empty())
        .enumerate()
    {
        let mut path = None;
        let mut branch = None;
        let mut bare = false;
        let mut locked = false;
        for line in record {
            if let Some(p) = line.strip_prefix(b"worktree ") {
                path = std::str::from_utf8(p).ok().map(str::to_owned);
            } else if let Some(name) = line.strip_prefix(b"branch refs/heads/") {
                branch = std::str::from_utf8(name).ok().map(str::to_owned);
            } else if *line == b"bare" {
                bare = true;
            } else if *line == b"locked" || line.starts_with(b"locked ") {
                locked = true;
            }
        }
        if let Some(path) = path {
            worktrees.push(WorktreeInfo {
                path,
                branch,
                open_workspace_id: None,
                is_bare: bare,
                is_linked_worktree: index > 0,
                locked,
            });
        }
    }
    let source_checkout_path = worktrees
        .first()
        .ok_or_else(|| error("Git listed no worktree"))?
        .path
        .clone();
    Ok(WorktreeList {
        source: WorktreeSource {
            source_checkout_path,
        },
        worktrees,
    })
}

/// The phone may only pick a worktree collied itself listed for the agent's repository.
/// The listing comes from metadata the agent can write, so the pick is re-checked like the
/// agent's own checkout and must share its Git common directory.
async fn worktree(
    checkout: &Cwd,
    worktrees: &[GitWorktree],
    requested: &Cwd,
    roots: &[PathBuf],
) -> Result<Option<Cwd>, Fail> {
    if !worktrees.iter().any(|w| &w.root == requested) {
        return Ok(None);
    }
    let common = metadata(checkout, "--git-common-dir", roots).await?;
    in_repository(&common, requested, roots).await
}

/// `requested` is a checkout's top level whose Git common directory is `common`.
async fn in_repository(
    common: &Cwd,
    requested: &Cwd,
    roots: &[PathBuf],
) -> Result<Option<Cwd>, Fail> {
    let Some(root) = root(requested, roots)
        .await?
        .filter(|root| root == requested)
    else {
        return Ok(None);
    };
    Ok((&metadata(&root, "--git-common-dir", roots).await? == common).then_some(root))
}

/// Fixed refs only: the phone never names the base. Its name, and its commit.
async fn base(root: &Cwd) -> Result<Option<(String, String)>, Fail> {
    for candidate in [
        "refs/remotes/origin/HEAD",
        "refs/heads/main",
        "refs/heads/master",
    ] {
        let (out, ok, truncated) = git(
            root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{candidate}^{{commit}}"),
            ],
            4096,
        )
        .await?;
        let sha = String::from_utf8_lossy(&out)
            .trim_end_matches('\n')
            .to_owned();
        if !ok || truncated || sha.is_empty() || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let (name, ok, truncated) =
            git(root, &["rev-parse", "--abbrev-ref", candidate], 4096).await?;
        let name = std::str::from_utf8(&name)
            .ok()
            .map(|n| n.trim_end_matches('\n'))
            .filter(|n| ok && !truncated && !n.is_empty())
            .unwrap_or(candidate);
        return Ok(Some((name.into(), sha)));
    }
    Ok(None)
}

fn digest(parts: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    hex(&hash.finalize()[..8])
}

fn diff_args(section: GitSection, range: &str) -> Vec<&str> {
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--find-renames",
        "--ignore-submodules=none",
    ];
    match section {
        GitSection::Committed => args.push(range),
        GitSection::Staged => args.push("--cached"),
        _ => {}
    }
    args
}

fn path(value: &[u8]) -> Result<String, Fail> {
    let value =
        std::str::from_utf8(value).map_err(|_| error("A changed file has a non-UTF-8 path"))?;
    if value.is_empty()
        || value.len() > 4096
        || !Path::new(value)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(error("Invalid Git file path"));
    }
    Ok(value.into())
}

fn tracked(out: &[u8], section: GitSection) -> Result<Vec<GitFile>, Fail> {
    let mut parts = out
        .split_inclusive(|b| *b == 0)
        .filter_map(|s| s.strip_suffix(&[0]))
        .peekable();
    let mut files = Vec::new();
    while parts.peek().is_some_and(|p| p.starts_with(b":")) {
        let header = parts.next().unwrap();
        let Some(name) = parts.next() else { break };
        let status = header.split(|b| *b == b' ').next_back().unwrap_or_default();
        let renamed = matches!(status.first(), Some(b'R' | b'C'));
        let (old_path, name) = if renamed {
            let Some(next) = parts.next() else { break };
            (Some(path(name)?), next)
        } else {
            (None, name)
        };
        let path = path(name)?;
        files.push(GitFile {
            hash: Some(digest(&[
                header,
                old_path.as_deref().unwrap_or_default().as_bytes(),
                path.as_bytes(),
            ])),
            path,
            old_path,
            section,
            status: String::from_utf8_lossy(status).into_owned(),
            additions: None,
            deletions: None,
        });
    }
    while let Some(stat) = parts.next() {
        let mut columns = stat.splitn(3, |b| *b == b'\t');
        let (Some(add), Some(del), Some(mut name)) =
            (columns.next(), columns.next(), columns.next())
        else {
            break;
        };
        if name.is_empty() {
            if parts.next().is_none() {
                break;
            }
            let Some(next) = parts.next() else { break };
            name = next;
        }
        if let Some(file) = files.iter_mut().find(|f| f.path.as_bytes() == name) {
            file.additions = std::str::from_utf8(add).ok().and_then(|s| s.parse().ok());
            file.deletions = std::str::from_utf8(del).ok().and_then(|s| s.parse().ok());
        }
    }
    Ok(files)
}

fn untracked_content(root: &Cwd, name: &str) -> Result<Vec<u8>, Fail> {
    path(name.as_bytes())?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut dir = rustix::fs::open(root.as_str(), flags, Mode::empty())
        .map_err(|_| error("Checkout changed; refresh"))?;
    let mut parts = Path::new(name).components().peekable();
    while let Some(Component::Normal(part)) = parts.next() {
        if parts.peek().is_some() {
            dir = rustix::fs::openat(&dir, part, flags, Mode::empty())
                .map_err(|_| error("File directory changed; refresh"))?;
        } else {
            let fd = match rustix::fs::openat(
                &dir,
                part,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::LOOP) => {
                    return rustix::fs::readlinkat(&dir, part, Vec::new())
                        .map(|s| s.as_bytes().to_vec())
                        .map_err(|_| error("Could not read symlink"));
                }
                Err(_) => return Err(error("File changed or cannot be read; refresh")),
            };
            let file = std::fs::File::from(fd);
            if !file
                .metadata()
                .map_err(|_| error("Could not inspect file"))?
                .is_file()
            {
                return Err(error("This entry is not a regular file"));
            }
            let mut bytes = Vec::new();
            file.take(FILE_LIMIT + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| error("Could not read file"))?;
            if bytes.len() as u64 > FILE_LIMIT {
                return Err(error("File is too large to preview (1 MiB limit)"));
            }
            return Ok(bytes);
        }
    }
    Err(error("Invalid file path"))
}

fn text_content(bytes: &[u8]) -> Option<&str> {
    if bytes.contains(&0) {
        None
    } else {
        std::str::from_utf8(bytes).ok()
    }
}

async fn head(root: &Cwd) -> Result<Option<String>, Fail> {
    let (branch, ok, _) = git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"], 4096).await?;
    Ok(ok.then(|| {
        String::from_utf8_lossy(&branch)
            .trim_end_matches('\n')
            .to_owned()
    }))
}

pub async fn branch(cwd: &Cwd, roots: &[PathBuf]) -> Result<Option<(Cwd, String)>, Fail> {
    let Some(root) = root(cwd, roots).await? else {
        return Ok(None);
    };
    Ok(head(&root).await?.map(|branch| (root, branch)))
}

/// No uncommitted or untracked file, in the checkout or a submodule, and no submodule off
/// its recorded commit, whatever the repository's config says. A file whose changes `git
/// status` would not look at (assume-unchanged, as `core.ignoreStat` sets, or
/// skip-worktree), in the checkout or a submodule, also counts as a change.
pub async fn clean(root: &Cwd) -> Result<bool, Fail> {
    let (out, ok, truncated) = git(
        root,
        &[
            "status",
            "--porcelain",
            "-z",
            "--untracked-files=normal",
            "--ignore-submodules=none",
        ],
        4096,
    )
    .await?;
    if !ok || truncated || !out.is_empty() {
        return Ok(false);
    }
    let (out, ok, truncated) = git(
        root,
        &["ls-files", "-v", "-z", "--recurse-submodules"],
        16 * 1024 * 1024,
    )
    .await?;
    Ok(ok
        && !truncated
        && out
            .split(|b| *b == 0)
            .filter_map(|entry| entry.first())
            .all(|tag| !tag.is_ascii_lowercase() && *tag != b'S'))
}

/// `HEAD` holds no commit that `commit`'s history does not.
pub async fn contains(root: &Cwd, commit: &str) -> Result<bool, Fail> {
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(false);
    }
    let (_, ok, _) = git(root, &["merge-base", "--is-ancestor", "HEAD", commit], 64).await?;
    Ok(ok)
}

pub async fn changes(
    cwd: &Cwd,
    roots: &[PathBuf],
    requested: Option<&Cwd>,
    scope: GitScope,
) -> Result<GitChanges, Fail> {
    let Some(checkout) = root(cwd, roots).await? else {
        return Ok(GitChanges {
            root: None,
            branch: None,
            base: None,
            files: Vec::new(),
            truncated: false,
            worktrees: Vec::new(),
            pull_requests: Vec::new(),
        });
    };
    let worktrees = worktrees(&checkout, roots).await?;
    if scope == GitScope::Worktrees {
        return overview(checkout, worktrees, roots).await;
    }
    let root = match requested {
        Some(requested) => worktree(&checkout, &worktrees, requested, roots)
            .await?
            .unwrap_or(checkout),
        None => checkout,
    };
    let branch = head(&root).await?;
    let mut changes = GitChanges {
        root: Some(root.clone()),
        branch,
        base: None,
        files: Vec::new(),
        truncated: false,
        worktrees,
        pull_requests: Vec::new(),
    };
    if scope == GitScope::Branch {
        if let Some((name, sha)) = base(&root).await? {
            let range = format!("{sha}...HEAD");
            let mut args = diff_args(GitSection::Committed, &range);
            args.extend(["--raw", "--no-abbrev", "--numstat", "-z", "--"]);
            let (out, ok, truncated) = git(&root, &args, OUTPUT_LIMIT).await?;
            if !ok {
                return Err(error(&format!(
                    "Could not compare this branch with {name}; it may share no history with it"
                )));
            }
            changes.base = Some(name);
            changes.truncated |= truncated;
            changes.files.extend(tracked(&out, GitSection::Committed)?);
        }
    } else {
        for section in [GitSection::Staged, GitSection::Unstaged] {
            let mut args = diff_args(section, "");
            args.extend(["--raw", "--no-abbrev", "--numstat", "-z", "--"]);
            let (out, ok, truncated) = git(&root, &args, OUTPUT_LIMIT).await?;
            if !ok {
                return Err(error("Could not list Git changes"));
            }
            changes.truncated |= truncated;
            changes.files.extend(tracked(&out, section)?);
        }
        let (out, ok, truncated) = git(
            &root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
            OUTPUT_LIMIT,
        )
        .await?;
        if !ok {
            return Err(error("Could not list untracked files"));
        }
        changes.truncated |= truncated;
        for name in out
            .split_inclusive(|b| *b == 0)
            .filter_map(|s| s.strip_suffix(&[0]))
        {
            if changes.files.len() >= MAX_FILES {
                changes.truncated = true;
                break;
            }
            let name = path(name)?;
            let contents = untracked_content(&root, &name).ok();
            let additions = contents
                .as_deref()
                .and_then(text_content)
                .map(|s| s.lines().count() as u32);
            changes.files.push(GitFile {
                hash: contents
                    .as_deref()
                    .map(|c| digest(&[b"?", name.as_bytes(), c])),
                path: name,
                old_path: None,
                section: GitSection::Untracked,
                status: "?".into(),
                additions,
                deletions: additions.map(|_| 0),
            });
        }
    }
    if changes.files.len() > MAX_FILES {
        changes.files.truncate(MAX_FILES);
        changes.truncated = true;
    }
    for file in changes
        .files
        .iter_mut()
        .filter(|f| f.section == GitSection::Unstaged && !f.status.starts_with('D'))
    {
        file.hash = match (&file.hash, untracked_content(&root, &file.path)) {
            (Some(hash), Ok(contents)) => Some(digest(&[hash.as_bytes(), &contents])),
            _ => None,
        };
    }
    limited(changes)
}

fn limited(mut changes: GitChanges) -> Result<GitChanges, Fail> {
    while serde_json::to_vec(&changes)
        .map_err(|_| error("Could not encode Git changes"))?
        .len()
        > RESPONSE_LIMIT
    {
        if changes.files.pop().is_none() && changes.pull_requests.pop().is_none() {
            return Err(error("Git changes are too large to list"));
        }
        changes.truncated = true;
    }
    Ok(changes)
}

/// Each worktree with its last commit, how far it is from the default branch and its
/// pull request, plus the user's open pull requests whose branch has no worktree.
async fn overview(
    checkout: Cwd,
    worktrees: Vec<GitWorktree>,
    roots: &[PathBuf],
) -> Result<GitChanges, Fail> {
    let common = metadata(&checkout, "--git-common-dir", roots).await.ok();
    let summaries = futures_util::stream::iter(worktrees)
        .map(|mut w| {
            let (checkout, common) = (&checkout, &common);
            async move {
                let same = match common {
                    Some(common) => in_repository(common, &w.root, roots).await,
                    None => Ok(None),
                };
                if &w.root == checkout || matches!(same, Ok(Some(r)) if r == w.root) {
                    let _ = summarize(&mut w).await;
                }
                w
            }
        })
        .buffered(8)
        .collect::<Vec<_>>();
    let mine = async {
        match crate::drive::gh() {
            Some(gh) => crate::pulls::mine(&gh, &checkout).await,
            None => Vec::new(),
        }
    };
    let (mut worktrees, mine) = tokio::join!(summaries, mine);
    for w in &mut worktrees {
        w.pull_request = mine
            .iter()
            .find(|(_, branch, _)| w.branch.as_ref() == Some(branch))
            .map(|(pr, ..)| pr.clone());
    }
    let pull_requests = mine
        .into_iter()
        .filter(|(pr, branch, _)| {
            matches!(pr.state, PullState::Open | PullState::Draft)
                && !worktrees.iter().any(|w| w.branch.as_ref() == Some(branch))
        })
        .map(|(pull_request, branch, title)| GitPullRequest {
            pull_request,
            branch,
            title,
        })
        .collect();
    limited(GitChanges {
        branch: head(&checkout).await?,
        root: Some(checkout),
        base: None,
        files: Vec::new(),
        truncated: false,
        worktrees,
        pull_requests,
    })
}

async fn summarize(w: &mut GitWorktree) -> Result<(), Fail> {
    let root = &w.root;
    if let Some((time, subject)) = line(
        root,
        &[
            "log",
            "-1",
            "--no-show-signature",
            "--format=%ct %s",
            "HEAD",
        ],
    )
    .await?
    .as_deref()
    .and_then(|l| l.split_once(' '))
    {
        w.committed_at_ms = time.parse::<u64>().ok().map(|t| t.saturating_mul(1000));
        w.subject = Some(crate::pulls::title(subject));
    }
    let (out, ok, truncated) = git(
        root,
        &[
            "status",
            "--porcelain",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
        OUTPUT_LIMIT,
    )
    .await?;
    if ok && !truncated {
        w.uncommitted = Some(status_count(&out));
    }
    let Some((_, sha)) = base(root).await? else {
        return Ok(());
    };
    w.ahead = line(root, &["rev-list", "--count", &format!("{sha}..HEAD")])
        .await?
        .and_then(|n| n.parse().ok());
    let (out, ok, truncated) = git(
        root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--ignore-submodules=none",
            "--numstat",
            "-z",
            "--merge-base",
            &sha,
            "--",
        ],
        OUTPUT_LIMIT,
    )
    .await?;
    if ok && !truncated {
        let (additions, deletions) = numstat_totals(&out);
        (w.additions, w.deletions) = (Some(additions), Some(deletions));
    }
    Ok(())
}

/// `git status --porcelain -z` entries; a rename or copy carries its source as one more.
fn status_count(out: &[u8]) -> u32 {
    let mut parts = out.split(|b| *b == 0).filter(|p| !p.is_empty());
    let mut count = 0u32;
    while let Some(entry) = parts.next() {
        if entry
            .get(..2)
            .is_some_and(|xy| xy.contains(&b'R') || xy.contains(&b'C'))
        {
            parts.next();
        }
        count = count.saturating_add(1);
    }
    count
}

/// `git diff --numstat -z`: a rename's paths follow as two more fields; a binary file
/// counts no lines.
fn numstat_totals(out: &[u8]) -> (u32, u32) {
    let mut parts = out.split(|b| *b == 0);
    let (mut additions, mut deletions) = (0u32, 0u32);
    while let Some(record) = parts.next() {
        let mut columns = record.splitn(3, |b| *b == b'\t');
        let (Some(add), Some(del), Some(name)) = (columns.next(), columns.next(), columns.next())
        else {
            continue;
        };
        if name.is_empty() {
            parts.next();
            parts.next();
        }
        let count = |n: &[u8]| {
            std::str::from_utf8(n)
                .ok()
                .and_then(|n| n.parse::<u32>().ok())
        };
        additions = additions.saturating_add(count(add).unwrap_or(0));
        deletions = deletions.saturating_add(count(del).unwrap_or(0));
    }
    (additions, deletions)
}

/// One file of GitHub's pull request file list. All of it is untrusted.
#[derive(Deserialize)]
struct PullFile {
    filename: String,
    previous_filename: Option<String>,
    status: String,
    #[serde(default)]
    additions: u64,
    #[serde(default)]
    deletions: u64,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    patch: Option<String>,
}

const PULL_FIELDS: &str = "map({filename, previous_filename, status, additions, deletions, sha})";
const PULL_PATCH: &str = "{count: length, file: ([.[] | select(.filename == $ENV.COLLIE_PATH) | {filename, previous_filename, status, patch}] | first)}";

fn pull_status(status: &str) -> &'static str {
    match status {
        "added" => "A",
        "removed" => "D",
        "renamed" => "R",
        "copied" => "C",
        "changed" => "T",
        _ => "M",
    }
}

/// A file whose path or previous path is not a plain relative path is left out.
fn pull_file(number: u64, f: &PullFile) -> Option<GitFile> {
    let path = path(f.filename.as_bytes()).ok()?;
    let old_path = match &f.previous_filename {
        Some(old) => Some(self::path(old.as_bytes()).ok()?),
        None => None,
    };
    let status = pull_status(&f.status);
    Some(GitFile {
        hash: Some(digest(&[
            &number.to_le_bytes(),
            status.as_bytes(),
            old_path.as_deref().unwrap_or_default().as_bytes(),
            path.as_bytes(),
            f.sha.as_deref().unwrap_or_default().as_bytes(),
        ])),
        path,
        old_path,
        section: GitSection::PullRequest,
        status: status.into(),
        additions: u32::try_from(f.additions).ok(),
        deletions: u32::try_from(f.deletions).ok(),
    })
}

fn pull_page(json: &[u8]) -> Result<Vec<PullFile>, Fail> {
    serde_json::from_slice(json).map_err(|_| error("GitHub's file list could not be read"))
}

/// `path`, when set, reaches gh's `--jq` program as `$ENV.COLLIE_PATH`, never as an argument.
async fn gh(checkout: &Cwd, args: &[&str], path: Option<&str>) -> Result<Vec<u8>, Fail> {
    let gh = crate::drive::gh().ok_or_else(|| error("gh is not installed on the machine"))?;
    let mut command = crate::pulls::gh_command(&gh);
    if let Some(path) = path {
        command.env("COLLIE_PATH", path);
    }
    command
        .args(args)
        .current_dir(checkout.as_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = tokio::time::timeout(crate::pulls::GH_TIMEOUT, command.output())
        .await
        .map_err(|_| error("gh took too long; try again"))?
        .map_err(|_| error("gh is unavailable"))?;
    if output.status.code() == Some(4) {
        return Err(error(
            "gh is not signed in; run `gh auth login` on the machine",
        ));
    }
    if !output.status.success() {
        return Err(error(&format!(
            "gh could not read the pull request: {}",
            crate::pulls::first_line(&output.stderr)
        )));
    }
    if output.stdout.len() > OUTPUT_LIMIT {
        return Err(error("The pull request is too large to preview"));
    }
    Ok(output.stdout)
}

fn files_url(number: u64, page: usize) -> String {
    format!("repos/{{owner}}/{{repo}}/pulls/{number}/files?per_page={PULL_PAGE}&page={page}")
}

/// The files of pull request `number` of the repository gh finds for the agent's checkout,
/// as GitHub lists them: its pushed head against its base, whether or not a worktree
/// still has the branch.
pub async fn pull_changes(cwd: &Cwd, roots: &[PathBuf], number: u64) -> Result<GitChanges, Fail> {
    let checkout = root(cwd, roots)
        .await?
        .ok_or_else(|| error("This folder is no longer a Git checkout"))?;
    deadline(pull_list(checkout, number)).await
}

/// All of a pull request read's gh calls share one `GH_TIMEOUT`, within the phone's wait.
async fn deadline<T>(read: impl Future<Output = Result<T, Fail>>) -> Result<T, Fail> {
    tokio::time::timeout(crate::pulls::GH_TIMEOUT, read)
        .await
        .map_err(|_| error("GitHub took too long; try again"))?
}

async fn pull_list(checkout: Cwd, number: u64) -> Result<GitChanges, Fail> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct View {
        number: u64,
        base_ref_name: String,
        head_ref_name: String,
        changed_files: u64,
    }
    let number_arg = number.to_string();
    let view = gh(
        &checkout,
        &[
            "pr",
            "view",
            &number_arg,
            "--json",
            "number,baseRefName,headRefName,changedFiles",
        ],
        None,
    )
    .await?;
    let view: View = serde_json::from_slice(&view)
        .ok()
        .filter(|v: &View| v.number == number)
        .ok_or_else(|| error("GitHub's answer for the pull request could not be read"))?;
    let mut changes = GitChanges {
        root: Some(checkout.clone()),
        branch: Some(crate::pulls::title(&view.head_ref_name)),
        base: Some(crate::pulls::title(&view.base_ref_name)),
        files: Vec::new(),
        truncated: false,
        worktrees: Vec::new(),
        pull_requests: Vec::new(),
    };
    let mut listed = 0;
    for page in 1..=MAX_FILES.div_ceil(PULL_PAGE) {
        let out = gh(
            &checkout,
            &["api", &files_url(number, page), "--jq", PULL_FIELDS],
            None,
        )
        .await?;
        let files = pull_page(&out)?;
        listed += files.len();
        changes
            .files
            .extend(files.iter().filter_map(|f| pull_file(number, f)));
        if files.len() < PULL_PAGE {
            break;
        }
    }
    changes.truncated = (listed as u64) < view.changed_files || changes.files.len() < listed;
    changes.files.truncate(MAX_FILES);
    limited(changes)
}

/// GitHub's own patch for one file, one page at a time.
async fn pull_patch(checkout: &Cwd, number: u64, name: &str) -> Result<String, Fail> {
    #[derive(Deserialize)]
    struct Page {
        count: usize,
        file: Option<PullFile>,
    }
    for page in 1..=MAX_FILES.div_ceil(PULL_PAGE) {
        let out = gh(
            checkout,
            &["api", &files_url(number, page), "--jq", PULL_PATCH],
            Some(name),
        )
        .await?;
        let page: Page = serde_json::from_slice(&out)
            .map_err(|_| error("GitHub's file list could not be read"))?;
        if let Some(file) = page.file {
            if file.filename != name {
                return Err(error(
                    "The pull request changed while reading its diff; refresh",
                ));
            }
            return Ok(pull_text(file.patch));
        }
        if page.count < PULL_PAGE {
            break;
        }
    }
    Err(error("File is no longer in the pull request; refresh"))
}

fn pull_text(patch: Option<String>) -> String {
    match patch {
        Some(patch) if !patch.is_empty() => {
            if patch.ends_with('\n') {
                patch
            } else {
                patch + "\n"
            }
        }
        _ => "Binary file, or a change GitHub shows no text diff for.\n".into(),
    }
}

pub async fn diff(cwd: &Cwd, roots: &[PathBuf], p: &AgentDiffParams) -> Result<GitDiff, Fail> {
    path(p.path.as_bytes())?;
    let Some(checkout) = root(cwd, roots).await? else {
        return Err(error("This folder is no longer a Git checkout"));
    };
    if let Some(number) = p.pull_request {
        return bounded(GitDiff {
            patch: deadline(pull_patch(&checkout, number, &p.path)).await?,
            truncated: false,
        });
    }
    let root = if checkout == p.root {
        checkout
    } else {
        worktree(
            &checkout,
            &worktrees(&checkout, roots).await?,
            &p.root,
            roots,
        )
        .await?
        .ok_or_else(|| error("Agent checkout changed; reopen Git changes"))?
    };
    let (bytes, truncated) = if p.section == GitSection::Untracked {
        let (out, ok, truncated) = git(
            &root,
            &[
                "ls-files",
                "--others",
                "--exclude-standard",
                "-z",
                "--",
                &p.path,
            ],
            8192,
        )
        .await?;
        if !ok || truncated || !out.split(|b| *b == 0).any(|s| s == p.path.as_bytes()) {
            return Err(error("File is no longer untracked; refresh"));
        }
        let bytes = untracked_content(&root, &p.path)?;
        let patch = match text_content(&bytes) {
            None => "Binary file. No text diff available.\n".into(),
            Some("") => "New empty file.\n".into(),
            Some(text) => {
                let mut patch = format!(
                    "--- /dev/null\n+++ {}\n@@ -0,0 +1,{} @@\n",
                    serde_json::to_string(&format!("b/{}", p.path))
                        .map_err(|_| error("Invalid file path"))?,
                    text.lines().count()
                );
                for line in text.split_inclusive('\n') {
                    patch.push('+');
                    patch.push_str(line);
                }
                if !text.ends_with('\n') {
                    patch.push_str("\n\\ No newline at end of file\n");
                }
                patch
            }
        };
        (patch.into_bytes(), false)
    } else {
        let range = if p.section == GitSection::Committed {
            base(&root)
                .await?
                .map(|(_, sha)| format!("{sha}...HEAD"))
                .ok_or_else(|| error("File no longer has changes in this section; refresh"))?
        } else {
            String::new()
        };
        let mut args = diff_args(p.section, &range);
        args.extend(["--raw", "-z", "--"]);
        let (out, ok, truncated) = git(&root, &args, OUTPUT_LIMIT).await?;
        if !ok || truncated {
            return Err(error("Could not resolve changed file; refresh"));
        }
        let file = tracked(&out, p.section)?
            .into_iter()
            .find(|f| f.path == p.path)
            .ok_or_else(|| error("File no longer has changes in this section; refresh"))?;
        let mut args = diff_args(p.section, &range);
        args.extend([
            "--raw",
            "-z",
            "--patch",
            "--unified=3",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--",
            &p.path,
        ]);
        if let Some(old) = &file.old_path {
            args.push(old);
        }
        let (out, ok, truncated) = git(&root, &args, 128 * 1024).await?;
        if !ok {
            return Err(error("Could not read Git diff"));
        }
        let boundary = out
            .windows(2)
            .position(|s| s == [0, 0])
            .ok_or_else(|| error("File changed while reading its diff; refresh"))?;
        let files = tracked(&out[..boundary + 1], p.section)?;
        let index = files
            .iter()
            .position(|f| f.path == p.path)
            .ok_or_else(|| error("File changed while reading its diff; refresh"))?;
        let patches = &out[boundary + 2..];
        let marker = b"\ndiff --git ";
        let mut starts = vec![0];
        starts.extend(
            patches
                .windows(marker.len())
                .enumerate()
                .filter_map(|(i, s)| (s == marker).then_some(i + 1)),
        );
        let start = *starts
            .get(index)
            .ok_or_else(|| error("Diff is too large to preview"))?;
        let end = starts.get(index + 1).copied().unwrap_or(patches.len());
        (
            patches[start..end].to_vec(),
            truncated && index + 1 == starts.len(),
        )
    };
    bounded(GitDiff {
        patch: String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
    })
}

fn bounded(mut diff: GitDiff) -> Result<GitDiff, Fail> {
    while serde_json::to_vec(&diff)
        .map_err(|_| error("Could not encode Git diff"))?
        .len()
        > RESPONSE_LIMIT
    {
        let mut end = diff.patch.len() * 3 / 4;
        while !diff.patch.is_char_boundary(end) {
            end -= 1;
        }
        diff.patch.truncate(end);
        diff.truncated = true;
    }
    Ok(diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(root: &Path, args: &[&str]) {
        assert!(
            std::process::Command::new("git")
                .current_dir(root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success(),
            "{args:?}"
        );
    }

    #[tokio::test]
    async fn worktree_list_reads_the_main_checkout_first() {
        let (_dir, cwd, roots) = repo();
        let root = &roots[0];
        run(root, &["commit", "-q", "--allow-empty", "-m", "start"]);
        let feat = root.join("wt/feat branch");
        let detached = root.join("wt/detached");
        run(
            root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat",
                feat.to_str().unwrap(),
            ],
        );
        run(
            root,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                detached.to_str().unwrap(),
            ],
        );
        let list = worktree_list(&cwd).await.unwrap();
        assert_eq!(list.source.source_checkout_path, cwd.as_str());
        let seen: Vec<(String, Option<String>, bool, bool)> = (list.worktrees.into_iter())
            .map(|w| (w.path, w.branch, w.is_linked_worktree, w.is_bare))
            .collect();
        let path = |p: &Path| p.to_str().unwrap().to_owned();
        assert_eq!(
            seen,
            [
                (path(root), Some("main".to_owned()), false, false),
                (path(&detached), None, true, false),
                (path(&feat), Some("feat".to_owned()), true, false),
            ]
        );

        let bare = root.join("bare.git");
        run(
            root,
            &["clone", "-q", "--bare", ".", bare.to_str().unwrap()],
        );
        let linked = root.join("from-bare");
        run(
            &bare,
            &["worktree", "add", "-q", linked.to_str().unwrap(), "main"],
        );
        let list = worktree_list(&Cwd::new(linked.to_str().unwrap()).unwrap())
            .await
            .unwrap();
        assert_eq!(list.source.source_checkout_path, path(&bare));
        assert!(list.worktrees[0].is_bare && !list.worktrees[0].is_linked_worktree);
        assert!(list.worktrees[1].is_linked_worktree && !list.worktrees[1].is_bare);

        let plain = tempfile::tempdir().unwrap();
        let plain = Cwd::new(plain.path().canonicalize().unwrap().to_str().unwrap()).unwrap();
        assert!(worktree_list(&plain).await.is_err());
    }

    fn repo() -> (tempfile::TempDir, Cwd, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        run(&root, &["init", "-b", "main"]);
        run(&root, &["config", "user.name", "Test"]);
        run(&root, &["config", "user.email", "test@example.com"]);
        let cwd = Cwd::new(root.to_str().unwrap()).unwrap();
        (dir, cwd, vec![root])
    }

    async fn patch(cwd: &Cwd, roots: &[PathBuf], name: &str, section: GitSection) -> GitDiff {
        diff(
            cwd,
            roots,
            &AgentDiffParams {
                terminal_id: protocol::TerminalId::new("term_test").unwrap(),
                root: cwd.clone(),
                path: name.into(),
                section,
                pull_request: None,
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn clean_and_contained_in_the_merged_head() {
        let (_dir, cwd, _) = repo();
        let root = Path::new(cwd.as_str());
        let rev = || {
            let out = std::process::Command::new("git")
                .current_dir(root)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        };
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "a"]);
        let first = rev();
        run(root, &["commit", "--allow-empty", "-m", "b"]);
        let merged = rev();
        std::fs::create_dir(root.join("target")).unwrap();
        std::fs::write(root.join("target/out"), "x").unwrap();
        assert!(clean(&cwd).await.unwrap());
        assert!(contains(&cwd, &merged).await.unwrap());

        run(root, &["config", "status.showUntrackedFiles", "no"]);
        std::fs::write(root.join("new.txt"), "x").unwrap();
        assert!(!clean(&cwd).await.unwrap());
        std::fs::remove_file(root.join("new.txt")).unwrap();
        std::fs::write(root.join("a.txt"), "changed\n").unwrap();
        assert!(!clean(&cwd).await.unwrap());
        run(root, &["add", "a.txt"]);
        assert!(!clean(&cwd).await.unwrap());

        run(root, &["commit", "-m", "not in the pull request"]);
        assert!(clean(&cwd).await.unwrap());
        assert!(!contains(&cwd, &merged).await.unwrap());
        run(root, &["reset", "-q", "--hard", &first]);
        assert!(contains(&cwd, &merged).await.unwrap());
        for bad in [&"0".repeat(40), "--all", "HEAD", &merged[..12]] {
            assert!(!contains(&cwd, bad).await.unwrap(), "{bad}");
        }

        for (flag, set, unset) in [
            (
                "--assume-unchanged",
                "update-index",
                "--no-assume-unchanged",
            ),
            ("--skip-worktree", "update-index", "--no-skip-worktree"),
        ] {
            run(root, &[set, flag, "a.txt"]);
            std::fs::write(root.join("a.txt"), "hidden\n").unwrap();
            assert!(!clean(&cwd).await.unwrap(), "{flag}");
            run(root, &[set, unset, "a.txt"]);
            run(root, &["checkout", "--", "a.txt"]);
            assert!(clean(&cwd).await.unwrap(), "{flag}");
        }
        run(root, &["config", "core.ignoreStat", "true"]);
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        run(root, &["add", "b.txt"]);
        run(root, &["commit", "-qm", "b"]);
        std::fs::write(root.join("b.txt"), "hidden\n").unwrap();
        assert!(!clean(&cwd).await.unwrap());
        run(root, &["config", "core.ignoreStat", "false"]);
        run(root, &["update-index", "--no-assume-unchanged", "b.txt"]);
        run(root, &["checkout", "--", "b.txt"]);
        assert!(clean(&cwd).await.unwrap());

        let (_sub_dir, sub, _) = repo();
        std::fs::write(Path::new(sub.as_str()).join("s.txt"), "s\n").unwrap();
        run(Path::new(sub.as_str()), &["add", "s.txt"]);
        run(Path::new(sub.as_str()), &["commit", "-qm", "sub"]);
        run(
            root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                sub.as_str(),
                "lib",
            ],
        );
        run(
            root,
            &["config", "-f", ".gitmodules", "submodule.lib.ignore", "all"],
        );
        run(root, &["commit", "-qam", "submodule"]);
        assert!(clean(&cwd).await.unwrap());
        std::fs::write(root.join("lib/new.txt"), "x").unwrap();
        assert!(!clean(&cwd).await.unwrap());
        std::fs::remove_file(root.join("lib/new.txt")).unwrap();
        let lib = root.join("lib");
        run(&lib, &["update-index", "--assume-unchanged", "s.txt"]);
        std::fs::write(lib.join("s.txt"), "hidden\n").unwrap();
        assert!(!clean(&cwd).await.unwrap());
        run(&lib, &["update-index", "--no-assume-unchanged", "s.txt"]);
        run(&lib, &["checkout", "--", "s.txt"]);
        assert!(clean(&cwd).await.unwrap());
        run(
            &root.join("lib"),
            &["commit", "--allow-empty", "-m", "not recorded"],
        );
        assert!(!clean(&cwd).await.unwrap());
    }

    #[tokio::test]
    async fn staged_unstaged_untracked_renamed_deleted_binary_and_empty() {
        let (_dir, cwd, roots) = repo();
        let root = Path::new(cwd.as_str());
        for (name, value) in [
            ("both.txt", "base\n"),
            ("delete.txt", "deleted\n"),
            ("old.txt", "renamed\n"),
        ] {
            std::fs::write(root.join(name), value).unwrap();
        }
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "base"]);
        std::fs::write(root.join("both.txt"), "staged\n").unwrap();
        run(root, &["add", "both.txt"]);
        std::fs::write(root.join("both.txt"), "unstaged\n").unwrap();
        std::fs::remove_file(root.join("delete.txt")).unwrap();
        run(root, &["mv", "old.txt", "new\tname.txt"]);
        std::fs::write(root.join("new file.txt"), "one\ntwo").unwrap();
        std::fs::write(root.join("binary"), [0, 1, 2]).unwrap();
        std::fs::write(root.join("empty"), "").unwrap();
        let list = changes(&cwd, &roots, None, GitScope::Uncommitted)
            .await
            .unwrap();
        assert_eq!(list.branch.as_deref(), Some("main"));
        assert_eq!(list.files.len(), 7);
        for section in [GitSection::Staged, GitSection::Unstaged] {
            let file = list
                .files
                .iter()
                .find(|f| f.path == "both.txt" && f.section == section)
                .unwrap();
            assert_eq!((file.additions, file.deletions), (Some(1), Some(1)));
        }
        let renamed = list
            .files
            .iter()
            .find(|f| f.path == "new\tname.txt")
            .unwrap();
        assert_eq!(renamed.old_path.as_deref(), Some("old.txt"));
        assert!(
            patch(&cwd, &roots, "new\tname.txt", GitSection::Staged)
                .await
                .patch
                .contains("rename from")
        );
        let staged = patch(&cwd, &roots, "both.txt", GitSection::Staged)
            .await
            .patch;
        assert!(staged.contains("-base\n+staged\n"));
        let unstaged = patch(&cwd, &roots, "both.txt", GitSection::Unstaged)
            .await
            .patch;
        assert!(unstaged.contains("-staged\n+unstaged\n"));
        assert!(
            patch(&cwd, &roots, "delete.txt", GitSection::Unstaged)
                .await
                .patch
                .contains("+++ /dev/null")
        );
        let new = patch(&cwd, &roots, "new file.txt", GitSection::Untracked)
            .await
            .patch;
        assert!(new.contains("@@ -0,0 +1,2 @@\n+one\n+two\n\\ No newline at end of file"));
        assert!(
            patch(&cwd, &roots, "binary", GitSection::Untracked)
                .await
                .patch
                .contains("Binary file")
        );
        assert!(
            patch(&cwd, &roots, "empty", GitSection::Untracked)
                .await
                .patch
                .contains("New empty file")
        );
        run(root, &["add", "binary", "empty", "new file.txt"]);
        assert!(
            patch(&cwd, &roots, "binary", GitSection::Staged)
                .await
                .patch
                .contains("Binary files")
        );
        assert!(
            patch(&cwd, &roots, "empty", GitSection::Staged)
                .await
                .patch
                .contains("new file mode")
        );
    }

    #[tokio::test]
    async fn unborn_clean_detached_non_git_and_worktree_checkout() {
        let (_dir, cwd, roots) = repo();
        assert!(
            changes(&cwd, &roots, None, GitScope::Uncommitted)
                .await
                .unwrap()
                .files
                .is_empty()
        );
        let root = Path::new(cwd.as_str());
        std::fs::write(root.join("first"), "new\n").unwrap();
        run(root, &["add", "."]);
        assert_eq!(
            changes(&cwd, &roots, None, GitScope::Uncommitted)
                .await
                .unwrap()
                .files[0]
                .additions,
            Some(1)
        );
        run(root, &["commit", "-m", "first"]);
        run(root, &["worktree", "add", "-b", "feature", "checkout"]);
        let checkout = Cwd::new(root.join("checkout").to_str().unwrap()).unwrap();
        std::fs::write(Path::new(checkout.as_str()).join("first"), "worktree\n").unwrap();
        let list = changes(&checkout, &roots, None, GitScope::Uncommitted)
            .await
            .unwrap();
        assert_eq!(list.root, Some(checkout.clone()));
        assert_eq!(list.branch.as_deref(), Some("feature"));
        let sub = Path::new(checkout.as_str()).join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert_eq!(
            branch(&Cwd::new(sub.to_str().unwrap()).unwrap(), &roots)
                .await
                .unwrap(),
            Some((checkout.clone(), "feature".to_owned()))
        );
        assert!(
            patch(&checkout, &roots, "first", GitSection::Unstaged)
                .await
                .patch
                .contains("+worktree")
        );
        run(Path::new(checkout.as_str()), &["checkout", "--detach"]);
        assert_eq!(
            changes(&checkout, &roots, None, GitScope::Uncommitted)
                .await
                .unwrap()
                .branch,
            None
        );
        assert_eq!(branch(&checkout, &roots).await.unwrap(), None);
        let outside = tempfile::tempdir().unwrap();
        let outside = outside.path().canonicalize().unwrap();
        let outside_cwd = Cwd::new(outside.to_str().unwrap()).unwrap();
        assert_eq!(
            changes(
                &outside_cwd,
                std::slice::from_ref(&outside),
                None,
                GitScope::Uncommitted
            )
            .await
            .unwrap()
            .root,
            None
        );
        assert_eq!(branch(&outside_cwd, &[outside]).await.unwrap(), None);
    }

    #[tokio::test]
    async fn listed_worktrees_can_be_selected_with_commits_since_origin() {
        let (_dir, cwd, roots) = repo();
        let root = Path::new(cwd.as_str());
        std::fs::write(root.join("base"), "base\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "base"]);
        run(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        run(
            root,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );
        run(root, &["worktree", "add", "-b", "feature", "checkout"]);
        std::fs::write(root.join(".git/info/exclude"), "checkout\n").unwrap();
        let checkout = Cwd::new(root.join("checkout").to_str().unwrap()).unwrap();
        let tree = Path::new(checkout.as_str());
        std::fs::write(tree.join("feature"), "committed\n").unwrap();
        run(tree, &["add", "."]);
        run(tree, &["commit", "-m", "feature"]);
        std::fs::write(tree.join("base"), "unstaged\n").unwrap();
        std::fs::write(root.join("upstream"), "upstream\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "upstream"]);
        run(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);

        let list = changes(&cwd, &roots, None, GitScope::Uncommitted)
            .await
            .unwrap();
        assert_eq!(list.root, Some(cwd.clone()));
        assert!(list.files.is_empty());
        let listed: Vec<_> = list
            .worktrees
            .iter()
            .map(|w| (w.root.clone(), w.branch.as_deref()))
            .collect();
        assert_eq!(
            listed,
            [
                (cwd.clone(), Some("main")),
                (checkout.clone(), Some("feature"))
            ]
        );

        let list = changes(&cwd, &roots, Some(&checkout), GitScope::Uncommitted)
            .await
            .unwrap();
        assert_eq!(list.root, Some(checkout.clone()));
        assert_eq!(list.branch.as_deref(), Some("feature"));
        assert_eq!(list.base, None);
        let files: Vec<_> = list
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.section))
            .collect();
        assert_eq!(files, [("base", GitSection::Unstaged)]);
        let list = changes(&cwd, &roots, Some(&checkout), GitScope::Branch)
            .await
            .unwrap();
        assert_eq!(list.root, Some(checkout.clone()));
        assert_eq!(list.base.as_deref(), Some("origin/main"));
        let files: Vec<_> = list
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.section))
            .collect();
        assert_eq!(files, [("feature", GitSection::Committed)]);
        let committed = diff(
            &cwd,
            &roots,
            &AgentDiffParams {
                terminal_id: protocol::TerminalId::new("term_test").unwrap(),
                root: checkout.clone(),
                path: "feature".into(),
                section: GitSection::Committed,
                pull_request: None,
            },
        )
        .await
        .unwrap();
        assert!(committed.patch.contains("+committed"));

        let foreign = root.join("foreign");
        run(root, &["init", "-b", "main", "foreign"]);
        std::fs::write(
            root.join(".git/worktrees/checkout/gitdir"),
            format!("{}\n", foreign.join(".git").display()),
        )
        .unwrap();
        let foreign = Cwd::new(foreign.to_str().unwrap()).unwrap();
        let list = changes(&cwd, &roots, Some(&foreign), GitScope::Uncommitted)
            .await
            .unwrap();
        assert!(list.worktrees.iter().any(|w| w.root == foreign));
        assert_eq!(list.root, Some(cwd.clone()));

        let other = tempfile::tempdir_in(root).unwrap();
        let other = Cwd::new(other.path().canonicalize().unwrap().to_str().unwrap()).unwrap();
        assert_eq!(
            changes(&cwd, &roots, Some(&other), GitScope::Uncommitted)
                .await
                .unwrap()
                .root,
            Some(cwd.clone())
        );
        let unlisted = AgentDiffParams {
            terminal_id: protocol::TerminalId::new("term_test").unwrap(),
            root: other,
            path: "base".into(),
            section: GitSection::Unstaged,
            pull_request: None,
        };
        assert!(diff(&cwd, &roots, &unlisted).await.is_err());
    }

    #[tokio::test]
    async fn worktrees_are_summarized_and_a_foreign_one_is_not() {
        let (_dir, cwd, roots) = repo();
        let root = Path::new(cwd.as_str());
        std::fs::write(root.join("base"), "one\ntwo\nthree\nfour\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "base"]);
        run(root, &["worktree", "add", "-b", "feature", "checkout"]);
        std::fs::write(root.join(".git/info/exclude"), "checkout\nforeign\n").unwrap();
        let checkout = Cwd::new(root.join("checkout").to_str().unwrap()).unwrap();
        let tree = Path::new(checkout.as_str());
        std::fs::write(tree.join("base"), "one\n2\nthree\nfour\n").unwrap();
        run(tree, &["commit", "-am", "Change\u{1b}[2J the \u{202e}base"]);
        run(tree, &["mv", "base", "renamed"]);
        std::fs::write(tree.join("new"), "a\nb\n").unwrap();

        let list = changes(&cwd, &roots, Some(&checkout), GitScope::Worktrees)
            .await
            .unwrap();
        assert_eq!(list.root, Some(cwd.clone()));
        assert!(list.files.is_empty() && list.pull_requests.is_empty());
        let summary = |w: &GitWorktree| {
            (
                w.branch.clone(),
                w.subject.clone(),
                w.ahead,
                w.uncommitted,
                w.additions,
                w.deletions,
            )
        };
        let main = &list.worktrees[0];
        assert_eq!(
            summary(main),
            (
                Some("main".into()),
                Some("base".into()),
                Some(0),
                Some(0),
                Some(0),
                Some(0)
            )
        );
        assert!(main.committed_at_ms.unwrap() > 1_600_000_000_000);
        assert_eq!(
            summary(&list.worktrees[1]),
            (
                Some("feature".into()),
                Some("Change[2J the base".into()),
                Some(1),
                Some(2),
                Some(1),
                Some(1)
            )
        );

        let foreign = root.join("foreign");
        run(root, &["init", "-b", "main", "foreign"]);
        std::fs::write(
            root.join(".git/worktrees/checkout/gitdir"),
            format!("{}\n", foreign.join(".git").display()),
        )
        .unwrap();
        let list = changes(&cwd, &roots, None, GitScope::Worktrees)
            .await
            .unwrap();
        let foreign = list
            .worktrees
            .iter()
            .find(|w| w.root.as_str() == foreign.to_str().unwrap())
            .unwrap();
        assert_eq!(summary(foreign).1, None);
        assert_eq!(summary(foreign).3, None);
    }

    #[test]
    fn status_and_numstat_counts_skip_rename_sources() {
        assert_eq!(status_count(b""), 0);
        assert_eq!(
            status_count(b"R  new\0old\0 M both\0?? untracked\0C  copy\0src\0"),
            4
        );
        assert_eq!(numstat_totals(b""), (0, 0));
        assert_eq!(
            numstat_totals(b"3\t1\ta.rs\0-\t-\tbinary\x002\t0\t\0old\0new\0x\ty\tbad\0"),
            (5, 1)
        );
        assert_eq!(
            numstat_totals(b"4294967295\t0\ta\x004294967295\t0\tb\0"),
            (u32::MAX, 0)
        );
    }

    #[test]
    fn github_file_lists_are_untrusted() {
        let page = pull_page(
            br#"[
                {"filename":"src/a.rs","previous_filename":null,"status":"modified","additions":3,"deletions":1,"sha":"s1"},
                {"filename":"b.rs","previous_filename":"old/b.rs","status":"renamed","additions":0,"deletions":0,"sha":"s2"},
                {"filename":"gone.rs","status":"removed","additions":0,"deletions":9,"sha":"s3"},
                {"filename":"new.rs","status":"added","additions":4294967296,"deletions":0},
                {"filename":"../etc/passwd","status":"added","additions":1,"deletions":0},
                {"filename":"/abs","status":"added","additions":1,"deletions":0},
                {"filename":"ok.rs","previous_filename":"../escape","status":"renamed"},
                {"filename":"","status":"added"},
                {"filename":"x.rs","status":"--evil"}
            ]"#,
        )
        .unwrap();
        assert_eq!(page.len(), 9);
        let files: Vec<_> = page.iter().filter_map(|f| pull_file(7, f)).collect();
        let shown: Vec<_> = files
            .iter()
            .map(|f| {
                (
                    f.path.as_str(),
                    f.old_path.as_deref(),
                    f.status.as_str(),
                    f.additions,
                    f.deletions,
                    f.section,
                )
            })
            .collect();
        let section = GitSection::PullRequest;
        assert_eq!(
            shown,
            [
                ("src/a.rs", None, "M", Some(3), Some(1), section),
                ("b.rs", Some("old/b.rs"), "R", Some(0), Some(0), section),
                ("gone.rs", None, "D", Some(0), Some(9), section),
                ("new.rs", None, "A", None, Some(0), section),
                ("x.rs", None, "M", Some(0), Some(0), section),
            ]
        );
        assert_ne!(
            pull_file(7, &page[0]).unwrap().hash,
            pull_file(8, &page[0]).unwrap().hash
        );
        assert!(pull_page(b"{\"message\":\"Not Found\"}").is_err());
        assert!(pull_page(b"[{\"status\":\"added\"}]").is_err());
        assert_eq!(
            pull_text(Some("@@ -1 +1 @@\n-a\n+b".into())),
            "@@ -1 +1 @@\n-a\n+b\n"
        );
        assert!(pull_text(None).starts_with("Binary "));
        assert!(pull_text(Some(String::new())).starts_with("Binary "));
    }

    #[tokio::test]
    async fn bounds_paths_and_symlinks_do_not_read_outside_checkout() {
        let (_dir, cwd, roots) = repo();
        let root = Path::new(cwd.as_str());
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "secret content").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("escape")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), root.join("link")).unwrap();
        assert!(untracked_content(&cwd, "escape/secret").is_err());
        assert!(untracked_content(&cwd, "../secret").is_err());
        assert!(untracked_content(&cwd, "/etc/passwd").is_err());
        let symlink = patch(&cwd, &roots, "link", GitSection::Untracked).await;
        assert!(!symlink.patch.contains("secret content"));
        assert!(
            changes(
                &cwd,
                &[outside.path().to_owned()],
                None,
                GitScope::Uncommitted
            )
            .await
            .is_err()
        );
        for index in 0..210 {
            std::fs::write(root.join(format!("file{index}")), "\u{1}\n".repeat(40000)).unwrap();
        }
        let list = changes(&cwd, &roots, None, GitScope::Uncommitted)
            .await
            .unwrap();
        assert!(list.truncated);
        assert!(list.files.len() <= MAX_FILES);
        assert!(serde_json::to_vec(&list).unwrap().len() <= RESPONSE_LIMIT);
        let diff = patch(&cwd, &roots, "file0", GitSection::Untracked).await;
        assert!(diff.truncated);
        assert!(serde_json::to_vec(&diff).unwrap().len() <= RESPONSE_LIMIT);
        std::fs::write(root.join("huge"), vec![b'a'; FILE_LIMIT as usize + 1]).unwrap();
        assert!(untracked_content(&cwd, "huge").is_err());
    }
    #[tokio::test]
    async fn rename_with_recreated_source_keeps_only_selected_file_patch() {
        let (_dir, cwd, roots) = repo();
        let root = Path::new(cwd.as_str());
        std::fs::write(root.join("old"), "old contents\n".repeat(10)).unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "base"]);
        run(root, &["mv", "old", "new"]);
        std::fs::write(root.join("new"), "old contents\n".repeat(9) + "changed\n").unwrap();
        std::fs::write(root.join("old"), "recreated source\n").unwrap();
        run(root, &["add", "."]);
        let patch = patch(&cwd, &roots, "new", GitSection::Staged).await.patch;
        assert!(patch.contains("+changed"));
        assert!(!patch.contains("recreated source"));
        assert_eq!(patch.matches("diff --git").count(), 1);
    }

    #[tokio::test]
    async fn reads_do_not_execute_external_diff_textconv_or_clean_filters() {
        let (_dir, cwd, roots) = repo();
        let root = Path::new(cwd.as_str());
        std::fs::write(root.join("file"), "base\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "base"]);
        std::fs::write(root.join("file"), "changed\n").unwrap();
        std::fs::write(root.join(".gitattributes"), "file filter=test diff=test\n").unwrap();
        let command = format!("touch {}; cat", root.join("executed").display());
        for key in [
            "diff.external",
            "diff.test.command",
            "diff.test.textconv",
            "filter.test.clean",
            "filter.test.process",
            "core.fsmonitor",
        ] {
            run(root, &["config", key, &command]);
        }
        run(root, &["config", "filter.test.required", "true"]);
        assert!(
            !changes(&cwd, &roots, None, GitScope::Uncommitted)
                .await
                .unwrap()
                .files
                .is_empty()
        );
        assert!(
            patch(&cwd, &roots, "file", GitSection::Unstaged)
                .await
                .patch
                .contains("+changed")
        );
        assert!(!root.join("executed").exists());
    }

    #[tokio::test]
    async fn reads_do_not_lazily_fetch_missing_blobs() {
        let (dir, cwd, _) = repo();
        let root = Path::new(cwd.as_str());
        std::fs::write(root.join("file"), "a\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "a"]);
        std::fs::write(root.join("file"), "b\n").unwrap();
        run(root, &["commit", "-am", "b"]);
        run(root, &["clone", "--bare", ".", "bare.git"]);
        run(
            root,
            &[
                "--git-dir=bare.git",
                "config",
                "uploadpack.allowFilter",
                "true",
            ],
        );
        let url = format!("file://{}", root.join("bare.git").display());
        run(root, &["clone", "--filter=blob:none", &url, "clone"]);
        let clone = root.join("clone");
        run(&clone, &["reset", "--soft", "HEAD~1"]);
        let marker = dir.path().join("fetched");
        let uploadpack = format!("touch {}; git-upload-pack", marker.display());
        run(&clone, &["config", "remote.origin.uploadpack", &uploadpack]);
        run(&clone, &["config", "protocol.file.allow", "always"]);
        let cwd = Cwd::new(clone.to_str().unwrap()).unwrap();
        let roots = vec![clone.clone()];
        let _ = changes(&cwd, &roots, None, GitScope::Uncommitted).await;
        let _ = diff(
            &cwd,
            &roots,
            &AgentDiffParams {
                terminal_id: protocol::TerminalId::new("term_test").unwrap(),
                root: cwd.clone(),
                path: "file".into(),
                section: GitSection::Staged,
                pull_request: None,
            },
        )
        .await;
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn fetches_do_not_fetch_bundle_uris() {
        let (dir, cwd, _) = repo();
        let root = Path::new(cwd.as_str());
        let marker = dir.path().join("helper");
        let helper = format!("!touch {}", marker.display());
        for (key, value) in [
            ("fetch.bundleURI", "https://127.0.0.1:1/bundle"),
            ("http.sslCert", "cert.pem"),
            ("http.sslCertPasswordProtected", "true"),
            ("credential.helper", &helper),
        ] {
            run(root, &["config", key, value]);
        }
        // A lazy fetch is a `git fetch` child inheriting the builder's environment and config.
        let _ = git_command(&cwd, &["fetch"], 4096, &[]).await;
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn branch_scope_falls_back_from_origin_head_to_main_then_master() {
        let (_dir, cwd, roots) = repo();
        let root = Path::new(cwd.as_str());
        let branch = async || changes(&cwd, &roots, None, GitScope::Branch).await.unwrap();
        let list = branch().await;
        assert_eq!((list.base, list.files), (None, Vec::new()));
        std::fs::write(root.join("base"), "base\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "base"]);
        run(root, &["branch", "-m", "trunk"]);
        assert_eq!(branch().await.base, None);
        assert!(
            diff(
                &cwd,
                &roots,
                &AgentDiffParams {
                    terminal_id: protocol::TerminalId::new("term_test").unwrap(),
                    root: cwd.clone(),
                    path: "base".into(),
                    section: GitSection::Committed,
                    pull_request: None,
                },
            )
            .await
            .is_err()
        );
        run(root, &["branch", "master"]);
        run(root, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(root.join("base"), "feature\n").unwrap();
        std::fs::write(root.join("added"), "added\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "feature"]);
        std::fs::write(root.join("base"), "uncommitted\n").unwrap();
        let list = branch().await;
        assert_eq!(list.base.as_deref(), Some("master"));
        let files: Vec<_> = list
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.section, f.status.as_str(), f.additions))
            .collect();
        assert_eq!(
            files,
            [
                ("added", GitSection::Committed, "A", Some(1)),
                ("base", GitSection::Committed, "M", Some(1))
            ]
        );
        let committed = patch(&cwd, &roots, "base", GitSection::Committed)
            .await
            .patch;
        assert!(committed.contains("-base\n+feature\n"));
        assert!(!committed.contains("uncommitted"));

        run(root, &["branch", "main", "feature"]);
        let list = branch().await;
        assert_eq!(list.base.as_deref(), Some("main"));
        assert!(list.files.is_empty());

        run(root, &["update-ref", "refs/remotes/origin/trunk", "trunk"]);
        run(
            root,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/trunk",
            ],
        );
        let list = branch().await;
        assert_eq!(list.base.as_deref(), Some("origin/trunk"));
        assert_eq!(list.files.len(), 2);
    }

    #[tokio::test]
    async fn file_hashes_change_only_when_the_file_diff_changes() {
        let (_dir, cwd, roots) = repo();
        let root = Path::new(cwd.as_str());
        std::fs::write(root.join("tracked"), "base\n").unwrap();
        std::fs::write(root.join("other"), "other\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "base"]);
        let hashes = async || {
            changes(&cwd, &roots, None, GitScope::Uncommitted)
                .await
                .unwrap()
                .files
                .into_iter()
                .map(|f| ((f.path, format!("{:?}", f.section)), f.hash.unwrap()))
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        std::fs::write(root.join("tracked"), "one\n").unwrap();
        std::fs::write(root.join("new"), "one\n").unwrap();
        let first = hashes().await;
        assert_eq!(first.len(), 2);
        assert_eq!(hashes().await, first);
        std::fs::write(root.join("tracked"), "two\n").unwrap();
        std::fs::write(root.join("new"), "two\n").unwrap();
        let second = hashes().await;
        for key in first.keys() {
            assert_ne!(first[key], second[key], "{key:?}");
        }
        run(root, &["add", "tracked"]);
        let staged = hashes().await;
        let key = ("tracked".to_owned(), "Staged".to_owned());
        std::fs::write(root.join("other"), "changed\n").unwrap();
        assert_eq!(hashes().await[&key], staged[&key]);
        std::fs::write(root.join("tracked"), "three\n").unwrap();
        run(root, &["add", "tracked"]);
        assert_ne!(hashes().await[&key], staged[&key]);
    }
}
