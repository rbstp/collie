# collied on Linux

`collied` runs as a systemd user unit. You can install a release download, which needs no Rust toolchain or checkout, or build from source. Keep the phone and desktop on matching releases.

## Install a release download

The Linux x86_64 download needs glibc 2.39 or later and systemd 255 or later. Download all assets of one [release](https://github.com/rbstp/collie/releases) into one folder and check them with `sha256sum -c SHA256SUMS`. Then extract the daemon archive and run, in its folder:

```sh
bash install.sh --replace
~/.cargo/bin/collied setup
```

[Desktop release downloads](../reference/desktop-releases.md) covers updates, rollback and removal. Arch Linux (AUR) packages are not published yet; until they are, use the download or a source build.

## Install from source

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

- `collied login` signs in collie's own Tailscale node. Use the same personal account as Collie on the phone. You need no system Tailscale client.
- `collied service install` writes the systemd user unit `~/.config/systemd/user/collied.service`.
- The publisher relay needs no APNs key. With `collied setup --direct-apns` for a self-built app, `collied` stores the APNs key as a systemd user credential, encrypted with the host key and sealed to the TPM2 when one is usable. Where `systemd-creds` cannot encrypt, `collied` keeps the key in a file readable only by your user instead. `collied doctor` reports which one you have.

> [!WARNING]
> Setup stops when the user manager loads the unit from another file, applies a `collied.service.d` drop-in to it, or the unit pins another `XDG_DATA_HOME`. Remove the override and run setup again.

## Files and logs

- The data directory is `$XDG_DATA_HOME/collie`, else `~/.local/share/collie`. It holds `collied.toml`, the node state, paired phones and the audit log.
- Logs go to the user journal: `journalctl --user -u collied`.
- A systemd user unit runs only while you have a session. To keep it running without a login, run `loginctl enable-linger`.

## A system Tailscale on the same machine

A system `tailscaled` is a separate node and plays no part in collie. The two run side by side: collie's node has its own keys and its own WireGuard port, and it adds no TUN device and no routes.

## Next

[Install the app](app.md), then [pair your phone](pair.md). On Omarchy, the [bar widget](../desktop/collietray.md) is optional.
