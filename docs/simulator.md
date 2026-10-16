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

Sign into Tailscale inside the Simulator app with the daemon owner's account. In **Machines > Add machine**, paste the printed `collie://pair#...` link instead of scanning it. Check the user and Simulator identity at the daemon's confirmation prompt, then approve that one pairing. The link is a one-use secret; do not commit or share it. Open a scratch workspace's agent and verify a real session before testing notifications. An ordinary pairing does not require relay enrollment.

## Local mock relay

This provisions a fresh local P-256 identity. It never imports a real installation key or reads the user's Keychain. The private directory contains the new software key, separate send secret, isolated D1 and the last encrypted mock payload. Keep it private and outside the repository.

```sh
cd relay
npm ci
npm run check
npm test
node dev/simulator.ts init /private/tmp/collie-simulator-relay
node dev/simulator.ts serve /private/tmp/collie-simulator-relay
```

The server listens only at `127.0.0.1:8788`. It refuses enrollment and every production APNs environment. Offline provisioning inserts the local public key into this isolated D1 only. Later signed assertions, control transactions, quotas and sends use the unmodified production validators. Its send secret cannot authorize production routes. No Cloudflare resources or Apple credentials are needed. Cloud deployments, when separately authorized, use `cf`; this local server does not deploy anything.

In a second terminal, from the repository root, copy only the newly created development identity into the dedicated DEBUG Simulator app:

```sh
simulator='iPhone 18 Pro Max'
simulator_id=$(xcrun simctl list devices available -j | jq -r --arg name "$simulator" '[.devices[][] | select(.name == $name) | .udid][0]')
container=$(xcrun simctl get_app_container "$simulator_id" dev.rbstp.collie data)
mkdir -p "$container/Library/Application Support/collie"
chmod 700 "$container/Library/Application Support/collie"
install -m 600 /private/tmp/collie-simulator-relay/simulator-relay.json "$container/Library/Application Support/collie/simulator-relay.json"
SIMCTL_CHILD_COLLIE_SIMULATOR_RELAY=mock xcrun simctl launch --terminate-running-process "$simulator_id" dev.rbstp.collie
```

`mock` supplies an explicit synthetic app destination so unsupported APNs hosts can exercise registration. It does not simulate a real Apple token, notification extension or remote ActivityKit token. Actual APNs registration, if available, can replace that synthetic destination.

The daemon must also be a source DEBUG build with `[push.relay]` enabled in its configuration. Stop that daemon's existing process first without replacing an installed binary or service, then run:

```sh
cargo build -p collied
COLLIE_SIMULATOR_RELAY=1 target/debug/collied run
```

This exact DEBUG-only switch selects `http://127.0.0.1:8788/v1/send`. There is no arbitrary endpoint option. A Release daemon always uses the HTTPS publisher endpoint. DEBUG Simulator builds alone receive the exact loopback ATS exception; Release and physical-device builds do not.

Pair this Simulator as above. Open the app to deliver app-token grants over its paired mutual-TLS session. Run `target/debug/collied apns test` against that running daemon. In mock mode `accepted` means the mock accepted the encrypted request. It does not mean Apple accepted it or an alert displayed. The mock saves only the last encrypted payload to `/private/tmp/collie-simulator-relay/last-push.apns`.

To inspect local rendering separately:

```sh
xcrun simctl push "$simulator_id" dev.rbstp.collie /private/tmp/collie-simulator-relay/last-push.apns
```

This is local injection, not evidence that APNs invoked ColliePush or that private text decrypted. The generic fallback must remain useful when decryption or extension execution is unavailable.

For an outage, stop the local server, restart it with the same directory and `COLLIE_SIMULATOR_APNS_OUTAGE=1`, send again, and inspect `target/debug/collied doctor`. Restart without the outage flag and verify bounded retries recover. The database and assertion counters survive server restart. Restarting the daemon discards its in-memory queue. Removing the scratch pairing in the app must revoke its grants; the automated local tests also exercise stale proofs, replay after a fresh counter, quota exhaustion and production rejection.

## Real sandbox APNs

Supported Apple silicon or T2 Macs running macOS 13+ with iOS 16+ Simulator can register with APNs Sandbox. A particular host/runtime/network may still fail registration. See [Xcode's Simulator APNs support](https://developer.apple.com/documentation/xcode-release-notes/xcode-14-release-notes) and [ActivityKit push testing](https://developer.apple.com/documentation/activitykit/starting-and-updating-live-activities-with-activitykit-push-notifications).

Use a locally held APNs signing key authorized for the app's Apple team and topic. Never distribute it, copy it from another user's Keychain, add it to the development identity file, or commit it. Create a fresh private directory using the matching Apple team ID:

```sh
cd relay
node dev/simulator.ts init /private/tmp/collie-simulator-sandbox YOURTEAMID
COLLIE_SIMULATOR_APNS_KEY_FILE=/absolute/private/path/AuthKey_KEYID.p8 \
COLLIE_SIMULATOR_APNS_KEY_ID=YOURKEYID1 \
node dev/simulator.ts serve /private/tmp/collie-simulator-sandbox
```

Both placeholders must be replaced with the actual ten-character team/key IDs. The local sender uses HTTP/2 and refuses any host other than `api.sandbox.push.apple.com`. No production APNs request can be made by this entry. Stop the mock server first because both modes deliberately share one loopback port.

Use a fresh dedicated Simulator, copy this directory's new identity as above, and launch with `SIMCTL_CHILD_COLLIE_SIMULATOR_RELAY=sandbox`. This mode supplies no synthetic token: the app must register with real APNs. Use a locally signed DEBUG build with the matching sandbox entitlement if unsigned registration is refused. Follow the normal pairing flow and confirm Settings reports a sandbox registration. If registration fails, report that gate and use mock tests, without substituting a fake token and claiming APNs success.

Run `collied apns test`, background the app, and separately record Apple acceptance and the displayed decrypted title/body from ColliePush. Exercise approval/done/agent alerts and the generic missing-key fallback. Follow a scratch agent, wait for its real ActivityKit push token registration, then change its status and end following. Record actual update and end observations. The mock's encrypted fixture tests do not prove these remote deliveries. Follow [notifications.md](notifications.md) for decisions, expiry and background-clear behavior.

## Release separation and cleanup

After building the Release app, Release daemon and production Worker dry-run bundle, check all three artifacts:

```sh
scripts/check-simulator-release.sh \
  target/ios/DerivedData/Build/Products/Release-iphonesimulator/Collie.app \
  target/release/collied relay/dist/index.js
```

This checks the built plist and rejects Simulator relay switches, paths, loopback endpoints and development provisioning markers. The Swift implementation is compiled only with `DEBUG && targetEnvironment(simulator)`; the daemon switch and endpoint are under `cfg(debug_assertions)`. `relay/dev` is not imported by the production Worker entry. The automated production-entry test uses its own unseeded D1 and independent HMAC secret, then rejects both forged enrollment and local send grants without forwarding to Apple.

Stop the owned local server and debug daemon, revoke the scratch pairing, close the scratch herdr workspace, and remove only the private test directory you created. Delete the dedicated Simulator if its app state is disposable. Do not delete a shared Simulator, installed daemon state or real pairings as cleanup.

Local mock success does not clear the relay's Free-plan CPU gate, establish genuine App Attest, or validate physical locked/background behavior. Record the tested mode and exact revision with each result. Real sandbox APNs and physical-device/TestFlight observations remain separate acceptance gates until performed.
