# Run Antigravity CLI from your phone

## What works

| Feature | Antigravity CLI |
|---|---|
| Grid preview, status and Inbox | Yes |
| [Chat](../app/chat-terminal.md) and Terminal | Yes; Chat needs the [hook](#set-up) |
| [Remaining context](../app/agents-tab.md#remaining-context) and [subagents](../app/agents-tab.md#running-subagents) | No |
| [Approval alerts](../notifications/approvals.md) | Yes, without Approve or Deny: answer in the session |
| [Plan usage](../app/usage.md) | Yes; needs the [status line tap](#plan-usage) |
| [Token cost](../app/usage.md#tokens-and-cost) | Yes |
| Slash command menu, notice buttons, draft sync and [pull request watch](../app/pull-requests.md#watch-a-pull-request) | No |
| [New task](../app/new-task.md) | Yes, in herdr, [Herdr Core](../app/new-task.md#herdr-core) or [Rex](../app/new-task.md#rex) |

## Set up

1. Install Antigravity CLI on the machine as `agy`.
2. In herdr, install herdr's Antigravity CLI hook, and again after every herdr update:

   ```sh
   herdr integration install antigravity-cli
   ```

   `collied doctor` checks it once `~/.gemini/antigravity-cli` (or `$ANTIGRAVITY_APP_DATA_DIR`) exists.

### Herdr Core and Rex

herdr's hook does nothing in Herdr Core (herdr-core) or Rex, so Chat needs `collied`'s own hook there. Add a `collie` entry in `~/.gemini/config/hooks.json` (or `$ANTIGRAVITY_CLI_CONFIG_DIR/hooks.json`), beside herdr's:

```json
{
  "collie": {
    "SessionStart": [
      { "type": "command", "command": "~/.cargo/bin/collied hook", "timeout": 5 }
    ],
    "PreInvocation": [
      { "type": "command", "command": "~/.cargo/bin/collied hook", "timeout": 5 }
    ]
  }
}
```

An Antigravity CLI session that was already running reports its conversation ID at its next prompt or new session. `collied doctor` checks the hook.

## Plan usage

Antigravity sends its 5-hour and weekly quota buckets to a custom status line command. To show them in [Usage](../app/usage.md), configure `statusLine` in `~/.gemini/antigravity-cli/settings.json` (or run `/statusline ~/.cargo/bin/collied statusline` in `agy`):

```json
{
  "statusLine": {
    "type": "command",
    "command": "~/.cargo/bin/collied statusline",
    "enabled": true,
    "stack_with_default": true
  }
}
```

With `stack_with_default` enabled, Antigravity keeps its built-in status line while piping quota updates to `collied`. `collied` keeps only the quota buckets (used percentage, reset time and window length) and the session's context window size; account fields such as email and workspace paths are discarded. If you already use a custom status line script, pipe its stdin into `~/.cargo/bin/collied statusline >/dev/null 2>&1 &` as for [Claude Code](../install/claude-code.md#plan-usage).

## Startup prompts

When a new task meets a startup prompt, such as sign-in or folder trust, it stops without sending your prompt. Open the session and answer the startup prompt in Terminal with the arrow keys, Return or Esc, then send your prompt.
