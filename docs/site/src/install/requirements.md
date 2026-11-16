# Requirements

## To run collie

- **An Apple silicon Mac**, not an Intel one, **or an x86_64 Linux machine** with a systemd user manager. On Linux, collie is tested only on Arch Linux with Omarchy.
- **herdr 0.9.5, herdr-core 0.1.16 (Herdr Core, `hc`), or both**, the tested versions, with no setting. Collie supports Herdr Core: you follow its agents' status, watch their screens, prompt them, answer their approvals, read a Claude Code or Codex agent's conversation, start, resume and archive Claude Code and Codex [tasks](../app/new-task.md#herdr-core), and close agents. On **Automatic**, new tasks start in herdr whenever herdr runs, and in Herdr Core when it does not; [Start tasks in](../app/new-task.md#where-tasks-start) can send them to one runtime instead. In Herdr Core, collie cannot bring an agent to the front on the desktop, and only Claude Code and Codex take your input there: you can watch Copilot CLI or another agent but not type into it.
- **On the Mac, Rex 0.1.0-preview.19** (Rex Beta.app), instead of herdr and Herdr Core or beside them, with no setting. Rex is a closed preview, and collie is tested with this version only, on macOS only; `collied doctor` says when it cannot follow another version. Collie supports Rex as it supports Herdr Core: you follow its agents' status, watch their screens, prompt them, answer their approvals, read a Claude Code or Codex agent's conversation, start, resume and archive Claude Code and Codex [tasks](../app/new-task.md#rex), and close agents. In Rex, collie cannot bring an agent to the front in Rex.app, and only Claude Code and Codex take your input. `collied` never starts Rex: it follows the server Rex.app runs, once it runs.
- **[Claude Code](../agents/claude-code.md), [Codex](../agents/codex.md) or [GitHub Copilot](../agents/copilot.md) CLI**, the only agents Collie is tested with; their features differ.
- **An iPhone on iOS 26 or later**, and optionally an Apple Watch on watchOS 26 or later.
- **A personal Tailscale account.** You need no Tailscale app; a restrictive policy may need a [grant](tailnet-policy.md).
- **A Mac with the [build tools](#to-build-from-source) and an Apple Developer account** to [build the app](app.md), also when `collied` runs on Linux, and an APNs key for push notifications.
- **Optional, for the Linux bar widget**: Omarchy 4 and `qrencode`, which Omarchy installs.

## To build from source

- **Both systems**: Rust (the version in `rust-toolchain.toml`), Go 1.27.3 or later, [just](https://github.com/casey/just), `cargo-deny` and [cargo-nextest](https://nexte.st).
- **macOS**: Xcode 27 with the watchOS platform, [XcodeGen](https://github.com/yonaskolb/XcodeGen), `jq`, a Developer ID Application certificate to sign `collied` and CollieBar, and an iPhone 18 Pro simulator for `just ios-test`.
- **Linux**, where you build only `collied`: a C compiler and libclang, and Node for `just tray-test`.

## Your own identifiers

Before you build, replace the maintainer's Apple team ID (`RM3UT3MMSR`) and `dev.rbstp` bundle identifiers in the justfile and the `project.yml` files, and its App Group, `group.dev.rbstp.collie`, in the app's entitlements and `Collie/Sources/Shared/PushHint.swift`. Keep `dev.rbstp.collied`, the identifier `collied` is signed with: `collied setup` checks it.
