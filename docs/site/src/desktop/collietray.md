# CollieTray on Linux

CollieTray is a bar widget for `collied` on [Omarchy](https://omarchy.org), with the icon and menu of [CollieBar](colliebar.md) except **Set Up…** and **Open at Login**. It needs Omarchy 4, Quickshell and `qrencode`. As an Omarchy plugin, it does not work as a GNOME or KDE tray icon.

## Download

Install the matching `collied` first. Then download `CollieTray-VERSION-omarchy.tar.gz` from the same [release](https://github.com/rbstp/collie/releases), extract it and run `bash install.sh`. The installer checks for those requirements, validates the plugin and puts it on the right of the bar.

To upgrade or roll back, run the installer from the release you want. To uninstall, run `omarchy plugin disable rbstp.collie` and remove `~/.config/omarchy/plugins/rbstp.collie`; the daemon's data and running state stay as they are. [Desktop release downloads](../reference/desktop-releases.md) has more detail, including the [Arch Linux (AUR) packages](../reference/desktop-releases.md#arch-linux-and-aur-preparation), not published yet.

## Build from source

```sh
just tray-install
```

This copies the widget to `~/.config/omarchy/plugins/rbstp.collie` and puts it on the right of the bar. To update it, run `git pull`, then `just tray-install` again.

## Use

- The menu shows the running daemon's version, for example **collied 0.1.1: Running**, or **collied: Running (version unavailable)** for an older daemon that does not report it. A stopped daemon shows **collied: Off**.
- The icon is bright while `collied` runs, dim while it is off, and shows a dot while an approval is pending.
- The menu turns `collied` off and on, pairs a phone, lists the paired phones and opens the audit log. **Quit** also stops `collied`, and takes the widget off the bar. To bring it back, run:

  ```sh
  omarchy plugin enable rbstp.collie
  ```

- To set up the machine or sign in again, run `collied setup` in a terminal.
- While enabled, the widget loads with the shell at every login.
