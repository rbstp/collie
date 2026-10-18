# Claude Code hook and plan usage

Two optional additions to Claude Code give collie more detail: the hook names the exact command in approval alerts, and the status line tap shares plan usage. `collied doctor` checks each one.

## The PermissionRequest hook

With the hook, an approval alert names the exact command or file. Without it, the alert shows the tool call as `collied` reads it from the screen. The hook only reports the call to `collied`, and Claude Code's own dialog does not change.

Add it to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "PermissionRequest": [
      { "hooks": [{ "type": "command", "command": "~/.cargo/bin/collied hook", "timeout": 5 }] }
    ]
  }
}
```

## Plan usage

Claude Code gives the plan's 5-hour and weekly limits only to the status line command. To share them with collie, add this line to your status line script, after it reads its input into `input` (for example with `input=$(cat)`):

```sh
printf '%s' "$input" | ~/.cargo/bin/collied statusline >/dev/null 2>&1 &
```

The line prints nothing, so your status line looks the same. `collied` records the limits and each session's context window size, so the context ring uses the exact window. It drops everything else in the status line input.

If your `statusLine.command` is a single command rather than a script, point it at a small script that reads `input=$(cat)`, runs the line above, then pipes `printf '%s' "$input"` into your old command.

> [!NOTE]
> Claude Code reports the limits only for claude.ai Pro and Max plans, after the session's first reply. `collied doctor` reports when the tap last recorded them.

The limits show in [Usage](../app/usage.md) and on the [Apple Watch](../notifications/apple-watch.md).
