# collied on macOS

`collied` runs as a launchd user agent from `~/.cargo/bin`. You can install a publisher-signed release download, which needs no developer toolchain and no Developer ID of your own, or build from source with your own Developer ID Application certificate. Keep the phone and desktop on matching releases.

## Install a release download

Download all assets of one [release](https://github.com/rbstp/collie/releases) into one folder and check them with `shasum -a 256 -c SHA256SUMS`. Then open the DMG and run its `install.sh` in Terminal, or extract the daemon tar archive and run, in its folder:

```sh
bash install.sh --replace
export PATH="$HOME/.cargo/bin:$PATH"
collied setup
```

[Desktop release downloads](../reference/desktop-releases.md) covers Homebrew, updates, rollback and removal.

## Install from source

1. Clone the repository with its submodules and build:

   ```sh
   git clone --recurse-submodules https://github.com/rbstp/collie
   cd collie
   just setup
   just collied-install
   ```

2. Choose **Set Up…** in CollieBar, or run the interactive setup:

   ```sh
   collied setup
   ```

## What setup does

`collied setup` runs these steps in order. It skips any step that is already done, so running it again resumes where it stopped:

1. **`collied login`**: the browser opens once. Sign in with the same personal Tailscale account you use inside Collie on the phone. If the browser does not open, use the printed link or QR code. You need no VPN profile.
2. **`collied service install`**: installs the launchd agent that runs `collied` at login. Setup installs it again, restarting `collied`, when it points at another binary or config, is stopped, or runs an older binary.
3. **`collied doctor`**: setup stops on a `fail` line. On a new machine, expect `warn` lines for peers, push, hooks, codex hook and plan usage, and one for reach until your phone is signed in to Tailscale.
4. **Push notifications**, when neither `[push.relay]` nor `[apns]` is configured yet. The default is the publisher relay, which needs no APNs key. For a self-built app, use `collied setup --direct-apns`. You can skip this step and come back later: see [Push notifications](push.md).
5. **`collied pair`**, offered at the end: see [Pair a phone](pair.md).

> [!NOTE]
> Setup needs a terminal, and `~/.cargo/bin/collied` must be the signed regular file, not a symlink. Setup never confirms a pairing for you: that stays your `y` on the machine. You can also run each step on its own.

## Files

`collied.toml`, the node state, paired phones and the audit log live in `~/Library/Application Support/collie/`. See [Configuration](../operate/configuration.md).

## Next

[Install the app](app.md), then [pair your phone](pair.md). The [menu bar app](../desktop/colliebar.md) is optional.
