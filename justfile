set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

setup:
    git submodule update --init --recursive

schema:
    cargo run -q -p protocol --bin protocol-schema docs/protocol

fmt:
    cargo fmt --all

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo deny check

test:
    cargo nextest run --workspace

# Release build of collied signed with the team RM3UT3MMSR Developer ID, installed to ~/.cargo/bin.
[macos]
collied-install:
    #!/usr/bin/env bash
    set -euo pipefail
    identity="$(security find-identity -v -p codesigning \
        | sed -n 's/^ *[0-9][0-9]*) [0-9A-F]\{40\} "\(Developer ID Application: .* (RM3UT3MMSR)\)"$/\1/p' | head -n 1)"
    [ -n "$identity" ] || { echo "no valid \"Developer ID Application: ... (RM3UT3MMSR)\" identity in the keychain"; exit 1; }
    cargo build --release -p collied
    bin=target/release/collied
    codesign --force --sign "$identity" --identifier dev.rbstp.collied --options runtime --timestamp "$bin"
    codesign --verify --strict --verbose=2 "$bin"
    dest="$HOME/.cargo/bin/collied"
    mkdir -p "$(dirname "$dest")"
    # A fresh inode: rewriting a signed binary in place can get the running copy killed.
    tmp="$(mktemp "$(dirname "$dest")/.collied.XXXXXX")"
    trap 'rm -f "$tmp"' EXIT
    cp "$bin" "$tmp"
    chmod 0755 "$tmp"
    mv -f "$tmp" "$dest"
    codesign --verify --strict --verbose=2 "$dest"
    echo "installed $dest, signed by $identity"
    agent="gui/$(id -u)/dev.rbstp.collied"
    if launchctl print "$agent" >/dev/null 2>&1; then
        launchctl kickstart -k "$agent"
        echo "restarted $agent"
    fi

# Release build of collied installed to ~/.cargo/bin; restarts the systemd user unit if it runs.
[linux]
collied-install:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -p collied
    bin=target/release/collied
    dest="$HOME/.cargo/bin/collied"
    mkdir -p "$(dirname "$dest")"
    # A fresh inode, so the running daemon keeps its own copy until it restarts.
    tmp="$(mktemp "$(dirname "$dest")/.collied.XXXXXX")"
    trap 'rm -f "$tmp"' EXIT
    cp "$bin" "$tmp"
    chmod 0755 "$tmp"
    mv -f "$tmp" "$dest"
    echo "installed $dest"
    if systemctl --user --quiet is-active collied.service; then
        systemctl --user restart collied.service
        echo "restarted collied.service"
    fi

ios_deployment_target := "26.0"
sim := "iPhone 18 Pro"
xcodebuild := "xcodebuild -project Collie/Collie.xcodeproj -scheme Collie -derivedDataPath target/ios/DerivedData"

ios-framework:
    #!/usr/bin/env bash
    set -euo pipefail
    export IPHONEOS_DEPLOYMENT_TARGET={{ ios_deployment_target }}
    pkg=Collie/CollieCore
    headers=target/ios/headers
    cargo build --release -p collie-core --target aarch64-apple-ios
    cargo build --release -p collie-core --target aarch64-apple-ios-sim
    cargo build --release -p uniffi-bindgen
    lib=target/aarch64-apple-ios/release/libcollie_core.a
    # Rewriting unchanged outputs makes Xcode recompile CollieCore and the app.
    stamp="$pkg/CollieCore.xcframework/.collie-build"
    want="$(shasum -a 256 "$lib" target/aarch64-apple-ios-sim/release/libcollie_core.a target/release/uniffi-bindgen)"
    if [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$want" ] && [ -f "$pkg/Sources/CollieCore/collie_core.swift" ]; then
        echo "CollieCore.xcframework is up to date"
        exit 0
    fi
    rm -rf "$headers" "$pkg/CollieCore.xcframework"
    target/release/uniffi-bindgen "$lib" "$pkg/Sources/CollieCore" --swift-sources
    target/release/uniffi-bindgen "$lib" "$headers/collie_coreFFI" --headers --modulemap \
        --module-name collie_coreFFI --modulemap-filename module.modulemap
    xcodebuild -create-xcframework \
        -library "$lib" -headers "$headers" \
        -library target/aarch64-apple-ios-sim/release/libcollie_core.a -headers "$headers" \
        -output "$pkg/CollieCore.xcframework"
    echo "$want" > "$stamp"

# libghostty-vt from a pinned Ghostty commit; skipped when already built for that pin.
ios-ghostty:
    scripts/ghostty/build-xcframework.sh

ios-project: ios-ghostty
    xcodegen generate --spec Collie/project.yml

ios-build-sim: ios-framework ios-project
    {{ xcodebuild }} -destination 'platform=iOS Simulator,name={{ sim }}' CODE_SIGNING_ALLOWED=NO build

ios-test: ios-framework ios-project
    swift test --package-path Collie/GhosttyTerminal
    {{ xcodebuild }} -destination 'platform=iOS Simulator,name={{ sim }}' CODE_SIGNING_ALLOWED=NO test

# Debug build with automatic signing on the first connected iPhone. Release keeps its manual TestFlight signing.
ios-run-device: ios-framework ios-project
    #!/usr/bin/env bash
    set -euo pipefail
    devices="$(mktemp)"
    trap 'rm -f "$devices"' EXIT
    xcrun devicectl list devices --json-output "$devices" >/dev/null
    udid="$(jq -r '[.result.devices[]
        | select(.hardwareProperties.platform == "iOS"
            and .hardwareProperties.reality == "physical"
            and .connectionProperties.pairingState == "paired"
            and .connectionProperties.tunnelState != "unavailable")
        | .hardwareProperties.udid][0] // empty' "$devices")"
    [ -n "$udid" ] || { echo "no connected, paired iOS device (see xcrun devicectl list devices)"; exit 1; }
    {{ xcodebuild }} -configuration Debug -destination "id=$udid" -allowProvisioningUpdates -allowProvisioningDeviceRegistration \
        CODE_SIGN_STYLE=Automatic DEVELOPMENT_TEAM=RM3UT3MMSR build
    xcrun devicectl device install app --device "$udid" target/ios/DerivedData/Build/Products/Debug-iphoneos/Collie.app
    xcrun devicectl device process launch --device "$udid" dev.rbstp.collie
