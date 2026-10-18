# Tailscale sign-in and policy

Sign in with the same personal Tailscale account in two places: inside Collie on the iPhone, and through `collied setup` on the computer. Each device runs its own Tailscale node inside collie, so you need no standalone Tailscale app, VPN profile or VPN switch in Settings. A separate corporate Tailscale VPN on your Mac can stay connected: Collie does not switch its account or change its routes.

## When the policy needs a grant

If your personal tailnet already lets your own devices connect to each other, you need no tags and no policy edit. Collie never changes the tailnet policy. A restrictive policy can still block TCP port 8457 or hide the machine from the phone. Then ask the administrator to allow that port between your own untagged devices, merging this grant into the existing policy rather than replacing it:

```hujson
{
  "grants": [
    { "src": ["autogroup:member"], "dst": ["autogroup:self"], "ip": ["tcp:8457"] },
  ],
}
```

Review the complete policy in Access controls and Preview rules. Broader grants you already have still apply. Your computer's node gets the same network access and normal key expiry as your other devices, not the separate tagged identity that earlier versions used.

## Coming from a tagged setup

The current `collied` refuses an existing tagged Collie node, and the app cannot reconnect with a pairing made before the change. Update both the app and `collied`, then reset the node and pair again as described in [Existing tagged installations](../reference/tailnet.md#existing-tagged-installations). Do not delete or change the system Tailscale client's state or your corporate VPN. [Tailnet setup](../reference/tailnet.md) also covers the exact state paths, authorization, expiry and revocation.
