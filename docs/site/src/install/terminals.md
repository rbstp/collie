# Terminals

Terminals let you use plain shell panes from the phone. Anything typed there runs as a command, so they are off by default, and only `collied.toml` on the machine turns them on.

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

3. Set a passcode on the phone before pairing: the terminal key that pairing records exists only while the phone has a passcode. If a terminal says "Pair this phone again to use terminals on this machine", [pair it again](pair.md).

## Use them

**Terminals** in the app lists the machine's shell panes, including the pane of an agent that exited.

1. Open a terminal and unlock it with Face ID or the passcode. One unlock covers that terminal for 5 minutes, or until the app goes to the background.
2. The command field runs one line at a time. The [key strip](../app/keys.md) works as in an agent session.

`collied` refuses to write to a pane where an agent now runs. The audit log records each unlock and command, without the command's text.
