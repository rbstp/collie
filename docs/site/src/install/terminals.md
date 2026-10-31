# Terminals

Terminals let you use plain shell panes from the phone. Anything you type in a shell runs as a command, so terminals are off by default, and only `collied.toml` on the machine can turn them on.

## Turn them on

1. Add this to [`collied.toml`](../operate/configuration.md#colliedtoml):

   ```toml
   [terminals]
   enabled = true
   ```

2. Restart `collied`, which reads the setting only at start:

   ```sh
   collied stop
   collied start
   ```

3. Give the phone a passcode before you pair it: pairing records the phone's terminal key, which exists only while the phone has a passcode. If a terminal says "Pair this phone again to use terminals on this machine", [pair it again](pair.md).

## Use them

The app lists the machine's shell panes under **Terminals**, including the pane of an agent that exited.

1. Open a terminal and unlock it with Face ID or the passcode. One unlock covers that terminal for 5 minutes, or until the app goes to the background.
2. The command field runs one line at a time. The [key strip](../app/keys.md) works as in an agent session.

`collied` refuses to write to a pane where an agent now runs. The audit log records each unlock and command, without the command's text.
