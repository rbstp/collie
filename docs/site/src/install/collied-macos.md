# collied on macOS

`collied` is built from a checkout, signed with your Developer ID Application certificate and installed to `~/.cargo/bin`.

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

## What setup does

`collied setup` runs these steps in order and skips any step that is already done, so running it again resumes where it stopped:

1. **`collied login`**: open the printed link and sign in as a tag owner. The node becomes `tag:collie-mac`.
2. **`collied service install`**: installs the launchd agent that runs `collied` at login. It is installed again, restarting `collied`, when it points at another binary or config, is stopped, or runs an older binary.
3. **`collied doctor`**: setup stops on a `fail` line. On a new machine, `warn` lines for peers, push, hooks, codex hook and plan usage are expected, and one for reach until your phone is signed in to Tailscale.
4. **Push notifications**, when neither `[push.relay]` nor `[apns]` is configured yet. The default is the publisher relay, without an APNs key. For a self-built app use `collied setup --direct-apns`. You can skip this and come back later: see [Push notifications](push.md).
5. **`collied pair`**, offered at the end: see [Pair a phone](pair.md).

> [!NOTE]
> Setup needs a terminal, and `~/.cargo/bin/collied` must be the signed regular file, not a symlink. Setup never confirms a pairing for you: that stays your `y` on the machine. Each step also runs on its own.

## Files

`collied.toml`, the node state, paired phones and the audit log live in `~/Library/Application Support/collie/`. See [Configuration](../operate/configuration.md).

## Next

[Install the app](app.md), then [pair your phone](pair.md). The [menu bar app](../desktop/colliebar.md) is optional.
