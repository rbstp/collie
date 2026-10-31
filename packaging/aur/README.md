# AUR preparation

These recipes are not published. Their pinned v0.1.2 archives are packaging fixtures only: v0.1.2 setup accepts only `~/.cargo/bin/collied`, not `/usr/bin/collied`.

Before the first publication, update both recipes and their `.SRCINFO` to a verified matching desktop release that accepts `/usr/bin/collied`: see [Update the AUR recipes](../../docs/desktop-releases.md#update-the-aur-recipes). Never publish v0.1.2, or guessed release URLs or hashes.

Then, as your login user, `bash packaging/aur/install.sh` builds both prebuilt packages and installs them together with pacman. It needs Arch's `base-devel`, `git` and `sudo`, but no Rust or Go toolchain or AUR account. After pulling reviewed recipe updates, run it again, then restart the Omarchy shell and, if it is active, the user service.
