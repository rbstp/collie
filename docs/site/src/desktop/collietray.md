# CollieTray on Linux

CollieTray is a bar widget for [Omarchy](https://omarchy.org), with the same icon states and menu as [CollieBar](colliebar.md). It needs Omarchy 4 and `qrencode`.

## Download

The [desktop release workflow](../reference/desktop-releases.md) prepares a complete architecture-independent `CollieTray-VERSION-omarchy.tar.gz`. Once a validated release is available, extract it and run `bash install.sh`. The installer checks Omarchy 4/Quickshell commands and `qrencode`, validates the plugin and enables it on the bar. Install the matching collied daemon separately. This is an Omarchy plugin; GNOME/KDE generic tray support is not provided.

Upgrade or roll back using the installer from the selected release. To uninstall, run `omarchy plugin disable rbstp.collie` and remove `~/.config/omarchy/plugins/rbstp.collie`. Daemon data and running state are retained.

## Build from source

```sh
just tray-install
```

This copies the widget to `~/.config/omarchy/plugins/rbstp.collie` and puts it on the right of the bar. It loads with the shell at every login while it is enabled.

## Use

- The menu status shows the running daemon's version, for example **collied 0.1.1: Running**. An older daemon that does not report its version shows **version unavailable**. When stopped, it shows **collied: Off**.
- The icon is bright while `collied` runs, dim when it is off, and shows a dot while an approval is pending.
- The menu turns `collied` off and on, pairs a phone, lists the paired phones, opens the audit log and quits.
- **Quit** stops `collied` and takes the widget off the bar. Bring it back with:

  ```sh
  omarchy plugin enable rbstp.collie
  ```

Update it with `git pull` and `just tray-install`.
