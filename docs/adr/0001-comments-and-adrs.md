# ADR-0001: Decisions live in ADRs; comments are data, directives or pointers

- Status: accepted
- Date: 2026-11-20

## Context

Code comments drift from the code they describe. A comment that repeats the code costs reading time and goes stale; a comment that excuses a shortcut lets the shortcut spread; and a decision written beside one line of code is invisible from every other place it shapes. The repository's rule was "Comments are critical-only", which left the judgment to each change and let the count grow to about 4,900 comment lines.

Some comments are not prose but input to a tool, and deleting them changes what the build produces.

## Decision

A comment may stay only in one of these classes, which `scripts/comment-ratchet.py` allows by pattern:

- **Data.** Rust `///` doc comments inside an item that derives `JsonSchema` (the descriptions in `docs/protocol/*.schema.json`), a UniFFI `Record`, `Enum`, `Object` or `Error` or carries `#[uniffi::export]` (the doc comments of the generated Swift bindings), or a clap `Parser`, `Subcommand`, `Args` or `ValueEnum` (`collied --help`). The comment line right above a justfile recipe or its attributes (`just --list`).
- **Directives.** Shebangs, `swift-tools-version:`, `swiftlint:`, `MARK:`, `shellcheck` and `go:build`.
- **Pins.** The version comment after a GitHub Action pinned by commit SHA (`uses: owner/action@<sha> # v1.2.3`), which AGENTS.md requires, and the provenance comments of `scripts/ghostty/pins.env`.
- **Pointers.** A comment holding only `ADR-NNNN`, naming a record in this folder, at the code that record governs. A pointer is used only where the invariant is security relevant or the code looks wrong without it.

Every other comment is one of four kinds, each with its action:

- A **tautology** repeats what the code shows: deleted.
- A **stale** comment describes code or behavior that is gone: deleted.
- An **excuse** lets a hack stand: deleted, and an issue is filed to fix the hack.
- A **decision record** documents a non-obvious decision: moved into an ADR here, grouped with the other comments of the same decision, and deleted from the code.

The ratchet: `scripts/comment-ratchet.py` counts, per tracked file in scope, the lines that hold a comment outside the classes above, and compares them with `scripts/comment-baseline.txt`. A count above its baseline fails, and so does any such comment in a file the baseline does not list. A lower count passes; `python3 scripts/comment-ratchet.py --update` writes the lower counts and refuses to raise any. `just lint` runs the check and CI runs it with the script's own tests. The `/delete-comment` skill (`.claude/skills/delete-comment/SKILL.md`) repeats the categorisation on a path.

Scope: Rust and Go under `crates/`, Swift under `Collie/` and `Mac/`, TypeScript under `relay/`, JavaScript and QML under `Linux/`, Python, shell and `.env` files under `scripts/`, the shell scripts of `docs/site/` and `packaging/aur/`, the justfile and the GitHub workflows. Test fixtures and generated files (`Linux/CollieTray/Printable.js`, `NerdFontConstraints.swift`) are out of scope, as are TOML, XcodeGen YAML, SQL and CSS.

## Consequences

- A file's comment count only goes down. Renaming or splitting a file that still holds counted comments fails the check, since the new path has no baseline: delete the comments, or move the entry by hand in the same change, where the reviewer sees it.
- `// SAFETY:` comments are counted: no lint enabled here asks for them (`clippy::undocumented_unsafe_blocks` is a restriction lint). Python docstrings are counted as comments.
- Doc comments that are data stay where they are: removing one changes the generated schemas, bindings or help, which `just schema`, the protocol schema test and a diff of `collied --help` show.
- A change that only touches `docs/site/*.sh` skips the CI job that runs the check (`ci.yml`'s `changes` job), so such a change is checked by `just lint` locally and by the next change that runs `rust`.

## Code

- `scripts/comment-ratchet.py`, `scripts/test-comment-ratchet.py`, `scripts/comment-baseline.txt`
- `justfile` (`lint`), `.github/workflows/ci.yml` (`rust` job)
- `.claude/skills/delete-comment/SKILL.md`, `AGENTS.md`
