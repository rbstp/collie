#!/usr/bin/env bash
set -euo pipefail

source_dir="$(cd "$(dirname "$0")" && pwd)"
binary="$source_dir/collied"
if [ ! -f "$binary" ]; then
    binary="$source_dir/../../bin/collied"
fi
[ -f "$binary" ] && [ -x "$binary" ] || { echo 'collied is missing from this package' >&2; exit 1; }
[ "$(id -u)" != 0 ] || { echo 'Run as your login user, without sudo' >&2; exit 1; }
[ "${1:-}" = --replace ] || {
    echo "Install $binary as ~/.cargo/bin/collied and refresh the existing service."
    echo 'Run with --replace to confirm replacing any installed daemon. User data is preserved.'
    exit 1
}
case "$(uname -s)" in
    Darwin)
        [ "$(uname -m)" = arm64 ] || { echo 'This package requires Apple silicon' >&2; exit 1; }
        codesign --verify --strict "$binary"
        signature="$(codesign -dv "$binary" 2>&1)"
        grep -Fx 'Identifier=dev.rbstp.collied' <<< "$signature" >/dev/null
        grep -Fx 'TeamIdentifier=RM3UT3MMSR' <<< "$signature" >/dev/null
        codesign --verify --strict -R=notarized --check-notarization "$binary"
        plist="$HOME/Library/LaunchAgents/dev.rbstp.collied.plist"
        agent="gui/$(id -u)/dev.rbstp.collied"
        running=false
        if launchctl print "$agent" >/dev/null 2>&1; then running=true; fi
        if [ -f "$plist" ]; then
            /usr/libexec/PlistBuddy -c 'Print :Label' "$plist" | grep -qx dev.rbstp.collied
            /usr/libexec/PlistBuddy -c 'Print :ProgramArguments:0' "$plist" >/dev/null
        fi
        ;;
    Linux)
        [ "$(uname -m)" = x86_64 ] || { echo 'This package requires Linux x86_64' >&2; exit 1; }
        [[ "$HOME" != *\"* && "$HOME" != *\'* && "$HOME" != *\\* ]] || { echo 'systemd cannot run an executable in this home path' >&2; exit 1; }
        if printf '%s' "$HOME" | LC_ALL=C grep -q '[[:cntrl:]]'; then echo 'Home path contains control characters' >&2; exit 1; fi
        command -v systemctl >/dev/null || { echo 'systemd is required' >&2; exit 1; }
        "$binary" --version
        unit="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/collied.service"
        running=false
        if systemctl --user --quiet is-active collied.service; then running=true; fi
        if [ -f "$unit" ]; then
            grep -Eq '^ExecStart="[^"]+" ("--config" "([^"\\]|\\.)*" )?"run"$' "$unit" || { echo 'Unrecognized collied ExecStart; update the service manually' >&2; exit 1; }
            actual="$(systemctl --user show collied.service --property=FragmentPath --value)"
            [ "$actual" = "$unit" ] || { echo "The user manager loads another unit: $actual" >&2; exit 1; }
        fi
        ;;
    *) echo 'Unsupported operating system' >&2; exit 1 ;;
esac

dest="$HOME/.cargo/bin/collied"
mkdir -p "$(dirname "$dest")"
tmp="$(mktemp "$(dirname "$dest")/.collied.XXXXXX")"
service_tmp=
trap 'rm -f "$tmp" "${service_tmp:-}"' EXIT
cp "$binary" "$tmp"
chmod 0755 "$tmp"
if [ "$(uname -s)" = Darwin ]; then codesign --verify --strict "$tmp"; fi
mv -f "$tmp" "$dest"

case "$(uname -s)" in
    Darwin)
        if [ -f "$plist" ]; then
            service_tmp="$(mktemp "$(dirname "$plist")/.collied.XXXXXX")"
            cp "$plist" "$service_tmp"
            /usr/libexec/PlistBuddy -c "Set :ProgramArguments:0 $dest" "$service_tmp"
            plutil -lint "$service_tmp"
            mv -f "$service_tmp" "$plist"
            if $running; then
                launchctl bootout "$agent"
                launchctl bootstrap "gui/$(id -u)" "$plist"
            fi
        fi
        ;;
    Linux)
        if [ -f "$unit" ]; then
            service_tmp="$(mktemp "$(dirname "$unit")/.collied.XXXXXX")"
            # systemd does not expand dollars in the executable path; percent still expands specifiers.
            escaped="$(printf '%s' "$dest" | sed 's/%/%%/g; s/[&|]/\\&/g')"
            sed "s|^ExecStart=\"[^\"]*\"|ExecStart=\"$escaped\"|" "$unit" > "$service_tmp"
            chmod 0644 "$service_tmp"
            mv -f "$service_tmp" "$unit"
            systemctl --user daemon-reload
            if $running; then systemctl --user restart collied.service; fi
        fi
        ;;
esac
printf 'Installed %s\n' "$dest"
echo 'Existing service running state and user data were preserved.'
echo 'On first install, add ~/.cargo/bin to PATH, then run collied setup.'
