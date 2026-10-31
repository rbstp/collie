# Tailnet setup

Collie embeds its own Tailscale nodes (libtailscale's tsnet, userspace netstack) in the phone app and `collied`. Sign in to both with the same personal Tailscale account; both nodes are user-owned and untagged.

Neither device needs the Tailscale app or a VPN profile: Collie installs no Network Extension, TUN interface, system routes or `tailscaled`, and keeps its node state and keys apart from a native Tailscale client. Leave a corporate Tailscale VPN client and its account alone; a corporate network or VPN policy can still restrict the underlying transport.

The phone dials the computer on TCP 8457 inside the tailnet; `collied` listens only there and never dials the phone.

Local testcontrol cannot prove coexistence with a corporate VPN on a physical phone and Mac, operation with MagicDNS off, administrator policies, real accounts or key-expiry recovery that keeps the same node and pins: those need real devices on a real tailnet.

## Computer node

- Run `collied setup` and sign in with the phone's personal account. On macOS, an interactive login without an auth key also tries once to open the sign-in page.
- For headless use, `collied login` accepts an untagged user auth key in `COLLIE_TS_AUTHKEY`. `collied` removes it from its environment, `collied run` ignores it, and no key is needed once the node state is saved.
- `[tailnet] hostname` (default `collie-<hostname>`) and `[tailnet] port` (default 8457) override the defaults.
- Node state lives in `~/Library/Application Support/collie/tsnet` on macOS, and in `$XDG_DATA_HOME/collie/tsnet`, else `~/.local/share/collie/tsnet`, on Linux: 0700 directories owned by the current OS user.
- Login and startup require a positive embedded self user ID, no tags, and an unshared whois of the node's own address matching that user and StableID; admission binds to this owner before the control socket or tailnet listener opens. An optional `[tailnet] owner_user_id`, the owner in `peers.json` and every paired phone must match that user, or startup fails closed, so no phone can choose the owner. After an intentional account change, resolve the old pairings while the daemon is stopped.

## Phone node

- Sign in inside Collie with the computer's personal account; the node is named `collie-phone`. An optional auth key must belong to that user and carry no tags.
- State lives in the app's `Application Support/collie`, a 0700 directory owned by the app's OS user, excluded from backups and protected as `completeUntilFirstUserAuthentication`. Deleting the app deletes its node identity.
- Each paired machine record keeps the positive user ID checked at pairing. Collie never infers an owner from the current sign-in: a record without a positive owner must be removed and paired again.

## Existing tagged installations

Desktop v0.1.0 created a tagged node, which later `collied` releases and the matching app refuse. Collie never retags or deletes it. To reset, update the app and `collied` together, then:

1. Stop `collied`.
2. Remove only Collie's embedded node in the Tailscale admin console.
3. Delete only its `tsnet` state directory.
4. Run `collied setup` with the personal account.
5. On the phone, remove the old machine and pair again.

## Policy file

A policy that already lets your devices reach each other needs no edit, and Collie never edits policy or asks for an administrator token. A restrictive policy can block TCP 8457 or hide the peer from the netmap. An administrator can then merge this same-user grant into it, adjusting the port if needed:

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

`autogroup:self` limits each member to their own devices, and the source can be narrowed to one user. Grants only add access, so this entry cancels no broader rule: review the whole policy in Access controls and Preview rules.

User ownership gives the computer node its user's network access, possibly including outbound access to other devices: a compromised daemon can use it, and Collie's gate protects only Collie's sessions ([Trust boundaries](threat-model.md#trust-boundaries)). The phone opens no Collie listener, but serves Tailscale's peerapi in-process to peers the policy allows.

`collied doctor`, which setup runs, counts visible same-user untagged peers; with none, setup also prints the grant above. A visible peer does not prove TCP 8457 is allowed.

## Pairing and connection authorization

1. Before the TLS handshake, `collied` verifies its own embedded identity and a fresh whois of the peer, which must be untagged, unshared and owned by the bound positive user, and either a paired StableID with the same stored user and TLS key or an unpaired phone during a locally opened pairing window. Every other connection is refused.
2. After TLS, `collied` repeats these checks under the peers, pairing and sessions locks; an identity change, a revocation or a window change refuses admission.
3. The pairing QR code carries the computer's netmap name, port, StableID, TLS key pin and one-time code. A pairing window lasts 120 seconds, the first `pair.complete` burns it, and even a right code needs a local **y/N** within 60 seconds ([Leaked pairing QR](threat-model.md#leaked-pairing-qr)).
4. Before using a session, the phone finds the computer's pinned entry in its own netmap, with no DNS lookup, and checks its own embedded user against the saved owner, that entry's same-user untagged ownership, a fresh unshared whois, the exact StableID, the pinned TLS key and the node ID in `hello`. A first pairing records the current verified user; reconnects and one-shot background approvals make the same checks.

Pairing needs a node whose netmap OS is macOS or Linux. That machine kind is display only and never authorizes a peer.

## Key expiry

Both nodes follow the tailnet's key expiry, 180 days by default for a new personal tailnet. When a sign-in expires, connections stop and background approvals fail as unauthorized instead of opening a login page. Sign in again with `collied setup` (or stop the service and run `collied login`) and inside Collie on the phone; a changed user or a new StableID requires pairing again.

On an unattended trusted computer, you can choose **Disable Key Expiry** for Collie's computer node only, on the admin console's Machines page; Collie never changes it. Keeping expiry on limits forgotten access ([Tailscale key expiry](https://tailscale.com/docs/features/access-control/key-expiry)); leave the phone's on.

## Revocation

Do both steps; each covers what the other cannot.

1. `collied peers revoke <label or StableID>` works with the daemon running or stopped: it removes the phone from `peers.json`, drops its push tokens and closes its live sessions. Collie access stops at once, even if the node stays in the tailnet. Removing the machine in the app does the same while the machine is reachable; otherwise run the command.
2. On the admin console's Machines page, **Remove** the phone's node: this deletes its node key, so it reaches nothing on the tailnet. **Expire key** alone only forces a new login, which a thief could complete if the phone still has a signed-in session with your identity provider.

For a compromised computer, remove it in the app, remove its node in the admin console and delete its `tsnet` directory ([Computer node](#computer-node)). If it uses direct APNs (`[apns]`), also revoke its APNs key in the Apple Developer portal.
