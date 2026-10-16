# Updating

The app and `collied` negotiate a common protocol revision. Revision 26 is the first baseline; update both once when coming from an older exact-match installation. Later updates can be installed in either order while their supported ranges overlap. When they no longer overlap, the app tells you which one to update: see [Version mismatch](version-mismatch.md).

## collied

Download the latest release for your operating system from [GitHub Releases](https://github.com/rbstp/collie/releases) and follow that release's installation instructions. For a source build:

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
