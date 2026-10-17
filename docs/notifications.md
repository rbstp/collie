# Notification testing and navigation

## Delivery-only test

```sh
collied apns test
```

This sends a generic alert to registered devices belonging to paired phones. It has no session target, so tapping it opens the app without selecting an agent. Registered peers must support the encrypted display codec; unsupported registrations are not sent private content.

The CLI reports APNs acceptance, not confirmation that the phone displayed the alert. Check the phone as well. Debug builds register sandbox tokens; production builds use production tokens. Direct delivery uses each device's registered environment. Relay delivery uses the environment bound into its attested grant. Both modes are selected in `collied.toml`.

## Session-linked test

Get an agent's terminal ID from `collied status`, then send:

```sh
collied apns test --terminal-id <terminal-id>
```

The daemon must be running so the CLI can get its node ID. The terminal ID is validated before sending.

This binds `node_id`, an opaque thread ID and a fresh event ID to the encrypted test display. Its private terminal ID is restored locally after decryption. Tapping it opens that agent's session on the Agents tab. The phone maps the node ID to a paired machine and validates the session link.

This is a harmless navigation test. It has no approval ID, approval actions, or tool command, and does not approve or deny anything. It tests the normal session-notification path, not a real permission decision.

## Approval notifications

Actual approval alerts carry a node ID and approval ID. Their public notification thread is an opaque hash of the terminal ID; the authenticated display restores the private terminal ID locally. Tapping an alert first resolves the pending approval to its machine and terminal. If the approval has already ended, a valid thread can still open the original session.

Loading shows the connection phase, and missing or unavailable machines produce a visible error. Connecting lookups have a 15-second retry window; an in-flight request still has its own RPC timeout. Connection failures are not reported as expired approvals, and a valid notification thread can still open the session to show its connection state.

Selecting another tab or session, or pressing Back to return to the Agents list, cancels a pending lookup so a delayed notification cannot replace the user's newer navigation. The initial empty navigation path does not cancel cold-start notification loading.

The iPhone has no separate Approvals tab. In-session approval controls, notification actions, Live Activities, and the Apple Watch approval list remain available. Structured approval actions still require their existing authentication and approval checks; manual terminal input is separate.

Attention previews prefer the pending question or action. Codex structured questions use the current question text between the numbered question heading and its choices, with the submit footer confirming the dialog is open. Notes and earlier conversation text are excluded. Codex conversation screens exclude the user input marked with `›` and use the following assistant response marked with `•`; an incomplete or ambiguous question falls back to `Blocked in <workspace>`. Completion alerts remain `Done in <workspace>` without a reply excerpt. Direct APNs and relay delivery carry the same encrypted preview. iPhone and Apple Watch display the authenticated notification text; CollieBar and CollieTray do not send desktop notifications. An iPhone notification mirrored to macOS carries the same preview.

## Observations from device testing

- The generic test reached the iPhone and opened the app, but did not open a session because its payload had no session target.
- The session-linked test reached the iPhone, and the user confirmed that it opened the intended session.
- Opening the app alone does not demonstrate that session routing worked. Test the targeted payload and check the resulting screen.

The payload helpers are in `crates/collied/src/push.rs`. iPhone action classification is in `Collie/Sources/Collie/Notifications.swift`; session routing is in `AppDelegate.swift`, `AppModel.swift`, and `ApprovalsModel.swift`.

## Encrypted display and fallback

Approval title/body/context, done alerts, PR watch wake and archive alerts are encrypted separately for each phone with its per-machine notification key. All alerts set mutable content and start with `Collie` plus `An agent needs attention` or `An agent finished`. The original has no approval category. ColliePush authenticates the machine, destination scope, event and approval before restoring display text and offered controls locally. Extension timeout, a missing key or tampering leaves useful generic text without controls. A response cannot approve or deny without an authenticated display; the actual nonce and decision still travel over the paired tailnet session.

Initial Live Activity attributes and state, foreground updates, remote updates and ends all serialize private display data as ciphertext. The widget uses its protected App Group key mirror and checks its immutable machine and random activity binding. Before the first unlock after boot, or after any decryption failure, it shows neutral state without approval controls. Activity relevance and expanded alert text remain generic. The sender reserves 1,536 bytes for static activity attributes on every update/end and caps encrypted state at 2,400 bytes. The phone checks these caps and their actual combined 4,000-byte limit on initial and local updates. Oversized state takes the encrypted notification fallback for approvals, never plaintext. Existing registrations need a refreshed encrypted-codec registration.

Apple can still observe delivery destinations, opaque machine/thread/approval/event/activity identifiers, generic categories and alerts, priorities, timing, frequency, payload size and activity lifecycle. A future relay sees the same metadata and ciphertext, with no notification keys. The exact envelope and shared vectors are in [architecture.md](architecture.md#encrypted-display-envelope).

Simulator unit checks verify the codec and neutral state separately from real APNs delivery. Locked/pre-first-unlock display, notification actions, and remote activity update/end behavior require physical iPhone validation; prior navigation observations above do not validate this new encrypted codec.

Every one-shot background approval or approval-list connection checks the saved positive Tailscale owner against the phone's current embedded user, the machine's untagged same-user netmap entry and fresh unshared whois, plus the existing StableID, independent TLS key and paired hello. Missing/zero old owner pins require explicit re-pairing. Expired sign-in returns unauthorized and asks the user to open Collie; it never starts browser sign-in or requires an OS VPN toggle from a notification.

## Relay delivery and validation

`[push.relay]` selects the publisher sender and `[apns]` selects direct APNs. They cannot be enabled together. `collied setup` offers the relay without importing credentials; use `collied setup --direct-apns` for a self-built app. `collied apns test` uses the selected mode. Relay availability still depends on the attested phone/control integration, the publisher deployment and the real-device/Paid-plan measurement gates in [relay.md](relay.md); configuring a sender alone proves none of those.

Routine relay Live Activity attempts are coalesced to at most one every 60 seconds per agent and phone, including unchanged refreshes. Urgent activity updates, ends, alerts and clears bypass that cadence. Installation quotas are shared across paired machines. During an outage, the daemon retains at most 64 pending encrypted destinations in RAM and retries transient failures with bounded backoff/jitter. Only the newest routine state survives. Non-approval work expires after two minutes; approval work expires with the still-live approval, capped at one hour. Pairing, exact token/grant generation, grant expiry, activity registration and approval state are checked again before retry. A restart discards queued work. An ambiguous response can cause duplicate delivery; this is not exactly-once delivery.

Daily routine exhaustion preserves urgent capacity, while a total quota or general outage pauses all work until the reported bounded reset/backoff. `collied doctor` reports the active mode, registration counts and earliest grant expiry, last actual send error, pending count and retry/reset times. It cannot report remaining server quota. Configuration failures keep registrations. A matching invalid-route reply removes only its failed destination generation, and an activity failure keeps the encrypted approval-alert fallback. Tailnet sessions, approvals, chat, terminals and desktop controls continue independently.

The phone's App Attest grants expire after 30 days and renew from seven days before expiry. A genuine APNs registration callback or Live Activity token event also renews that route when the token is unchanged, so an invalidated cached grant can recover. Ordinary refreshes reuse fresh grants; an inactive phone still needs a later registration event or renewal. Old in-flight invalidations cannot remove a replacement generation. An inactive phone loses relay pushes after expiry until it renews. Removing a machine offline retains remote revocation work, so previously authorized sends may continue until success or expiry. Existing physical navigation observations above do not validate encrypted relay delivery, locked/pre-first-unlock display or activity update/end. Those remain explicit physical-device gates.

The phone uses grants only for negotiated actual relay mode; public enrollment and measured Paid-plan capacity remain rollout gates. See [relay.md](relay.md#attested-control-contract) for the control limits, privacy and retention policy.

## Simulator testing

[Simulator relay testing](simulator.md) gives separate commands for real paired sessions, local mock APNs, payload injection and optional real sandbox delivery. Mock acceptance and `simctl push` do not establish notification service extension execution, remote ActivityKit delivery or physical-device acceptance. A fresh explicitly provisioned DEBUG Simulator identity and an isolated local database cannot enroll with production.
