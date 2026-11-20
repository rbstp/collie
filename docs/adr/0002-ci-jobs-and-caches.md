# ADR-0002: CI runs jobs by changed paths, caches native builds and boots the simulator early

- Status: accepted
- Date: 2026-11-20 (recorded from the comments of `ci.yml`, `testflight.yml`, the justfile and the test budget script)

## Context

Every pull request waits on the macOS `rust` and `ios` jobs, and most of their time is native builds (Go, Rust, libghostty-vt, Xcode) and the simulator. Required checks must still report on every pull request, and a push to `master` must test every commit it brings.

## Decision

Job selection (`ci.yml`, `changes` job):

- A pull request's new run cancels its older run. A push to `master` never cancels one: each run covers only its own push's commits.
- `rust` and `ios` are required checks, so a pull request skips them, and `linux`, with a job-level `if` (a skipped job passes) rather than with `paths-ignore`, which would leave them pending.
- `changes` checks out the whole history's commits and trees without blobs, since a push's `before` can be any number of commits back. A push covers every commit since `before`, as when a stack merges. A first push (all zeros), a rewritten branch or a missing commit has no usable range: every job runs (fail open).
- `code` is false only when every changed path is one no test reads; tests read `docs/protocol/` and `docs/tailnet.md`. `ios` runs for what the app builds from or its tests read (PreviewTests reads collied's fixtures), `collied` (the `linux` job) for what collied builds from.
- The `linux` job builds collied for Linux on the pull request: otherwise the Linux release would be the first build linking libghostty-vt and libtailscale there.

Caches:

- The native build (the simulator xcframework and the acknowledgements) is keyed on every input, libtailscale's pin included, so a hit skips Go and Rust. It is restored before the simulator boot, which slows every restore for minutes, and on a hit CI runs only the XcodeGen half of the `ios-project` recipe.
- Xcode's compilation cache is keyed on content, unlike DerivedData's timestamps. Pull requests restore it; `master` builds without it and saves, so an entry holds one build. Only `master`'s newest entry is read, so `prune-compilation-cache` deletes the older ones, which would otherwise push rarely read entries such as `ios-sim` out, and only entries older than the run's own, since `master`'s runs end in any order.
- The Go toolchain is cached because `setup-go` otherwise unpacks Go from scratch on every macOS runner, and `target/ghostty/collied` because `crates/ghostty-vt-sys` otherwise builds libghostty-vt, Zig included (about 90 s).
- `just ios-framework` rewrites the CollieCore xcframework and bindings only when their stamp changes, because rewriting unchanged outputs makes Xcode recompile CollieCore and the app.

The simulator:

- Its first boot on a fresh runner overlaps the builds, but starts after the native build's restores: the boot saturates the runner for minutes, and twice SimLaunchHost crashed with SIGBUS while the cargo cache restored during the boot, and xcodebuild then ran the UI suite inside a second boot.
- Before the tests, a boot in progress is waited for: only runs whose xcodebuild started inside it had the UI suite's first launch hang (#753). A simulator that died is booted again before the tests rather than during them; the job goes on if that boot does not finish, since xcodebuild boots it too.
- The watchOS platform is installed in `ci.yml` and `testflight.yml`: the iOS app embeds the watch app, so every iOS build also builds for watchOS.

The test budget (`scripts/ios-test-budget.py measure`):

- It fails over a ceiling in `Collie/Tests/budget.json`, and reports after failed tests too. On `master` a passed ceiling only warns: pull requests enforce it, and a slow runner there leaves nothing to act on. The metrics are uploaded for `ios-test-budget.yml` (ADR-0003).
- The slowest UI test is reported, never enforced: one 60 s idle stall (#694) in a passing attempt would fail the job with no regression.
- App launches count a test's last run only: a retried test is counted once, as it passed. A parameterized test's own duration is the mean of its arguments, so its cost is their sum; a retried test's cost leaves out its failed attempts, which the retry absorbed, and the table names the test.
- Xcode 27.2 nests a parameterized test's retries under the argument that failed, and marks a retried argument and its test failed even when the retry passed (Arguments, then a failed and a passing Repetition), with no duration of the argument's own: what counts is each argument's last attempt. `scripts/test-ios-test-budget.py` holds that shape as a fixture.
- Test names match `NAME`, whose parts are bounded at 200 characters; the suite's longest part is 83.

`testflight.yml` tests the revision it ships, since `ci.yml` tested the pull request: unit tests only, as a UI regression shows on `master`'s `ci` run and holds back no build.

## Consequences

- A change to a path an existing filter does not list runs every job only when it is added to the filter; AGENTS.md lists both filters.
- A new input of the native build must join its cache key, or a stale xcframework is restored.

## Code

- `.github/workflows/ci.yml`, `.github/workflows/testflight.yml`
- `justfile` (`ios-framework`)
- `scripts/ios-test-budget.py`, `scripts/test-ios-test-budget.py`
