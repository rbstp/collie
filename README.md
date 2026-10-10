# collie

[![ci](https://github.com/rbstp/collie/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/rbstp/collie/actions/workflows/ci.yml)

collie is a remote control for [herdr](https://github.com/herdrdev/herdr) coding agents. A small daemon, `collied`, runs next to herdr on your Mac or Linux machine, and the Collie app on your iPhone and Apple Watch connects to it over your own tailnet. From the phone you can see every agent and its status, read its terminal, prompt it, send keys, attach files, start new tasks, follow an agent in a Live Activity, and approve or deny a blocked agent from the lock screen. A menu bar app on macOS and an Omarchy bar widget on Linux control the daemon from the desktop.

- Every herdr agent across several machines, with live terminal views and a sessions inbox
- Lock-screen, Live Activity and Apple Watch approvals, with the command end-to-end encrypted in the push
- Claude Code, Codex and GitHub Copilot CLI agents, with remaining context and plan usage
- Tailnet only: an embedded Tailscale node on each side, no Tailscale app and no open TCP port on the real network
- Mutual TLS pinned at pairing, with the phone's key in the Secure Enclave

## Overview

collie has these parts:

| Component | Platform | Role |
|---|---|---|
| `collied` | macOS (Apple silicon), Linux (x86_64) | Rust daemon. Talks to herdr over its local Unix socket and joins your tailnet on its own through an embedded [libtailscale](https://github.com/tailscale/libtailscale). |
| Collie | iOS | SwiftUI app over a Rust core (`collie-core`, exposed with UniFFI) that embeds its own Tailscale node. Includes the ColliePush notification extension and the CollieWidgets Live Activities. |
| CollieWatch | watchOS | Apple Watch app and usage complication. It goes through the iPhone and never talks to `collied`. |
| CollieBar | macOS | Menu bar app, a client of `collied`'s local control socket. |
| CollieTray | Linux ([Omarchy](https://omarchy.org)) | Bar widget with the same menu as CollieBar. |

No Tailscale app is needed on either device, and no TCP port is opened outside the tailnet: the only socket on the real network is Tailscale's own WireGuard UDP socket.

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

Design details and limits are in [docs/architecture.md](docs/architecture.md).

### Agents

- **Machines**: pair several Macs and Linux machines. Each sends its own pushes, and one that is asleep or off shows as offline without slowing the others. Removing a machine in the app also revokes the phone on that machine when it is reachable.
- **Agent list**: every herdr agent with its status (`idle`, `working`, `blocked`, `done`) as compact icons, grouped by machine with blocked agents first. The Agents tab has three layouts: Grid (the default), List and Inbox. Long-press an agent, or use the agent screen's menu, to close its pane or workspace.
- **Grid**: live previews of each agent. Claude Code, Codex and Copilot CLI cards keep the agent's output and drop its input box and status lines; a blocked agent's card shows its whole screen. Long-press a card to star it: starred cards lead their machine's section at twice the height. Stars are kept on the machine until the pane closes or the machine is removed.
- **Inbox**: search agents across machines and filter by Needs attention, Working or Inactive. Working includes blocked agents, Done shows recent completions, and Inactive includes idle agents and completions older than 24 hours. Each row shows the latest reply, your last prompt, the workspace, agent kind, machine and elapsed time.
- **Remaining context**: a small ring shows how much context a Claude Code or Codex agent has left, read from its transcript on the machine.
- **Plan usage**: the Usage view (the Agents | Usage switch, or swipe left) shows one card per machine. Claude Code shows its 5-hour and weekly limits from the status line tap ([step 7](#getting-started)). Codex shows the monthly used and allowed credits and reset date for a credit-metered workspace, read through the local Codex app-server every five minutes. The Codex row clears after sign-out or a switch to an account without a monthly limit.

### Terminal and input

- **Terminal view**: the last 200 lines of the agent's pane (500 or 1000 in Settings > Terminal > History), rendered with libghostty-vt in the MesloLGS NF font, with optional word wrapping. Long-press to select and copy text (Universal Clipboard included) or open an http or https link. Claude Code's fullscreen mode (`"tui": "fullscreen"`) keeps its history out of the pane, so only one screen shows. The machine checks the pane four times a second, and once a second after 5 s without a change or while the iPhone is in Low Data Mode.
- **Gestures** (Settings > Terminal > Gestures): double-tap pastes into the prompt field, pinch sets the font size, swiping sideways switches agents while lines wrap, and triple-tap can send Esc (off by default).
- **Prompt and keys**: send a prompt, or keys from the key strip (`esc ← ↑ ↓ → ⇥ ⇧⇥ ⏎ ⌃⏎`). Codex sessions also show ⇧← at all times; when a question is queued, it is the only active key until the question opens. "Focus on <machine>" brings the agent's pane to the front in herdr. Settings > Prompt can keep the keyboard open after sending.
- **Draft sync**: a Claude Code prompt typed on the machine but not sent shows up in the phone's prompt field, and sending from the phone replaces it. An unsent prompt stays with its agent on the phone, and its attachments for up to 23 hours. Text from the mic button or a double-tap paste stays on the phone until you send it.
- **Slash commands**: while a Claude Code prompt on the phone starts with `/`, its command is mirrored into the agent's input box so Claude Code's command menu appears live. ↑ ↓ highlight a command, ⇥ takes it into the phone's prompt, and only Send runs it; ⏎ and ⌃⏎ are off while the command shows.
- **Jump to bottom**: when Claude Code's fullscreen transcript is scrolled up on the machine, a Jump to bottom button brings it back down. Until then the machine refuses keys and prompts, so a permission prompt scrolled out of view cannot be answered unseen.
- **Claude Code notices**: the options of a notice above the input box (the "Heads up" tip, its explanation and feedback row, the session rating question, or a plugin that draws its options the same way) show as buttons. A tap sends that one digit, without Enter, only while the screen still shows that option above an empty input box. Chat in main session and Turn off suggestions only fill the input box; nothing is sent until you send it. The tip feedback options send Anthropic a feedback signal about the tip, never its text or the transcript. Follow-ups to a rating are answered on the machine.
- **Dictation**: the mic button dictates into the prompt field with Apple's on-device speech models, in English (US) or French (Canada). Audio never leaves the phone.
- **Attachments**: up to 10 photos or files per prompt, uploaded over the tailnet and shown as removable thumbnails. The agent receives their paths on the machine; files are deleted after 24 hours.
- **New task**: start Claude Code, Codex or GitHub Copilot CLI in a new workspace. Each machine can have a base folder (Machines > the machine > Base folder, a full path such as `/Users/you/git`, since `~` is not expanded) inside one of `collied`'s task roots (`[tasks] roots`, the home folder by default). Typing a name then means that folder inside the base, with matching folders offered as you type, and Browse lists the base's folders. Recent folders and absolute paths also work. New folder creates one empty folder (no `git init`) in the base, or in a task root when no base is set, and starts the agent there; it stays if the agent fails to start.
- **Git worktree tasks**: in New task, choose Git worktree and enter the session checkout inside a task root. Collie lists its worktrees automatically. Create generates a branch name unless you enter one, then places the new checkout under that checkout's `.worktree/` directory. Slashes in a custom branch name become hyphens in the checkout folder name. Open selects an existing checkout from the list. Check the branch and path shown before Start. Collie starts the agent in herdr's returned root pane. An occupied pane is refused. If agent startup fails, the checkout and workspace stay in place and the error identifies them. Archive closes the linked workspace and removes its checkout, including uncommitted changes, then runs `gh poi` to clean up merged branches and worktrees in the repository. For a regular folder, Archive closes only its pane and leaves the folder in place; it runs `gh poi` when the repository is inside a task root. The result reports any skipped or failed cleanup.
- **Terminals** (off by default, enabled per machine in `collied.toml`): plain shell panes, listed under Terminals. When an agent exits, its screen becomes the pane's shell. Face ID or the passcode unlocks a terminal for 5 minutes; the command field then runs one line at a time and the key strip sends `esc ⇥ ^C ← ↑ ↓ → ⏎`.

### Approvals and notifications

- **Approvals**: when an agent blocks on a permission prompt, a push notification names the tool call (the exact command or file with the Claude Code hook, otherwise as read from the screen). Approve or deny from the lock screen (after unlocking the iPhone) or in the app, with Face ID or the passcode for each decision. The notification is removed once the agent moves on, on a best-effort basis.
- **In-app answers**: add a note to an approval or a denial, send feedback on a plan, and answer Claude Code's question menus by picking an option or typing an answer. Codex queued questions can also be answered in the session, including questions that only ask you to type. Notes and answers are one line, without control, bidi or invisible formatting characters, so the agent receives exactly the text you see. Claude Code's folder trust prompt is answered the same way: Approve trusts the folder, Deny ends the session. Settings pickers such as `/effort` and `/config` raise no notification.
- **Other agents**: Approve and Deny from the phone are for Claude Code, whose permission hook ties the decision to the exact tool call. Codex and Copilot CLI offer no such hook, and reading their permission prompts off the screen could approve the wrong thing, so those prompts are answered in the terminal on the machine ([details](docs/architecture.md)). A blocked Codex or Copilot agent still sends a notification, without Approve or Deny and never on a Live Activity.
- **Done alerts**: when an agent finishes a turn that took 30 seconds or more, a notification with its name and workspace, one per agent and none while it is on screen. A tap opens the agent. Settings > Notifications > Notify when an agent finishes turns it off.
- **Live Activities**: "Follow on Lock Screen" (the agent screen's menu, or a long press on the agent) shows up to 5 agents in a Live Activity and the Dynamic Island, with their status and how long they have been in it. When a followed agent blocks on a permission prompt, the activity shows the command with Approve and Deny buttons; other prompts arrive as a notification. Following is off by default.

### Apple Watch

- Pending approvals with the command and a countdown, the agents as in the inbox, and a complication with the 5-hour plan usage, refreshed about every 15 minutes when watchOS allows it.
- Approvals are read-only unless Settings > Apple Watch > Decide from Apple Watch is on; the watch app can then approve, deny or answer a question menu. Decisions are made in the watch app, never from the alert itself.
- The watch asks the iPhone for pending approvals when it opens, at most once a minute unless an approval alert opened it, and works while the iPhone is locked. The agents are as the iPhone app last saw them.

### Desktop companions

- **Menu bar app** (macOS): the icon is bright while `collied` runs, dim when it is off, with a dot while an approval is pending. The menu turns `collied` off and on, pairs a phone, lists the paired phones, opens the audit log, and quits (which stops `collied`).
- **Bar widget** (Linux, Omarchy): the same icon states and menu in the Omarchy shell. Quit stops `collied` and takes the widget off the bar.

## Security

Security is the first requirement. In short:

- **Tailnet only**: `collied` accepts connections only through its embedded Tailscale node, on one port. A test asserts it opens no kernel TCP listener.
- **Whois gate**: every connection is checked with Tailscale whois before the WebSocket upgrade. The peer must be untagged, not shared in from another tailnet, owned by the collie owner (`owner_user_id` in `collied.toml`, otherwise the user of the first phone you confirm at pairing), and a paired phone whose node `StableID` still belongs to the user it was paired with. Authorization is re-checked before every frame, and revoking a phone cuts its live sessions.
- **Mutual TLS**: inside the tunnel, the phone pins the machine's TLS key from the pairing QR, and `collied` pins the phone's key, which lives in the iPhone's Secure Enclave. A copied Tailscale node key, or a node injected by a compromised control plane, gets no session.
- **Pairing**: a QR code shown by `collied pair`, plus a local y/N confirmation on the machine running `collied`.
- **Approvals**: a nonce plus a fingerprint of the prompt on screen. `collied` re-reads the screen and presses Enter only if the fingerprint still matches; a prompt that changed is answered `superseded`. A small gap between that last re-read and Enter remains until herdr supports conditional input.
- **Apple Watch**: deciding from the watch is off by default and turning it on needs Face ID or the passcode. A decision then needs the watch unlocked and on the wrist, and goes through the iPhone, which re-checks the setting and only sends an answer it showed the watch, on the same nonce and fingerprint path. There is no Approve always on the watch, and the watch holds no keys.
- **Terminals**: shell input is command execution, so it is off unless `collied.toml` on that machine turns it on (never from the phone). Every unlock is a grant signed by a second Secure Enclave key after Face ID or the passcode, for one terminal, one session and 5 minutes. `collied` refuses to write to a pane where an agent now runs, and the audit log records each grant and command without its text.
- **Push notifications**: the cleartext part only says which agent is blocked and where. The command is end-to-end encrypted (ChaCha20-Poly1305) under a per-machine key generated on the phone, and decrypted by the phone's notification extension. Apple still sees agent and workspace names, ids and timing.
- **Transcripts and plan usage**: only the context percentage, one line of the latest reply and prompt, plan usage and its timestamps go to the paired phone, never to a log or a push. `collied statusline` keeps only Claude's limits and each session's context window size in a 0600 `usage.json`; the rest of the status line input is dropped. The daemon also records Codex's monthly used and allowed credits and reset date there; its OAuth token stays with Codex.
- **Attachments**: size-capped (20 MiB per file, 200 MiB in total), checksummed, stored under sanitized names in fresh random directories of a private cache (0700 directories, 0600 non-executable files), and deleted after 24 hours.
- **Secrets**: on macOS, the APNs signing key is stored in the login Keychain, readable without a prompt only by the Developer ID-signed `collied`. On Linux it is a systemd user credential, encrypted with the host key and sealed to the TPM2 when one is usable, with a 0600 file as the fallback where `systemd-creds` cannot encrypt. This is weaker than the Keychain: any process running as your user can decrypt it.

What is and is not covered: [docs/threat-model.md](docs/threat-model.md).

## Requirements

**To run:**

- An Apple silicon Mac, or an x86_64 Linux machine with a systemd user manager, running [herdr](https://github.com/herdrdev/herdr) 0.9.3 (the version `collied` is tested against). Intel Macs are not supported. On Linux, `collied` is built and tested on Arch Linux; aarch64 is mapped in the build but not built or tested, and the encrypted APNs key needs systemd 256 or later (`systemd-creds --user`), older versions fall back to a 0600 file.
- An iPhone on iOS 26 or later, and optionally an Apple Watch on watchOS 26 or later.
- A Tailscale account whose policy file you can edit.
- An Apple Developer account with a Developer ID Application certificate (`collied` is always signed on macOS, never on Linux) and an APNs key for push notifications.
- Optional, for the Linux bar widget: Omarchy 4 (its shell runs on Quickshell 0.3) with `qrencode`, which Omarchy installs.

**To build on macOS:** Rust (see `rust-toolchain.toml`), Go, Xcode 27 with the watchOS platform and an iPhone 18 Pro simulator, [just](https://github.com/casey/just), [XcodeGen](https://github.com/yonaskolb/XcodeGen), `cargo-deny`, [cargo-nextest](https://nexte.st), and `jq` (for `just ios-run-device`). The libghostty-vt build script downloads its own pinned Zig.

**To build `collied` on Linux:** Rust (see `rust-toolchain.toml`), Go 1.27.2 or later, a C compiler, libclang (for bindgen), [just](https://github.com/casey/just), `cargo-deny` and [cargo-nextest](https://nexte.st). For `just tray-test`, also Node.

The justfile and `Collie/project.yml` are set to the maintainer's Apple team ID and `dev.rbstp` bundle identifiers. Change them to your own before building.

## Getting started

1. **Tailnet policy**: add a `tag:collie-mac` tag owner and a grant from your user to `tag:collie-mac` on TCP 8457. For a Linux machine, add a `tag:collie-linux` tag owner and the same grant to `tag:collie-linux`. The full policy, with tests, is in [docs/tailnet.md](docs/tailnet.md). `collied setup` prints the entries to add when the tag owner or the grant is missing.

2. **Build, install and set up `collied`**. It is installed to `~/.cargo/bin`, signed with your Developer ID Application certificate on macOS and unsigned on Linux:

   ```sh
   git clone --recurse-submodules https://github.com/rbstp/collie
   cd collie
   just setup
   just collied-install
   collied setup
   ```

   `collied setup` is interactive. It runs these steps in order and skips each one already done, so running it again resumes where it stopped:

   - `collied login`: sign in as the tag owner. The node becomes `tag:collie-mac` on macOS, `tag:collie-linux` on Linux.
   - `collied service install`: the launchd agent that runs `collied` at login on macOS, the systemd user unit on Linux. It is installed again, restarting `collied`, when it points at another binary or config, is stopped, or runs an older binary than the installed one.
   - `collied doctor`: setup stops on a fail line. Warn lines for peers, apns, hooks and plan usage are expected on a new machine, and one for reach until your phone is signed in to Tailscale (step 3); setup then prints the policy entries in case the grant is missing.
   - Push notifications, when `[apns]` is not configured yet: offered, and skippable (step 5).
   - `collied pair`: offered at the end (step 4).

   Setup needs a terminal and `~/.cargo/bin/collied` as a regular file rather than a symlink (signed, on macOS). It stops when the service already runs another `--config`, and on Linux when the user manager loads the unit from another file, applies a `collied.service.d` drop-in to it, or the unit pins another `XDG_DATA_HOME`. It never confirms a pairing itself: that stays your `y` on the machine. Each step also runs on its own.

3. **Install the app** on a connected iPhone with Developer Mode on: `just ios-run-device`. The watch app installs with it; signing it needs the paired Apple Watch registered to your team once (connect the watch in Xcode's Devices and Simulators window, or add its UDID in the developer portal). Sign in to Tailscale inside the app with your own account.

4. **Pair**: accept the pairing offered at the end of `collied setup`, or run `collied pair` (or Pair a Phone in the menu bar app or bar widget). Scan the QR code in the app (Machines, Add machine) and confirm with `y` (or Pair in the app's window) on the machine.

5. **Push notifications** (optional): `collied setup` asks for the `.p8` path and the team and bundle IDs, adds the `[apns]` section and imports the key. By hand: add an `[apns]` section to `collied.toml`, import the key with `collied apns import AuthKey_<KEY_ID>.p8`, then check with `collied apns test`. Each machine uses its own APNs key (its own key ID, revoked on its own) and sends its own pushes. On Linux, `collied doctor` reports whether the key is a credential or a file and, for a credential, its seal. Full steps: [docs/release.md](docs/release.md).

6. **Claude Code hook** (optional): add `collied hook` as a `PermissionRequest` hook in `~/.claude/settings.json` so approvals name the exact tool call. It only reports the call to `collied`; Claude Code's dialog is unchanged. `collied doctor` checks it.

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

8. **Terminals** (optional): to use plain shell panes from the phone, add this to `collied.toml` and restart `collied` (`collied stop`, then `collied start`):

   ```toml
   [terminals]
   enabled = true
   ```

   The phone must have a passcode and must be paired after both sides are updated, since pairing records the phone's terminal key. A phone paired before shows "Pair this phone again to use terminals on this machine".

9. **Desktop companion** (optional):

   - macOS: `just mac-install` builds CollieBar, signs it with the same Developer ID and installs it to `~/Applications`. Open at Login in its menu starts it at login (off by default). It is not notarized; notarization is optional and only needed for another Mac ([docs/release.md](docs/release.md)).
   - Linux with Omarchy: `just tray-install` copies `Linux/CollieTray` to `~/.config/omarchy/plugins/rbstp.collie` and puts it on the right of the bar. It loads with the shell at every login while enabled; after Quit, `omarchy plugin enable rbstp.collie` puts it back.

## Operating collied

| Command | Purpose |
|---|---|
| `collied status` | The running daemon's state: its node and tags, sessions, paired phones, herdr version and the agents it sees |
| `collied doctor` | Checks the config, service, herdr, the tailnet node, its tag and reach, push, hooks and plan usage |
| `collied peers list` | Lists paired phones |
| `collied peers revoke <label or StableID>` | Revokes a phone and closes its live sessions (removing the machine in the app does the same when it is reachable) |
| `collied stop` | Turns `collied` off and keeps it off, across reboots, until `collied start` |
| `collied start` | Starts it again |
| `collied service uninstall` | Stops and removes the launchd agent or systemd user unit |

**Configuration and data**: on macOS, `collied.toml` is in `~/Library/Application Support/collie/`. On Linux, the data directory is `$XDG_DATA_HOME/collie`, else `~/.local/share/collie`, and holds `collied.toml`, the node state, paired phones and the audit log. Logs go to the user journal (`journalctl --user -u collied`). A systemd user unit runs while you have a session; `loginctl enable-linger` (optional) keeps it running without a login.

**Updating**: `git pull`, `just setup`, `just collied-install`, then `collied setup` again. It keeps the node, the paired phones and the config, and offers to pair a phone again. Update the bar widget with `git pull` and `just tray-install`, and the menu bar app with `just mac-install`.

## Development

| Command | What it does |
|---|---|
| `just setup` | Fetches the submodules |
| `just fmt` | `cargo fmt` |
| `just lint` | `cargo fmt --check`, clippy with `-D warnings`, `cargo deny` |
| `just test` | All Rust tests with cargo-nextest, including end-to-end tests over a local test tailnet |
| `just schema` | Regenerates the protocol JSON Schemas in `docs/protocol/` |
| `just ios-framework` | Builds the `CollieCore` xcframework and UniFFI bindings |
| `just ios-ghostty` | Builds libghostty-vt from its pinned commit (skipped when already built) |
| `just ios-project` | Builds libghostty-vt and generates the Xcode project with XcodeGen |
| `just ios-test` | GhosttyTerminal package tests on macOS, then the iOS unit tests on the simulator |
| `just ios-build-sim` | Simulator build |
| `just ios-run-device` | Builds, installs and launches on a connected iPhone |
| `just mac-project` | Generates the menu bar app's Xcode project (`Mac/project.yml`) |
| `just mac-build` | Unsigned Release build of the menu bar app |
| `just mac-test` | The menu bar app's unit tests (never touches the real control socket) |
| `just mac-install` | Builds the menu bar app, signs it with the Developer ID and the hardened runtime, installs it to `~/Applications` and opens it |
| `just mac-notarize` | Optional: notarizes and staples the app `mac-install` signed, with a `notarytool` keychain profile |
| `just tray-test` | The Omarchy bar widget's tests (Linux): icon parity with the Mac, plugin validation, wire format and parsing under node, and `Service.qml` in `qs` against a fake `collied` (never the real control socket) |
| `just tray-install` | Copies the Omarchy bar widget to `~/.config/omarchy/plugins/rbstp.collie` and puts it on the bar (Linux) |
| `just collied-install` | Release build of `collied`, installed to `~/.cargo/bin`. On macOS it is signed and restarts the launchd agent if installed; on Linux it is unsigned and restarts the systemd user unit if active |

CI runs on macOS on every pull request and on pushes to `master`: lint, the Rust tests, the GhosttyTerminal package tests, and the iOS simulator build and tests. Merging a pull request that changes code into `master` uploads a build to the maintainer's TestFlight ([docs/release.md](docs/release.md)).

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
Collie/            iOS app (XcodeGen project.yml), ColliePush and CollieWidgets extensions,
                   CollieWatch app and its CollieWatchWidgets complication, CollieCore
                   package (xcframework and UniFFI bindings), GhosttyTerminal package
Mac/               macOS menu bar app CollieBar (XcodeGen project.yml), a client of
                   collied's control socket
Linux/             Omarchy bar widget CollieTray (a Quickshell plugin), a client of
                   collied's control socket, and its tests
docs/              Architecture, threat model, tailnet setup, release, protocol schemas
scripts/           libghostty-vt xcframework build
```

## Documentation

- [Architecture](docs/architecture.md): design, protocol behavior, herdr and Tailscale integration, and how the main features work
- [Threat model](docs/threat-model.md): assets, trust boundaries, threats and what is not covered
- [Tailnet setup](docs/tailnet.md): the policy file, tags, owner, key expiry and revocation
- [Release](docs/release.md): Apple setup, APNs, TestFlight, notarization and `collied` on Linux
- [Protocol](docs/protocol): JSON Schemas of the client and server frames (generated with `just schema`) and shared test fixtures

## Contributing

Bugs and feature requests are tracked as [GitHub issues](https://github.com/rbstp/collie/issues). Before opening a pull request, run `just lint` and `just test`, plus the tests of any app you changed (`just ios-test`, `just mac-test` or `just tray-test`).

## License

collie is licensed under the [Apache License 2.0](LICENSE). The `tailscale-sys` crate, which builds the vendored libtailscale, is licensed `Apache-2.0 AND BSD-3-Clause`.
