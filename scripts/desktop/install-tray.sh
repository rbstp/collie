#!/usr/bin/env bash
set -euo pipefail
[ "$(uname -s)" = Linux ] || { echo 'CollieTray requires Linux with Omarchy 4' >&2; exit 1; }
for command in omarchy-plugin-validate omarchy-shell omarchy qs qrencode; do
    command -v "$command" >/dev/null || { echo "Missing dependency: $command (Omarchy 4, Quickshell and qrencode are required)" >&2; exit 1; }
done
source_dir="$(cd "$(dirname "$0")" && pwd)/CollieTray"
omarchy-plugin-validate "$source_dir"
plugins="$HOME/.config/omarchy/plugins"
dest="$plugins/rbstp.collie"
mkdir -p "$plugins"
tmp="$(mktemp -d "$plugins/.rbstp.collie.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
cp "$source_dir"/* "$tmp/"
chmod 0755 "$tmp"
updating=false
[ -d "$dest" ] && updating=true
rm -rf "$dest"
mv "$tmp" "$dest"
if $updating; then omarchy restart shell >/dev/null; else omarchy-shell shell rescanPlugins >/dev/null; fi
result=
for _ in $(seq 100); do
    result="$(omarchy-shell shell putBarWidget rbstp.collie '{"section":"right"}' 2>&1 || true)"
    [ "$result" = ok ] && break
    sleep 0.1
done
[ "$result" = ok ] || { echo "Could not enable widget: $result" >&2; exit 1; }
echo "Installed $dest"
