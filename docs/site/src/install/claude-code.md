# Claude Code plan usage

The status line tap is optional; `collied doctor` checks it.

## The PermissionRequest hook

You no longer need Claude Code's PermissionRequest hook: `collied` names the command or file from the prompt on screen. If `~/.claude/settings.json` still runs `collied hook` under `PermissionRequest`, that entry does nothing, and you can remove it.

## Plan usage

Only the status line command receives Claude Code's 5-hour and weekly limits. To show them in [Usage](../app/usage.md), add this line to your status line script after it reads stdin into `input`:

```sh
printf '%s' "$input" | ~/.cargo/bin/collied statusline >/dev/null 2>&1 &
```

`collied` keeps only the limits and each session's context window size, which [remaining context](../app/agents-tab.md#remaining-context) uses.

If `statusLine.command` is a single command, point it at a script that reads `input=$(cat)`, runs the line above, then pipes `printf '%s' "$input"` into the old command.

> [!NOTE]
> The limits come only with claude.ai Pro and Max plans, after a session's first reply. `collied doctor` shows when the tap last recorded them.
