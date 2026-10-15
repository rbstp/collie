# Requirements

## To run collie

- **A machine for the agents**: an Apple silicon Mac, or an x86_64 Linux machine with a systemd user manager. Intel Macs are not supported. On Linux, `collied` is built and tested on Arch Linux.
- **herdr 0.9.5**, the version `collied` is tested against.
- **An iPhone on iOS 26 or later.** An Apple Watch on watchOS 26 or later is optional.
- **A Tailscale account** whose policy file you can edit.
- **An Apple Developer account**, with a Developer ID Application certificate (`collied` is always signed on macOS, never on Linux) and an APNs key if you want push notifications.
- **Optional, for the Linux bar widget**: Omarchy 4 with `qrencode`, which Omarchy installs.

> [!NOTE]
> On Linux, the encrypted APNs key needs systemd 256 or later (`systemd-creds --user`). Older versions fall back to a file readable only by your user.

## To build on macOS

- Rust (the version in `rust-toolchain.toml`) and Go
- Xcode 27 with the watchOS platform and an iPhone 18 Pro simulator
- [just](https://github.com/casey/just), [XcodeGen](https://github.com/yonaskolb/XcodeGen), `cargo-deny`, [cargo-nextest](https://nexte.st) and `jq`

The libghostty-vt build downloads its own pinned Zig.

## To build collied on Linux

- Rust (the version in `rust-toolchain.toml`) and Go 1.27.2 or later
- A C compiler and libclang
- [just](https://github.com/casey/just), `cargo-deny` and [cargo-nextest](https://nexte.st)

## Your own identifiers

The justfile and `Collie/project.yml` use the maintainer's Apple team ID and `dev.rbstp` bundle identifiers. Change them to your own before you build.
