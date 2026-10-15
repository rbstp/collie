# Documentation site

The documentation site at <https://collie.rbstp.dev> is built with [mdBook](https://github.com/rust-lang/mdBook) from `docs/site/` and published to GitHub Pages by `.github/workflows/pages.yml`.

## Sources

- `docs/site/book.toml`: the mdBook configuration.
- `docs/site/src/`: the pages. `SUMMARY.md` is the table of contents.
- `docs/site/src/reference/`: each page is a single mdBook include directive for `architecture.md`, `threat-model.md`, `tailnet.md`, `notifications.md`, `release.md` and this file, so those documents have one source.
- `docs/site/theme/`: the Dark Gold theme (`collie.css`), the favicon and logo (`favicon.png`, the app icon at 128 px) and `head.hbs`, which puts the logo in the top bar.

The site lives under `docs/`, so a change to it starts no TestFlight build (`testflight.yml` ignores `docs/**`). `ci.yml` ignores `docs/site/**` and `docs/deploy-pages.md` on pushes to `master`. On a pull request, its `changes` job lists the changed files, and when nothing outside those two paths changed, the `rust` and `ios` jobs are skipped. A job skipped by its condition counts as passing, so the required checks are still met. Other docs stay out of that list because tests read `docs/protocol/` and `docs/tailnet.md`.

## The workflow

`pages` runs on pull requests and on pushes to `master` that touch `docs/**`, `README.md` or the workflow itself, and on demand (Actions, pages, Run workflow, on `master`).

1. **build**, on every run: installs mdBook 0.5.4 (checked against its SHA-256), runs `docs/site/stage-site.sh` into a temporary directory, then `docs/site/check-site.sh` on the result. A pull request stops here, so the merge gate and the deploy gate are the same checks.
2. On `master` only, **build** uploads the staged directory with `actions/upload-pages-artifact`, and **deploy** runs `actions/configure-pages` and `actions/deploy-pages` in the `github-pages` environment. Only **deploy** gets `pages: write` and `id-token: write`; it is in the `pages` concurrency group with `cancel-in-progress: false`, so a deployment is never cut off halfway.

`stage-site.sh <outdir>` builds the book and adds:

- `reference/protocol/`, a copy of `docs/protocol/`, which the included reference pages link to;
- `CNAME` with `collie.rbstp.dev` (override with `SITE_DOMAIN`), in the artifact as a record of the domain. GitHub ignores `CNAME` files for sites deployed by a custom Actions workflow: the custom domain lives only in Settings > Pages;
- `.nojekyll`.

mdBook writes `404.html` itself, from `src/404.md`. With `site-url = "/"`, it carries `<base href="/">`, so its links work at any depth. mdBook also adds a content hash to every asset name, so a deploy never mixes new pages with cached old assets.

`check-site.sh <site-dir>` fails when:

- a required page, the search index, `CNAME` or `.nojekyll` is missing;
- `CNAME` is not `collie.rbstp.dev` (or `SITE_DOMAIN`);
- a page still shows an mdBook directive, which means an include failed;
- any `href` or `src` that is not an external URL points at a missing file, or at an `id` that does not exist in its target page.

`book.toml` sets `create-missing = false`, so a `SUMMARY.md` entry without its file fails the build instead of creating an empty page.

## Build locally

Download mdBook 0.5.4 for your platform from its [releases](https://github.com/rust-lang/mdBook/releases/tag/v0.5.4), then:

```sh
MDBOOK=/path/to/mdbook docs/site/stage-site.sh /tmp/collie-site
docs/site/check-site.sh /tmp/collie-site
python3 -m http.server -d /tmp/collie-site 8080    # http://127.0.0.1:8080/
```

## Theme

The palettes are built around the app's Dark Gold accent (`AppAccent.gold` in `Collie/Sources/Collie/DevicePrefs.swift`): `#805C14` in light mode and `#C79E40` in dark mode. Gold is used for links, the active sidebar item, callout edges and titles, and focus rings. mdBook's Light and Rust themes use the light palette; Coal, Navy and Ayu use the dark one. The theme menu offers Auto, which follows the system, Light and Coal.

Every text color passes WCAG AA (4.5:1) on the backgrounds it is drawn on:

| Text | Light | Dark |
|---|---|---|
| Body text | `#24211B` on `#FBF9F4`: 15.25 | `#E8E3D8` on `#16140F`: 14.38 |
| Body text in tables (header) | `#24211B` on `#EAE3D3`: 12.56 | `#E8E3D8` on `#2E2A21`: 11.17 |
| Body text in tables (alternate row) | `#24211B` on `#F5F1E8`: 14.24 | `#E8E3D8` on `#1C1914`: 13.69 |
| Body text in callouts | `#24211B` on `#F5F0E6`: 14.13 | `#E8E3D8` on `#1F1C16`: 13.28 |
| Code text | `#24211B` on `#F2EDE2`: 13.75 | `#E8E3D8` on `#211E18`: 12.99 |
| Search highlight, links in it included | `#24211B` on `#F0DCA6`: 11.84 | `#E8E3D8` on `#5C4716`: 6.93 |
| Search field | `#24211B` on `#FFFFFF`: 16.05 | `#E8E3D8` on `#211E18`: 12.99 |
| Theme menu | `#24211B` on `#FFFDF8`: 15.79 | `#E8E3D8` on `#1D1A15`: 13.55 |
| Links | `#805C14` on `#FBF9F4`: 5.77 | `#C79E40` on `#16140F`: 7.36 |
| Links in table headers | `#805C14` on `#EAE3D3`: 4.75 | `#C79E40` on `#2E2A21`: 5.71 |
| Links in callouts, callout titles | `#805C14` on `#F5F0E6`: 5.35 | `#C79E40` on `#1F1C16`: 6.79 |
| Links in code | `#805C14` on `#F2EDE2`: 5.20 | `#C79E40` on `#211E18`: 6.64 |
| Active sidebar item | `#805C14` on `#F3EEE3`: 5.25 | `#C79E40` on `#1D1A15`: 6.93 |
| Sidebar text | `#3B362C` on `#F3EEE3`: 10.37 | `#D2CABA` on `#1D1A15`: 10.65 |
| Muted text, icons | `#5E5647` on `#FBF9F4`: 6.89 | `#B3AB9A` on `#16140F`: 8.07 |
| Caution title | `#B3261E` on `#F5F0E6`: 5.75 | `#F2867A` on `#1F1C16`: 6.85 |
| Code comments | `#6B6252` on `#F2EDE2`: 5.15 | `#A39A88` on `#211E18`: 5.96 |
| Code strings | `#3F6B21` on `#F2EDE2`: 5.39 | `#A9C77E` on `#211E18`: 8.83 |
| Code keywords | `#8A3B12` on `#F2EDE2`: 6.63 | `#E39A6B` on `#211E18`: 7.20 |
| Code numbers | `#7A4A9E` on `#F2EDE2`: 5.45 | `#C9A6E8` on `#211E18`: 8.00 |
| Code keys | `#805C14` on `#F2EDE2`: 5.20 | `#C79E40` on `#211E18`: 6.64 |
| Code titles | `#1F5C7A` on `#F2EDE2`: 6.27 | `#86C1D9` on `#211E18`: 8.42 |

Recompute these when a color in `collie.css` changes.

## 1. GitHub settings

These are repository settings, not files, so a pull request cannot set them. Do them in this order:

1. **Settings > Pages > Build and deployment > Source: GitHub Actions.** Until this is set, Pages keeps building the repository root with Jekyll from the branch, and the workflow cannot deploy.
2. Add the Cloudflare record ([below](#2-cloudflare-dns)).
3. **Settings > Pages > Custom domain: `collie.rbstp.dev`**, then Save. GitHub checks the DNS record on save, so it must exist first.
4. Wait for the certificate, then tick **Enforce HTTPS**. The checkbox stays unavailable until the certificate is issued, which usually takes minutes and can take up to an hour.

The `github-pages` environment is created on the first deployment. If it ever gets protection rules, keep `master` as an allowed deployment branch.

## 2. Cloudflare DNS

In the Cloudflare dashboard, zone `rbstp.dev`, **DNS > Records > Add record**:

| Field | Value |
|---|---|
| Type | `CNAME` |
| Name | `collie` |
| Target | `rbstp.github.io` |
| Proxy status | **DNS only** (gray cloud) |
| TTL | Auto |

Use a `CNAME`, not A records: `collie` is a subdomain. Nothing else in the zone changes.

### Pitfalls

- **Proxying before the certificate exists.** GitHub gets its Let's Encrypt certificate with an HTTP challenge on `http://collie.rbstp.dev/.well-known/acme-challenge/`. A proxied (orange) record can hide that request from GitHub, and GitHub's domain check sees Cloudflare's addresses instead of its own. Keep the record gray. If you proxy it later, after Enforce HTTPS is on, set SSL/TLS to **Full (strict)**; **Flexible** loops with GitHub's HTTPS redirect (`ERR_TOO_MANY_REDIRECTS`).
- **Always Use HTTPS.** This zone setting answers the plain HTTP challenge with a redirect, but only for proxied records. A gray record is not affected.
- **CAA records.** If the zone has CAA records, one must allow `letsencrypt.org`, or GitHub can never get a certificate.
- **HSTS with `includeSubDomains`.** Browsers then refuse plain HTTP on `collie.rbstp.dev`, so expect a TLS error rather than a page until the certificate is issued.
- **A wildcard record.** If `*.rbstp.dev` exists, `collie.rbstp.dev` resolves before you add anything, so a working `dig` proves nothing. The explicit record wins once added.

## 3. Verify

```sh
dig +short CNAME collie.rbstp.dev          # rbstp.github.io.
dig +short A collie.rbstp.dev              # 185.199.108.153 to 185.199.111.153 (gray cloud)
dig +short CAA rbstp.dev                   # empty, or must allow letsencrypt.org

curl -sSI https://collie.rbstp.dev/ | grep -i '^server:'               # server: GitHub.com
curl -sSI http://collie.rbstp.dev/ | sed -n '1p;/^[Ll]ocation:/p'      # 301 to https://collie.rbstp.dev/

echo | openssl s_client -connect collie.rbstp.dev:443 -servername collie.rbstp.dev 2>/dev/null \
  | openssl x509 -noout -issuer -dates -ext subjectAltName                # Let's Encrypt, collie.rbstp.dev

for p in / /privacy.html /support.html /troubleshooting.html /reference/threat-model.html; do
  printf '%-32s %s\n' "$p" "$(curl -sS -o /dev/null -w '%{http_code}' "https://collie.rbstp.dev$p")"
done                                                                     # all 200

curl -sS -o /dev/null -w '%{http_code}\n' https://collie.rbstp.dev/no/such/page    # 404
curl -sS https://collie.rbstp.dev/no/such/page | grep -o 'Page not found'
```

## If something is wrong

| Symptom | Cause |
|---|---|
| The site still shows the README | Source is still "Deploy from a branch". Set it to GitHub Actions and run the workflow. |
| The deploy job fails | Source is not GitHub Actions, or `github-pages` protection rules exclude `master`. |
| No deploy after a merge | The push touched nothing under `docs/`, `README.md` or the workflow. Run it by hand: Actions, pages, Run workflow. |
| The custom domain is rejected | The DNS record has not propagated: check `dig` first. |
| Enforce HTTPS stays unavailable for over an hour | No certificate. Check that the record is gray and the CAA records. |
| `ERR_TOO_MANY_REDIRECTS` | Proxied with SSL/TLS set to Flexible. Use Full (strict), or go back to gray. |
| The custom domain is blank | Set it again in Settings > Pages. With an Actions deploy, that setting is the only source; the artifact's `CNAME` does not restore it. |
