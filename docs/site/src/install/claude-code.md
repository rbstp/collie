# Claude Code hook and plan usage

The hooks and the status line tap are optional; `collied doctor` checks them.

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

## The PreToolUse hook

To have `collied` [stop agents at the plan limit](../agents/claude-code.md#plan-limits), run the same command before each tool call:

```json
{
  "hooks": {
    "PreToolUse": [
      { "hooks": [{ "type": "command", "command": "~/.cargo/bin/collied hook", "timeout": 5 }] }
    ]
  }
}
```

Put it beside `PermissionRequest` in the same `hooks` object. The hook asks `collied` whether the plan is at its limit and stops the agent only while **Stop at the plan limit** is on and a limit is reached. When `collied` is off or does not answer within 2 seconds, the agent goes on. It needs the status line tap below, which tells `collied` that a limit is reached.

## Plan usage

Only the status line command receives Claude Code's 5-hour and weekly limits. To show them in [Usage](../app/usage.md), add this line to your status line script after it reads stdin into `input`:

```sh
printf '%s' "$input" | ~/.cargo/bin/collied statusline >/dev/null 2>&1 &
```

`collied` keeps only the limits and each session's context window size, which [remaining context](../app/agents-tab.md#remaining-context) uses.

If `statusLine.command` is a single command, point it at a script that reads `input=$(cat)`, runs the line above, then pipes `printf '%s' "$input"` into the old command.

> [!NOTE]
> The limits come only with claude.ai Pro and Max plans, after a session's first reply. `collied doctor` shows when the tap last recorded them.
