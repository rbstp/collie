# CollieTray on Linux

CollieTray is a bar widget for [Omarchy](https://omarchy.org), with the same icon states and menu as [CollieBar](colliebar.md). It needs Omarchy 4 and `qrencode`.

## Install

```sh
just tray-install
```

This copies the widget to `~/.config/omarchy/plugins/rbstp.collie` and puts it on the right of the bar. It loads with the shell at every login while it is enabled.

## Use

- The icon is bright while `collied` runs, dim when it is off, and shows a dot while an approval is pending.
- The menu turns `collied` off and on, pairs a phone, lists the paired phones, opens the audit log and quits.
- **Quit** stops `collied` and takes the widget off the bar. Bring it back with:

  ```sh
  omarchy plugin enable rbstp.collie
  ```

Update it with `git pull` and `just tray-install`.
