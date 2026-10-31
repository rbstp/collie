# Run Codex from your phone

You can follow Codex agents, read them in Chat and answer their prompts from the phone.

## What works

| Feature | Codex |
|---|---|
| Grid preview, status and Inbox | Yes |
| [Chat](../app/chat-terminal.md) and Terminal | Yes; Chat needs the [hook](#set-up) |
| [Context ring](../app/agents-tab.md#remaining-context) and [subagents](../app/agents-tab.md#running-subagents) | With the [hook](#set-up) |
| Approval alerts | Text read from the screen, without Approve or Deny |
| Answer prompts in the session | Yes |
| [Plan usage](../app/usage.md) | Monthly credits, for a credit-metered workspace |
| [Token cost](../app/usage.md#tokens-and-cost) | Yes |
| Slash command menu, notice buttons and draft sync | No |
| [Watch a pull request](../app/pull-requests.md#watch-a-pull-request) | No |
| [New task](../app/new-task.md) | Yes, with [Resume](../app/new-task.md#resume-a-session) |

## Set up

1. Install herdr's Codex hook, and install it again after every herdr update:

   ```sh
   herdr integration install codex
   ```

   The hook tells herdr which Codex session runs in each pane; Chat, the context ring and subagents read that session. herdr 0.9.5 installs version 10, which drops the turn hooks behind Codex's "clamping Interrupt hook timeout" warning. Once `~/.codex` (or `$CODEX_HOME`) exists, `collied doctor` warns when the hook is missing or older.

2. Start Codex tasks from the phone, or start Codex with `--no-daemon` yourself.

## Why `--no-daemon`

collie starts new Codex tasks with `codex --no-daemon`, so each pane reports its own session, also after `/new` or a resume. Each window then runs its own server, which uses more memory.

Sessions already running on a shared daemon stay as they are. To move one over, finish the current turn, then relaunch it in the same pane with its real session ID, not one guessed from the folder:

```sh
codex resume <session-id> --no-daemon
```

## Status and done alerts

herdr reads Codex's status from its screen. If statuses or done alerts look wrong, see [Troubleshooting](../troubleshooting.md#codex-status-or-done-alerts-missing).

## Plan usage

For a credit-metered workspace, Usage shows the credits used and allowed this month and the reset date. `collied` asks the local Codex app-server every five minutes, and Codex keeps its sign-in token. The row clears after you sign out or switch to an account without a monthly limit.

## Startup prompts

When a startup prompt such as folder trust or Codex's update picker appears, the new task stops with a message, without sending its prompt. Answer the prompt in Terminal; collie never types into the update picker.
