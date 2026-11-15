# Troubleshooting

Start with `collied doctor` on the machine, then find your symptom below.

```sh
collied doctor
```

## The machine shows offline

The app says why it cannot reach the machine, and often what to do. To retry at once, pull to refresh on the Agents tab; to copy the full error, use the Machines tab.

Otherwise, read `collied doctor` on the machine:

- **`daemon` warns "not running"**: run `collied start`.
- **`tailnet` warns "no node state (not logged in)", or `daemon` fails with a node not `Running`**: the node is signed out. Run `collied setup` to sign in again.
- **`ownership` fails**: Collie's node is tagged. See [Existing tagged installations](reference/tailnet.md#existing-tagged-installations).
- **`tailnet` fails**: the node state folder is wrong, for example its permissions.
- **`reach` warns**: `collied` sees no other device of yours. Sign the phone in to Tailscale in the app with the same account, then check the [tailnet policy](install/tailnet-policy.md).

## "Update collied" or "Update the app"

The app and `collied` share no protocol revision. Update the side the message names: see [Version mismatch](operate/version-mismatch.md).

## No push notifications

1. Read the `push` line of `collied doctor`, or its `apns` lines with direct APNs. If push is not set up, follow [Push notifications](install/push.md). On macOS, if `collied` cannot read the APNs key, reinstall it with `just collied-install`.
2. Run `collied apns test`, then check the phone.
3. In the iPhone's Settings, allow notifications for Collie.
4. For done alerts, turn on **Settings > Notifications > Notify when an agent finishes**, and see [when they come](notifications/approvals.md#done-alerts).

## Chat unavailable

- **No session ID**: Chat retries when one arrives. For Codex, install herdr's hook with `herdr integration install codex`, and in Herdr Core or Rex [`collied`'s own](agents/codex.md#herdr-core-and-rex).
- **A Codex session started before the hook**: finish the turn, then relaunch it in the same pane with `codex resume <session-id> --no-daemon`.
- **`collied` is older than the app**: update `collied`.

## Codex status or done alerts missing

If a Codex agent stays `unknown`, or a finished turn sends no done alert:

1. Use the herdr version in [Requirements](install/requirements.md), and reinstall its Codex hook with `herdr integration install codex`.
2. Check the agent's status in `collied status` while it works. A turn that ends in `unknown` sends no done alert.
3. Relaunch sessions that run on a shared Codex daemon: see [Codex](agents/codex.md#why---no-daemon).

## Rex agents missing

1. Start Rex.app: `collied` never starts Rex, and finds a server that starts later within 30 seconds.
2. Read the `rex` lines of `collied doctor`. A warning names what keeps `collied` from following the server, such as an API revision it does not speak, which a restart of the Rex server after an update fixes ([Reading `collied doctor`](operate/commands.md#reading-collied-doctor)).
3. Look for the agent under its Rex session's name in the Agents tab. Claude Code, Codex and Copilot CLI appear, as does any program that reports a status; shells do not.

## Pairing refused

- **The window closed**: it lasts 120 seconds and closes at the first attempt, right or wrong. Run `collied pair` again.
- **Nobody confirmed**: confirm on the machine within 60 seconds.
- **Another user**: `collied` accepts only its owner, the Tailscale account it is signed in with, which must also match `owner_user_id`, if set, and the owner of earlier pairings. Sign the app in to Tailscale with that account.
- **A tagged or shared-in phone**: the phone's node must be untagged and in your own tailnet. Do not sign the app in with a tagged auth key.
- **The policy blocks the port**: check the `reach` line and the [tailnet policy](install/tailnet-policy.md).

## Still stuck

Open an issue: see [Support](support.md).
