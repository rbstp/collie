#!/usr/bin/env bash
# Builds libghostty-vt from a pinned upstream Ghostty commit into
# Collie/GhosttyTerminal/GhosttyVt.xcframework (ios, ios-simulator, macOS for `swift test`).
#
# Upstream main dropped iOS from the full GhosttyKit build ("only libghostty-vt supports iOS"),
# and no release yet ships the libghostty-vt terminal and render-state API (v1.3.1 has only the
# key/OSC/SGR parsers), so this pins a main commit.
set -euo pipefail

GHOSTTY_COMMIT=befcdfd2c3a1cb24d9ec886e93c95b2b5daa7028
ZIG_VERSION=0.16.0
ZIG_SHA256=b23d70deaa879b5c2d486ed3316f7eaa53e84acf6fc9cc747de152450d401489
# Render state only: no Kitty graphics (file, temp-file and shared-memory image media), no input
# encoders, no snapshot/formatter/search. The terminal parser itself is always built.
VT_FEATURES=-all,+render-state

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
work="$root/target/ghostty"
src="$work/src"
out="$root/Collie/GhosttyTerminal/GhosttyVt.xcframework"
stamp="$out/.collie-build"
want="$GHOSTTY_COMMIT $ZIG_VERSION $VT_FEATURES"

if [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$want" ]; then
    echo "GhosttyVt.xcframework is up to date ($GHOSTTY_COMMIT)"
    exit 0
fi

mkdir -p "$work"

if command -v zig >/dev/null && [ "$(zig version)" = "$ZIG_VERSION" ]; then
    zig="$(command -v zig)"
else
    zigdir="$work/zig-$ZIG_VERSION"
    zig="$zigdir/zig"
    if [ ! -x "$zig" ]; then
        [ "$(uname -m)" = arm64 ] || { echo "only arm64 macOS is supported" >&2; exit 1; }
        tarball="$work/zig-aarch64-macos-$ZIG_VERSION.tar.xz"
        curl -fsSL -o "$tarball" "https://ziglang.org/download/$ZIG_VERSION/zig-aarch64-macos-$ZIG_VERSION.tar.xz"
        echo "$ZIG_SHA256  $tarball" | shasum -a 256 -c -
        rm -rf "$zigdir"
        mkdir -p "$zigdir"
        tar -xJf "$tarball" --strip-components=1 -C "$zigdir"
        rm -f "$tarball"
    fi
fi
echo "zig: $zig ($("$zig" version))"

if [ ! -d "$src/.git" ]; then
    git init -q "$src"
    git -C "$src" remote add origin https://github.com/ghostty-org/ghostty.git
fi
if [ "$(git -C "$src" rev-parse -q --verify HEAD || true)" != "$GHOSTTY_COMMIT" ]; then
    git -C "$src" fetch -q --depth 1 origin "$GHOSTTY_COMMIT"
    git -C "$src" checkout -q --force --detach FETCH_HEAD
fi
git -C "$src" clean -qfdx -e .zig-cache
[ "$(git -C "$src" rev-parse HEAD)" = "$GHOSTTY_COMMIT" ]
echo "ghostty: $GHOSTTY_COMMIT"

cache="$work/zig-global-cache"
build() {
    rm -rf "$work/out"
    (
        cd "$src"
        ZIG_GLOBAL_CACHE_DIR="$cache" "$zig" build \
            -Demit-lib-vt=true \
            -Demit-xcframework=true \
            -Doptimize=ReleaseFast \
            -Dvt-features="$VT_FEATURES" \
            --prefix "$work/out"
    )
}

# Fallback for when an upstream package host (codeberg.org, GitHub) is down: the same packages,
# published on this repo's ghostty-deps-<commit> release. `zig fetch` recomputes each package hash,
# which ghostty's build.zig.zon files pin, so the mirror cannot substitute content.
# Regenerate the release from target/ghostty/zig-global-cache/p when GHOSTTY_COMMIT changes.
MIRROR="https://github.com/rbstp/collie/releases/download/ghostty-deps-${GHOSTTY_COMMIT:0:8}"
MIRRORED_PACKAGES=(
    aro-0.0.0-JSD1Qk6lNgDdcDV4Vh7Sfy-34m2TluIVOdPzMmj_0BjX
    N-V-__8AAB0eQwD-0MdOEBmz7intriBReIsIDNlukNVoNu6o
    N-V-__8AAGmZhABbsPJLfbqrh6JTHsXhY6qCaLAQyx25e0XE
    N-V-__8AAM94BAAFk_hn4UW0x_OBD2g0vOwexeAAyWNNo4eB
    translate_c-0.0.0-Q_BUWhVNBwDOEcIqub4VFPJPB6D9dgwzUMHTX5KWr8Xr
    uucode-0.2.0-ZZjBPuuFVgC8YZ8eld4fOKsZANLIhTFMzULQxhkLi1C7
)
seed_from_mirror() {
    mkdir -p "$work/mirror"
    for pkg in "${MIRRORED_PACKAGES[@]}"; do
        [ -f "$cache/p/$pkg.tar.gz" ] && continue
        file="$work/mirror/$pkg.tar.gz"
        curl -fsSL --retry 3 -o "$file" "$MIRROR/$pkg.tar.gz"
        got="$(cd "$src" && "$zig" fetch --global-cache-dir "$cache" "$file" | tail -n 1)"
        rm -f "$file"
        if [ "$got" != "$pkg" ]; then
            echo "mirrored package $pkg hashes to $got" >&2
            exit 1
        fi
        echo "seeded $pkg from the mirror"
    done
}

if ! build; then
    echo "zig build failed; seeding missing packages from $MIRROR and retrying" >&2
    seed_from_mirror
    build
fi

built="$work/out/lib/ghostty-vt.xcframework"
headers="$work/headers"
rm -rf "$headers" "$out"
mkdir -p "$headers/GhosttyVt"
(cd "$src/include" && find ghostty -name '*.h' -print0 | xargs -0 tar -cf -) | tar -xf - -C "$headers"
# A module map at the Headers root would collide with any other static xcframework doing the same
# once Xcode merges them into one include directory.
cat >"$headers/GhosttyVt/module.modulemap" <<'EOF'
module GhosttyVt {
    umbrella header "../ghostty/vt.h"
    export *
}
EOF

xcodebuild -create-xcframework \
    -library "$built/ios-arm64/libghostty-vt-fat.a" -headers "$headers" \
    -library "$built/ios-arm64-simulator/libghostty-vt-fat.a" -headers "$headers" \
    -library "$built/macos-arm64_x86_64/libghostty-vt.a" -headers "$headers" \
    -output "$out"
echo "$want" >"$stamp"
echo "built $out"
