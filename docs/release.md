# Release

`.github/workflows/testflight.yml` builds, signs and uploads the iOS app to TestFlight. The Apple side is set up once, by hand.

## One-time Apple setup

In the Apple Developer portal and App Store Connect, team `RM3UT3MMSR`:

1. Register the App Group `group.dev.rbstp.collie`.
2. Register each App ID below as an explicit App ID, with **App Groups** enabled, `group.dev.rbstp.collie` assigned and the listed capabilities.
3. Create each profile below as an App Store Connect distribution profile with the team's Apple Distribution certificate. The names must match `Collie/project.yml` and `Collie/ExportOptions.plist`.
4. Create the App Store Connect app record for `dev.rbstp.collie`, with an internal TestFlight group.

| App ID | Target | Other capabilities | Profile |
| --- | --- | --- | --- |
| `dev.rbstp.collie` | the app | Push Notifications, App Attest | `Collie App Store` |
| `dev.rbstp.collie.push` | ColliePush (notification service extension) | Push Notifications | `Collie Push App Store` |
| `dev.rbstp.collie.widgets` | CollieWidgets (Live Activities) | none | `Collie Widgets App Store` |
| `dev.rbstp.collie.watchkitapp` | CollieWatch | none | `Collie Watch App Store` |
| `dev.rbstp.collie.watchkitapp.widgets` | the watch's usage complication | none | `Collie Watch Widgets App Store` |

- CollieWidgets needs no push capability: Live Activity pushes use the app's topic and the same APNs key.
- On the watch, the group is the watch's own container, and there is no keychain group.
- A profile is a snapshot of its App ID's capabilities: after any capability change, regenerate it (Edit, Save) and update its secret.

## APNs key for direct APNs

A machine on the publisher relay needs no key. Give each machine its own key, so you can revoke a compromised machine's key alone. The key is not the App Store Connect API key and never goes into GitHub.

In Certificates, Identifiers & Profiles, Keys, add a key with **Apple Push Notifications service (APNs)** and environment **Sandbox & Production**, so one collied serves debug builds from `just ios-run-device` and TestFlight builds. Download the `.p8` (offered once) and note the Key ID.

With no push mode configured, `collied setup --direct-apns` does the rest, asking for the `.p8` (already mode 0600) and the IDs. By hand:

```toml
# macOS: ~/Library/Application Support/collie/collied.toml
# Linux: ~/.local/share/collie/collied.toml ($XDG_DATA_HOME/collie/collied.toml when absolute)
[apns]
key_id = "<KEY_ID>"
team_id = "RM3UT3MMSR"
bundle_id = "dev.rbstp.collie"
```

```sh
just collied-install                      # signed on macOS; restarts a loaded agent or an active unit
chmod 0600 AuthKey_<KEY_ID>.p8
collied apns import AuthKey_<KEY_ID>.p8   # stores the key, reads it back, offers to delete the file
collied doctor                            # apns line (and codesign on macOS) must be ok
collied apns test                         # needs a paired phone
```

`key` defaults to `"keychain"` on macOS and `"systemd-creds"` on Linux.

- **macOS**: the login Keychain item trusts the program that created it, so only collied signed with your Developer ID Application identity reads it without a prompt. Import with the signed binary, and after every change to collied reinstall with `just collied-install`, never `cargo install`.
- **Linux**: the key is a systemd user credential sealed with the host key, plus the TPM2 when one is usable, or a 0600 file where systemd-creds cannot encrypt ([APNs key on Linux](architecture.md#apns-key-on-linux)). The credential stops decrypting after an OS reinstall, a machine-id change, a uid or user name change, or a move to another machine: keep a copy of the `.p8` offline to import again.

## Menu bar app notarization (optional)

`just mac-install` signs the menu bar app with the Developer ID Application certificate and the hardened runtime. It has no entitlements, so it needs no App ID or provisioning profile, and runs unnotarized on the Mac that built it. To run it on another Mac without a Gatekeeper prompt, notarize it:

1. Accept the latest Program License Agreement, or Apple refuses submissions.
2. Store one notarytool credential in the login keychain, from an App Store Connect API team key with the Developer role (the TestFlight key works) or an Apple ID app-specific password (appleid.apple.com):
   - `xcrun notarytool store-credentials collie --key AuthKey_<KEY_ID>.p8 --key-id <KEY_ID> --issuer <ISSUER_ID>`
   - `xcrun notarytool store-credentials collie --apple-id <apple id> --team-id RM3UT3MMSR --password <app-specific password>`
3. Run `just mac-install`, then `just mac-notarize` (or `just mac-notarize <profile>`) to submit, wait, staple the app in `target/mac` and `~/Applications`, and check it with `spctl`. On a failure, `xcrun notarytool log <submission id> --keychain-profile collie` says why.

## Repository secrets

Set these repository secrets, encoding the base64 ones with `base64 -i <file> | gh secret set <NAME>`:

| Secret | Content |
| --- | --- |
| `APPLE_DIST_CERT_P12` | Apple Distribution certificate and key, `.p12`, base64 |
| `APPLE_DIST_CERT_PASSWORD` | password of that `.p12` |
| `APPLE_PROVISIONING_PROFILE`, `_PUSH`, `_WIDGETS`, `_WATCH`, `_WATCH_WIDGETS` | the five [profiles](#one-time-apple-setup) in that order, `.mobileprovision`, base64 |
| `APPLE_KEY_P8` | App Store Connect API key (Developer role), `.p8`, base64 |
| `APPLE_KEY_ID` | that key's ID |
| `APPLE_ISSUER_ID` | App Store Connect issuer ID |

The certificate and profiles expire after a year; renew them and update their secrets.

## Cutting a release

Keep published iOS, collied, CollieBar and CollieTray releases on the same version, tag and source commit, including UI-only changes.

- Merging a pull request into `master` ships a build unless its title contains `[skip-release]` or it changes only `.github/`, `docs/` (the site included), `Mac/`, `README.md`, `AGENTS.md` or `CLAUDE.md`. The site deploys separately ([deploy-pages.md](deploy-pages.md)).
- Version: the latest `v*` tag with its minor number bumped when any commit since that tag, or the pull request title, is a `feat`, otherwise its patch number. Build number: the workflow run number.
- A manual run (Actions, testflight, Run workflow, on `master` only) ships the given version, for example `1.0.0`.
- After the upload, the workflow creates the tag `v<version>` and starts the [desktop release](#desktop-github-releases) and the [documentation screenshots](#documentation-screenshots) refresh.

## iOS release notes

Copy the release notes and tested-support disclosure from [iOS release notes](ios-release-notes.md) into the App Store version's What's New, or into Description for the first App Store version, which has no What's New ([platform version information](https://developer.apple.com/help/app-store-connect/reference/app-information/platform-version-information)). Copy its test checklist into TestFlight's What to Test. Keep the disclosure in release metadata until broader testing is recorded.

The upload workflow publishes none of this, and a successful upload does not mean the build is processed and installable: check both in App Store Connect before distributing the build.

## App icon and TestFlight

- The icon is a single 1024x1024 universal image, `Collie/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon.png`, from which Xcode derives every other size; the watch app has its own copy, `Collie/CollieWatch/Assets.xcassets/AppIcon.appiconset/AppIcon.png`. App Store validation rejects an asset-catalog icon without `CFBundleIconName`, so `Info.plist` sets `CFBundleIconName` to `AppIcon` and `project.yml` sets `ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon`.
- The iOS app embeds the watch app, so every iOS build, test and archive also builds for watchOS, and `ci` and `testflight` install the watchOS platform when the runner lacks it.

## Debug symbols

Before each TestFlight upload, the workflow keeps an `ios-symbols-<version>-<build>-<attempt>` Actions artifact for 90 days: the archive's dSYM bundles and a `build.txt` with the source commit, app version and build, and symbol UUIDs, with no app archive, profile or signing credential. Download it from that run; keep a private copy if support may outlast it.

Before symbolicating, match the app UUID in a device report to `build.txt` or `xcrun dwarfdump --uuid <app.dSYM>`; never use symbols from another build, or from a rebuilt binary whose UUID differs. Keep device reports and captured logs private, out of public issues and release artifacts.

## Export compliance

This is not legal advice; confirm with counsel before an App Store release.

Beyond Apple's own encryption, the app uses standard cryptography: WireGuard (Curve25519, ChaCha20-Poly1305) in libtailscale, TLS 1.3 (rustls with ring) to collied, and CryptoKit for notification payloads. It has no proprietary or [non-standard](https://www.ecfr.gov/current/title-15/subtitle-B/chapter-VII/subchapter-C/part-772/section-772.1) cryptography, and the source is public.

- **US (EAR)**: once generally available on the App Store, mass market under Note 3 to Category 5 Part 2, self-classified as 5D992.c under [15 CFR 740.17(b)(1)](https://www.ecfr.gov/current/title-15/subtitle-B/chapter-VII/subchapter-C/part-740/section-740.17). No CCATS. Since the 2021 rule (86 FR 16482), the annual self-classification report of 740.17(e)(3) covers components and certain executable software, not an end-item app like this one, so none is filed; this section is the classification record. A report, if ever filed, is due February 1 for the previous year, by email to `crypt-supp8@bis.doc.gov` and `enc@nsa.gov`.
- **Apple**: `Info.plist` sets [`ITSAppUsesNonExemptEncryption`](https://developer.apple.com/documentation/bundleresources/information-property-list/itsappusesnonexemptencryption) to `false`. Apple's "exempt" means exempt from uploading documentation: standard algorithms not provided by the OS need a document only for the App Store in France ([Apple's table](https://developer.apple.com/help/app-store-connect/reference/app-information/export-compliance-documentation-for-encryption)). The key answers the encryption question for every build, so TestFlight uploads never stop at Missing Compliance. Never set it to `true` without the matching [`ITSEncryptionExportComplianceCode`](https://developer.apple.com/documentation/bundleresources/information-property-list/itsencryptionexportcompliancecode): builds then stop at Missing Compliance, and a code Apple did not issue can fail the upload.
- **Internal TestFlight** (today): builds reach only the owner in Canada, and [Apple](https://developer.apple.com/documentation/security/complying-with-encryption-export-regulations) applies US export law only to distribution outside the US or Canada. Nothing else to do.

Decision (owner, 2026-10-20): App Store availability is every country or region except France.

Before an App Store release:

1. In App Store Connect, Pricing and Availability, App Availability, choose specific countries or regions and leave out France; All Countries or Regions includes it.
2. If App Store Connect asks about encryption (App Information, App Encryption Documentation, or Manage next to a build), answer: uses encryption, yes; algorithms, "Standard encryption algorithms instead of, or in addition to, using or accessing the encryption within Apple's operating system"; available on the App Store in France, no. No document is needed.
3. To ship in France instead: file a declaration with [ANSSI](https://cyber.gouv.fr/reglementation/reglementation-identite-confiance-numerique/controles-reglementaires-cryptographie/controle-moyen-de-cryptologie/) (up to a month), fill in the App Description and availability, [upload the declaration](https://developer.apple.com/help/app-store-connect/manage-app-information/determine-and-upload-app-encryption-documentation) under App Encryption Documentation with France set to yes, and wait for Apple's approval. Then, in one commit, set `ITSAppUsesNonExemptEncryption` to `true` and `ITSEncryptionExportComplianceCode` to the key value Apple shows next to the approved document.

Leaving out France does not keep external TestFlight testers out of France. Apple documents no country or region setting for external testing, only tester criteria for devices and OS versions ([Invite external testers](https://developer.apple.com/help/app-store-connect/test-a-beta-version/invite-external-testers/), [TestFlight overview](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview/)); availability by country or region applies to the App Store ([Manage availability](https://developer.apple.com/help/app-store-connect/manage-your-apps-availability/manage-availability-for-your-app-on-the-app-store/)). Apple's France requirement names only the App Store; until that is settled, invite no external testers located in France.

## Adding a signed extension target

1. Register its App ID, with the capabilities its entitlements use, and an App Store profile for it.
2. Add its secret, for example `APPLE_PROVISIONING_PROFILE_<NAME>`, to the `env` of the "check for the signing secrets" and "import the signing certificate and profiles" steps; they check and install every `APPLE_PROVISIONING_PROFILE*` variable.
3. Map its bundle ID to the profile name under `provisioningProfiles` in `Collie/ExportOptions.plist`, and set `PROVISIONING_PROFILE_SPECIFIER` in its Release settings in `Collie/project.yml`.

## Public submission

[Public submission preparation](public-submission.md) has the release gates and offline metadata and disclosure checks for #359, which records the submission results. [App Review notes](app-review.md) has the review notes, the real-device video and the [submission checklist](app-review.md#submission-checklist), which gates the public TestFlight link. The [relay runbook](relay-runbook.md) has the operating procedures.

## Documentation screenshots

After a successful upload, `testflight` dispatches `.github/workflows/docs-screenshots.yml` from `master` with the shipped commit, so the images in `docs/site/src/images` match the shipped app. A failed refresh never affects the build, the tag or the desktop release.

1. **capture**, on `xcode-27` with read-only contents and no secret, refuses a commit that is not on `master`, runs `just screenshot_watch=false ios-screenshots` and keeps the images as the `screenshots` artifact for 7 days. It restores caches but never saves them, so it cannot change what `testflight` and `ci` restore.
2. **propose**, on `master` only, in the `docs-screenshots` environment, copies the artifact's PNG files onto `master`, failing on any other file, and stops when nothing changed. Otherwise it force-pushes them to the `docs-screenshots` branch and opens or reuses the pull request "Refresh documentation screenshots [skip-release]" with squash auto-merge. After the normal checks, the merge ships no build and deploys the site.

Pull requests, pushes and merges made with `GITHUB_TOKEN` start no workflow, and the repository does not let Actions create pull requests, so **propose** uses `DOCS_SCREENSHOTS_TOKEN` from the `docs-screenshots` environment: a fine-grained personal access token of the owner, limited to `rbstp/collie`, with **Contents: Read and write** and **Pull requests: Read and write**, an expiration date and no workflow permission. Keep the environment restricted to `master`: the owner's role bypasses the `master` ruleset, so a leaked token could push any file outside `.github/workflows/` to `master`. Renew the token before it expires, and enter it at the prompt, never in a command argument or committed file:

```sh
gh secret set DOCS_SCREENSHOTS_TOKEN --repo rbstp/collie --env docs-screenshots
```

Only **propose** sees the token, and it never builds or runs the captured revision. Without the secret, **propose** fails with a message pointing here, after **capture** has kept the images.

Only the iPhone screenshots refresh, since the runner's watchOS simulator runtime loses its files minutes after it is mounted ([#539](https://github.com/rbstp/collie/issues/539)). When the watch app changes, the owner refreshes `watch-usage.png` and `watch-widget.png` with a local `just ios-screenshots` and commits them.

To refresh without a release, run the workflow by hand (Actions, docs screenshots, Run workflow, on `master`), optionally with a `master` commit to capture instead of the head. Usage reset times and the watch's clock follow the capture time, so `usage-light.png` and `usage-dark.png` change on every run and every release opens a refresh pull request; a local run also changes the watch images.

## App Store screenshots

`just ios-screenshots` also writes the App Store set to `target/ios/app-store`, which Git ignores and the workflow does not keep: the capture before resizing and compression, as PNG without alpha, which App Store Connect requires:

- the twelve iPhone screenshots (Agents, Usage, New task, Chat, Terminal and an approval, light and dark) at 1320 x 2868, the iPhone 18 Pro Max simulator's native size and a portrait size of iPhone with Dynamic Island (large display);
- `watch-usage.png` and `watch-widget.png` at 416 x 496, the Apple Watch Series 12 (46mm) simulator's native size, accepted for Series 10, 11 and 12.

Check both sizes against Apple's [screenshot specifications](https://developer.apple.com/help/app-store-connect/reference/screenshot-specifications) before submitting; that page says iPhone Duo screenshots become required in April 2027 for apps built with the iOS 27.1 SDK or later.

The default run needs an `iPhone 18 Pro Max` simulator paired with a watch simulator (`xcrun simctl pair <watch> <phone>`); `screenshot_watch=false` skips the watch. It deletes `target/ios/app-store` first, so run it before any `screenshot_device` run.

For version 0.1.8, App Store Connect first asked for an iPhone 6.5" Display set (1242 x 2688 or 1284 x 2778), then for iPhone with Dynamic Island (medium display) (1206 x 2622 or 1179 x 2556). `just screenshot_device="<simulator>" ios-screenshots` captures only the twelve iPhone screenshots, on that simulator, into `target/ios/app-store/<width>x<height>`: `iPhone 18 Pro` gives 1206 x 2622, an iPhone 14 Plus simulator (`xcrun simctl create "iPhone 14 Plus" com.apple.CoreSimulator.SimDeviceType.iPhone-14-Plus`) 1284 x 2778, and iPhone 11 Pro Max 1242 x 2688. The iOS 27.2 runtime cannot create an iPhone Xs Max or an iPhone Duo; the iPhone Duo set needs the iOS 27.1 runtime, a one-time 7.9 GB download:

```sh
xcodebuild -downloadPlatform iOS -buildVersion 27.1 -architectureVariant arm64
xcrun simctl create "iPhone Duo" com.apple.CoreSimulator.SimDeviceType.iPhone-Duo com.apple.CoreSimulator.SimRuntime.iOS-27-1
just screenshot_device="iPhone Duo" ios-screenshots
```

The set lands in `target/ios/app-store/1398x2034`, the cover display's native size and one of the two portrait sizes App Store Connect accepts for iPhone Duo. Xcode 27.2 cannot capture the inner display (2007 x 2853): the simulator boots folded, and neither Xcode nor `simctl` unfolds it.

Upload them by hand:

1. On the commit being submitted, so the screenshots match the binary, run `just ios-screenshots`, then a `screenshot_device` run for each other size App Store Connect lists.
2. In App Store Connect, open the iOS version being prepared, then **Previews and Screenshots**. Under iPhone, for each listed size, add up to 10 screenshots from its folder in display order, for example the six of one appearance. Apple names the medium display as the required iPhone size and scales a larger set for smaller sizes; before submitting, check that App Store Connect shows the iPhone requirement as met.
3. Under Apple Watch, add `watch-usage.png` and `watch-widget.png`, with the same watch size in every localization, then save.

## Desktop GitHub releases

After TestFlight creates the version tag, `testflight` dispatches `.github/workflows/desktop-release.yml` from `master` with that tag and source commit. [Desktop release downloads](desktop-releases.md) covers the workflow and its secrets, assets, installation, Homebrew, AUR recipes and acceptance checks.

## Third-party notices

`scripts/notices.py` builds the notices from the dependency graph: cargo-about 0.9.2 (checked against its pinned SHA-256) for the Rust crates, go-licenses v2.0.1 (verified through the Go checksum database) for libtailscale's Go modules, the Go and Rust standard library licenses, and the libghostty-vt license texts that `scripts/ghostty/build-xcframework.sh` collects from the pinned sources. A license that `about.toml` does not accept, or that cannot be identified, fails the build; accept a new license there and in `deny.toml`.

- **App**: `just ios-project` writes `Collie/Resources/Acknowledgements.json` (not committed), shown in Acknowledgements in Settings and on the Welcome screen. Besides the app's linked code, it covers the JetBrains Mono and Symbols Nerd Font terminal fonts (license files committed beside the fonts) and Octicons. The watch app and the extensions are pure Swift and link no third-party code.
- **collied**: the desktop release workflow writes `THIRD_PARTY_NOTICES` before any signing secret is in the environment, since the script runs downloaded tools, and `scripts/desktop/package.py --notices` copies it into both collied archives. Besides collied's dependencies for macOS and Linux, it covers LiteLLM's price table.
- **CollieBar and CollieTray** contain no third-party code and carry no notices; the CollieTray archive includes the project `LICENSE`.

## Protocol compatibility

Revision 26 is the negotiated baseline and the supported floor. Installations through revision 25 matched versions exactly and need one update on each side; pairing keys stay valid. Later releases keep the current and previous two implemented negotiated revisions; [Negotiated protocol compatibility](architecture.md#negotiated-protocol-compatibility) has the codec rules every wire change must follow. Publish every floor increase here and in the release notes, once fixtures cover both upgrade orders and the removed revision's refusal.
