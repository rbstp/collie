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
- **Resume Rex Agents After Restart**, on by default, reopens your Rex agents after the Rex server restarts. It shows while `collied` runs.

## Resume Rex agents

When the Rex server restarts or the Mac reboots, Rex brings back your sessions, tabs and terminals, but each terminal starts as an empty shell. `collied` remembers which Claude Code, Codex or Antigravity CLI conversation ran in which terminal, and types the agent's own resume command into that terminal once its shell shows an empty prompt:

- `claude --resume ID` for Claude Code
- `codex resume ID` for Codex
- `agy --conversation ID` for Antigravity CLI

Each conversation reopens once, in the terminal it ran in. `collied` types nothing into a terminal whose shell is in another folder, where you started typing, or where the conversation already runs, and it stops trying 90 seconds after the Rex server started. A conversation you quit stays closed, unless you quit it less than 15 seconds before the server stopped. Copilot CLI conversations are not reopened: `collied` does not learn their ID in Rex.

The command runs through your shell, so your aliases and functions for `claude`, `codex` and `agy` apply. If `collied` was off while Rex restarted, nothing is typed.

To stop it, uncheck **Resume Rex Agents After Restart**; `collied` then forgets what it remembered. CollieTray has no such item, since collie follows Rex only on the Mac.
