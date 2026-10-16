#!/bin/sh
set -eu
[ "$#" = 3 ] || { echo 'usage: check-simulator-release.sh RELEASE_APP RELEASE_COLLIED PRODUCTION_WORKER_BUNDLE' >&2; exit 1; }
app="$1"
for artifact in "$app/Info.plist" "$app/Collie" "$2" "$3"; do
    [ -f "$artifact" ] && [ -r "$artifact" ] && [ -s "$artifact" ] || {
        echo "Missing, unreadable or empty Release artifact: $artifact" >&2
        exit 1
    }
done
[ -x "$app/Collie" ] && [ -x "$2" ] || { echo 'Release app and daemon must be executable' >&2; exit 1; }
plutil -lint "$app/Info.plist" >/dev/null
if /usr/libexec/PlistBuddy -c 'Print :NSAppTransportSecurity:NSExceptionDomains:127.0.0.1' "$app/Info.plist" >/dev/null 2>&1; then
    echo 'Release app contains the Simulator ATS exception' >&2
    exit 1
fi
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
for artifact in "$app/Collie" "$2" "$3"; do
    strings "$artifact" > "$scratch/strings"
    if rg 'COLLIE_SIMULATOR_RELAY|COLLIE_VM_TEST|collie_vm_test_supported|vm-tailnet\.json|vm-core-(rejection|identity)\.json|127\.0\.0\.1:878[89]|simulator-relay\.json|simulator-provisioned|local_provisioning_required' "$scratch/strings"; then
        echo 'Release artifact contains Simulator relay support' >&2
        exit 1
    else
        result=$?
        [ "$result" -eq 1 ] || exit "$result"
    fi
done
echo 'Release app, daemon and production Worker exclude Simulator relay support'
