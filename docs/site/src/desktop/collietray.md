# CollieTray on Linux

CollieTray is a widget for `collied` on the right of the [Omarchy](https://omarchy.org) bar. It needs Omarchy 4, Quickshell and `qrencode`; it is not a GNOME or KDE tray icon.

## Download

Install the matching `collied` first. Then download `CollieTray-VERSION-omarchy.tar.gz` from the same [release](https://github.com/rbstp/collie/releases), extract it and run `bash install.sh`. To upgrade or roll back, run the installer from another release.

To uninstall, run `omarchy plugin disable rbstp.collie` and remove `~/.config/omarchy/plugins/rbstp.collie`; this leaves `collied` alone. [Desktop release downloads](../reference/desktop-releases.md) covers the rest, including the unpublished [AUR packages](../reference/desktop-releases.md#arch-linux-and-aur-preparation).

## Build from source

```sh
just tray-install
```

To update, run `git pull`, then `just tray-install` again.

## Use

- The icon is bright while `collied` runs, dim while it is off, and shows a dot for a pending approval.
- The menu shows the status and version, turns `collied` off and on, pairs a phone, lists the paired phones and opens the audit log. **Quit** also stops `collied` and takes the widget off the bar; `omarchy plugin enable rbstp.collie` brings it back.
- To set up the machine or sign in again, run `collied setup` in a terminal.
- While enabled, it loads at every login.
