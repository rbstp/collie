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

rm -rf "$work/out"
(
    cd "$src"
    ZIG_GLOBAL_CACHE_DIR="$work/zig-global-cache" "$zig" build \
        -Demit-lib-vt=true \
        -Demit-xcframework=true \
        -Doptimize=ReleaseFast \
        -Dvt-features="$VT_FEATURES" \
        --prefix "$work/out"
)

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
