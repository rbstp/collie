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

   Import writes the credential to `apns/<KEY_ID>.cred` in the data directory. systemd picks the seal, not collied: host key and TPM2 when a TPM2 is usable, host key only otherwise. Doctor's apns line says which, and warns when a TPM2 becomes usable after a host-only import. Import keeps an existing credential that decrypts to the same key, so to bind it to the TPM2, delete `apns/<KEY_ID>.cred` and import the `.p8` again. Where systemd-creds cannot encrypt (for example no `systemd-creds.socket`, systemd older than 256, a container), import falls back to a 0600 copy at `apns/AuthKey_<KEY_ID>.p8` in the data directory, sets `key_path` to it and says so. Doctor reports the fallback, and warns once systemd-creds works: import that file to encrypt it. The credential stops decrypting after an OS reinstall, a machine-id change, a uid or user name change, or a move to another machine: keep a copy of the `.p8` offline and import it again. Revoke the key in the portal if the machine is compromised.

## collied on Linux

There is no package. Install from a checkout, with the prerequisites in the README:

```sh
just setup                # submodules
just collied-install      # release build to ~/.cargo/bin/collied as a fresh inode
collied login             # advertises tag:collie-linux, refuses until the node has it
collied service install   # systemd user unit, ~/.config/systemd/user/collied.service
```

Update: `git pull`, `just setup`, `just collied-install`, which restarts the user unit if it is active.

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

- Merging a pull request into `master` ships a build, unless its title contains `[skip-release]` or it only touches `.github/`, `docs/` or `README.md`.
- Version: the latest `v*` tag bumped by minor when any commit since that tag (or the pull request title) is a `feat`, by patch otherwise. Build number: the workflow run number.
- A manual run (Actions, testflight, Run workflow, on `master` only) ships the given version, for example `1.0.0`.
- After the upload, the workflow pushes the tag `v<version>`.

## App icon and TestFlight

- The icon is a single 1024x1024 universal image, `Collie/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon.png`; Xcode derives every other size. The app's `Info.plist` sets `CFBundleIconName` to `AppIcon` and `project.yml` sets `ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon`: App Store validation rejects an upload whose icon is only in the asset catalog without `CFBundleIconName`.
- `Info.plist` does not set `ITSAppUsesNonExemptEncryption`, so App Store Connect asks the export compliance question for every uploaded build, and the build waits as "Missing Compliance" until it is answered (TestFlight, the build, Manage). The app uses encryption beyond Apple's own: WireGuard and TLS inside the embedded Tailscale node.

The iOS app embeds the watch app, so every iOS build, test and archive also builds for watchOS: both workflows install the watchOS platform on the `xcode-27` runner when it is missing. The watch app has its own copy of the icon, `Collie/CollieWatch/Assets.xcassets/AppIcon.appiconset/AppIcon.png`.

## Adding a signed extension target

CollieWidgets went in this way. `Collie/ExportOptions.plist` maps each bundle ID to its profile name under `provisioningProfiles` (`dev.rbstp.collie.widgets` to `Collie Widgets App Store`), and the workflow installs `APPLE_PROVISIONING_PROFILE_WIDGETS` with the others.

1. Register its App ID (with the capabilities its entitlements use) and an App Store profile for it.
2. Add its secret, for example `APPLE_PROVISIONING_PROFILE_<NAME>`, to the `env` of the "check for the signing secrets" and "import the signing certificate and profiles" steps; every `APPLE_PROVISIONING_PROFILE*` variable is checked and installed.
3. Map its bundle ID to the profile name in `Collie/ExportOptions.plist` and set `PROVISIONING_PROFILE_SPECIFIER` in its Release settings in `Collie/project.yml`.
