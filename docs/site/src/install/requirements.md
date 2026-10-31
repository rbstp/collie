# Requirements

## To run collie

- **A machine for the agents**: an Apple silicon Mac, or an x86_64 Linux machine with a systemd user manager. Intel Macs are not supported. On Linux, collie has been tested only on Arch Linux with Omarchy.
- **herdr 0.9.5**, the version `collied` is tested against.
- **An iPhone on iOS 26 or later**, and optionally an Apple Watch on watchOS 26 or later.
- **A personal Tailscale account**, signed in inside Collie on the phone and through `collied setup` on the machine. You need no Tailscale app and no VPN profile. A restrictive tailnet policy may need an administrator to allow TCP port 8457 (see [Tailnet policy](tailnet-policy.md)).
- **An Apple Developer account**, to build the app and install it on your iPhone ([Install the app](app.md)), and an APNs key if you want push notifications.
- **Optional, for the Linux bar widget**: Omarchy 4 (its shell runs on Quickshell 0.3) and `qrencode`, which Omarchy installs.

## Tested agent integrations

Collie has been tested only with Claude Code CLI, Codex CLI and GitHub Copilot CLI, and each offers a different set of features: see [Claude Code](../agents/claude-code.md), [Codex](../agents/codex.md) and [Copilot CLI](../agents/copilot.md). herdr supports more agents, but Collie has not been validated with them.

## To build on macOS

- Rust (the version in `rust-toolchain.toml`) and Go 1.27.3 or later
- Xcode 27 with the watchOS platform, and an iPhone 18 Pro simulator for `just ios-test`
- [just](https://github.com/casey/just), [XcodeGen](https://github.com/yonaskolb/XcodeGen), `cargo-deny`, [cargo-nextest](https://nexte.st) and `jq`
- A Developer ID Application certificate, to sign `collied` and CollieBar (release downloads need none)

You do not need Zig: the libghostty-vt build downloads its own pinned copy.

## To build collied on Linux

- Rust (the version in `rust-toolchain.toml`) and Go 1.27.3 or later
- A C compiler and libclang
- [just](https://github.com/casey/just), `cargo-deny` and [cargo-nextest](https://nexte.st)
- Node, for `just tray-test`

## Your own identifiers

The justfile and the `project.yml` files use the maintainer's Apple team ID (`RM3UT3MMSR`) and `dev.rbstp` bundle identifiers, and the app's entitlements and `Collie/Sources/Shared/PushHint.swift` use its App Group, `group.dev.rbstp.collie`. Change them to your own before you build, but keep `dev.rbstp.collied`, the identifier `collied` is signed with: `collied setup` checks it.
