# Notification testing and navigation

## Delivery-only test

```sh
collied apns test
```

This sends "Test notification from collied" to registered devices belonging to paired phones. It has no machine or session identifiers. Tapping it opens the app, but does not select an agent session.

The CLI reports APNs acceptance, not confirmation that the phone displayed the alert. Check the phone as well. Debug builds register sandbox tokens; production builds use production tokens. The sender uses each device's registered environment.

## Session-linked test

Get an agent's terminal ID from `collied status`, then send:

```sh
collied apns test --terminal-id <terminal-id>
```

The daemon must be running so the CLI can get its node ID. The terminal ID is validated before sending.

This adds `node_id`, `terminal_id`, and the terminal's `aps.thread-id` to the test alert. Tapping it opens that agent's session on the Agents tab. The phone maps the node ID to a paired machine and validates the session link.

This is a harmless navigation test. It has no approval ID, approval actions, or tool command, and does not approve or deny anything. It tests the normal session-notification path, not a real permission decision.

## Approval notifications

Actual approval alerts carry a node ID and approval ID. Their notification thread identifies the terminal. Tapping an alert first resolves the pending approval to its machine and terminal. If the approval has already ended, a valid thread can still open the original session.

Loading and missing-machine errors are shown in the app. Selecting another tab or session cancels a pending lookup so a delayed notification cannot replace the user's newer navigation.

The iPhone has no separate Approvals tab. In-session approval controls, notification actions, Live Activities, and the Apple Watch approval list remain available. Structured approval actions still require their existing authentication and approval checks; manual terminal input is separate.

## Observations from device testing

- The generic test reached the iPhone and opened the app, but did not open a session because its payload had no session target.
- The session-linked test reached the iPhone, and the user confirmed that it opened the intended session.
- Opening the app alone does not demonstrate that session routing worked. Test the targeted payload and check the resulting screen.

The payload helpers are in `crates/collied/src/push.rs`. iPhone action classification is in `Collie/Sources/Collie/Notifications.swift`; session routing is in `AppDelegate.swift`, `AppModel.swift`, and `ApprovalsModel.swift`.
