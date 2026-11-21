#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"

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
want="$LIBGHOSTTY_VT_VERSION $ZIG_VERSION $VT_FEATURES $zig_target $(inputs_sum "${BASH_SOURCE[0]}")"
if [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$want" ] && [ -f "$out/lib/libghostty-vt.a" ] && [ -d "$out/licenses" ]; then
    exit 0
fi
if restore_build "$out" "$want"; then
    exit 0
fi

find_zig
echo "zig: $zig ($("$zig" version)), libghostty-vt $LIBGHOSTTY_VT_VERSION for $zig_target"
build="$out.build"
rm -rf "$out" "$build"
unpack_sources "$build"
src="$build/libghostty-vt-$LIBGHOSTTY_VT_VERSION"

# ADR-0005
(
    cd "$src"
    GIT_CEILING_DIRECTORIES="$work" ZIG_GLOBAL_CACHE_DIR="$build/zig-global-cache" "$zig" build \
        --system "$src-deps" \
        -Dtarget="$zig_target" \
        -Dcpu=baseline \
        -Doptimize=ReleaseSafe \
        -Dvt-features="$VT_FEATURES" \
        --prefix "$build/out"
)

mkdir -p "$out/lib"
cp "$build/out/lib/libghostty-vt.a" "$out/lib/"
write_licenses "$src" "$src-deps" "$out/licenses"
rm -rf "$build"
echo "$want" >"$stamp"
save_build "$out" "$want"
