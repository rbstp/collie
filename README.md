# collie

Watch and steer your [herdr](https://github.com/herdrdev/herdr) coding agents from your iPhone: see every agent and its status, read its terminal, prompt it, send keys, attach files, start new tasks, and approve or deny a blocked agent from the lock screen.

collie has two parts:

- **`collied`**, a Rust daemon on your Mac. It talks to herdr over herdr's local Unix socket and joins your tailnet on its own through an embedded [libtailscale](https://github.com/tailscale/libtailscale).
- **Collie**, the iOS app: SwiftUI over a Rust core (`collie-core`, exposed with UniFFI) that embeds its own Tailscale node too.

No Tailscale app is needed on either device, and no TCP port is opened outside the tailnet (the only socket on the real network is Tailscale's own WireGuard UDP socket).

```
 iPhone                                                  Mac
┌──────────────────────────┐   tailnet (WireGuard)   ┌──────────────────────────┐
│ Collie (SwiftUI)         │      TCP 8457, WS       │ collied                  │
│  └ collie-core (Rust)    │ ──────────────────────► │  ├ whois gate + pairing  │
│     └ libtailscale node  │                         │  ├ libtailscale node     │
│ ColliePush (NSE)         │ ◄─ APNs (encrypted ──── │  └ herdr client ─► herdr │
└──────────────────────────┘    approval context)    └──────────────────────────┘
```

## Features

- **Agents**: every herdr agent with its status (`idle`, `working`, `blocked`, `done`), grouped by Mac with blocked agents first, and each agent's workspace under its title. Long-press an agent, or use the agent screen's menu, to close its pane or workspace.
- **Terminal**: a live view of the last 240 lines of the agent's pane, rendered with libghostty-vt, with optional line wrapping and the MesloLGS NF font so Nerd Font glyphs match the Mac.
- **Prompt and keys**: send a prompt, or keys from the key strip (`esc ⏎ ← ↑ ↓ → ⇥ ⇧⇥ ^C`). "Focus on Mac" brings the agent's pane to the front in herdr.
- **Attachments**: up to 10 photos or files per prompt, uploaded over the tailnet and shown as pills; the agent receives their paths on the Mac.
- **New task**: start an agent in a new workspace from the phone.
- **Approvals**: when an agent blocks on a permission prompt, you get a push notification showing the command. Approve or deny from the lock screen (the iPhone must be unlocked first) or in the app (Face ID or the passcode for each decision).

## Security model

Security is the first requirement. The short version:

- **Tailnet only**: collied accepts connections only through its embedded Tailscale node, on one port. A test asserts it opens no kernel TCP listener.
- **Whois gate**: every connection is checked with Tailscale whois before the WebSocket upgrade. The peer must be untagged, not shared in from another tailnet, owned by the collie owner (`owner_user_id` in `collied.toml`, or else the user of the first phone you confirm at pairing), and a paired phone: its node `StableID` must be paired and still belong to the user it was paired with. Authorization is re-checked before every frame, and revoking a phone cuts its live sessions.
- **Pairing**: a QR code shown by `collied pair`, plus a local y/N confirmation on the Mac.
- **Approvals**: a nonce plus a fingerprint of the prompt on screen. collied moves the cursor, re-reads the screen, and presses Enter only if the fingerprint still matches; a prompt that changed is answered `superseded`. A small gap between that last re-read and Enter remains until herdr supports conditional input.
- **Push notifications**: the cleartext part of the push only says which agent is blocked and where. The command itself is end-to-end encrypted (ChaCha20-Poly1305, under a per-Mac key generated on the phone, kept in its Keychain and handed to collied over the tailnet). The phone's notification extension decrypts it, so Apple sees the command only as ciphertext. Apple still sees agent and workspace names, ids and timing.
- **Attachments**: uploads are size-capped (20 MiB per file, 200 MiB in total) and checksummed. Each is stored in a fresh random directory inside a private cache directory (0700 directories, 0600 non-executable files), under a sanitized copy of the file name, and deleted after 24 hours.
- **Secrets**: `collied apns import` stores the APNs signing key in the Mac's login Keychain. The item's ACL lets only the Developer ID-signed collied read it without a Keychain prompt.

Details, including what is not covered: [docs/threat-model.md](docs/threat-model.md) and [docs/architecture.md](docs/architecture.md).

## Status

| Phase | Scope | State |
|---|---|---|
| 0 | Skeleton, protocol, libtailscale build | done |
| 1 | Tailnet login, whois gate, pairing, agent list | done |
| 2 | Terminal, prompt, keys, new task | done |
| 3 | Lock-screen approvals with encrypted context, attachments | done |
| 4 | Live Activities and Dynamic Island for agents you follow | next |
| 5 | Claude Code hooks enrichment, audit viewer, multiple computers (macOS and Linux) | planned |
| 6 | Mutual TLS inside the tunnel, with a Secure Enclave key on the phone | planned |
| 7 | Improvements: a Mac menu bar icon (on/off, pairing, pending approvals, quit) | planned |

## Requirements

- An Apple silicon Mac running [herdr](https://github.com/herdrdev/herdr) 0.9.3 (the version collied is tested against). Intel Macs are not supported.
- An iPhone on iOS 26 or later.
- A Tailscale account whose policy file you can edit.
- To build: Rust (see `rust-toolchain.toml`), Go, Xcode 27 with an iPhone 18 Pro simulator, [just](https://github.com/casey/just), [XcodeGen](https://github.com/yonaskolb/XcodeGen), `cargo-deny`, and `jq` (for `just ios-run-device`). The libghostty-vt build script downloads its own pinned Zig.
- An Apple Developer account with a Developer ID Application certificate (collied is always signed), and an APNs key for push notifications.

The justfile and `Collie/project.yml` are set to the maintainer's Apple team ID and `dev.rbstp` bundle identifiers. Change them to your own before building.

## Getting started

1. **Tailnet policy**: add a `tag:collie-mac` tag owner and a grant from your user to `tag:collie-mac` on TCP 8457. The full policy, with tests, is in [docs/tailnet.md](docs/tailnet.md).
2. **Build and install collied** (signed with your Developer ID Application certificate, installed to `~/.cargo/bin`):

   ```sh
   git clone --recurse-submodules https://github.com/rbstp/collie
   cd collie
   just setup
   just collied-install
   collied login             # sign in as the tag owner; the node becomes tag:collie-mac
   collied service install   # launchd agent that runs collied at login
   collied doctor            # no line should say fail; warn lines for peers, apns and hooks are expected now
   ```

3. **Install the app** on a connected iPhone with Developer Mode on (`just ios-run-device`). Sign in to Tailscale inside the app with your own account.
4. **Pair**: run `collied pair` on the Mac, scan the QR code with the app, and confirm with `y` on the Mac.
5. **Push notifications** (optional): add an `[apns]` section to `~/Library/Application Support/collie/collied.toml`, import the key with `collied apns import AuthKey_<KEY_ID>.p8`, then check with `collied apns test`. The full steps are in [docs/release.md](docs/release.md).

List paired phones with `collied peers list`, revoke one with `collied peers revoke <label or StableID>`, and inspect the daemon with `collied status`.

`collied stop` turns collied off and keeps it off, across reboots, until `collied start`.

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
| `just collied-install` | Release build of collied, signed, installed to `~/.cargo/bin`; restarts the launchd agent if it is installed |

CI runs lint, the Rust tests, and the iOS simulator build and tests on every pull request and on pushes to `master`. Merging a pull request that changes code into `master` uploads a build to the maintainer's TestFlight ([docs/release.md](docs/release.md)).

## Repository layout

```
crates/
  protocol/        Wire types (allowlisted requests, validated newtypes), JSON Schema
  tailscale-sys/   libtailscale (pinned submodule + patches) built as a Go c-archive
  tailnet/         Safe Rust wrapper: node, listener, dial, whois
  collied/         The Mac daemon
  collie-core/     The phone's Rust core, exposed to Swift with UniFFI
  uniffi-bindgen/  UniFFI binding generator used by `just ios-framework`
  e2e/             End-to-end tests: phone core against collied over a local test tailnet
Collie/            iOS app (XcodeGen project.yml), ColliePush extension, GhosttyTerminal package
docs/              Architecture, threat model, tailnet setup, release, protocol schemas
scripts/           libghostty-vt xcframework build
```
