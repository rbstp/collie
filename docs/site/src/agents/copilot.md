# Run Copilot CLI from your phone

You can follow GitHub Copilot CLI agents, read them in Chat and work with them in Terminal.

## What works

| Feature | Copilot CLI |
|---|---|
| Grid preview, status and Inbox | Yes |
| [Chat](../app/chat-terminal.md) and Terminal | Yes |
| [Context ring](../app/agents-tab.md#remaining-context) and [subagents](../app/agents-tab.md#running-subagents) | Subagents only |
| Approval alerts | Text read from the screen, without Approve or Deny |
| Answer prompts in the session | Yes |
| [Plan usage](../app/usage.md) | Monthly premium usage, with `gh` signed in |
| [Token cost](../app/usage.md#tokens-and-cost) | Yes |
| Slash command menu, notice buttons and draft sync | No |
| [Watch a pull request](../app/pull-requests.md#watch-a-pull-request) | No |
| [New task](../app/new-task.md) | Yes |

## Set up

1. Install Copilot CLI on the machine as `copilot`, and leave `copilot` in `[tasks] agents` in `collied.toml`, as it is by default. herdr starts it and reads its status from the screen.
2. For plan usage, install the GitHub CLI (`gh`) on the machine and sign in with `gh auth login` to the github.com account that has the Copilot seat.

## Chat

Chat shows the conversation the pane shows, and follows `/new` to the next one. A subagent's steps appear under the task that started it. Copilot names a session after your first prompt; until then, or when two conversations in the pane share a name, Chat says it cannot tell which conversation the pane shows rather than guess, and Terminal still works.

## Plan usage

Usage shows this month's premium usage: the credits used and allowed (requests for a seat billed by request, "Unlimited" for a seat without a cap) and the reset date. `collied` asks GitHub every five minutes with your own `gh` login, and the token stays with `gh`. The row clears when `gh` is signed out or the account has no premium quota. A seat assigned by a GitHub Enterprise Cloud account with data residency (`*.ghe.com`) does not show.

## Startup prompts

When Copilot asks whether to trust the folder, the new task stops with a message, without sending its prompt. Answer the prompt in Terminal, with the key strip or by typing in the composer.
