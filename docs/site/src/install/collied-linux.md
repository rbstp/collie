# collied on Linux

`collied` runs as a systemd user unit. Install the release download or build from source. Keep the phone and desktop on matching releases.

## Install a release download

The x86_64 download needs glibc 2.39 or later and systemd 255 or later. [Desktop release downloads](../reference/desktop-releases.md) covers updates, rollback and removal.

1. Download all assets of one [release](https://github.com/rbstp/collie/releases) into one folder and check them:

   ```sh
   sha256sum -c SHA256SUMS
   ```

2. Extract the daemon archive, and in its folder run:

   ```sh
   bash install.sh --replace
   ~/.cargo/bin/collied setup
   ```

## Install from source

```sh
git clone --recurse-submodules https://github.com/rbstp/collie
cd collie
just setup
just collied-install
collied setup
```

## What setup does

Setup runs the [macOS steps](collied-macos.md#what-setup-does), except:

- `~/.cargo/bin/collied` needs no signature.
- `collied login` prints the sign-in link and QR code, and opens no browser.
- `collied service install` writes a systemd user unit instead of a launchd agent.
- With `--direct-apns`, `collied` stores the APNs key as an encrypted systemd user credential (systemd 256 or later), or, where systemd cannot encrypt it, in a file only you can read. `collied doctor` says which.

## Running and logs

- A systemd user unit runs only while you are logged in. To keep it running after you log out, run `loginctl enable-linger`.
- Read the logs with `journalctl --user -u collied`. [Configuration](../operate/configuration.md) lists `collied`'s files.
- A system `tailscaled` plays no part in collie: collie's node runs beside it with its own keys and WireGuard port, and adds no TUN device or routes.

Next, [install the app](app.md) and [pair your phone](pair.md).
