# Run Codex from your phone

You can follow Codex agents, read them in Chat and answer their prompts from the phone. Codex approvals come as blocked notifications, without Approve or Deny buttons.

## What works

| Feature | Codex |
|---|---|
| Grid preview, status and Inbox | Yes |
| [Chat](../app/chat-terminal.md) and Terminal | Yes, with herdr's Codex hook |
| Remaining context ring | Yes |
| Blocked notifications | Yes, without Approve or Deny |
| Answering prompts | Terminal buttons or typing in the composer |
| [Plan usage](../app/usage.md) | Monthly credits for a credit-metered workspace |
| [Resume](../app/new-task.md#resume-a-session) in New task | Yes |

## Set up

1. Install herdr's Codex hook, and run it again after every herdr update:

   ```sh
   herdr integration install codex
   ```

   The hook tells herdr which Codex session runs in each pane, and Chat reads that session. herdr 0.9.5 installs version 10 of the hook. It removes the turn hooks that herdr 0.9.4 added, which make Codex warn "clamping Interrupt hook timeout". Once Codex has its `~/.codex` folder (or `$CODEX_HOME`), `collied doctor` warns when the hook is missing or older.

2. Start Codex tasks from the phone, or start Codex with `--no-daemon` yourself.

## Why `--no-daemon`

collie starts new Codex tasks with `codex --no-daemon`, so each pane reports its own session, including after `/new` or a resume. Each window then starts its own server and uses some more memory.

collie leaves sessions already running on a shared daemon as they are. To move one over, finish the current turn, then relaunch it in the same pane:

```sh
codex --no-daemon resume <session-id>
```

Use the session's real ID, not one guessed from the folder.

## Plan usage

For a credit-metered workspace, the Usage view shows the credits used and allowed this month, and the reset date. `collied` reads them from the local Codex app-server every five minutes, and Codex's sign-in token stays with Codex. The row clears after you sign out or switch to an account without a monthly limit.

## Status and done alerts

herdr reads Codex's status from its screen. If Codex statuses or done alerts look wrong, see [Troubleshooting](../troubleshooting.md#codex-status-or-done-alerts-missing).

## Startup prompts

When a startup prompt, such as folder trust or Codex's update picker, appears, the new task stops with a message. Answer the prompt in Terminal. collie never types into the update picker.
