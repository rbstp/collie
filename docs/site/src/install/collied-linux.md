# collied on Linux

There is no package yet. `collied` is built from a checkout, unsigned, and runs as a systemd user unit.

## Install

1. Clone the repository with its submodules and build:

   ```sh
   git clone --recurse-submodules https://github.com/rbstp/collie
   cd collie
   just setup
   just collied-install
   ```

2. Run the interactive setup:

   ```sh
   collied setup
   ```

Setup runs the same steps as [on macOS](collied-macos.md#what-setup-does), with these differences:

- `collied login` tags the node `tag:collie-linux`, and `collied` refuses to run until the node has that tag.
- `collied service install` writes the systemd user unit `~/.config/systemd/user/collied.service`.
- Publisher relay mode needs no APNs key. With `collied setup --direct-apns` for a self-built app, the APNs key is stored as a systemd user credential, encrypted with the host key and sealed to the TPM2 when one is usable. Where `systemd-creds` cannot encrypt, it falls back to a file readable only by your user. `collied doctor` reports which one you have.

> [!WARNING]
> Setup stops when the user manager loads the unit from another file, applies a `collied.service.d` drop-in to it, or the unit pins another `XDG_DATA_HOME`. Remove the override and run setup again.

## Files and logs

- The data directory is `$XDG_DATA_HOME/collie`, else `~/.local/share/collie`. It holds `collied.toml`, the node state, paired phones and the audit log.
- Logs go to the user journal: `journalctl --user -u collied`.
- A systemd user unit runs while you have a session. `loginctl enable-linger` keeps it running without a login.

## A system Tailscale on the same machine

A system `tailscaled` is a separate node and plays no part in collie. Both run side by side: the embedded node has its own keys, uses a userspace network stack (no TUN device, no routes) and its own WireGuard port.

## Next

[Install the app](app.md), then [pair your phone](pair.md). On Omarchy, the [bar widget](../desktop/collietray.md) is optional.
