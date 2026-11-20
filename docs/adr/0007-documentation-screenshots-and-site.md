# ADR-0007: Documentation screenshots come from the demo on a fixed simulator; the site is staged and checked

- Status: accepted
- Date: 2026-11-20 (recorded from the comments of the justfile, `docs-screenshots.yml` and `docs/site`)

## Context

`just ios-screenshots` makes the site's light and dark screenshots and the App Store set from the app's demo data, locally and in `docs-screenshots.yml`. docs/deploy-pages.md and docs/release.md describe the site and the screenshot workflow; this records the decisions in the scripts.

## Decision

- The app is installed fresh before the captures: that drops drafts left by other UI tests, and the demo takes its accent from the prefs file.
- The iOS 27.2 simulator applies an appearance change at its next boot only, so the simulator is shut down and booted again for each appearance.
- `simctl bootstatus` exits 0 even when a new watch simulator shuts down during its first-boot data migration; booting it again completes, so the recipe boots up to three times and fails if it does not stay booted. watchOS has no light mode and no status bar override, so the watch gets one copy of each shot, with the current time.
- App Store Connect refuses alpha; a PPM round trip drops it without loss.
- `pngquant` compresses only this run's captures, since compressing a committed image again can still change it, and keeps a file as is on exit codes 98 and 99 (the result would be larger, or below the quality floor).
- In CI, pngquant's macOS build (3.0.3) is published only at an unversioned URL, so a new upload fails the checksum. The runner's watchOS runtime loses its files minutes after it is mounted (#539), so CI keeps the watch shots as committed.
- The site: `stage-site.sh` and `check-site.sh` are documented in docs/deploy-pages.md. mdBook roots `404.html` with `<base href="/">`, since it is served at any depth, so `check-site.sh` resolves that page's links from the site root.

## Consequences

- The watch screenshots change only when someone runs `just ios-screenshots` locally with the watch on.

## Code

- `justfile` (`ios-screenshots`), `.github/workflows/docs-screenshots.yml`
- `docs/site/stage-site.sh`, `docs/site/check-site.sh`
