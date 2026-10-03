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
    cargo test --workspace

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
    rm -rf "$headers" "$pkg/CollieCore.xcframework"
    lib=target/aarch64-apple-ios/release/libcollie_core.a
    target/release/uniffi-bindgen "$lib" "$pkg/Sources/CollieCore" --swift-sources
    target/release/uniffi-bindgen "$lib" "$headers/collie_coreFFI" --headers --modulemap \
        --module-name collie_coreFFI --modulemap-filename module.modulemap
    xcodebuild -create-xcframework \
        -library "$lib" -headers "$headers" \
        -library target/aarch64-apple-ios-sim/release/libcollie_core.a -headers "$headers" \
        -output "$pkg/CollieCore.xcframework"

ios-project:
    xcodegen generate --spec Collie/project.yml

ios-build-sim: ios-framework ios-project
    {{ xcodebuild }} -destination 'platform=iOS Simulator,name={{ sim }}' CODE_SIGNING_ALLOWED=NO build

ios-test: ios-framework ios-project
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
