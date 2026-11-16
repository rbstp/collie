# Sourced by build-xcframework.sh and build-collied.sh: the pins, the pinned Zig, the checked
# release tarballs and the license texts scripts/notices.py reads.

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$here/pins.env"
root="$(cd "$here/../.." && pwd)"
work="$root/target/ghostty"

sha256_check() {
    if command -v sha256sum >/dev/null; then
        echo "$1  $2" | sha256sum -c - >&2
    else
        echo "$1  $2" | shasum -a 256 -c - >&2
    fi
}

# The checksum-verified download on the hosts pins.env has one for; elsewhere a zig on PATH, only at
# the pinned version. The marker is written only after a whole verified extraction, which is then
# moved into place, so an interrupted download is never trusted.
find_zig() {
    case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) host=aarch64-macos sha="$ZIG_SHA256_AARCH64_MACOS" ;;
    Linux-x86_64) host=x86_64-linux sha="$ZIG_SHA256_X86_64_LINUX" ;;
    *)
        if command -v zig >/dev/null && [ "$(zig version)" = "$ZIG_VERSION" ]; then
            zig="$(command -v zig)"
            return
        fi
        echo "no pinned Zig download for $(uname -s) $(uname -m): put Zig $ZIG_VERSION on PATH" >&2
        exit 1
        ;;
    esac
    zigdir="$work/zig-$ZIG_VERSION"
    zig="$zigdir/zig"
    [ -x "$zig" ] && [ "$(cat "$zigdir/.collie-sha256" 2>/dev/null)" = "$sha" ] && return
    mkdir -p "$work"
    tmp="$(mktemp -d "$work/.zig-$ZIG_VERSION.XXXXXX")"
    curl -fsSL --retry 3 -o "$tmp/zig.tar.xz" "https://ziglang.org/download/$ZIG_VERSION/zig-$host-$ZIG_VERSION.tar.xz"
    sha256_check "$sha" "$tmp/zig.tar.xz"
    mkdir "$tmp/zig"
    tar -xJf "$tmp/zig.tar.xz" --strip-components=1 -C "$tmp/zig"
    echo "$sha" >"$tmp/zig/.collie-sha256"
    rm -rf "$zigdir"
    mv "$tmp/zig" "$zigdir"
    rm -rf "$tmp"
}

# What a build stamp records of its inputs beyond the pins: the scripts that made it ($1 is the
# build script).
inputs_sum() {
    cat "$here/pins.env" "$here/common.sh" "$1" | cksum
}

# Unpacks the source and dependency tarballs into $1, as $1/libghostty-vt-<version>[-deps].
unpack_sources() {
    for name in "libghostty-vt-$LIBGHOSTTY_VT_VERSION:$SOURCE_SHA256" "libghostty-vt-$LIBGHOSTTY_VT_VERSION-deps:$DEPS_SHA256"; do
        tarball="$1/${name%%:*}.tar.gz"
        mkdir -p "$1"
        # Fallback for when release.files.ghostty.org is down: the same tarballs on this repo's release.
        curl -fsSL --retry 3 -o "$tarball" "https://release.files.ghostty.org/libghostty-vt/$LIBGHOSTTY_VT_VERSION/${name%%:*}.tar.gz" ||
            curl -fsSL --retry 3 -o "$tarball" "https://github.com/rbstp/collie/releases/download/ghostty-vt-$LIBGHOSTTY_VT_VERSION/${name%%:*}.tar.gz"
        sha256_check "${name#*:}" "$tarball"
        rm -rf "${1:?}/${name%%:*}"
        tar -xzf "$tarball" -C "$1"
        rm -f "$tarball"
    done
}

# Writes the license texts of what links into the library, from sources $1 and dependencies $2,
# into the new folder $3. scripts/notices.py names each file's license.
write_licenses() {
    mkdir "$3"
    cp "$1/LICENSE" "$3/libghostty-vt"
    {
        cat "$1/pkg/simdutf/vendor/LICENSE-MIT"
        printf '\ninclude/simdutf/internal/isadetection.h is derived from\n'
        sed -n '/^https:\/\/github.com\/endorno\/pytorch/,/POSSIBILITY OF SUCH DAMAGE\./p' "$1/pkg/simdutf/vendor/simdutf.h"
    } >"$3/simdutf"
    grep -q 'Redistributions in binary form' "$3/simdutf"
    highway="$(sed -n 's/^ *\.hash = "\(.*\)",$/\1/p' "$1/pkg/highway/build.zig.zon")"
    cp "$2/$highway/LICENSE-BSD3" "$3/Highway"
    {
        cat "$2"/uucode-*/LICENSE.md
        echo
        fetch_license "$UUCODE_LICENSES/LICENSE_unicode" "$UUCODE_UNICODE_SHA256" "$3"
        echo
        fetch_license "$UUCODE_LICENSES/LICENSE_Bjoern_Hoehrmann" "$UUCODE_HOEHRMANN_SHA256" "$3"
    } >"$3/uucode"
    # Beside the binary in Zig's tarball, one level up in Homebrew's.
    zig_license="$(dirname "$(realpath "$zig")")/LICENSE"
    [ -f "$zig_license" ] || zig_license="$(dirname "$zig_license")/../LICENSE"
    cp "$zig_license" "$3/Zig compiler-rt"
}

fetch_license() {
    curl -fsSL --retry 3 -o "$3/.download" "$1"
    sha256_check "$2" "$3/.download"
    cat "$3/.download"
    rm -f "$3/.download"
}
