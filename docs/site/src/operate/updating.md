# Updating

You can update the app and `collied` in either order: when they connect, they agree on a protocol revision both support. If you are updating from a release older than protocol revision 26, which needed both sides to match exactly, update both once. When the two drift too far apart, the app tells you which one to update: see [Version mismatch](version-mismatch.md).

## collied

Download the latest release for your operating system from [GitHub Releases](https://github.com/rbstp/collie/releases) and follow that release's installation instructions. To update a source build:

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

After you update herdr, install its Codex hook again with `herdr integration install codex`. `collied doctor` warns when the hook is missing or older.
