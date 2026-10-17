# Troubleshooting

This page describes the next matching phone and desktop candidate; user-owned setup is unreleased. Published desktop v0.1.0 still uses tagged nodes. Follow the [v0.1.0 tailnet procedure](https://github.com/rbstp/collie/blob/v0.1.0/docs/tailnet.md) until both peers are updated.

Start with `collied doctor` on the machine. Each line is `ok`, `warn` or `fail`, with the name of the check. Then find your symptom below.

```sh
collied doctor
```

## The machine shows offline

The phone could not reach the machine over the tailnet.

1. Check that the machine is awake and `collied` runs: the `daemon` line in `collied doctor`, or `collied status`.
2. If you ran `collied stop`, run `collied start`.
3. A `warn` on the `tailnet` line saying "no node state (not logged in)", or a `fail` on the `daemon` line whose node is not `Running`, means the node is logged out: run `collied login`. A `fail` on the `tag` line means the node lost its tag; `collied login` prints what to fix. A `fail` on the `tailnet` line is a problem with the node state folder itself, such as its permissions.
4. A `warn` on the `reach` line means no device of yours can reach the node: check the [tailnet policy](install/tailnet-policy.md) and that the phone is signed in to Tailscale in the app.
5. Pull to refresh on the Agents tab to retry at once.

Connection errors are shown in full in the Machines tab, with a button to copy them.

## "Update collied" or "Update the app"

The app and `collied` speak different protocol versions. Update the side named in the message: see [Version mismatch](operate/version-mismatch.md).

## No push notifications

1. Look at the `apns` line in `collied doctor`. If it is not configured, follow [Push notifications](install/push.md). On macOS, a key the daemon cannot read usually means `collied` was rebuilt without `just collied-install`: reinstall it.
2. Run `collied apns test`. It reports whether Apple accepted the push; then check the phone.
3. Check that notifications are allowed for Collie in the iPhone's Settings.
4. Done alerts only come for turns of 30 seconds or more, never for an agent that is on screen, and not at all when **Settings > Notifications > Notify when an agent finishes** is off.

## Chat unavailable

- **The agent has no session ID**: Chat says so and retries when one arrives. For Codex, install herdr's hook with `herdr integration install codex`, and again after each herdr update. The `codex hook` line in `collied doctor` warns when it is missing or older.
- **The session was started before the hook**: for Codex, finish the turn and relaunch it in the same pane with `codex --no-daemon resume <session-id>`.
- **`collied` is older than the app**: update `collied` to enable Chat.
- **The transcript is missing**: Chat shows the history as unavailable, with a button to open Terminal.

## Codex status or done alerts missing

herdr reads Codex's status from its screen. If a Codex agent stays `unknown`, or finished turns send no done alert:

1. Make sure herdr is the version `collied` is tested against (see [Requirements](install/requirements.md)) and install its Codex hook again: `herdr integration install codex`.
2. Check the agent's status in `collied status` while it works. A turn that ends in `unknown` sends no done alert.
3. Codex tasks started from collie use `--no-daemon`. Relaunch older shared-daemon sessions as described in [Codex](agents/codex.md#why---no-daemon).

## Pairing refused

- **The window closed**: a pairing window lasts 120 seconds and the first attempt closes it, right or wrong. Run `collied pair` again.
- **No answer on the machine**: without a `y` within 60 seconds, the answer is no.
- **Another user**: `collied` only accepts the owner (its verified embedded Tailscale user, also checked against configured and stored owner pins). Sign in to Tailscale in the app with the owner's account.
- **A tagged or shared-in phone**: the phone's node must be untagged and belong to your tailnet. Do not sign the app in with a tagged auth key.
- **The policy blocks the port**: check the `reach` line and the [tailnet policy](install/tailnet-policy.md).

## Still stuck

Open an issue: see [Support](support.md).
