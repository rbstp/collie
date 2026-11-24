# ADR-0013: A Rex agent resumes once, in place of its own untouched restored shell, never typed

- Status: accepted
- Date: 2026-11-23 (#927; revised after review the same day)

## Context

When the Rex server restarts, or the Mac reboots, Rex restores its sessions, windows and blocks under the ids they had, each block as a new shell. The Claude Code, Codex or Antigravity CLI conversation that ran in a block is gone until someone runs its resume command. collied already knows each Rex agent's kind, conversation and folder.

The first design typed the resume command into the restored shell. Review showed screen checks cannot make that safe: in a vi keymap text after the cursor survives Ctrl+E and Ctrl+U and runs with the line, a shell waiting in a builtin `read` looks like an idle prompt and the typed Enter answers it, and a program started between the checks and the write (ssh, a REPL) or a `cd` receives the line.

Checked on a scratch server (Rex.app 0.1.0 build 2104, API revision 19, restore on, its own state folder): after a restart the session and block ids were the same, `server.status` named a new `instance_id` and a new `started_at`, a shell block came back as a shell in the folder its shell was last in, and a block started with a command came back as a shell in that block's folder. Stopping the server with SIGTERM while a program ran in a shell sent no `process_changed` or `child_exited` before the connection closed. Rex has no call that replaces a block's program, but `session.new_split` anchored on a block, followed by `block.close` of that block, leaves the new block in the old one's place: the same split id, ratio and rect, and the focus. `format` with `extra_cursor` adds the cursor position to the VT rendering, also on the revision 13 collied speaks.

## Decision

- Nothing is typed. The agent starts as an argument vector in a new block, `session.new_split` anchored on the restored block (`side: before`) with a shell-flavor block whose `options.command` is what a phone's resume task runs (`/usr/bin/env -u` with each stripped name, the CLI by absolute path, then `--resume ID`, `resume ID --no-daemon` or `--conversation ID`, as `runtime::herdr::agent_args` gives them) and whose `cwd` is the recorded folder. No shell parses it. The new block is given up, then the restored block is closed, so the agent takes its place.
- A restart is a new `instance_id`. collied keeps the instance with its agents in `rex-resume.json` (0600, data folder, flushed on each change) and plans only when it follows another instance than the recorded one, whose `started_at` is less than 90 s before now and not in the future. The plan lasts until 90 s after that start, measured from the planning moment on collied's monotonic clock, so a clock change does not extend it. A reconnect to the same instance keeps the plan and the grace timers.
- The record holds what collied lists: a Claude Code, Codex or Antigravity CLI agent with a bound conversation and an absolute foreground folder, by block id. It is never read past 256 KiB, and what is read keeps at most 256 agents with a valid block id, a known kind, a lowercase hyphenated UUID and a folder of at most 4,096 bytes. An agent that leaves its block stays recorded for 15 s while collied follows the same instance, since a shutdown ends the agents before the server stops, then goes. Copilot CLI has no conversation collied binds in Rex, so it is never recorded.
- Planning takes the agents off the record and plans only once that is saved, so a crash or a restart of collied never starts one twice.
- A planned block is replaced only when, read just before acting: it is on the new instance with a shell in front, and a fresh `process` read shows the shell running nothing (it is the block's own program, or the only process under Rex.app's `login`); the screen shows the cursor after a blank, a prompt before it and nothing at or after it on its row or below; the recorded folder is a directory; no agent in the live model runs the conversation; and a claim under the lock the option's toggle takes finds the block planned, the option on, the window open and the conversation not started on this instance. After the new block starts, a second `process` read must show the same idle shell, and the option and window must still hold, before the old block is closed; otherwise the new block is closed and the user's stays.
- The option is on by default. `rex-resume-off` in the data folder turns it off, and turning it off deletes the record. The control socket's `rex_resume` request reads it and sets it, audited, and CollieBar shows it as a menu item; CollieTray has none, since collie follows Rex on the Mac only.

## Consequences

- The resumed agent is its block's program: when it exits, the block closes, as a task's does, instead of returning to a shell.
- A shell the user starts using in the round trip between the second process read and the close is closed with what was typed in it. A startup file waiting in `read` at a prompt ending in a blank looks idle, and its shell is closed with the question unanswered.
- A restored shell used and left at an empty prompt within the window, and a conversation quit less than 15 s before the server stopped, get the agent back.
- A prompt that does not end in a blank, or that draws text after the cursor (a right prompt), is never replaced.
- Rex.app's restored login shells were not checked against the owner's server.

## Code

- `crates/collied/src/runtime/rex/resume.rs`, `crates/collied/src/runtime/rex/model.rs` (`shell_block`), `crates/collied/src/runtime/screen.rs` (`at_empty_prompt`), `crates/collied/src/runtime/rex/client.rs` (`session.new_split`), `crates/collied/src/control.rs` (`rex_resume`)
- `Mac/CollieBar/ControlClient.swift`, `Mac/CollieBar/Daemon.swift`, `Mac/CollieBar/CollieBarApp.swift`
- `crates/e2e/tests/live_rex.rs` (`live_rex_resume`)
