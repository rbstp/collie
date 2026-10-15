# Pull requests

When an agent works on a branch with a GitHub pull request, collie shows it. `collied` reads it with the `gh` CLI on the machine, so `gh` must be installed and signed in. `collied doctor` warns when it is not.

## Status

A badge on grid cards, Inbox rows and the session header shows the pull request's number and state (open, draft, merged or closed), with its checks while it is open or a draft.

On the session header, tap the badge for a menu:

- **Open PR**;
- **Fix failing checks**, when checks fail;
- **Address review comments**, when there are reviews or comments;
- **Resolve conflicts**, when the branch conflicts;
- **Explain this PR**, always.

Each prompt fills the composer with one line naming the pull request, ready for you to edit and send.

## Watch a pull request

For a Claude Code agent with an open or draft pull request, the session menu offers **Watch PR**, checking every 2, 5, 15 or 30 minutes. `collied` keeps watching with or without the phone, and wakes the agent with a one-line prompt when:

- a check fails;
- all required checks pass;
- someone else comments or reviews;
- the branch starts to conflict.

It only wakes an agent that is idle or done and still on the watched branch, and sends you a "Woke" alert with a short summary. The watch stops when the pull request is merged or closed, after 10 wakes in a row made only of comments, after 8 failed reads, or when you choose **Stop watching**.

## Archive when the pull request merges

A merged pull request's badge reads **Merged**, and the session menu puts **Archive** first.

Turn on **Settings > Sessions > Archive when the PR merges** to let `collied` archive the task itself. It only archives a linked worktree whose agents are all idle or done and whose checkout has no uncommitted changes, then sends an "Archived" alert. A merge is noticed within about 2 minutes.

Done, Woke and Archived alerts all follow **Settings > Notifications > Notify when an agent finishes**.
