# Claude Code hook and plan usage

Both are optional; `collied doctor` checks them.

## The PermissionRequest hook

Add this to `~/.claude/settings.json` so approval alerts name the exact command or file:

```json
{
  "hooks": {
    "PermissionRequest": [
      { "hooks": [{ "type": "command", "command": "~/.cargo/bin/collied hook", "timeout": 5 }] }
    ]
  }
}
```

The hook only reports the call: Claude Code's dialog does not change.

## Plan usage

Only the status line command receives Claude Code's 5-hour and weekly limits. To show them in [Usage](../app/usage.md), add this line to your status line script after it reads stdin into `input`:

```sh
printf '%s' "$input" | ~/.cargo/bin/collied statusline >/dev/null 2>&1 &
```

`collied` keeps only the limits and each session's context window size, which the [context ring](../app/agents-tab.md#remaining-context) uses.

If `statusLine.command` is a single command, point it at a script that reads `input=$(cat)`, runs the line above, then pipes `printf '%s' "$input"` into the old command.

> [!NOTE]
> The limits come only with claude.ai Pro and Max plans, after a session's first reply. `collied doctor` shows when the tap last recorded them.
