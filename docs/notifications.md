# Notification testing and navigation

## Delivery-only test

```sh
collied apns test
```

This sends a generic alert to the registered devices of every paired phone. With no session target, tapping it opens the app without selecting an agent. A registration without the encrypted display codec gets no alert: both senders refuse an alert without encrypted content.

The CLI reports that APNs or the relay accepted the push, not that the phone displayed it, so check the phone too. Debug builds register sandbox tokens and production builds production tokens. Direct delivery uses the environment each device registered; relay delivery uses the one bound into its attested grant.

## Session-linked test

Get an agent's terminal ID from `collied status`, then send:

```sh
collied apns test --terminal-id <terminal-id>
```

The daemon must be running, so the CLI can get its node ID, and the terminal ID must be well formed. The test binds `node_id`, an opaque thread ID and a fresh event ID to the encrypted display. After decryption, the phone restores the private terminal ID, maps the node ID to a paired machine, validates the session link and opens that agent's session on the Agents tab.

With no approval ID, approval actions or tool command, the test approves or denies nothing. It exercises the normal session-notification path, not a permission decision.

## Approval notifications

Approval alerts carry a node ID and an approval ID. Their public thread ID is an opaque hash of the terminal ID; the authenticated display restores the private terminal ID locally. Tapping an alert first resolves the pending approval to its machine and terminal; if the approval has already ended, a valid thread can still open the original session.

While loading, the app shows the connection phase, and a missing or unavailable machine produces a visible error. Connecting lookups retry for 15 seconds; an in-flight request keeps its own RPC timeout. Connection failures are not reported as expired approvals, and a valid thread can still open the session to show its connection state.

Selecting another tab or session, or going Back to the Agents list, cancels a pending lookup, so a late notification cannot replace newer navigation. The initial empty navigation path does not cancel cold-start notification loading.

The iPhone has no Approvals tab: in-session controls, notification actions, Live Activities and the Apple Watch approval list decide approvals, with their usual authentication and approval checks. Manual terminal input is separate.

Attention previews prefer the pending question or action. For a Codex structured question, the preview is the current question text between the numbered question heading and its choices, with the submit footer confirming the dialog is open; notes and earlier conversation are excluded. On a Codex conversation screen, it skips the user input marked `›` and uses the assistant response marked `•` after it. An incomplete or ambiguous question falls back to `Blocked in <workspace>`. Completion alerts stay `Done in <workspace>`, with no reply excerpt. Both senders carry the same encrypted preview. iPhone and Apple Watch show the authenticated text, as does an iPhone notification mirrored to macOS; CollieBar and CollieTray send no desktop notifications.

## Observations from device testing

- The generic test reached the iPhone and opened the app, but no session, because its payload had no session target.
- The session-linked test reached the iPhone, and the user confirmed that it opened the intended session.
- Opening the app alone does not show that session routing worked: test the targeted payload and check the resulting screen.

The payload helpers are in `crates/collied/src/push.rs`. iPhone action classification is in `Collie/Sources/Collie/Notifications.swift`; session routing is in `AppDelegate.swift`, `AppModel.swift` and `ApprovalsModel.swift`.

## Encrypted display and fallback

Every alert, including done, PR watch wake and archive alerts, and every Live Activity state carries its private text as ciphertext under the phone's per-machine notification key. In clear, an alert reads only `Collie` with `An agent needs attention` or `An agent finished`, with no approval category. ColliePush and the widget restore text and approval controls only from an authenticated envelope. An extension timeout, a missing key, tampering or a phone not yet unlocked since boot leaves generic or neutral content without controls, and activity state too large to fit falls back to the encrypted approval notification, never plaintext. A response cannot approve or deny without an authenticated display, and the nonce and decision still travel over the paired tailnet session. [Encrypted display envelope](architecture.md#encrypted-display-envelope) has the format, size limits and shared vectors; the [threat model](threat-model.md#apple-sees-push-payload-metadata) lists what Apple and the relay still observe.

Simulator unit checks verify the codec and neutral state, not real APNs delivery. Remote Live Activity update and end through the relay are confirmed on a physical iPhone. Locked and pre-first-unlock display, and notification actions, still need physical iPhone validation; the navigation observations above do not validate this codec.

One-shot background approval and approval-list sessions make the same ownership, StableID, TLS key and hello checks as a reconnect ([tailnet.md](tailnet.md)). An expired sign-in returns unauthorized and asks the user to open Collie; a notification never starts browser sign-in or needs an OS VPN toggle.

## Relay delivery

`[push.relay]` selects the publisher relay and `[apns]` direct APNs; the config refuses both. The app enrolls with App Attest once a paired daemon reports relay mode, and uses its grants only in that mode. [Relay sender](architecture.md#relay-sender) covers the daemon's coalescing, retries, expiry, quotas and diagnostics. [relay.md](relay.md) covers the deployed relay, its [grants](relay.md#attested-control-contract) and its [retention and cleanup](relay.md#retention-and-cleanup).

## Simulator testing

[Simulator relay testing](simulator.md) has separate commands for real paired sessions, local mock APNs, payload injection and optional real sandbox delivery. Mock acceptance and `simctl push` do not show that the notification service extension ran, that remote ActivityKit delivery works or that a physical device accepts the push. A fresh, explicitly provisioned DEBUG Simulator identity with an isolated local database cannot enroll with production.
