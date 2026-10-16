# Run Claude Code from your phone

Claude Code gets the most complete support in collie.

## What works

| Feature | Claude Code |
|---|---|
| Grid preview, status and Inbox | Yes |
| [Chat](../app/chat-terminal.md) and Terminal | Yes |
| Remaining context ring | Yes |
| Approve, Deny and option choices from the app, the lock screen, a Live Activity or the watch | Yes |
| Approval alerts naming the exact command | With the [hook](../install/claude-code.md#the-permissionrequest-hook) |
| [Plan usage](../app/usage.md) (5-hour and weekly) | With the [status line tap](../install/claude-code.md#plan-usage) |
| Slash command menu mirrored from the phone | Yes |
| Notices as buttons | Yes |
| Draft sync with the machine | Yes |
| [Watch a pull request](../app/pull-requests.md#watch-a-pull-request) | Yes |
| [Resume](../app/new-task.md#resume-a-session) and Ultracode in New task | Yes |

## Set up

1. Install the [PermissionRequest hook](../install/claude-code.md#the-permissionrequest-hook) so approvals name the exact tool call.
2. Add the [status line tap](../install/claude-code.md#plan-usage) for plan usage and exact context windows.
3. Check both with `collied doctor`.

## Approvals

When Claude Code blocks on a permission prompt, the alert names the tool call. Approve, Deny and the prompt's other options need Face ID or the passcode for each decision. Folder trust uses the same actions: Approve trusts the folder, Deny ends the session. You can add a one-line note to an approval. Settings pickers such as `/effort` and `/config` raise no notification.

Questions and trust prompts can also be answered with the terminal buttons or by typing in the session composer, without Face ID. See [Approvals and alerts](../notifications/approvals.md).

## Slash commands

While a prompt on the phone starts with `/`, the command is mirrored into Claude Code's input box, so its command menu appears live on the phone. Use ↑ and ↓ to highlight a command and ⇥ to take it into the phone's prompt; only **Send** runs it.

## Notices

The options of a notice above the input box, such as the "Heads up" tip or the session rating question, show as buttons. A tap sends that single digit, and only while the screen still shows that option above an empty input box. The tip feedback options make Claude Code send Anthropic a feedback signal about the tip, never its text or your transcript.

## Draft sync

A prompt typed on the machine but not sent shows up in the phone's prompt field, and sending from the phone replaces it. Text from the mic button or a double-tap paste stays on the phone until you send it.

## Fullscreen mode

Claude Code's fullscreen mode (`"tui": "fullscreen"`) keeps its history out of the pane, so Terminal shows one screen. When its transcript is scrolled up on the machine, a **Jump to bottom** button brings it back down; keys and prompts wait until it is back at the bottom.
