# CollieBar on macOS

CollieBar is a menu bar app that controls `collied` on the Mac.

## Install

```sh
just mac-install
```

This builds CollieBar, signs it with your Developer ID and installs it to `~/Applications`. It is not notarized, which is only needed to run it on another Mac: see [Release](../reference/release.md#menu-bar-app-notarization-optional).

## Use

- The icon is bright while `collied` runs, dim when it is off, and shows a dot while an approval is pending.
- The menu turns `collied` off and on, pairs a phone, lists the paired phones, opens the audit log and quits. Quitting also stops `collied`.
- **Open at Login** in its menu starts it at login. It is off by default.

Update it with `git pull` and `just mac-install`.
