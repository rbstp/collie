#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"

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

cache="$work/zig-global-cache"
rm -rf "$cache" "$work/out"
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
