use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use protocol::{AgentDiffParams, Cwd, ErrorCode, GitChanges, GitDiff, GitFile, GitSection};
use rustix::fs::{Mode, OFlags};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::drive::{Fail, resolve_cwd};

const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const FILE_LIMIT: u64 = 1024 * 1024;
const RESPONSE_LIMIT: usize = 48 * 1024;
const MAX_FILES: usize = 200;

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
        let (out, ok, truncated) = git(&root, &["rev-parse", option], 4096).await?;
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
        .map_err(error)?;
    }
    Ok(Some(root))
}

fn diff_args(section: GitSection) -> Vec<&'static str> {
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--find-renames",
        "--ignore-submodules=none",
    ];
    if section == GitSection::Staged {
        args.push("--cached");
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
        files.push(GitFile {
            path: path(name)?,
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

pub async fn changes(cwd: &Cwd, roots: &[PathBuf]) -> Result<GitChanges, Fail> {
    let Some(root) = root(cwd, roots).await? else {
        return Ok(GitChanges {
            root: None,
            branch: None,
            files: Vec::new(),
            truncated: false,
        });
    };
    let (branch, ok, _) = git(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"], 4096).await?;
    let branch = ok.then(|| {
        String::from_utf8_lossy(&branch)
            .trim_end_matches('\n')
            .to_owned()
    });
    let mut changes = GitChanges {
        root: Some(root.clone()),
        branch,
        files: Vec::new(),
        truncated: false,
    };
    for section in [GitSection::Staged, GitSection::Unstaged] {
        let mut args = diff_args(section);
        args.extend(["--raw", "--numstat", "-z", "--"]);
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
            path: name,
            old_path: None,
            section: GitSection::Untracked,
            status: "?".into(),
            additions,
            deletions: additions.map(|_| 0),
        });
    }
    if changes.files.len() > MAX_FILES {
        changes.files.truncate(MAX_FILES);
        changes.truncated = true;
    }
    while serde_json::to_vec(&changes)
        .map_err(|_| error("Could not encode Git changes"))?
        .len()
        > RESPONSE_LIMIT
    {
        changes.files.pop();
        changes.truncated = true;
    }
    Ok(changes)
}

pub async fn diff(cwd: &Cwd, roots: &[PathBuf], p: &AgentDiffParams) -> Result<GitDiff, Fail> {
    path(p.path.as_bytes())?;
    let Some(root) = root(cwd, roots).await? else {
        return Err(error("This folder is no longer a Git checkout"));
    };
    if root != p.root {
        return Err(error("Agent checkout changed; reopen Git changes"));
    }
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
        let mut args = diff_args(p.section);
        args.extend(["--raw", "-z", "--"]);
        let (out, ok, truncated) = git(&root, &args, OUTPUT_LIMIT).await?;
        if !ok || truncated {
            return Err(error("Could not resolve changed file; refresh"));
        }
        let file = tracked(&out, p.section)?
            .into_iter()
            .find(|f| f.path == p.path)
            .ok_or_else(|| error("File no longer has changes in this section; refresh"))?;
        let mut args = diff_args(p.section);
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
        let list = changes(&cwd, &roots).await.unwrap();
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
        assert!(changes(&cwd, &roots).await.unwrap().files.is_empty());
        let root = Path::new(cwd.as_str());
        std::fs::write(root.join("first"), "new\n").unwrap();
        run(root, &["add", "."]);
        assert_eq!(
            changes(&cwd, &roots).await.unwrap().files[0].additions,
            Some(1)
        );
        run(root, &["commit", "-m", "first"]);
        run(root, &["worktree", "add", "-b", "feature", "checkout"]);
        let checkout = Cwd::new(root.join("checkout").to_str().unwrap()).unwrap();
        std::fs::write(Path::new(checkout.as_str()).join("first"), "worktree\n").unwrap();
        let list = changes(&checkout, &roots).await.unwrap();
        assert_eq!(list.root, Some(checkout.clone()));
        assert_eq!(list.branch.as_deref(), Some("feature"));
        assert!(
            patch(&checkout, &roots, "first", GitSection::Unstaged)
                .await
                .patch
                .contains("+worktree")
        );
        run(Path::new(checkout.as_str()), &["checkout", "--detach"]);
        assert_eq!(changes(&checkout, &roots).await.unwrap().branch, None);
        let outside = tempfile::tempdir().unwrap();
        let outside = outside.path().canonicalize().unwrap();
        let outside_cwd = Cwd::new(outside.to_str().unwrap()).unwrap();
        assert_eq!(changes(&outside_cwd, &[outside]).await.unwrap().root, None);
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
        assert!(changes(&cwd, &[outside.path().to_owned()]).await.is_err());
        for index in 0..210 {
            std::fs::write(root.join(format!("file{index}")), "\u{1}\n".repeat(40000)).unwrap();
        }
        let list = changes(&cwd, &roots).await.unwrap();
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
        assert!(!changes(&cwd, &roots).await.unwrap().files.is_empty());
        assert!(
            patch(&cwd, &roots, "file", GitSection::Unstaged)
                .await
                .patch
                .contains("+changed")
        );
        assert!(!root.join("executed").exists());
    }
}
