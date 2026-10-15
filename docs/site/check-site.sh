#!/usr/bin/env bash
# Asserts a staged copy of the documentation site is complete, with no broken internal link.
#
#     docs/site/check-site.sh <site-dir>
#
# Env:
#   SITE_DOMAIN   hostname expected in CNAME (default collie.rbstp.dev)
set -euo pipefail

SITE="$(cd "${1:?usage: check-site.sh <site-dir>}" && pwd)"
SITE_DOMAIN="${SITE_DOMAIN:-collie.rbstp.dev}"
fail=0

for f in index.html 404.html privacy.html support.html troubleshooting.html security/overview.html \
         reference/architecture.html reference/threat-model.html reference/tailnet.html \
         reference/notifications.html reference/release.html reference/protocol/notification-vector.json \
         toc.html CNAME; do
  test -s "$SITE/$f" || { echo "missing or empty: $f"; fail=1; }
done
test -f "$SITE/.nojekyll" || { echo "missing: .nojekyll"; fail=1; }
ls "$SITE"/searchindex-*.js >/dev/null 2>&1 || { echo "missing: search index"; fail=1; }
test "$(cat "$SITE/CNAME" 2>/dev/null)" = "$SITE_DOMAIN" || { echo "CNAME is not $SITE_DOMAIN"; fail=1; }

if grep -rl --include='*.html' '{{#' "$SITE" >/dev/null; then
  echo "unexpanded mdBook directive in: $(grep -rl --include='*.html' '{{#' "$SITE" | tr '\n' ' ')"; fail=1
fi

links=0
while IFS= read -r page; do
  dir="$(dirname "$page")"
  # 404.html is served at any depth, so mdBook roots it with <base href="/">.
  if grep -q '<base href="/">' "$page"; then dir="$SITE"; fi
  while IFS= read -r ref; do
    case "$ref" in
      http:*|https:*|mailto:*|data:*|javascript:*|//*|'') continue ;;
    esac
    path="${ref%%#*}"
    frag=""
    case "$ref" in *'#'*) frag="${ref#*#}" ;; esac
    path="${path%%\?*}"
    if [ -z "$path" ]; then
      target="$page"
    elif [ "${path#/}" != "$path" ]; then
      target="$SITE$path"
    else
      target="$dir/$path"
    fi
    if [ -d "$target" ]; then target="$target/index.html"; fi
    links=$((links + 1))
    if [ ! -e "$target" ]; then
      echo "broken link in ${page#"$SITE"/}: $ref"; fail=1
    elif [ -n "$frag" ] && [ "${target%.html}" != "$target" ] && ! grep -qF "id=\"$frag\"" "$target"; then
      echo "broken anchor in ${page#"$SITE"/}: $ref"; fail=1
    fi
  done < <(grep -oE '(href|src)="[^"]*"' "$page" | sed -E 's/^(href|src)="//; s/"$//')
done < <(find "$SITE" -name '*.html')

test "$fail" -eq 0 || exit 1
echo "staged site looks right ($(find "$SITE" -name '*.html' | wc -l | tr -d ' ') pages, $links internal links)"
