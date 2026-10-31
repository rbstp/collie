# Desktop release downloads

Each `vMAJOR.MINOR.PATCH` release on [GitHub Releases](https://github.com/rbstp/collie/releases) carries `collied` for macOS and Linux, the CollieBar menu bar app and the CollieTray Omarchy widget, all built from one source commit. Releases are immutable. Install the daemon and the desktop companions from the same release.

## Assets and requirements

| Asset | Runs on |
|---|---|
| `collied-VERSION-macos-arm64.tar.gz` | macOS 26+ on Apple silicon; signed and notarized daemon |
| `collied-VERSION-macos-arm64.dmg` | The same daemon and installer in a signed, notarized and stapled disk image |
| `CollieBar-VERSION-macos-arm64.zip` | macOS 26+ on Apple silicon; signed, notarized and stapled app |
| `collied-VERSION-linux-x86_64.tar.gz` | Linux x86_64 with glibc 2.39+, systemd 255+ and CA certificates |
| `CollieTray-VERSION-omarchy.tar.gz` | Omarchy 4 with Quickshell, qrencode and the matching `collied`; architecture-independent |
| `homebrew-VERSION.tar.gz` | `Formula/collied.rb` and `Casks/colliebar.rb` for [rbstp/homebrew-tap](https://github.com/rbstp/homebrew-tap) |
| `source.json`, `SHA256SUMS` | The tag and source commit, and the SHA-256 of every other asset |

Each daemon archive also holds `THIRD_PARTY_NOTICES` and `release.json`, with the version, source commit and supported protocol range; on Linux, `release.json` also lists the highest GLIBC symbol version and every `DT_NEEDED` library. The Ubuntu 24.04 builder runs `--version`, `--help` and `ldd`, and fails on an unresolved library or a GLIBC symbol above 2.39. That proves the builder's runtime, not every distribution.

Controlling agents needs herdr; the agent CLIs and the tailnet policy are separate prerequisites. There is no Linux arm64, Intel Mac or generic GNOME or KDE tray build.

## Install a daemon download

Download all assets of one release into one folder and verify them there, with `shasum -a 256 -c SHA256SUMS` on macOS or `sha256sum -c SHA256SUMS` on Linux. Checksums catch damaged downloads; GitHub's HTTPS and the macOS signatures prove their origin.

On macOS, open the DMG and run its `install.sh` in Terminal, or extract the daemon tar archive. On Linux, extract the daemon tar archive. In the daemon folder:

```sh
bash install.sh --replace
export PATH="$HOME/.cargo/bin:$PATH"
collied setup
```

The installer replaces `~/.cargo/bin/collied` with a new file and needs no Xcode, Rust, publisher credential or `sudo`. On macOS it first checks the strict code signature, the signing identifier `dev.rbstp.collied`, Developer ID team RM3UT3MMSR and Apple's `notarized` requirement, the [notarization check](https://developer.apple.com/forums/thread/130560) for a standalone executable. That check looks up the ticket online, so allow network access even when installing from the stapled DMG. Do not remove quarantine or disable Gatekeeper.

An existing launchd agent or systemd user unit keeps its configuration arguments, environment and data paths; the installer changes only its executable. A loaded launchd agent or an active systemd unit restarts; a stopped service stays stopped. A first install creates no service: `collied setup` does. On Linux, the installer refuses an unrecognized `ExecStart` or a unit loaded from another file.

To roll back, run the previous release's installer with `--replace`, after checking that it supports the app's protocol. Node identity, configuration, pairings and audit data stay.

To uninstall, run `~/.cargo/bin/collied service uninstall` and delete `~/.cargo/bin/collied`; keep the data directory to keep the node identity and pairings. To remove CollieBar, turn off its Open at Login setting, quit it and delete it from Applications.

## Homebrew

```sh
brew install rbstp/tap/collied
collied-install --replace
brew install --cask rbstp/tap/colliebar
```

The formula installs `collied` and `collied-install` but runs no install hook and defines no `brew services` service, so do not use `brew services`. `collied-install --replace` copies the formula's binary to `~/.cargo/bin/collied`, the regular file setup expects, and refreshes an existing service, so a Cellar cleanup cannot delete the service's executable. After a first install, run `~/.cargo/bin/collied setup` (setup refuses to run from Homebrew's copy); after every `brew upgrade collied`, run `collied-install --replace`. Uninstalling the formula leaves the copy and the service: run `~/.cargo/bin/collied service uninstall` and delete the copy yourself. The cask installs CollieBar to Applications; CollieBar finds the daemon through its launchd plist.

## CollieTray

Extract the Omarchy archive from the same release as the daemon and run `bash install.sh`. It checks for Omarchy's plugin commands, Quickshell (`qs`) and `qrencode`, validates the plugin, copies it to `~/.config/omarchy/plugins/rbstp.collie` and puts it on the right of the bar. To update or roll back, run the installer from the release you want; an update restarts the Omarchy shell once. Configuration, pairings and the daemon's running state stay as they are.

To disable it, run `omarchy plugin disable rbstp.collie`; to uninstall it, also delete `~/.config/omarchy/plugins/rbstp.collie`.

## Arch Linux and AUR preparation

The recipes in `packaging/aur/` are not published. They pin v0.1.2 archives as packaging fixtures only, because v0.1.2 setup and service diagnostics accept only `~/.cargo/bin/collied`. Publish them only after [updating them](#update-the-aur-recipes) to a later release whose `collied` accepts `/usr/bin/collied`.

Once updated, the recipes install from GitHub with no AUR account. Install Arch's `base-devel` and `git`, then:

```sh
git clone --no-checkout https://github.com/rbstp/collie.git collie-packages
cd collie-packages
git checkout REVIEWED_RECIPE_COMMIT
bash packaging/aur/install.sh
/usr/bin/collied setup
collie-tray-activate
```

`REVIEWED_RECIPE_COMMIT` is the packaging commit the publisher records after verifying the release and updating the recipes; recipes with the final checksums can only be committed after the release tag. Do not check out an unreleased `master` or the v0.1.2 recipes.

`install.sh` packages both prebuilt archives with an unprivileged `makepkg --nodeps`, compiling nothing, then installs both packages and their repository dependencies in one `sudo pacman -U` transaction, so the exact daemon and widget version match holds during upgrades. Review the recipes and helpers first. Once both packages are on the AUR, `paru -S collie-tray` replaces these steps.

`collied-bin` supports Arch Linux x86_64 with glibc 2.39+, systemd 255+ and CA certificates. `collie-tray` requires the same `collied-bin` version, Quickshell, qrencode and an Omarchy 4 session with Omarchy's bundled `qs.Commons` and `qs.Ui` QML modules. Omarchy is not a package dependency: `collie-tray-activate` checks its commands in your login session. Other distributions and desktop shells are not supported.

Pacman owns `/usr/bin/collied` and `/usr/share/collie-tray`, and no root install hook writes to user homes, logs in a node or starts a service. `/usr/bin/collied setup` writes the usual systemd user unit with `/usr/bin/collied` as its executable. `collie-tray-activate` validates the package directory, links `~/.config/omarchy/plugins/rbstp.collie` to it (Omarchy loads plugins through top-level links there), restarts the shell and enables the widget, keeping its bar section, position and settings, or putting it on the right.

### Adopt an existing installation

Keep the same login user, `XDG_*_HOME` values and custom config, and do not run setup again:

```sh
bash packaging/aur/install.sh
collied-aur-adopt
collie-tray-activate
```

`collied-aur-adopt` backs up the user unit to `collied.service.before-aur` and changes only its `ExecStart` executable to `/usr/bin/collied`; arguments, environment, configuration, data, node identity, pairings and push settings stay. A running service restarts; a stopped one stays stopped. It accepts only the quoted `collied` executable with optional `--config` and `run` arguments, and refuses wrappers, other commands, an existing backup, drop-ins or a unit loaded from another file. For those, inspect `systemctl --user cat collied`, change only the effective executable by hand, then run `systemctl --user daemon-reload` and `systemctl --user try-restart collied`.

Run `type -a collied` to find older copies earlier in `PATH`, and rename `~/.cargo/bin/collied` so it cannot shadow the package binary; until then, call `/usr/bin/collied` explicitly. Do not run a download installer or `just collied-install` over the package installation. `collie-tray-activate` moves an existing copied plugin to the hidden `.rbstp.collie.before-aur` directory before linking; it never deletes that backup or replaces an unexpected link.

### Upgrade and remove

```sh
cd /path/to/collie-packages
git fetch origin
git checkout REVIEWED_RECIPE_COMMIT
bash packaging/aur/install.sh
systemctl --user try-restart collied.service
omarchy restart shell
```

Use the recipe commit recorded for the release you want; after AUR publication, `paru -Syu` replaces the fetch and install steps. `try-restart` loads the new daemon only if it was running. Omarchy's plugin watcher does not follow the link, so restart the shell to load the new widget; the restart keeps placement and disabled widgets. Check `/usr/bin/collied --version`, `systemctl --user show collied -p ExecStart` and the version in the widget. To roll back, keep the daemon and widget on the same version, and check compatibility with the phone first.

To remove the packages, in the same login session:

```sh
omarchy plugin disable rbstp.collie
rm ~/.config/omarchy/plugins/rbstp.collie
/usr/bin/collied service uninstall
sudo pacman -Rns collie-tray collied-bin
omarchy restart shell
```

Node data, pairings, configuration and adoption backups stay. Pacman leaves those user files alone; uninstall the service first, or it restarts in a loop.

### Update the AUR recipes

After the matching release is public, download its `source.json`, `SHA256SUMS`, `collied-VERSION-linux-x86_64.tar.gz` and `CollieTray-VERSION-omarchy.tar.gz` over HTTPS. Verify the tag and source commit, and complete the [acceptance checks](#validation-before-public-rollout). For the first publication, check that `/usr/bin/collied setup` and `doctor` accept the package executable.

```sh
python3 scripts/desktop/aur.py /path/to/release-downloads
(cd packaging/aur/collied-bin && makepkg --printsrcinfo > .SRCINFO)
(cd packaging/aur/collie-tray && makepkg --printsrcinfo > .SRCINFO)
```

`aur.py` checks both archives against `SHA256SUMS`, their shared version, source and protocol metadata and the widget manifest, then updates both recipes' versions and hashes; it downloads, builds and publishes nothing. Review `PKGBUILD`, the helpers and `.SRCINFO`. In a disposable Arch system, build with unprivileged `makepkg`, inspect the packages, and test a fresh setup, adoption, upgrades in the same Omarchy session and removal. Only then, with a publishing identity, publish each directory as its own AUR repository, the daemon first. Never publish guessed URLs or checksums.

## Publisher setup and release controls

Signing runs in the protected `desktop-release` environment, restricted to `master`, with `DESKTOP_CERT_P12` (base64 Developer ID Application certificate and private key for team RM3UT3MMSR), `DESKTOP_CERT_PASSWORD`, `APPLE_KEY_P8` (base64 App Store Connect API key allowed to notarize), `APPLE_KEY_ID` and `APPLE_ISSUER_ID`. The repository's Apple API secrets can be reused; never use the iOS distribution certificate as the Developer ID. The certificate goes into a temporary keychain, deleted with the key files on exit.

The workflow signs only for a `master` dispatch or a pull request merged into `master`. The `validate` job, on trusted `master`, requires the source commit on `master` and any existing tag at that commit, creates a missing tag and outputs the resolved commit. The build and publish jobs copy `master`'s validator out of the checkout and run it again; it detaches to the commit and rechecks the commit and tag before any submodule or code of that commit runs, so an older release commit needs no current validator. Release jobs neither restore nor save shared build caches.

After TestFlight uploads a build and creates its tag, it dispatches this workflow from `master` with the tag and source commit, using only `actions: write`. It dispatches because a merged pull request's own ref, `refs/pull/N/merge`, is not allowed in the master-only signing environment, and GitHub starts no workflow for a tag pushed with `GITHUB_TOKEN`. One `publish` job creates the release for both platforms.

The `update-homebrew` job needs a `homebrew-tap` environment in `rbstp/collie`, restricted to `master`, holding `HOMEBREW_TAP_TOKEN`: a fine-grained personal access token owned by the tap publisher, limited to `rbstp/homebrew-tap`, with **Contents: Read and write** (metadata read is implicit), no workflow permission and no Apple credential. Enter it at the prompt, never in a command argument or a committed file:

```sh
gh secret set HOMEBREW_TAP_TOKEN --repo rbstp/collie --env homebrew-tap
```

Only this job sees the token, and it runs trusted `master` code, never the release source. After publication, it verifies the tag, source, eight assets, GitHub digests and checksums, then writes the published `Formula/collied.rb` and `Casks/colliebar.rb` to the tap in one commit, which starts the tap's own CI, and leaves other tap files alone. It changes only versions, URLs and SHA-256 values; a recipe behavior change needs a reviewed pull request. A tap already at the version is left as is; an older release, a different file at the same version or a concurrent tap change stops the job without forcing or retrying. If the secret is missing or the update conflicts, the job fails after publication: fix the cause and rerun only that job, or merge the two generated files by hand after review.

For a desktop-only release, dispatch from `master` with a new tag and its full `master` commit SHA:

```sh
gh workflow run desktop-release.yml --ref master -f tag=v0.3.0 -f source_sha=FULL_COMMIT_SHA
```

To retry, rerun the failed jobs or dispatch the same tag and SHA. Tags never move, and a retry only verifies a published release. A new draft records its tag and source in its body and target commit before any upload, and a retry adds a missing `source.json` only when those match. A retry of an interrupted draft keeps the signed packages already uploaded, uploads only the missing ones, and builds the Homebrew files and checksums from the kept packages. A release from another owner or source is refused. Do not edit assets or move tags during a release.

## Validation before public rollout

The macOS packaging test loads the generated formula and cask with the installed Homebrew in a temporary tap, prefix, trust store and cache, requiring tap-only paths and explicit tap trust, without touching the installed taps or configuration.

Local packaging and CI smoke tests do not prove clean-machine acceptance. Before advertising the first release, record:

- Real Developer ID signing, notary acceptance and staple validation, and a browser download that passes Gatekeeper on a clean supported Mac without developer tools or credentials.
- On a clean Mac, binary and Homebrew install, start, stop, upgrade, rollback, uninstall and CollieBar service discovery, with running and stopped services, custom config paths, and identity and pairings kept.
- On a clean Linux machine at the documented floor, installation and dynamic dependencies, systemd start, stop, update and uninstall, pairing and agent control. On Omarchy 4, run `just tray-test` and check the downloaded widget's status, QR pairing and controls across an upgrade.
- The generated Homebrew files published to the tap from the actual release assets, and formula and cask installation.
