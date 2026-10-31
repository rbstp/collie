# Release

`.github/workflows/testflight.yml` builds, signs and uploads the iOS app to TestFlight. The Apple side is set up once, by hand.

## One-time Apple setup

Team `RM3UT3MMSR`, in the Apple Developer portal and App Store Connect:

1. **App Group**: register `group.dev.rbstp.collie`.
2. **App IDs** (explicit), each with **App Groups** enabled and `group.dev.rbstp.collie` assigned:
   - `dev.rbstp.collie`, the app, also with **Push Notifications** and **App Attest**;
   - `dev.rbstp.collie.push`, the ColliePush notification service extension, also with **Push Notifications**;
   - `dev.rbstp.collie.widgets`, the CollieWidgets extension (Live Activities), which reads the notification key mirror in the group. It needs no push capability: Live Activity pushes use the app's topic and the same APNs key;
   - `dev.rbstp.collie.watchkitapp`, the CollieWatch app, and `dev.rbstp.collie.watchkitapp.widgets`, its usage complication. On the watch, the group is the watch's own container. No push capability and no keychain group.
3. **Provisioning profiles**: App Store Connect distribution, with the team's Apple Distribution certificate, named exactly as `Collie/project.yml` and `Collie/ExportOptions.plist` refer to them:
   - **`Collie App Store`** for `dev.rbstp.collie`;
   - **`Collie Push App Store`** for `dev.rbstp.collie.push`;
   - **`Collie Widgets App Store`** for `dev.rbstp.collie.widgets`;
   - **`Collie Watch App Store`** for `dev.rbstp.collie.watchkitapp`;
   - **`Collie Watch Widgets App Store`** for `dev.rbstp.collie.watchkitapp.widgets`.

   A profile is a snapshot of its App ID's capabilities: after any capability change, regenerate the profile (Edit, Save) and update its secret.
4. **App Store Connect app record** for `dev.rbstp.collie`, with an internal TestFlight group.
5. **APNs key for direct APNs, one per machine.** Only a machine set up with `collied setup --direct-apns` needs one; a machine on the publisher relay needs none. In Certificates, Identifiers & Profiles, Keys, add a key with **Apple Push Notifications service (APNs)**, environment **Sandbox & Production**, so one collied serves both debug builds from `just ios-run-device` (sandbox tokens) and TestFlight builds (production tokens). Download the `.p8` (offered once) and note the Key ID. A key per machine lets each machine be revoked alone. The key is a runtime secret of that machine's daemon, separate from the App Store Connect API key below, and never goes into GitHub.

   On the Mac, the key lives in the login Keychain, readable without a prompt only by collied signed with your Developer ID Application identity:

   ```toml
   # ~/Library/Application Support/collie/collied.toml
   [apns]
   key = "keychain"
   key_id = "<KEY_ID>"
   team_id = "RM3UT3MMSR"
   bundle_id = "dev.rbstp.collie"
   ```

   ```sh
   just collied-install                      # signed build to ~/.cargo/bin/collied, restarts the agent
   chmod 0600 AuthKey_<KEY_ID>.p8
   collied apns import AuthKey_<KEY_ID>.p8   # stores, reads back, offers to delete the file
   collied doctor                            # codesign and apns lines must be ok
   collied apns test
   ```

   With no push mode configured, `collied setup --direct-apns` offers these steps: it asks for the path of the `.p8`, which must already be mode 0600, and the IDs (defaults `RM3UT3MMSR` and `dev.rbstp.collie`), writes the section, imports the key and restarts collied.

   The Keychain item trusts the program that created it, so import with the signed binary, and reinstall with `just collied-install` (never `cargo install`) after every change to collied, or the daemon cannot read the key without a prompt. For a config that still has `key_path`, `collied apns import` on that file moves the key and rewrites the line to `key = "keychain"`. Revoke the key in the portal if the Mac is compromised.

   On Linux, the key is a systemd user credential, encrypted with the host key (and the TPM2 when one is usable); there is no code signing:

   ```toml
   # ~/.local/share/collie/collied.toml ($XDG_DATA_HOME/collie/collied.toml when absolute)
   [apns]
   key = "systemd-creds"                     # the default on Linux, may be omitted
   key_id = "<KEY_ID>"
   team_id = "RM3UT3MMSR"
   bundle_id = "dev.rbstp.collie"
   ```

   ```sh
   just collied-install                      # release build to ~/.cargo/bin/collied, restarts the unit if active
   chmod 0600 AuthKey_<KEY_ID>.p8
   collied apns import AuthKey_<KEY_ID>.p8   # encrypts, decrypts back, offers to delete the file
   collied doctor                            # apns line must be ok and shows the seal
   collied apns test                         # needs a paired phone (see collied on Linux)
   ```

   `collied setup --direct-apns` offers the same steps. Import writes the credential to `apns/<KEY_ID>.cred` in the data directory. systemd seals it with the host key, plus the TPM2 when one is usable, and doctor's apns line shows which. When a TPM2 becomes usable after a host-only import, doctor warns, and importing the `.p8` again seals the credential to it. Where systemd-creds cannot encrypt (for example no `systemd-creds.socket`, systemd older than 256, a container), import falls back to a 0600 copy at `apns/AuthKey_<KEY_ID>.p8` in the data directory, sets `key_path` to it and says so; doctor reports the fallback and, once systemd-creds works, warns you to import that file to encrypt it. The credential stops decrypting after an OS reinstall, a machine-id change, a uid or user name change, or a move to another machine: keep a copy of the `.p8` offline to import again. Revoke the key in the portal if the machine is compromised.

## Menu bar app notarization (optional)

`just mac-install` signs the menu bar app with the Developer ID Application certificate and the hardened runtime. It runs unnotarized on the Mac that built it and needs no App ID or provisioning profile, since it has no entitlements.

To run it on another Mac without a Gatekeeper prompt, notarize it with one notarytool credential stored in the login keychain:

- an App Store Connect API team key with the Developer role (the TestFlight key works): `xcrun notarytool store-credentials collie --key AuthKey_<KEY_ID>.p8 --key-id <KEY_ID> --issuer <ISSUER_ID>`
- or an Apple ID app-specific password (appleid.apple.com): `xcrun notarytool store-credentials collie --apple-id <apple id> --team-id RM3UT3MMSR --password <app-specific password>`

With the latest Program License Agreement accepted (otherwise submissions are refused), run `just mac-install`, then `just mac-notarize` (or `just mac-notarize <profile>`): it submits, waits, staples the app in `target/mac` and `~/Applications`, and checks it with `spctl`. On a failure, `xcrun notarytool log <submission id> --keychain-profile collie` says why.

## collied on Linux

[Desktop release downloads](desktop-releases.md) covers the release download. From a checkout, with the README's prerequisites:

```sh
just setup                # submodules
just collied-install      # release build to ~/.cargo/bin/collied as a fresh inode
collied setup             # login, service install, doctor, push notifications, pairing
```

`collied setup` runs `collied login`, `collied service install` (the systemd user unit `~/.config/systemd/user/collied.service`), `collied doctor`, the push step (the publisher relay, or the APNs import with `--direct-apns`) and `collied pair`, skipping steps already done. To update, run `git pull`, `just setup`, `just collied-install` (which restarts an active unit) and `collied setup`, which keeps the node and the paired phones.

## Repository secrets

All are repository secrets. Encode the base64 ones with `base64 -i <file> | gh secret set <NAME>`:

| Secret | Content |
| --- | --- |
| `APPLE_DIST_CERT_P12` | Apple Distribution certificate and key, `.p12`, base64 |
| `APPLE_DIST_CERT_PASSWORD` | password of that `.p12` |
| `APPLE_PROVISIONING_PROFILE` | `Collie App Store` profile, `.mobileprovision`, base64 |
| `APPLE_PROVISIONING_PROFILE_PUSH` | `Collie Push App Store` profile, `.mobileprovision`, base64 |
| `APPLE_PROVISIONING_PROFILE_WIDGETS` | `Collie Widgets App Store` profile, `.mobileprovision`, base64 |
| `APPLE_PROVISIONING_PROFILE_WATCH` | `Collie Watch App Store` profile, `.mobileprovision`, base64 |
| `APPLE_PROVISIONING_PROFILE_WATCH_WIDGETS` | `Collie Watch Widgets App Store` profile, `.mobileprovision`, base64 |
| `APPLE_KEY_P8` | App Store Connect API key (Developer role), `.p8`, base64 |
| `APPLE_KEY_ID` | that key's ID |
| `APPLE_ISSUER_ID` | App Store Connect issuer ID |

The certificate and the profiles expire after a year; renew them and update their secrets.

## Cutting a release

Keep published iOS, collied, CollieBar and CollieTray releases on the same version, tag and source commit, including UI-only changes.

- Merging a pull request into `master` ships a build unless its title contains `[skip-release]` or it changes only `.github/`, `docs/` (the site included), `Mac/`, `README.md`, `AGENTS.md` or `CLAUDE.md`. The site deploys through its own workflow, `pages` ([deploy-pages.md](deploy-pages.md)); a change only to `docs/site/`, `docs/deploy-pages.md`, `README.md`, `AGENTS.md` or `CLAUDE.md` also skips the macOS `rust` and `ios` jobs of `ci`.
- Version: the latest `v*` tag with its minor number bumped when any commit since that tag, or the pull request title, is a `feat`, otherwise its patch number. Build number: the workflow run number.
- A manual run (Actions, testflight, Run workflow, on `master` only) ships the given version, for example `1.0.0`.
- After the upload, the workflow creates the tag `v<version>` and starts the [desktop release](#desktop-github-releases) and the [documentation screenshots](#documentation-screenshots) refresh.

## iOS release notes

[iOS release notes](ios-release-notes.md) holds the text for the next combined release. Copy its release notes and tested-support disclosure into the App Store version's What's New, and its test checklist into TestFlight's What to Test. The first App Store version has no What's New, so its text goes in Description ([platform version information](https://developer.apple.com/help/app-store-connect/reference/app-information/platform-version-information)). Keep the disclosure in release metadata until broader testing is recorded.

The upload workflow publishes none of this: confirm the text in App Store Connect before distributing the build. A successful upload does not mean the build is processed and installable.

## App icon and TestFlight

- The icon is a single 1024x1024 universal image, `Collie/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon.png`, from which Xcode derives every other size. The app's `Info.plist` sets `CFBundleIconName` to `AppIcon` and `project.yml` sets `ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon`: App Store validation rejects an upload whose icon is in the asset catalog without `CFBundleIconName`.
- The iOS app embeds the watch app, so every iOS build, test and archive also builds for watchOS: `ci` and `testflight` install the watchOS platform on the `xcode-27` runner when it is missing. The watch app has its own copy of the icon, `Collie/CollieWatch/Assets.xcassets/AppIcon.appiconset/AppIcon.png`.

## Debug symbols

Before each TestFlight upload, the workflow keeps an `ios-symbols-<version>-<build>-<attempt>` Actions artifact for 90 days: the archive's dSYM bundles and a `build.txt` with the source commit, app version and build, and symbol UUIDs, with no app archive, profile or signing credential. Download it from that exact run, and keep a private copy if support may outlast it.

Before symbolicating, match the app UUID in a device report to `build.txt` or `xcrun dwarfdump --uuid <app.dSYM>`; never use symbols from another build or a rebuilt binary whose UUID differs. Keep device reports and captured logs private, out of public issues and release artifacts.

## Export compliance

This is not legal advice; confirm with counsel before an App Store release.

The app implements standard encryption on top of Apple's: WireGuard (Curve25519, ChaCha20-Poly1305) in libtailscale and TLS 1.3 (rustls with ring) to collied. Notification payloads use CryptoKit. There is no proprietary or [non-standard](https://www.ecfr.gov/current/title-15/subtitle-B/chapter-VII/subchapter-C/part-772/section-772.1) cryptography, and the source is public.

- **US (EAR)**: once generally available on the App Store, mass market under Note 3 to Category 5 Part 2, self-classified as 5D992.c under [15 CFR 740.17(b)(1)](https://www.ecfr.gov/current/title-15/subtitle-B/chapter-VII/subchapter-C/part-740/section-740.17). No CCATS. Since the 2021 rule (86 FR 16482), the annual self-classification report of 740.17(e)(3) covers components and certain executable software, not an end-item app like this one, so none is filed; this section is the classification record. A report, if ever filed, is due February 1 for the previous year, by email to `crypt-supp8@bis.doc.gov` and `enc@nsa.gov`.
- **Apple**: `Info.plist` sets [`ITSAppUsesNonExemptEncryption`](https://developer.apple.com/documentation/bundleresources/information-property-list/itsappusesnonexemptencryption) to `false`. Apple's "exempt" means exempt from uploading documentation: standard algorithms not provided by the OS need a document only for the App Store in France ([Apple's table](https://developer.apple.com/help/app-store-connect/reference/app-information/export-compliance-documentation-for-encryption)). The key answers the encryption question for every build, so TestFlight uploads never stop at Missing Compliance. Never set `true` without the matching [`ITSEncryptionExportComplianceCode`](https://developer.apple.com/documentation/bundleresources/information-property-list/itsencryptionexportcompliancecode): builds then stop at Missing Compliance, and a code Apple did not issue can fail the upload.
- **Internal TestFlight** (today): builds reach only the owner in Canada, and [Apple](https://developer.apple.com/documentation/security/complying-with-encryption-export-regulations) applies US export law only to distribution outside the US or Canada. Nothing else to do.

Decision (owner, 2026-10-20): App Store availability is every country or region except France.

Before an App Store release:

1. In App Store Connect, Pricing and Availability, App Availability, choose specific countries or regions and leave out France; All Countries or Regions includes it.
2. If App Store Connect asks about encryption (App Information, App Encryption Documentation, or Manage next to a build), answer: uses encryption, yes; algorithms, "Standard encryption algorithms instead of, or in addition to, using or accessing the encryption within Apple's operating system"; available on the App Store in France, no. No document is needed.
3. To ship in France instead: file a declaration with [ANSSI](https://cyber.gouv.fr/reglementation/reglementation-identite-confiance-numerique/controles-reglementaires-cryptographie/controle-moyen-de-cryptologie/) (up to a month), fill in the App Description and availability, [upload the declaration](https://developer.apple.com/help/app-store-connect/manage-app-information/determine-and-upload-app-encryption-documentation) under App Encryption Documentation with France set to yes, wait for Apple's approval, then in one commit set `ITSAppUsesNonExemptEncryption` to `true` and `ITSEncryptionExportComplianceCode` to the key value Apple shows next to the approved document.

Leaving out France does not keep external TestFlight testers out of France: Apple documents no country or region setting for external testing. A public link's tester criteria cover devices and OS versions ([Invite external testers](https://developer.apple.com/help/app-store-connect/test-a-beta-version/invite-external-testers/), [TestFlight overview](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview/)), and availability by country or region applies to the App Store ([Manage availability for your app on the App Store](https://developer.apple.com/help/app-store-connect/manage-your-apps-availability/manage-availability-for-your-app-on-the-app-store/)). Apple's France requirement names only the App Store; until that is settled, invite no external testers located in France.

## Adding a signed extension target

CollieWidgets went in this way. `Collie/ExportOptions.plist` maps each bundle ID to its profile name under `provisioningProfiles` (`dev.rbstp.collie.widgets` to `Collie Widgets App Store`), and the workflow installs `APPLE_PROVISIONING_PROFILE_WIDGETS` with the others.

1. Register its App ID (with the capabilities its entitlements use) and an App Store profile for it.
2. Add its secret, for example `APPLE_PROVISIONING_PROFILE_<NAME>`, to the `env` of the "check for the signing secrets" and "import the signing certificate and profiles" steps; every `APPLE_PROVISIONING_PROFILE*` variable is checked and installed.
3. Map its bundle ID to the profile name in `Collie/ExportOptions.plist`, and set `PROVISIONING_PROFILE_SPECIFIER` in its Release settings in `Collie/project.yml`.

## Public submission

[Public submission preparation](public-submission.md) has the offline metadata and disclosure checks for #359, which records the submission results, [App Review notes](app-review.md) the review notes and submission checklist, and the [relay runbook](relay-runbook.md) the operating procedures. The public TestFlight link and its capacity wait on the [submission checklist](app-review.md#submission-checklist).

## App Review sample workspace

[App Review notes](app-review.md#review-notes) has the text that points reviewers to **Explore sample workspace**, and the prior approval it needs. The sample runs in Release builds on the iOS Simulator and iPhone, but does not show real pairing, background APNs delivery, Live Activities or device attestation: include real-hardware evidence for those, and record the Release simulator and signed-device validation with the submission evidence.

## Documentation screenshots

After a successful upload, `testflight` dispatches `.github/workflows/docs-screenshots.yml` from `master` with the shipped commit, so the images in `docs/site/src/images` (also shown by the README) match the shipped app. A failed `docs screenshots` run never fails or holds back the build, the tag or the desktop release. Its `docs-screenshots` concurrency group makes a second release wait for the first refresh.

1. **capture**, on `xcode-27` with read-only contents and no secret: refuses a commit that is not on `master`, builds the simulator framework, creates an iPhone 18 Pro Max simulator if the runner has none, installs pngquant 3.0.3 from pngquant.org (checked against its SHA-256), runs `just screenshot_watch=false ios-screenshots` and keeps the images as the `screenshots` artifact for 7 days. It restores caches but never saves them, so it cannot change what `testflight` and `ci` restore.
2. **propose**, on `master` only, in the `docs-screenshots` environment: copies the artifact's PNG files onto `master`, failing on any other file, and stops when nothing changed. Otherwise it force-pushes them to the `docs-screenshots` branch, opens or reuses the pull request "Refresh documentation screenshots [skip-release]" and enables squash auto-merge. The pull request runs the normal checks (`ci` skips `rust` and `ios` for a `docs/site/` change), the merge ships no build, and its push to `master` deploys the site.

Pull requests, pushes and merges made with `GITHUB_TOKEN` start no workflow, and the repository does not let Actions create pull requests, so **propose** uses `DOCS_SCREENSHOTS_TOKEN`, held by the `docs-screenshots` environment of `rbstp/collie`. It is a fine-grained personal access token of the owner, limited to `rbstp/collie`, with **Contents: Read and write** and **Pull requests: Read and write** (metadata read is implicit), an expiration date and no workflow permission. Keep the environment restricted to `master`: the owner's role bypasses the `master` ruleset, so a leaked token could push any file outside `.github/workflows/` to `master`. Renew the token before it expires, and enter it at the prompt, never in a command argument or committed file:

```sh
gh secret set DOCS_SCREENSHOTS_TOKEN --repo rbstp/collie --env docs-screenshots
```

Only **propose** sees the token, and it never builds or runs the captured revision; its commit is authored as the token's owner. Without the secret, **propose** fails with a message pointing here, after **capture** has kept the images.

The workflow refreshes only the iPhone screenshots: the `xcode-27` runner's watchOS simulator runtime loses its files minutes after it is mounted, so a watch does not stay booted through the capture ([#539](https://github.com/rbstp/collie/issues/539)). When the watch app changes, the owner refreshes `watch-usage.png` and `watch-widget.png` locally with `just ios-screenshots` and commits them.

To refresh without a release, run the workflow by hand (Actions, docs screenshots, Run workflow, on `master`), optionally with a `master` commit to capture; the default is the head of `master`. The demo's usage reset times follow the capture time, so `usage-light.png` and `usage-dark.png` change on every run and every release opens a refresh pull request. A local run also changes the two watch images, whose clock shows the capture time.

## App Store screenshots

`just ios-screenshots` also writes the App Store set to `target/ios/app-store` and prints its path; Git ignores it and the workflow does not keep it. The set is the same capture as the documentation images, before resizing and compression, as PNG without alpha, which App Store Connect requires:

- the twelve iPhone screenshots (Agents, Usage, New task, Chat, Terminal and an approval, light and dark) at 1320 x 2868, the native size of the iPhone 18 Pro Max simulator and a portrait size of iPhone with Dynamic Island (large display). Apple's page says iPhone Duo screenshots become required in April 2027 for apps built with the iOS 27.1 SDK or later;
- `watch-usage.png` and `watch-widget.png` at 416 x 496, the native size of the Apple Watch Series 12 (46mm) simulator and the accepted size for Series 10, 11 and 12.

Both sizes come from Apple's [screenshot specifications](https://developer.apple.com/help/app-store-connect/reference/screenshot-specifications); check them again before a submission.

For version 0.1.8, App Store Connect first asked for an iPhone 6.5" Display set (1242 x 2688 or 1284 x 2778), then for iPhone with Dynamic Island (medium display) (1206 x 2622 or 1179 x 2556). `just screenshot_device="<simulator>" ios-screenshots` captures the same twelve screenshots on that simulator and writes only the App Store set, to `target/ios/app-store/<width>x<height>`, leaving the documentation images and the watch alone. `iPhone 18 Pro` gives the 1206 x 2622 medium display set, an iPhone 14 Plus simulator (`xcrun simctl create "iPhone 14 Plus" com.apple.CoreSimulator.SimDeviceType.iPhone-14-Plus`) 1284 x 2778, and iPhone 11 Pro Max 1242 x 2688; iPhone Xs Max cannot be created on the iOS 27.2 runtime.

The iOS 27.2 runtime refuses to create an iPhone Duo, so its set needs the iOS 27.1 runtime. Download it once with `xcodebuild -downloadPlatform iOS -buildVersion 27.1 -architectureVariant arm64` (7.9 GB), create the simulator with `xcrun simctl create "iPhone Duo" com.apple.CoreSimulator.SimDeviceType.iPhone-Duo com.apple.CoreSimulator.SimRuntime.iOS-27-1`, then run `just screenshot_device="iPhone Duo" ios-screenshots`. The set lands in `target/ios/app-store/1398x2034`, the native size of the cover display and one of the two portrait sizes App Store Connect accepts for iPhone Duo. The inner display (2007 x 2853) cannot be captured with Xcode 27.2: the simulator boots folded, and neither Xcode nor `simctl` can unfold it. Collie fills the cover display except for a strip along its trailing edge, which the system keeps for the status bar, the navigation bar buttons and the tab bar; the cover display does not rotate.

Locally, the default run needs an available `iPhone 18 Pro Max` simulator paired with a watch simulator (`xcrun simctl pair <watch> <phone>`); `just screenshot_watch=false ios-screenshots` skips the watch, as the workflow does, so the set has no watch screenshots. The iOS 27.2 simulator applies an appearance change only at boot, so the recipe reboots the phone simulator for each appearance, dark then light, and restores the original appearance at the end, which a simulator that started dark shows from its next boot. The phone shows the overridden 9:41 status bar; the watch, which has no status bar override, shows the capture time. The default run deletes `target/ios/app-store` first, so run it before any `screenshot_device` run.

To upload:

1. On the commit being submitted, so the screenshots match the binary, run `just ios-screenshots`, then `just screenshot_device="<simulator>" ios-screenshots` for each other size App Store Connect lists.
2. In App Store Connect, open the app, select the iOS version being prepared and open **Previews and Screenshots**.
3. Under iPhone, for each listed size (iPhone with Dynamic Island (large display) from `target/ios/app-store`, medium display from `target/ios/app-store/1206x2622`, 6.5" from `target/ios/app-store/1284x2778`, iPhone Duo from `target/ios/app-store/1398x2034`), add up to 10 screenshots in display order, for example the six of one appearance. Apple names the medium display as the required iPhone size and scales a larger set for smaller sizes; before submitting, check that App Store Connect shows the iPhone requirement as met.
4. Under Apple Watch, add `watch-usage.png` and `watch-widget.png`, with the same watch size in every localization.
5. Save. Nothing in the repository uploads screenshots.

## Desktop GitHub releases

After TestFlight creates the version tag, `testflight` dispatches `.github/workflows/desktop-release.yml` from `master` with that tag and source commit; the workflow can also be dispatched by hand for a `master` commit. One publish job creates the macOS and Linux release, keeps signed assets on draft retries and only verifies a published release. A trusted-master job then verifies the published release and updates the two Collie definitions in the Homebrew tap; it needs its own token ([Publisher setup and release controls](desktop-releases.md#publisher-setup-and-release-controls)).

[Desktop release downloads](desktop-releases.md) covers the assets, the Developer ID and notary secrets, installation, Homebrew, the AUR recipes and the clean-machine acceptance checks.

## Third-party notices

`scripts/notices.py` writes the notices from the dependency graph at build time: cargo-about 0.9.2 for the Rust crates, go-licenses v2.0.1 for libtailscale's Go modules, the Go and Rust standard library licenses from the toolchains that build them, and the libghostty-vt license texts that `scripts/ghostty/build-xcframework.sh` collects from the pinned sources. It downloads cargo-about against its pinned SHA-256 and installs go-licenses with `go install`, which checks the Go checksum database, both into `target/tools`. A license that `about.toml` does not accept, or that cannot be identified, fails the build; accept a new license there and in `deny.toml`.

- `just ios-project` writes `Collie/Resources/Acknowledgements.json` (not committed), shown in Acknowledgements in Settings and on the Welcome screen: libtailscale and its Go modules for iOS, libghostty-vt and what it links, the JetBrains Mono and Symbols Nerd Font terminal fonts (license files committed beside the fonts), Octicons, the Rust standard library and CollieCore's Rust crates. The watch app and the extensions are pure Swift and link none of them.
- The desktop release workflow writes `THIRD_PARTY_NOTICES` in a build step before any signing secret is in the environment, since the script runs downloaded tools, and `scripts/desktop/package.py --notices` copies it into both collied archives. It lists libtailscale and its Go modules for macOS and Linux, LiteLLM's price table, the Rust standard library and collied's Rust crates for both targets.
- CollieBar and CollieTray contain no third-party code and carry no notices; the CollieTray archive includes the project `LICENSE`.

## Protocol compatibility

Revision 26 is the negotiated baseline and the supported floor. Installations through revision 25 matched versions exactly and need one update on each side; pairing keys stay valid. The baseline app sends a legacy-shaped hello so a revision-25 daemon can tell it to update collied. Later releases keep the current and previous two implemented negotiated revisions; [Negotiated protocol compatibility](architecture.md#negotiated-protocol-compatibility) has the codec rules every wire change must follow. Publish every floor increase here and in the release notes, once fixtures cover both upgrade orders and the removed revision's refusal.

When the app reports an incompatible daemon, install the latest release for the machine's operating system from [GitHub Releases](https://github.com/rbstp/collie/releases) with its installation instructions; a source build updates with `just collied-install`. Update an incompatible app through its distribution channel, then reopen it or pull to refresh. Protocol updates need no re-pairing.
