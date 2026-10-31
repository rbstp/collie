# Requirements

## To run collie

- **An Apple silicon Mac**, not an Intel one, **or an x86_64 Linux machine** with a systemd user manager. On Linux, collie is tested only on Arch Linux with Omarchy.
- **herdr 0.9.5**, the tested version.
- **[Claude Code](../agents/claude-code.md), [Codex](../agents/codex.md) or [GitHub Copilot](../agents/copilot.md) CLI**, the only agents Collie is tested with; their features differ.
- **An iPhone on iOS 26 or later**, and optionally an Apple Watch on watchOS 26 or later.
- **A personal Tailscale account.** You need no Tailscale app; a restrictive policy may need a [grant](tailnet-policy.md).
- **An Apple Developer account** to [build the app](app.md), and an APNs key for push notifications.
- **Optional, for the Linux bar widget**: Omarchy 4 and `qrencode`, which Omarchy installs.

## To build from source

- **Both systems**: Rust (the version in `rust-toolchain.toml`), Go 1.27.3 or later, [just](https://github.com/casey/just), `cargo-deny` and [cargo-nextest](https://nexte.st).
- **macOS**: Xcode 27 with the watchOS platform, [XcodeGen](https://github.com/yonaskolb/XcodeGen), `jq`, a Developer ID Application certificate to sign `collied` and CollieBar, and an iPhone 18 Pro simulator for `just ios-test`.
- **Linux**, where you build only `collied`: a C compiler and libclang, and Node for `just tray-test`.

## Your own identifiers

Before you build, replace the maintainer's Apple team ID (`RM3UT3MMSR`) and `dev.rbstp` bundle identifiers in the justfile and the `project.yml` files, and its App Group, `group.dev.rbstp.collie`, in the app's entitlements and `Collie/Sources/Shared/PushHint.swift`. Keep `dev.rbstp.collied`, the identifier `collied` is signed with: `collied setup` checks it.
