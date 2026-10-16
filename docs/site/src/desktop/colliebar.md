# CollieBar on macOS

CollieBar is a menu bar app that controls `collied` on the Mac.

## Downloads

Publisher-signed/notarized macOS 26+ Apple silicon release archives and a Homebrew cask are prepared by the [desktop release workflow](../reference/desktop-releases.md). Once the first validated release and tap update are published, unzip `CollieBar-VERSION-macos-arm64.zip` and move CollieBar to Applications, or install `rbstp/tap/colliebar`. Install the matching daemon first.

## Build from source

```sh
just mac-install
```

This builds CollieBar, signs it with your Developer ID and installs it to `~/Applications`. It is not notarized, which is only needed to run it on another Mac: see [Release](../reference/release.md#menu-bar-app-notarization-optional).

## Use

- The icon is bright while `collied` runs, dim when it is off, and shows a dot while an approval is pending.
- While the daemon is off, **Set Up…** opens its interactive setup in Terminal. If the daemon is missing, install it first. Complete setup in Terminal, then return to the menu. You can use it again to sign in after expiry; if the daemon is running, turn it off first. During Starting, wait for it to finish or use `collied stop` followed by `collied setup` in Terminal.
- The menu turns `collied` off and on, pairs a phone, lists the paired phones, opens the audit log and quits. Quitting also stops `collied`.
- **Open at Login** in its menu starts it at login. It is off by default.

Update it with `git pull` and `just mac-install`.
