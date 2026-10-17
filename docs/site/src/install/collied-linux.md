# collied on Linux

`collied` runs as a systemd user unit. Linux x86_64 download packaging targets glibc 2.39+ and systemd 255+: see [Desktop release downloads](../reference/desktop-releases.md). Published daemon archives include `install.sh`; extract an archive and run `bash install.sh --replace`, then `~/.cargo/bin/collied setup`. A Rust toolchain or checkout is not required for those downloads. Keep the phone and desktop on matching releases.

AUR packaging is being prepared for Arch Linux x86_64 and Omarchy 4. The recipes are not published; their v0.1.2 downloads are test fixtures because that binary predates package-path setup support. After a matching release containing that support, the GitHub recipe installer will install both components without an AUR account or Rust/Go builds. A later AUR publication will also allow `paru -S collie-tray`. See [AUR preparation and activation](../reference/desktop-releases.md#arch-linux-and-aur-preparation) for first setup, adoption of an existing service and upgrades. Do not use the fixture recipes for a production install.

The source installation remains available below.

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

- `collied login` signs in Collie's separate embedded user-owned node. Use the same personal account as Collie on the phone; no system Tailscale client is required.
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
