# collied on macOS

`collied` runs as a launchd user agent from `~/.cargo/bin`. Install the publisher-signed release download, which needs no developer toolchain and no Developer ID of your own, or build from source and sign with your own Developer ID Application certificate. Keep the phone and the desktop on matching releases.

## Install a release download

The macOS download needs macOS 26 or later.

1. Download all assets of one [release](https://github.com/rbstp/collie/releases) into one folder and check them:

   ```sh
   shasum -a 256 -c SHA256SUMS
   ```

2. Open the DMG or extract the daemon tar archive, then run in Terminal, in its folder:

   ```sh
   bash install.sh --replace
   export PATH="$HOME/.cargo/bin:$PATH"
   collied setup
   ```

[Desktop release downloads](../reference/desktop-releases.md) covers Homebrew, updates, rollback and removal.

## Install from source

1. Clone the repository with its submodules:

   ```sh
   git clone --recurse-submodules https://github.com/rbstp/collie
   cd collie
   just setup
   ```

2. Set [your own identifiers](requirements.md#your-own-identifiers), then build, sign and install `collied`:

   ```sh
   just collied-install
   ```

3. Run `collied setup`, or choose **Set Up…** in CollieBar.

## What setup does

`collied setup` runs these steps in order. It skips any step that is already done, so running it again resumes where it stopped.

1. **`collied login`**: the browser opens once. Sign in with the personal Tailscale account you use inside Collie on the phone. If the browser does not open, use the printed link or QR code.
2. **`collied service install`**: installs the launchd agent that runs `collied` at login. Setup installs it again, restarting `collied`, when it is stopped or runs another binary, an older build or another config.
3. **`collied doctor`**: setup stops on a `fail` line. On a new machine, expect `warn` lines for what you have not set up yet, including `reach` until your phone signs in to Tailscale.
4. **Push notifications**, when neither `[push.relay]` nor `[apns]` is configured. Setup offers the publisher relay, which needs no APNs key but does not accept a self-built app; for a self-built app, run `collied setup --direct-apns`. You can skip this step: see [Push notifications](push.md).
5. **`collied pair`**, offered last: see [Pair a phone](pair.md).

> [!NOTE]
> Setup needs a terminal, and `~/.cargo/bin/collied` must be the signed regular file, not a symlink. Setup never confirms a pairing for you: you still answer `y` on the machine. You can also run each step on its own.

## Files

`collied.toml`, the node state, paired phones and the audit log live in `~/Library/Application Support/collie/`. See [Configuration](../operate/configuration.md).

## Next

[Install the app](app.md), then [pair your phone](pair.md). The [menu bar app](../desktop/colliebar.md) is optional.
