# Release

`.github/workflows/testflight.yml` builds, signs and uploads the iOS app to TestFlight. The Apple side is set up once, by hand.

## One-time Apple setup

Team `RM3UT3MMSR`, in the Apple Developer portal and App Store Connect:

1. **App Group**: register `group.dev.rbstp.collie`.
2. **App ID** `dev.rbstp.collie` (explicit): enable **Push Notifications** and **App Groups** (assign `group.dev.rbstp.collie`).
3. **Provisioning profile**: App Store Connect distribution, App ID `dev.rbstp.collie`, the team's Apple Distribution certificate, named exactly **`Collie App Store`** (the name `Collie/project.yml` and `Collie/ExportOptions.plist` refer to). Regenerate it, and update its secret, after any capability change on the App ID.
4. **App Store Connect app record** for `dev.rbstp.collie`, with an internal TestFlight group.
5. **APNs key**: create a dedicated APNs auth key (`.p8`) for collied. It is a runtime secret of the Mac daemon, separate from the App Store Connect API key below, and is never stored in GitHub. TestFlight builds register with the production APNs environment.

## Repository secrets

All repository-level, base64 values encoded with `base64 -i <file> | gh secret set <NAME>`:

| Secret | Content |
| --- | --- |
| `APPLE_DIST_CERT_P12` | Apple Distribution certificate and key, `.p12`, base64 |
| `APPLE_DIST_CERT_PASSWORD` | password of that `.p12` |
| `APPLE_PROVISIONING_PROFILE` | `Collie App Store` profile, `.mobileprovision`, base64 |
| `APPLE_KEY_P8` | App Store Connect API key (Developer role), `.p8`, base64 |
| `APPLE_KEY_ID` | that key's ID |
| `APPLE_ISSUER_ID` | App Store Connect issuer ID |

The certificate and the profile expire after a year; renew both and update their secrets.

## Cutting a release

- Merging a pull request into `master` ships a build, unless its title contains `[skip-release]` or it only touches `.github/`, `docs/` or `README.md`.
- Version: the latest `v*` tag bumped by minor when any commit since that tag (or the pull request title) is a `feat`, by patch otherwise. Build number: the workflow run number.
- A manual run (Actions, testflight, Run workflow, on `master` only) ships the given version, for example `1.0.0`.
- After the upload, the workflow pushes the tag `v<version>`.

## Adding a signed extension target

1. Register its App ID (App Groups enabled) and an App Store profile for it.
2. Add its secret, for example `APPLE_PROVISIONING_PROFILE_WIDGETS`, to the `env` of the "check for the signing secrets" and "import the signing certificate and profiles" steps; every `APPLE_PROVISIONING_PROFILE*` variable is checked and installed.
3. Map its bundle ID to the profile name in `Collie/ExportOptions.plist` and set `PROVISIONING_PROFILE_SPECIFIER` in its Release settings in `Collie/project.yml`.
