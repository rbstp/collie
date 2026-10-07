# collie

Watch and steer your [herdr](https://github.com/herdrdev/herdr) coding agents from your iPhone: see every agent and its status, read its terminal, prompt it, send keys, attach files, start new tasks, follow an agent in a Live Activity, and approve or deny a blocked agent from the lock screen.

collie has two parts:

- **`collied`**, a Rust daemon on your Mac or Linux machine. It talks to herdr over herdr's local Unix socket and joins your tailnet on its own through an embedded [libtailscale](https://github.com/tailscale/libtailscale).
- **Collie**, the iOS app: SwiftUI over a Rust core (`collie-core`, exposed with UniFFI) that embeds its own Tailscale node too.

No Tailscale app is needed on either device, and no TCP port is opened outside the tailnet (the only socket on the real network is Tailscale's own WireGuard UDP socket).

```
 iPhone                                                  Mac or Linux machine
┌──────────────────────────┐   tailnet (WireGuard)   ┌──────────────────────────┐
│ Collie (SwiftUI)         │      TCP 8457, WS       │ collied                  │
│  └ collie-core (Rust)    │ ──────────────────────► │  ├ whois gate + pairing  │
│     └ libtailscale node  │                         │  ├ libtailscale node     │
│ ColliePush (NSE)         │ ◄─ APNs (encrypted ──── │  └ herdr client ─► herdr │
│ CollieWidgets (Live Act.)│    approval context)    │                          │
│ CollieWatch (via phone)  │                         │                          │
└──────────────────────────┘                         └──────────────────────────┘
```

## Features

- **Machines**: pair several Macs and Linux machines. Each one sends its own pushes, and one that is asleep or off is shown as offline (gray) without slowing the others. Removing a machine in the app also revokes the phone on that machine when it is reachable.
- **Agents**: every herdr agent with its status (`idle`, `working`, `blocked`, `done`, as compact icons), grouped by machine (Mac or Linux) with blocked agents first, and each agent's workspace under its title. A small ring shows how much context a Claude Code or Codex agent has left, read from its transcript on the machine. Long-press an agent, or use the agent screen's menu, to close its pane or workspace. Long-press a card in the grid to star it: starred cards lead their machine's section at twice the height. The machine keeps the stars, so they survive a reinstall of the app and a reboot, until the pane closes or the machine is removed from the phone.
- **Inbox**: a third layout of the Agents tab, with agents grouped as Working, Done and Archived. Each shows its latest reply line, "You: <your last prompt>", the workspace, the agent kind, the machine and how long ago.
- **Plan usage**: the Agents tab's Usage view (an Agents | Usage switch above the grid, inbox or list) shows one card per machine with its Claude Code subscription's 5-hour and weekly limits: a bar of the percent used with a tick at the share of the window gone by, the percent, when each resets, whether usage runs slower or faster than that pace, and how long ago it was recorded (dimmed once more than 5 minutes old). It comes from Claude Code's status line, through a one-line tap (step 7 below). Codex records no limits for this plan, so it shows none.
- **Terminal**: a live view of the last 200 lines of the agent's pane, or 500 or 1000 set in Settings > Terminal > History (Claude Code's fullscreen mode, `"tui": "fullscreen"`, keeps its history out of the pane, so only one screen shows), rendered with libghostty-vt, with optional line wrapping at word boundaries (which rejoins Claude Code's paragraphs wrapped at the Mac's width and shrinks the padding of box-drawn panels) and the MesloLGS NF font so Nerd Font glyphs match the Mac. Long-press to select text, drag the handles to adjust, and copy (Universal Clipboard included), or open an http or https link the selection touches. The machine checks the pane four times a second, and once a second after 5 s without a change (the next change can then take up to 1 s to show) or while the iPhone is in Low Data Mode.
- **Gestures** (Settings > Terminal > Gestures): double-tap pastes into the prompt field, pinch sets the font size, swiping sideways switches to the previous or next agent (while lines wrap), and triple-tap can send Esc (off by default). Gestures stay off while text is selected.
- **Terminals** (off by default, turned on per machine in `collied.toml`): plain shell panes, listed under Terminals with the Ghostty icon. When an agent exits, its screen turns into the pane's shell. Face ID or the passcode unlocks a terminal for 5 minutes, then the command field runs one line at a time and the key strip sends `esc ⇥ ^C ← ↑ ↓ → ⏎`.
- **Prompt and keys**: send a prompt, or keys from the key strip (`esc ← ↑ ↓ → ⇥ ⇧⇥ ⏎ ⌃⏎`). A Claude Code prompt typed on the machine but not sent shows up in the phone's prompt field, and sending from the phone replaces it. A prompt not yet sent stays with its agent, on the phone only, when you go back, switch agents or close the app; its attachments for up to 23 hours, as the machine deletes them after 24. "Focus on <machine>" brings the agent's pane to the front in herdr. Settings > Prompt can keep the keyboard open after sending.
- **Dictation**: the mic button dictates into the prompt field on the phone itself (Apple's on-device speech models; audio never leaves the phone), in English (US) or French (Canada). Nothing is sent until you send it.
- **Attachments**: up to 10 photos or files per prompt, uploaded over the tailnet and shown as pills; the agent receives their paths on the machine.
- **New task**: start Claude Code, Codex or GitHub Copilot CLI in a new workspace from the phone. Approvals are for Claude Code only: a blocked Codex or Copilot agent shows up, and its prompt is answered in the terminal on the machine.
- **Approvals**: when an agent blocks on a permission prompt, you get a push notification naming the tool call (with the Claude Code hook, the exact command or file; otherwise as read from the screen). Approve or deny from the lock screen (the iPhone must be unlocked first) or in the app (Face ID or the passcode for each decision). Once the agent moves on, however it was answered, the notification is removed (best effort: iOS may delay or skip the background push that does it). In the app you can also add a note to an approval or a denial, send feedback on a plan, and answer Claude Code's question menus by picking an option or typing an answer. Notes, feedback and typed answers are one line, without control, bidi or invisible formatting characters (so an emoji sequence joined with U+200D, or a flag drawn with tag characters such as Scotland's, is refused too), so the text the agent receives is the text you see. The app checks a note before asking for Face ID.
- **Follow**: "Follow on Lock Screen" (the agent screen's menu, or a long press on the agent) shows the agent (up to 5) in a Live Activity and the Dynamic Island, with its status and how long it has been in it; the compact island shows the agent's kind icon (or its name) and, of several followed agents, the one that most needs you. When a followed agent blocks on a permission prompt, the activity shows the command with Approve and Deny buttons instead of a separate notification; other prompts arrive as a notification. Following is off by default; followed agents get a pin in the list.
- **Apple Watch**: pending approvals with the command and a countdown, the agents as in the inbox, and a complication with the 5-hour plan usage inside a ring of the time left until it resets, refreshed in the background about every 15 minutes when watchOS allows it. When it opens, the watch asks the iPhone for the pending approvals, also while the iPhone is locked; the agents are as the iPhone app last saw them. Approvals in the watch app are read-only unless Settings > Apple Watch > Decide from Apple Watch is on; then the watch app can approve, deny or answer a question menu. Tapping an approval alert on the watch, or its buttons, opens that approval in the watch app; it never decides from the alert.

## Security model

Security is the first requirement. The short version:

- **Tailnet only**: collied accepts connections only through its embedded Tailscale node, on one port. A test asserts it opens no kernel TCP listener.
- **Whois gate**: every connection is checked with Tailscale whois before the WebSocket upgrade. The peer must be untagged, not shared in from another tailnet, owned by the collie owner (`owner_user_id` in `collied.toml`, or else the user of the first phone you confirm at pairing), and a paired phone: its node `StableID` must be paired and still belong to the user it was paired with. Authorization is re-checked before every frame, and revoking a phone cuts its live sessions.
- **Mutual TLS**: inside the tunnel, the phone pins the machine's TLS key from the pairing QR, and collied pins the phone's key, which lives in the iPhone's Secure Enclave and never leaves it. A Tailscale node key copied off the phone, or a node injected by a compromised control plane, gets no session.
- **Pairing**: a QR code shown by `collied pair`, plus a local y/N confirmation on the computer running collied.
- **Approvals**: a nonce plus a fingerprint of the prompt on screen. collied moves the cursor, re-reads the screen, and presses Enter only if the fingerprint still matches; a prompt that changed is answered `superseded`. A small gap between that last re-read and Enter remains until herdr supports conditional input.
- **Apple Watch**: deciding from the watch is off by default, and turning it on needs Face ID or the passcode. A decision then needs the watch unlocked and on the wrist, and goes through the iPhone, which re-reads the setting on each one and only sends an answer it showed the watch, through the lock-screen decide path (nonce and fingerprint on the machine). It works while the iPhone is locked. There is no Approve always on the watch. The watch holds no keys and never talks to collied.
- **Terminals**: typing into a shell is command execution, so it is off unless `collied.toml` on that machine turns it on (never from the phone), its methods are a separate class in the allowlist, and every unlock is a grant collied verifies: a signature by a second Secure Enclave key on the phone, which signs only after Face ID or the passcode, for one terminal, one session and 5 minutes. collied re-reads the pane before every write and refuses one where an agent now runs. The audit log records each grant and command without its text.
- **Push notifications**: the cleartext part of the push only says which agent is blocked and where. The command itself is end-to-end encrypted (ChaCha20-Poly1305, under a per-machine key generated on the phone, kept in its Keychain and handed to collied over the tailnet). The phone's notification extension decrypts it, so Apple sees the command only as ciphertext. Apple still sees agent and workspace names, ids and timing.
- **Transcripts**: for the context ring and the inbox, collied reads the end of each live Claude Code or Codex agent's transcript on the machine. Only the percentage left, one line of the latest reply, one line of the latest prompt and the time of the last change go to the paired phone, over the same session as the terminal view. They are never logged or put in a push.
- **Plan usage**: `collied statusline` keeps only the two limits' percentages and reset times, and each session's context window size, in a 0600 `usage.json` in collied's data directory; the rest of the status line input is dropped. Only the percentages, the reset times and when they were recorded go to the paired phone, never to a log or a push.
- **Attachments**: uploads are size-capped (20 MiB per file, 200 MiB in total) and checksummed. Each is stored in a fresh random directory inside a private cache directory (0700 directories, 0600 non-executable files), under a sanitized copy of the file name, and deleted after 24 hours.
- **Secrets**: `collied apns import` stores the APNs signing key in the Mac's login Keychain. The item's ACL lets only the Developer ID-signed collied read it without a Keychain prompt. On Linux it stores the key as a systemd user credential: encrypted with the host key, and also sealed to the TPM2 when one is usable. Where `systemd-creds` cannot encrypt, it falls back to a 0600 file. This is weaker than the Keychain: any process running as your user can decrypt the credential, not only collied ([docs/threat-model.md](docs/threat-model.md)).

Details, including what is not covered: [docs/threat-model.md](docs/threat-model.md) and [docs/architecture.md](docs/architecture.md).

## Status

| Phase | Scope | State |
|---|---|---|
| 0 | Skeleton, protocol, libtailscale build | done |
| 1 | Tailnet login, whois gate, pairing, agent list | done |
| 2 | Terminal, prompt, keys, new task | done |
| 3 | Lock-screen approvals with encrypted context, attachments | done |
| 4 | Live Activities and Dynamic Island for agents you follow, approvals on the activity, question menus | done |
| 5 | Multiple computers (macOS and Linux), Claude Code hooks enrichment | done |
| 6 | Mutual TLS inside the tunnel, with a Secure Enclave key on the phone | done |
| 7 | Improvements: compact status icons, opening links, gestures, dictation, live terminal previews with starred cards, more scrollback, plain terminals, remaining context and a sessions inbox, plan usage, Codex and Copilot CLI agents, battery and reconnect fixes, an Apple Watch app (done); a Mac menu bar icon, a performance and battery check, and more (see the milestone) | in progress |

Outside the phases: an audit log viewer, and smaller fixes tracked as [issues](https://github.com/rbstp/collie/issues).

## Requirements

- An Apple silicon Mac running [herdr](https://github.com/herdrdev/herdr) 0.9.3 (the version collied is tested against). Intel Macs are not supported.
- Or an x86_64 Linux machine with a systemd user manager, running herdr 0.9.3. The encrypted APNs key needs systemd 256 or later (`systemd-creds --user`); older versions fall back to a 0600 file. collied is built and tested on Arch Linux. aarch64 Linux is mapped in the build but not built or tested.
- An iPhone on iOS 26 or later.
- Optional: an Apple Watch on watchOS 26 or later.
- A Tailscale account whose policy file you can edit.
- To build on macOS: Rust (see `rust-toolchain.toml`), Go, Xcode 27 with the watchOS platform and an iPhone 18 Pro simulator, [just](https://github.com/casey/just), [XcodeGen](https://github.com/yonaskolb/XcodeGen), `cargo-deny`, [cargo-nextest](https://nexte.st), and `jq` (for `just ios-run-device`). The libghostty-vt build script downloads its own pinned Zig.
- To build collied on Linux: Rust (see `rust-toolchain.toml`), Go 1.27.1 or later, a C compiler, libclang (for bindgen), [just](https://github.com/casey/just), `cargo-deny` and [cargo-nextest](https://nexte.st).
- An Apple Developer account with a Developer ID Application certificate (collied is always signed on macOS; it is not signed on Linux), and an APNs key for push notifications.

The justfile and `Collie/project.yml` are set to the maintainer's Apple team ID and `dev.rbstp` bundle identifiers. Change them to your own before building.

## Getting started

1. **Tailnet policy**: add a `tag:collie-mac` tag owner and a grant from your user to `tag:collie-mac` on TCP 8457. For a Linux machine, add a `tag:collie-linux` tag owner and the same TCP 8457 grant to `tag:collie-linux`. The full policy, with tests, is in [docs/tailnet.md](docs/tailnet.md).
2. **Build and install collied** on macOS (signed with your Developer ID Application certificate, installed to `~/.cargo/bin`):

   ```sh
   git clone --recurse-submodules https://github.com/rbstp/collie
   cd collie
   just setup
   just collied-install
   collied login             # sign in as the tag owner; the node becomes tag:collie-mac
   collied service install   # launchd agent that runs collied at login
   collied doctor            # no line should say fail; warn lines for peers, apns, hooks and plan usage are expected now
   ```

   On Linux (not signed, installed to `~/.cargo/bin`):

   ```sh
   git clone --recurse-submodules https://github.com/rbstp/collie
   cd collie
   just setup
   just collied-install
   collied login             # sign in as the tag owner; the node becomes tag:collie-linux
   collied service install   # systemd user unit that runs collied in your user session
   collied doctor            # no line should say fail; warn lines for peers, apns, hooks and plan usage are expected now
   ```

   On Linux, collied's data directory is `$XDG_DATA_HOME/collie`, else `~/.local/share/collie`. It holds `collied.toml`, the node state, paired phones and the audit log. Logs go to the user journal: `journalctl --user -u collied`. A user unit runs while you have a session. `loginctl enable-linger` (optional) keeps it running without a login.

3. **Install the app** on a connected iPhone with Developer Mode on (`just ios-run-device`). The watch app installs with it; signing it needs the paired Apple Watch registered to your team once first (connect the watch in Xcode's Devices and Simulators window, or add its UDID in the developer portal). Sign in to Tailscale inside the app with your own account.
4. **Pair**: run `collied pair` on the computer, scan the QR code in the app (Machines, Add machine), and confirm with `y` on the computer. A phone paired before mutual TLS (Phase 6) pairs again the same way once both sides are updated.
5. **Push notifications** (optional): add an `[apns]` section to `collied.toml` (`~/Library/Application Support/collie/collied.toml` on macOS, in the data directory on Linux), import the key with `collied apns import AuthKey_<KEY_ID>.p8`, then check with `collied apns test`. Each machine uses its own APNs key (its own key ID, revoked on its own) and sends its own pushes. On Linux, `collied apns import` encrypts the key into a systemd user credential (also sealed to the TPM2 when one is usable), with a 0600 file as the fallback where `systemd-creds` cannot encrypt. `collied doctor` reports which one is in use and, for a credential, its seal. The full steps are in [docs/release.md](docs/release.md).

6. **Claude Code hook** (optional): add `collied hook` as a `PermissionRequest` hook in `~/.claude/settings.json`, so approvals name the exact tool call. It only reports the call to collied; Claude Code's dialog is unchanged. `collied doctor` checks it.

   ```json
   {
     "hooks": {
       "PermissionRequest": [
         { "hooks": [{ "type": "command", "command": "~/.cargo/bin/collied hook", "timeout": 5 }] }
       ]
     }
   }
   ```

7. **Plan usage** (optional): Claude Code hands the plan's limits only to the status line command, on its stdin. Add this line to your status line script, after it reads its input into `input` (for example `input=$(cat)`):

   ```sh
   printf '%s' "$input" | ~/.cargo/bin/collied statusline >/dev/null 2>&1 &
   ```

   It records the limits and the session's context window (so the context ring uses the exact window instead of the model's) and prints nothing, so the status line is unchanged. A `statusLine.command` without a script can call one that does `input=$(cat)`, the line above, then the old command with `printf '%s' "$input" |`. The limits exist for claude.ai Pro and Max plans only, after the session's first reply. `collied doctor` reports when the tap last recorded them.

8. **Terminals** (optional): to use plain shell panes from the phone, add this to `collied.toml` and restart collied (`collied stop`, then `collied start`):

   ```toml
   [terminals]
   enabled = true
   ```

   The phone must have a passcode, and must be paired after both sides are updated: the pairing records the phone's terminal key. A phone paired before shows "Pair this phone again to use terminals on this machine".

List paired phones with `collied peers list`, revoke one with `collied peers revoke <label or StableID>` (removing the machine in the app does the same when the machine is reachable), and inspect the daemon with `collied status` (its tags and the herdr agents it sees).

`collied stop` turns collied off and keeps it off, across reboots, until `collied start`. This is the same with the launchd agent and the systemd user unit.

## Development

| Command | What it does |
|---|---|
| `just setup` | Fetches the submodules |
| `just fmt` | `cargo fmt` |
| `just lint` | `cargo fmt --check`, clippy with `-D warnings`, `cargo deny` |
| `just test` | All Rust tests with cargo-nextest, including end-to-end tests over a local test tailnet |
| `just schema` | Regenerates the protocol JSON Schemas in `docs/protocol/` |
| `just ios-framework` | Builds the `CollieCore` xcframework and UniFFI bindings |
| `just ios-ghostty` | Builds libghostty-vt from its pinned Ghostty commit (skipped when already built) |
| `just ios-project` | Builds libghostty-vt and generates the Xcode project with XcodeGen |
| `just ios-test` | GhosttyTerminal package tests on macOS, then the iOS unit tests on the simulator |
| `just ios-build-sim` | Simulator build |
| `just ios-run-device` | Builds, installs and launches on a connected iPhone |
| `just collied-install` | Release build of collied, installed to `~/.cargo/bin`. On macOS it is signed and restarts the launchd agent if it is installed. On Linux it is not signed and restarts the systemd user unit if it is active |

CI runs on macOS: lint, the Rust tests, the GhosttyTerminal package tests, and the iOS simulator build and tests, on every pull request and on pushes to `master`. Merging a pull request that changes code into `master` uploads a build to the maintainer's TestFlight ([docs/release.md](docs/release.md)).

## Repository layout

```
crates/
  protocol/        Wire types (allowlisted requests, validated newtypes), JSON Schema
  tailscale-sys/   libtailscale (pinned submodule + patches) built as a Go c-archive
  tailnet/         Safe Rust wrapper: node, listener, dial, whois
  collied/         The daemon (macOS and Linux)
  tls/             Mutual TLS: raw public key verifiers and key pins (collie-tls)
  collie-core/     The phone's Rust core, exposed to Swift with UniFFI
  uniffi-bindgen/  UniFFI binding generator used by `just ios-framework`
  e2e/             End-to-end tests: phone core against collied over a local test tailnet
Collie/            iOS app (XcodeGen project.yml), ColliePush and CollieWidgets extensions, CollieWatch app and its CollieWatchWidgets complication, GhosttyTerminal package
docs/              Architecture, threat model, tailnet setup, release, protocol schemas
scripts/           libghostty-vt xcframework build
```
