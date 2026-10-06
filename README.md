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
└──────────────────────────┘                         └──────────────────────────┘
```

## Features

- **Machines**: pair several Macs and Linux machines. Each one sends its own pushes, and one that is asleep or off is shown as offline (gray) without slowing the others. Removing a machine in the app also revokes the phone on that machine when it is reachable.
- **Agents**: every herdr agent with its status (`idle`, `working`, `blocked`, `done`, as compact icons), grouped by machine (Mac or Linux) with blocked agents first, and each agent's workspace under its title. Long-press an agent, or use the agent screen's menu, to close its pane or workspace.
- **Terminal**: a live view of the last 1000 lines of the agent's pane (Claude Code's fullscreen mode, `"tui": "fullscreen"`, keeps its history out of the pane, so only one screen shows), rendered with libghostty-vt, with optional line wrapping and the MesloLGS NF font so Nerd Font glyphs match the Mac. Long-press to select text, drag the handles to adjust, and copy (Universal Clipboard included), or open an http or https link the selection touches.
- **Gestures** (Settings > Gestures): double-tap pastes into the prompt field, pinch sets the font size, swiping sideways switches to the previous or next agent, and triple-tap can send Esc (off by default). Gestures stay off while text is selected.
- **Prompt and keys**: send a prompt, or keys from the key strip (`esc ← ↑ ↓ → ⇥ ⇧⇥ ⏎ ⌃⏎`). A Claude Code prompt typed on the machine but not sent shows up in the phone's prompt field, and sending from the phone replaces it. "Focus on <machine>" brings the agent's pane to the front in herdr.
- **Dictation**: the mic button dictates into the prompt field on the phone itself (Apple's on-device speech models; audio never leaves the phone), in English (US) or French (Canada). Nothing is sent until you send it.
- **Attachments**: up to 10 photos or files per prompt, uploaded over the tailnet and shown as pills; the agent receives their paths on the machine.
- **New task**: start Claude Code, Codex or GitHub Copilot CLI in a new workspace from the phone. Approvals are for Claude Code only: a blocked Codex or Copilot agent shows up, and its prompt is answered in the terminal on the machine.
- **Approvals**: when an agent blocks on a permission prompt, you get a push notification naming the tool call (with the Claude Code hook, the exact command or file; otherwise as read from the screen). Approve or deny from the lock screen (the iPhone must be unlocked first) or in the app (Face ID or the passcode for each decision). In the app you can also add a note to an approval or a denial, send feedback on a plan, and answer Claude Code's question menus by picking an option or typing an answer.
- **Follow**: "Follow on Lock Screen" (agent menu or long-press in the list) shows the agent (up to 5) in a Live Activity and the Dynamic Island, with its status and how long it has been in it. When a followed agent blocks on a permission prompt, the activity shows the command with Approve and Deny buttons instead of a separate notification; other prompts arrive as a notification. Following is off by default; followed agents get a pin in the list.

## Security model

Security is the first requirement. The short version:

- **Tailnet only**: collied accepts connections only through its embedded Tailscale node, on one port. A test asserts it opens no kernel TCP listener.
- **Whois gate**: every connection is checked with Tailscale whois before the WebSocket upgrade. The peer must be untagged, not shared in from another tailnet, owned by the collie owner (`owner_user_id` in `collied.toml`, or else the user of the first phone you confirm at pairing), and a paired phone: its node `StableID` must be paired and still belong to the user it was paired with. Authorization is re-checked before every frame, and revoking a phone cuts its live sessions.
- **Mutual TLS**: inside the tunnel, the phone pins the machine's TLS key from the pairing QR, and collied pins the phone's key, which lives in the iPhone's Secure Enclave and never leaves it. A Tailscale node key copied off the phone, or a node injected by a compromised control plane, gets no session.
- **Pairing**: a QR code shown by `collied pair`, plus a local y/N confirmation on the computer running collied.
- **Approvals**: a nonce plus a fingerprint of the prompt on screen. collied moves the cursor, re-reads the screen, and presses Enter only if the fingerprint still matches; a prompt that changed is answered `superseded`. A small gap between that last re-read and Enter remains until herdr supports conditional input.
- **Push notifications**: the cleartext part of the push only says which agent is blocked and where. The command itself is end-to-end encrypted (ChaCha20-Poly1305, under a per-machine key generated on the phone, kept in its Keychain and handed to collied over the tailnet). The phone's notification extension decrypts it, so Apple sees the command only as ciphertext. Apple still sees agent and workspace names, ids and timing.
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
| 7 | Improvements: compact status icons, opening links, gestures, dictation, live terminal previews with starred cards, more scrollback, Codex and Copilot CLI agents (done); battery, reconnects, a Mac menu bar icon, remaining context and a sessions inbox, an Apple Watch app, and more (see the milestone) | in progress |

Outside the phases: an audit log viewer, and smaller fixes tracked as [issues](https://github.com/rbstp/collie/issues).

## Requirements

- An Apple silicon Mac running [herdr](https://github.com/herdrdev/herdr) 0.9.3 (the version collied is tested against). Intel Macs are not supported.
- Or an x86_64 Linux machine with a systemd user manager, running herdr 0.9.3. The encrypted APNs key needs systemd 256 or later (`systemd-creds --user`); older versions fall back to a 0600 file. collied is built and tested on Arch Linux. aarch64 Linux is mapped in the build but not built or tested.
- An iPhone on iOS 26 or later.
- A Tailscale account whose policy file you can edit.
- To build on macOS: Rust (see `rust-toolchain.toml`), Go, Xcode 27 with an iPhone 18 Pro simulator, [just](https://github.com/casey/just), [XcodeGen](https://github.com/yonaskolb/XcodeGen), `cargo-deny`, and `jq` (for `just ios-run-device`). The libghostty-vt build script downloads its own pinned Zig.
- To build collied on Linux: Rust (see `rust-toolchain.toml`), Go 1.27.1 or later, a C compiler, libclang (for bindgen), [just](https://github.com/casey/just) and `cargo-deny`.
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
   collied doctor            # no line should say fail; warn lines for peers, apns and hooks are expected now
   ```

   On Linux (not signed, installed to `~/.cargo/bin`):

   ```sh
   git clone --recurse-submodules https://github.com/rbstp/collie
   cd collie
   just setup
   just collied-install
   collied login             # sign in as the tag owner; the node becomes tag:collie-linux
   collied service install   # systemd user unit that runs collied in your user session
   collied doctor            # no line should say fail; warn lines for peers, apns and hooks are expected now
   ```

   On Linux, collied's data directory is `$XDG_DATA_HOME/collie`, else `~/.local/share/collie`. It holds `collied.toml`, the node state, paired phones and the audit log. Logs go to the user journal: `journalctl --user -u collied`. A user unit runs while you have a session. `loginctl enable-linger` (optional) keeps it running without a login.

3. **Install the app** on a connected iPhone with Developer Mode on (`just ios-run-device`). Sign in to Tailscale inside the app with your own account.
4. **Pair**: run `collied pair` on the computer, scan the QR code with the app, and confirm with `y` on the computer. A phone paired before mutual TLS (Phase 6) pairs again the same way once both sides are updated.
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

List paired phones with `collied peers list`, revoke one with `collied peers revoke <label or StableID>` (removing the machine in the app does the same when the machine is reachable), and inspect the daemon with `collied status` (its tags and the herdr agents it sees).

`collied stop` turns collied off and keeps it off, across reboots, until `collied start`. This is the same with the launchd agent and the systemd user unit.

## Development

| Command | What it does |
|---|---|
| `just lint` | `cargo fmt --check`, clippy with `-D warnings`, `cargo deny` |
| `just test` | All Rust tests, including end-to-end tests over a local test tailnet |
| `just schema` | Regenerates the protocol JSON Schemas in `docs/protocol/` |
| `just ios-framework` | Builds the `CollieCore` xcframework and UniFFI bindings |
| `just ios-project` | Builds libghostty-vt and generates the Xcode project with XcodeGen |
| `just ios-test` | iOS unit tests on the simulator |
| `just ios-build-sim` | Simulator build |
| `just ios-run-device` | Builds, installs and launches on a connected iPhone |
| `just collied-install` | Release build of collied, installed to `~/.cargo/bin`. On macOS it is signed and restarts the launchd agent if it is installed. On Linux it is not signed and restarts the systemd user unit if it is active |

CI runs on macOS: lint, the Rust tests, and the iOS simulator build and tests, on every pull request and on pushes to `master`. Merging a pull request that changes code into `master` uploads a build to the maintainer's TestFlight ([docs/release.md](docs/release.md)).

## Repository layout

```
crates/
  protocol/        Wire types (allowlisted requests, validated newtypes), JSON Schema
  tailscale-sys/   libtailscale (pinned submodule + patches) built as a Go c-archive
  tailnet/         Safe Rust wrapper: node, listener, dial, whois
  collied/         The daemon (macOS and Linux)
  collie-core/     The phone's Rust core, exposed to Swift with UniFFI
  uniffi-bindgen/  UniFFI binding generator used by `just ios-framework`
  e2e/             End-to-end tests: phone core against collied over a local test tailnet
Collie/            iOS app (XcodeGen project.yml), ColliePush and CollieWidgets extensions, GhosttyTerminal package
docs/              Architecture, threat model, tailnet setup, release, protocol schemas
scripts/           libghostty-vt xcframework build
```
