# Terminals

Terminals let you use plain shells from the phone. Anything typed there runs as a command, so they are off by default, and only `collied.toml` on the machine turns them on.

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

**Terminals** in the app lists the machine's shells: herdr's shell panes, including the pane of an agent that exited, and Rex terminals that run a shell, each under its Rex session. A Rex terminal stays listed while a command runs in it. Rex terminals on other machines, Rex terminals that run a program without a shell, such as a session started with `rex new -- top`, a Rex terminal whose shell keeps an agent in front, such as `sh -c 'claude'`, once that agent reports its status, and Herdr Core's shells do not appear. A terminal can also stay out of the list while Rex holds an agent's status for a shell in it other than your login shell; the agent's next report, its exit, or a new shell brings it back.

1. Open a terminal and unlock it with Face ID or the passcode. One unlock covers that terminal for 5 minutes, or until the app goes to the background. The terminal's screen then goes blank until you unlock it again.
2. Tap the terminal and type, as in an agent's [Terminal view](../app/chat-terminal.md#terminal): every key goes to the shell, and Return runs the line. The [key strip](../app/keys.md) works as in an agent session. In Rex, a line or typed text arrives as a paste, so a program that turned bracketed paste on, such as an editor in normal mode or many REPLs, takes it as text rather than as commands; herdr types it key by key.

`collied` refuses to write to a pane or Rex terminal where an agent now runs. The audit log records each unlock and what you send, never its text.
