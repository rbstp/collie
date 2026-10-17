# Desktop release downloads

Desktop publishing is implemented by `.github/workflows/desktop-release.yml`. Public downloads require the publisher to provision Developer ID/notary credentials, run and validate the first release, and update the Homebrew tap. Until those gates pass, use the existing source installation instructions. Relay-first setup additionally depends on the relay sender work; these packaging changes do not remove the current optional APNs setup.

## Assets and requirements

Each immutable `vMAJOR.MINOR.PATCH` release contains:

| Asset | Supported runtime |
|---|---|
| `collied-VERSION-macos-arm64.tar.gz` | macOS 26+, Apple silicon; publisher signed and notarized binary |
| `collied-VERSION-macos-arm64.dmg` | Same daemon and installer in a signed, notarized, stapled disk image |
| `CollieBar-VERSION-macos-arm64.zip` | macOS 26+, Apple silicon; signed, notarized and stapled app |
| `collied-VERSION-linux-x86_64.tar.gz` | Linux x86_64, glibc 2.39+, systemd 255+, CA certificates |
| `CollieTray-VERSION-omarchy.tar.gz` | Architecture-independent Omarchy 4 plugin, Quickshell, qrencode, matching collied |
| `homebrew-VERSION.tar.gz` | Formula/collied.rb and Casks/colliebar.rb ready for rbstp/homebrew-tap |
| `source.json`, `SHA256SUMS` | Exact source commit and SHA-256 for every packaged download |

Both daemon packages include `release.json` with the exact source SHA, version and supported protocol range. Linux additionally records the highest required GLIBC symbol and every ELF `DT_NEEDED` library. The Ubuntu 24.04 builder runs `--version`, `--help` and `ldd`, fails unresolved libraries and refuses symbols above GLIBC 2.39. Those checks establish the builder runtime, not every distribution. herdr is needed to control agents; agent CLIs and the existing tailnet policy remain separate prerequisites. No Linux arm64, Intel Mac or generic GNOME/KDE tray binary is advertised.

## Install a daemon download

Download all assets from one release. Verify `SHA256SUMS` with `shasum -a 256 -c SHA256SUMS` on macOS or `sha256sum -c SHA256SUMS` on Linux in the directory containing all listed downloads. Checksums detect damaged downloads; GitHub HTTPS and the publisher's macOS signatures establish their origin.

On macOS, open the DMG and run its `install.sh` in Terminal, or extract the daemon tar archive. On Linux, extract the daemon tar archive. In either extracted daemon directory:

```sh
bash install.sh --replace
export PATH="$HOME/.cargo/bin:$PATH"
collied setup
```

The installer explicitly replaces `~/.cargo/bin/collied` using a fresh inode. macOS checks Developer ID team RM3UT3MMSR, the collied signing identifier, the strict code signature and Apple's explicit `notarized` requirement before copying. Standalone executables use this [notarization check](https://developer.apple.com/forums/thread/130560), rather than an app-only execution assessment. The check forces an online Apple ticket lookup, so allow network access even when installing from the stapled DMG. This needs no Xcode, Rust, publisher credential or `sudo`. Do not remove quarantine or disable Gatekeeper.

For an existing service the installer updates only its executable argument, preserving custom configuration arguments, pinned environment and data paths. It refreshes the same launchd label or systemd user unit. A loaded launchd agent or active systemd unit restarts; a stopped service remains stopped. No first-install service is created until `collied setup`. Linux refuses an unrecognized service command or a unit loaded from a different location. Keep the previous download for rollback and run its installer with `--replace`; node identity, configuration, pairings and audit data are retained. Ensure the previous version supports the installed app's protocol before rolling back.

To uninstall, run `~/.cargo/bin/collied service uninstall`, then remove `~/.cargo/bin/collied`. Keep the data directory to preserve identity and pairings. Remove CollieBar from Applications after quitting it and disabling its Open at Login setting.

## Homebrew

The existing tap is [rbstp/homebrew-tap](https://github.com/rbstp/homebrew-tap). A desktop release generates formula/cask files with final asset URLs and hashes. The publisher reviews and updates that tap only after the release exists; generated assets alone do not make `brew install` available.

Once those files are published:

```sh
brew install rbstp/tap/collied
collied-install --replace
brew install --cask rbstp/tap/colliebar
```

The formula's binary is the package source. `collied-install --replace` explicitly copies it to the stable regular file expected by setup and refreshes existing services. There is no automatic install hook and no `brew services` label. After `brew upgrade collied`, run `collied-install --replace` again to adopt the new binary. A Cellar cleanup cannot delete the service's executable. Homebrew uninstall leaves the explicitly installed user copy and service alone: uninstall the service and remove that copy separately. The cask installs CollieBar to Applications; its existing daemon controls find the executable through the launchd plist.

## Arch Linux and AUR preparation

The recipes in `packaging/aur/` are preparation, not published AUR packages. Their v0.1.2 assets are verified packaging fixtures. Do not publish them: v0.1.2 requires a regular `~/.cargo/bin/collied` for setup and service diagnostics. The first AUR publication must use a later matching desktop release containing the Linux `/usr/bin/collied` support in this change. No release or AUR upload happens when these recipes merge.

After that matching release is verified and the recipes are updated, install directly from GitHub. An AUR account is not needed. Install Arch's `base-devel` and `git` first, then:

```sh
git clone --no-checkout https://github.com/rbstp/collie.git collie-packages
cd collie-packages
git checkout REVIEWED_RECIPE_COMMIT
bash packaging/aur/install.sh
/usr/bin/collied setup
collie-tray-activate
```

Replace `REVIEWED_RECIPE_COMMIT` with the immutable packaging commit recorded after the publisher verifies the release and updates the recipes. Do not check out unreleased master or the v0.1.2 reference recipe. The release source tag precedes final artifact checksums, so the publisher records a separate reviewed recipe commit after the binary release.

The installer runs unprivileged `makepkg --nodeps` to package both prebuilt archives, then installs both packages in one `sudo pacman -U` transaction, including repository runtime dependencies. No Rust or Go build is performed. A single transaction keeps the exact daemon/widget version dependency satisfied during upgrades. Review the recipes and helpers before running it. Once an AUR publishing identity is available and both packages are published, `paru -S collie-tray` can replace the GitHub download/build/install steps.

`collie-tray` depends on the exact matching `collied-bin` version. The daemon package supports Arch Linux x86_64 with glibc 2.39+, systemd 255+ and CA certificates; herdr and agent CLIs are separate prerequisites for controlling agents. The widget additionally requires an installed Omarchy 4 session, Quickshell, qrencode and Omarchy's bundled `qs.Commons` and `qs.Ui` QML modules. Omarchy is not an Arch package dependency; `collie-tray-activate` checks its commands in your login session. Other distributions and desktop shells are outside this package's support boundary.

Pacman owns `/usr/bin/collied` and `/usr/share/collie-tray`. Setup writes the usual per-user systemd unit with `/usr/bin/collied` as its executable. No root install hook writes user homes, logs in a node or starts services. Activation validates the package directory, then links `~/.config/omarchy/plugins/rbstp.collie` to it. The real Omarchy loader discovers top-level directory links in that directory; there is no assumed system plugin search path. Activation restarts the shell and enables the widget, retaining its existing bar section, position and settings, or using the right section when absent.

### Adopt an existing installation

Keep the same login user, `XDG_*_HOME` values and custom config. Do not run setup again just to adopt an existing node:

```sh
bash packaging/aur/install.sh
collied-aur-adopt
collie-tray-activate
```

The daemon helper backs up the loaded regular user unit to `collied.service.before-aur`, then changes only the first `ExecStart` executable to `/usr/bin/collied`. It preserves all remaining arguments, environment, configuration, data, node identity, pairing and Direct/relay settings. An active service restarts; a stopped service stays stopped. It recognizes the native quoted `collied` executable with optional `--config` and `run` arguments. It refuses wrappers, unrelated executables, existing backups, drop-ins, a different loaded unit or an unrecognized command rather than discarding customization. For those cases, inspect `systemctl --user cat collied` and manually change only the effective executable, then run `systemctl --user daemon-reload` and `systemctl --user try-restart collied`.

Use `type -a collied` to find older copies earlier in PATH. Move `~/.cargo/bin/collied` to an unused backup filename so it cannot shadow the package binary; use `/usr/bin/collied` explicitly until that is done. Do not run the old download installer or `just collied-install` over the package installation. The tray helper moves an existing copied plugin to the hidden `.rbstp.collie.before-aur` directory before linking the package; it never deletes that backup or replaces an unexpected link. User layout and daemon data stay in place.

### Upgrade and remove

```sh
cd /path/to/collie-packages
git fetch origin
git checkout REVIEWED_RECIPE_COMMIT
bash packaging/aur/install.sh
systemctl --user try-restart collied.service
omarchy restart shell
```

Use the new immutable recipe commit recorded for the selected verified release. After later AUR publication, `paru -Syu` replaces the GitHub pull and installer steps. The package upgrade replaces the active installation paths, with no copied executable or widget to refresh. The user-session commands load the new daemon and QML; `try-restart` preserves a stopped daemon, and shell restart preserves placement and disabled widgets. Omarchy's recursive plugin watcher does not traverse the link, so a shell restart is required after widget upgrades. Verify `/usr/bin/collied --version`, `systemctl --user show collied -p ExecStart` and the running version in the widget. Keep daemon and tray versions matched when rolling back, and check compatibility with the phone first.

Before removing the packages, in the same login session:

```sh
omarchy plugin disable rbstp.collie
rm ~/.config/omarchy/plugins/rbstp.collie
/usr/bin/collied service uninstall
sudo pacman -Rns collie-tray collied-bin
omarchy restart shell
```

This removes the package link and service, retaining node data, pairings, configuration and explicit adoption backups. A bare pacman removal deliberately leaves those user-owned files alone; stop and uninstall the service first to avoid its restart loop.

### Update the AUR recipes

After the matching desktop release is public, download `source.json`, `SHA256SUMS`, `collied-VERSION-linux-x86_64.tar.gz` and `CollieTray-VERSION-omarchy.tar.gz` from its GitHub release over HTTPS. Verify the tag and source commit and complete the desktop release acceptance checks. For the first publication, explicitly verify that `/usr/bin/collied setup` and `doctor` accept the package executable, not the v0.1.2 fixture guard.

```sh
python3 scripts/desktop/aur.py /path/to/release-downloads
(cd packaging/aur/collied-bin && makepkg --printsrcinfo > .SRCINFO)
(cd packaging/aur/collie-tray && makepkg --printsrcinfo > .SRCINFO)
```

The updater checks both archive hashes against `SHA256SUMS`, their shared version/source/protocol metadata and the widget manifest, then updates both recipe versions and all source hashes together. It does not download, build or publish anything. Review `PKGBUILD`, helpers and `.SRCINFO`; build with unprivileged `makepkg` in disposable Arch, inspect package contents/dependencies and test fresh setup, existing-service adoption, upgrades in the same Omarchy session and removal. Publish each directory as its separate AUR repository only after review and publishing identity are available. Publish the matching daemon before the tray, and never publish guessed URLs or checksums.

## CollieTray

Extract the version-matched Omarchy archive and run `bash install.sh`. It checks for Omarchy plugin validation/control commands, Quickshell (`qs`) and `qrencode`, validates the complete plugin, copies it to `~/.config/omarchy/plugins/rbstp.collie` and enables it on the right of the bar. Updating uses the same installer and restarts the Omarchy shell once. Config, pairings and the daemon's running state are untouched. This is an Omarchy widget, with no generic tray support.

To disable it run `omarchy plugin disable rbstp.collie`; then remove `~/.config/omarchy/plugins/rbstp.collie` to uninstall. Keep daemon and widget on the same release. A previous archive can be installed the same way for rollback.

## Publisher setup and release controls

Use the protected GitHub environment `desktop-release`, restricted to master. Manually provision `DESKTOP_CERT_P12` (base64 Developer ID Application certificate and private key for team RM3UT3MMSR), `DESKTOP_CERT_PASSWORD`, and `APPLE_KEY_P8` (base64 App Store Connect API key authorized for notarization), `APPLE_KEY_ID`, `APPLE_ISSUER_ID`. Existing repository Apple API secrets can be reused; never use the iOS distribution certificate as Developer ID. The workflow imports into a temporary keychain and removes keychain/key files on exit. Signing runs only for a master dispatch or a merged master pull request, never untrusted PRs. The validation job checks out trusted master, checks source ancestry and immutable tag identity, and emits the canonical commit resolved by Git. Build and publish jobs also check out literal master first. They copy its validator outside the checkout, validate the requested commit and tag again, and only then detach to the exact Git-resolved commit. They recheck HEAD, master ancestry and tag before initializing submodules or running selected source code. This also handles older release commits that lack the current validator. Release jobs do not restore or save shared build caches.

After a successful TestFlight upload and immutable tag creation, TestFlight dispatches the desktop workflow from master with the same tag and exact source SHA. A pull request title containing `[ios-only]` skips that desktop dispatch while retaining the TestFlight upload and tag; ordinary pull requests and manual TestFlight runs keep the combined release path. A merged pull request's inherited deployment ref is still `refs/pull/N/merge`, which the master-only signing environment rejects. The dispatch uses only `actions: write`; the desktop workflow retains its source/tag validation and master-only signing environment. GitHub allows workflow dispatches created with GITHUB_TOKEN to start runs but suppresses tag-push workflows, so publishing has no tag-push dependency. Linux and macOS share one publish job that owns release creation.

For a desktop-only release, select master and supply a new version tag and its full master commit SHA:

```sh
gh workflow run desktop-release.yml --ref master -f tag=v0.3.0 -f source_sha=FULL_COMMIT_SHA
```

For retries, rerun failed jobs or dispatch the same tag and exact SHA. Existing tags are never moved. Published releases are immutable: retries verify source identity, asset completeness and every checksum without replacing assets. The initial draft creation records its exact tag/source identity in the release body and target commit before any asset upload. A retry repairs a missing source marker only when those claims and the current tag match exactly. Interrupted drafts retain already uploaded signed packages and upload only missing assets; Homebrew hashes and final checksums use the retained packages. A release from another owner or source is rejected. Do not manually edit assets or move tags during a release.

## Validation before public rollout

The macOS packaging test loads the generated formula and cask with the installed Homebrew DSL in a temporary tap, prefix, trust store and cache. It enforces tap-only paths and explicit tap trust, rejects a loose formula path, and leaves installed taps and Homebrew configuration untouched.

Local packaging and CI smoke checks do not establish clean-machine acceptance. Before advertising the first release, record:

- Real Developer ID signing, notary acceptance and staple validation; download through a browser and pass Gatekeeper on a clean supported Mac without developer tools or credentials.
- Clean Mac binary and Homebrew install, start, stop, upgrade, rollback, uninstall and CollieBar service discovery. Check both previously running and stopped services, custom config paths, retained identity and pairings.
- Clean Linux installation and dynamic dependencies on the documented floor, systemd start/stop/update/uninstall, pairing and agent control. On Omarchy 4, run `just tray-test`, verify the downloaded widget's status, QR pairing and controls across upgrade.
- Publish the generated Homebrew files to the existing tap using the actual immutable release assets and validate formula/cask installation. No production publishing or credential provisioning happens merely by merging this workflow.
