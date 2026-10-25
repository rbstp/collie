# Release

`.github/workflows/testflight.yml` builds, signs and uploads the iOS app to TestFlight. The Apple side is set up once, by hand.

## One-time Apple setup

Team `RM3UT3MMSR`, in the Apple Developer portal and App Store Connect:

1. **App Group**: register `group.dev.rbstp.collie`.
2. **App IDs** (explicit):
   - `dev.rbstp.collie`, the app, and `dev.rbstp.collie.push`, the ColliePush notification service extension, each with **Push Notifications** and **App Groups** (assign `group.dev.rbstp.collie`) enabled;
   - `dev.rbstp.collie.widgets`, the CollieWidgets extension (Live Activities), with **App Groups** (assign `group.dev.rbstp.collie`; the widget reads the notification key mirror there). No push capability: Live Activity pushes are addressed to the app's topic and use the same APNs key.
   - `dev.rbstp.collie.watchkitapp`, the CollieWatch app, and `dev.rbstp.collie.watchkitapp.widgets`, its usage complication, each with **App Groups** (assign `group.dev.rbstp.collie`; on the watch it is the watch's own container). No push capability and no keychain group.
3. **Provisioning profiles**: App Store Connect distribution, the team's Apple Distribution certificate, named exactly as `Collie/project.yml` and `Collie/ExportOptions.plist` refer to them:
   - **`Collie App Store`** for `dev.rbstp.collie`;
   - **`Collie Push App Store`** for `dev.rbstp.collie.push`;
   - **`Collie Widgets App Store`** for `dev.rbstp.collie.widgets`;
   - **`Collie Watch App Store`** for `dev.rbstp.collie.watchkitapp`;
   - **`Collie Watch Widgets App Store`** for `dev.rbstp.collie.watchkitapp.widgets`.

   A profile is a snapshot of its App ID's capabilities: after any capability change, regenerate the profile (Edit, Save) and update its secret.
4. **App Store Connect app record** for `dev.rbstp.collie`, with an internal TestFlight group.
5. **APNs key for collied, one per machine**: Certificates, Identifiers & Profiles, Keys, add a key with **Apple Push Notifications service (APNs)**, environment **Sandbox & Production**. Debug builds installed with `just ios-run-device` register sandbox tokens, TestFlight builds production tokens, and one collied serves both. Download the `.p8` (offered once) and note the Key ID. Each machine that runs collied gets its own key, with its own Key ID: it pushes on its own and is revoked on its own. The key is a runtime secret of the daemon on that machine, separate from the App Store Connect API key below, and is never stored in GitHub.

   On the Mac, it lives in the login Keychain, readable only by collied signed with your Developer ID Application identity:

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

   `collied setup` offers these steps when `[apns]` is not configured: it asks for the `.p8` path and the IDs (defaults `RM3UT3MMSR` and `dev.rbstp.collie`), writes the section, imports the key and restarts collied. The `.p8` must already be mode 0600.

   Import with the signed binary: the Keychain item trusts the program that created it. Re-run `just collied-install` (never `cargo install`) after every change to collied, or the daemon cannot read the key without a prompt. A config still using `key_path` is moved over by the same `collied apns import <key_path file>`, which rewrites that line to `key = "keychain"`. Revoke the key in the portal if the Mac is compromised.

   On Linux, it is a systemd user credential, encrypted with the host key (and the TPM2 when one is usable). There is no code signing:

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

   `collied setup` offers these steps on Linux too, the same way as on macOS; the import picks the credential or the 0600 fallback as below.

   Import writes the credential to `apns/<KEY_ID>.cred` in the data directory. systemd picks the seal, not collied: host key and TPM2 when a TPM2 is usable, host key only otherwise. Doctor's apns line says which, and warns when a TPM2 becomes usable after a host-only import. Import keeps an existing credential that decrypts to the same key, so to bind it to the TPM2, delete `apns/<KEY_ID>.cred` and import the `.p8` again. Where systemd-creds cannot encrypt (for example no `systemd-creds.socket`, systemd older than 256, a container), import falls back to a 0600 copy at `apns/AuthKey_<KEY_ID>.p8` in the data directory, sets `key_path` to it and says so. Doctor reports the fallback, and warns once systemd-creds works: import that file to encrypt it. The credential stops decrypting after an OS reinstall, a machine-id change, a uid or user name change, or a move to another machine: keep a copy of the `.p8` offline and import it again. Revoke the key in the portal if the machine is compromised.

## Menu bar app notarization (optional)

`just mac-install` signs the menu bar app with the Developer ID Application certificate and the hardened runtime; it runs on the Mac that built it without notarization. Nothing is needed in the developer portal: no App ID and no provisioning profile, as it has no entitlements.

To run it on another Mac without a Gatekeeper prompt, notarize it. notarytool needs one credential, stored once in the login keychain:

- an App Store Connect API team key with the Developer role (the TestFlight key works): `xcrun notarytool store-credentials collie --key AuthKey_<KEY_ID>.p8 --key-id <KEY_ID> --issuer <ISSUER_ID>`
- or an Apple ID app-specific password (appleid.apple.com): `xcrun notarytool store-credentials collie --apple-id <apple id> --team-id RM3UT3MMSR --password <app-specific password>`

The latest Program License Agreement must be accepted, or submissions are refused. Then `just mac-install` and `just mac-notarize` (or `just mac-notarize <profile>`), which submits, waits, staples the app in `target/mac` and in `~/Applications`, and checks it with `spctl`. On a failure, `xcrun notarytool log <submission id> --keychain-profile collie` says why.

## collied on Linux

There is no package. Install from a checkout, with the prerequisites in the README:

```sh
just setup                # submodules
just collied-install      # release build to ~/.cargo/bin/collied as a fresh inode
collied setup             # login, service install, doctor, APNs, pairing
```

`collied setup` runs `collied login` (advertises tag:collie-linux, refuses until the node has it), `collied service install` (systemd user unit, `~/.config/systemd/user/collied.service`), `collied doctor`, the APNs import and `collied pair`, each skipped when already done.

Update: `git pull`, `just setup`, `just collied-install`, which restarts the user unit if it is active, then `collied setup`, which keeps the node and the paired phones.

## Repository secrets

All repository-level, base64 values encoded with `base64 -i <file> | gh secret set <NAME>`:

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

- Merging a pull request into `master` ships a build, unless its title contains `[skip-release]` or it only touches `.github/`, `docs/` (the documentation site's sources, in `docs/site/`, included), `Mac/`, `README.md`, `AGENTS.md` or `CLAUDE.md`. Changes only to `docs/site/`, `docs/deploy-pages.md`, `README.md`, `AGENTS.md` and `CLAUDE.md` also skip the macOS `ci` jobs, on pull requests and on `master`; the site has its own workflow, `pages` ([deploy-pages.md](deploy-pages.md)).
- Version: the latest `v*` tag bumped by minor when any commit since that tag (or the pull request title) is a `feat`, by patch otherwise. Build number: the workflow run number.
- A manual run (Actions, testflight, Run workflow, on `master` only) ships the given version, for example `1.0.0`.
- After the upload, the workflow pushes the tag `v<version>`, then starts the desktop release and the [documentation screenshots](#documentation-screenshots) refresh.

## iOS release notes

Use [iOS release notes](ios-release-notes.md) for the next combined release. Copy the release summary and tested-support disclosure into the App Store version description or What's New, and the test checklist into TestFlight's What to Test. Keep the tested-support disclosure in future release metadata until broader testing is recorded. For the first App Store release, put it in Description: Apple does not provide What's New until subsequent versions ([platform version information](https://developer.apple.com/help/app-store-connect/reference/app-information/platform-version-information)).

The upload workflow does not publish this metadata. Confirm the text in App Store Connect before distributing the build. A successful upload does not establish that the build is processed and installable.

## App icon and TestFlight

- The icon is a single 1024x1024 universal image, `Collie/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon.png`; Xcode derives every other size. The app's `Info.plist` sets `CFBundleIconName` to `AppIcon` and `project.yml` sets `ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon`: App Store validation rejects an upload whose icon is only in the asset catalog without `CFBundleIconName`.

The iOS app embeds the watch app, so every iOS build, test and archive also builds for watchOS: both workflows install the watchOS platform on the `xcode-27` runner when it is missing. The watch app has its own copy of the icon, `Collie/CollieWatch/Assets.xcassets/AppIcon.appiconset/AppIcon.png`.

## Debug symbols

Before each TestFlight upload, the workflow retains an `ios-symbols-<version>-<build>-<attempt>` Actions artifact for 90 days. It contains only the archive's dSYM bundles and a `build.txt` manifest with the source commit, app version/build and symbol UUIDs. It excludes the app archive, provisioning profiles and signing credentials. Download the artifact from that exact workflow run and keep a private copy if support is needed beyond its retention period.

Match the app UUID in a device report to `build.txt` or `xcrun dwarfdump --uuid <app.dSYM>` before symbolicating. Do not substitute symbols from another build or a rebuilt binary unless its UUID matches. Device reports and captured logs stay private; do not attach them to public issues or release artifacts.

## Export compliance

This is not legal advice; confirm with counsel before an App Store release.

The app implements standard encryption on top of Apple's: WireGuard (Curve25519, ChaCha20-Poly1305) in libtailscale and TLS 1.3 (rustls with ring) to collied. Notification payloads use CryptoKit. There is no proprietary or [non-standard](https://www.ecfr.gov/current/title-15/subtitle-B/chapter-VII/subchapter-C/part-772/section-772.1) cryptography, and the source is public.

- **US (EAR)**: once generally available on the App Store, mass market under Note 3 to Category 5 Part 2, self-classified as 5D992.c under [15 CFR 740.17(b)(1)](https://www.ecfr.gov/current/title-15/subtitle-B/chapter-VII/subchapter-C/part-740/section-740.17). No CCATS. Since the 2021 rule (86 FR 16482), the annual self-classification report of 740.17(e)(3) covers components and certain executable software, not an end-item app like this one, so none is filed; this section is the classification record. If one is ever filed, it is due February 1 for the previous year, by email to `crypt-supp8@bis.doc.gov` and `enc@nsa.gov`.
- **Apple**: `Info.plist` sets [`ITSAppUsesNonExemptEncryption`](https://developer.apple.com/documentation/bundleresources/information-property-list/itsappusesnonexemptencryption) to `false`. Apple's "exempt" means exempt from uploading documentation: standard algorithms not provided by the OS need a document only for the App Store in France ([Apple's table](https://developer.apple.com/help/app-store-connect/reference/app-information/export-compliance-documentation-for-encryption)). The key answers the encryption question for every build, so TestFlight uploads never stop at Missing Compliance. Never set `true` without the matching [`ITSEncryptionExportComplianceCode`](https://developer.apple.com/documentation/bundleresources/information-property-list/itsencryptionexportcompliancecode): builds then stop at Missing Compliance, and a code Apple did not issue can fail the upload.
- **Internal TestFlight** (today): builds reach only the owner in Canada, and [Apple](https://developer.apple.com/documentation/security/complying-with-encryption-export-regulations) applies US export law only to distribution outside the US or Canada. Nothing else to do.

Decision (owner, 2026-10-20): App Store availability is every country or region except France.

Before an App Store release:

1. In App Store Connect, Pricing and Availability, App Availability, choose specific countries or regions and leave out France; All Countries or Regions includes it.
2. If App Store Connect asks about encryption (App Information, App Encryption Documentation, or Manage next to a build), answer: uses encryption, yes; algorithms, "Standard encryption algorithms instead of, or in addition to, using or accessing the encryption within Apple's operating system"; available on the App Store in France, no. No document is needed.
3. To ship in France instead: file a declaration with [ANSSI](https://cyber.gouv.fr/reglementation/reglementation-identite-confiance-numerique/controles-reglementaires-cryptographie/controle-moyen-de-cryptologie/) (up to a month), fill in the App Description and availability, [upload the declaration](https://developer.apple.com/help/app-store-connect/manage-app-information/determine-and-upload-app-encryption-documentation) under App Encryption Documentation with France set to yes, wait for Apple's approval, then in one commit set `ITSAppUsesNonExemptEncryption` to `true` and `ITSEncryptionExportComplianceCode` to the key value Apple shows next to the approved document.

An external TestFlight group is not limited by App Store availability, so leaving out France does not keep its testers out of France. Apple's France requirement names only the App Store; until that is settled, invite no external testers located in France. Apple's TestFlight help documents no country or region setting for external testing: a public link's tester criteria cover devices and OS versions ([Invite external testers](https://developer.apple.com/help/app-store-connect/test-a-beta-version/invite-external-testers/), [TestFlight overview](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview/)), and availability by country or region applies to the App Store ([Manage availability for your app on the App Store](https://developer.apple.com/help/app-store-connect/manage-your-apps-availability/manage-availability-for-your-app-on-the-app-store/)). A public link therefore cannot keep out testers in France.

## Adding a signed extension target

CollieWidgets went in this way. `Collie/ExportOptions.plist` maps each bundle ID to its profile name under `provisioningProfiles` (`dev.rbstp.collie.widgets` to `Collie Widgets App Store`), and the workflow installs `APPLE_PROVISIONING_PROFILE_WIDGETS` with the others.

1. Register its App ID (with the capabilities its entitlements use) and an App Store profile for it.
2. Add its secret, for example `APPLE_PROVISIONING_PROFILE_<NAME>`, to the `env` of the "check for the signing secrets" and "import the signing certificate and profiles" steps; every `APPLE_PROVISIONING_PROFILE*` variable is checked and installed.
3. Map its bundle ID to the profile name in `Collie/ExportOptions.plist` and set `PROVISIONING_PROFILE_SPECIFIER` in its Release settings in `Collie/project.yml`.

## Public submission

[Public submission preparation](public-submission.md) contains the offline metadata and disclosure checks for #359, [App Review notes](app-review.md) the review notes and submission checklist, and the [relay runbook](relay-runbook.md) the operating procedures. Public capacity and submission remain unapproved until its evidence gates pass.

## App Review sample workspace

Review notes should direct reviewers to **Explore sample workspace** on Welcome, with no account or desktop required. Existing users can open the same sample from Settings. Ask reviewers to inspect Agents, Chat, Terminal, Usage and Machines, send a sample prompt, try a terminal key, then approve or deny the blocked sample agent. The persistent sample banner identifies every action as simulated. Reset is in sample Settings and Exit returns to onboarding or the existing paired setup.

The sample is available in Release builds on iOS Simulator and iPhone. It does not demonstrate real pairing, background APNs delivery, Live Activities or device attestation. Include real-hardware review evidence for those workflows. Obtain any required prior App Review approval for using the sample instead of credentials before submission; sample availability does not establish that approval. Record the actual Release simulator and signed-device validation with the submission evidence.

## Documentation screenshots

After a successful upload, `testflight` dispatches `.github/workflows/docs-screenshots.yml` from `master` with the shipped commit, so the images in `docs/site/src/images` (also shown by the README) match the app that shipped. It runs separately from the release: a failure shows as a failed `docs screenshots` run and never fails or holds back the build, the tag or the desktop release. It is in the `docs-screenshots` concurrency group, so a second release waits for the first refresh to finish.

1. **capture**, on `xcode-27` with read-only contents and no secret: builds the simulator framework, creates an iPhone 18 Pro Max simulator if the runner has none, installs pngquant 3.0.3 from pngquant.org (checked against its SHA-256), runs `just screenshot_watch=false ios-screenshots` and keeps the images as the `screenshots` artifact for 7 days. It refuses a commit that is not on `master`, and it only restores caches, never saves them, so a refresh cannot change what `testflight` and `ci` restore.
2. **propose**, on `master` only, in the `docs-screenshots` environment: copies the artifact's PNG files onto `master` and fails on any other file. When nothing changed, it stops. Otherwise it force-pushes them to the `docs-screenshots` branch, opens (or reuses) the pull request "Refresh documentation screenshots [skip-release]" and enables auto-merge with squash. The pull request runs the normal checks (`ci` skips `rust` and `ios` for a `docs/site/` change), the merge ships no build, and its push to `master` deploys the site through `pages`.

Pull requests, pushes and merges made with `GITHUB_TOKEN` start no workflow, and the repository does not let Actions create pull requests, so **propose** uses a token of the owner instead. Before the next release, create a `docs-screenshots` environment in `rbstp/collie`, restrict deployments to `master`, and add `DOCS_SCREENSHOTS_TOKEN`: a fine-grained personal access token owned by the owner, limited to `rbstp/collie`, with **Contents: Read and write** and **Pull requests: Read and write** (metadata read is implicit), and an expiration date. It needs no workflow permission. Keep the environment restricted to `master`: the owner's role bypasses the `master` ruleset, so a leaked token could push any file outside `.github/workflows/` to `master`. Renew the token before it expires. Enter it at the prompt, never in a command argument or committed file:

```sh
gh secret set DOCS_SCREENSHOTS_TOKEN --repo rbstp/collie --env docs-screenshots
```

The token is exposed only to **propose**, which never builds or runs the captured revision. Its commit is authored as the token's owner. Without the secret, **propose** fails with a message pointing here, after **capture** has kept the images.

The workflow refreshes only the iPhone screenshots. The watchOS simulator runtime on the `xcode-27` runner loses its files minutes after it is mounted, so a watch there does not stay booted through the capture ([#539](https://github.com/rbstp/collie/issues/539)). `watch-usage.png` and `watch-widget.png` stay as committed. When the watch app changes, the owner refreshes them locally with `just ios-screenshots` and commits them.

To refresh without a release, run it by hand (Actions, docs screenshots, Run workflow, on `master`), optionally with the commit to capture on `master`; the default is the head of `master`.

The demo's usage reset times and the watch's clock follow the capture time, so `usage-light.png` and `usage-dark.png` change on every run, and every release opens a refresh pull request. A local `just ios-screenshots` changes the two watch images too.

## App Store screenshots

`just ios-screenshots` also writes the App Store set to `target/ios/app-store` and prints its path. Git ignores it and the `docs screenshots` workflow does not keep it. The set is the same capture as the documentation images, before resizing and compression, as PNG without alpha, which App Store Connect requires:

- the twelve iPhone screenshots (Agents, Usage, New task, Chat, Terminal and an approval, light and dark) at 1320 x 2868, the native size of the iPhone 18 Pro Max simulator and a portrait size of iPhone with Dynamic Island (large display) (Apple's page says iPhone Duo screenshots become required in April 2027 for apps built with the iOS 27.1 SDK or later);
- `watch-usage.png` and `watch-widget.png` at 416 x 496, the native size of the Apple Watch Series 12 (46mm) simulator and the accepted size for Series 10, 11 and 12.

Both sizes come from Apple's [screenshot specifications](https://developer.apple.com/help/app-store-connect/reference/screenshot-specifications); check them again before a submission.

For version 0.1.8, App Store Connect first asked this app for an iPhone 6.5" Display set (1242 x 2688 or 1284 x 2778), then for iPhone with Dynamic Island (medium display) (1206 x 2622 or 1179 x 2556). `just screenshot_device="<simulator>" ios-screenshots` captures the same twelve screenshots on that simulator, writes only the App Store set, to `target/ios/app-store/<width>x<height>`, and leaves the documentation images and the watch alone. `just screenshot_device="iPhone 18 Pro" ios-screenshots` gives the 1206 x 2622 medium display set. On an iPhone 14 Plus simulator (`xcrun simctl create "iPhone 14 Plus" com.apple.CoreSimulator.SimDeviceType.iPhone-14-Plus`) that is 1284 x 2778. iPhone 11 Pro Max gives 1242 x 2688; iPhone Xs Max cannot be created on the iOS 27.2 runtime.

The iPhone Duo set needs the iOS 27.1 simulator runtime, because the iOS 27.2 runtime refuses to create an iPhone Duo. Download it once with `xcodebuild -downloadPlatform iOS -buildVersion 27.1 -architectureVariant arm64` (7.9 GB), create the simulator with `xcrun simctl create "iPhone Duo" com.apple.CoreSimulator.SimDeviceType.iPhone-Duo com.apple.CoreSimulator.SimRuntime.iOS-27-1`, then run `just screenshot_device="iPhone Duo" ios-screenshots`, which writes the set to `target/ios/app-store/1398x2034`. That is the native size of the cover display and one of the two portrait sizes App Store Connect accepts for iPhone Duo. The inner display (2007 x 2853) cannot be captured with Xcode 27.2: the simulator boots folded, and neither Xcode nor `simctl` can unfold it. Collie fills the cover display; the system keeps a strip along its trailing edge for the status bar, the navigation bar buttons and the tab bar, and the cover display does not rotate.

Locally, the default run needs an available simulator named `iPhone 18 Pro Max` paired with a watch simulator (`xcrun simctl pair <watch> <phone>`). `just screenshot_watch=false ios-screenshots` skips the watch, as the workflow does, and writes no watch screenshots to the set. The recipe reboots the phone simulator for each appearance, dark then light, because the iOS 27.2 simulator applies an appearance change only at boot. At the end it sets the simulator's original appearance back, which a simulator that started dark shows from its next boot. The phone shows the overridden 9:41 status bar; the watch shows the time of the capture, because watchOS has no status bar override. The default run deletes `target/ios/app-store` first, so run it before any `screenshot_device` run.

To upload:

1. Run `just ios-screenshots`, then `just screenshot_device="<simulator>" ios-screenshots` for each other size App Store Connect lists, on the commit being submitted, so the screenshots match the binary.
2. In App Store Connect, open the app, select the iOS version being prepared and open **Previews and Screenshots**.
3. Under iPhone, in each size App Store Connect lists (iPhone with Dynamic Island (large display) from `target/ios/app-store`, medium display from `target/ios/app-store/1206x2622`, 6.5" from `target/ios/app-store/1284x2778`, iPhone Duo from `target/ios/app-store/1398x2034`), add up to 10 screenshots in display order, for example the six of one appearance. Apple names iPhone with Dynamic Island (medium display) as the required iPhone size and fills the smaller sizes with scaled screenshots of a larger one; before submitting, check that App Store Connect shows the iPhone requirement as met.
4. Under Apple Watch, add `watch-usage.png` and `watch-widget.png`. Keep the same watch size in every localization.
5. Save. Nothing in the repository uploads screenshots.

## Desktop GitHub releases

The reusable desktop workflow is called explicitly after TestFlight creates its immutable version tag. After desktop publication, its trusted-master Homebrew job verifies the public artifacts and updates the two Collie tap definitions automatically; provision its separate token as described in [Desktop release downloads](desktop-releases.md#publisher-setup-and-release-controls). It also supports a master-only manual dispatch at an exact master SHA. One publish job owns the shared macOS/Linux release, retains signed assets on draft retries, and verifies completed releases without replacing assets. This avoids GITHUB_TOKEN tag-push suppression and duplicate release owners.

See [Desktop release downloads](desktop-releases.md) for publisher Developer ID/notary secrets, notarized macOS archives and disk image, Linux runtime manifests, Homebrew tap artifacts, install/refresh commands and the clean-machine rollout gates. No signing credentials, public release or Homebrew tap update is provisioned by adding the workflow.

## Third-party notices

`scripts/notices.py` writes the notices from the dependency graph at build time: cargo-about 0.9.2 for the Rust crates, go-licenses v2.0.1 for libtailscale's Go modules, the Go and Rust standard library licenses from the toolchains that build them, and the license texts `scripts/ghostty/build-xcframework.sh` collects for libghostty-vt from the pinned sources. It downloads cargo-about against its pinned SHA-256 and installs go-licenses with `go install`, which checks the Go checksum database, both into `target/tools`. A license that `about.toml` does not accept, or that cannot be identified, fails the build; accept a new license there and in `deny.toml`.

- `just ios-project` writes `Collie/Resources/Acknowledgements.json` (not committed), which the app shows in Acknowledgements, in Settings and on the Welcome screen: libtailscale and its Go modules for iOS, libghostty-vt and what it links, the Meslo font with its DejaVu base and Nerd Fonts glyphs (license files committed beside the fonts), Octicons, the Rust standard library and the Rust crates in CollieCore. The watch app and the extensions are pure Swift and link none of them.
- The desktop release workflow writes `THIRD_PARTY_NOTICES` in its build steps, before any signing secret is in the environment, since the script runs downloaded tools; `scripts/desktop/package.py --notices` copies it into both collied archives. It lists libtailscale and its Go modules for macOS and Linux, LiteLLM's price table, the Rust standard library and collied's Rust crates for both targets.
- CollieBar and CollieTray contain no third-party code, so their archives carry only the project `LICENSE`.

## Protocol compatibility

The negotiated baseline is revision 26, with supported floor 26. Existing exact-match installations through revision 25 need one update on each side; pairing keys stay valid. The baseline app uses a legacy-shaped hello so a revision-25 daemon can tell it to update collied. Subsequent releases retain the current and previous two implemented negotiated revisions. Publish every floor increase here and in the release notes, after fixtures cover both upgrade orders and the removed revision's refusal. A range alone does not establish compatibility: wire codecs must gate methods, parameters, response/event fields and enum variants, with separate negotiated capabilities for new relay grant and encrypted payload formats.

When an installed app reports an incompatible daemon, download the latest release for the machine's operating system from [GitHub Releases](https://github.com/rbstp/collie/releases) and follow its installation instructions. Source builds can still use `just collied-install`. Update an incompatible app through its distribution channel, then reopen it or pull to refresh. Protocol updates do not require re-pairing.
