# Run Claude Code from your phone

## What works

| Feature | Claude Code |
|---|---|
| Grid preview, status and Inbox | Yes |
| [Chat](../app/chat-terminal.md) and Terminal | Yes |
| [Remaining context](../app/agents-tab.md#remaining-context) and [subagents](../app/agents-tab.md#running-subagents) | Yes |
| [Approval alerts](../notifications/approvals.md) | Approve, Deny or another option, from the phone or watch |
| [Plan usage](../app/usage.md) | 5-hour and weekly, with the [status line tap](../install/claude-code.md#plan-usage) |
| [Plan limits](#plan-limits) | Stops at the limit instead of using extra usage, and continues at the reset |
| [Token cost](../app/usage.md#tokens-and-cost) | Yes |
| [Slash command menu](#slash-commands), [notice buttons](#notices), [draft sync](#draft-sync), [prompt suggestions](#prompt-suggestions) and [pull request watch](../app/pull-requests.md#watch-a-pull-request) | Yes |
| [New task](../app/new-task.md) | Yes, with [Resume](../app/new-task.md#resume-a-session) and Ultracode, in herdr or [Herdr 2](../app/new-task.md#herdr-2) |

## Set up

1. Install the [PermissionRequest hook](../install/claude-code.md#the-permissionrequest-hook) so approval alerts name the exact command.
2. Add the [status line tap](../install/claude-code.md#plan-usage) for plan usage and exact context windows.
3. To stop agents at the plan limit, also install the [PreToolUse hook](../install/claude-code.md#the-pretooluse-hook).
4. Check them with `collied doctor`.

## Plan limits

With extra usage turned on for your account, Claude Code keeps working past your 5-hour or weekly limit and bills the excess. To have your agents wait for the reset instead, turn on **Stop at the plan limit** in [Usage](../app/usage.md#stop-at-the-plan-limit) and install the [PreToolUse hook](../install/claude-code.md#the-pretooluse-hook).

- When an agent uses 99% of its 5-hour or weekly limit, `collied` stops it before its next tool call and notifies you once, for example "Stopped at the 5-hour limit, resumes in 2 h 5 min". Stopping at 99% normally leaves the agent short of extra usage, but one large request can still cross the limit. When both limits are reached, it waits for the later reset.
- The agent shows an hourglass with the time it resumes. A minute after the reset, `collied` types **Continue.** into it and notifies you.
- To change that, tap the hourglass on the agent's screen: **Cancel resume** leaves the agent stopped, and **Continue now** sends **Continue.** at once, on extra usage.
- A prompt you type on the machine or send from the phone goes through, on extra usage, and cancels the resume.
- Nothing else wakes a stopped agent: a background task that finishes, a subagent that reports back, a scheduled or `/loop` prompt and a message from another session are stopped again, and a pull request watch waits for the resume.
- `collied` does not resume an agent you closed, that started another session, or that is busy at the reset; it tells you when it does not resume one. When an agent hits the limit again right after resuming, `collied` waits once more, then leaves it stopped and tells you.
- Keep the machine awake at the reset. When it wakes later, `collied` resumes the agent then, unless the reset is more than 3 hours past; then it tells you and leaves the agent stopped.
- Each session stops only at its own account's limit, so a session signed in to another account, or using an API key, is not stopped.

## Approvals

When the prompt offers to amend, you can add a one-line note. `/effort` and some `/config` pickers raise no alert.

## Slash commands

Start a prompt with `/` and collie copies the command into Claude Code's input box, so its menu shows live on the phone. ↑ and ↓ highlight a command, ⇥ takes it into your prompt, and only **Send** runs it. Dictation and double-tap pastes are not copied. In Herdr 2, the machine's box takes about 3 s to follow each further change, since herdr-core waits for typing to pause before it sends the next keys.

## Notices

Notices above the input box, such as the "Heads up" tip or the session rating question, show their options as buttons. Answering the tip's feedback options makes Claude Code send Anthropic a signal about the tip, never its text or your transcript.

## Draft sync

Unsent text in the machine's input box fills the phone's empty prompt field; sending from the phone replaces it.

## Prompt suggestions

After a turn, Claude Code may suggest your next prompt, grayed out in its empty input box. The phone shows it too: in Terminal, a ⇥ key in the bottom row takes it into the box; in Chat, an empty prompt field shows it, and ⇥ fills the field with it. Claude Code skips a suggestion when the next step is not obvious, in the first turns of a session, near your plan limits, in plan mode and while a permission prompt waits. The suggestion never goes into notifications, Live Activities or the watch.

## Fullscreen mode

In fullscreen mode (`"tui": "fullscreen"`), Terminal shows one screen. `collied` refuses keys and prompts while the transcript is scrolled up on the machine; tap **Jump to bottom**.

## Startup prompts

The folder trust prompt arrives as an approval: Approve trusts the folder, and Deny ends the session. A new task that meets it stops without sending your prompt.
