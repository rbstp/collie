# ADR-0005: libghostty-vt is built from pinned tarballs with a checked Zig, a private cache and ReleaseSafe in collied

- Status: accepted
- Date: 2026-11-20 (recorded from the comments of `scripts/ghostty` and `desktop-release.yml`)

## Context

The app renders terminals with libghostty-vt, and collied links it to parse Rex's streamed terminals. Both build it from source with Zig. docs/libghostty-vt.md covers why collied uses it and how a version bump is checked; this records how the builds are made.

## Decision

- `scripts/ghostty/pins.env` pins the release tarballs (SHA-256 from the release's minisign-checked `SHA256SUMS`), Ghostty's commit and Zig. `build-xcframework.sh` builds `Collie/GhosttyTerminal/GhosttyVt.xcframework` (iOS, the simulator, and macOS for `swift test`); `build-collied.sh`, run by `crates/ghostty-vt-sys/build.rs` and by `desktop-release.yml`, builds a static library per Rust target into `target/ghostty/collied/<target>`, `lib/libghostty-vt.a` and the license texts `scripts/notices.py` reads. `common.sh` holds what both share.
- Features: the app builds the render state only (no Kitty graphics and its file, temp-file and shared-memory image media, no input encoders, no snapshot, formatter or search; the terminal parser is always built). collied builds grid introspection, the formatter and the snapshot decoder: what its Rex and Herdr Core screens read.
- Every build gets an empty `ZIG_GLOBAL_CACHE_DIR`, which PACKAGING-LIBGHOSTTY.md requires (with Zig 0.17 a cache holding a package archive can crash the build), and a git ceiling, so Ghostty's build does not take this repository's tags for its version.
- collied's library targets a baseline CPU, not the build host's, since the release runners build for every machine. It is ReleaseSafe, unlike the app's ReleaseFast: collied parses untrusted program output in the process that holds its keys, so a slice-bounds, overflow or assertion fault in the Zig code aborts collied (launchd or systemd restarts it) instead of corrupting memory. Pointer arithmetic, use after free and the C++ parts (simdutf, Highway) stay unchecked.
- Apple's linker takes the dylib over the archive for `-l`, so the archive gets a folder alone. The xcframework's module map sits under `Headers/GhosttyVt`, not the Headers root, where it would collide with any other static xcframework doing the same once Xcode merges them into one include directory.
- Zig: on the hosts `pins.env` has a checksum for (arm64 macOS, x86_64 Linux), only the checked download is used; elsewhere a `zig` on `PATH` only at the pinned version. The marker is written only after a whole verified extraction, which is then moved into place, so an interrupted download is never trusted. A concurrent build (another script, another `CARGO_TARGET_DIR`) may have installed it meanwhile and be running it, so an installed one is kept. Zig's license sits beside the binary in Zig's tarball and one level up in Homebrew's.
- The shared cache: the pinned Zig and finished builds are shared by every checkout and worktree of the user, in `~/Library/Caches/dev.rbstp.collie-build/ghostty` on macOS and `$XDG_CACHE_HOME/collie-build/ghostty` elsewhere. What it holds is linked into collied and the app, so it is trusted as `target/` is: only a folder the user owns, set to 0700, is used; without one, nothing is shared and `target/ghostty` holds it all. A build is stamped with the pins and the scripts that made it, copied into the cache whole before it gets its name, so a reader never sees half of one, and a concurrent build may have stored it first.
- The sourcing scripts set no `EXIT` trap of their own; one that does must chain `common.sh`'s.
- When release.files.ghostty.org is down, the same tarballs are fetched from this repository's `ghostty-vt-<version>` release, and checked against the same pins.
- `scripts/ghostty/nerd-font-constraints.py` writes `NerdFontConstraints.swift` from Ghostty's `src/font/nerd_font_tables.zig` at the pinned commit; its `FIELDS` follow `NerdConstraint`'s memberwise initializer.

## Consequences

- A change to `pins.env`, `common.sh` or a build script rebuilds the libraries, locally and in CI, whose cache keys hash those files.
- A fault in libghostty-vt's checked Zig code restarts collied rather than leaving it running on corrupt memory.

## Code

- `scripts/ghostty/common.sh`, `scripts/ghostty/build-collied.sh`, `scripts/ghostty/build-xcframework.sh`, `scripts/ghostty/nerd-font-constraints.py`, `scripts/ghostty/pins.env`
- `.github/workflows/desktop-release.yml`
