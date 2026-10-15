# Updating

Update the app and `collied` together. When only one side is updated, the app tells you which one to update: see [Version mismatch](version-mismatch.md).

## collied

```sh
git pull
just setup
just collied-install
collied setup
```

`just collied-install` restarts the running service. `collied setup` keeps the node, the paired phones and the config, and offers to pair a phone again.

## The app

```sh
just ios-run-device
```

## Desktop companions

- CollieBar: `just mac-install`.
- CollieTray: `git pull`, then `just tray-install`.

## herdr

After updating herdr, install its Codex hook again: `herdr integration install codex`. `collied doctor` warns when the hook is missing or older.
