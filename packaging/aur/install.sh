#!/usr/bin/bash
set -euo pipefail
[ "$(id -u)" != 0 ] || { echo 'Run as your login user, without sudo' >&2; exit 1; }
here="$(cd "$(dirname "$0")" && pwd)"
packages=()
for name in collied-bin collie-tray; do
    (
        cd "$here/$name"
        makepkg --force --cleanbuild --nodeps
    )
    while IFS= read -r package; do packages+=("$package"); done < <(
        cd "$here/$name"
        makepkg --packagelist
    )
done
sudo pacman -U "${packages[@]}"
