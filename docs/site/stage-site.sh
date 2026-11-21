#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${1:?usage: stage-site.sh <outdir>}"
mkdir -p "$TARGET"
OUT="$(cd "$TARGET" && pwd)"
SITE_DOMAIN="${SITE_DOMAIN:-collie.rbstp.dev}"
MDBOOK="${MDBOOK:-mdbook}"

log="$(mktemp)"
trap 'rm -f "$log"' EXIT
"$MDBOOK" build "$HERE" --dest-dir "$OUT" 2>&1 | tee "$log"
if grep -q 'unclosed HTML tag' "$log"; then
  echo "unclosed HTML tag in the Markdown: escape it as \\<placeholder> or put it in backticks" >&2
  exit 1
fi

cp -R "$HERE/../protocol" "$OUT/reference/protocol"

: > "$OUT/.nojekyll"
printf '%s\n' "$SITE_DOMAIN" > "$OUT/CNAME"

echo "staged $OUT ($(find "$OUT" -name '*.html' | wc -l | tr -d ' ') html files, CNAME=$SITE_DOMAIN)"
