# CollieBar on macOS

CollieBar is a menu bar app for `collied` on the Mac. Its icon shows whether `collied` runs and whether an approval is pending, and its menu turns `collied` off and on and pairs a phone.

## Download

Install the matching `collied` first. Then download `CollieBar-VERSION-macos-arm64.zip` from the same [release](https://github.com/rbstp/collie/releases), unzip it and move CollieBar to Applications, or install it with Homebrew:

```sh
brew install --cask rbstp/tap/colliebar
```

The release app is signed and notarized, and needs macOS 26 or later on Apple silicon. [Desktop release downloads](../reference/desktop-releases.md) covers updates and removal.

## Build from source

```sh
just mac-install
```

With [your own identifiers](../install/requirements.md#your-own-identifiers) set, this builds CollieBar, signs it with your Developer ID and installs it to `~/Applications`. The build is not notarized, which matters only on another Mac: see [Release](../reference/release.md#menu-bar-app-notarization-optional). To update it, run `git pull`, then `just mac-install` again.

## Use

- The menu shows the running daemon's version, for example **collied 0.1.1: Running**, or **collied: Running (version unavailable)** for an older daemon that does not report it. A stopped daemon shows **collied: Off**.
- The icon is bright while `collied` runs, dim while it is off, and shows a dot while an approval is pending.
- The menu turns `collied` off and on, pairs a phone, lists the paired phones and opens the audit log. **Quit** also stops `collied`.
- While `collied` is off, **Set Up…** runs `collied setup` in Terminal, to set up the machine or to sign in again after your sign-in expires. Install `collied` first, and turn it off before you sign in again. While the status shows **Starting…**, Set Up… is unavailable: wait, or run `collied stop`, then `collied setup`, in Terminal.
- **Open at Login**, off by default, starts CollieBar when you log in.
