# Tailnet setup

Desktop v0.1.1 and later, with the matching phone app, use user-owned nodes. An install set up with v0.1.0 that has not been reset and re-paired yet still has a tagged desktop node: the [v0.1.0 tailnet procedure](https://github.com/rbstp/collie/blob/v0.1.0/docs/tailnet.md) describes it, and [Existing tagged installations](#existing-tagged-installations) explains the reset.

Collie embeds independent Tailscale nodes through libtailscale (tsnet, userspace netstack). Sign in inside Collie on the phone and through `collied setup` on the computer with the same personal Tailscale account. Fresh nodes are user-owned and untagged. No tag ownership setup is needed.

Neither device needs the standalone Tailscale app, an OS VPN profile or the VPN toggle in Settings. Collie does not install a Network Extension, a TUN interface, system routes or a system `tailscaled`. Its private node state, keys and userspace connections are separate from a native Tailscale client. On a Mac connected to a corporate Tailscale VPN, leave that client and account alone: Collie's embedded nodes use the personal tailnet sign-in instead. A corporate network or VPN policy can still restrict the underlying internet transport. Actual corporate VPN coexistence on a physical phone and Mac remains a release acceptance check, not a claim established by local testcontrol.

The phone dials the computer on TCP 8457 inside the tailnet. collied listens only through `tailnet::Node::listen`; it never dials the phone.

## Computer node

- Run `collied setup` and use the same personal account as Collie on the phone. On macOS, interactive login without an auth key tries to open the Tailscale HTTPS sign-in page once; use the printed link or QR if it does not open. Setup retains the local pairing confirmation.
- `collied login` also accepts an untagged user auth key in `COLLIE_TS_AUTHKEY` for headless use. It removes that variable from its own environment; `collied run` ignores it. No key needs to be retained after the node state is saved.
- The default hostname is `collie-<hostname>`; `[tailnet] hostname` overrides it. The default port is 8457; `[tailnet] port` overrides it.
- macOS node state lives in `~/Library/Application Support/collie/tsnet`; Linux uses `$XDG_DATA_HOME/collie/tsnet`, else `~/.local/share/collie/tsnet`. The directories are 0700 and belong to the current OS user.
- Login and server startup require an actual positive embedded self user ID, no tags, and own-address whois matching that user and StableID without sharing. The server binds admission to this owner before exposing its control socket or tailnet listener.
- An optional `[tailnet] owner_user_id` is an extra pin. It and the owner and every paired phone recorded in `peers.json` must match the actual embedded self user, or startup fails closed. The first phone cannot choose the owner.
- The pairing QR carries the computer's netmap name, StableID and independent TLS key. The phone resolves the pinned entry directly from its netmap, without a DNS lookup. MagicDNS-off operation still needs genuine tailnet acceptance.

## Phone node

- Sign in inside Collie with the computer's personal account. The hostname is `collie-phone`.
- An advanced auth key must belong to that user without tags. Tagged, shared and foreign-user peers are refused in both directions.
- State lives under the app's `Application Support/collie`, excluded from backups with data protection `completeUntilFirstUserAuthentication`: node keys in `tsnet/`, paired machines in `machines.json`, and pending Live Activity ends in `activity-ends.json` (IDs and times only). The directory is 0700 and owned by the app's OS user. Deleting the app deletes its node identity.
- A paired machine record includes the positive user ID checked during pairing. Missing or zero owner records cannot reconnect or approve in the background: remove that machine and pair again explicitly. Collie does not infer an owner from the current sign-in or upgrade old tag-based pins.

## Existing tagged installations

This is a coordinated update of the phone and daemon. Existing tagged desktop nodes are rejected, never automatically retagged or deleted. Startup preserves cached advertised-tag requests through the pinned Tailscale backend API, so this refusal does not first remove tags or force reauthentication. An already-expired node still needs sign-in; Collie does not promise to repair existing expired state. Stop collied, remove only its embedded node in the Tailscale admin console, delete only its `tsnet` state directory, then run `collied setup` with the personal account. Remove old machine records on the phone and pair again. Preserve the system Tailscale client's state and corporate VPN configuration.

Stored owner conflicts also fail closed. Resolve an intentionally changed account and its old pairings explicitly while the daemon is stopped; Collie performs no migration or automatic trust reset.

## Policy file

If the existing personal tailnet policy already permits connections between your devices, no policy edit is required. Collie does not edit policy, request an administrator token or replace existing rules.

A restrictive policy can still block TCP 8457 or hide the peer from the netmap. An administrator may merge a same-user grant into the existing policy, adjusting the port if needed:

```hujson
{
  "grants": [
    {
      "src": ["autogroup:member"],
      "dst": ["autogroup:self"],
      "ip": ["tcp:8457"],
    },
  ],
}
```

`autogroup:self` matches a member's own user-owned devices. It does not grant another user access to yours. This example applies to every member's own devices; an administrator can restrict the source further to a specific user. Review the complete policy in Access controls and Preview rules. Grants add access, so this narrow entry does not cancel broader existing rules.

User ownership has a network privilege tradeoff: the embedded computer node receives the access granted to that user, which may include outbound access to other devices. It no longer has the separate service-tag confinement of the previous setup. Collie's same-user gate and mTLS protect its application sessions, not the rest of the user's tailnet from a compromised daemon. The phone opens no Collie listener, but Tailscale's in-process peerapi remains subject to policy.

`collied doctor` and setup report the number of visible same-user untagged peers. Zero can mean the phone has not signed in, has not appeared yet, or is hidden by policy. Seeing a peer is not proof that TCP 8457 is permitted. Isolated tests exercise an allow-all network with foreign-user first-pair rejection and a deny-all network that blocks even same-user nodes; administrator policy and genuine account behavior still need live acceptance.

## Pairing and connection authorization

1. Before first pairing or a reconnect, collied verifies its current embedded self identity and fresh whois for the accepted peer address. The peer must be untagged, unshared and owned by the bound positive user.
2. A paired StableID must have the same stored user and independent TLS key. An unpaired phone needs a locally opened pairing window. All other connections are refused before the WebSocket upgrade.
3. One pairing window lasts 120 seconds. The first `pair.complete` burns it, correct code or not. The right code still needs an explicit local **y/N** after the computer displays the candidate label, login, user ID and StableID; no answer within 60 seconds means no.
4. After TLS, collied rechecks actual self identity, fresh peer whois and the same gate under the peers, pairing and sessions locks. Identity changes, revocation and window changes refuse admission.
5. The phone checks its current embedded self user against the saved owner, the computer's same-user untagged netmap entry, fresh unshared whois, exact StableID, pinned TLS key and hello node ID before using a session. Initial pairing uses the current verified user and records it. Reconnects and one-shot background approvals use the same checks.

Machine kind comes from netmap OS metadata at pairing and is presentation only. It cannot authorize a peer or replace a user, StableID or key pin. A missing/unsupported OS needs a supported node for pairing, while reconnects retain their existing display kind.

## Key expiry

Both embedded nodes are now ordinary user-owned devices with the tailnet's normal key expiry. A new personal tailnet normally uses 180 days. Do not assume the desktop node's key is non-expiring as it was when first authenticated with a tag.

When sign-in expires, connections stop. Run `collied setup` or stop the service and run `collied login` on the computer; sign in again inside Collie on the phone. Background approvals fail as unauthorized instead of opening a login page. Recovery preserving a same-user node and pins must be tested on real devices; a changed user or new StableID requires explicit pairing again. Revocation remains necessary for a lost device.

For an unattended trusted computer, its owner can optionally choose **Disable Key Expiry** for only Collie's embedded computer node on the Tailscale admin console Machines page. This is a per-device choice, not a policy edit; Collie never changes it automatically. Keeping expiry enabled limits forgotten access. See [Tailscale key expiry](https://tailscale.com/docs/features/access-control/key-expiry). Leave the phone's expiry enabled.

## Revocation

Do both steps; each covers what the other cannot.

1. `collied peers revoke <label or StableID>` removes the phone from `peers.json` and drops its push tokens. Removing the machine in the app does the same when the machine is reachable at that moment; otherwise run the command. With the daemon running, it saves first and then closes every live session of that node; with the daemon stopped, it edits `peers.json` directly. It stops collie access immediately, even if the node stays in the tailnet.
2. Admin console, Machines, the phone node: **Remove**. This deletes its node key from the tailnet, so it can no longer reach anything, collie or not. **Expire key** alone only forces a re-login, which a thief could complete if the phone still holds a signed-in session with your identity provider.

For a compromised Mac: remove the Mac node in the admin console, delete `~/Library/Application Support/collie/tsnet`, and revoke the APNs key in the Apple Developer portal. For a compromised Linux machine: the same with its own node, `~/.local/share/collie/tsnet` (or `$XDG_DATA_HOME/collie/tsnet`) and its own APNs key; each machine's key is revoked on its own.
