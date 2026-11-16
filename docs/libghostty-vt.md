# libghostty-vt in collied

Spike for [#739](https://github.com/rbstp/collie/issues/739), 2026-11-15. collied emulates Herdr Core and Rex terminals with `alacritty_terminal` 0.26.0 (`crates/collied/src/runtime/screen.rs`) to produce the ANSI screens the phone reads. The phone renders with libghostty-vt, and herdr, Herdr Core and Rex all emulate with Ghostty's terminal. This page answers whether collied should move to libghostty-vt, with the evidence. It changes nothing in collied.

## Summary

- **Recommendation: stay on `alacritty_terminal`.** Switching would cost no binary size, and libghostty-vt covers everything collied reads, but today it buys four edge cases and lower memory, while it puts a Zig build and an unsafe binding into every collied build. Rex streaming, the one real gain, matters only when a phone watches a terminal with a very long scrollback, and nothing shows that to be slow in practice. Do not take libghostty-vt for Rex streaming only: collied would then carry two emulators. [Revisit](#revisit-when) when a measurement says so.
- **Size: neutral.** On macOS arm64, collied is 43,568,816 bytes today, 42,619,872 without any emulator (so `alacritty_terminal` costs 0.95 MB), and 43,572,704 with libghostty-vt linked statically in its place: 3,888 bytes more than today (verified). On Linux x86_64 the library adds about 1.27 MB stripped (verified on a small program; collied itself was not built for Linux), against roughly 1 MB that `alacritty_terminal` gives back (inferred).
- **Version bumps:** the pins already live in one place, `scripts/ghostty/build-xcframework.sh`. collied could share that pin and the verified tarballs, but needs its own build per target with other features, Zig on the Linux release runner and in CI's `rust` job, and its notices. A bump stays a one-file pin change plus the checks the iOS bump already needs; Rex streaming adds one more: the snapshot format is versioned apart from the library and outside its ABI promise ([Version bumps](#version-bumps)).
- **A newer release exists:** libghostty-vt 1.0.1 (tag `vt-1.0.1`, 2026-11-11, 91 commits after 1.0.0: resize and scrollback fixes, render fixes, simdutf fixes, snapshot code changes). The app pins 1.0.0, which the repository's latest-stable rule says to move ([Follow-up](#follow-up)).
- **Herdr:** neither herdr, nor Herdr Core, nor the Herdr app uses `alacritty_terminal` any more; all three link libghostty-vt built with Zig 0.16 ([6](#6-herdr)).

## Evidence

Verified by running, on an M3 Max Mac (Darwin 27.3), with Zig 0.17.1 (the iOS pin), Rust 1.99.0 and collied at `e73fcad9`:

- libghostty-vt 1.0.0 built from the pinned, checksum-verified tarballs for macOS arm64 and, cross-compiled from the Mac with Zig alone, for Linux x86_64 (glibc 2.39), static and shared, with collied's feature set (`-Dvt-features=-all,+grid-introspection`) and with the snapshot decoder added (`+snapshot`).
- A throwaway prototype ([Prototype](#prototype)): collied's `Screen` reimplemented on libghostty-vt's C API, run beside a copy of today's `Screen` on the same bytes, and dropped into a scratch checkout of collied, where collied's own `runtime::` tests ran against it and release builds measured the size.
- `cargo deny check` with collied's `deny.toml`, on that scratch checkout and on a scratch crate depending on the crates.io bindings.
- The quick check recorded on the issue (Rex 0.1.0-preview.19, API revision 14, on a Rex server of its own): libghostty-vt reads Rex's `GHOSTSNP` snapshot and the stream continues byte-exact. Not redone here.

Read, not run: the C headers and `PACKAGING-LIBGHOSTTY.md` of 1.0.0; the `vt-1.0.0...vt-1.0.1` comparison and the `vt-1.0.1` release files; `Cargo.toml`, `Cargo.lock` and the build scripts of herdr (`herdrdev/herdr` at `ee5bfb50`, 2026-10-29, herdr 0.9.5), Herdr Core (`herdrdev/herdr-core` at `bb8a2f77`, 2026-11-15) and the Herdr app (`herdrdev/herdr-app` at `8e1208de`, 2026-11-15), all public; collied's runtime, approvals and their tests; `.github/workflows/` and `scripts/desktop/`.

Not verified: a collied build for Linux (it needs a Linux Rust target and a cross Go toolchain for libtailscale; Docker was not running), so every Linux size of collied itself is inferred; the time a GitHub runner takes to build the library; a live Herdr Core or Rex terminal through the prototype (it ran on recorded bytes); how Rex.app and Herdr Core set Ghostty's grapheme mode 2027.

## Questions

### 1. API coverage

Yes, all of it (verified): the prototype's `Screen` reads every fixture and every test case byte-identical to today's, except where the emulators differ ([4](#4-fidelity)).

| collied uses (alacritty) | libghostty-vt 1.0.0 |
|---|---|
| feed bytes (`Processor::advance`) | `ghostty_terminal_vt_write` |
| rows and cells: character, zero-width additions, wide-character spacers | grid references: `ghostty_terminal_grid_ref`, `ghostty_grid_ref_graphemes` (the whole cluster), `GHOSTTY_CELL_DATA_WIDE` (wide, spacer tail, spacer head) |
| attributes: bold, dim, italic, underline, inverse, hidden, strikeout, foreground and background colors | `ghostty_grid_ref_style`: flags plus palette or RGB colors. A cell erased with a background color holds that color as its content, not its style (`GHOSTTY_CELL_DATA_CONTENT_TAG`), so a reader must check both |
| soft-wrap flag per row (`WRAPLINE`, for `recent_unwrapped`) | `GHOSTTY_ROW_DATA_WRAP` (and `WRAP_CONTINUATION`) |
| scrollback (`scrolling_history`, `history_size`) | `GHOSTTY_POINT_TAG_SCREEN` points, `GHOSTTY_TERMINAL_DATA_TOTAL_ROWS`, `GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES` and `..._MAX_BYTES` |
| resize | `ghostty_terminal_resize`, which reflows: a row wrapped at 10 columns rejoined at 20 |
| synchronized updates (mode 2026: alacritty holds the bytes until the update ends or times out) | applied at once; the `GHOSTTY_TERMINAL_OPT_RENDER_HOLD` callback says when a hold starts and ends |

What a binding would have to get right (verified):

- The default scrollback is 10,000 bytes, so both limits must be set.
- The line limit prunes whole pages, and can keep fewer lines than asked: a limit of 1,000 kept 553 lines at 80 columns, 10,000 kept 9,433. collied's test of a 1,000-line read failed until the prototype asked for twice the lines plus 1,024.
- Synchronized updates: today a read during an open update shows the screen before it (`an_open_synchronized_update_shows_the_screen_before_it`). libghostty-vt draws the update as it arrives, so collied would hold the bytes itself, or keep its last read for the duration of a hold.
- Grid references cost about 30 ns a cell. A read of 120 rows at 200 columns took 0.95 ms against alacritty's 0.25 ms; reading every row of a 10,000-line history took 60 ms against 1.3 ms. A real binding reads only the rows a read returns, or uses the formatter for history.
- A terminal handle is a raw pointer, so the Rust type needs `unsafe impl Send` (one thread at a time, as collied uses it).

### 2. Build cost

- **Zig:** 0.17.1, already pinned for iOS; `minimum_zig_version` is 0.17.0 at both `vt-1.0.0` and `vt-1.0.1`.
- **Time (verified, under `nice -n 15`):** 83 s for a cold macOS arm64 build (an empty Zig cache, which `PACKAGING-LIBGHOSTTY.md` requires for every build); 25 s for another feature set once Zig's own build runner is cached; 28 to 34 s for Linux x86_64 cross-compiled from the Mac. collied's own release build took 74 s cold here, and 27 to 30 s after a change to `screen.rs`.
- **Targets:** collied ships macOS arm64 and Linux x86_64 (`desktop-release.yml`); `deny.toml` also names Linux aarch64. Zig cross-compiles all of them with no sysroot. The app's xcframework holds a macOS slice, but with `-all,+render-state` only: no grid references, history reads or snapshot decoder. collied would need its own build per target (inferred: the iOS build could add those features to share one, at the cost of reading what grows in the app).
- **Binding:** collied's only C dependency today is libtailscale, through `crates/tailscale-sys`. A local `crates/ghostty-vt-sys` beside it, with hand-written declarations for the dozen functions collied calls, adds no third-party crate: on the scratch checkout, `cargo deny check` passed every check with `alacritty_terminal` removed and the binding in place. Removing `alacritty_terminal` drops 15 packages from `Cargo.lock`. The crates.io bindings (`ghostty-vt` and `ghostty-vt-sys` 0.9.0+ghostty-vt.1.0.1, one maintainer) fail collied's licence check: their build script downloads the library with `ureq`, which brings `webpki-roots` and `webpki-root-certs` (CDLA-Permissive-2.0).
- **No C++ runtime:** the bundled simdutf and Highway link without one, and the macOS binary gained no dynamic library (verified with `otool -L`).
- **CI and release:** see [Version bumps](#version-bumps).

### 3. Rex streaming

Answered by the quick check on the issue: libghostty-vt's snapshot decoder reads the `GHOSTSNP` snapshot that opens Rex's data stream, and the live bytes after it reproduce Rex's own `format` exactly. The parser ignores Rex's `ESC _ rex1;...` messages; `ghostty_terminal_vt_write_until` can stop at APC boundaries, which would let collied pick out Rex's `resize` messages (read in `terminal.h`, not run).

The gain exists only if collied follows the stream instead of polling `format`. A poll renders the whole scrollback on Rex's side: 45 to 58 ms at 100,000 lines, every 250 ms while a phone watches a terminal that keeps changing ([rex.md](rex.md#4-screen)). A followed terminal costs one snapshot (8.8 MB and 48 to 60 ms at 100,000 lines), one held connection and a local copy, and then only the bytes the program prints. Without streaming the gain is nil: `format` is already Ghostty's output. The decoder adds 0.10 MB (macOS) and 0.17 MB (Linux) to a static link (verified).

### 4. Fidelity

Every recorded screen reads the same (verified): the Herdr Core fixtures (Claude Code 2.1.316 and Codex 0.166.0, at their recorded 200x120 with and without history, and narrowed to 80x24 and 60x20), the Rex fixture (Claude Code 2.1.325 at 198x68), Rex's `format` of 1,000 numbered lines, and 31 of 37 crafted cases: CJK, combining marks, emoji with VS15 and VS16, skin tones, ZWJ families (also under mode 2027), wide characters at the right edge, tabs, the 256 and RGB palettes, default colors, private-use glyphs, box drawing, the alternate screen, insert mode, scroll regions, DEC line drawing and the controls collied drops.

The six cases that differ come from four behaviors, each of which the phone can show:

| Bytes | alacritty | libghostty-vt | Phone |
|---|---|---|---|
| A flag (two regional indicators), then a cursor move to an absolute column | one column per indicator | two columns per indicator, so the move overwrites the second | the flag shows whole today, half with Ghostty |
| `ESC[21m` | turns bold off | double underline | Ghostty underlines the text |
| `ESC[2J` on a screen with content (two cases, and a third inside a synchronized update) | pushes the screen into history | erases it | today a `recent` read of a Herdr Core terminal shows screens a clear removed, which herdr, Herdr Core and Rex do not keep (inferred: they emulate with Ghostty) |
| An open synchronized update, read before it ends | the screen before it | the half-drawn update | see [1](#1-api-coverage) |

In each case libghostty-vt agrees with the terminal collied reads from, since herdr, Herdr Core and Rex all emulate with Ghostty. The differences reach the phone only through Herdr Core reads and watches, which feed the core's paint and then the program's own bytes to the emulator: a Rex read emulates Rex's `format`, which has already been through Ghostty. Without mode 2027 neither emulator joins a ZWJ family or a skin tone into one cell, and with it libghostty-vt does; collied re-emits the same characters either way, so the phone shows the same.

Other measurements on the same bytes: libghostty-vt parsed 1.5 to 2.4 times as fast (246 to 300 MB/s against 166 to 176 MB/s on 5.1 MB of Rex output; 160 against 73 MB/s on the Codex fixture) and held less memory: at 200 columns, 4.6 MB for a 2,000-line history limit and 18.6 MB for 10,000, against 11.3 MB and 53.7 MB (libghostty-vt keeping somewhat fewer lines, since its limit prunes whole pages).

### 5. Risk

What is pinned to the emulator's output:

- `runtime::screen` tests: the exact SGR form of a read (captured from Herdr Core 0.1.16's paint), soft-wrap joining, the blank row a paint leaves above the screen, a style across a wrap, the controls that never reach a read, the open synchronized update, and the two Herdr Core fixtures against the core's `terminal.read`.
- `runtime::rex::screen_tests`: the Rex fixture's VT emulated reads as Rex's `format` text, and a 1,000-line read of 100,000 lines of scrollback.
- Approvals, menus and fingerprints read the emulator only for Rex: `detection_text` is herdr's read for herdr and the core's own `terminal.read` for Herdr Core, but the emulated `format` text for Rex. A fingerprint hashes that text, so it changes only if the text does; the Rex fixture's text is identical.
- The `claude-*`, `codex-*` and `copilot-*` fixtures are text the parser reads, not emulator input.

How a switch would be verified: the existing tests are the bar. Run against the prototype, 121 of collied's 123 `runtime::` tests passed (verified). The two failures: the open synchronized update, which a switch must handle; and a tasks test that sets a `OnceLock` twice when `cargo test` runs every test in one process, which passes alone; CI runs nextest, one process per test. Before the prototype asked for extra scrollback, the 1,000-line Rex read failed too. Then `live_herdr_core` and `live_rex`, and the Rex fixture through the snapshot path if streaming comes.

### 6. Herdr

No (verified by reading, at the commits in [Evidence](#evidence)): `alacritty_terminal` appears in none of the three lockfiles.

- herdr 0.9.5 builds a vendored libghostty-vt (Ghostty `44f2a44d`, "1.3.2-HEAD", from before the library's 1.0 release) through its own `crates/ghostty-vt`, a 466 KB generated binding whose `build.rs` runs Zig 0.16. herdr dropped its vt100 backend on 2026-04-05 (`b2e6929a`).
- Herdr Core builds a Ghostty submodule pinned at `1e5cdb86` (2026-10-11, 586 commits before `vt-1.0.0`) through its own `crates/ghostty-vt`, with Zig 0.16 and `-Dcpu=baseline`. The `hc` binary was not run.
- The Herdr app links Ghostty through `crates/ghostty-sys`. [herdr-core.md](herdr-core.md#shape) chose `alacritty_terminal` partly because Herdr UI's `herdr-terminal` used it; that no longer holds.

## Size

The owner's question: would it be large to include in collied? No. Release builds of collied `e73fcad9` for `aarch64-apple-darwin` (`MACOSX_DEPLOYMENT_TARGET=26.0`, as `desktop-release.yml` builds them), the library with collied's features:

| collied, macOS arm64 | Binary | Stripped | gzip -9 |
|---|---|---|---|
| Today (`alacritty_terminal`) | 43,568,816 | 36,786,664 | 16,112,394 |
| No emulator | 42,619,872 | 36,023,368 | 15,830,259 |
| libghostty-vt, static | 43,572,704 | 36,867,544 | 16,164,646 |
| libghostty-vt, dynamic | 42,620,176, plus a 1,239,896-byte dylib (1,040,336 stripped) | | |

So `alacritty_terminal` costs 0.95 MB and libghostty-vt 0.95 MB in its place: 3,888 bytes more, and 52 KB more in the archive (verified). The dynamic library saves nothing: the two files together are larger, and the dylib must be shipped, signed and found beside collied.

Linux x86_64 (collied not built, so inferred from a small C program linked with `zig cc`, which on macOS came within 21 KB of collied's own delta): the static library adds 1,267,680 bytes, 169,328 more with the snapshot decoder. The shared library is 1,266,896 bytes stripped (6.6 MB with the debug information the library keeps by default). Against roughly 1 MB that `alacritty_terminal` gives back (its macOS share; not measured on Linux), collied's Linux binary would grow by about 0.3 MB.

For scale, the v0.1.20 downloads: collied is 43,290,464 bytes on macOS (archive 15,973,225) and 53,342,088 on Linux (archive 18,022,392); libtailscale's Go runtime is most of it.

## Version bumps

The owner's question: can a new version be added easily? Today's iOS bump, from `scripts/ghostty/build-xcframework.sh`:

1. Check the new release's `SHA256SUMS` against its minisign signature with the Ghostty release key (`PACKAGING-LIBGHOSTTY.md`).
2. In the script: `LIBGHOSTTY_VT_VERSION`, `GHOSTTY_COMMIT` (the `vt-` tag's commit), `SOURCE_SHA256` and `DEPS_SHA256`; `ZIG_VERSION` and `ZIG_SHA256` when `minimum_zig_version` moves; the uucode license URLs and checksums when the dependency tarball's uucode moves.
3. Run `scripts/ghostty/nerd-font-constraints.py` again (the app's font tables).
4. Upload both tarballs to a `ghostty-vt-<version>` release of this repository, the script's fallback mirror.
5. CI follows by itself: the `ios` job's cache key hashes the script, and `scripts/ghostty/` is in the `changes` filter.

If collied linked the library, the pin could stay in that one file (or a pins file the script and collied's build both read), and a bump would still be steps 1 to 5, plus (inferred, nothing built):

- collied's build: `crates/ghostty-vt-sys/build.rs` links a library the script built for the target, so `cargo build -p collied` needs that library first (`just` recipe, CI step), or runs the script itself and needs Zig wherever collied builds: developer Macs, Linux builds from source, CI's `rust` job and both release jobs.
- `ci.yml`: the `rust` job builds or restores the macOS library (about 80 s cold, cached on the script's hash like the `ios` job).
- `desktop-release.yml`: the Linux job downloads Zig for x86_64 Linux (a second Zig checksum in the pins) and builds the library before `cargo build`; the macOS job builds it too. A release builds from scratch, so add about a minute and a half to each.
- `scripts/notices.py collied`: add the libghostty-vt licenses, as the `ios` notices do.
- The binding: only for new functions. 1.x promises additive ABI changes only and keeps the `.1` SONAME, checked by its own CI against a baseline.

What breaks a bump: a major version (new SONAME, any function may change); a new `minimum_zig_version`; and, with Rex streaming, the snapshot format, which has its own envelope version (1 at both `vt-1.0.0` and `vt-1.0.1`) outside the ABI promise. Rex writes snapshots with its own Ghostty, which moves on Rex's schedule, so collied would refuse an unknown envelope version and poll `format` instead.

Making it easier: the manual part is steps 1 to 4. A script that takes a version, checks the signature, rewrites the pins and prints what to rerun would turn a bump into one command and a reviewed diff; a scheduled check of Ghostty's `vt-*` tags against the pin would say when one is due. Neither is built.

## Planned work

- **Copilot input guard (#742)** reads the screen before each write to a Copilot agent. For Herdr Core that is the core's own `terminal.read`, with no emulator; for Rex it is one `format` read into collied's emulator, about 1 to 2 ms for a short terminal and up to 58 ms at 100,000 lines, once per prompt or key. It needs nothing libghostty-vt adds. #743 and #744 do not touch the emulator.
- **Rex's plain terminals (#755)** list shells under Terminals. herdr's shells show no grid preview, so listing them reads no screen; opening one watches it, which polls `format` like an agent's watch. Shells are where long scrollbacks live, so #755 is the likeliest place to meet the 45 to 58 ms poll. Following many terminals favors polling: a followed terminal holds a connection and a local copy whether or not a phone looks at it. Neither issue changes the answer; #755 is where to measure.

## Revisit when

- A watch of a long-scrollback Rex terminal measurably costs the Mac too much (for example while checking #755), or the phone starts showing many Rex screens at once. Then switch collied whole, both Herdr Core and Rex, with Rex streaming as its second step, as the [Follow-up](#follow-up) drafts describe.
- Rex gives `format` a range or a since parameter, or an output event: then polling stays cheap and the question closes.
- One of the four [differences](#4-fidelity) shows up on a real screen.

## Follow-up

One issue regardless of the answer: move the app's pin to 1.0.1 (the repository's latest-stable rule). If the owner decides to switch anyway, two more, in order: switch collied's emulator to libghostty-vt (the binding, the build, CI, release and notices, a synchronized-update hold, scrollback over-provisioning, Herdr Core and Rex); then follow Rex terminals as streams (snapshot, live bytes, `resize` messages, the fallback to `format`, `data.connect` in the allowlist and the threat model).

## Prototype

A throwaway outside the repository (scratchpad, not committed):

- `emucmp`: a Rust binary holding a copy of today's `screen.rs` and the same `Screen` on libghostty-vt (hand-written declarations for 13 C functions, grid references, about 300 lines), linked statically. `cases` runs the crafted cases on both, `cmp FILE COLS ROWS HISTORY [LINES]` compares the three reads on a recorded stream, `dump` prints both grids cell by cell, `rows` the history each keeps, `bench` the parse and read times, and `mem` one emulator alone for its peak memory.
- A scratch checkout of collied with that `Screen` as `runtime/screen.rs`, `alacritty_terminal` removed and a `build.rs` linking the library statically or dynamically: the release builds measured above, `cargo deny check`, and the `runtime::` tests.
- `use.c`: the calls collied needs, linked with `zig cc` for macOS and Linux against an empty program, for the Linux size.
