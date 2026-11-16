#!/usr/bin/env bash
# Builds collied's static libghostty-vt into target/ghostty/collied/lib from the pins in
# build-xcframework.sh, with the features collied reads: grid references, the formatter and the
# snapshot decoder. Prototype: arm64 macOS only (#764).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
eval "$(grep -E '^(LIBGHOSTTY_VT_VERSION|SOURCE_SHA256|DEPS_SHA256|ZIG_VERSION|ZIG_SHA256)=' "$here/build-xcframework.sh")"
VT_FEATURES=-all,+grid-introspection,+formatter,+snapshot

work="$root/target/ghostty"
src="$work/libghostty-vt-$LIBGHOSTTY_VT_VERSION"
deps="$src-deps"
out="$work/collied"
stamp="$out/.collie-build"
want="$LIBGHOSTTY_VT_VERSION $ZIG_VERSION $VT_FEATURES"

if [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$want" ] && [ -f "$out/lib/libghostty-vt.a" ]; then
    exit 0
fi
[ "$(uname -s)-$(uname -m)" = Darwin-arm64 ] || { echo "the prototype builds libghostty-vt on arm64 macOS only" >&2; exit 1; }
mkdir -p "$work"

if command -v zig >/dev/null && [ "$(zig version)" = "$ZIG_VERSION" ]; then
    zig="$(command -v zig)"
else
    zigdir="$work/zig-$ZIG_VERSION"
    zig="$zigdir/zig"
    if [ ! -x "$zig" ]; then
        tarball="$work/zig-aarch64-macos-$ZIG_VERSION.tar.xz"
        curl -fsSL -o "$tarball" "https://ziglang.org/download/$ZIG_VERSION/zig-aarch64-macos-$ZIG_VERSION.tar.xz"
        echo "$ZIG_SHA256  $tarball" | shasum -a 256 -c - >&2
        rm -rf "$zigdir"
        mkdir -p "$zigdir"
        tar -xJf "$tarball" --strip-components=1 -C "$zigdir"
        rm -f "$tarball"
    fi
fi

unpack() {
    tarball="$work/$1.tar.gz"
    curl -fsSL --retry 3 -o "$tarball" "https://release.files.ghostty.org/libghostty-vt/$LIBGHOSTTY_VT_VERSION/$1.tar.gz" ||
        curl -fsSL --retry 3 -o "$tarball" "https://github.com/rbstp/collie/releases/download/ghostty-vt-$LIBGHOSTTY_VT_VERSION/$1.tar.gz"
    echo "$2  $tarball" | shasum -a 256 -c - >&2
    rm -rf "${work:?}/$1"
    tar -xzf "$tarball" -C "$work"
    rm -f "$tarball"
}
unpack "libghostty-vt-$LIBGHOSTTY_VT_VERSION" "$SOURCE_SHA256"
unpack "libghostty-vt-$LIBGHOSTTY_VT_VERSION-deps" "$DEPS_SHA256"

# PACKAGING-LIBGHOSTTY.md requires an empty ZIG_GLOBAL_CACHE_DIR for every build.
cache="$work/zig-global-cache-collied"
rm -rf "$cache" "$out"
(
    cd "$src"
    GIT_CEILING_DIRECTORIES="$work" ZIG_GLOBAL_CACHE_DIR="$cache" "$zig" build \
        --system "$deps" \
        -Doptimize=ReleaseFast \
        -Dvt-features="$VT_FEATURES" \
        --prefix "$out/install" >&2
)
# Apple's linker takes the dylib over the archive for -l, so the archive gets a folder alone.
mkdir -p "$out/lib"
cp "$out/install/lib/libghostty-vt.a" "$out/lib/"
echo "$want" >"$stamp"
