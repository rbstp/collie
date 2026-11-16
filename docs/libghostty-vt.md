# libghostty-vt in collied

Spike for [#739](https://github.com/rbstp/collie/issues/739), 2026-11-15. collied emulates Herdr Core and Rex terminals with `alacritty_terminal` 0.26.0 (`crates/collied/src/runtime/screen.rs`) to produce the ANSI screens the phone reads. The phone renders with libghostty-vt, and herdr, Herdr Core and Rex all emulate with Ghostty's terminal. This page answers whether collied should move to libghostty-vt, with the evidence. It changes nothing in collied.

## Summary

- **Recommendation: switch collied to libghostty-vt, and follow a watched Rex terminal as a stream instead of polling `format`.** This reverses the first draft of this page, which recommended staying on `alacritty_terminal` without having measured a watch. Measured on a Rex server of its own ([Watch cost](#watch-cost)): while a phone watches a changing Rex terminal, today's polling costs Rex 6% of a core at 1,000 lines and 28 to 32% at 100,000 lines and near its scrollback cap, and each read takes 26 to 87 ms; a stream costs Rex 0.2 to 0.3% of a core above idle, answers a read locally in under 0.3 ms, and pays once per watch a snapshot of up to 31 MB in about 40 ms, plus up to 100 MB in collied while the watch lasts. A stream pays for itself within the first 3 s of a watch at 1,000 lines and within the first second near the cap. Switch Herdr Core to the same emulator rather than keep `alacritty_terminal` beside it: two emulators would mean two code paths and both libraries in the binary. The price is a Zig build in every collied build path, an unsafe binding, and the build work in [Version bumps](#version-bumps); binary size is not part of it.
- **Size: neutral.** On macOS arm64, collied is 43,568,816 bytes today, 42,619,872 without any emulator (so `alacritty_terminal` costs 0.95 MB), and 43,572,704 with libghostty-vt linked statically in its place: 3,888 bytes more than today (verified). On Linux x86_64 the library adds about 1.27 MB stripped (verified on a small program; collied itself was not built for Linux), against the 0.76 to 0.95 MB that `alacritty_terminal` gives back on macOS: about +0.3 to +0.6 MB (inferred).
- **Version bumps:** the pins live in one place, `scripts/ghostty/build-xcframework.sh`, but that script, and the license path `scripts/notices.py` reads, work only on arm64 macOS for the app. For collied to share the pin, the pins move to a file both read, the library build splits from the xcframework step so it runs on the Linux release runner, the notices read licenses from that build, and CI gains a Linux build of collied (none exists today). After that, a bump is one pins-file change plus the checks the iOS bump already needs; Rex streaming adds one more: the snapshot format is versioned apart from the library and outside its ABI promise ([Version bumps](#version-bumps)).
- **A newer release exists:** libghostty-vt 1.0.1 (tag `vt-1.0.1`, 2026-11-11, 91 commits after 1.0.0: resize and scrollback fixes, render fixes, simdutf fixes, snapshot code changes). The app pins 1.0.0, which the repository's latest-stable rule says to move ([Follow-up](#follow-up)).
- **Herdr:** yes, Herdr's GPUI client still does: Sheprd (the repository Herdr UI now redirects to, the client [herdr-core.md](herdr-core.md#shape) modelled collied's emulator on) depends on `alacritty_terminal` 0.26.0, the version collied pins. herdr and Herdr Core link libghostty-vt built with Zig 0.16, and `herdrdev/herdr-app` links Ghostty ([6](#6-herdr)).

## Evidence

Verified by running, on an M3 Max Mac (Darwin 27.3), with Zig 0.17.1 (the iOS pin), Rust 1.99.0 and collied at `e73fcad9`:

- libghostty-vt 1.0.0 built from the pinned, checksum-verified tarballs for macOS arm64 and, cross-compiled from the Mac with Zig alone, for Linux x86_64 (glibc 2.39), static and shared, with collied's feature set (`-Dvt-features=-all,+grid-introspection`) and with the snapshot decoder added (`+snapshot`).
- A throwaway prototype ([Prototype](#prototype)): collied's `Screen` reimplemented on libghostty-vt's C API, run beside a copy of today's `Screen` on the same bytes, and dropped into a scratch checkout of collied, where collied's own `runtime::` tests ran against it and release builds measured the size.
- `cargo deny check` with collied's `deny.toml`, on that scratch checkout and on a scratch crate depending on the crates.io bindings.
- The quick check recorded on the issue (Rex 0.1.0-preview.19, API revision 14, on a Rex server of its own): libghostty-vt reads Rex's `GHOSTSNP` snapshot and the stream continues byte-exact. Not redone here.
- The watch measurement in [Watch cost](#watch-cost), on a Rex server of its own (0.1.0-preview.19, commit `ad5a836c`, API revision 14), started as AGENTS.md says and stopped by its pid, with a standalone client, not collied.

Read, not run: the C headers and `PACKAGING-LIBGHOSTTY.md` of 1.0.0; the `vt-1.0.0...vt-1.0.1` comparison and the `vt-1.0.1` release files; `Cargo.toml`, `Cargo.lock` and the build scripts of herdr (`herdrdev/herdr` at `ee5bfb50`, 2026-10-29, herdr 0.9.5), Herdr Core (`herdrdev/herdr-core` at `bb8a2f77`, 2026-11-15), `herdrdev/herdr-app` (`8e1208de`, 2026-11-15) and Sheprd (`sheprd-labs/sheprd` at `80e82293`, 2026-11-16), all public; collied's runtime, approvals and their tests; `.github/workflows/` and `scripts/desktop/`.

Not verified: the watch cost through collied itself (the client is standalone), on Rex.app's server, or on Linux; a collied build for Linux (it needs a Linux Rust target and a cross Go toolchain for libtailscale; Docker was not running), so every Linux size of collied itself is inferred; the time a GitHub runner takes to build the library; a live Herdr Core or Rex terminal through the prototype (it ran on recorded bytes); how Rex.app and Herdr Core set Ghostty's grapheme mode 2027.

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
- **Binding:** collied's only C dependency today is libtailscale, through `crates/tailscale-sys`. A local `crates/ghostty-vt-sys` beside it, with hand-written declarations for the dozen functions collied calls, adds no third-party crate: on the scratch checkout, `cargo deny check` passed every check with `alacritty_terminal` removed and the binding in place. Removing `alacritty_terminal` drops 15 packages from `Cargo.lock`. The crates.io bindings (`ghostty-vt` and `ghostty-vt-sys` 0.9.0+ghostty-vt.1.0.1, one maintainer) fail collied's license check: their build script downloads the library with `ureq`, which brings `webpki-roots` and `webpki-root-certs` (CDLA-Permissive-2.0).
- **No C++ runtime:** the bundled simdutf and Highway link without one, and the macOS binary gained no dynamic library (verified with `otool -L`).
- **CI and release:** see [Version bumps](#version-bumps).

### 3. Rex streaming

Answered by the quick check on the issue: libghostty-vt's snapshot decoder reads the `GHOSTSNP` snapshot that opens Rex's data stream, and the live bytes after it reproduce Rex's own `format` exactly. The parser ignores Rex's `ESC _ rex1;...` messages; `ghostty_terminal_vt_write_until` can stop at APC boundaries, which would let collied pick out Rex's `resize` messages (read in `terminal.h`, not run).

The gain exists only if collied follows the stream instead of polling `format`: without streaming it is nil, since `format` is already Ghostty's output. A poll renders the whole scrollback on Rex's side, every 250 ms while a phone watches a terminal that keeps changing (every second once it has been still for 5 s). A stream costs, per watch, one snapshot, one held connection and a local copy, and then only the bytes the program prints. Scoped as collied's Herdr Core watch already is (one attach per watched terminal, closed with the last watch: `watch` and `Hold` in `runtime/herdr_core.rs`), collied would follow a Rex terminal only while a phone watches it.

#### Watch cost

Verified by running, with Rex 0.1.0-preview.19 (`ad5a836c`) on a server of its own and a standalone client (`rexbench`, [Prototype](#prototype)): an 80x24 terminal, filled with numbered lines, then printing one line every 0.2 s, so its screen changes between any two polls; 60 s per row. "Poll" does what collied's `runtime/rex/screen.rs` does for a watch: every 250 ms a `format` (VT, unwrapped, a 1-byte head) and a `blob_read` of the 64 KiB tail, emulated with a copy of today's `Screen` and read as 200 recent lines. "Stream" opens a data connection, restores the snapshot with libghostty-vt (snapshot, formatter and grid references built in), writes the live bytes into it, and every 250 ms reads the same 200 lines through the formatter. Each also makes a detection read every 5 s: the screen as text, what #742 adds before each write (by `format` for poll, locally for stream). Rex's CPU is its process's CPU time over the minute; the "no client" row is a terminal of the same kind with no client attached.

| | 1,000 lines | 100,000 lines (Rex kept 96,450; 4.5 MB of text) | Near the cap (96,128 full rows kept; 7.7 MB of text) |
|---|---|---|---|
| Rex CPU, no client | 0.6% of a core | 0.6% | 0.7% |
| Rex CPU, poll | 5.8% | 28.2% | 31.9% |
| Rex CPU, stream | 0.9% | 0.9% | 0.9% |
| Client CPU, poll / stream | 1.3% / 0.3% | 0.4% / 0.3% | 0.4% / 0.3% |
| Client peak memory, poll / stream | 7.2 / 5.0 MB | 6.8 / 88.2 MB | 6.4 / 100.6 MB |
| Bytes moved in the minute, poll / stream | 15.8 / 0.2 MB | 15.8 / 18.1 MB | 15.8 / 30.5 MB |
| Watch read, median (max), poll | 26 (80) ms | 79 (116) ms | 87 (102) ms |
| Watch read, median (max), stream | 0.18 (2.1) ms | 0.24 (1.0) ms | 0.19 (1.9) ms |
| Detection read, median, poll / stream | 22 / 0.01 ms | 44 / 0.02 ms | 49 / 0.02 ms |
| Snapshot: size, data ticket to restored terminal | 188 KB, 1.6 ms | 18.1 MB, 39 ms | 30.5 MB, 41 ms |

What the table says:

- Rex's scrollback cap (64 MiB, [rex.md](rex.md#4-screen)) is about 96,000 rows at 80 columns: asked for 100,000 short lines or 150,000 full ones, Rex kept 96,450 and 96,128. The third column is as near the cap as an 80-column terminal gets; a wider one keeps fewer, longer rows (inferred).
- A stream's whole cost to Rex over the minute, snapshot included, is 0.16 to 0.17 s of CPU above idle at every size, so the snapshot itself costs Rex little. Polling costs 3.1 s a minute above idle at 1,000 lines and 16.5 to 18.8 s near the cap. Break-even: a watch longer than about 3.3 s at 1,000 lines, 0.6 s at 100,000 and 0.5 s near the cap costs Rex less as a stream (an upper bound, counting the whole minute's stream cost as the snapshot's).
- The client's price for a stream is memory: the restored scrollback, up to 100 MB near the cap, held while the watch lasts, against 6 to 7 MB for polling. Asking the decoder to compress history (`GHOSTTY_SNAPSHOT_DECODER_OPT_COMPRESS_HISTORY`) did not lower this client's peak, since it keeps the received bytes until the decode ends; a client that drops them as it decodes should hold less (not measured).
- While polling, Rex's own memory rose from 130 to 200 MB to 530 to 580 MB, as [rex.md](rex.md#4-screen) saw for its four retained `format` results.
- Not measured: a still terminal (polling slows to one `format` a second, which still renders the whole scrollback; a stream then costs nothing, inferred), several watches at once, Rex.app as the server, a terminal wider than 80 columns, Linux.

Earlier figures, for comparison and on other terminals: [rex.md](rex.md#4-screen) measured a poll at 39 to 46 ms and a whole read through collied at 45 to 58 ms on 100,000 numbered lines, and a 4.5 MB snapshot in 65 ms; the quick check on the issue measured a poll at 50 to 55 ms and an 8.8 MB snapshot in 48 to 60 ms. The table's polls are slower because each pays a fresh `format` after a 250 ms gap (2 ms back to back at 1,000 lines, 6 ms after a gap, measured) and the first `blob_read` of each result. On the small program of [Size](#size) the decoder added 0.10 MB (macOS) and 0.17 MB (Linux) to a static link; collied built with the snapshot feature but not calling it grew 832 bytes (verified).

### 4. Fidelity

Every recorded screen reads the same (verified), on a small corpus: the Herdr Core fixtures (Claude Code 2.1.316 and Codex 0.166.0, at their recorded 200x120 with and without history, and narrowed to 80x24 and 60x20), the Rex fixture (Claude Code 2.1.325 at 198x68), Rex's `format` of 1,000 numbered lines, and 31 of 37 crafted cases: CJK, combining marks, emoji with VS15 and VS16, skin tones, ZWJ families (also under mode 2027), wide characters at the right edge, tabs, the 256 and RGB palettes, default colors, private-use glyphs, box drawing, the alternate screen, insert mode, scroll regions, DEC line drawing and the controls collied drops.

The six cases that differ come from four behaviors, each of which the phone can show:

| Bytes | alacritty | libghostty-vt | Phone |
|---|---|---|---|
| A flag (two regional indicators), then a cursor move to an absolute column | one column per indicator | two columns per indicator, so the move overwrites the second | the flag shows whole today, half with Ghostty |
| `ESC[21m` | turns bold off | double underline | Ghostty underlines the text |
| `ESC[2J` on a screen with content (two cases, and a third inside a synchronized update; in the recorded fixtures it appears only in the prologue of a paint, on an empty screen) | pushes the screen into history | erases it | today a `recent` read of a Herdr Core terminal shows screens a clear removed, which herdr, Herdr Core and Rex do not keep (inferred: they emulate with Ghostty) |
| An open synchronized update, read before it ends | the screen before it | the half-drawn update | see [1](#1-api-coverage) |

In each case libghostty-vt agrees with the terminal collied reads from, since herdr, Herdr Core and Rex all emulate with Ghostty. The differences should reach the phone only through Herdr Core reads and watches (inferred from the code paths, not observed), which feed the core's paint and then the program's own bytes to the emulator: a Rex read emulates Rex's `format`, which has already been through Ghostty. Without mode 2027 neither emulator joins a ZWJ family or a skin tone into one cell, and with it libghostty-vt does; collied re-emits the same characters either way, so the phone shows the same.

Other measurements on the same bytes: libghostty-vt parsed 1.4 to 1.8 times as fast on 5.1 MB of Rex output (246 to 300 MB/s against 166 to 176 MB/s) and 2.2 times as fast on the Codex fixture (160 against 73 MB/s). Memory is a wash at collied's limit: at 200 columns, holding collied's 1,000 lines of history cost `alacritty_terminal` 6.0 MB, and libghostty-vt 6.3 MB at the limit a binding must ask for to keep those lines (3,024, [1](#1-api-coverage)), 3.0 MB at a limit of 1,000.

### 5. Risk

What is pinned to the emulator's output:

- `runtime::screen` tests: the exact SGR form of a read (captured from Herdr Core 0.1.16's paint), soft-wrap joining, the blank row a paint leaves above the screen, a style across a wrap, the controls that never reach a read, the open synchronized update, and the two Herdr Core fixtures against the core's `terminal.read`.
- `runtime::rex::screen_tests`: the Rex fixture's VT emulated reads as Rex's `format` text, and a 1,000-line read of 100,000 lines of scrollback.
- Approvals, menus and fingerprints read the emulator only for Rex: `detection_text` is herdr's read for herdr and the core's own `terminal.read` for Herdr Core, but the emulated `format` text for Rex. A fingerprint hashes that text, so it changes only if the text does; the Rex fixture's text is identical.
- The `claude-*`, `codex-*` and `copilot-*` fixtures are text the parser reads, not emulator input.

How a switch would be verified: the existing tests are the bar. In a scratch checkout of collied (master `e73fcad9`, not kept) with the prototype's `Screen` as `runtime/screen.rs` and `alacritty_terminal` removed, `cargo nextest run -p collied --lib runtime::`, as CI runs tests, passed 122 of 123 (verified). The failure is the open synchronized update, which a switch must handle. Under plain `cargo test`, which runs every test in one process, one Rex tasks test also failed, a different one in each of two runs; the one examined set a process-wide `OnceLock` twice, and neither fails under nextest. Before the prototype asked for extra scrollback, the 1,000-line Rex read failed too. Then `live_herdr_core` and `live_rex`, and the Rex fixture through the snapshot path if streaming comes.

### 6. Herdr

Yes, in Herdr's GPUI client; no in herdr, Herdr Core and `herdrdev/herdr-app` (verified by reading, at the commits in [Evidence](#evidence)).

- herdr 0.9.5 builds a vendored libghostty-vt (Ghostty `44f2a44d`, "1.3.2-HEAD", from before the library's 1.0 release) through its own `crates/ghostty-vt`, a 466 KB generated binding whose `build.rs` runs Zig 0.16. herdr dropped its vt100 backend on 2026-04-05 (`b2e6929a`).
- Herdr Core builds a Ghostty submodule pinned at `1e5cdb86` (2026-10-11, 586 commits before `vt-1.0.0`) through its own `crates/ghostty-vt`, with Zig 0.16 and `-Dcpu=baseline`. The `hc` binary was not run.
- `herdrdev/herdr-app` links Ghostty through `crates/ghostty-sys`.
- Sheprd (`sheprd-labs/sheprd`, "Native client for Herdr", where `penso/herdr-gpui`, the Herdr UI that [herdr-core.md](herdr-core.md#shape) cites, now redirects) keeps `alacritty_terminal` 0.26.0 in its workspace dependencies, used by `crates/sheprd-terminal`, and in its lockfile. So the reason herdr-core.md gave for choosing `alacritty_terminal` (Herdr UI's `herdr-terminal` uses it) still holds for that client; the daemons themselves and the other app use Ghostty.

## Size

The owner's question: would it be large to include in collied? No. Release builds of collied `e73fcad9` for `aarch64-apple-darwin` (`MACOSX_DEPLOYMENT_TARGET=26.0`, as `desktop-release.yml` builds them), the library with collied's features:

| collied, macOS arm64 | Binary | Stripped | gzip -9 |
|---|---|---|---|
| Today (`alacritty_terminal`) | 43,568,816 | 36,786,664 | 16,112,394 |
| No emulator | 42,619,872 | 36,023,368 | 15,830,259 |
| libghostty-vt, static | 43,572,704 | 36,867,544 | 16,164,646 |
| libghostty-vt, dynamic | 42,620,176, plus a 1,239,896-byte dylib (1,040,336 stripped) | | |

So `alacritty_terminal` costs 0.95 MB and libghostty-vt 0.95 MB in its place: 3,888 bytes more, and 52 KB more in the archive (verified). The dynamic library saves nothing: the two files together are larger, and the dylib must be shipped, signed and found beside collied.

Linux x86_64 (collied not built, so inferred from a small C program linked with `zig cc`, which on macOS came within 21 KB of collied's own delta): the static library adds 1,267,680 bytes, 169,328 more with the snapshot decoder. The shared library is 1,266,896 bytes stripped (6.6 MB with the debug information the library keeps by default). The released Linux collied keeps its symbols but carries no debug sections (Cargo strips debug information from release builds; checked on the v0.1.20 binary), so the library's 9 MB of debug information should not reach it, and its symbols should add about 0.1 MB, as on macOS (inferred). Against `alacritty_terminal`'s share, 0.76 MB stripped or 0.95 MB with symbols on macOS (not measured on Linux), collied's Linux binary would grow by about 0.3 to 0.6 MB (inferred). A Linux build of collied settles it.

For scale, the v0.1.20 downloads: collied is 43,290,464 bytes on macOS (archive 15,973,225) and 53,342,088 on Linux (archive 18,022,392); libtailscale's Go runtime is most of it.

## Version bumps

The owner's question: can a new version be added easily? Today's iOS bump, from `scripts/ghostty/build-xcframework.sh`:

1. Check the new release's `SHA256SUMS` against its minisign signature with the Ghostty release key (`PACKAGING-LIBGHOSTTY.md`).
2. In the script: `LIBGHOSTTY_VT_VERSION`, `GHOSTTY_COMMIT` (the `vt-` tag's commit), `SOURCE_SHA256` and `DEPS_SHA256`; `ZIG_VERSION` and `ZIG_SHA256` when `minimum_zig_version` moves; the uucode license URLs and checksums when the dependency tarball's uucode moves.
3. Run `scripts/ghostty/nerd-font-constraints.py` again (the app's font tables).
4. Upload both tarballs to a `ghostty-vt-<version>` release of this repository, the script's fallback mirror.
5. CI follows by itself: the `ios` job's cache key hashes the script, and `scripts/ghostty/` is in the `changes` filter.

If collied linked the library, the pin could be shared, but not the script or the rest of the path as they stand (verified by reading):

- `scripts/ghostty/build-xcframework.sh` runs only on arm64 macOS: its Zig download refuses any other host, it calls `xcodebuild` and it writes into `Collie/GhosttyTerminal/GhosttyVt.xcframework`, with the app's features (`-all,+render-state`). The Linux release runner could not build the library through it.
- `scripts/notices.py` reads the libghostty-vt license texts only from the `licenses` folder that script writes into the xcframework, so `notices.py collied` could not produce collied's notices on Linux either.
- No CI job builds collied for Linux: `rust` runs on the macOS runner and `desktop-packaging` runs only the packaging scripts' Python tests. A broken Linux link would first show in `desktop-release`.
- Dependabot watches GitHub Actions, Cargo and one Go module, not the tarball pin, so nothing says when a release is out.

So "one file" would take this work first (inferred, nothing built):

- A pins file (version, commit, the two tarball checksums, Zig's version and a checksum per host: arm64 macOS, x86_64 Linux) that the iOS script and collied's build both read.
- A library build split from the xcframework step, for any host and target, with collied's features, writing the license texts where `notices.py` reads them for both the app and collied.
- collied's build: `crates/ghostty-vt-sys/build.rs` links what that step built, so `cargo build -p collied` needs it first (a `just` recipe and a CI step), or runs the step itself and needs Zig wherever collied builds: developer Macs, Linux builds from source, CI's `rust` job and both release jobs.
- `ci.yml`: the `rust` job builds or restores the macOS library (about 80 s cold, cached on the pins' hash like the `ios` job), and a Linux job builds collied, so a broken Linux link shows on the pull request.
- `desktop-release.yml`: both jobs build the library before `cargo build` (inferred: a minute or two each, from scratch).
- The binding: only for new functions. 1.x promises additive ABI changes only and keeps the `.1` SONAME, checked by its own CI against a baseline.

After that, a bump is steps 1 to 5 on the pins file, and CI rebuilds both the app's and collied's library.

What breaks a bump: a major version (new SONAME, any function may change); a new `minimum_zig_version`; and, with Rex streaming, the snapshot format, which has its own envelope version (1 at both `vt-1.0.0` and `vt-1.0.1`) outside the ABI promise. Rex writes snapshots with its own Ghostty, which moves on Rex's schedule, so collied would refuse an unknown envelope version and poll `format` instead.

Making it easier: the manual part is steps 1 to 4. A script that takes a version, checks the signature, rewrites the pins and prints what to rerun would turn a bump into one command and a reviewed diff; a scheduled check of Ghostty's `vt-*` tags against the pin would say when one is due. Neither is built.

## Planned work

- **Copilot input guard (#742)** reads the screen before each write to a Copilot agent. For Herdr Core that is the core's own `terminal.read`, with no emulator. For Rex it is one `format` read into collied's emulator: 22 to 49 ms in [Watch cost](#watch-cost), once per prompt or key, against 0.02 ms from a streamed terminal. A write usually comes while the phone shows that agent, so its terminal would be streamed already (inferred). #742 does not need the switch and works either way; it adds to the case for it. #743 and #744 do not touch the emulator.
- **Rex's plain terminals (#755)** list shells under Terminals. Its premise, that following many terminals is where a stream would matter, holds only if collied followed terminals all the time. Scoped to watches, a stream follows only the terminals a phone shows, and listing shells reads no screen either way (herdr's shells have no grid preview). What #755 changes is what gets watched: shells are where long scrollbacks build up (inferred), and there polling costs Rex 28 to 32% of a core; and each one a phone opens would be polled. So #755 strengthens the case for streaming, through long scrollbacks rather than through the number of terminals.

## What would change the answer

- Rex gives `format` a range or a since parameter, or an output event, so a poll stops rendering the whole scrollback: then polling could stay, and the switch would rest on the four [differences](#4-fidelity) alone, which do not justify it.
- A watch-scoped stream that holds 100 MB per long terminal proves too much for collied with several watches at once (not measured).

## Follow-up

Three issues, in order: move the app's pin to 1.0.1 (the repository's latest-stable rule, whatever the answer); switch collied's emulator to libghostty-vt (pins file, split build, notices, a Linux CI build, the binding, a synchronized-update hold, scrollback over-provisioning, Herdr Core and Rex, `alacritty_terminal` removed); then follow watched Rex terminals as streams (snapshot, live bytes, `resize` messages, the fallback to `format`, `data.connect` in the allowlist and the threat model).

## Prototype

A throwaway outside the repository (scratchpad, not committed):

- `emucmp`: a Rust binary holding a copy of today's `screen.rs` and the same `Screen` on libghostty-vt (hand-written declarations for 13 C functions, grid references, about 300 lines), linked statically. `cases` runs the crafted cases on both, `cmp FILE COLS ROWS HISTORY [LINES]` compares the three reads on a recorded stream, `dump` prints both grids cell by cell, `rows` the history each keeps, `bench` the parse and read times, and `mem` one emulator alone for its peak memory.
- A scratch checkout of collied with that `Screen` as `runtime/screen.rs`, `alacritty_terminal` removed and a `build.rs` linking the library statically or dynamically: the release builds measured above, `cargo deny check`, and the `runtime::` tests.
- `use.c`: the calls collied needs, linked with `zig cc` for macOS and Linux against an empty program, for the Linux size.
- `rexbench`: the [Watch cost](#watch-cost) client, in Rust with `tungstenite` over the Unix socket, a copy of today's `Screen` and libghostty-vt 1.0.0 linked statically. `setup` creates a session whose block runs the line printer, `idle`, `poll` and `stream` run one mode for a given time and report Rex's CPU time (`ps`), the client's own CPU and peak memory, and the read times; `destroy` ends the sessions. Driven by a shell script that starts the server with an empty environment, a scratch `HOME`, `XDG_RUNTIME_DIR`, `REX_RUNTIME_DIR` and `REX_CONFIG`, `--autostart=false`, `-S` and `--restore=false` on a socket under `/tmp`, and stops it by its pid.
