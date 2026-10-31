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
