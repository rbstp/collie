#!/usr/bin/env bash
# Builds the documentation site into <outdir> for GitHub Pages.
#
#     docs/site/stage-site.sh <outdir>
#
# Env:
#   SITE_DOMAIN   hostname written to CNAME (default collie.rbstp.dev)
#   MDBOOK        mdBook binary (default mdbook)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${1:?usage: stage-site.sh <outdir>}"
mkdir -p "$TARGET"
OUT="$(cd "$TARGET" && pwd)"
SITE_DOMAIN="${SITE_DOMAIN:-collie.rbstp.dev}"
MDBOOK="${MDBOOK:-mdbook}"

"$MDBOOK" build "$HERE" --dest-dir "$OUT"

# The included reference pages link to these files relative to themselves.
cp -R "$HERE/../protocol" "$OUT/reference/protocol"

: > "$OUT/.nojekyll"
# In the artifact, so a deploy never drops the custom domain.
printf '%s\n' "$SITE_DOMAIN" > "$OUT/CNAME"

echo "staged $OUT ($(find "$OUT" -name '*.html' | wc -l | tr -d ' ') html files, CNAME=$SITE_DOMAIN)"
