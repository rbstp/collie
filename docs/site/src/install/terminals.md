# Terminals

Terminals let you use plain shell panes from the phone. Because anything you type in a shell runs as a command, terminals are off by default, and only `collied.toml` on the machine can turn them on, never the phone.

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

3. Set a passcode on the phone. The phone must also be paired after you updated both the app and `collied`, because pairing records the phone's terminal key. A phone paired earlier shows "Pair this phone again to use terminals on this machine": [pair it again](pair.md).

## Use them

The app lists terminals under **Terminals**. When an agent exits, its pane becomes a shell and shows up there.

1. Open a terminal and unlock it with Face ID or the passcode. One unlock covers that terminal for 5 minutes.
2. The command field runs one line at a time. The [key strip](../app/keys.md) works as in an agent session.

`collied` refuses to write to a pane where an agent now runs. The audit log records each unlock and command, without the command's text.
