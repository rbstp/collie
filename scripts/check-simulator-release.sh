#!/bin/sh
set -eu
[ "$#" = 3 ] || { echo 'usage: check-simulator-release.sh RELEASE_APP RELEASE_COLLIED PRODUCTION_WORKER_BUNDLE' >&2; exit 1; }
app="$1"
if /usr/libexec/PlistBuddy -c 'Print :NSAppTransportSecurity:NSExceptionDomains:127.0.0.1' "$app/Info.plist" >/dev/null 2>&1; then
    echo 'Release app contains the Simulator ATS exception' >&2
    exit 1
fi
for artifact in "$app/Collie" "$2" "$3"; do
    if strings "$artifact" | rg 'COLLIE_SIMULATOR_RELAY|127\.0\.0\.1:8788|simulator-relay\.json|simulator-provisioned|local_provisioning_required'; then
        echo 'Release artifact contains Simulator relay support' >&2
        exit 1
    fi
done
echo 'Release app, daemon and production Worker exclude Simulator relay support'
