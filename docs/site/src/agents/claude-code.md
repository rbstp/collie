# Run Claude Code from your phone

## What works

| Feature | Claude Code |
|---|---|
| Grid preview, status and Inbox | Yes |
| [Chat](../app/chat-terminal.md) and Terminal | Yes |
| [Context ring](../app/agents-tab.md#remaining-context) and [subagents](../app/agents-tab.md#running-subagents) | Yes |
| [Approval alerts](../notifications/approvals.md) | Approve, Deny or another option, from the phone or watch |
| [Plan usage](../app/usage.md) | 5-hour and weekly, with the [status line tap](../install/claude-code.md#plan-usage) |
| [Token cost](../app/usage.md#tokens-and-cost) | Yes |
| [Slash command menu](#slash-commands), [notice buttons](#notices), [draft sync](#draft-sync) and [pull request watch](../app/pull-requests.md#watch-a-pull-request) | Yes |
| [New task](../app/new-task.md) | Yes, with [Resume](../app/new-task.md#resume-a-session) and Ultracode |

## Set up

1. Install the [PermissionRequest hook](../install/claude-code.md#the-permissionrequest-hook) so approval alerts name the exact command.
2. Add the [status line tap](../install/claude-code.md#plan-usage) for plan usage and exact context windows.
3. Check both with `collied doctor`.

## Approvals

When the prompt offers to amend, you can add a one-line note. `/effort` and some `/config` pickers raise no alert.

## Slash commands

Start a prompt with `/` and collie copies the command into Claude Code's input box, so its menu shows live on the phone. ↑ and ↓ highlight a command, ⇥ takes it into your prompt, and only **Send** runs it. Dictation and double-tap pastes are not copied.

## Notices

Notices above the input box, such as the "Heads up" tip or the session rating question, show their options as buttons. Answering the tip's feedback options makes Claude Code send Anthropic a signal about the tip, never its text or your transcript.

## Draft sync

Unsent text in the machine's input box fills the phone's empty prompt field; sending from the phone replaces it.

## Fullscreen mode

In fullscreen mode (`"tui": "fullscreen"`), Terminal shows one screen. `collied` refuses keys and prompts while the transcript is scrolled up on the machine; tap **Jump to bottom**.

## Startup prompts

The folder trust prompt arrives as an approval: Approve trusts the folder, and Deny ends the session. A new task that meets it stops without sending your prompt.
