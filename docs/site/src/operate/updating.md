# Updating

Update the app and `collied` in either order: they use the highest protocol revision both support. When they share none, the app says which one to update: see [Version mismatch](version-mismatch.md).

## collied

Download the latest [release](https://github.com/rbstp/collie/releases) and install it as on [macOS](../install/collied-macos.md) or [Linux](../install/collied-linux.md). To update a source build:

```sh
git pull
just setup
just collied-install
collied setup
```

`just collied-install` restarts the running service. `collied setup` keeps the node, paired phones and config, and offers to pair a phone again.

## The app

In the updated checkout, run `just ios-run-device`.

## Desktop companions

Update [CollieBar](../desktop/colliebar.md) and [CollieTray](../desktop/collietray.md) the way you installed them: from the new release, or with `git pull`, then `just mac-install` or `just tray-install`.

## herdr

After you update herdr, run `herdr integration install codex` again. `collied doctor` warns when the Codex hook is missing or older.
