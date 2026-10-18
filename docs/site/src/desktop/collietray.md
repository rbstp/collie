# CollieTray on Linux

CollieTray is a bar widget for `collied` on [Omarchy](https://omarchy.org), with the same icon states and menu as [CollieBar](colliebar.md). It needs Omarchy 4 and `qrencode`. CollieTray is an Omarchy plugin, so it does not work as a generic GNOME or KDE tray icon.

## Download

Install the matching `collied` first. Then download `CollieTray-VERSION-omarchy.tar.gz` from the same [release](https://github.com/rbstp/collie/releases), extract it and run `bash install.sh`. The installer checks for Omarchy 4, Quickshell and `qrencode`, validates the plugin and enables it on the bar.

To upgrade or roll back, run the installer from the release you want. To uninstall, run `omarchy plugin disable rbstp.collie` and remove `~/.config/omarchy/plugins/rbstp.collie`. Uninstalling leaves the daemon's data and running state alone. [Desktop release downloads](../reference/desktop-releases.md) has more detail.

Arch Linux (AUR) packages are not published yet. [Arch Linux and AUR preparation](../reference/desktop-releases.md#arch-linux-and-aur-preparation) describes them.

## Build from source

```sh
just tray-install
```

This copies the widget to `~/.config/omarchy/plugins/rbstp.collie` and puts it on the right of the bar. While it is enabled, it loads with the shell at every login.

## Use

- The menu status shows the running daemon's version, for example **collied 0.1.1: Running**. An older daemon that does not report its version shows **version unavailable**. When `collied` is stopped, the status shows **collied: Off**.
- The icon is bright while `collied` runs, dim when it is off, and shows a dot while an approval is pending.
- The menu turns `collied` off and on, pairs a phone, lists the paired phones, opens the audit log and quits.
- **Quit** stops `collied` and takes the widget off the bar. To bring it back, run:

  ```sh
  omarchy plugin enable rbstp.collie
  ```

To update a source build, run `git pull` and `just tray-install`.
