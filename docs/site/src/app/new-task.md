# New task

**New task** starts Claude Code, Codex or Copilot CLI in a new herdr workspace on the machine you pick.

## Choose a folder

Each machine can have a **base folder** (Machines > the machine > Base folder). Use a full path such as `/Users/you/git`, since `~` is not expanded. It must be inside one of `collied`'s task roots (`[tasks] roots` in `collied.toml`, your home folder by default).

- Type a name to mean that folder inside the base, with matching folders offered as you type.
- **Browse** lists the base's folders.
- Recent folders and absolute paths inside a task root also work.
- **New folder** creates an empty folder (no `git init`) in the base, or in a task root when no base is set, and starts the agent there. The folder stays if the agent fails to start.

## Clone a repository

Give `owner/repo`, an `https://` URL or a `git@host:path` address. `collied` clones it into a new folder inside the chosen folder with `gh` or `git`, then starts the agent there.

## Git worktree

Choose **Git worktree** to work on a branch in its own checkout:

- **Create** makes a new branch (a name is generated unless you type one) and puts the checkout under the repository's `.worktree/` folder.
- **Open** picks an existing checkout from the list.
- **Pull request** takes the number of a pull request of that repository. `collied` asks `gh` for its branch, fetches it from `origin` and creates the checkout. Pull requests from forks are refused.

Check the branch and path shown before you tap **Start**.

## Resume a session

For Claude Code and Codex, New task can list a folder's recent sessions with their first prompt, and resume one in a new workspace. A session already open in a herdr pane is not listed.

## Options

- **Ultracode** (Claude Code only, off by default) starts the session with Claude Code's Ultracode setting. The task fails without sending the prompt if Claude Code does not turn it on.
- Codex tasks start with `--no-daemon`: see [Codex](../agents/codex.md).

## Archive

**Archive** in a session's menu closes a linked worktree's workspace and removes its checkout, including uncommitted changes, then runs `gh poi` to clean up merged branches and worktrees. For a regular folder, Archive closes only the pane and leaves the folder in place. A banner shows progress, then confirms for 10 seconds.
