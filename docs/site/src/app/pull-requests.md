# Pull requests

collie shows the GitHub pull request of an agent's branch and can act on it. Install `gh` on the machine and sign in; `collied doctor` checks it.

## Status

A badge on grid cards, Inbox rows and the session header shows the pull request's state and checks. In the session header, tap it for **Open PR** or a prompt to edit and send: **Fix failing checks**, **Address review comments** or **Resolve conflicts** when they apply, and **Explain this PR**.

## Watch a pull request

For a Claude Code agent's open or draft pull request, choose **Watch PR** in the session's **More** menu and an interval of 2, 5, 15 or 30 minutes. `collied` then wakes the agent, even while the phone is away, when a check fails, all required checks pass, someone else comments or reviews, or the branch starts to conflict. It wakes only an idle or done agent still on the branch, and sends you a "Woke" alert.

The watch stops when you choose **Stop watching**, when the pull request is merged or closed, after 10 wakes in a row made only of comments, or after 8 failed reads in a row.

## Archive when the pull request merges

After a merge, **Archive** leads the session's **More** menu.

With **Settings > Sessions > Archive when the PR merges** on, `collied` [archives](new-task.md#archive) a merged task within about 2 minutes and sends an "Archived" alert, if it is a linked worktree whose:

- panes all hold idle or done agents, not shells;
- agents have no subagent running, which `collied` must be able to confirm;
- checkout has no uncommitted or untracked changes and no commits outside the pull request.

Done, Woke and Archived alerts follow **Settings > Notifications > Notify when an agent finishes**.
