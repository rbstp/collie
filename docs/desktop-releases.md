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

Both daemon packages include `release.json` with the exact source SHA, version and supported protocol range. Linux additionally records the highest required GLIBC symbol and every ELF `DT_NEEDED` library. The Ubuntu 24.04 builder runs `--version`, `--help` and `ldd`, fails unresolved libraries and refuses symbols above GLIBC 2.39. Those checks establish the builder runtime, not every distribution. herdr is needed to control agents; agent CLIs and the existing tailnet policy remain separate prerequisites. No Linux arm64, Intel Mac, generic GNOME/KDE tray binary or AUR package is advertised.

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

## CollieTray

Extract the version-matched Omarchy archive and run `bash install.sh`. It checks for Omarchy plugin validation/control commands, Quickshell (`qs`) and `qrencode`, validates the complete plugin, copies it to `~/.config/omarchy/plugins/rbstp.collie` and enables it on the right of the bar. Updating uses the same installer and restarts the Omarchy shell once. Config, pairings and the daemon's running state are untouched. This is an Omarchy widget, with no generic tray support.

To disable it run `omarchy plugin disable rbstp.collie`; then remove `~/.config/omarchy/plugins/rbstp.collie` to uninstall. Keep daemon and widget on the same release. A previous archive can be installed the same way for rollback.

## Publisher setup and release controls

Use the protected GitHub environment `desktop-release`, restricted to master. Manually provision `DESKTOP_CERT_P12` (base64 Developer ID Application certificate and private key for team RM3UT3MMSR), `DESKTOP_CERT_PASSWORD`, and `APPLE_KEY_P8` (base64 App Store Connect API key authorized for notarization), `APPLE_KEY_ID`, `APPLE_ISSUER_ID`. Existing repository Apple API secrets can be reused; never use the iOS distribution certificate as Developer ID. The workflow imports into a temporary keychain and removes keychain/key files on exit. Signing runs only for a master dispatch or a merged master pull request, never untrusted PRs. The validation job checks out trusted master, checks source ancestry and immutable tag identity, and emits the canonical commit resolved by Git. Build and publish jobs also check out literal master first. They copy its validator outside the checkout, validate the requested commit and tag again, and only then detach to the exact Git-resolved commit. They recheck HEAD, master ancestry and tag before initializing submodules or running selected source code. This also handles older release commits that lack the current validator. Release jobs do not restore or save shared build caches.

After a successful TestFlight upload and immutable tag creation, TestFlight dispatches the desktop workflow from master with the same tag and exact source SHA. A merged pull request's inherited deployment ref is still `refs/pull/N/merge`, which the master-only signing environment rejects. The dispatch uses only `actions: write`; the desktop workflow retains its source/tag validation and master-only signing environment. GitHub allows workflow dispatches created with GITHUB_TOKEN to start runs but suppresses tag-push workflows, so publishing has no tag-push dependency. Linux and macOS share one publish job that owns release creation.

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
