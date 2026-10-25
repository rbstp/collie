#!/usr/bin/env bash
set -euo pipefail
for name in DESKTOP_CERT_P12 DESKTOP_CERT_PASSWORD APPLE_KEY_P8 APPLE_KEY_ID APPLE_ISSUER_ID; do
    [ -n "${!name}" ] || { echo "Missing secret: $name" >&2; exit 1; }
done
material="$(mktemp -d "$RUNNER_TEMP/collie-desktop.XXXXXX")"
keychain="$material/signing.keychain-db"
security list-keychains -d user > "$material/keychains"
cleanup() {
    security delete-keychain "$keychain" >/dev/null 2>&1 || true
    xargs security list-keychains -d user -s < "$material/keychains" || true
    rm -rf "$material"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
password="$(openssl rand -base64 24)"
printf '%s' "$DESKTOP_CERT_P12" | base64 --decode > "$material/cert.p12"
printf '%s' "$APPLE_KEY_P8" | base64 --decode > "$material/AuthKey.p8"
chmod 0600 "$material"/*
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$material/cert.p12" -k "$keychain" -P "$DESKTOP_CERT_PASSWORD" -T /usr/bin/codesign -f pkcs12
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
xargs security list-keychains -d user -s "$keychain" < "$material/keychains"
identity="$(security find-identity -v -p codesigning "$keychain" | sed -n 's/^ *[0-9][0-9]*) \([0-9A-F]\{40\}\) "Developer ID Application: .* (RM3UT3MMSR)"$/\1/p')"
[ -n "$identity" ] && [ "$(printf '%s\n' "$identity" | wc -l | tr -d ' ')" = 1 ]
binary=target/aarch64-apple-darwin/release/collied
app=target/mac/Build/Products/Release/CollieBar.app
codesign --force --sign "$identity" --keychain "$keychain" --identifier dev.rbstp.collied --options runtime --timestamp "$binary"
codesign --force --sign "$identity" --keychain "$keychain" --options runtime --timestamp "$app"
codesign --verify --strict "$binary"
codesign --verify --strict "$app"
lipo "$binary" -verify_arch arm64
lipo "$app/Contents/MacOS/CollieBar" -verify_arch arm64
mkdir -p dist
python3 scripts/desktop/package.py package --tag "$TAG" --sha "$SOURCE_SHA" --platform macos-arm64 --binary "$binary" --notices target/THIRD_PARTY_NOTICES --out dist
mkdir "$material/daemon"
tar -xzf "dist/collied-${TAG#v}-macos-arm64.tar.gz" -C "$material/daemon"
hdiutil create -volname collied -srcfolder "$material/daemon" -format UDZO "dist/collied-${TAG#v}-macos-arm64.dmg"
codesign --sign "$identity" --keychain "$keychain" --timestamp "dist/collied-${TAG#v}-macos-arm64.dmg"
ditto -c -k --keepParent "$app" "$material/CollieBar.zip"
for package in "dist/collied-${TAG#v}-macos-arm64.dmg" "$material/CollieBar.zip"; do
    xcrun notarytool submit "$package" --key "$material/AuthKey.p8" --key-id "$APPLE_KEY_ID" --issuer "$APPLE_ISSUER_ID" --wait --output-format json > "$material/notary.json"
    /usr/bin/plutil -extract status raw "$material/notary.json" | grep -qx Accepted
done
xcrun stapler staple "dist/collied-${TAG#v}-macos-arm64.dmg"
xcrun stapler validate "dist/collied-${TAG#v}-macos-arm64.dmg"
xcrun stapler staple "$app"
xcrun stapler validate "$app"
spctl --assess --type open --context context:primary-signature "dist/collied-${TAG#v}-macos-arm64.dmg"
codesign --verify --strict -R=notarized --check-notarization "$binary"
spctl --assess --type execute "$app"
ditto -c -k --keepParent "$app" "dist/CollieBar-${TAG#v}-macos-arm64.zip"
