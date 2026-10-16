#!/usr/bin/env bash
set -euo pipefail

[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Tag must be vMAJOR.MINOR.PATCH' >&2; exit 1; }
[[ "$SOURCE_SHA" =~ ^[0-9a-f]{40}$ ]] || { echo 'Source must be a full commit SHA' >&2; exit 1; }
[ "$(git rev-parse HEAD)" = "$(git rev-parse refs/remotes/origin/master)" ] || { echo 'Validation must run from the trusted master checkout' >&2; exit 1; }
resolved_commit="$(git rev-parse --verify "$SOURCE_SHA^{commit}")"
git merge-base --is-ancestor "$resolved_commit" refs/remotes/origin/master
if git show-ref --verify --quiet "refs/tags/$TAG"; then
    [ "$(git rev-parse "$TAG^{commit}")" = "$resolved_commit" ] || { echo 'Existing tag points at another commit' >&2; exit 1; }
fi
{
    printf 'source_sha=%s\n' "$resolved_commit"
    printf 'tag=%s\n' "$TAG"
} >> "$GITHUB_OUTPUT"
