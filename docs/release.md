# Release

`.github/workflows/testflight.yml` builds, signs and uploads the iOS app to TestFlight. The Apple side is set up once, by hand.

## One-time Apple setup

Team `RM3UT3MMSR`, in the Apple Developer portal and App Store Connect:

1. **App Group**: register `group.dev.rbstp.collie`.
2. **App IDs** (explicit), each with **Push Notifications** and **App Groups** (assign `group.dev.rbstp.collie`) enabled:
   - `dev.rbstp.collie`, the app;
   - `dev.rbstp.collie.push`, the ColliePush notification service extension.
3. **Provisioning profiles**: App Store Connect distribution, the team's Apple Distribution certificate, named exactly as `Collie/project.yml` and `Collie/ExportOptions.plist` refer to them:
   - **`Collie App Store`** for `dev.rbstp.collie`;
   - **`Collie Push App Store`** for `dev.rbstp.collie.push`.

   A profile is a snapshot of its App ID's capabilities: after any capability change, regenerate the profile (Edit, Save) and update its secret.
4. **App Store Connect app record** for `dev.rbstp.collie`, with an internal TestFlight group.
5. **APNs key for collied**: Certificates, Identifiers & Profiles, Keys, add a key with **Apple Push Notifications service (APNs)**, environment **Sandbox & Production**. Debug builds installed with `just ios-run-device` register sandbox tokens, TestFlight builds production tokens, and one collied serves both. Download the `.p8` (offered once) and note the Key ID. It is a runtime secret of the Mac daemon, separate from the App Store Connect API key below, and is never stored in GitHub. On the Mac:

   ```sh
   install -m 0600 AuthKey_<KEY_ID>.p8 "$HOME/Library/Application Support/collie/apns.p8"
   rm AuthKey_<KEY_ID>.p8
   ```

   ```toml
   # ~/Library/Application Support/collie/collied.toml
   [apns]
   key_path = "/Users/<you>/Library/Application Support/collie/apns.p8"
   key_id = "<KEY_ID>"
   team_id = "RM3UT3MMSR"
   bundle_id = "dev.rbstp.collie"
   ```

   `collied doctor` checks that the key file is 0600 and owned by you. Revoke the key in the portal if the Mac is compromised.

## Repository secrets

All repository-level, base64 values encoded with `base64 -i <file> | gh secret set <NAME>`:

| Secret | Content |
| --- | --- |
| `APPLE_DIST_CERT_P12` | Apple Distribution certificate and key, `.p12`, base64 |
| `APPLE_DIST_CERT_PASSWORD` | password of that `.p12` |
| `APPLE_PROVISIONING_PROFILE` | `Collie App Store` profile, `.mobileprovision`, base64 |
| `APPLE_PROVISIONING_PROFILE_PUSH` | `Collie Push App Store` profile, `.mobileprovision`, base64 |
| `APPLE_KEY_P8` | App Store Connect API key (Developer role), `.p8`, base64 |
| `APPLE_KEY_ID` | that key's ID |
| `APPLE_ISSUER_ID` | App Store Connect issuer ID |

The certificate and the profiles expire after a year; renew them and update their secrets.

## Cutting a release

- Merging a pull request into `master` ships a build, unless its title contains `[skip-release]` or it only touches `.github/`, `docs/` or `README.md`.
- Version: the latest `v*` tag bumped by minor when any commit since that tag (or the pull request title) is a `feat`, by patch otherwise. Build number: the workflow run number.
- A manual run (Actions, testflight, Run workflow, on `master` only) ships the given version, for example `1.0.0`.
- After the upload, the workflow pushes the tag `v<version>`.

## Adding a signed extension target

1. Register its App ID (App Groups enabled) and an App Store profile for it.
2. Add its secret, for example `APPLE_PROVISIONING_PROFILE_WIDGETS`, to the `env` of the "check for the signing secrets" and "import the signing certificate and profiles" steps; every `APPLE_PROVISIONING_PROFILE*` variable is checked and installed.
3. Map its bundle ID to the profile name in `Collie/ExportOptions.plist` and set `PROVISIONING_PROFILE_SPECIFIER` in its Release settings in `Collie/project.yml`.
