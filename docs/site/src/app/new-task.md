# New task

**New task** starts Claude Code, Codex or Copilot CLI in a new herdr workspace on the machine you pick.

<div class="screenshots">
<img class="screenshot-light" src="../images/new-task-light.png" width="300" height="652" loading="lazy" alt="The New task sheet: location, folder, agent picker and prompt">
<img class="screenshot-dark" src="../images/new-task-dark.png" width="300" height="652" loading="lazy" alt="The New task sheet: location, folder, agent picker and prompt">
</div>

## Plan usage

When the agent you pick has used 80% or more of a [plan limit](usage.md) on that machine, a line under the agent picker says so, such as "Claude 5-hour limit at 92%, resets at 3:40 PM", and turns orange at 100%.

## Choose a folder

Each machine can have a **base folder** (**Machines** > the machine > **Base folder**). Give its full path, such as `/Users/you/git`: collie does not expand `~`. It must be inside one of `collied`'s task roots (`[tasks] roots` in `collied.toml`, your home folder by default).

Type a folder's name in the base, tap **Browse**, pick a recent folder or give an absolute path inside a task root. **New folder** creates an empty folder, without `git init`, in the base, or in a task root when no base is set.

## Clone a repository

Choose **Clone** and give `owner/repo`, an `https://` URL or a `git@host:path` address. `collied` clones it with `gh` or `git` into a new folder in the base (or a task root).

## Git worktree

Choose **Git worktree**, pick the repository as you would a folder, then:

- **Create** a new branch, named for you unless you type a name, with its checkout under the repository's `.worktree/` folder;
- **Open** an existing checkout;
- check out a **Pull request** of that repository by number; `collied` fetches its branch from `origin` with `gh`, and refuses pull requests from forks.

Check the branch and path before you tap **Start**. If the agent fails to start, the checkout and workspace stay, and the error names them.

## Resume a session

Choose **Resume** and a folder to list its recent Claude Code and Codex sessions by first prompt, then resume one in a new workspace, with an optional prompt. Sessions open in a herdr pane are left out.

## Options

- **Ultracode** (Claude Code only, off by default) turns on Claude Code's Ultracode setting; if it stays off, the task fails without sending the prompt.
- Codex tasks start with `--no-daemon`: see [Codex](../agents/codex.md).

## Archive

**Archive**, in a session's **More** menu or an agent's long-press menu, cleans up a finished task:

| Task in | Archive |
|---|---|
| A linked worktree | Closes the workspace and removes the checkout, **uncommitted changes included**, then runs `gh poi` to clean up merged branches and worktrees |
| A regular folder | Closes only the pane; runs `gh poi` when the repository is inside a task root |

Archiving the last pane of a repository workspace with linked worktree sessions leaves a shell there to keep those sessions open.
