# Notification testing and navigation

## Delivery-only test

```sh
collied apns test
```

This sends a generic alert to every registered device of each paired phone; tapping it opens only the app. A registration without the encrypted display codec gets none: both senders refuse unencrypted alerts. The CLI reports acceptance by APNs or the relay, not display on the phone.

Debug builds register sandbox tokens and production builds production ones; the relay uses the environment bound into each attested grant.

## Session-linked test

```sh
collied apns test --terminal-id <terminal-id>
```

Take the terminal ID from `collied status`; the daemon must be running, since the CLI asks it for the node ID. After decrypting the display, the phone restores the terminal ID, maps the node ID to a paired machine, validates the session link and opens that agent's session on the Agents tab. The test carries no approval, so it checks navigation only.

## Approval notifications

A tap on an approval alert first resolves its node and approval IDs to a machine and terminal. If the approval has ended, a valid thread still opens its session.

While resolving, the app shows the connection phase. A missing or unavailable machine shows an error, never an expired approval; a valid thread can still open the session to show its connection state. Connecting lookups retry for 15 seconds; an in-flight request keeps its own RPC timeout. Selecting another tab or session, or going Back to the Agents list, cancels the lookup, so a late notification cannot replace newer navigation; the initial empty navigation path does not cancel a cold-start lookup.

Attention previews prefer the pending question or action:

- Codex structured question: the question text between the numbered heading and its choices, while the submit footer shows the dialog is open, without notes or earlier conversation.
- Codex conversation screen: the assistant response marked `•` after the user input marked `›`, never that input.
- Incomplete or ambiguous question: `Blocked in <workspace>`.
- Completion: `Done in <workspace>`, with no reply excerpt.

iPhone and Apple Watch show the authenticated preview, as does an iPhone notification mirrored to macOS; CollieBar and CollieTray send no desktop notifications.

## Done alert at the desk

collied skips a done alert (#774) only when three signals, read when the turn ends, are all known and true; any missing, stale or failed signal sends it. Approval alerts, pull request watch and archive alerts, Live Activities and Usage are unchanged. Design and calls: [architecture](architecture.md#apns).

- The pane is focused: the agent is herdr's, and herdr's `session.snapshot` names its pane as `focused_pane_id`.
- Its window is in front: the frontmost normal window's application, or a process under it, holds a client attached to the session collied follows.
- The user is active: keyboard or pointer input in the last 60 s, the screen unlocked, the display awake and the session on the console.

It applies on macOS with herdr only. Herdr Core and Rex report no client's view, and on Linux idle and lock states would need a Wayland client and D-Bus, so those send as before. collied sees the application, not its tab or window: with herdr's terminal in front showing another tab, the alert is skipped.

To try it on a Mac: run an agent in herdr in a terminal, give it a task of more than 30 s, and keep its pane focused and the terminal in front while moving the mouse now and then. No alert arrives (with `RUST_LOG=info`, collied logs "done alert skipped"). Repeat with another app in front, with the screen locked, or after a minute without input: the alert arrives.

## Observations from device testing

- The generic test reached the iPhone and opened the app, but no session, because its payload had no session target.
- The session-linked test reached the iPhone, and the user confirmed that it opened the intended session.
- Opening the app alone does not show that session routing worked: test the targeted payload and check the resulting screen.

Payload helpers: `crates/collied/src/push.rs`. iPhone action classification: `Collie/Sources/Collie/Notifications.swift`. Session routing: `AppDelegate.swift`, `AppModel.swift` and `ApprovalsModel.swift`.

## Encrypted display and fallback

Every alert and Live Activity state carries its private text encrypted with the phone's per-machine notification key. In clear, an alert reads only `Collie` with `An agent needs attention` or `An agent finished`, with no approval category. Without an authenticated envelope (an extension timeout, a missing key, tampering, or a phone not unlocked since boot), ColliePush and the widget show generic or neutral content without controls. Activity state too large to fit falls back to the encrypted approval notification, never plaintext. [Encrypted display envelope](architecture.md#encrypted-display-envelope) has the format, limits and shared vectors; the [threat model](threat-model.md#apple-sees-push-payload-metadata) lists what Apple and the relay still observe.

Simulator unit checks verify the codec and neutral state, not real APNs delivery. On a physical iPhone, remote Live Activity update and end through the relay are confirmed; locked and pre-first-unlock display, and notification actions, still need validation. The navigation observations above do not validate this codec.

One-shot background approval and approval-list sessions make a reconnect's ownership and identity checks ([tailnet.md](tailnet.md)). After a sign-in expires, they fail as unauthorized and the app asks the user to open Collie; a notification never starts a browser sign-in.

## Relay delivery

Once a paired daemon reports relay mode (`[push.relay]`), the app enrolls with App Attest; it uses its grants only in that mode. [Relay sender](architecture.md#relay-sender) covers the daemon's coalescing, retries, expiry, quotas and diagnostics, and [relay.md](relay.md) the deployed relay, its [grants](relay.md#attested-control-contract) and its [retention](relay.md#retention-and-cleanup).

## Simulator testing

[Simulator relay testing](simulator.md) covers real paired sessions, local mock APNs, payload injection and optional sandbox delivery. Mock acceptance and `simctl push` prove neither that the notification service extension ran, nor remote ActivityKit delivery, nor acceptance on a physical device. A provisioned DEBUG Simulator identity cannot enroll with production.
