# Terminals

Terminals are plain shell panes you can use from the phone. Shell input is command execution, so they are off by default, and only `collied.toml` on the machine can turn them on, never the phone.

## Turn them on

1. Add this to `collied.toml`:

   ```toml
   [terminals]
   enabled = true
   ```

2. Restart `collied`:

   ```sh
   collied stop
   collied start
   ```

3. The phone must have a passcode, and must be paired after both the app and `collied` were updated, because pairing records the phone's terminal key. A phone paired earlier shows "Pair this phone again to use terminals on this machine": [pair it again](pair.md).

## Use them

Terminals are listed under **Terminals** in the app. When an agent exits, its pane becomes a shell and shows up there.

1. Open a terminal and unlock it with Face ID or the passcode. One unlock covers that terminal for 5 minutes.
2. The command field runs one line at a time. The [key strip](../app/keys.md) works as in an agent session.

`collied` refuses to write to a pane where an agent now runs, and the audit log records each unlock and command without its text.
