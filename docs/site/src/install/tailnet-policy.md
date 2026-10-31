# Tailscale sign-in and policy

Sign in with the same personal Tailscale account inside Collie on the iPhone and through `collied setup` on the computer. collie runs its own node on each device: you need no Tailscale app or VPN profile, and a corporate Tailscale VPN on your Mac can stay connected.

## When the policy needs a grant

If your own devices already reach each other, you need no tags or policy edit; Collie never edits the policy. If a restrictive policy blocks TCP port 8457 or hides the machine from the phone, ask the administrator to merge this grant into it:

```hujson
{
  "grants": [
    { "src": ["autogroup:member"], "dst": ["autogroup:self"], "ip": ["tcp:8457"] },
  ],
}
```

Your computer's node gets the same network access and key expiry as your other devices. [Tailnet setup](../reference/tailnet.md) covers expiry and revocation.

> [!NOTE]
> `collied` and the app refuse the tagged node that desktop v0.1.0 created. Update both, then reset the node and pair again: see [Existing tagged installations](../reference/tailnet.md#existing-tagged-installations).
