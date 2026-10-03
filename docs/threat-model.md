# Threat model

Scope: collied on the Mac, Collie.app on the iPhone, the tailnet between them, herdr as collied's local backend, and APNs. Tailnet configuration is in [tailnet.md](tailnet.md), design in [architecture.md](architecture.md).

## Assets

| Asset | Where |
|---|---|
| Control of every shell and agent on the Mac | herdr socket (0600, no auth: same UID means full control); reachable from the phone only through collied's allowlist |
| Mac node keys | `~/Library/Application Support/collie/tsnet` (0700) |
| Phone node keys | app container `Application Support/collie/tsnet` (0700, `completeUntilFirstUserAuthentication`), not excluded from backups yet |
| Pairings (`StableID` list) and the owner user ID | `peers.json` in collied's data dir (0600); optional `owner_user_id` in `collied.toml` |
| Pairing code | the QR and the invite URI that `collied pair` prints as text in the terminal; 16 random bytes, one window at a time, 120 s, burned by the first attempt, redacted in `Debug` |
| Approval nonces | collied memory and the phone session; 32 random bytes, single use |
| APNs `.p8` key | file named in `collied.toml` (0600) |
| Terminal content, agent and workspace names | herdr, the tailnet session, the phone |

## Trust boundaries

- **Tailnet to collied.** The only network entry point is the tsnet listener on TCP 8457. Every connection passes the whois gate (untagged, not shared in, owned by the owner once one is known, paired `StableID` with its paired user) before the WebSocket upgrade, then a fail-closed method allowlist (`parse_client_frame`: unknown methods rejected before params are decoded, unknown fields denied, herdr privileged methods unreachable).
- **collied to herdr.** Same UID. Nothing on the Mac running as the user is a boundary.
- **Tailscale control plane.** Trusted for node identity: whois and the phone's netmap check both come from it.
- **Apple.** APNs sees what collied pushes.

## Status legend

- **built**: in the tree and tested today.
- **P1**: Phase 1 deliverable (login, listener, whois gate, pairing, `flock.snapshot`). The gate, pairing window, peers store, control socket, revoke, per-peer rate limit, audit log, the `collied` CLI and the app's onboarding are in the tree with unit tests; none of it is verified end to end on a device yet.
- **design**: specified in architecture.md for a later phase, not implemented yet.

## Threats from the spec

### Stolen phone

| | |
|---|---|
| Assets | Paired session (read and, once drive methods exist, control of agents); phone node keys |
| Attack | Thief opens the unlocked app, or unlocks the phone, and drives agents or answers approvals. |
| Mitigations | iOS passcode and data protection on the node state (built). Revocation: `collied peers revoke` plus removing the node in the admin console (P1, [tailnet.md](tailnet.md#revocation)). Phone node key expiry (Tailscale default 180 days) bounds a forgotten revocation. Destructive calls need `confirm: true` (field built in the protocol, enforcement design). Every frame is rate limited per paired `StableID` (20/s, burst 40) and every method other than `hello`, `flock.snapshot` and `workspace.list` is written to the audit log (P1). Approval-specific limits and audit (design). |
| Residual | Until revoked, an unlocked phone has full collie access. No app-level biometric gate exists or is designed. Lock-screen approve/deny would widen this; its authentication is undesigned. |
| Phase | Revocation P1. Lock-screen approval authentication: Phase 3 spike. App-level biometric gate: not planned. |

### Leaked pairing QR

| | |
|---|---|
| Assets | Pairing code, Mac tailnet node name, Mac `StableID` |
| Attack | Someone photographs the QR, or copies the invite URI that `collied pair` prints next to it: from terminal scrollback, a screen share or recording, or a herdr pane that other same-UID code can read. They then pair their own device. |
| Mitigations | The code alone grants nothing: the pairing session is only offered to an untagged, not-shared-in node (owned by the owner, once one is known) and only while a locally opened window is active (P1). One window at a time, 120 s; the first `pair.complete` from any connection burns it, right or wrong code (P1). With the right code, collied shows the candidate's device label, node name, `StableID`, login name and user ID and asks y/N on the Mac, 60 s timeout (P1). Code compared in constant time (P1); carried in the URI fragment and redacted in `Debug` (built). The policy grants port 8457 to the owner's devices only. |
| Residual | When `owner_user_id` is not configured and nothing is paired yet, the owner is set by the first confirmed pairing (trust on first use), so any tailnet member the policy lets reach the port can race the window with a leaked code; the y/N prompt, which shows the login name, is then the only check. An attacker controlling a node logged in as the owner (account compromise) can race the window at any time. Any node admitted to a pairing-only session can burn an open window (denial of pairing only). The printed URI stays in scrollback and screen recordings after the window closes; it is useless once burned or expired, but a capture read live during the 120 s is as good as the QR. Host name and node ID in the QR are not secret. |
| Phase | P1 |

### Malicious herdr plugin on the Mac

| | |
|---|---|
| Assets | Everything on the Mac; the phone's view of agents and approvals |
| Attack | herdr plugins are unsandboxed code running as the user with the full herdr CLI. A plugin can drive every pane directly, read collied's data dir (node keys, `peers.json`, `.p8`), drive collied's control socket, and write pane text that collied turns into approval snippets to mislead the phone. |
| Mitigations | None against the Mac side: same UID is out of scope for file modes. Snippets are capped at 200 chars with controls stripped (design), so a plugin cannot inject escapes into the phone UI through them. Approval decisions only send keystrokes from collied's fixed per-rule keystroke map, never text from the pane (design). |
| Residual | Full Mac compromise. A plugin can show the phone fake but well-formed approvals. It also controls the gate's inputs: it can rewrite `collied.toml` and `peers.json` (owner and paired `StableID`s), answer y/N over the control socket, and run the Mac node keys elsewhere. Other users' devices are then kept out only by the tailnet policy, whose `src` names the owner alone and which only the admin account can change. |
| Phase | APNs key moved to a Keychain item bound to collied's signing identity: Phase 3. Otherwise not planned (install only trusted plugins). |

### Compromised Tailscale coordination plane

| | |
|---|---|
| Assets | Node identity, which every collie authorization decision rests on |
| Attack | The control server (or a stolen admin account) adds a node with its own key and gives it the phone's `StableID` and the owner's user ID, or changes the tag owners and grants, or points the phone's netmap at a fake Mac. |
| Mitigations | Admin account protection (IdP, MFA) is the user's. collied pins `StableID` + user ID + untagged, and the phone pins the Mac's `StableID` + `tag:collie-mac` (P1), which stops casual misconfiguration but not a control plane that lies. The y/N prompt at pairing catches a new device only during pairing. |
| Residual | High: whois and netmap data both come from control, so a malicious control plane passes the gate on both ends. Tailnet Lock (node keys signed by trusted devices) would close the node-injection path but is not part of the design and untested with tsnet. An application-level key exchanged at pairing would also close it. |
| Phase | Not planned; open decision. |

### Replayed notification

| | |
|---|---|
| Assets | Approval decisions |
| Attack | An old push (or a captured approval decision) is replayed to approve something again or something else. |
| Mitigations | The push payload never carries the nonce; it is a hint, and the approval is fetched and decided over the tailnet session (design). Nonce: 32 random bytes, single use, constant-time compare, expires after 10 minutes; every attempt, including replays and expired ones, is audited (design). Decisions are bound to a fingerprint and fail as `superseded` if the prompt changed (design). The `Nonce` type rejects malformed values and is redacted in `Debug` (built). |
| Residual | A replayed push can show a stale notification; tapping it shows the current approvals only. |
| Phase | Phase 3 (approvals and APNs) |

### Leaked APNs key

| | |
|---|---|
| Assets | `.p8` key (team wide), device tokens |
| Attack | With the key and a device token, an attacker sends arbitrary pushes to Collie (or, the key being team wide, to any app of the team) to phish the user. |
| Mitigations | Dedicated, revocable key, never in GitHub ([release.md](release.md)). `collied doctor` fails unless the key file is 0600 and owned by the user (built). Pushes carry no nonce and cannot create an approval; the app trusts only what the tailnet session returns (design). |
| Residual | Fake notifications until the key is revoked. 0600 does not stop same-UID processes. |
| Phase | Keychain item bound to collied's signing identity: Phase 3 |

### Phone node key extraction

| | |
|---|---|
| Assets | Phone node keys, which pass the whois gate as the paired phone |
| Attack | Keys read from the device (forensic tools, jailbreak, exploit) or from a backup, then used from another machine. |
| Mitigations | App sandbox; state dir 0700 with data protection `completeUntilFirstUserAuthentication`, set by `StateDirectory.swift` and checked by collie-core (P1). State dir excluded from backups (P1 requirement, not in the tree yet). Revocation and key expiry as for a stolen phone. |
| Residual | Keys are files, not Keychain or Secure Enclave items: readable after first unlock by code inside the app's sandbox. Until the backup exclusion lands, an iCloud or Finder backup carries the node identity, and restoring it on another device passes the gate as the paired phone. Extracted keys give the attacker the phone's full collie access from anywhere until revoked. |
| Phase | Backup exclusion (`isExcludedFromBackup`, or keys in a `ThisDeviceOnly` Keychain item): P1. Keychain or Secure Enclave keys: not planned. |

## Threats from the design

### Same-UID malware on the Mac

| | |
|---|---|
| Assets | herdr, collied's data dir, the APNs key, the control socket |
| Attack | Any process running as the user controls herdr directly, reads node keys and `peers.json`, uses the 0600 control socket. |
| Mitigations | None by design: same UID already owns every shell. Data dir 0700, files and control socket 0600, control socket peer UID checked: these stop other local users only (built for tsnet state and doctor checks; peers store and control socket P1). |
| Residual | Full compromise of the Mac side. Stolen Mac node keys let the malware impersonate the Mac to the phone and show fake state or fake approvals. It controls the gate's configuration (`collied.toml`, `peers.json`) and can answer the y/N prompt, so the gate no longer protects anything; other users' devices are kept out only by the tailnet policy, which needs the admin account to change. |
| Phase | APNs key in Keychain: Phase 3. Otherwise out of scope. |

### LAN attacker

| | |
|---|---|
| Assets | Any socket the Mac or phone exposes |
| Attack | Scan and connect to ports on the Mac or the phone from the local network. |
| Mitigations | No kernel TCP listener (built): collied listens only through tsnet; `tailscale_loopback` (127.0.0.1 SOCKS5 and LocalAPI) is never called; the peerapi kernel listener, which upstream binds on every interface in netstack mode, is patched out. `crates/tailnet/tests/end_to_end.rs` asserts the process has no TCP LISTEN socket. The only real-interface socket is magicsock UDP, which accepts only WireGuard and disco packets authenticated against netmap keys. Control socket is a Unix socket (P1). |
| Residual | magicsock UDP is reachable and parses unauthenticated packets before rejecting them. Its NAT port mapping (UPnP, NAT-PMP, PCP) can expose that UDP port beyond the LAN. Traffic metadata (timing, DERP use) is visible. On the tailnet side, the phone node is user-owned, so any policy rule reaching the owner's devices reaches it: it opens no listener of its own, but netstack serves Tailscale's peerapi to those peers in-process. |
| Phase | Built |

### Prompt injection into the terminal

| | |
|---|---|
| Assets | The shells behind herdr panes |
| Attack | A prompt or key sequence sent from the phone, or a compromised phone session, escapes the agent and runs commands in the shell. |
| Mitigations | `PromptText` rejects ESC and every C0/C1 control except newline and tab, since herdr sends prompts as bracketed paste and an embedded `ESC[201~` would end the paste (built, tested). `Key` is a closed list of nine keys (built). herdr privileged methods (`pane.send_text`, `pane.send_input`, `agent.start`, `plugin.*`, `server.*`) are not in the allowlist (built, tested). collied refuses `agent.prompt` and `agent.send_keys` while an agent is `blocked`, re-checked with `agent.get` just before the call, so blocked prompts are answered only through approvals (design). Foreground agent check before sending (design). |
| Residual | herdr checks the foreground agent before queuing the text and the delayed Enter: if the agent exits in between, the shell receives the prompt. Closing it needs conditional input in herdr upstream. Text an agent reads from a repository can still steer the agent itself; that is outside collie. |
| Phase | Validation built; refusal and foreground check with the drive methods (after Phase 1); herdr conditional input: upstream proposal. |

### Approval TOCTOU

| | |
|---|---|
| Assets | The decision the user meant to make |
| Attack | The question under a still-`blocked` agent changes (new tool call, different command) between display and decision, so "approve" answers a different question. |
| Mitigations | Approval bound to a fingerprint: hash of the detection-region text, matched rule id, `terminal_id`, agent session id. Re-read immediately before the keystroke; any change marks it `superseded` (design). Outcome is `applied` only if the agent leaves `blocked`, otherwise `unconfirmed`; the lock screen never claims success on a queued write (design). 10 minute expiry (design). |
| Residual | The read-then-send gap remains until herdr offers conditional input. |
| Phase | Phase 3 |

### Apple sees push payload metadata

| | |
|---|---|
| Assets | Project and agent names, activity timing |
| Attack | Apple (or anyone with APNs access) reads payloads and metadata. |
| Mitigations | Alert carries agent name, status and workspace label only, never terminal text, snippet or nonce; details are fetched over the tailnet (design). |
| Residual | Labels, timing, frequency and the device token are visible to Apple. Encrypting the payload for the NSE would hide labels but not timing; not designed. |
| Phase | Phase 3 (APNs). Encrypted payload: not planned. |
