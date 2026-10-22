# Requirements

To run collie, you need the machine, phone and accounts listed first. To build it from source, you also need the tools listed further down.

## To run collie

- **A machine for the agents**: an Apple silicon Mac, or an x86_64 Linux machine with a systemd user manager. Intel Macs are not supported. On Linux, collie has been tested only on Arch Linux with Omarchy.
- **herdr 0.9.5**, the version `collied` is tested against.
- **An iPhone on iOS 26 or later.** An Apple Watch on watchOS 26 or later is optional.
- **A personal Tailscale account**, signed in inside Collie on both the phone and the machine. You need no VPN profile and no standalone Tailscale app. Most existing policies work unchanged; a restrictive one may need an administrator to allow TCP port 8457 (see [Tailnet policy](tailnet-policy.md)).
- **An Apple Developer account**, with a Developer ID Application certificate (`collied` is always signed on macOS, never on Linux) and an APNs key if you want push notifications.
- **Optional, for the Linux bar widget**: Omarchy 4 (its shell runs on Quickshell 0.3) with `qrencode`, which Omarchy installs.

> [!NOTE]
> On Linux, `collied` stores the encrypted APNs key with `systemd-creds --user`, which needs systemd 256 or later. With an older systemd, it keeps the key in a file readable only by your user.

## Tested agent integrations

Collie has been tested only with Claude Code CLI, Codex CLI and GitHub Copilot CLI. herdr supports more agents, but Collie has not been validated with them. Each integration offers a different set of features: see [Claude Code](../agents/claude-code.md), [Codex](../agents/codex.md) and [Copilot CLI](../agents/copilot.md).

## To build on macOS

- Rust (the version in `rust-toolchain.toml`) and Go
- Xcode 27 with the watchOS platform and an iPhone 18 Pro simulator
- [just](https://github.com/casey/just), [XcodeGen](https://github.com/yonaskolb/XcodeGen), `cargo-deny`, [cargo-nextest](https://nexte.st) and `jq` (for `just ios-run-device`)

You do not need to install Zig: the libghostty-vt build downloads its own pinned copy.

## To build collied on Linux

- Rust (the version in `rust-toolchain.toml`) and Go 1.27.3 or later
- A C compiler and libclang
- [just](https://github.com/casey/just), `cargo-deny` and [cargo-nextest](https://nexte.st)
- Node, for `just tray-test`

## Your own identifiers

The justfile and `Collie/project.yml` use the maintainer's Apple team ID and `dev.rbstp` bundle identifiers. Before you build, change them to your own.
