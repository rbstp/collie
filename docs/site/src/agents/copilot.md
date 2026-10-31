# Run Copilot CLI from your phone

## What works

| Feature | Copilot CLI |
|---|---|
| Grid preview, status and Inbox | Yes |
| [Chat](../app/chat-terminal.md) and Terminal | Yes |
| [Context ring](../app/agents-tab.md#remaining-context) and [subagents](../app/agents-tab.md#running-subagents) | Subagents only |
| [Approval alerts](../notifications/approvals.md) | Yes, without Approve or Deny: answer in the session |
| [Plan usage](../app/usage.md) | Monthly premium usage, with `gh` signed in |
| [Token cost](../app/usage.md#tokens-and-cost) | Yes |
| Slash command menu, notice buttons, draft sync and [pull request watch](../app/pull-requests.md#watch-a-pull-request) | No |
| [New task](../app/new-task.md) | Yes |

## Set up

1. Install Copilot CLI on the machine as `copilot`.
2. For plan usage, install the GitHub CLI and run `gh auth login` with the github.com account that has the Copilot seat.

## Chat

Chat follows `/new`. Copilot names a session after your first prompt; until then, or while two conversations in the pane share a name, Chat cannot tell which one the pane shows and says so. Terminal still works.

## Plan usage

`collied` reads premium usage from GitHub every five minutes with your own `gh` login; the token stays with `gh`. The row clears when `gh` is signed out or the account has no premium quota. A seat assigned by a GitHub Enterprise Cloud account with data residency (`*.ghe.com`) does not show.

## Startup prompts

When a new task meets Copilot's folder trust prompt, it stops without sending your prompt. Answer the trust prompt in Terminal.
