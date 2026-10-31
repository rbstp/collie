# collied on macOS

`collied` runs as a launchd user agent from `~/.cargo/bin`. Install the signed release download or build from source. Keep the phone and desktop on matching releases.

## Install a release download

The download needs macOS 26 or later. [Desktop release downloads](../reference/desktop-releases.md) covers Homebrew, updates, rollback and removal.

1. Download all assets of one [release](https://github.com/rbstp/collie/releases) into one folder and check them:

   ```sh
   shasum -a 256 -c SHA256SUMS
   ```

2. Open the DMG or extract the daemon archive, and in its folder run:

   ```sh
   bash install.sh --replace
   export PATH="$HOME/.cargo/bin:$PATH"
   collied setup
   ```

## Install from source

1. Clone the repository:

   ```sh
   git clone --recurse-submodules https://github.com/rbstp/collie
   cd collie
   just setup
   ```

2. Set [your own identifiers](requirements.md#your-own-identifiers), then build, sign and install `collied` with `just collied-install`.
3. Run `collied setup`, or choose **Set Up…** in [CollieBar](../desktop/colliebar.md).

## What setup does

`collied setup` needs a terminal and the signed regular file `~/.cargo/bin/collied`, not a symlink. It skips steps already done, so a rerun resumes where it stopped:

1. **`collied login`** signs in with your personal Tailscale account in the browser, or through the printed link or QR code.
2. **`collied service install`** installs the launchd agent that starts `collied` at login, or reinstalls it when it is stopped or outdated.
3. **`collied doctor`** stops setup on a `fail` line. Expect `warn` lines for parts not set up yet, such as `reach` before your phone signs in.
4. **Push notifications**, optional: setup offers the publisher relay. A self-built app needs direct APNs instead: run `collied setup --direct-apns` ([Push notifications](push.md)).
5. **`collied pair`**, offered last ([Pair a phone](pair.md)).

Next, [install the app](app.md) and [pair your phone](pair.md). [Configuration](../operate/configuration.md) lists where `collied` keeps files.
