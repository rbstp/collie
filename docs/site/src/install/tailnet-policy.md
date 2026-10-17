# Tailscale sign-in and policy

This page describes the next matching phone and desktop candidate; user-owned setup is unreleased. Published desktop v0.1.0 still uses tagged nodes. Follow the [v0.1.0 tailnet procedure](https://github.com/rbstp/collie/blob/v0.1.0/docs/tailnet.md) until both peers are updated.

Sign in inside Collie on the iPhone and through `collied setup` on the computer with the same personal Tailscale account. Both are independent user-owned embedded nodes. No standalone Tailscale app, OS VPN profile or VPN Settings toggle is needed. Your Mac's separate corporate Tailscale VPN can remain connected; Collie does not switch its account or change its routes.

If the current personal tailnet permits your own devices to connect, no tag ownership or policy edit is needed. Collie never changes administrator policy. A restrictive policy can still block TCP 8457 or hide the peer: ask the administrator to allow that port between the same user's untagged devices. Merge a grant into the existing policy rather than replacing it:

```hujson
{
  "grants": [
    { "src": ["autogroup:member"], "dst": ["autogroup:self"], "ip": ["tcp:8457"] },
  ],
}
```

Review the complete policy in Access controls and Preview rules. Existing broader grants still apply. User-owned desktop nodes receive their user's network privileges and normal key expiry, rather than the previous separate service-tag identity.

Existing tagged Collie nodes and old phone pairing records need a coordinated update and explicit reset/re-pair. Do not delete or change the system Tailscale client's state or your corporate VPN. See [Tailnet setup](../reference/tailnet.md) for exact state paths, authorization, expiry and revocation.

Actual physical-device operation alongside an active corporate VPN remains a required acceptance check before App Store submission. Local testcontrol and Simulator tests do not establish that result.
