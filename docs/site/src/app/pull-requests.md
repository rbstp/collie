# Pull requests

When an agent works on a branch with a GitHub pull request, collie shows the pull request and can act on it. `collied` reads it with the `gh` CLI on the machine, so `gh` must be installed and signed in; `collied doctor` warns when it is not.

## Status

A badge on grid cards, Inbox rows and the session header shows the pull request's number and state (open, draft, merged or closed), with its checks while it is open or a draft.

Tap the badge on the session header for a menu:

- **Open PR**;
- **Fix failing checks**, when checks fail;
- **Address review comments**, when there are reviews or comments;
- **Resolve conflicts**, when the branch conflicts;
- **Explain this PR**, always.

Each prompt adds one line naming the pull request to the composer, for you to edit and send.

## Watch a pull request

For a Claude Code agent with an open or draft pull request, the session's **More** menu offers **Watch PR**, which checks every 2, 5, 15 or 30 minutes. `collied` keeps watching whether or not the phone is connected, and wakes the agent with a one-line prompt when:

- a check fails;
- all required checks pass;
- someone else comments or reviews;
- the branch starts to conflict.

`collied` wakes only an agent that is idle or done and still on the watched branch, and sends you a "Woke" alert with a short summary. Chat, the grid card preview and Inbox show the wake as one line with an eye and that summary, such as "PR watch: CI failed on #123"; in Chat, tap it to read the full prompt. A prompt you type that repeats a wake's text also shows as that wake; in Inbox, matching its first 159 characters is enough.

The watch stops when you choose **Stop watching**, when the pull request is merged or closed, after 10 wakes in a row made only of comments, or after 8 failed reads in a row.

## Archive when the pull request merges

A merged pull request's badge reads **Merged**, and the session's **More** menu puts **Archive** first.

To let `collied` archive the task itself, turn on **Settings > Sessions > Archive when the PR merges**. `collied` notices a merge within about 2 minutes. It archives only a linked worktree whose panes all hold idle or done agents (a shell pane holds it back) and whose checkout has no uncommitted or untracked changes and no commits outside the pull request, then sends an "Archived" alert. It never archives a regular folder.

Done, Woke and Archived alerts all follow **Settings > Notifications > Notify when an agent finishes**.
