# Simulator relay testing

No Simulator mode establishes genuine App Attest or Secure Enclave trust.

| Mode | Coverage |
|---|---|
| Normal pairing over Tailscale and mutual TLS | Paired sessions, approvals, chat and terminal, with software identity keys |
| Local relay with mock APNs (loopback HTTP, isolated D1) | Signed control, encrypted envelopes, routes, quotas, replay, revocation, outages |
| `simctl push` payload injection | Rendering and routing only, not APNs, the notification service extension or decryption |
| Local relay with sandbox APNs (real Apple HTTP/2) | Sandbox alert, extension and ActivityKit delivery on supported hosts, only when observed |
| Physical device or TestFlight (genuine App Attest and APNs) | Production integrity, Secure Enclave, background, locked and pre-first-unlock behavior, production delivery |

## Build and pair normally

Use a dedicated Simulator, a daemon you own and a scratch herdr workspace. Never send input to an existing agent. Keep the normal [tailnet policy](tailnet.md) and local pairing confirmation.

```sh
just setup
export GO=/opt/homebrew/opt/go/bin/go
simulator='iPhone 18 Pro Max'
just sim="$simulator" ios-build-sim
simulator_id=$(xcrun simctl list devices available -j | jq -r --arg name "$simulator" '[.devices[][] | select(.name == $name) | .udid][0]')
xcrun simctl bootstatus "$simulator_id" -b
xcrun simctl install "$simulator_id" target/ios/DerivedData/Build/Products/Debug-iphonesimulator/Collie.app
xcrun simctl launch "$simulator_id" dev.rbstp.collie
collied pair --show-uri
```

In the Simulator app, sign into Tailscale as the daemon owner and paste the printed `collie://pair#...` link, a one-use secret you never commit or share, in **Machines > Add machine**. Check the user and Simulator identity at the daemon's prompt and approve that one pairing. Verify a real session with a scratch agent before testing notifications.

## Local mock relay

The harness creates a fresh P-256 identity and never imports a real installation key or reads your Keychain. Keep its private directory (key, send secret, isolated D1, last mock payload) outside the repository.

```sh
cd relay
npm ci
npm run check
npm test
node dev/simulator.ts init /private/tmp/collie-simulator-relay
node dev/simulator.ts serve /private/tmp/collie-simulator-relay
```

The server listens only on `127.0.0.1:8788`, refuses enrollment and production APNs, and needs no Cloudflare or Apple credentials. Beyond provisioning, it runs the production validators and a real Miniflare `RelayCrypto` actor ([design](architecture.md#simulator-relay-isolation)). Its send secret cannot authorize production routes.

In a second terminal at the repository root, copy only the new identity into the DEBUG Simulator app:

```sh
simulator='iPhone 18 Pro Max'
simulator_id=$(xcrun simctl list devices available -j | jq -r --arg name "$simulator" '[.devices[][] | select(.name == $name) | .udid][0]')
container=$(xcrun simctl get_app_container "$simulator_id" dev.rbstp.collie data)
mkdir -p "$container/Library/Application Support/collie"
chmod 700 "$container/Library/Application Support/collie"
install -m 600 /private/tmp/collie-simulator-relay/simulator-relay.json "$container/Library/Application Support/collie/simulator-relay.json"
SIMCTL_CHILD_COLLIE_SIMULATOR_RELAY=mock xcrun simctl launch --terminate-running-process "$simulator_id" dev.rbstp.collie
```

`mock` gives the app a synthetic push destination so hosts without APNs support can exercise registration; it simulates no Apple token, extension or ActivityKit token.

Stop the existing daemon process, leaving any installed binary or service alone, and run a source Debug daemon with `[push.relay]` configured. Its Debug-only switch fixes the endpoint to `http://127.0.0.1:8788/v1/send`.

```sh
cargo build -p collied
COLLIE_SIMULATOR_RELAY=1 target/debug/collied run
```

Pair, open the app so it sends its app-token grants, then run `COLLIE_SIMULATOR_RELAY=1 target/debug/collied apns test` (its own sender needs the switch too). `accepted` means only the mock accepted the encrypted request. To render the last payload, the only one the mock keeps:

```sh
xcrun simctl push "$simulator_id" dev.rbstp.collie /private/tmp/collie-simulator-relay/last-push.apns
```

The generic fallback must stay useful without decryption or the extension.

For an outage, restart the server with the same directory and `COLLIE_SIMULATOR_APNS_OUTAGE=1`, send, check `target/debug/collied doctor`, then restart without the flag and verify that bounded retries recover. Server restarts keep the database and assertion counters; daemon restarts drop the in-memory queue. Removing the scratch pairing must revoke its grants.

## Real sandbox APNs

Apple silicon or T2 Macs on macOS 13 or later with an iOS 16 or later Simulator can register with APNs Sandbox, though some hosts, runtimes or networks still fail ([Simulator APNs support](https://developer.apple.com/documentation/xcode-release-notes/xcode-14-release-notes), [ActivityKit push testing](https://developer.apple.com/documentation/activitykit/starting-and-updating-live-activities-with-activitykit-push-notifications)).

Use a locally held APNs signing key authorized for the app's team and topic. Never share it, copy it from another user's Keychain, put it in the identity file or commit it. Stop the mock server (both modes use the same port) and create a fresh directory with your ten-character team and key IDs:

```sh
cd relay
node dev/simulator.ts init /private/tmp/collie-simulator-sandbox YOURTEAMID
COLLIE_SIMULATOR_APNS_KEY_FILE=/absolute/private/path/AuthKey_KEYID.p8 \
COLLIE_SIMULATOR_APNS_KEY_ID=YOURKEYID1 \
node dev/simulator.ts serve /private/tmp/collie-simulator-sandbox
```

The sender refuses every host but `api.sandbox.push.apple.com`.

Copy this identity into a fresh dedicated Simulator as above and launch with `SIMCTL_CHILD_COLLIE_SIMULATOR_RELAY=sandbox`. The app then needs a real APNs token, from a locally signed DEBUG build with the sandbox entitlement if unsigned registration is refused. After pairing, Settings must report a sandbox registration; if it fails, report that and use mock tests, never a fake token.

Run the owned Debug daemon with the same isolated configuration and data directory. Send a generic `COLLIE_SIMULATOR_RELAY=1 target/debug/collied apns test` and record Apple's response apart from display; its unknown node ID cannot prove decryption. For that, background the app and send a session-linked alert with your scratch workspace's terminal ID (from its creation response):

```sh
COLLIE_SIMULATOR_RELAY=1 target/debug/collied apns test --terminal-id "$terminal_id"
```

Require Sandbox acceptance and **Tap to open the selected agent session.** in the system notification; that text comes from the encrypted display envelope, unlike the generic fallback. Check the accessibility hierarchy and a screenshot separately: combined labels can hide text from an exact query, and a black screenshot proves nothing. Bound sends in advance and never resend automatically after a failed assertion.

Test approval, done and agent alerts and the generic missing-key fallback separately. For a Live Activity:

1. Follow a scratch agent, wait for its real ActivityKit token from `pushTokenUpdates`, then background the app, still following.
2. Record the decoded status and workspace name, change the session to another non-fallback status, and require the new values in the system activity while the app stays backgrounded. `unknown` proves nothing: it is also the decryption-failure fallback.
3. Close only that scratch terminal or workspace, still backgrounded, and require the daemon's `event=end` send and an ended activity. Unfollowing ends the activity only locally.

Activity pushes use their own token: require topic `dev.rbstp.collie.push-type.liveactivity`, push type `liveactivity` and Apple acceptance for each update and end. [notifications.md](notifications.md) covers decisions, expiry and background clearing.

Validation on 2026-10-17 used revision `13929290177e309445dae633a6f1f85eff4564ff`, a Debug iOS 27.2 Simulator app and an isolated macOS 27 VM:

- Apple accepted two real Sandbox alert requests; the session-linked alert showed its decrypted text in system accessibility and a visible screenshot. A disappearing-banner assertion later failed the private test runner as a whole.
- ActivityKit registered a genuine activity token, and Apple accepted two update requests. The decoded initial `idle` state showed while the app was suspended, but the distinct `working` state did not appear within the bounded wait, and remote end was not reached. The initial state can come from the local activity request, so remote ActivityKit update and end remain unverified.
- Routine priority `5` pushes may be delayed or batched, so a bounded-wait failure alone does not establish an app bug ([Apple's APNs delivery behavior](https://developer.apple.com/documentation/usernotifications/sending-notification-requests-to-apns)).
- These results do not establish genuine App Attest or physical protected-lock behavior.

## Release separation and cleanup

Check the built Release app, Release daemon and production Worker dry-run bundle:

```sh
scripts/check-simulator-release.sh \
  target/ios/DerivedData/Build/Products/Release-iphonesimulator/Collie.app \
  target/release/collied relay/dist/index.js
```

The script fails if the app's plist has a loopback ATS exception, or if any artifact contains Simulator relay or VM test switches, paths, loopback endpoints or provisioning markers. [Simulator relay isolation](architecture.md#simulator-relay-isolation) lists the compile gates that exclude them.

To clean up, stop the owned server and Debug daemon, revoke the scratch pairing, close the scratch workspace and remove only your private directory; delete the dedicated Simulator if disposable. Never delete a shared Simulator, installed daemon state or real pairings.

Record the mode and exact revision with each result. Mock success does not replace the relay's [production checks](relay.md).
