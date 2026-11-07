#!/usr/bin/env bash
# Builds libghostty-vt from its pinned release tarballs into
# Collie/GhosttyTerminal/GhosttyVt.xcframework (ios, ios-simulator, macOS for `swift test`).
#
# The tarball SHA-256s are the ones in the release's SHA256SUMS, whose minisign signature was checked
# against the Ghostty release key in PACKAGING-LIBGHOSTTY.md when pinning.
set -euo pipefail

# Moving this pin means rerunning scripts/ghostty/nerd-font-constraints.py.
LIBGHOSTTY_VT_VERSION=1.0.0
# The vt-1.0.0 tag, which nerd-font-constraints.py reads Ghostty's font tables from.
GHOSTTY_COMMIT=0fa3d577a5614150ff8e2c152de5fb9d5c6d2dec
SOURCE_SHA256=4a6ef758be721923e1969e7b81b443dcc92d8dd3fb5a39600c03a8817a753676
DEPS_SHA256=4d5ea5e448e4c7ea8fb55ccbaa8668f2a97d44f438cee831e2b7ecd162d85c29
ZIG_VERSION=0.17.1
ZIG_SHA256=75f1bc7a1e9852f17ec9a0c1895f767c4ec56d46bc8a834920eece1d0359dd8d
# Render state only: no Kitty graphics (file, temp-file and shared-memory image media), no input
# encoders, no snapshot/formatter/search. The terminal parser itself is always built.
VT_FEATURES=-all,+render-state
# License texts uucode's Zig package leaves out, at the commit the dependency tarball holds.
UUCODE_LICENSES=https://raw.githubusercontent.com/jacobsandlund/uucode/1fb73433bba5d93366c57f23ff2e9d7939746500/licenses
UUCODE_UNICODE_SHA256=1eda5a3b026870c737b22e8bcd4954338612c790db688242e003f41a4fa95175
UUCODE_HOEHRMANN_SHA256=de219cece932aad5a817bf763393d8d149d378a15d2ad5320e3331eac07626dd

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
work="$root/target/ghostty"
src="$work/libghostty-vt-$LIBGHOSTTY_VT_VERSION"
deps="$src-deps"
out="$root/Collie/GhosttyTerminal/GhosttyVt.xcframework"
stamp="$out/.collie-build"
want="$LIBGHOSTTY_VT_VERSION $ZIG_VERSION $VT_FEATURES"

if [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$want" ] && [ -d "$out/licenses" ]; then
    echo "GhosttyVt.xcframework is up to date (libghostty-vt $LIBGHOSTTY_VT_VERSION)"
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

unpack() {
    tarball="$work/$1.tar.gz"
    # Fallback for when release.files.ghostty.org is down: the same tarballs on this repo's release.
    curl -fsSL --retry 3 -o "$tarball" "https://release.files.ghostty.org/libghostty-vt/$LIBGHOSTTY_VT_VERSION/$1.tar.gz" ||
        curl -fsSL --retry 3 -o "$tarball" "https://github.com/rbstp/collie/releases/download/ghostty-vt-$LIBGHOSTTY_VT_VERSION/$1.tar.gz"
    echo "$2  $tarball" | shasum -a 256 -c -
    rm -rf "${work:?}/$1"
    tar -xzf "$tarball" -C "$work"
    rm -f "$tarball"
}
unpack "libghostty-vt-$LIBGHOSTTY_VT_VERSION" "$SOURCE_SHA256"
unpack "libghostty-vt-$LIBGHOSTTY_VT_VERSION-deps" "$DEPS_SHA256"
echo "libghostty-vt: $LIBGHOSTTY_VT_VERSION"

# A new cache each time: Zig 0.17 can crash on a cache that holds package archives, as the
# fetches of earlier pins left in this one.
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
# Read by scripts/notices.py, which names each file's license.
licenses="$out/licenses"
mkdir "$licenses"
fetch() {
    curl -fsSL --retry 3 -o "$work/license" "$1"
    echo "$2  $work/license" | shasum -a 256 -c - >/dev/null
    cat "$work/license"
    rm -f "$work/license"
}
cp "$src/LICENSE" "$licenses/libghostty-vt"
{
    cat "$src/pkg/simdutf/vendor/LICENSE-MIT"
    printf '\ninclude/simdutf/internal/isadetection.h is derived from\n'
    sed -n '/^https:\/\/github.com\/endorno\/pytorch/,/POSSIBILITY OF SUCH DAMAGE\./p' "$src/pkg/simdutf/vendor/simdutf.h"
} >"$licenses/simdutf"
grep -q 'Redistributions in binary form' "$licenses/simdutf"
highway="$(sed -n 's/^ *\.hash = "\(.*\)",$/\1/p' "$src/pkg/highway/build.zig.zon")"
cp "$deps/$highway/LICENSE-BSD3" "$licenses/Highway"
{
    cat "$deps"/uucode-*/LICENSE.md
    echo
    fetch "$UUCODE_LICENSES/LICENSE_unicode" "$UUCODE_UNICODE_SHA256"
    echo
    fetch "$UUCODE_LICENSES/LICENSE_Bjoern_Hoehrmann" "$UUCODE_HOEHRMANN_SHA256"
} >"$licenses/uucode"
# Beside the binary in Zig's tarball, one level up in Homebrew's.
zig_license="$(dirname "$(realpath "$zig")")/LICENSE"
[ -f "$zig_license" ] || zig_license="$(dirname "$zig_license")/../LICENSE"
cp "$zig_license" "$licenses/Zig compiler-rt"
echo "$want" >"$stamp"
echo "built $out"
