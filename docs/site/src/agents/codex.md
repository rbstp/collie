# Run Codex from your phone

## What works

| Feature | Codex |
|---|---|
| Grid preview, status and Inbox | Yes; if status looks wrong, see [Troubleshooting](../troubleshooting.md#codex-status-or-done-alerts-missing) |
| [Chat](../app/chat-terminal.md) and Terminal | Yes; Chat needs the [hook](#set-up) |
| [Context ring](../app/agents-tab.md#remaining-context) and [subagents](../app/agents-tab.md#running-subagents) | With the [hook](#set-up) |
| [Approval alerts](../notifications/approvals.md) | Yes, without Approve or Deny: answer in the session |
| [Plan usage](../app/usage.md) | Monthly credits, for a credit-metered workspace |
| [Token cost](../app/usage.md#tokens-and-cost) | Yes |
| Slash command menu, notice buttons, draft sync and [pull request watch](../app/pull-requests.md#watch-a-pull-request) | No |
| [New task](../app/new-task.md) | Yes, with [Resume](../app/new-task.md#resume-a-session) |

## Set up

1. Install herdr's Codex hook, and again after every herdr update:

   ```sh
   herdr integration install codex
   ```

   `collied doctor` checks it once `~/.codex` (or `$CODEX_HOME`) exists.

2. Start Codex tasks from the phone, or run `codex --no-daemon` yourself.

## Why `--no-daemon`

With `--no-daemon`, each pane reports its own session, also after `/new` or a resume, but each window runs its own server and uses more memory. To move a session off a shared daemon, finish the current turn, then relaunch it in the same pane with its own session ID:

```sh
codex resume <session-id> --no-daemon
```

## Plan usage

`collied` reads the credits from the local Codex app-server every five minutes; Codex keeps its sign-in token. The row clears after you sign out or switch to an account without a monthly limit.

## Startup prompts

When a new task meets a startup prompt, such as folder trust or the update picker, it stops without sending your prompt. Answer the startup prompt in Terminal.
