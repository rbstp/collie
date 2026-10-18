# CollieBar on macOS

CollieBar is a menu bar app for `collied` on the Mac. From its menu, you can turn `collied` off and on and pair a phone, and its icon shows when an approval is pending.

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

This builds CollieBar, signs it with your Developer ID and installs it to `~/Applications`. The build is not notarized, which matters only if you want to run it on another Mac: see [Release](../reference/release.md#menu-bar-app-notarization-optional).

## Use

- The menu status shows the running daemon's version, for example **collied 0.1.1: Running**. An older daemon that does not report its version shows **version unavailable**. When `collied` is stopped, the status shows **collied: Off**.
- The icon is bright while `collied` runs, dim when it is off, and shows a dot while an approval is pending.
- While `collied` is off, **Set Up…** opens its interactive setup in Terminal. If `collied` is missing, install it first. Complete setup in Terminal, then return to the menu. You can also use **Set Up…** to sign in again after your sign-in expires; if `collied` is running, turn it off first. While the status shows Starting, wait for it to finish, or run `collied stop` and then `collied setup` in Terminal.
- The menu turns `collied` off and on, pairs a phone, lists the paired phones, opens the audit log and quits. Quitting also stops `collied`.
- **Open at Login** in the menu starts CollieBar at login. It is off by default.

To update a source build, run `git pull` and `just mac-install`.
