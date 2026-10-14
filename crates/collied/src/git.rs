use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use protocol::{
    AgentDiffParams, Cwd, ErrorCode, GitChanges, GitDiff, GitFile, GitScope, GitSection,
    GitWorktree, hex,
};
use rustix::fs::{Mode, OFlags};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::drive::{Fail, resolve_cwd};

const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const FILE_LIMIT: u64 = 1024 * 1024;
const RESPONSE_LIMIT: usize = 48 * 1024;
const MAX_FILES: usize = 200;
const MAX_WORKTREES: usize = 20;

fn error(message: &str) -> Fail {
    (ErrorCode::InvalidParams, message.into())
}

async fn git(cwd: &Cwd, args: &[&str], limit: usize) -> Result<(Vec<u8>, bool, bool), Fail> {
    let mut overrides = Vec::new();
    if args.first() == Some(&"diff") {
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
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
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
            list.push(GitWorktree { root, branch });
        }
    }
    Ok(list)
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
    let Some(root) = root(requested, roots)
        .await?
        .filter(|root| root == requested)
    else {
        return Ok(None);
    };
    let common = metadata(checkout, "--git-common-dir", roots).await?;
    Ok((metadata(&root, "--git-common-dir", roots).await? == common).then_some(root))
}

/// Fixed refs only: the phone never names the base.
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
        return Ok(Some((name.into(), format!("{sha}...HEAD"))));
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
        });
    };
    let worktrees = worktrees(&checkout, roots).await?;
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
    };
    if scope == GitScope::Branch {
        if let Some((name, range)) = base(&root).await? {
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
    while serde_json::to_vec(&changes)
        .map_err(|_| error("Could not encode Git changes"))?
        .len()
        > RESPONSE_LIMIT
    {
        if changes.files.pop().is_none() {
            return Err(error("Git changes are too large to list"));
        }
        changes.truncated = true;
    }
    Ok(changes)
}

pub async fn diff(cwd: &Cwd, roots: &[PathBuf], p: &AgentDiffParams) -> Result<GitDiff, Fail> {
    path(p.path.as_bytes())?;
    let Some(checkout) = root(cwd, roots).await? else {
        return Err(error("This folder is no longer a Git checkout"));
    };
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
                .map(|(_, range)| range)
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
    let mut diff = GitDiff {
        patch: String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
    };
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
            },
        )
        .await
        .unwrap()
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
        };
        assert!(diff(&cwd, &roots, &unlisted).await.is_err());
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
