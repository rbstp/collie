# Troubleshooting

Start with `collied doctor` on the machine. Each line is `ok`, `warn` or `fail`, with the name of the check. Then find your symptom below.

```sh
collied doctor
```

## The machine shows offline

The machine's section on the Agents tab and its row on the Machines tab say why the phone cannot reach it:

- **"This phone is not connected to Tailscale."** The phone's own node is not running. Sign in to Tailscale again in the app, or check the phone's network.
- **"\<machine> is asleep or off. Last seen ..."** Tailscale reports the machine offline. Wake it or turn it on; the phone connects as soon as Tailscale sees it again.
- **"\<machine> is online, but collied is not answering."** The machine is on the tailnet, but `collied` refused the connection or did not answer. Run `collied start` on the machine.
- **"Cannot reach \<machine>."** Another network error. Copy the full error with the button on the Machines tab, then work through the checks below.

To retry at once, pull to refresh on the Agents tab.

When the message does not settle it, run `collied doctor` on the machine:

1. Check that the machine is awake and `collied` runs: see the `daemon` line, or `collied status`.
2. If the node is logged out, run `collied login`. `collied doctor` shows this as a `warn` on the `tailnet` line saying "no node state (not logged in)", or a `fail` on the `daemon` line whose node is not `Running`.
3. A `fail` on the `ownership` line means Collie's node is tagged: see [Existing tagged installations](reference/tailnet.md#existing-tagged-installations). `collied login` prints what to fix.
4. A `fail` on the `tailnet` line means a problem with the node state folder itself, such as its permissions.
5. A `warn` on the `reach` line means `collied` sees no other device of yours. Check that the phone is signed in to Tailscale in the app with the same account, then check the [tailnet policy](install/tailnet-policy.md).

## "Update collied" or "Update the app"

The app and `collied` share no protocol version. Update the side the message names: see [Version mismatch](operate/version-mismatch.md).

## No push notifications

1. Look at the `push` line in `collied doctor`, or the `apns` lines with direct APNs. If push is not configured, follow [Push notifications](install/push.md). On macOS, when `collied` cannot read the APNs key, it was usually rebuilt without `just collied-install`: reinstall it.
2. Run `collied apns test`. It reports whether the push was accepted; then check the phone.
3. In the iPhone's Settings, check that notifications are allowed for Collie.
4. Done alerts come only for turns of 30 seconds or more, never for an agent that is on screen, and not at all when **Settings > Notifications > Notify when an agent finishes** is off.

## Chat unavailable

- **The agent has no session ID**: Chat says so and retries when one arrives. For Codex, install herdr's hook with `herdr integration install codex`, and again after each herdr update. The `codex hook` line in `collied doctor` warns when the hook is missing or older.
- **The session was started before the hook**: for Codex, finish the turn and relaunch it in the same pane with `codex --no-daemon resume <session-id>`.
- **`collied` is older than the app**: update `collied` to enable Chat.
- **The transcript is missing**: Chat shows the history as unavailable, with a button to open Terminal.

## Codex status or done alerts missing

herdr reads Codex's status from its screen. If a Codex agent stays `unknown`, or you get no done alert when a turn finishes:

1. Make sure herdr is the version `collied` is tested against (see [Requirements](install/requirements.md)), and install its Codex hook again with `herdr integration install codex`.
2. While the agent works, check its status in `collied status`. A turn that ends in `unknown` sends no done alert.
3. collie starts Codex tasks with `--no-daemon`. Relaunch older sessions that run on a shared daemon as described in [Codex](agents/codex.md#why---no-daemon).

## Pairing refused

- **The window closed**: a pairing window lasts 120 seconds, and the first attempt closes it, right or wrong. Run `collied pair` again.
- **No answer on the machine**: without a `y` within 60 seconds, `collied` refuses the pairing.
- **Another user**: `collied` accepts only its owner, the Tailscale account it is signed in with. That account must also match `owner_user_id`, if set, and the owner of earlier pairings. Sign in to Tailscale in the app with the owner's account.
- **A tagged or shared-in phone**: the phone's node must be untagged and belong to your tailnet. Do not sign the app in with a tagged auth key.
- **The policy blocks the port**: check the `reach` line and the [tailnet policy](install/tailnet-policy.md).

## Still stuck

Open an issue: see [Support](support.md).
