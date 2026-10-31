# Tailscale sign-in and policy

Sign in with the same personal Tailscale account inside Collie on the iPhone and through `collied setup` on the computer. Each device runs its own Tailscale node inside collie, so you need no Tailscale app, VPN profile or VPN switch in Settings. A corporate Tailscale VPN on your Mac can stay connected: Collie does not switch its account or change its routes.

## When the policy needs a grant

If your tailnet already lets your own devices connect to each other, you need no tags and no policy edit. Collie never changes the policy. A restrictive policy can still block TCP port 8457 or hide the machine from the phone. Then ask the administrator to allow that port between your own untagged devices, merging this grant into the existing policy rather than replacing it:

```hujson
{
  "grants": [
    { "src": ["autogroup:member"], "dst": ["autogroup:self"], "ip": ["tcp:8457"] },
  ],
}
```

Review the complete policy in Access controls and Preview rules; broader grants you already have still apply. Your computer's node gets the same network access and key expiry as your other devices.

## Coming from a tagged setup

`collied` refuses a Collie node tagged by desktop v0.1.0, and the app cannot reconnect with a pairing made with it. Update the app and `collied`, then reset the node and pair again as [Existing tagged installations](../reference/tailnet.md#existing-tagged-installations) describes. Leave the system Tailscale client's state and your corporate VPN alone. [Tailnet setup](../reference/tailnet.md) also covers state paths, authorization, expiry and revocation.
