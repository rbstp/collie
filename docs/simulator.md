# Simulator relay testing

These modes prove different things. A Simulator never establishes genuine App Attest or Secure Enclave trust.

| Mode | Transport | Coverage |
|---|---|---|
| Normal pairing | Authenticated Tailscale and mutual TLS | Real paired sessions, approvals, chat and terminal with Simulator software identity keys |
| Local relay, mock APNs | Loopback HTTP, isolated local D1, mock Apple response | Signed control, encrypted envelopes, route generation, quotas, replay, revocation and outage handling |
| `simctl push` | Local payload injection | Rendering and routing only; not real APNs or notification service extension acceptance |
| Local relay, sandbox APNs | Loopback control/send, real Apple HTTP/2 | On supported hosts, actual sandbox alert/NSE and ActivityKit delivery, only when observed |
| Physical device/TestFlight | Genuine App Attest and APNs | Production integrity, Secure Enclave, background/locked/pre-first-unlock behavior and production delivery |

## Build and pair normally

Use a dedicated Simulator, a daemon you own and a scratch herdr workspace. Never send input into an existing agent. Keep the normal [tailnet policy](tailnet.md) and local pairing confirmation.

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

Sign into Tailscale in the Simulator app with the daemon owner's account. In **Machines > Add machine**, paste the printed `collie://pair#...` link instead of scanning it. At the daemon's prompt, check the user and Simulator identity, then approve that one pairing. The link is a one-use secret: do not commit or share it. Before testing notifications, open a scratch workspace's agent and verify a real session. Ordinary pairing needs no relay enrollment.

## Local mock relay

The harness provisions a fresh local P-256 identity and never imports a real installation key or reads the user's Keychain. Its private directory, which you keep outside the repository, holds the new software key, a separate send secret, an isolated D1 and the last encrypted mock payload.

```sh
cd relay
npm ci
npm run check
npm test
node dev/simulator.ts init /private/tmp/collie-simulator-relay
node dev/simulator.ts serve /private/tmp/collie-simulator-relay
```

The server listens only on `127.0.0.1:8788` and refuses enrollment and every production APNs environment. Offline provisioning puts the local public key into the isolated D1 only. Everything after it (signed assertions, control transactions, quotas, sends) runs the unmodified production validators, with a real Miniflare SQLite `RelayCrypto` actor verifying assertions and signing APNs provider tokens. The separate crypto entry answers every HTTP request with 404, and missing crypto fails closed with no inline verifier or signing fallback. The send secret cannot authorize production routes. The server needs no Cloudflare resources or Apple credentials and deploys nothing.

In a second terminal, from the repository root, copy only the new development identity into the dedicated DEBUG Simulator app:

```sh
simulator='iPhone 18 Pro Max'
simulator_id=$(xcrun simctl list devices available -j | jq -r --arg name "$simulator" '[.devices[][] | select(.name == $name) | .udid][0]')
container=$(xcrun simctl get_app_container "$simulator_id" dev.rbstp.collie data)
mkdir -p "$container/Library/Application Support/collie"
chmod 700 "$container/Library/Application Support/collie"
install -m 600 /private/tmp/collie-simulator-relay/simulator-relay.json "$container/Library/Application Support/collie/simulator-relay.json"
SIMCTL_CHILD_COLLIE_SIMULATOR_RELAY=mock xcrun simctl launch --terminate-running-process "$simulator_id" dev.rbstp.collie
```

`mock` gives the app a synthetic destination so hosts without APNs support can exercise registration; it simulates no real Apple token, notification extension or remote ActivityKit token. A real APNs registration, if available, can replace it.

The daemon must be a source DEBUG build with `[push.relay]` configured. Stop its existing process, without replacing an installed binary or service, and run:

```sh
cargo build -p collied
COLLIE_SIMULATOR_RELAY=1 target/debug/collied run
```

This DEBUG-only switch selects exactly `http://127.0.0.1:8788/v1/send`; no option accepts another endpoint, and a Release daemon always uses the HTTPS publisher endpoint.

Pair this Simulator as above and open the app so it delivers app-token grants over the paired mutual-TLS session. Then run `COLLIE_SIMULATOR_RELAY=1 target/debug/collied apns test` with the same configuration: the command builds its own sender, so it needs the switch too. In mock mode, `accepted` means only that the mock accepted the encrypted request, not that Apple accepted it or an alert displayed. The mock keeps only the last payload, in `/private/tmp/collie-simulator-relay/last-push.apns`. To inspect local rendering:

```sh
xcrun simctl push "$simulator_id" dev.rbstp.collie /private/tmp/collie-simulator-relay/last-push.apns
```

Injection does not show that APNs invoked ColliePush or that private text decrypted, and the generic fallback must stay useful when decryption or the extension is unavailable.

To test an outage, restart the local server with the same directory and `COLLIE_SIMULATOR_APNS_OUTAGE=1`, send again and inspect `target/debug/collied doctor`; then restart without the flag and verify that bounded retries recover. The database and assertion counters survive a server restart, but restarting the daemon discards its in-memory queue. Removing the scratch pairing in the app must revoke its grants. The automated local tests also cover stale proofs, replay after a fresh counter, quota exhaustion and production rejection.

## Real sandbox APNs

Apple silicon or T2 Macs on macOS 13 or later with an iOS 16 or later Simulator can register with APNs Sandbox, though a given host, runtime or network may still fail. See [Xcode's Simulator APNs support](https://developer.apple.com/documentation/xcode-release-notes/xcode-14-release-notes) and [ActivityKit push testing](https://developer.apple.com/documentation/activitykit/starting-and-updating-live-activities-with-activitykit-push-notifications).

Use a locally held APNs signing key authorized for the app's Apple team and topic. Never distribute it, copy it from another user's Keychain, add it to the development identity file or commit it. Stop the mock server, since both modes use the same loopback port, then create a fresh private directory with the matching team ID:

```sh
cd relay
node dev/simulator.ts init /private/tmp/collie-simulator-sandbox YOURTEAMID
COLLIE_SIMULATOR_APNS_KEY_FILE=/absolute/private/path/AuthKey_KEYID.p8 \
COLLIE_SIMULATOR_APNS_KEY_ID=YOURKEYID1 \
node dev/simulator.ts serve /private/tmp/collie-simulator-sandbox
```

Replace both placeholders with the real ten-character team and key IDs. The sender uses HTTP/2 and refuses any host but `api.sandbox.push.apple.com`, so it cannot reach production APNs.

Copy this directory's identity into a fresh dedicated Simulator as above and launch with `SIMCTL_CHILD_COLLIE_SIMULATOR_RELAY=sandbox`. This mode has no synthetic token: the app must register with real APNs, from a locally signed DEBUG build with the matching sandbox entitlement if unsigned registration is refused. Pair normally and confirm that Settings reports a sandbox registration. If registration fails, report that gate and use mock tests; never substitute a fake token and claim APNs success.

Run the owned Debug daemon with the same isolated configuration and data directory. Send one generic `COLLIE_SIMULATOR_RELAY=1 target/debug/collied apns test` and record Apple's response separately from display. The generic test uses an unknown node ID, so it cannot prove that ColliePush loaded a paired machine's notification key and decrypted its text. For that, take the terminal ID from your scratch workspace's creation response, background the app and send a session-linked alert:

```sh
COLLIE_SIMULATOR_RELAY=1 target/debug/collied apns test --terminal-id "$terminal_id"
```

Require both Sandbox acceptance and **Tap to open the selected agent session.** in the system notification: that text comes from the encrypted display envelope, and the generic fallback alone is not decryption evidence. Check the banner's or Notification Center's accessibility hierarchy and a screenshot separately. SwiftUI settings and system notifications may combine labels, so an exact text-element query can miss a successful registration or delivery, and a black screenshot does not prove rendering even when the accessibility text is present. Bound sends before starting and do not resend automatically after an assertion failure.

Test approval, done and agent alerts and the generic missing-key fallback separately. For a Live Activity, follow a scratch agent, wait for its real ActivityKit token from `pushTokenUpdates`, and background the app while still following. Record the decoded initial status and the owned workspace name, change the owned session to a different non-fallback status, and require the new decoded status and workspace in the system activity while the app stays backgrounded. `unknown` alone is not proof: it is also the decryption-failure fallback. For a remote end, close only that scratch terminal or workspace with the app backgrounded, and require the daemon's `event=end` send, Apple acceptance and an observed ended activity. Unfollowing ends the activity locally and revokes the route, so it does not prove remote end delivery. The activity destination differs from the app alert token: require topic `dev.rbstp.collie.push-type.liveactivity`, push type `liveactivity`, Apple acceptance, and observed update and end states. The mock's encrypted fixture tests do not prove these remote deliveries. [notifications.md](notifications.md) covers decisions, expiry and background clearing.

Validation on 2026-10-17 used revision `13929290177e309445dae633a6f1f85eff4564ff`, a Debug iOS 27.2 Simulator app and an isolated macOS 27 VM. Apple accepted two real Sandbox alert requests, and the session-linked alert showed its decrypted text in both system accessibility and a visible screenshot. The private test runner later failed a disappearing-banner assertion, so the runner as a whole did not pass. ActivityKit registered a genuine activity token and Apple accepted two update requests. A decoded initial `idle` state was visible while the app was suspended, but the distinct `working` state did not appear within the bounded wait, and remote end was not reached. The initial state can come from the local activity request, so remote ActivityKit update and end remain unverified. Routine priority `5` pushes may be delayed or batched, so a bounded-wait failure alone does not establish an app bug ([Apple's APNs delivery behavior](https://developer.apple.com/documentation/usernotifications/sending-notification-requests-to-apns)). These results do not establish genuine App Attest or physical protected-lock behavior.

## Release separation and cleanup

After building the Release app, Release daemon and production Worker dry-run bundle, check all three:

```sh
scripts/check-simulator-release.sh \
  target/ios/DerivedData/Build/Products/Release-iphonesimulator/Collie.app \
  target/release/collied relay/dist/index.js
```

The script rejects a loopback ATS exception in the built plist, and Simulator relay and VM test switches, paths, loopback endpoints and development provisioning markers in all three artifacts. The Swift code compiles only under `DEBUG && targetEnvironment(simulator)`, the loopback ATS exception goes only into DEBUG Simulator builds, and the daemon switch and endpoint compile only under `cfg(debug_assertions)`. The production Worker entry does not import `relay/dev`. The automated production-entry test uses its own unseeded D1 and independent HMAC secret, and rejects both forged enrollment and local send grants without forwarding to Apple.

To clean up, stop the owned local server and debug daemon, revoke the scratch pairing, close the scratch herdr workspace and remove only the private test directory you created. Delete the dedicated Simulator if its app state is disposable. Never delete a shared Simulator, installed daemon state or real pairings.

Record the tested mode and exact revision with each result. Local mock success does not replace the relay's [production checks](relay.md) or validate physical locked or background behavior; real sandbox APNs and physical-device or TestFlight observations remain separate acceptance gates until performed.
