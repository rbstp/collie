# Run Claude Code from your phone

Claude Code has the most complete support in collie, and is the only agent whose approval alerts offer Approve and Deny.

## What works

| Feature | Claude Code |
|---|---|
| Grid preview, status and Inbox | Yes |
| [Chat](../app/chat-terminal.md) and Terminal | Yes |
| [Context ring](../app/agents-tab.md#remaining-context) and [subagents](../app/agents-tab.md#running-subagents) | Yes |
| Approval alerts | Approve, Deny or another option from the phone or watch; the exact command with the [hook](../install/claude-code.md#the-permissionrequest-hook) |
| Answer prompts in the session | Yes |
| [Plan usage](../app/usage.md) | 5-hour and weekly, with the [status line tap](../install/claude-code.md#plan-usage) |
| [Token cost](../app/usage.md#tokens-and-cost) | Yes |
| Slash command menu, notice buttons and draft sync | Yes |
| [Watch a pull request](../app/pull-requests.md#watch-a-pull-request) | Yes |
| [New task](../app/new-task.md) | Yes, with [Resume](../app/new-task.md#resume-a-session) and Ultracode |

## Set up

1. Install the [PermissionRequest hook](../install/claude-code.md#the-permissionrequest-hook) so approval alerts name the exact command.
2. Add the [status line tap](../install/claude-code.md#plan-usage) for plan usage and exact context windows.
3. Check both with `collied doctor`.

## Approvals

When Claude Code blocks on a permission prompt, collie sends an alert that names the tool call. Decide in the app, on the alert, on a Live Activity or on the watch: the app asks for Face ID or the passcode for each decision, and the others need the iPhone or watch unlocked. When the prompt offers to amend, you can add a one-line note. `/effort` and some `/config` pickers raise no alert.

You can also answer with the terminal buttons or by typing in the composer, without Face ID. See [Approvals and alerts](../notifications/approvals.md).

## Slash commands

When a prompt on the phone starts with `/`, collie mirrors the command into Claude Code's input box, so Claude Code's command menu shows live on the phone. Use ↑ and ↓ to highlight a command and ⇥ to take it into the phone's prompt. Only **Send** runs it. Dictated text and double-tap pastes are not mirrored.

## Notices

Claude Code's notices above the input box, such as the "Heads up" tip or the session rating question, show their options as buttons. A tap sends that option's digit, only while the screen still shows the option above an empty input box. If you answer the tip's feedback options, Claude Code sends Anthropic a feedback signal about the tip, never its text or your transcript.

## Draft sync

Unsent text in Claude Code's input box on the machine fills the phone's empty prompt field, and sending from the phone replaces it.

## Fullscreen mode

Claude Code's fullscreen mode (`"tui": "fullscreen"`) keeps its history out of the pane, so Terminal shows one screen. When someone scrolls the transcript up on the machine, a **Jump to bottom** button brings it back down; until then, `collied` refuses keys and prompts.

## Startup prompts

Claude Code's folder trust prompt arrives as an approval: Approve trusts the folder, and Deny ends the session. A new task that meets it stops with a message, without sending its prompt.
