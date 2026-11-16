#!/usr/bin/env bash
# Builds collied's static libghostty-vt for one Rust target from the release tarballs pinned in
# pins.env into target/ghostty/collied/<target>: lib/libghostty-vt.a and the license texts
# scripts/notices.py reads. Run by crates/ghostty-vt-sys/build.rs.
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Grid references, the formatter and the snapshot decoder: what collied's Rex and Herdr Core screens read.
VT_FEATURES=-all,+grid-introspection,+formatter,+snapshot

target="${1:?usage: build-collied.sh <rust target>}"
case "$target" in
aarch64-apple-darwin) zig_target=aarch64-macos ;;
x86_64-unknown-linux-gnu) zig_target=x86_64-linux-gnu ;;
aarch64-unknown-linux-gnu) zig_target=aarch64-linux-gnu ;;
*)
    echo "libghostty-vt: no Zig target for $target" >&2
    exit 1
    ;;
esac

out="$work/collied/$target"
stamp="$out/.collie-build"
want="$LIBGHOSTTY_VT_VERSION $ZIG_VERSION $VT_FEATURES $zig_target"
if [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$want" ] && [ -f "$out/lib/libghostty-vt.a" ] && [ -d "$out/licenses" ]; then
    exit 0
fi

find_zig
echo "zig: $zig ($("$zig" version)), libghostty-vt $LIBGHOSTTY_VT_VERSION for $zig_target"
build="$out.build"
rm -rf "$out" "$build"
unpack_sources "$build"
src="$build/libghostty-vt-$LIBGHOSTTY_VT_VERSION"

# PACKAGING-LIBGHOSTTY.md requires an empty ZIG_GLOBAL_CACHE_DIR for every build. The ceiling keeps
# Ghostty's build from taking this repository's git tags for its version. A baseline CPU, not the
# build host's, since the release runners build for every machine.
(
    cd "$src"
    GIT_CEILING_DIRECTORIES="$work" ZIG_GLOBAL_CACHE_DIR="$build/zig-global-cache" "$zig" build \
        --system "$src-deps" \
        -Dtarget="$zig_target" \
        -Dcpu=baseline \
        -Doptimize=ReleaseFast \
        -Dvt-features="$VT_FEATURES" \
        --prefix "$build/out"
)

# Apple's linker takes the dylib over the archive for -l, so the archive gets a folder alone.
mkdir -p "$out/lib"
cp "$build/out/lib/libghostty-vt.a" "$out/lib/"
write_licenses "$src" "$src-deps" "$out/licenses"
rm -rf "$build"
echo "$want" >"$stamp"
