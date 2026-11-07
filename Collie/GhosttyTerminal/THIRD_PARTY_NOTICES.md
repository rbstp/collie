# Third-party notices: GhosttyVt.xcframework

`GhosttyVt.xcframework` is built by `scripts/ghostty/build-xcframework.sh` from the libghostty-vt
release tarballs pinned there (`LIBGHOSTTY_VT_VERSION`), with `-Dvt-features=-all,+render-state`.
It is linked statically into Collie.app. Keep this file in sync when the pin changes.

| Component | Source | Version | License |
|---|---|---|---|
| libghostty-vt (Ghostty) | https://github.com/ghostty-org/ghostty | 1.0.0 (tag `vt-1.0.0`, `0fa3d577a5614150ff8e2c152de5fb9d5c6d2dec`) | MIT, Copyright (c) 2024 Mitchell Hashimoto, Ghostty contributors |
| simdutf (vendored in `pkg/simdutf`) | https://github.com/simdutf/simdutf | 9.0.0 | Apache-2.0 or MIT |
| Highway (`pkg/highway`) | https://github.com/google/highway | 1.2.0 | Apache-2.0 or BSD-3-Clause |
| uucode (Unicode property tables) | https://github.com/jacobsandlund/uucode | 0.2.0 (`1fb7343`) | MIT, Copyright (c) 2026 Jacob Sandlund; includes Unicode data (Unicode License v3) and a UTF-8 decoder by Bjoern Hoehrmann (MIT) |
| Zig compiler-rt | https://github.com/ziglang/zig | 0.17.1 | MIT, Copyright (c) Zig contributors |

Build-time only, not linked: the Zig toolchain, Aro and translate-c (C header translation),
Ghostty's theme and test data. Wuffs, in the release's dependency tarball, is compiled only with
Kitty graphics, which this build leaves out.

`scripts/ghostty/build-xcframework.sh` collects the license texts from the pinned sources into
`GhosttyVt.xcframework/licenses`, and the app shows them in Settings > Acknowledgements.
