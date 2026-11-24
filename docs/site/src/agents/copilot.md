# Run Copilot CLI from your phone

## What works

| Feature | Copilot CLI |
|---|---|
| Grid preview, status and Inbox | Yes |
| [Chat](../app/chat-terminal.md) and Terminal | Yes |
| [Remaining context](../app/agents-tab.md#remaining-context) and [subagents](../app/agents-tab.md#running-subagents) | Subagents only |
| [Approval alerts](../notifications/approvals.md) | Yes, without Approve or Deny: answer in the session |
| [Plan usage](../app/usage.md) | Monthly premium usage, with `gh` signed in |
| [Token cost](../app/usage.md#tokens-and-cost) | Yes |
| Slash command menu, notice buttons, draft sync and [pull request watch](../app/pull-requests.md#watch-a-pull-request) | No |
| [New task](../app/new-task.md) | Yes, Resume included; in [Herdr Core](../app/new-task.md#herdr-core) and [Rex](../app/new-task.md#rex), Copilot CLI 1.0.106 or later |

## Set up

1. Install Copilot CLI on the machine as `copilot`.
2. For plan usage, install the GitHub CLI and run `gh auth login` with the github.com account that has the Copilot seat.

## Chat

In herdr, Chat follows `/new`. Copilot names a session after your first prompt; until then, or while two conversations in the pane share a name, Chat cannot tell which one the pane shows and says so. Terminal still works.

In Herdr Core and Rex, Chat shows the conversation Copilot CLI marks as its own when it starts or resumes. After `/new`, Copilot CLI keeps both conversations until it exits, so Chat says it cannot tell which one you see; use Terminal, or quit and [resume](../app/new-task.md#resume-a-session) the one you want.

## Prompts and keys

Your prompts, typed answers and keys reach Copilot CLI only while it shows its input box. While a picker such as `/model` or another dialog is open, collie refuses them and says to answer the dialog in Terminal on the machine; this holds in herdr, Herdr Core and Rex. A prompt also needs the input box to be empty: send or clear what is typed there first. A picker you open from the phone's keyboard can only be closed on the machine, and so can Copilot's sidebar, its Sessions tab and its help, which Left on an empty input box, Tab and `?` open.

In Herdr Core and Rex, Copilot CLI takes your input only from version 1.0.106, which reports its own status; an older one is shown but read-only.

## Plan usage

`collied` reads premium usage from GitHub every five minutes with your own `gh` login; the token stays with `gh`. The row clears when `gh` is signed out or the account has no premium quota. A seat assigned by a GitHub Enterprise Cloud account with data residency (`*.ghe.com`) does not show.

## Startup prompts

When a new task meets Copilot's folder trust prompt, it stops without sending your prompt. Answer the trust prompt in Terminal: move with the arrow keys and press Return, press a digit to pick its option at once, or press Esc. Then send your prompt from Chat.
