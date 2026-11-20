# ADR-0003: Workflows that hold write tokens never run pull request code

- Status: accepted
- Date: 2026-11-20 (recorded from the comments of `ios-test-budget.yml`, `docs-screenshots.yml` and `scripts/ios-test-budget.py`)

## Context

Two workflows hold a token that can write to the repository or its pull requests: `ios-test-budget.yml` comments the test budget on pull requests, and `docs-screenshots.yml` opens a pull request with new documentation screenshots. Code from a pull request, or a file it produced, must never run with those tokens.

## Decision

- `ios-test-budget.yml` runs on `workflow_run` from `master`, with `pull-requests: write`. It checks out `master`'s `scripts/ios-test-budget.py` alone and never runs a pull request's code. A pull request's metrics artifact is whatever that pull request made it, so it is read only through `load()`: the zip is read in memory and bounded, nothing is extracted to disk, and `validate()` accepts exactly what `measure` writes and raises `ValueError` otherwise. Every cell of the comment comes from a validated number or a name matching `NAME`.
- The comparison baseline is the latest successful `ci` run on `master` that has metrics, since some pushes skip the `ios` job. The workflow keeps one comment per pull request and updates it in place.
- `docs-screenshots.yml`'s capture job runs only a commit on `master`, since it reads the caches that `testflight` and `ci` restore. The token stays out of that job: the `propose` job, in the `docs-screenshots` environment, never builds or runs the captured revision.
- `propose` copies only regular PNG files (name pattern, PNG signature, no symlinks) from the artifact into `docs/site/src/images`, because Pages serves whatever lands there.
- `propose` opens its pull request with `DOCS_SCREENSHOTS_TOKEN`: a pull request opened with `GITHUB_TOKEN` starts no workflow, so the required checks and the pages deploy would never run.

## Consequences

- A change to how metrics are read changes `load()` and `validate()` on `master`; a pull request cannot change what its own comment run executes.
- `scripts/ios-test-budget.py` usage: `measure [--warn-only] <budget.json> <xcresult or folder> <metrics.json>` (the `ios` job), and `comment --pr <artifact.zip> [--baseline <artifact.zip>] --pr-sha <sha> [--baseline-sha <sha>] --conclusion <text>` (`ios-test-budget.yml`), which prints the comment's markdown.

## Code

- `.github/workflows/ios-test-budget.yml`, `.github/workflows/docs-screenshots.yml`
- `scripts/ios-test-budget.py` (`load`, `validate`, `render`)
