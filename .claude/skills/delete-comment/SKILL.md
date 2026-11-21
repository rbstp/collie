---
name: delete-comment
description: Clear the code comments of a path in this repository. Categorises each comment the comment ratchet counts as a tautology, stale, an excuse or a decision record, deletes it, files an issue for an excuse, moves a decision into an ADR under docs/adr, and lowers scripts/comment-baseline.txt. Use with /delete-comment <path>, or when asked to delete, clean up or categorise comments, or to lower the comment baseline.
argument-hint: <path>
---

# Delete comments

Clear the comments of `$ARGUMENTS` (a file or a folder) under ADR-0001 (`docs/adr/0001-comments-and-adrs.md`). Work in a worktree of your own and follow AGENTS.md.

## 1. List what the ratchet counts

```sh
python3 scripts/comment-ratchet.py --list $ARGUMENTS
python3 scripts/comment-ratchet.py --list --all $ARGUMENTS
```

The first prints every counted comment as `path:line:-:line|trailing:first line`; the second adds the allowed ones with their class. Leave a file another change has open alone.

## 2. Leave the comments that do work

These stay, whatever their wording, and the ratchet does not count them:

- Doc comments that become data: `///` inside an item deriving `JsonSchema` (`docs/protocol/*.schema.json`), a UniFFI `Record`, `Enum`, `Object` or `Error` or an `#[uniffi::export]` item (the Swift bindings), a clap `Parser`, `Subcommand`, `Args` or `ValueEnum` (`collied --help`), and the comment line right above a justfile recipe or its attributes (`just --list`).
- Directives: shebangs, `swift-tools-version:`, `swiftlint:`, `MARK:`, `shellcheck`, `go:build`.
- Pins: the `# vX.Y.Z` after an action pinned by SHA, and `scripts/ghostty/pins.env`.
- Pointers: a comment holding only `ADR-NNNN`.

If a counted comment does work the patterns miss (a test reads it, a tool parses it), stop and say so: the pattern in `scripts/comment-ratchet.py` and ADR-0001 change together, with a test in `scripts/test-comment-ratchet.py`.

## 3. Categorise each counted comment

Read the code around it, not only the next line.

- **Tautology**: every claim it makes is visible in the code next to it (names, types, the body). A comment that adds a fact the code does not show is not one.
- **Stale**: it names code, files, versions or behavior that are gone or changed. Check with `rg` before saying so.
- **Excuse**: it explains why a shortcut is fine (a sleep, a retry, a skipped check, a workaround the code could avoid) rather than why the design is right.
- **Decision record**: anything else; it documents a non-obvious decision.

## 4. Act

- Tautology and stale: delete.
- Excuse: delete, and file one issue per hack, related hacks of one area together: `gh issue create -R rbstp/collie --milestone "<current phase milestone>"` (`gh api repos/rbstp/collie/milestones` lists them). A plain imperative title; the body says what is wrong, where (file and function), and what is wanted. No em dashes.
- Decision record: find the ADR that already holds the decision (`docs/adr/README.md`) and add to it, or write a new one: `docs/adr/NNNN-short-title.md`, the next number, with a status, a date, Context, Decision, Consequences and the code it governs. Group every comment of one decision into one ADR; never one ADR per comment. Keep the facts the comment held (numbers, versions, issue numbers, the reason). Add a new record to the index in `docs/adr/README.md`.
- Leave a bare `ADR-NNNN` pointer (the language's comment syntax, nothing else on the line) only where the invariant is security relevant or the code looks wrong without it.

Security-critical code (`crates/collied/src/push*`, pairing, peers, `crates/tls`, the Keychain, and the notification code in `Collie/Sources`) gets a pass of its own, never folded into a larger one.

Deleting a comment must not leave a dangling "see above", an empty block, two blank lines in a row, a blank line at the start of a block, or a neighbor that now misleads. A recipe doc line of the justfile that was the tail of a longer comment is rewritten into one line that stands alone.

## 5. Lower the baseline

```sh
python3 scripts/comment-ratchet.py --update
python3 scripts/comment-ratchet.py
```

`--update` refuses to raise a count. A renamed or split file with counted comments fails until its comments go, or its baseline entry is moved by hand in the same change.

## 6. Check that only comments changed

- `git diff` holds removed comment lines, removed blank lines and added `ADR-NNNN` pointers only, plus `docs/adr/`.
- `just lint` and `python3 scripts/test-comment-ratchet.py`.
- Rust: `cargo nextest run -p <crate>` for each crate touched; for collied add `-E 'not test(round_trip_on_a_temporary_keychain)'`, and never run that keychain test locally.
- `just schema` leaves `docs/protocol/` unchanged; `collied --help` for each subcommand and `just --list` read as before.
- Swift: the iOS unit tests (`just ios-test`, skipping `CollieUITests`) for `Collie/`, `just mac-test` for `Mac/`.
- `relay/`: `npm run check` and `npm test`. `Linux/`: `node --test Linux/Tests`.
- Docs: the site check in AGENTS.md.

A pull request that only deletes comments and adds ADRs takes `[skip-release]` in its title, and its body gives the totals per category, the ADRs, the issues filed and what the baseline still holds.
