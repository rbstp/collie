# Third-party notices: GhosttyVt.xcframework

`GhosttyVt.xcframework` is built by `scripts/ghostty/build-xcframework.sh` from upstream Ghostty
at the commit pinned there (`GHOSTTY_COMMIT`), with `-Demit-lib-vt -Dvt-features=-all,+render-state`.
It is linked statically into Collie.app. Keep this file in sync when the pin changes.

| Component | Source | Version | License |
|---|---|---|---|
| libghostty-vt (Ghostty) | https://github.com/ghostty-org/ghostty | `befcdfd2c3a1cb24d9ec886e93c95b2b5daa7028` | MIT, Copyright (c) 2024 Mitchell Hashimoto, Ghostty contributors |
| simdutf (vendored in `pkg/simdutf`) | https://github.com/simdutf/simdutf | 9.0.0 | Apache-2.0 or MIT |
| Highway (`pkg/highway`) | https://github.com/google/highway | 1.2.0 | Apache-2.0 or BSD-3-Clause |
| uucode (Unicode property tables) | https://github.com/jacobsandlund/uucode | 0.2.0 (`9d55524`) | MIT, Copyright (c) 2026 Jacob Sandlund; includes Unicode data (Unicode License v3) and a UTF-8 decoder by Bjoern Hoehrmann (MIT) |
| Zig compiler-rt | https://github.com/ziglang/zig | 0.16.0 | MIT, Copyright (c) Zig contributors |

Build-time only, not linked: the Zig toolchain, Aro and translate-c (C header translation),
Ghostty's theme and test data.

Full license texts are in each project's repository at the versions above; the MIT and BSD
notices must be reproduced in the app's acknowledgements before App Store distribution.
