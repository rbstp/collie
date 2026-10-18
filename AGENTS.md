# AGENTS.md

Guidance for coding agents working in this repository. User-facing documentation is the site at <https://collie.rbstp.dev>, built from `docs/site`; design and protocol behavior are in `docs/architecture.md`.

## What lives where

- `crates/protocol`: wire types and `PROTOCOL_VERSION`; `just schema` writes the JSON Schemas to `docs/protocol/`.
- `crates/collied`: the daemon (macOS and Linux). `crates/collie-core`: the phone's Rust core, exposed to Swift with UniFFI.
- `crates/tailscale-sys`, `crates/tailnet`, `crates/tls`: libtailscale (pinned submodule plus a patch series), its safe wrapper, mutual TLS.
- `crates/e2e`: phone core against collied over a local test tailnet.
- `Collie/`: iOS app, extensions and Apple Watch app (XcodeGen `Collie/project.yml`). `Mac/`: the CollieBar menu bar app. `Linux/`: the CollieTray Omarchy bar widget.
- Generated, never committed: `*.xcodeproj`, the CollieCore xcframework and bindings, the Ghostty xcframework.

## Working style

- Keep changes barebone: the smallest change that solves the problem. No speculative options, helpers, abstractions or config knobs, and no drive-by refactors. If something extra seems worth doing, say so in the pull request instead.
- Comments are critical-only: invariants, security rationale, provenance, constraints the code cannot show. No narration of what the code does and no doc comments that restate a signature. Do not add comments the surrounding code would not have.
- Never use em dashes anywhere: code, comments, docs, commit messages, pull requests and issues.
- No migration code. There is a single user; a format change means re-pairing, not migrating.
- UI: no sheep or flock branding in UI text. Follow the existing SwiftUI and Liquid Glass patterns (`docs/reviews/liquid-glass.md`).
- Keep CollieBar (`Mac/`) and CollieTray (`Linux/`) behavior aligned: when changing either desktop bar, apply the equivalent change to both where applicable, including their documentation and tests.

## Build and test

| Command | When |
|---|---|
| `just lint` | Always: `cargo fmt --check`, clippy with `-D warnings`, `cargo deny` |
| `just test` | Always: all Rust tests with cargo-nextest, end-to-end tests included |
| `just ios-test` | iOS, watch or CollieCore changes |
| `just mac-test` | `Mac/` changes |
| `just tray-test` | `Linux/` changes (Linux only) |
| `just schema` | Any change to `crates/protocol` types |

When the CollieCore framework and the Xcode project are already current, `just --no-deps sim="<simulator>" ios-test` skips rebuilding them. Flags go before variable overrides.

- Before asking the user to start a release acceptance test, verify the exact TestFlight build is processed and installable, matching desktop artifacts are published and verified, and any required public endpoint passes TLS and protocol checks.
- UI tests must not format fixture text into assertion messages (CodeQL flags it).
- Captured fixtures (for example `crates/collied/tests/fixtures`) must be scrubbed of personal paths, names and secrets before they are committed.
- When a test needs a live herdr, create a scratch workspace (`herdr workspace create ... --label <name>`) and close it afterwards. Never send input to panes you did not create, and do not modify the user's agent CLI configuration.

## Protocol

- Bump `PROTOCOL_VERSION` (`crates/protocol/src/lib.rs`) only for wire changes an older peer would refuse or misread, and add a line to the version history in `docs/architecture.md`.
- Optional, additive fields on server-to-phone types need no bump; say "No version bump" and why in `docs/architecture.md`.
- Run `just schema` and commit the regenerated `docs/protocol/` files.

## Dependencies

- Always pin the latest stable release of every dependency: crates, Swift packages, toolchains and tools downloaded in CI (checked against their SHA-256).
- GitHub Actions are pinned by commit SHA with a version comment: `uses: actions/checkout@<sha> # v7.0.1`.
- libtailscale is a submodule pinned by commit with patches in `crates/tailscale-sys/patches`; bumping it follows `docs/architecture.md` (Tailscale section). libghostty-vt is built from a pinned commit by `scripts/ghostty/build-xcframework.sh`.

## Documentation follows features

When a feature or behavior changes, update its documentation in the same pull request:

- The site pages under `docs/site/src`, and `docs/site/src/SUMMARY.md` for a new page.
- `docs/architecture.md` for design, protocol and behavior.
- `docs/threat-model.md` when the change is security-relevant.
- `docs/notifications.md` and the privacy policy (`docs/site/src/privacy.md`) when data handling changes or what Apple or other third parties can see changes.
- `README.md` only for entry-point facts (what collie is, requirements, quick start, recipes, layout).

Pages under `docs/site/src/reference/` are mdBook includes of `docs/*.md`, never copies. Check the site before pushing (mdBook 0.5.4, see `docs/deploy-pages.md`):

```sh
MDBOOK=/path/to/mdbook docs/site/stage-site.sh /tmp/collie-site
docs/site/check-site.sh /tmp/collie-site
```

## Issues, commits and pull requests

- After a pull request is merged or closed, remove its clean local worktree and local branch once no active task depends on them. Preserve unique commits and required artifacts first. Keep the default branch and branches used by open pull requests.
- Planned work is a GitHub issue in the current phase milestone (for example "Phase 7: Improvements"). The pull request body says `Closes #N`.
- Commit messages and pull request titles are plain sentences: imperative, sentence case, no type prefix, no trailing period, like the existing history ("Keep a merged pull request until the branch moves").
- Merging a pull request into `master` uploads a TestFlight build. Put `[skip-release]` in the title of a pull request that should not ship one (docs, CI, tests only).
- Pull requests that change only `docs/site/`, `docs/deploy-pages.md`, `README.md`, `AGENTS.md` or `CLAUDE.md` skip the macOS `rust` and `ios` CI jobs (the `changes` job in `.github/workflows/ci.yml`). Any other path runs them; tests read `docs/protocol/` and `docs/tailnet.md`.
- Keep published iOS, collied, CollieBar and CollieTray releases on the same version, tag and source commit, including UI-only changes.
- Release details, signing and secrets: `docs/release.md`.

## Cloudflare deployments

Use the `cf` CLI with its active OAuth login for Cloudflare deployments.
Before deployment, unset `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_API_KEY` and `CLOUDFLARE_EMAIL`, then verify the intended account with `cf auth whoami`.
Set the verified `CLOUDFLARE_ACCOUNT_ID` explicitly. Wrangler is limited to local builds and dry runs.
