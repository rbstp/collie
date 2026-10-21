set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

setup:
    git submodule update --init --recursive

schema:
    cargo run -q -p protocol --features schema --bin protocol-schema docs/protocol

litellm_commit := "0675c22d56a2c8592c6c716cd60177406a9751b9"

# The Claude and OpenAI chat prices from LiteLLM's table at `litellm_commit`, bundled in collied.
prices:
    curl -fsSL "https://raw.githubusercontent.com/BerriAI/litellm/{{ litellm_commit }}/model_prices_and_context_window.json" \
        | jq -S 'with_entries(select((.key | test("^[a-z0-9][a-z0-9.-]*$")) and (.value | type == "object") and (.value.litellm_provider == "anthropic" or .value.litellm_provider == "openai") and (.value.mode == "chat" or .value.mode == "responses") and (.value.input_cost_per_token | type == "number") and (.value.output_cost_per_token | type == "number"))) | map_values({input_cost_per_token, output_cost_per_token, cache_read_input_token_cost, cache_creation_input_token_cost, cache_creation_input_token_cost_above_1hr, input_cost_per_token_above_272k_tokens, output_cost_per_token_above_272k_tokens, cache_read_input_token_cost_above_272k_tokens, cache_creation_input_token_cost_above_272k_tokens, fast: .provider_specific_entry.fast} | with_entries(select(.value != null)))' \
        > crates/collied/data/prices.json

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
screenshot_watch := "true"
xcodebuild := "xcodebuild -project Collie/Collie.xcodeproj -scheme Collie -derivedDataPath target/ios/DerivedData"

ios-framework target="all" profile="release":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{ target }}" != all ] && [ "{{ target }}" != sim ]; then
        echo "ios-framework target must be all or sim" >&2
        exit 1
    fi
    cargo_args=(build -p collie-core)
    if [ "{{ profile }}" = release ]; then
        cargo_args+=(--release)
    elif [ "{{ profile }}" = debug ] && [ "{{ target }}" = sim ]; then
        :
    elif [ "{{ profile }}" != release ]; then
        echo "Debug core is only supported for the dedicated Simulator test" >&2
        exit 1
    fi
    export IPHONEOS_DEPLOYMENT_TARGET={{ ios_deployment_target }}
    pkg=Collie/CollieCore
    headers=target/ios/headers
    if [ "{{ target }}" = all ]; then
        cargo "${cargo_args[@]}" --target aarch64-apple-ios
    fi
    cargo "${cargo_args[@]}" --target aarch64-apple-ios-sim
    cargo build --release -p uniffi-bindgen
    sim_lib=target/aarch64-apple-ios-sim/{{ profile }}/libcollie_core.a
    lib="$sim_lib"
    if [ "{{ target }}" = all ]; then
        lib=target/aarch64-apple-ios/release/libcollie_core.a
    fi
    # Rewriting unchanged outputs makes Xcode recompile CollieCore and the app.
    stamp="$pkg/CollieCore.xcframework/.collie-build"
    want="$(printf '%s\n' '{{ target }}' '{{ profile }}'; shasum -a 256 "$lib" "$sim_lib" target/release/uniffi-bindgen)"
    if [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$want" ] && [ -f "$pkg/Sources/CollieCore/collie_core.swift" ]; then
        echo "CollieCore.xcframework is up to date"
        exit 0
    fi
    rm -rf "$headers" "$pkg/CollieCore.xcframework"
    target/release/uniffi-bindgen "$lib" "$pkg/Sources/CollieCore" --swift-sources
    target/release/uniffi-bindgen "$lib" "$headers/collie_coreFFI" --headers --modulemap \
        --module-name collie_coreFFI --modulemap-filename module.modulemap
    libraries=(-library "$sim_lib" -headers "$headers")
    if [ "{{ target }}" = all ]; then
        libraries=(-library "$lib" -headers "$headers" "${libraries[@]}")
    fi
    xcodebuild -create-xcframework "${libraries[@]}" -output "$pkg/CollieCore.xcframework"
    echo "$want" > "$stamp"

# libghostty-vt from a pinned Ghostty commit; skipped when already built for that pin.
ios-ghostty:
    scripts/ghostty/build-xcframework.sh

ios-project: ios-ghostty
    xcodegen generate --spec Collie/project.yml

ios-build-sim: ios-framework ios-project
    {{ xcodebuild }} -destination 'platform=iOS Simulator,name={{ sim }}' CODE_SIGNING_ALLOWED=NO build

ios-test diagnostics="on-failure": ios-framework ios-project
    swift test --package-path Collie/GhosttyTerminal
    cd Collie/GhosttyTerminal && xcodebuild -scheme GhosttyTerminal -destination 'platform=iOS Simulator,name={{ sim }}' -derivedDataPath ../../target/ios/GhosttyTerminal CODE_SIGNING_ALLOWED=NO test
    {{ xcodebuild }} -destination 'platform=iOS Simulator,name={{ sim }}' -collect-test-diagnostics '{{ diagnostics }}' CODE_SIGNING_ALLOWED=NO test

# The documentation screenshots, light and dark, from the demo on one fixed simulator and its paired watch, plus the App Store set in target/ios/app-store.
ios-screenshots: ios-framework ios-project
    #!/usr/bin/env bash
    set -euo pipefail
    device="iPhone 18 Pro Max"
    udid="$(xcrun simctl list devices available -j | jq -r --arg name "$device" '[.devices[][] | select(.name == $name) | .udid][0] // empty')"
    [ -n "$udid" ] || { echo "no available simulator named $device"; exit 1; }
    watch_state=
    if [ "{{ screenshot_watch }}" = true ]; then
        watch="$(xcrun simctl list pairs -j | jq -r --arg phone "$udid" '[.pairs[] | select(.phone.udid == $phone) | .watch.udid][0] // empty')"
        [ -n "$watch" ] || { echo "no watch simulator paired with $device (xcrun simctl pair)"; exit 1; }
        watch_state="$(xcrun simctl list devices -j | jq -r --arg id "$watch" '.devices[][] | select(.udid == $id) | .state')"
    fi
    xcrun simctl bootstatus "$udid" -b
    appearance="$(xcrun simctl ui "$udid" appearance)"
    trap 'xcrun simctl status_bar "$udid" clear; xcrun simctl ui "$udid" appearance "$appearance"; if [ "$watch_state" = Shutdown ]; then xcrun simctl shutdown "$watch" || true; fi' EXIT
    xcrun simctl status_bar "$udid" override --time 9:41 --dataNetwork wifi --wifiMode active --wifiBars 3 \
        --cellularMode active --cellularBars 4 --operatorName '' --batteryState discharging --batteryLevel 100
    {{ xcodebuild }} -destination "id=$udid" CODE_SIGNING_ALLOWED=NO build-for-testing
    # A fresh install drops drafts left by other UI tests; the demo takes its accent from the prefs file.
    xcrun simctl uninstall "$udid" dev.rbstp.collie
    xcrun simctl install "$udid" target/ios/DerivedData/Build/Products/Debug-iphonesimulator/Collie.app
    state="$(xcrun simctl get_app_container "$udid" dev.rbstp.collie data)/Library/Application Support/collie"
    mkdir -p "$state"
    chmod 700 "$state"
    echo '{"accent":"gold"}' > "$state/prefs.json"
    raw="$PWD/target/ios/screenshots"
    rm -rf "$raw"
    mkdir -p "$raw"
    TEST_RUNNER_COLLIE_SCREENSHOTS="$raw" {{ xcodebuild }} -destination "id=$udid" CODE_SIGNING_ALLOWED=NO \
        -only-testing CollieUITests/ScreenshotTests test-without-building
    out=docs/site/src/images
    mkdir -p "$out"
    for f in "$raw"/*.png; do
        sips --resampleWidth 600 "$f" --out "$out/$(basename "$f")" >/dev/null
    done
    if [ "{{ screenshot_watch }}" = true ]; then
        # bootstatus exits 0 even when a new watch shuts down during its first-boot data migration; booting it again completes.
        for _ in 1 2 3; do
            xcrun simctl bootstatus "$watch" -b
            watch_now="$(xcrun simctl list devices -j | jq -r --arg id "$watch" '.devices[][] | select(.udid == $id) | .state')"
            [ "$watch_now" != Booted ] || break
        done
        [ "$watch_now" = Booted ] || { echo "the watch simulator did not stay booted ($watch_now)"; exit 1; }
        # watchOS has no light mode and no status bar override, so one copy each, with the current time.
        xcrun simctl install "$watch" target/ios/DerivedData/Build/Products/Debug-watchsimulator/CollieWatch.app
        for mode in widget usage; do
            SIMCTL_CHILD_COLLIE_DEMO_SCREENSHOTS="$mode" xcrun simctl launch --terminate-running-process "$watch" dev.rbstp.collie.watchkitapp >/dev/null
            sleep 4
            xcrun simctl io "$watch" screenshot "$raw/watch-$mode.png" >/dev/null 2>&1
            cp "$raw/watch-$mode.png" "$out/"
        done
        xcrun simctl terminate "$watch" dev.rbstp.collie.watchkitapp
    fi
    store="$PWD/target/ios/app-store"
    rm -rf "$store"
    mkdir -p "$store"
    # App Store Connect refuses alpha; a PPM round trip drops it without loss.
    for f in "$raw"/*.png; do
        sips -s format pbm "$f" --out "$store/tmp.ppm" >/dev/null
        sips -s format png "$store/tmp.ppm" --out "$store/$(basename "$f")" >/dev/null
    done
    rm "$store/tmp.ppm"
    if command -v pngquant >/dev/null; then
        # Only this run's captures: compressing a committed image again can still change it.
        for f in "$raw"/*.png; do
            # 98 and 99: kept as is, the result would be larger or below the quality floor.
            pngquant --quality 70-90 --strip --skip-if-larger --force --ext .png "$out/$(basename "$f")" || [ $? -ge 98 ]
        done
    else
        echo "pngquant not found: the screenshots are not compressed (brew install pngquant)"
    fi
    du -ch "$out"/*.png | tail -n 1
    echo "App Store screenshots: $store"

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

mac_xcodebuild := "xcodebuild -project Mac/CollieBar.xcodeproj -scheme CollieBar -derivedDataPath target/mac/DerivedData"

[macos]
mac-project:
    xcodegen generate --spec Mac/project.yml

# Unsigned Release build of the menu bar app.
[macos]
mac-build: mac-project
    {{ mac_xcodebuild }} -configuration Release CODE_SIGNING_ALLOWED=NO build

[macos]
mac-test: mac-project
    {{ mac_xcodebuild }} -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO test

# Menu bar app signed with the team RM3UT3MMSR Developer ID and the hardened runtime, installed to ~/Applications. Not notarized.
[macos]
mac-install: mac-build
    #!/usr/bin/env bash
    set -euo pipefail
    identity="$(security find-identity -v -p codesigning \
        | sed -n 's/^ *[0-9][0-9]*) [0-9A-F]\{40\} "\(Developer ID Application: .* (RM3UT3MMSR)\)"$/\1/p' | head -n 1)"
    [ -n "$identity" ] || { echo "no valid \"Developer ID Application: ... (RM3UT3MMSR)\" identity in the keychain"; exit 1; }
    app=target/mac/CollieBar.app
    rm -rf "$app"
    ditto target/mac/DerivedData/Build/Products/Release/CollieBar.app "$app"
    codesign --force --sign "$identity" --options runtime --timestamp "$app"
    codesign --verify --strict --verbose=2 "$app"
    entitlements="$(codesign -d --entitlements - "$app" 2>/dev/null)"
    [ -z "$entitlements" ] || { echo "unexpected entitlements:"; echo "$entitlements"; exit 1; }
    osascript -e 'quit app id "dev.rbstp.colliebar"' >/dev/null 2>&1 || true
    dest="$HOME/Applications/CollieBar.app"
    mkdir -p "$HOME/Applications"
    # A fresh copy beside the old one, then swapped in: the path stays stable for the login item.
    tmp="$(mktemp -d "$HOME/Applications/.CollieBar.XXXXXX")"
    trap 'rm -rf "$tmp"' EXIT
    ditto "$app" "$tmp/CollieBar.app"
    rm -rf "$dest"
    mv "$tmp/CollieBar.app" "$dest"
    codesign --verify --strict --verbose=2 "$dest"
    echo "installed $dest, signed by $identity"
    open "$dest"

# Optional: notarizes and staples the app mac-install signed. Credentials from `xcrun notarytool store-credentials <profile>`.
[macos]
mac-notarize profile="collie":
    #!/usr/bin/env bash
    set -euo pipefail
    app=target/mac/CollieBar.app
    info="$(codesign -dvv "$app" 2>&1)" || true
    grep -q '^Authority=Developer ID Application: .* (RM3UT3MMSR)$' <<<"$info" \
        || { echo "$app is not signed with the RM3UT3MMSR Developer ID: run just mac-install first"; exit 1; }
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    ditto -c -k --keepParent "$app" "$tmp/CollieBar.zip"
    # On failure: xcrun notarytool log <submission id> --keychain-profile "{{ profile }}"
    xcrun notarytool submit "$tmp/CollieBar.zip" --keychain-profile "{{ profile }}" --wait
    xcrun stapler staple "$app"
    dest="$HOME/Applications/CollieBar.app"
    if [ -d "$dest" ]; then xcrun stapler staple "$dest"; fi
    spctl --assess --type execute --verbose=2 "$app"

# The Omarchy bar widget's tests: wire format and parsing under node, Service.qml in qs against a fake collied.
[linux]
tray-test:
    cmp Mac/CollieBar/Assets.xcassets/MenuIcon.imageset/MenuIcon.svg Linux/CollieTray/MenuIcon.svg
    omarchy-plugin-validate Linux/CollieTray
    node --test Linux/Tests/model.test.cjs
    node Linux/Tests/run-service-test.cjs

# Copies the Omarchy bar widget to ~/.config/omarchy/plugins/rbstp.collie and puts it on the bar.
[linux]
tray-install:
    #!/usr/bin/env bash
    set -euo pipefail
    omarchy-plugin-validate Linux/CollieTray
    plugins="$HOME/.config/omarchy/plugins"
    dest="$plugins/rbstp.collie"
    mkdir -p "$plugins"
    # A dot directory: the shell's watcher ignores it, so the copy reloads plugins once, at the swap.
    tmp="$(mktemp -d "$plugins/.rbstp.collie.XXXXXX")"
    trap 'rm -rf "$tmp"' EXIT
    cp Linux/CollieTray/{manifest.json,Service.qml,Panel.qml,Model.js,Printable.js,MenuIcon.svg} "$tmp"
    chmod 0755 "$tmp"
    updating=false
    [ -d "$dest" ] && updating=true
    rm -rf "$dest"
    mv "$tmp" "$dest"
    # The shell's hot reload can keep a changed widget's old component cached.
    if $updating; then omarchy restart shell >/dev/null; else omarchy-shell shell rescanPlugins >/dev/null; fi
    # The shell answers "not ready" until it has found the plugin; on the bar already is ok.
    r=""
    for _ in $(seq 100); do
        r="$(omarchy-shell shell putBarWidget rbstp.collie '{"section":"right"}' 2>&1 || true)"
        [ "$r" = ok ] && break
        sleep 0.1
    done
    [ "$r" = ok ] || { echo "tray-install: could not put rbstp.collie on the bar: $r" >&2; exit 1; }
    echo "installed $dest"

vm-setup:
    python3 scripts/vm-simulator.py setup

vm-test:
    python3 scripts/vm-simulator.py test

vm-reset:
    python3 scripts/vm-simulator.py reset

vm-stop:
    python3 scripts/vm-simulator.py stop

vm-clean:
    python3 scripts/vm-simulator.py clean

vm-desktop-ready:
    python3 scripts/vm-simulator.py desktop-ready
