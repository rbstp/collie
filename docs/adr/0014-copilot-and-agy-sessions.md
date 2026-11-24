# ADR-0014: Copilot CLI and Antigravity CLI sessions in Herdr Core and Rex come from the files their process holds, and bind nothing when those files name more than one

- Status: accepted
- Date: 2026-11-24 (#744)

## Context

In herdr, collied finds a Copilot CLI conversation from the pane's processes and title, and herdr's own hooks report an Antigravity CLI (`agy`) conversation. Herdr Core and Rex have neither: herdr's hooks exit outside herdr, and collied lists a Herdr Core terminal or a Rex block by its foreground process only. Without a session, a Copilot CLI or agy agent there had no Chat and no running subagents, and was left out of nothing in `task.sessions` and `task.new`'s resume check, so a running conversation could be resumed a second time.

What the CLIs leave (checked in #741 and #932, and on the owner's machine by file name only):

- Copilot CLI keeps `session-state/<id>/inuse.<pid>.lock` naming its pid and holds an `flock` on `inuse.<pid>.hold` from its start, before any prompt, beside a `workspace.yaml` naming the session and the real folder; its `events.jsonl` appears only with the first prompt (package 1.0.115, a new session on a scratch home, at its folder trust prompt). After `/new` the same process holds markers in both sessions until it exits. A killed process leaves both files unlocked; one that gets SIGTERM removes them.
- agy keeps `presence/<id>.lock` under its data folder open read-write for its conversation before any prompt, beside stale locks of past conversations no process has open. Its hook (`SessionStart`, `PreInvocation`) runs only with a prompt, and its report lives in collied's memory. Whether agy flocks the lock was not shown live.
- An agy transcript (`brain/<id>/.system_generated/logs/transcript.jsonl`) names no folder. Its prompt history, `history.jsonl` in its data folder, has one entry per typed prompt with `workspace` and, for all but `/` commands, `conversationId`. On the owner's machine every conversation missing from it was one agy marks internal.

## Decision

- Copilot CLI: the session whose marker the foreground process holds, checked as for Rex resume (ADR-0013): the `flock` held, `inuse.<pid>.lock` naming that pid, the process started before the marker, `workspace.yaml` naming the id and the process's folder, `events.jsonl` starting with that id's `session.start`. Exactly one such session binds; two (after `/new`), a folder collied cannot read, or a listing past its cap bind none.
- agy: the presence locks the foreground process has open (device and inode against its open files) and the hook's report for that process. With no report, exactly one open lock binds. A report binds when the process has no presence lock open (the lock may not exist yet), or has the reported one open among those it has. When the report names a conversation the process has no lock open for, nothing binds: a stale report and a stale lock look alike, and no binding is better than a wrong one. An inspection that cannot finish (a presence folder that cannot be read or is past its cap, descriptors that cannot be listed whole) binds nothing and clears the binding; only a missing presence folder counts as no lock.
- Both are read on the reconcile tick (1 s) and, in Rex, when a block is read or reports, always outside the lock listings take, and kept with the foreground pid, so a binding holds only while that process leads the terminal. A read that finds none clears the binding, unlike Claude Code's pid file, which a later read keeps: a Copilot CLI after `/new` must lose its first session. An agy report is checked against the locks as it arrives, and a report Rex remembers across a reconnect is never shown before that check. In Rex each read of a block takes a scan number and commits only if no later read of that block started, so an older read never overwrites a newer one; Herdr Core reads only from the reconcile loop.
- `task.sessions` lists Copilot CLI sessions from `workspace.yaml` (the folder) and `events.jsonl` (the first typed prompt), and agy conversations from the oldest `history.jsonl` entry naming them still in its last 1 MiB (the folder, which can change once older entries leave that window) and their transcript (the first explicit prompt). Copilot CLI sessions are filtered by folder before the 400 newest transcripts are opened, and each kind is capped at 20 on its own. A Copilot CLI session any process holds, or whose folder collied cannot read, and an agy conversation whose presence lock is flocked, open in any of the user's processes named `agy`, or cannot be checked whole, are neither listed nor resumable, whatever binds. Rex resume waits on the same agy check. `task.new` resumes the same sources, `copilot --resume=<id>` and `agy --conversation <id>`, only where the runtime starts that kind.

## Consequences

- A new Copilot CLI session binds with its first prompt, when `events.jsonl` appears; a resumed one binds at once. After `/new`, a Copilot CLI agent in Herdr Core or Rex has no Chat or running subagents until it exits; Terminal works.
- An agy agent whose hook is not installed gets Chat from its lock alone. An agy that keeps several presence locks open has Chat only once its hook names one of them.
- An agy conversation started outside the CLI, or whose prompts are older than the last 1 MiB of its history, is not listed for Resume. An agy running under another process name is not found holding its lock.
- Each tick lists `session-state` once for all Copilot CLI agents, reading a session folder's names again only when its mtime or ctime changed and checking a marker's `flock` only for an agent's pid, and lists `presence` once; a listing reads none.

## Code

- `crates/collied/src/runtime/agents.rs` (`open_files`, `presence_locks`, `agy_open`, `agy_lock`, `agy_session`, `agy_holders`, `agy_held`), `crates/collied/src/runtime/herdr_core.rs` (`reread_sessions`, `session`), `crates/collied/src/runtime/rex/sessions.rs` (`read_files`), `crates/collied/src/runtime/rex/model.rs` (`reads`, `scan`, `bind_scan`)
- `crates/collied/src/transcript.rs` (`sessions`, `has_session`, `in_use`), `crates/collied/src/transcript/live.rs` (`CopilotMarks`, `copilot_bound`, `copilot_session`, `copilot_selection`), `crates/collied/src/drive.rs` (`task_sessions`, `resumable`), `crates/collied/src/runtime/herdr.rs` (`agent_args`)
- `crates/e2e/tests/live_rex.rs`, `crates/e2e/tests/live_herdr_core.rs` (`live_*_phone`), `crates/e2e/tests/common/mod.rs` (`copilot_stand_in`)
