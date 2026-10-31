# Desktop release downloads

Each `vMAJOR.MINOR.PATCH` release on [GitHub Releases](https://github.com/rbstp/collie/releases) carries `collied` for macOS and Linux, the CollieBar menu bar app and the CollieTray Omarchy widget, built from one source commit. Releases are immutable. Install the daemon and desktop companions from the same release.

## Assets and requirements

| Asset | Runs on |
|---|---|
| `collied-VERSION-macos-arm64.tar.gz` | macOS 26+ on Apple silicon; signed and notarized daemon |
| `collied-VERSION-macos-arm64.dmg` | The same, in a signed, notarized and stapled disk image |
| `CollieBar-VERSION-macos-arm64.zip` | macOS 26+ on Apple silicon; signed, notarized and stapled app |
| `collied-VERSION-linux-x86_64.tar.gz` | Linux x86_64 with glibc 2.39+, systemd 255+ and CA certificates |
| `CollieTray-VERSION-omarchy.tar.gz` | Omarchy 4 with Quickshell, qrencode and the matching `collied`; architecture-independent |
| `homebrew-VERSION.tar.gz` | `Formula/collied.rb` and `Casks/colliebar.rb` for [rbstp/homebrew-tap](https://github.com/rbstp/homebrew-tap) |
| `source.json`, `SHA256SUMS` | The tag and source commit, and the SHA-256 of every other asset |

Each daemon archive holds `THIRD_PARTY_NOTICES` and `release.json` with the version, source commit and supported protocol range. The Linux glibc and library floor is checked only on the Ubuntu 24.04 builder, not on every distribution.

Controlling agents needs herdr; the agent CLIs and the tailnet policy are separate prerequisites. There is no Linux arm64, Intel Mac or generic GNOME or KDE tray build.

## Install a daemon download

Download all assets of one release into one folder and verify them, with `shasum -a 256 -c SHA256SUMS` on macOS or `sha256sum -c SHA256SUMS` on Linux. Checksums catch damaged downloads; GitHub's HTTPS and the macOS signatures prove origin.

Open the DMG on macOS, or extract the daemon tar archive. In the daemon folder, as your login user:

```sh
bash install.sh --replace
export PATH="$HOME/.cargo/bin:$PATH"
collied setup
```

The installer replaces `~/.cargo/bin/collied` with a new file. On macOS, it first checks the strict code signature, identifier and Developer ID team, and Apple's [notarization](https://developer.apple.com/forums/thread/130560), which it looks up online even for the stapled DMG: allow network access, and do not remove quarantine or disable Gatekeeper. An existing launchd agent or systemd user unit keeps its arguments, environment and data paths, and restarts only if it was running; on Linux, the installer refuses an unrecognized `ExecStart` or a unit loaded from another file. A first install creates no service: `collied setup` does.

To roll back, run the previous release's installer with `--replace`, after checking that it supports the app's protocol. Node identity, configuration, pairings and audit data stay.

To uninstall, run `~/.cargo/bin/collied service uninstall` and delete `~/.cargo/bin/collied`; keep the data directory for the node identity and pairings. To remove CollieBar, turn off its Open at Login setting, quit it and delete it from Applications.

## Homebrew

```sh
brew install rbstp/tap/collied
collied-install --replace
brew install --cask rbstp/tap/colliebar
```

Do not use `brew services`: the formula defines no service. `collied-install --replace` copies the formula's binary to `~/.cargo/bin/collied`, where setup expects it and a Cellar cleanup cannot delete it, and refreshes an existing service; run it after every `brew upgrade collied`. After a first install, run `~/.cargo/bin/collied setup`: setup refuses Homebrew's copy. Uninstalling the formula leaves the copy and the service, so first run `~/.cargo/bin/collied service uninstall`, then delete the copy.

## CollieTray

Extract the Omarchy archive and run `bash install.sh`. It checks for Omarchy, Quickshell (`qs`) and `qrencode`, then installs the plugin to `~/.config/omarchy/plugins/rbstp.collie`, on the right of the bar. To update or roll back, run the installer from the release you want; an update restarts the Omarchy shell once. Configuration, pairings and the daemon's running state stay.

To disable it, run `omarchy plugin disable rbstp.collie`; to uninstall it, also delete `~/.config/omarchy/plugins/rbstp.collie`.

## Arch Linux and AUR preparation

The recipes in `packaging/aur/` are unpublished packaging fixtures pinned to v0.1.2, whose setup and service diagnostics accept only `~/.cargo/bin/collied`. Publish them only after [updating them](#update-the-aur-recipes) to a later release whose `collied` accepts `/usr/bin/collied`.

Once updated, they install from GitHub with no AUR account. With Arch's `base-devel` and `git` installed:

```sh
git clone --no-checkout https://github.com/rbstp/collie.git collie-packages
cd collie-packages
git checkout REVIEWED_RECIPE_COMMIT
bash packaging/aur/install.sh
/usr/bin/collied setup
collie-tray-activate
```

`REVIEWED_RECIPE_COMMIT` is the commit the publisher records after the release, once the recipes carry its verified checksums; never check out an unreleased `master` or the v0.1.2 recipes. Review the recipes and helpers first. `install.sh` packages both prebuilt archives with an unprivileged `makepkg --nodeps`, then installs them with their dependencies in one `sudo pacman -U` transaction that keeps the daemon and widget versions matched. Once both packages are on the AUR, `paru -S collie-tray` replaces these steps.

`collied-bin` has the Linux daemon's requirements. `collie-tray` needs the same `collied-bin` version, Quickshell, qrencode and an Omarchy 4 session with its bundled `qs.Commons` and `qs.Ui` QML modules. Other distributions and desktop shells are not supported. Run `collie-tray-activate` in your Omarchy session: it checks for Omarchy, which is not a package dependency, then links and enables the widget, restarting the shell and keeping the widget's bar placement and settings.

### Adopt an existing installation

Keep the same login user, `XDG_*_HOME` values and custom config, and do not run setup again:

```sh
bash packaging/aur/install.sh
collied-aur-adopt
collie-tray-activate
```

`collied-aur-adopt` backs up the user unit to `collied.service.before-aur` and changes only its `ExecStart` executable to `/usr/bin/collied`. Configuration, data, node identity, pairings and push settings stay; a running service restarts, a stopped one stays stopped. It accepts only an `ExecStart` of a quoted `collied` path, an optional `--config` and `run`, and refuses an existing backup, drop-ins and a unit loaded from another file. For those, change only the effective executable by hand (see `systemctl --user cat collied`), then run `systemctl --user daemon-reload` and `systemctl --user try-restart collied`.

Rename any `~/.cargo/bin/collied` (`type -a collied` finds older copies in `PATH`) so it cannot shadow the package binary; until then, call `/usr/bin/collied`. Do not run a download installer or `just collied-install` over the package installation. `collie-tray-activate` moves a copied plugin to the hidden directory `.rbstp.collie.before-aur`.

### Upgrade and remove

```sh
cd /path/to/collie-packages
git fetch origin
git checkout REVIEWED_RECIPE_COMMIT
bash packaging/aur/install.sh
systemctl --user try-restart collied.service
omarchy restart shell
```

After AUR publication, `paru -Syu` replaces the fetch and install steps. The shell restart loads the new widget, which Omarchy's plugin watcher misses behind the link. Check `/usr/bin/collied --version`, `systemctl --user show collied -p ExecStart` and the widget's version. To roll back, keep the daemon and widget on one version that the phone supports.

To remove the packages, uninstall the service first, in the same login session, or it restarts in a loop:

```sh
omarchy plugin disable rbstp.collie
rm ~/.config/omarchy/plugins/rbstp.collie
/usr/bin/collied service uninstall
sudo pacman -Rns collie-tray collied-bin
omarchy restart shell
```

Node data, pairings, configuration and adoption backups stay.

### Update the AUR recipes

After the release is public, download its `source.json`, `SHA256SUMS` and both Linux archives over HTTPS, verify the tag and source commit, and complete the [acceptance checks](#validation-before-public-rollout). For the first publication, check that `/usr/bin/collied setup` and `doctor` accept the package executable.

```sh
python3 scripts/desktop/aur.py /path/to/release-downloads
(cd packaging/aur/collied-bin && makepkg --printsrcinfo > .SRCINFO)
(cd packaging/aur/collie-tray && makepkg --printsrcinfo > .SRCINFO)
```

`aur.py` verifies both archives against `SHA256SUMS` and each other, then updates both recipes' versions and hashes, nothing else. Review `PKGBUILD`, the helpers and `.SRCINFO`, then, in a disposable Arch system, build with unprivileged `makepkg`, inspect the packages and test a fresh setup, adoption, upgrades in the same Omarchy session and removal. Only then publish each directory as its own AUR repository, the daemon first. Never publish guessed URLs or checksums.

## Publisher setup and release controls

Signing runs in the protected `desktop-release` environment, restricted to `master`, with `DESKTOP_CERT_P12` (base64 Developer ID Application certificate and private key for team RM3UT3MMSR), `DESKTOP_CERT_PASSWORD`, `APPLE_KEY_P8` (base64 App Store Connect API key allowed to notarize), `APPLE_KEY_ID` and `APPLE_ISSUER_ID`. The repository's Apple API secrets can be reused; never use the iOS distribution certificate as the Developer ID.

The workflow signs only for a `master` dispatch or a pull request merged into `master`, and uses no shared build cache. Before any code of the release commit runs, trusted `master` code checks that the commit is on `master` and any existing tag points at it, creating a missing tag. TestFlight dispatches the workflow from `master`: the signing environment refuses a merged pull request's own ref, and a tag pushed with `GITHUB_TOKEN` starts no workflow.

The `update-homebrew` job needs a `homebrew-tap` environment in `rbstp/collie`, restricted to `master`, holding `HOMEBREW_TAP_TOKEN`: a fine-grained personal access token owned by the tap publisher, limited to `rbstp/homebrew-tap`, with **Contents: Read and write**, no workflow permission and no Apple credential. Enter it at the prompt, never in a command argument or committed file:

```sh
gh secret set HOMEBREW_TAP_TOKEN --repo rbstp/collie --env homebrew-tap
```

Only this job sees the token, and it runs trusted `master` code, never the release source. It verifies the published release, then commits only the formula's and cask's new versions, URLs and SHA-256 values; recipe behavior changes need a reviewed tap pull request. It stops without forcing on an older release, a different file at the same version or a concurrent tap change. If the secret is missing or the update conflicts, the job fails after publication: fix the cause and rerun only that job, or merge the two generated files by hand after review.

For a desktop-only release, dispatch from `master` with a new tag and its full `master` commit SHA:

```sh
gh workflow run desktop-release.yml --ref master -f tag=v0.3.0 -f source_sha=FULL_COMMIT_SHA
```

To retry, rerun the failed jobs or dispatch the same tag and SHA. Tags never move; a retry only verifies a published release, and completes an interrupted draft without replacing its signed packages. Do not edit assets or move tags during a release.

## Validation before public rollout

Local packaging and CI smoke tests are not clean-machine acceptance. Before advertising the first release, record:

- Real Developer ID signing, notarization and stapling, and a browser download that passes Gatekeeper on a clean supported Mac without developer tools or credentials.
- On a clean Mac: binary and Homebrew install, start, stop, upgrade, rollback, uninstall and CollieBar service discovery, with running and stopped services and custom config paths, keeping identity and pairings.
- On a clean Linux machine at the documented floor: installation, dynamic dependencies, systemd start, stop, update and uninstall, pairing and agent control. On Omarchy 4, `just tray-test` and the downloaded widget's status, QR pairing and controls across an upgrade.
- The Homebrew files generated from the actual release assets, published to the tap, and formula and cask installation.
