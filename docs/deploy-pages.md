# Documentation site

The site at <https://collie.rbstp.dev> is built with [mdBook](https://github.com/rust-lang/mdBook) from `docs/site/` and published to GitHub Pages by `.github/workflows/pages.yml`.

## Sources

- `docs/site/book.toml`: the configuration. Its `create-missing = false` fails the build on a `SUMMARY.md` entry without a file.
- `docs/site/src/`: the pages, listed in `SUMMARY.md`.
- `docs/site/src/reference/`: one mdBook include directive per page, so each included document under `docs/`, such as this one, has one source.
- `docs/site/src/images/`: light and dark iPhone screenshots and the watch screenshots, made by `just ios-screenshots` from the app's demo data. Install [pngquant](https://pngquant.org) (`brew install pngquant`) first: uncompressed, the images are several times larger. A page shows a pair inside `<div class="screenshots">` with the classes `screenshot-light` and `screenshot-dark`, and `collie.css` hides the copy that does not match the theme. A workflow refreshes the iPhone screenshots after every TestFlight release ([release.md](release.md#documentation-screenshots)).
- `docs/site/theme/`: `collie.css`, the Dark Gold theme; `favicon.png`, the app icon at 128 px, also the logo; and `head.hbs`, which puts the logo in the top bar.

Site changes start no TestFlight build (`testflight.yml` ignores `docs/**`). When only `docs/site/`, `docs/deploy-pages.md`, `README.md`, `AGENTS.md` or `CLAUDE.md` change, `ci.yml` does not run on a push to `master`, and on a pull request it skips the required `rust` and `ios` jobs, which then count as passing. Tests read `docs/protocol/` and `docs/tailnet.md`, so other docs stay out of that list.

## The workflow

`pages` runs on pull requests and pushes to `master` that touch `docs/**`, `README.md` or the workflow, and by hand (Actions, pages, Run workflow, on `master`). Its **build** job installs mdBook 0.5.4 (checked against its SHA-256), runs `docs/site/stage-site.sh`, then `docs/site/check-site.sh`; a pull request stops there, so merging and deploying pass the same checks. On `master`, **deploy** publishes the staged site in the `github-pages` environment. Only **deploy** gets `pages: write` and `id-token: write`, and a new deploy never cancels one in progress.

`stage-site.sh <outdir>` builds the book and fails on an unclosed HTML tag (a bare `<placeholder>` in prose: write `\<placeholder>` or use backticks). It adds `.nojekyll`, `reference/protocol/` (a copy of `docs/protocol/`, which the reference pages link to) and `CNAME`. GitHub ignores `CNAME` for a site deployed by a custom Actions workflow, so the file only records the domain, `collie.rbstp.dev` or `SITE_DOMAIN`. The custom domain lives only in Settings > Pages: if it goes blank, set it again there.

`check-site.sh <site-dir>` fails on a missing required page, search index, `CNAME` or `.nojekyll`, a `CNAME` other than that domain, an mdBook directive left by a failed include, or an internal `href` or `src` to a missing file or to an `id` missing from its target page.

## Build locally

Download mdBook 0.5.4 for your platform from its [releases](https://github.com/rust-lang/mdBook/releases/tag/v0.5.4), then:

```sh
MDBOOK=/path/to/mdbook docs/site/stage-site.sh /tmp/collie-site
docs/site/check-site.sh /tmp/collie-site
python3 -m http.server -d /tmp/collie-site 8080    # http://127.0.0.1:8080/
```

## Theme

The palettes follow the app's Dark Gold accent (`AppAccent.gold` in `Collie/Sources/Collie/DevicePrefs.swift`), `#805C14` in light mode and `#C79E40` in dark mode, for links, the active sidebar item, callout edges and titles, and focus rings. mdBook's Light and Rust themes use the light palette, and Coal, Navy and Ayu the dark one; the theme menu offers only Auto (follows the system), Light and Coal.

Every text color passes WCAG AA (4.5:1) on its backgrounds. Recompute these ratios when a color in `collie.css` changes.

Body text, `#24211B` (light) and `#E8E3D8` (dark), on:

| Background | Light | Dark |
|---|---|---|
| Page | `#FBF9F4`: 15.25 | `#16140F`: 14.38 |
| Table header | `#EAE3D3`: 12.56 | `#2E2A21`: 11.17 |
| Table alternate row | `#F5F1E8`: 14.24 | `#1C1914`: 13.69 |
| Callout | `#F5F0E6`: 14.13 | `#1F1C16`: 13.28 |
| Code | `#F2EDE2`: 13.75 | `#211E18`: 12.99 |
| Search highlight, links in it included | `#F0DCA6`: 11.84 | `#5C4716`: 6.93 |
| Search field | `#FFFFFF`: 16.05 | `#211E18`: 12.99 |
| Theme menu | `#FFFDF8`: 15.79 | `#1D1A15`: 13.55 |

Gold, `#805C14` (light) and `#C79E40` (dark), on:

| Text and background | Light | Dark |
|---|---|---|
| Links | `#FBF9F4`: 5.77 | `#16140F`: 7.36 |
| Links in table headers | `#EAE3D3`: 4.75 | `#2E2A21`: 5.71 |
| Links in callouts, callout titles | `#F5F0E6`: 5.35 | `#1F1C16`: 6.79 |
| Links in code, code keys | `#F2EDE2`: 5.20 | `#211E18`: 6.64 |
| Active sidebar item | `#F3EEE3`: 5.25 | `#1D1A15`: 6.93 |

Other text:

| Text | Light | Dark |
|---|---|---|
| Sidebar text | `#3B362C` on `#F3EEE3`: 10.37 | `#D2CABA` on `#1D1A15`: 10.65 |
| Muted text, icons | `#5E5647` on `#FBF9F4`: 6.89 | `#B3AB9A` on `#16140F`: 8.07 |
| Caution title | `#B3261E` on `#F5F0E6`: 5.75 | `#F2867A` on `#1F1C16`: 6.85 |
| Code comments | `#6B6252` on `#F2EDE2`: 5.15 | `#A39A88` on `#211E18`: 5.96 |
| Code strings | `#3F6B21` on `#F2EDE2`: 5.39 | `#A9C77E` on `#211E18`: 8.83 |
| Code keywords | `#8A3B12` on `#F2EDE2`: 6.63 | `#E39A6B` on `#211E18`: 7.20 |
| Code numbers | `#7A4A9E` on `#F2EDE2`: 5.45 | `#C9A6E8` on `#211E18`: 8.00 |
| Code titles | `#1F5C7A` on `#F2EDE2`: 6.27 | `#86C1D9` on `#211E18`: 8.42 |

## 1. GitHub settings

No pull request can change these repository settings. In this order:

1. **Settings > Pages > Build and deployment > Source: GitHub Actions.** Until then, Pages builds the repository root with Jekyll, so the site shows the README, and the workflow cannot deploy: run it by hand after the change.
2. Add the [Cloudflare record](#2-cloudflare-dns).
3. **Settings > Pages > Custom domain: `collie.rbstp.dev`**, then Save. GitHub checks the DNS record on save, so wait until `dig` shows it.
4. Tick **Enforce HTTPS** once GitHub has the certificate, usually within minutes and at most about an hour; if it takes longer, see the pitfalls below.

If the `github-pages` environment, which the first deployment creates, ever gets protection rules, keep `master` allowed.

## 2. Cloudflare DNS

In the Cloudflare dashboard, zone `rbstp.dev`, **DNS > Records > Add record**:

| Field | Value |
|---|---|
| Type | `CNAME` |
| Name | `collie` |
| Target | `rbstp.github.io` |
| Proxy status | **DNS only** (gray cloud) |
| TTL | Auto |

`collie` is a subdomain, so use a `CNAME`, not A records. Nothing else in the zone changes.

### Pitfalls

- **Proxying before the certificate exists.** A proxied (orange) record can hide GitHub's Let's Encrypt HTTP challenge on `http://collie.rbstp.dev/.well-known/acme-challenge/`, and makes GitHub's domain check see Cloudflare's addresses. Keep the record gray. To proxy it after Enforce HTTPS is on, set SSL/TLS to **Full (strict)**: **Flexible** loops with GitHub's HTTPS redirect (`ERR_TOO_MANY_REDIRECTS`).
- **Always Use HTTPS** redirects the HTTP challenge, but only on proxied records.
- **CAA records**, if the zone has any, must allow `letsencrypt.org`, or GitHub never gets a certificate.
- **HSTS with `includeSubDomains`** makes browsers show a TLS error instead of the site until the certificate is issued.
- **A wildcard `*.rbstp.dev` record** makes `collie.rbstp.dev` resolve before you add anything, so a working `dig` proves nothing; the explicit record wins once added.

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
