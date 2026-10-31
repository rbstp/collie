# Tailnet setup

Collie embeds independent Tailscale nodes, through libtailscale (tsnet, userspace netstack), in the phone app and in `collied` on the computer. Sign in to both with the same personal Tailscale account; both nodes are user-owned and untagged, so no tag setup is needed.

Neither device needs the standalone Tailscale app, an OS VPN profile or the VPN toggle in Settings. Collie installs no Network Extension, TUN interface, system routes or system `tailscaled`, and keeps its node state, keys and connections separate from a native Tailscale client. On a Mac with a corporate Tailscale VPN, leave that client and its account alone. A corporate network or VPN policy can still restrict the underlying internet transport, and coexistence with a corporate VPN on a physical phone and Mac remains a release acceptance check, not something local testcontrol establishes.

The phone dials the computer on TCP 8457 inside the tailnet. `collied` listens only through `tailnet::Node::listen` and never dials the phone.

## Computer node

- Run `collied setup` and sign in with the phone's personal account. It prints the sign-in link and a QR code; on macOS, an interactive login without an auth key also tries once to open the page in the browser. Pairing still needs its local confirmation.
- For headless use, `collied login` also accepts an untagged user auth key in `COLLIE_TS_AUTHKEY`. `collied` removes the variable from its own environment, and `collied run` ignores it. Once the node state is saved, no key needs to be kept.
- The hostname defaults to `collie-<hostname>` and the port to 8457; `[tailnet] hostname` and `[tailnet] port` override them.
- Node state lives in `~/Library/Application Support/collie/tsnet` on macOS, and in `$XDG_DATA_HOME/collie/tsnet`, else `~/.local/share/collie/tsnet`, on Linux. The directories are 0700 and owned by the current OS user.
- Login and server startup require a positive embedded self user ID, no tags, and a whois of the node's own address that matches that user and StableID without sharing. The server binds admission to this owner before it opens its control socket or tailnet listener.
- An optional `[tailnet] owner_user_id` adds a pin. It, the owner recorded in `peers.json` and every paired phone there must match the embedded self user, or startup fails closed. The first phone cannot choose the owner.
- The pairing QR code carries the computer's netmap name, port, StableID, TLS key pin and one-time code. The phone resolves the pinned entry from its own netmap, without a DNS lookup. Operation with MagicDNS off still needs acceptance on a real tailnet.

## Phone node

- Sign in inside Collie with the computer's personal account. The node's hostname is `collie-phone`.
- An auth key, the advanced option, must belong to that user and carry no tags. Collie refuses tagged, shared and foreign-user peers in both directions.
- State lives in the app's `Application Support/collie`, a 0700 directory owned by the app's OS user, excluded from backups and under data protection `completeUntilFirstUserAuthentication`. It holds node keys in `tsnet/`, paired machines in `machines.json` and pending Live Activity ends in `activity-ends.json` (IDs and times only). Deleting the app deletes its node identity.
- A paired machine record keeps the positive user ID checked at pairing. A record with a missing or zero owner cannot reconnect or approve in the background: remove that machine and pair again. Collie never infers an owner from the current sign-in.

## Existing tagged installations

Desktop v0.1.0 created a tagged desktop node, which later `collied` releases and the matching app refuse. Collie never retags or deletes it automatically. Startup keeps the node's cached tag requests, so the refusal neither strips its tags nor forces a new sign-in; an already expired node still needs one. To reset, update the app and `collied` together, then:

1. Stop `collied`.
2. Remove only Collie's embedded node in the Tailscale admin console.
3. Delete only its `tsnet` state directory.
4. Run `collied setup` with the personal account.
5. On the phone, remove the old machine and pair again.

Leave the system Tailscale client's state and the corporate VPN configuration alone.

A stored owner that differs from the signed-in user also fails closed. After an intentional account change, resolve the old pairings yourself while the daemon is stopped; Collie never migrates or resets trust automatically.

## Policy file

If your tailnet policy already lets your devices connect to each other, you need no policy edit. Collie never edits the policy, asks for an administrator token or replaces rules.

A restrictive policy can still block TCP 8457 or hide the peer from the netmap. An administrator can merge a same-user grant into the existing policy, adjusting the port if needed:

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

`autogroup:self` matches a member's own user-owned devices; it does not give another user access to yours. This example covers every member's own devices, and an administrator can narrow the source to one user. Review the whole policy in Access controls and Preview rules. Grants only add access, so this entry does not cancel broader rules.

User ownership has a cost: the computer node gets its user's network access, which may include outbound access to other devices, with no service-tag confinement. Collie's same-user gate and mutual TLS protect Collie's sessions, not the rest of your tailnet from a compromised daemon. The phone opens no Collie listener, but Tailscale's in-process peerapi remains subject to policy.

`collied doctor`, which setup also runs, reports how many same-user untagged peers are visible; when there are none, setup also prints the policy grant above. Zero can mean the phone has not signed in or appeared yet, or that policy hides it. A visible peer does not prove TCP 8457 is allowed. Isolated tests cover an allow-all network that still rejects a foreign user's first pairing and a deny-all network that blocks even same-user nodes; administrator policy and real account behavior still need live acceptance.

## Pairing and connection authorization

1. Before a first pairing or a reconnect, `collied` verifies its own embedded identity and a fresh whois of the peer's address. The peer must be untagged, unshared and owned by the bound positive user.
2. A paired StableID must have the same stored user and TLS key. An unpaired phone needs a pairing window opened locally. Every other connection is refused before the TLS handshake.
3. A pairing window lasts 120 seconds. The first `pair.complete` burns it, right code or not. A right code still needs a local **y/N** after the computer shows the candidate's device label, node, StableID, login and user ID; no answer within 60 seconds means no.
4. After TLS, `collied` rechecks its own identity, a fresh peer whois and the same gate while holding the peers, pairing and sessions locks. An identity change, a revocation or a window change refuses admission.
5. Before using a session, the phone checks its own embedded user against the saved owner, the computer's same-user untagged netmap entry, a fresh unshared whois, the exact StableID, the pinned TLS key and the node ID in `hello`. A first pairing uses the current verified user and records it. Reconnects and one-shot background approvals make the same checks.

Machine kind (Mac or Linux) comes from the netmap's OS metadata at pairing and is presentation only: it never authorizes a peer or replaces a user, StableID or key pin. Pairing needs a node that reports macOS or Linux; reconnects keep the kind recorded at pairing.

## Key expiry

Both embedded nodes are ordinary user-owned devices with the tailnet's normal key expiry, 180 days by default for a new personal tailnet.

When a sign-in expires, connections stop. On the computer, run `collied setup`, or stop the service and run `collied login`; on the phone, sign in again inside Collie. Background approvals fail as unauthorized instead of opening a login page. Recovery that keeps the same node and pins still needs testing on real devices; a changed user or a new StableID requires pairing again. A lost device still needs revocation.

On an unattended trusted computer, you can choose **Disable Key Expiry** for Collie's embedded computer node only, on the Machines page of the Tailscale admin console. It is a per-device choice, not a policy edit, and Collie never changes it. Keeping expiry on limits forgotten access; see [Tailscale key expiry](https://tailscale.com/docs/features/access-control/key-expiry). Leave the phone's expiry on.

## Revocation

Do both steps; each covers what the other cannot.

1. `collied peers revoke <label or StableID>` removes the phone from `peers.json` and drops its push tokens. Removing the machine in the app does the same if the machine is reachable at that moment; otherwise run the command. With the daemon running, it saves first, then closes every live session of that node; with the daemon stopped, it edits `peers.json` directly. Collie access stops at once, even if the node stays in the tailnet.
2. In the admin console, under Machines, choose **Remove** for the phone's node. This deletes its node key from the tailnet, so it can reach nothing, Collie or not. **Expire key** alone only forces a new login, which a thief could complete if the phone still has a signed-in session with your identity provider.

For a compromised computer, remove it in the app, remove its node in the admin console and delete its `tsnet` directory (`~/Library/Application Support/collie/tsnet` on macOS; `$XDG_DATA_HOME/collie/tsnet` or `~/.local/share/collie/tsnet` on Linux). If it uses direct APNs (`[apns]`), also revoke its APNs key in the Apple Developer portal.
