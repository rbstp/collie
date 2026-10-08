# Tailnet setup

collie runs two Tailscale nodes of its own, both embedded through libtailscale (tsnet, userspace netstack):

| Node | Runs in | Identity | Login |
|---|---|---|---|
| Mac | collied | `tag:collie-mac` (tag-owned, no user) | `collied login`: interactive by a tag owner, or a `tag:collie-mac` auth key |
| Linux machine | collied | `tag:collie-linux` (tag-owned, no user) | `collied login`: interactive by a tag owner, or a `tag:collie-linux` auth key |
| Phone | Collie.app (CollieCore) | user-owned by you | interactive login inside the app, or an auth key for your own user |

No Tailscale app is needed or used on either device. Neither node installs a VPN profile, a Network Extension or a system `tailscaled`; each is a separate device in the admin console. A Tailscale app already installed on the Mac or the iPhone is a different node and plays no part in collie.

The phone only dials (TCP to the Mac on port 8457). The Mac only listens through `tailnet::Node::listen` on that port; it never dials the phone.

## Mac node

- `collied login` starts the node with `advertise_tags = ["tag:collie-mac"]` and prints the login URL (and its QR). Open it and sign in as a user listed in `tagOwners` for `tag:collie-mac`. The node then belongs to the tag, not to you: tags replace user ownership.
- Instead of the interactive login, `collied login` accepts an auth key in `COLLIE_TS_AUTHKEY`. Use a key that applies `tag:collie-mac`. collied reads the variable and removes it from its own environment before doing anything else; `collied run` ignores it. The key is only needed once: the node keys persist in the state directory.
- Hostname defaults to `collie-<mac hostname>` (`[tailnet] hostname` in `collied.toml` overrides it).
- Node state (machine and node keys) lives in `~/Library/Application Support/collie/tsnet`, a 0700 directory owned by your user. `tailnet::Node::new` refuses a directory that is group or world accessible.
- The pairing QR carries the Mac's tailnet node name (its netmap DNS name) and `StableID`. The phone matches them in its own netmap and dials the node's IP directly, so no DNS lookup is involved (see [Phone side](#phone-side)). MagicDNS should therefore not be required; this is not yet verified on a tailnet with MagicDNS off.
- The port is `[tailnet] port` in `collied.toml` (default 8457). If you change it, change the grant and the `tests` below to match.

## Linux node

- Same as the Mac node, with `tag:collie-linux` instead of `tag:collie-mac`, hostname `collie-<hostname>`, and node state in `$XDG_DATA_HOME/collie/tsnet` (else `~/.local/share/collie/tsnet`), 0700.
- `collied login` and `collied run` refuse a node whose own tags do not include `tag:collie-linux`. Logging in again does not change the tags of a node that is already registered: remove it in the admin console (Machines), delete the `tsnet` directory, check that your user is a tag owner of `tag:collie-linux`, then run `collied login` again (phones pair again).
- A system Tailscale (`tailscaled`, the `tailscale` CLI) on the same machine is a different node and plays no part in collie. The two run side by side: the embedded node has its own state and keys, uses a userspace netstack (no TUN device, no routes) and its own WireGuard UDP port.
- The phone accepts a Linux node at pairing and from then on requires `tag:collie-linux` on it.

## Phone node

- The app starts its node (hostname `collie-phone`) and opens the Tailscale login page. Sign in as the user who owns collied (see [Owner](#owner)).
- The onboarding screen also accepts an auth key, used once and not stored. It must be a key for your own user, not a tagged key: a tagged phone node is refused by collied.
- The node must stay untagged and must not be shared in from another tailnet: collied refuses both.
- State lives in the app container under `Application Support/collie` (node keys in `tsnet/`, paired Macs in `machines.json`, pending Live Activity ends in `activity-ends.json`, ids and times only). `Collie/Sources/Collie/StateDirectory.swift` creates it 0700 with data protection `completeUntilFirstUserAuthentication`, and collie-core refuses a directory that is group or world accessible or owned by another user. Deleting the app deletes the node identity.
- The state directory must be excluded from iCloud and Finder backups. Otherwise restoring a backup, on this phone or another device, brings back the same node keys and `StableID`, which collied accepts as the paired phone. `StateDirectory.swift` excludes it from backup.

## Policy file

Replace `you@example.com` with your login. HuJSON, current grants syntax:

```hujson
{
  "tagOwners": {
    "tag:collie-mac": ["you@example.com"],
    "tag:collie-linux": ["you@example.com"],
  },

  "grants": [
    // Your devices may open TCP 8457 on the collie Mac node. No rule names
    // tag:collie-mac as a source, so the Mac node cannot initiate anything.
    {
      "src": ["you@example.com"],
      "dst": ["tag:collie-mac"],
      "ip": ["tcp:8457"],
    },
    // The same for a Linux machine running collied.
    {
      "src": ["you@example.com"],
      "dst": ["tag:collie-linux"],
      "ip": ["tcp:8457"],
    },
  ],

  "tests": [
    {
      "src": "you@example.com",
      "proto": "tcp",
      "accept": ["tag:collie-mac:8457"],
      "deny": ["tag:collie-mac:22", "tag:collie-mac:8458"],
    },
    {
      "src": "you@example.com",
      "proto": "udp",
      "deny": ["tag:collie-mac:8457"],
    },
    {
      "src": "tag:collie-mac",
      "deny": ["you@example.com:8457", "you@example.com:22", "you@example.com:443"],
    },
    {
      "src": "you@example.com",
      "proto": "tcp",
      "accept": ["tag:collie-linux:8457"],
      "deny": ["tag:collie-linux:22", "tag:collie-linux:8458"],
    },
    {
      "src": "tag:collie-linux",
      "deny": ["you@example.com:8457", "you@example.com:22", "you@example.com:443", "tag:collie-mac:8457"],
    },
  ],
}
```

Grants only add access, so this snippet is only tight if nothing else in the policy reaches `tag:collie-mac` or lets it reach anything:

- Remove the default allow-all grant (`{"src": ["*"], "dst": ["*"], "ip": ["*"]}`) or its legacy `acls` form (`"src": ["*"], "dst": ["*:*"]`). Both include tagged nodes in both directions. If you want your own devices to keep reaching each other, `{"src": ["autogroup:member"], "dst": ["autogroup:self"], "ip": ["*"]}` does that without touching tagged nodes.
- No other grant may use `*`, `autogroup:tagged`, `tag:collie-mac` or `tag:collie-linux` in `src` or `dst`, nor anything that covers the Mac node's tailnet addresses: a CIDR such as `100.64.0.0/10` or `fd7a:115c:a1e0::/48`, a `hosts` alias, or an `ipsets` entry.
- The phone node is user-owned, so any rule whose `dst` covers your devices (for example `autogroup:self`) also reaches it. It opens no listener of its own; see the threat model for what remains reachable.
- To add another user, give them their own collie Mac tag and grant; do not widen `src`. Each collied serves one owner.

### Checking it in the admin console

1. Access controls: paste the snippet, save. The `tests` block runs on every save and rejects a policy that breaks it.
2. Access controls, **Preview rules** tab: select your user. For collie the only entries involving the collie tags must be `tag:collie-mac:8457` and `tag:collie-linux:8457` over TCP. Any broader entry (`*`, other ports) means another rule still matches.
3. Select another user, if the tailnet has one: neither collie tag may appear. This matters most before the first pairing, when collied has no owner yet (see below).
4. Machines: the Mac node shows the `collie-mac` tag and no owner; the phone node shows your user and no tag.

## What collied enforces anyway

The policy is the first filter, not the authorization. collied checks every accepted connection before the WebSocket upgrade (`crates/collied/src/gate.rs`):

1. The peer address comes from `tailscale_accept_with_addr` (patched libtailscale), returned with the connection fd itself. The fd-keyed `tailscale_getremoteaddr`, which mis-attributed peers under concurrency, is never used.
2. `whois(peer)` must return a node that is **untagged** and **not shared in** (`Sharer == 0`).
3. If an owner is known, the node must be owned by that user (`WhoIsNode::is_owned_by`).
4. Then:
   - its `StableID` is in `peers.json` with the same user ID it was paired with and a pinned TLS key: full session (`hello` first, then the method allowlist). A paired `StableID` that now reports another user is refused;
   - it is not paired, or was paired before mutual TLS and has no TLS key, and a pairing window opened locally with `collied pair` is active: pairing-only session (`hello`, `pair.complete`, then close);
   - otherwise: closed before the upgrade and audited.
5. The TLS handshake must prove the key pinned for that phone (any P-256 key in a pairing-only session), and the gate then decides again under the peers, pairing and sessions locks: a different decision (a revoke, a pairing, or the pairing window closing during the handshake) closes the connection.

So a device of yours that the policy lets through still gets nothing until it is paired, and a tagged or shared-in node is refused even if a policy mistake lets it reach port 8457.

### Pairing

- One window at a time, open for 120 s. The first `pair.complete` from any connection closes it, whether the code is right or wrong.
- With the right code, `collied pair` shows the candidate's device label, node name, `StableID`, login name and numeric user ID (from whois) and asks **y/N** on the Mac. No answer within 60 s means no.
- Only a confirmed candidate is written to `peers.json`.

### Owner

`owner_user_id` is the numeric Tailscale user ID of the person allowed to use this collied. It comes from one of two places:

- `[tailnet] owner_user_id` in `collied.toml`. When set, the gate refuses every other user and pairing refuses to store one. collied refuses to start if it differs from the owner recorded in `peers.json`.
- Otherwise, the first pairing you confirm records that device's user as the owner in `peers.json` (trust on first use). Revoking every phone does not clear it.

Until an owner is known, step 3 is skipped: any untagged, not-shared-in node that the policy lets reach the port can open a pairing-only session while a window is open, and only the pairing code and your y/N stand in its way. Before the first pairing, make sure the policy admits only you (check 3 above), and on the y/N prompt check that the login name is yours.

Neither device has a Tailscale CLI, so the numeric ID is not shown anywhere else: read it from the y/N prompt or, after the first pairing, from `collied peers list`. Copying it into `collied.toml` keeps the owner pinned even if `peers.json` is deleted.

### Phone side

The phone checks the other direction (`crates/collie-core/src/pin.rs`). It does not use a DNS answer: it takes the machine's IP from its own netmap entry whose DNS name matches and whose `StableID` is the one pinned at pairing, requires that entry to carry the tag seen at pairing (`tag:collie-mac` or `tag:collie-linux`; pairing accepts either), and dials that IP once whois confirms the IP is that node and not shared in. The TLS handshake must show the machine's key pinned at pairing, and `hello` must report the pinned node ID. A different node under the name, a different key, or a machine that lost or changed its tag, is a pin violation and is not retried automatically.

## Key expiry

| Node | Recommendation | On expiry |
|---|---|---|
| Mac or Linux machine (`tag:collie-mac`, `tag:collie-linux`) | Expiry disabled. Tailscale disables it by default for a device tagged at first authentication; confirm "Expiry disabled" on the Machines page. | collied's node goes to `NeedsLogin`, the phone cannot connect, `collied login` re-authenticates. |
| Phone (user-owned) | Keep the tailnet's expiry (default 180 days). It bounds how long a lost phone's node key works if revocation is forgotten. | The app's node goes to `NeedsLogin` and shows the login URL. Re-authenticating the same node should keep its `StableID`, so the pairing stays valid (not yet verified on a device). Signing in as another user changes the node's user and collied refuses it. |

Deleting and reinstalling the app creates a new node with a new `StableID`: pair again and remove the old device in the admin console.

## Revocation

Do both steps; each covers what the other cannot.

1. `collied peers revoke <label or StableID>` removes the phone from `peers.json` and drops its push tokens. Removing the machine in the app does the same when the machine is reachable at that moment; otherwise run the command. With the daemon running, it saves first and then closes every live session of that node; with the daemon stopped, it edits `peers.json` directly. It stops collie access immediately, even if the node stays in the tailnet.
2. Admin console, Machines, the phone node: **Remove**. This deletes its node key from the tailnet, so it can no longer reach anything, collie or not. **Expire key** alone only forces a re-login, which a thief could complete if the phone still holds a signed-in session with your identity provider.

For a compromised Mac: remove the Mac node in the admin console, delete `~/Library/Application Support/collie/tsnet`, and revoke the APNs key in the Apple Developer portal. For a compromised Linux machine: the same with its own node, `~/.local/share/collie/tsnet` (or `$XDG_DATA_HOME/collie/tsnet`) and its own APNs key; each machine's key is revoked on its own.
