# collie

[![ci](https://github.com/rbstp/collie/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/rbstp/collie/actions/workflows/ci.yml)

collie is a remote control for [herdr](https://github.com/herdrdev/herdr) coding agents. A small daemon, `collied`, runs next to herdr on your Mac or Linux machine, and the Collie app on your iPhone and Apple Watch reaches it over your own tailnet, with no open port on your network. From the phone, you follow every Claude Code, Codex and Copilot CLI agent, read its conversation or terminal, prompt it, start tasks and approve a blocked Claude Code agent from the lock screen.

Documentation: <https://collie.rbstp.dev>

<p>
<img src="docs/site/src/images/agents-dark.png" width="300" alt="The Agents tab: a grid of agent previews in three workspaces">
<img src="docs/site/src/images/chat-dark.png" width="300" alt="An agent's Chat view with its prompt field">
</p>

## Requirements

- An Apple silicon Mac, or an x86_64 Linux machine with a systemd user manager, running herdr 0.9.5
- An iPhone on iOS 26 or later; an Apple Watch on watchOS 26 or later is optional
- A personal Tailscale account
- An Apple Developer account to build the app, and an APNs key for push notifications

Collie is tested only with Claude Code CLI, Codex CLI and GitHub Copilot CLI, and on Linux only on Arch Linux with Omarchy. [Requirements](https://collie.rbstp.dev/install/requirements.html) lists the build tools and the Apple identifiers to replace before you build.

## Quick start

1. Check that your [tailnet policy](https://collie.rbstp.dev/install/tailnet-policy.html) allows TCP port 8457 between your own devices.
2. Install `collied` from a [release download](https://github.com/rbstp/collie/releases) or from source ([macOS](https://collie.rbstp.dev/install/collied-macos.html), [Linux](https://collie.rbstp.dev/install/collied-linux.html)). From source, with the [build tools](https://collie.rbstp.dev/install/requirements.html#to-build-from-source) installed and, on macOS, [your own identifiers](https://collie.rbstp.dev/install/requirements.html#your-own-identifiers) set:

   ```sh
   git clone --recurse-submodules https://github.com/rbstp/collie
   cd collie
   just setup
   just collied-install
   collied setup
   ```

3. [Install the app](https://collie.rbstp.dev/install/app.html) with `just ios-run-device`, then [pair it](https://collie.rbstp.dev/install/pair.html).
4. Optional: [push notifications](https://collie.rbstp.dev/install/push.html), the [Claude Code hook and plan usage](https://collie.rbstp.dev/install/claude-code.html), [terminals](https://collie.rbstp.dev/install/terminals.html) and the [desktop companions](https://collie.rbstp.dev/desktop/colliebar.html).

## Documentation

The [site](https://collie.rbstp.dev) covers install, use, [troubleshooting](https://collie.rbstp.dev/troubleshooting.html), [security](https://collie.rbstp.dev/security/overview.html), [privacy](https://collie.rbstp.dev/privacy.html) and [support](https://collie.rbstp.dev/support.html); it is built from [docs/site](docs/site) ([docs/deploy-pages.md](docs/deploy-pages.md)). Design docs: [architecture](docs/architecture.md), [threat model](docs/threat-model.md), [tailnet setup](docs/tailnet.md), [notifications](docs/notifications.md), [release](docs/release.md) and the [protocol schemas](docs/protocol).

## Development

| Command | What it does |
|---|---|
| `just setup` | Fetches the submodules |
| `just lint` | `cargo fmt --check`, clippy with `-D warnings`, `cargo deny`; `just fmt` formats |
| `just test` | All Rust tests, end-to-end included; `PROPTEST_CASES` raises the 256 property-test cases |
| `just schema` | Regenerates the protocol JSON Schemas in `docs/protocol/` |
| `just ios-test` | GhosttyTerminal package tests, then the iOS unit and UI tests on the simulator |
| `just ios-build-sim` | Simulator build |
| `just ios-screenshots` | Regenerates the documentation screenshots and the App Store set in `target/ios/app-store` |
| `just mac-test` | CollieBar's unit tests |
| `just tray-test` | CollieTray's tests against a fake `collied` (Linux) |

Install recipes, described on the site: `just collied-install` (the daemon, signed on macOS), `just ios-run-device` (the app), `just mac-install` (CollieBar; `just mac-notarize` optionally notarizes it) and `just tray-install` (CollieTray). `ios-framework`, `ios-ghostty`, `ios-project`, `mac-project` and `mac-build` run as dependencies; `just --list` shows every recipe.

CI runs lint, the Rust tests and the iOS simulator build and tests on macOS for pull requests and pushes to `master`; changes only to the site, the README or AGENTS.md skip them. Merging code into `master` uploads a TestFlight build ([docs/release.md](docs/release.md)).

## Repository layout

```
crates/
  protocol/        Wire types and their JSON Schemas
  tailscale-sys/   libtailscale (pinned submodule + patches) as a Go c-archive
  tailnet/         Safe wrapper over tailscale-sys
  collied/         The daemon
  tls/             Mutual TLS with raw public keys and key pins
  collie-core/     The phone's Rust core, exposed to Swift with UniFFI
  uniffi-bindgen/  UniFFI binding generator
  e2e/             Phone core against collied over a local test tailnet
Collie/            iOS app, extensions, Apple Watch app, Swift packages (XcodeGen)
Mac/               CollieBar menu bar app
Linux/             CollieTray Omarchy bar widget and its tests
relay/             The publisher's push relay (Cloudflare Worker)
packaging/         Arch Linux (AUR) recipes, not published yet
docs/              Design docs, protocol schemas and the site
scripts/           libghostty-vt build, desktop packaging, acknowledgements, VM test harness
```

## Contributing

File bugs and feature requests as [GitHub issues](https://github.com/rbstp/collie/issues). Before you open a pull request, run `just lint`, `just test` and the tests of any app you changed. [AGENTS.md](AGENTS.md) holds the repository's conventions.

## License

collie is licensed under the [Apache License 2.0](LICENSE). The `tailscale-sys` crate, which builds the vendored libtailscale, is licensed `Apache-2.0 AND BSD-3-Clause`.
