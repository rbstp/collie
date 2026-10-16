# Notification testing and navigation

## Delivery-only test

```sh
collied apns test
```

This sends a generic alert to registered devices belonging to paired phones. It has no session target, so tapping it opens the app without selecting an agent. Registered peers must support the encrypted display codec; unsupported registrations are not sent private content.

The CLI reports APNs acceptance, not confirmation that the phone displayed the alert. Check the phone as well. Debug builds register sandbox tokens; production builds use production tokens. The sender uses each device's registered environment.

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

## Observations from device testing

- The generic test reached the iPhone and opened the app, but did not open a session because its payload had no session target.
- The session-linked test reached the iPhone, and the user confirmed that it opened the intended session.
- Opening the app alone does not demonstrate that session routing worked. Test the targeted payload and check the resulting screen.

The payload helpers are in `crates/collied/src/push.rs`. iPhone action classification is in `Collie/Sources/Collie/Notifications.swift`; session routing is in `AppDelegate.swift`, `AppModel.swift`, and `ApprovalsModel.swift`.

## Encrypted display and fallback

Approval title/body/context, done alerts, PR watch wake and archive alerts are encrypted separately for each phone with its per-machine notification key. All alerts set mutable content and start with `Collie` plus `An agent needs attention` or `An agent finished`. The original has no approval category. ColliePush authenticates the machine, destination scope, event and approval before restoring display text and offered controls locally. Extension timeout, a missing key or tampering leaves useful generic text without controls. A response cannot approve or deny without an authenticated display; the actual nonce and decision still travel over the paired tailnet session.

Initial Live Activity attributes and state, foreground updates, remote updates and ends all serialize private display data as ciphertext. The widget uses its protected App Group key mirror and checks its immutable machine and random activity binding. Before the first unlock after boot, or after any decryption failure, it shows neutral state without approval controls. Activity relevance and expanded alert text remain generic. Payloads over APNs' 4 KiB or ActivityKit's bounded initial size are refused, never retried in plaintext. Existing registrations need a refreshed encrypted-codec registration.

Apple can still observe delivery destinations, opaque machine/thread/approval/event/activity identifiers, generic categories and alerts, priorities, timing, frequency, payload size and activity lifecycle. A future relay sees the same metadata and ciphertext, with no notification keys. The exact envelope and shared vectors are in [architecture.md](architecture.md#encrypted-display-envelope).

Simulator unit checks verify the codec and neutral state separately from real APNs delivery. Locked/pre-first-unlock display, notification actions, and remote activity update/end behavior require physical iPhone validation; prior navigation observations above do not validate this new encrypted codec.
