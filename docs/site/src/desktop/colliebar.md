# CollieBar on macOS

CollieBar is a menu bar app for `collied`. It needs macOS 26 or later on Apple silicon.

## Download

Install the matching `collied` first. Then download `CollieBar-VERSION-macos-arm64.zip` from the same [release](https://github.com/rbstp/collie/releases), unzip it and move CollieBar to Applications, or run:

```sh
brew install --cask rbstp/tap/colliebar
```

[Desktop release downloads](../reference/desktop-releases.md) covers updates and removal.

## Build from source

With [your own identifiers](../install/requirements.md#your-own-identifiers) set, run:

```sh
just mac-install
```

This signs CollieBar with your Developer ID and installs it to `~/Applications`, without notarizing it. To run it on another Mac, see [Release](../reference/release.md#menu-bar-app-notarization-optional). To update, run `git pull`, then `just mac-install` again.

## Use

- The icon is bright while `collied` runs, dim while it is off, and shows a dot for a pending approval.
- The menu shows the status and version, turns `collied` off and on, pairs a phone, lists the paired phones and opens the audit log. **Quit** also stops `collied`.
- **Set Up…** runs `collied setup` in Terminal, to set up the machine or sign in again. It shows only while `collied` is off, so turn it off first to sign in again.
- **Open at Login**, off by default, starts CollieBar when you log in.
- **Resume Rex Agents After Restart**, on by default, reopens your Claude Code, Codex, Copilot CLI and Antigravity CLI conversations in Rex after the Rex server restarts. It shows while `collied` runs.

## Resume Rex agents

When the Rex server restarts or the Mac reboots, Rex brings back your sessions, tabs and terminals, but each terminal starts as an empty shell. `collied` remembers which agent conversation ran in which terminal and starts it again beside the restored shell, in the folder it ran in, through your login shell, as if you had typed `claude --resume ID` (or `codex resume ID --no-daemon`, `copilot --resume=ID` or `agy --conversation ID`) in a new terminal: your shell's startup files, aliases and functions apply. Once the agent runs that conversation, `collied` closes the restored shell, so the agent takes its place in the window. If the agent fails to start, Rex closes its terminal as the agent exits, and the restored shell stays.

`collied` types nothing into your shell. It leaves alone a terminal where something runs, or whose screen changed after `collied` first looked at it (once its prompt had a moment to draw), so a terminal you started typing in keeps its shell. Each conversation reopens once, never one that already runs elsewhere, and `collied` stops trying 90 seconds after the Rex server started. A conversation you quit stays closed, unless you quit it less than 15 seconds before the server stopped. A conversation you never sent a prompt to has nothing to reopen and is skipped.

`collied` reopens a Codex conversation only if Codex ran with `--no-daemon`: under Codex's shared background server, `collied` cannot tell which terminal runs which conversation. An Antigravity CLI conversation can take its shell's place only once Antigravity CLI reports it to `collied`; until then, both stay. Your login shell must be zsh or bash.

When a resumed agent exits, its terminal closes, as a task's does. If `collied` starts more than 90 seconds after the Rex server, nothing is reopened.

To stop it, uncheck **Resume Rex Agents After Restart**; `collied` then forgets what it remembered. CollieTray has no such item, since collie follows Rex only on the Mac.
