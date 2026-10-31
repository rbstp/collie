# Updating

You can update the app and `collied` in either order: when they connect, they use the highest protocol revision both support. When they share none, the app says which one to update: see [Version mismatch](version-mismatch.md).

## collied

Download the latest [release](https://github.com/rbstp/collie/releases) and install it as on [macOS](../install/collied-macos.md) or [Linux](../install/collied-linux.md). To update a source build:

```sh
git pull
just setup
just collied-install
collied setup
```

`just collied-install` restarts the running service. `collied setup` keeps the node, the paired phones and the config, and offers to pair a phone again.

## The app

In the updated checkout, run:

```sh
just ios-run-device
```

## Desktop companions

- CollieBar: install the new release download, or run `git pull`, then `just mac-install`.
- CollieTray: run the new release's installer, or run `git pull`, then `just tray-install`.

## herdr

After you update herdr, install its Codex hook again with `herdr integration install codex`. `collied doctor` warns when the hook is missing or older.
