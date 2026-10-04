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
| Approval nonces | collied memory and the phone session (collie-core only, never handed to Swift); 32 random bytes, single use |
| APNs `.p8` key | login Keychain item `dev.rbstp.collied.apns`/`<key ID>`, ACL trusting only the Developer ID signed collied (legacy: 0600 file named by `key_path`); a backup in the user's password manager |
| Live Activity update tokens | collied's `push.json` (0600) next to the device token, bound to the device's `StableID`; collie-core memory. Never logged, audited or written to disk on the phone; redacted in `Debug` |
| Notification keys (32 bytes, one per paired Mac) | iOS Keychain (`dev.rbstp.collie.notify`/`<Mac node ID>`, `AfterFirstUnlockThisDeviceOnly`, shared with ColliePush); collied's `push.json` (0600) next to the device token; collie-core memory. Never logged, audited or written elsewhere; redacted in `Debug` |
| Terminal content, agent and workspace names | herdr, the tailnet session, the phone |
| Attachments (photos and files sent from the phone) | the phone; collied's attachments dir (`~/Library/Caches/dev.rbstp.collied/attachments`, 0700 dirs, 0600 files) for up to 24 h; the tailnet session |

## Trust boundaries

- **Tailnet to collied.** The only network entry point is the tsnet listener on TCP 8457. Every connection passes the whois gate (untagged, not shared in, owned by the owner once one is known, paired `StableID` with its paired user) before the WebSocket upgrade, then a fail-closed method allowlist (`parse_client_frame`: unknown methods rejected before params are decoded, unknown fields denied, herdr privileged methods unreachable).
- **collied to herdr.** Same UID. Nothing on the Mac running as the user is a boundary.
- **Tailscale control plane.** Trusted for node identity: whois and the phone's netmap check both come from it.
- **Apple.** APNs sees what collied pushes: labels and ids in clear, the pending action only as ciphertext, and the Live Activity content of followed agents.

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
| Mitigations | iOS passcode and data protection on the node state (built). Revocation: `collied peers revoke` plus removing the node in the admin console (P1, [tailnet.md](tailnet.md#revocation)). Phone node key expiry (Tailscale default 180 days) bounds a forgotten revocation. Destructive calls need `confirm: true` (field built in the protocol, enforcement design). Every frame is rate limited per paired `StableID` (20/s, burst 40) and every method other than `hello`, `flock.snapshot` and `workspace.list` is written to the audit log (P1). Approvals: in-app decisions need LocalAuthentication (`deviceOwnerAuthentication`, fresh context per decision), lock-screen actions need an iOS unlock (`authenticationRequired`), `approval.decide` is limited to 1/s, burst 5, per `StableID`, and every attempt is audited (built). |
| Residual | Until revoked, an unlocked phone drives agents (prompts, keys, new tasks, closes) without any per-action check; only approval decisions ask for Face ID or the passcode. A thief who knows the passcode passes both approval gates. |
| Phase | Revocation P1. Approval authentication built. App-level gate for the other drive methods: not planned. |

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
| Mitigations | None against the Mac side: same UID is out of scope for file modes. Snippets are capped at 200 chars with controls, bidi and format characters stripped (built), so a plugin cannot inject escapes into the phone UI through them. Approval decisions only send `up`, `down`, `enter` or `esc`, never text from the pane (built). |
| Residual | Full Mac compromise. A plugin can draw a fake but well-formed Claude Code menu in a pane and so show the phone a fake approval. It also controls the gate's inputs: it can rewrite `collied.toml` and `peers.json` (owner and paired `StableID`s), answer y/N over the control socket, and run the Mac node keys elsewhere. Other users' devices are then kept out only by the tailnet policy, whose `src` names the owner alone and which only the admin account can change. |
| Phase | Built as far as it goes (see same-UID malware for the Keychain item's limits). Otherwise not planned (install only trusted plugins). |

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
| Mitigations | The push payload never carries the nonce, only `approval_id` and `node_id` as lookup keys (built). A lock-screen action fetches the nonce over the tailnet (`approval.list`) from the pinned, paired Mac and decides there; the app validates both keys and trusts nothing else in the payload (built). Nonce: 32 random bytes, constant-time compare, burned by the first attempt past the rate limit, whatever its result, `approval_ttl` 10 minutes; a burned id answers `approval_already_resolved` (built). Every attempt, including replays and expired ones, is audited (built). Decisions are bound to a fingerprint and fail as `superseded` if the prompt changed (built). APNs `apns-expiration` is the approval's expiry, and the `terminal_id` collapse id makes a reissued alert replace the dead one (built). The `Nonce` type rejects malformed values and is redacted in `Debug` (built). |
| Residual | A replayed or stale push shows a notification whose `approval_id` no longer resolves: acting on it answers "no longer pending, nothing was sent". A fresh alert for the same terminal can follow each reissue while the agent stays blocked. |
| Phase | Built |

### Leaked APNs key

| | |
|---|---|
| Assets | `.p8` key (team wide), device tokens |
| Attack | With the key and a device token, an attacker sends arbitrary pushes to Collie (or, the key being team wide, to any app of the team) to phish the user. |
| Mitigations | Dedicated, revocable key, never in GitHub ([release.md](release.md)). Stored in the login Keychain by `collied apns import`, which deletes the file after confirmation; the item's ACL lets only collied signed with the team's Developer ID (`just collied-install`) read it without a prompt; `collied doctor` warns about a `key_path` config or a `.p8` left on disk (built). Pushes carry no nonce and cannot create or decide an approval: `approval_id` and `node_id` are validated lookup keys, and a lock-screen action decides only what the pinned, paired Mac returns over the tailnet (built). |
| Residual | A leaked key sends fake notifications to Collie (and to every app of the team) until revoked in the portal: a phishing alert with Approve/Deny buttons whose `approval_id` matches nothing ends as "no longer pending, nothing was sent", but its text is the attacker's (as a plaintext body only: without the notification key it cannot produce an `enc` the NSE opens). Copies outside the Keychain (the downloaded file before import, the password manager backup) are protected only by where they sit. |
| Phase | Built |

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
| Mitigations | None by design for herdr and collied's files: same UID already owns every shell. Data dir 0700, files and control socket 0600, control socket peer UID checked: these stop other local users only (built for tsnet state and doctor checks; peers store and control socket P1). The APNs key is the exception: it is a Keychain item whose ACL trusts only collied's designated requirement (Developer ID Application, team `RM3UT3MMSR`, identifier `dev.rbstp.collied`), and the binary is signed with the hardened runtime, so other programs cannot read the key bytes without a Keychain prompt (built). |
| Residual | Full compromise of the Mac side. Stolen Mac node keys let the malware impersonate the Mac to the phone and show fake state or fake approvals. It controls the gate's configuration (`collied.toml`, `peers.json`) and can answer the y/N prompt, so the gate no longer protects anything; other users' devices are kept out only by the tailnet policy, which needs the admin account to change. Keychain ACL limits: malware can raise a prompt that the user may accept; anyone with the login password can change the ACL or export the item; it can run the signed collied itself (its own `--config` or `HOME`) to make it send pushes with the key, so it can push through collied without holding the key; a `key_path` config or an un-deleted `.p8` gets none of this protection. |
| Phase | Built (Keychain item). Otherwise out of scope. |

### Live Activity on the lock screen

| | |
|---|---|
| Assets | Followed agents' status and labels; the update tokens |
| Attack | Someone looks at the locked phone, or a party holding the APNs key and a token pushes fake content. |
| Mitigations | Nothing is shown unless the user follows the agent, which is off by default. The activity shows status, labels, elapsed time and the approvals count. While an approval routed to it is pending it also shows the command, decrypted on the phone by the widget with the per-Mac notification key and marked `privacySensitive`, so iOS redacts it while the phone is locked or the display dimmed, and Approve and Deny, only when `enc` opens with the `approvalId` as AAD (an id pushed without a valid `enc` shows "Approval needed" and no buttons). The buttons are a `LiveActivityIntent` with `authenticationPolicy = .requiresAuthentication`, so iOS asks for Face ID or the passcode before the decision runs, then the same background path as the notification actions, one decision at a time per approval (nonce fetched from the pinned, paired Mac over the tailnet, fingerprint checked, `approval.decide` rate limited and audited). Tapping elsewhere only opens the app on that agent, which validates both ids and decides nothing. Tokens live only in collied's `push.json` (0600) and collie-core memory, are never logged or audited, are bound to the registering device's `StableID` (another paired device can neither replace nor end them) and are dropped on revoke, on `Unregistered`/`BadDeviceToken`, on end, when the terminal is gone and 8 h after the activity first registered (ActivityKit's activity lifetime), so a dead activity stops receiving plaintext; at most 8 per device, one per terminal (built). The alert that expands the Dynamic Island is the approval alert itself, under the same rules (a new prompt alerts, the same prompt reissued within 30 s does not), and replaces the approval notification on that device, so a flapping agent cannot turn it into an alert stream (built in collied). |
| Residual | Anyone who sees the locked phone sees the followed agents' names, workspaces, whether they wait for approval and the Approve and Deny buttons, but not the command. With the APNs key and a token, an attacker can make an activity show any status or text until it ends, including a fake "Approval needed"; buttons need an `enc` sealed under the notification key, and a decision still needs an unlock and the Mac's nonce. `enc` is bound to the approval id but not to the terminal, so such an attacker can replay a real approval's `approvalId` and `enc` onto another followed agent's activity: the real command shows, under the other agent's name. |
| Phase | Built; to be verified on device. |

### Lock-screen and in-app approval

| | |
|---|---|
| Assets | The decision on a blocked agent |
| Attack | Someone holding the phone approves or denies from a notification or from the app, or a fake alert tricks the owner into an action. |
| Mitigations | Lock-screen Approve and Deny are `authenticationRequired` on the notification and `.requiresAuthentication` on the Live Activity's intent: iOS asks for Face ID or the passcode before the app runs; collie adds no check of its own there (built for notifications). "Approve always" is not offered on the lock screen. In the app, each decision first passes LocalAuthentication `deviceOwnerAuthentication` with a fresh `LAContext`, so one unlock never covers a later approval (built). The background decision runs a 20 s capped one-shot session: never a Tailscale login (`NeedsLogin` fails at once as `Unauthorized`), pinned Mac `StableID`, paired `hello`, then the nonce from the Mac; the follow-up notification claims success only on `applied` (built). |
| Residual | The lock-screen gate is iOS's unlock and nothing more: a device unlocked recently enough for iOS to skip the prompt, or a known passcode, passes it. The alert title and body come through Apple and are shown before any check. |
| Phase | Built; timings to be measured on device. |

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
| Mitigations | `PromptText` rejects ESC and every C0/C1 control except newline and tab, since herdr sends prompts as bracketed paste and an embedded `ESC[201~` would end the paste (built, tested). `Key` is a closed list of nine keys (built). herdr privileged methods (`pane.send_text`, `pane.send_input`, `agent.start`, `plugin.*`, `server.*`) are not in the allowlist (built, tested); collied itself calls `pane.send_text` only for `agent.type_text`, with one line of `PromptText` (no newline, tab, ESC or other control) and only on a Claude Code question's free-text field. collied refuses `agent.prompt` while an agent is `blocked`, re-checked with `agent.get` just before the call; keys and typed text reach a blocked agent only under the conditions of [Keys and text on a blocked prompt](#keys-and-text-on-a-blocked-prompt) (built). Foreground agent check before sending (built). Draft sync: before a prompt to Claude Code, collied itself may send `down`, `ctrl+e`, `ctrl+u` and `backspace` to clear the input box, only when the box holds exactly the draft the phone named and no collapsed paste, image or mode other than `❯` (bash mode would run the prompt as a shell command); a box it cannot read refuses the prompt; the phone cannot choose these keys, `ctrl+c` and `ctrl+d` are never sent, authorization is re-checked before each batch, and the prompt is sent only once a re-read shows the box empty (built, tested). |
| Residual | herdr checks the foreground agent before queuing the text and the delayed Enter: if the agent exits in between, the shell receives the prompt. Closing it needs conditional input in herdr upstream. Text an agent reads from a repository can still steer the agent itself; that is outside collie. |
| Phase | Built; herdr conditional input: upstream proposal. |

### Approval TOCTOU

| | |
|---|---|
| Assets | The decision the user meant to make |
| Attack | The question under a still-`blocked` agent changes (new tool call, different command) between display and decision, so "approve" answers a different question. |
| Mitigations | Approval bound to a SHA-256 fingerprint of agent kind, matched rule id, `terminal_id`, agent session id and the dialog region with its cursor. Re-read before any key (same `state_change_seq` and fingerprint, else `superseded`). Arrows and Enter are separate calls: after the arrows collied re-reads and sends Enter only if the screen matches the target option's cursor fingerprint. Deny uses `esc` where the menu labels it `(esc)` and on the trust prompt, so it does not depend on the cursor. No key after the 6 s budget. Outcome is `applied` only if the agent leaves `blocked` within 3 s, otherwise `unconfirmed`; neither the app nor the lock-screen follow-up claims success otherwise. 10 minute expiry (all built). |
| Residual | The gap between the last re-read and the final key remains until herdr offers conditional input: a question that replaces the dialog inside it, with the same option layout, gets that key. |
| Phase | Built; herdr conditional input: upstream proposal. |

### Keys and text on a blocked prompt

| | |
|---|---|
| Assets | The decision on a blocked agent |
| Attack | A paired phone (or a stolen, unlocked one) uses `agent.send_keys` or `agent.type_text` to answer a permission prompt with Enter or `y`, skipping the approval's nonce, expiry, fingerprint and the app's LocalAuthentication. |
| Mitigations | Keys and text reach a blocked agent only when collied re-reads the screen at that moment (`agent.get`, `agent.explain`, then the detection text) and finds a Claude Code question form: agent kind `claude`, herdr rule `live_blocked_form`, and no numbered or `❯` line in the dialog whose label reads as a decision or starts with "yes", and no trust wording. The rule is an allowlist: a permission rule, an unknown rule, no rule (a hook-reported status) or another agent kind refuses, whatever the screen parses to, since a permission prompt that does not parse (wrapped option labels, Codex's `›` cursor, a `[y/n]` question) would otherwise read as decision-less. The line scan does not need a parsed menu, so a wrapped "Yes, and don't ask again" or the unnumbered trust prompt (which matches the same rule) still refuses. Plans refuse too: accepting one sets a permission mode. Where collied offers no decision there is nothing for a nonce to bind: before this the agent could only be answered on the Mac. Decision-less menus (questions, plans) can instead be answered through `approval.decide` with `choose`, which keeps the nonce, fingerprint, cursor check, expiry and LocalAuthentication, and is refused on permission prompts. Key allowlist unchanged; typed text is one line with no control character and is typed only into the question's "Type something." field: collied moves the cursor there and checks it arrived before typing, and sends Enter only once the field reads as the text with the rest of the dialog unchanged and the agent is still `blocked` with the same `state_change_seq`, so typed text never confirms another option; authorization re-checked before every herdr write; every send audited with its keys, text never logged (built, tested). |
| Residual | Keys and text skip the nonce and LocalAuthentication: on a question the phone's session alone is the gate, like `agent.prompt` on an idle agent. A permission dialog that Claude Code draws under the question rule with labels `classify` does not know (neither a decision nor "yes") would count as a question. A prompt that replaces the screen between collied's re-read and the herdr call gets the keys (the same gap as the approval TOCTOU, closed only by herdr conditional input). |
| Phase | Built |

### Attachment uploads

| | |
|---|---|
| Assets | The Mac's disk; files an agent may read; the contents of attached files |
| Attack | A paired phone (or a stolen, unlocked one) can now write files on the Mac. It tries to fill the disk, to write outside the attachments dir or over an existing file (a crafted name, `..`, a symlink), to plant an executable or a hidden dotfile, to tamper with or hijack another session's upload, or to get file content into the logs. |
| Mitigations | Same gate as `agent.prompt`: paired full session, re-checked on every frame and before `begin`, each chunk and the commit (built). Caps: 20 MiB per file (phone, decoder and collied), 32 KiB chunks in order and never past the declared size, 2 uploads in flight per session, idle uploads dropped after 60 s, 200 MiB stored (each file counted at least at its allocated blocks) including what is reserved by uploads in flight, at most 1000 entries under the root so a flood of one-byte uploads cannot use more disk than the byte cap suggests nor make the per-`begin` scan unbounded, chunk frames rate limited per `StableID` (128/s) and the other frames by the shared 20/s bucket (built). The phone's name is a hint only: `AttachmentName` rejects separators, `.`, `..`, controls, bidi and format characters, then collied keeps `[A-Za-z0-9._-]`, drops leading dots and dashes and caps the length. Each upload writes into a fresh random 0700 directory created with `create_dir` under a root that must be a 0700 directory owned by the user, not a symlink; the partial file is opened `create_new` with `O_NOFOLLOW`, mode 0600, and renamed inside that directory only, so no existing file is ever overwritten and nothing is executable. The SHA-256 and size are checked before the rename; a mismatch deletes the partial file. An upload id is 16 random bytes bound to the session and `StableID` that began it. Files expire after 24 h; partials of a previous run are removed at start. The audit log gets the sanitized name, size and a SHA-256 prefix, never content or base64; `ChunkData` is redacted in `Debug` (built). |
| Residual | Up to 200 MiB of phone-chosen bytes sit on the Mac for 24 h, and the path is put into a prompt: a file is as trusted as the phone that sent it, and an agent told to read it may follow instructions inside it (prompt injection through the file, as with any repository content). Contents are readable by anything running as the user and are not encrypted at rest beyond FileVault. The name, while sanitized, is still the phone's choice and shows up in the prompt and in the audit log. |
| Phase | Built |

### Apple sees push payload metadata

| | |
|---|---|
| Assets | Project and agent names, activity timing |
| Attack | Apple (or anyone with APNs access) reads payloads and metadata. |
| Mitigations | Alert title is the herdr agent name or the agent kind, never the terminal title or a title the pane's program sets; body is `Blocked in <workspace label>`; plus `approval_id`, `node_id` and `terminal_id` (thread and collapse id). The pending action (up to 600 chars of the command or question) travels only in `enc`: ChaCha20-Poly1305 under the phone's per-Mac notification key, fresh random nonce per send, bound to the `approval_id` as AAD, opened by ColliePush on the phone. The key is generated on the phone and reaches collied only inside the authenticated tailnet session. Never the snippet or nonce; details are fetched over the tailnet (built). Live Activity pushes go only to agents the user follows (off by default) and carry the status, when it started, the same title and workspace label as the alert and the pending approvals count, in clear: ActivityKit cannot decrypt a content state. While an approval routed to the activity is pending they also carry its `approvalId` in clear (as the alert does) and the context only as `enc`, sealed exactly like the alert's; the widget opens it on the phone, so Apple sees only ciphertext. No terminal text, terminal title or nonce (built). |
| Residual | Agent and workspace names, opaque ids, the ciphertext's length (so roughly the command's length), timing, frequency and the device token are visible to Apple. For a followed agent Apple also sees every status change (idle, working, blocked, done) with its time, and the activity's update token. A leaked notification key (from `push.json` or a Mac compromise, which already exposes far more; from the phone's Keychain) lets whoever also has the APNs payloads read past and future alert contexts for that Mac until the phone pairs again; it grants no decision, since the nonce never travels in a push. With a leaked APNs key as well, it also lets an attacker forge an alert whose body the NSE shows as genuine context. |
| Phase | Built. |
