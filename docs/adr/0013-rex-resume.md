# ADR-0013: A Rex agent resumes only in its own restored block, once, at an empty prompt

- Status: accepted
- Date: 2026-11-23 (#927)

## Context

When the Rex server restarts, or the Mac reboots, Rex restores its sessions, windows and blocks under the ids they had, each block as a new shell. The Claude Code, Codex or Antigravity CLI conversation that ran in a block is gone until someone types its resume command. collied already knows each Rex agent's kind, conversation and folder, and can type into a block. Typing is the most sensitive thing collied does on the machine: a line typed in the wrong place, or twice, runs as a command.

Checked on a scratch server (Rex.app 0.1.0 build 2104, API revision 19, restore on, its own state folder): after a restart the session and block ids were the same, `server.status` named a new `instance_id` and a new `started_at`, a shell block came back as a shell in the folder its shell was last in, and a block started with a command (as collied starts a task) came back as a shell in that block's folder. Stopping the server with SIGTERM while a program ran in a shell sent no `process_changed` or `child_exited` before the connection closed. `format` with `extra_cursor` adds the cursor position to the VT rendering, also on the revision 13 collied speaks (checked by `live_rex_resume`).

## Decision

- A restart is a new `instance_id`. collied keeps the instance with its agents in `rex-resume.json` (0600, data folder, flushed on each change) and plans resumes only when it follows another instance than the recorded one, never on a reconnect to the same one, and only while that instance's `started_at` is less than 90 s old, so blocks used while collied was off are left alone. A server without `instance_id` or `started_at` gets none.
- The record holds what collied lists: a Claude Code, Codex or Antigravity CLI agent with a bound conversation and an absolute foreground folder, by block id, at most 256. An agent that leaves its block stays recorded for 15 s while collied follows the same instance, then goes: a shutdown or logout ends the agents before the server stops, and collied cannot tell that from a quit. Copilot CLI has no conversation collied binds in Rex, so it is never recorded.
- Planning takes the agents off the record and saves it before anything is typed, so a crash or a restart of collied never types a line twice.
- A planned block gets its line only when it is on the new instance, has not exited and has a shell in front (by `argv0`); that shell's folder is the recorded one, as Rex reports both, so no path is ever typed; no listed agent runs the conversation and no block got it on this instance, so a conversation recorded for two blocks opens in one; and the screen, read with `format` VT and `extra_cursor` through collied's emulator, has the cursor after a blank, a prompt before it on its row and nothing below it. A block in another folder, or whose conversation already runs, is dropped; one not ready is read again on each reconcile tick until the 90 s have passed.
- The line is fixed text and validated ids only: `claude --resume ID`, `codex resume ID` or `agy --conversation ID`, ID a lowercase hyphenated UUID. No other flag, no permission flag, nothing read from a screen. It goes as Ctrl+E, Ctrl+U, a paste, then Enter, each only while the block is still listed as a terminal, so text typed after the screen check is cleared rather than run.
- The option is on by default. `rex-resume-off` in the data folder turns it off, and turning it off deletes the record. The control socket's `rex_resume` request reads it and sets it, audited, and CollieBar shows it as a menu item; CollieTray has none, since collie follows Rex on the Mac only.

## Consequences

- A conversation quit less than 15 s before the server stopped reopens.
- A restored shell used and left at an empty prompt in the same folder within the 90 s gets the line; one with text at its prompt does not, and a vi keymap keeps text after the cursor, which runs with the line.
- An agent started outside its shell's folder (`sh -c 'cd x && claude'`) is not reopened. Rex.app's restored login shells were not checked against the owner's server; Rexume, which also types into them, relies on the same restored folder.
- A line typed into a shell runs through the user's own aliases and functions for the agent's command.

## Code

- `crates/collied/src/runtime/rex/resume.rs`, `crates/collied/src/runtime/rex/input.rs` (`type_line`), `crates/collied/src/runtime/rex/model.rs` (`idle_shell`), `crates/collied/src/runtime/screen.rs` (`at_empty_prompt`), `crates/collied/src/control.rs` (`rex_resume`)
- `Mac/CollieBar/ControlClient.swift`, `Mac/CollieBar/Daemon.swift`, `Mac/CollieBar/CollieBarApp.swift`
- `crates/e2e/tests/live_rex.rs` (`live_rex_resume`)
