# ADR-0004: Release signing, versioning and third-party notices

- Status: accepted
- Date: 2026-11-20 (recorded from the comments of `testflight.yml`, `scripts/desktop`, `scripts/notices.py` and the justfile)

## Context

Releases sign with secrets held by the `testflight` and `desktop-release` workflows, and every release ships the notices of what it links. docs/release.md and docs/desktop-releases.md describe the procedure; this records the decisions inside the scripts.

## Decision

- The TestFlight version: without a requested version, every commit since the last tag counts toward the bump, so a cancelled pending run cannot lose a `feat` bump.
- Signing on the runner: the certificate goes into a temporary keychain with a partition list, without which codesign's first use of the key blocks on a UI prompt. Every `APPLE_PROVISIONING_PROFILE*` variable is installed, so a new signed target adds its secret there and to the check before it. No `PROVISIONING_PROFILE_SPECIFIER` is passed to `xcodebuild archive`: a command-line setting applies to every target, so each target names its profile in `Collie/project.yml`.
- Desktop publishing: on a retry, the signed assets already on the release win, and are downloaded back over the local ones, because signatures and notarization are not reproducible.
- Notices: `scripts/notices.py` runs downloaded tools (`go-licenses`, `cargo-about`), so it runs in a step without signing secrets and `scripts/desktop/package.py` takes its output as `--notices`. `go install` verifies `go-licenses` against the Go checksum database. The libghostty-vt license list comes from the build scripts' license texts of the pinned sources (ADR-0005). Go's `LICENSE` is read from GOROOT, or beside it as Homebrew keeps it. Symbols Nerd Font's Material Design Icons license only links Apache-2.0, whose section 4(a) asks for the full text, so the text is appended. collied's notices use the host's own target, since the release jobs build collied for the runner they run on.
- `just mac-notarize`: when notarization fails, `xcrun notarytool log <submission id> --keychain-profile <profile>` shows why.

## Consequences

- A new signed iOS target needs its profile secret in both places in `testflight.yml` and its profile named in `project.yml`.
- A failed release is retried with the same signed assets rather than new signatures.

## Code

- `.github/workflows/testflight.yml`
- `scripts/desktop/package.py`, `scripts/desktop/publish.py`, `scripts/notices.py`
- `justfile` (`mac-notarize`)
