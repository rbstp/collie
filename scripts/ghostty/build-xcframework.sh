#!/usr/bin/env bash
# Builds libghostty-vt from the release tarballs pinned in pins.env into
# Collie/GhosttyTerminal/GhosttyVt.xcframework (ios, ios-simulator, macOS for `swift test`).
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Render state only: no Kitty graphics (file, temp-file and shared-memory image media), no input
# encoders, no snapshot/formatter/search. The terminal parser itself is always built.
VT_FEATURES=-all,+render-state

src="$work/libghostty-vt-$LIBGHOSTTY_VT_VERSION"
deps="$src-deps"
out="$root/Collie/GhosttyTerminal/GhosttyVt.xcframework"
stamp="$out/.collie-build"
want="$LIBGHOSTTY_VT_VERSION $ZIG_VERSION $VT_FEATURES $(inputs_sum "${BASH_SOURCE[0]}")"

if [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$want" ] && [ -d "$out/licenses" ]; then
    echo "GhosttyVt.xcframework is up to date (libghostty-vt $LIBGHOSTTY_VT_VERSION)"
    exit 0
fi
if restore_build "$out" "$want"; then
    echo "GhosttyVt.xcframework restored from $user_cache (libghostty-vt $LIBGHOSTTY_VT_VERSION)"
    exit 0
fi

find_zig
echo "zig: $zig ($("$zig" version))"
unpack_sources "$work"
echo "libghostty-vt: $LIBGHOSTTY_VT_VERSION"

# PACKAGING-LIBGHOSTTY.md requires an empty ZIG_GLOBAL_CACHE_DIR for every build: with Zig 0.17,
# a cache that holds a package archive can crash the build.
cache="$work/zig-global-cache"
rm -rf "$cache" "$work/out"
# The ceiling keeps Ghostty's build from taking this repository's git tags for its version.
(
    cd "$src"
    GIT_CEILING_DIRECTORIES="$work" ZIG_GLOBAL_CACHE_DIR="$cache" "$zig" build \
        --system "$deps" \
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
write_licenses "$src" "$deps" "$out/licenses"
echo "$want" >"$stamp"
save_build "$out" "$want"
echo "built $out"
