# AUR preparation

These recipes are not published. The pinned v0.1.2 archives are packaging fixtures, not a supported AUR installation: their setup executable guard predates `/usr/bin/collied` support.

Before the first publication, update both recipes and `.SRCINFO` from a verified matching desktop release containing that support. Follow [the release update and acceptance procedure](../../docs/desktop-releases.md#update-the-aur-recipes). Do not publish v0.1.2 or guessed future release URLs/hashes.

After the matching release and recipe update, `bash packaging/aur/install.sh` builds both prebuilt packages as your login user and installs them together with pacman. It needs Arch `base-devel`, `git` and `sudo`, no Rust/Go toolchain or AUR account. Run it again after pulling reviewed recipe updates, then restart the user service if active and the Omarchy shell.
