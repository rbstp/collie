# Rex backend plan

Spike for [#703](https://github.com/rbstp/collie/issues/703), 2026-11-12. This page is the plan, the evidence behind it and the follow-up issues for "Phase 8: Rex integration". Nothing here is built yet. Most of the design reuses herdr-core's: its plan is [herdr-core.md](herdr-core.md), and what collied does with it is in [architecture.md](architecture.md#herdr-core-herdr-2).

## Summary

- Rex 0.1.0-preview.16 (API revision 13) already gives collied what it needs to follow Claude Code agents beside herdr's, with no hook and no polling for status. Each terminal block keeps the OSC 7501 status its programs report (`program_status`, pushed as `program_status_changed`) and its foreground process (`process`, pushed as `process_changed`), reads its screen as text or escape sequences (`format`), and takes pastes and named keys (`block.event`). The issue's list of what Rex leaves undocumented predates this build: the server documents its whole API, both WebSocket protocols included, at `/llms.txt` on its own socket.
- Claude Code 2.1.322 reports `idle`, `working`, `blocked` (`kind: permission`, `msg: approve Bash: touch rex-spike-probe.txt`) and `done` in a Rex block, as it does under herdr-core, and nothing at its folder trust prompt (verified). Codex and Copilot CLI were not run here: Codex reports OSC 7501 under herdr-core and Copilot CLI does not, so a Copilot agent would be read-only from the phone.
- collied should talk to the control WebSocket (`/api/control`, subprotocol `rex.control.v1`), not run the CLI. A call costs 0.2 to 0.3 ms on an open connection against 25 to 35 ms for each `rex` process, and status, process and title changes arrive as events. collied already depends on `tokio-tungstenite`.
- collied's approval parser, menu gate and fingerprint read Rex's `format` text unchanged, and its `alacritty_terminal` emulator fed with `format`'s VT output draws the same screen (verified). Reads and watches poll `format`, as collied polls herdr: every poll is the terminal's whole current state, so no output is applied twice or lost. A poll costs about 1 ms at 1,000 lines of scrollback and 39 to 46 ms at 100,000. The data stream is not used: nothing lines it up with `format`, and it opens with a binary Ghostty snapshot (4.5 MB at 100,000 lines) that `alacritty_terminal` cannot read ([Screen](#4-screen)).
- Lost or weaker for a Rex agent: `terminal.send`'s `if` and its typing hold, which Rex has no equivalent for, closing a whole Rex session from the phone (refused, as for a herdr-core group), and probably desktop focus. Built in collied: the agent kind, the session binding (Claude Code's `sessions/<pid>.json`, as for herdr-core), `state_change_seq`, task records and worktrees.
- Shape: a third runtime beside herdr and herdr-core, `crates/collied/src/runtime/rex.rs`, with ids `rex:block:<uuid>`, one group per Rex session (`rex:session:<uuid>`), and the phone protocol unchanged.

## Evidence

Verified by running, on macOS (Darwin 27.3), with the `rex` CLI inside Rex Beta.app (`Contents/Helpers/Rex Server.rex/Contents/MacOS/rex`: 0.1.0-preview.16, build 1940, API revision 13; nothing was installed), Claude Code 2.1.322 (`--model haiku`), and the user's herdr 0.9.5 and herdr-core 0.1.19 running their agents:

- The throwaway prototype [below](#prototype), against a Rex server it starts for itself, and exploratory sessions against two more scratch servers (the second for the plan's review: every key over the socket, two identical prompts in a row, poll and snapshot costs).
- The server's self-description: `rex api list --json` and `help.all` (74 methods with JSON Schemas), `block_creator.list`, `session.describe_block`, and `/llms.txt`, which the server serves on its socket.
- collied's `prompt::Menu::parse`, `prompt::context`, `approvals::fingerprint` and `runtime::screen::Screen` on Rex screens, through a throwaway binary depending on collied by path.

How the spike kept off the user's Rex, whose server (Rex.app's, build 1858) ran throughout: apart from the first `--help` listings, every `rex` process, every scratch server and everything they started ran under `sandbox-exec` with a profile denying reads and writes in `~/Library/Application Support/rex` and `~/Library/Application Support/com.superlogical.rex` and any connection to the user's socket. Each started from an empty environment with a scratch `HOME`, `XDG_RUNTIME_DIR`, `REX_RUNTIME_DIR` and `REX_CONFIG`, and every CLI call passed `--autostart=false` and `-S` with the scratch socket. The user's two folders were compared (names, sizes, modes and mtimes) before and after every phase: unchanged, apart from files the user's own server rewrites by itself (its block snapshots, its log, its Tailscale node's logs and netmap cache), which also changed before the first scratch server started. The user's server was never queried, so nothing here says what build 1858 supports. The sandbox kept the scratch servers from running `/usr/bin/login`, a setuid binary, so their blocks started with `shell: none`; Rex.app's terminals start through `login` on macOS.

Read, not run:

- The four Rex pages the issue links. They document neither the WebSocket protocols, nor OSC 7501, nor where the socket lives; `/llms.txt` documents the first two, and the CLI's own errors on scratch paths showed the third.
- Rex's [shared server](https://www.superlogical.com/rex/docs/linux/shared-server) and [remote access](https://www.superlogical.com/rex/docs/linux/remote-access) pages, for who else can reach a server ([Alongside the others](#10-alongside-the-others)).
- Rex.app's binary, for its client actions and the message it answers them with while Remote Control is off. Rex.app was never opened.
- collied's runtime (`crates/collied/src/runtime`), [herdr-core.md](herdr-core.md) and [architecture.md](architecture.md#herdr-core-herdr-2).

Not verified: Codex and Copilot CLI in Rex, collied's own lookup of Claude Code's session file ([Mapping](#mapping)), a program with a keyboard protocol, Linux, Rex.app (the size it gives a block it owns, whether it follows a focus change made on the server), a server started by `--autostart` (what environment it passes on), sessions restored after a server restart, and builds other than 1940.

## What Rex is

- One server per user, owning sessions, their windows (tabs) and their blocks (today, terminals). Clients such as Rex.app and the CLI connect to it. Ids (`session:<uuid>`, `window:<uuid>`, `block:<uuid>`, UUIDv7) never change and are never reused; labels can change and repeat.
- The CLI finds the local server at `$REX_RUNTIME_DIR/server.sock`, else `$XDG_RUNTIME_DIR/rex/server.sock`, else, on macOS, `~/Library/Application Support/rex/server.sock`, where Rex.app's server listens; `REX_SERVER` and `-S` name another server (verified from the CLI's errors on scratch paths). Linux without `XDG_RUNTIME_DIR` was not checked (the API examples in the binary show `/run/user/1000/rex/server.sock`). A socket path over 103 bytes is refused. A personal server's socket is `0600` in a `0700` folder, and the server admits callers by their peer credentials. Rex also documents a shared server on Linux (`/run/rex/server.sock`, open to the `rex` group, programs run as `rex`) and network listeners, all off by default.
- Two WebSockets on that socket. `/api/control` (`rex.control.v1`): a `hello` naming the API revisions the client speaks, a `welcome`, then JSON requests, responses and events; each response carries the `seq` of the last event it already reflects. `/api/data`: one block's output per connection, opened with a single-use ticket from `data.connect`.
- Events: a connection receives a session's events, block events included, once it calls `session.attach` (or `session.bootstrap`); `session.attach_all` with `event_scope: session` adds sessions being created and destroyed but no block events, and `event_scope: all` adds every client joining and leaving (each CLI call is four events). `seq` counts per connection and starts again at 1 on a new one.
- A terminal block's methods include `format` (text, HTML or VT, screen and scrollback), `program_status`, `process`, `title`, `size`, `modes`, `write`, `resize`, `blob_read`, `list_dir`, `upload_begin` and `download_begin`. Its events include `program_status_changed`, `program_status_removed`, `process_changed`, `title_changed`, `size_changed`, `child_exited` and `desktop_notification`.
- Every terminal gets `REX_SESSION`, `REX_BLOCK`, `REX_BIN_DIR`, `TERM_PROGRAM=rex`, `COLORTERM=truecolor` and the server's own environment (verified: the scratch server's `REX_CONFIG` and `XDG_*` reached its blocks). The docs add `REX_SERVER`; a server started with `rex server run` did not set it.
- A client may own a block, which then takes that client's size and colors; a client that creates a block owns it. A block kept its creator (the CLI) as owner after that client left, and another client's resize then waited (seen). A block no client owns or sized is 80x24.
- The server saves each terminal's screen to disk (`<state>/terminal/<pid>-<id>/<uuid>.snap`, `0600` in `0700` folders), even with `--restore=false`.
- Beyond terminals, the API stops the server, turns Tailscale on and signs it in, edits the host list, grants macOS file access, loads and runs Lua (`client.set_script`, `client.action`), asks Rex.app to perform its actions (`client.perform`), lists folders and moves files (`list_dir`, `upload_begin`, `download_begin`).

## Mapping

Every runtime call collied makes today (`crates/collied/src/runtime/mod.rs`), with herdr's and herdr-core's call, mapped to Rex. "Built" means collied does it itself; "lost" means a Rex agent goes without. `terminal.x` stands for the block method `com.superlogical.terminal.x`, called on the control connection with `session_id`, `block_id` and `args`.

| collied call | herdr | herdr-core | Rex | |
|---|---|---|---|---|
| `ping` | `ping` | `hello` | `hello` (API revision), `server.status` (version, pid, listeners, Tailscale) | equivalent |
| `session_snapshot`, `agent_list`, `poll` | `session.snapshot`, `agent.list`, `workspace.list`, polled every 1 s | `events.subscribe` | events first, then `session.list`, `session.list_blocks`, and `terminal.program_status`, `terminal.process` and `terminal.title` per terminal ([Transport](#3-transport)) | equivalent, push instead of poll; groups built |
| `agents_of`, `snapshot_of` (`find_agent`, `find_pane`) | herdr's list for herdr ids | `hc:` ids: the merged list | `rex:` ids: the merged list, as `hc:` ids | equivalent, routed by prefix |
| `herdr_snapshot` (archive's occupancy check) | `session.snapshot` | herdr only, beside the core's `occupants()` | herdr only, beside Rex's blocks and their `process` folders | herdr only, unchanged |
| `agent_get` (ready check) | `agent.get` | `terminal.get` | `terminal.program_status` and `terminal.process` | equivalent |
| `pane_get` (an open worktree's root pane, `still_new`) | `pane.get` | herdr only (`herdr_only`) | herdr only: both callers follow herdr's `worktree.open` and `agent.start`; Rex's occupancy comes from its own listing | herdr only, unchanged |
| `pane_columns` | `pane.layout` | `terminal.get` `cols` | `terminal.size` | equivalent, exact |
| `detection_text` (approvals, menus, startup checks) | `pane.read` `detection` | `terminal.read` | `terminal.format {format: text}`, its last `rows` lines | equivalent (verified) |
| `agent_read`, `pane_read` (ANSI, `visible`, `recent`) | `agent.read`, `pane.read` | emulator on a watch-only attach | `terminal.format {format: vt, max_bytes}`, then `terminal.blob_read` of the tail, into collied's emulator | built (verified for the screen) |
| `watch` | nothing to hold, reads polled | watch-only attach | nothing to hold, reads polled as for herdr | equivalent ([Screen](#4-screen)) |
| `agent_explain` | `agent.explain` | `status.kind` | the status record's `kind` | replaced (verified) |
| `agent_prompt`, `type_text` | `agent.prompt`, `pane.send_input` | `terminal.send` (`if`, `force`) | `block.event` paste, then `block.event` key Enter | equivalent, without `if` (verified) |
| `agent_send_keys`, `scroll_bottom`, `notice_digit` | `agent.send_keys`, `pane.send_text` | `terminal.keys` | `block.event` key | equivalent, except `cmd`, `ctrl+[`, `ctrl+alt+[` and `ctrl+shift` combinations, refused (verified, [Input](#5-input)) |
| `pane_send_text` (slash mirror, notes) | `pane.send_text` | typing attach | `block.event` paste, no Enter | equivalent (verified) |
| `pane_send_keys`, `pane_send_input` | as named | herdr only | `block.event` | herdr only, as for herdr-core |
| `agent_focus` | `agent.focus` | lost | `session.focus_block` | unverified ([Focus](#9-focus)) |
| `agent_start`, `workspace_create` | `agent.start`, `workspace.create` | `terminal.run` | `session.create` with one window running the agent | equivalent; kind and task record built |
| `tab_create` (keeps an archived worktree's workspace open) | `tab.create` | not needed | not needed: a session outlives its last block (verified) | |
| `worktree_list`, `worktree_create`, `worktree_open`, `worktree_remove` | `worktree.*` | `git worktree` | `git worktree` | built, as for herdr-core |
| `workspace_close` | `workspace.close` | refused | refused: `session.destroy` would end blocks the phone never shows | lost ([Grouping](#7-grouping)) |
| `pane_processes` (`claude attach` target, transcript pids) | `pane.process_info` | herdr only | `terminal.process` (`foreground`, `ancestors`, `child`) | equivalent |
| `pane_close` | `pane.close` | `terminal.kill`, `terminal.rm` | `block.close` | equivalent |
| `agent_manifests` | `server.agent_manifests` | built | built (the CLIs on the machine) | built |
| `agent_session` (Chat, costs, context, resume, hook binding) | herdr's hooks | pid file; Codex hook | Claude Code's `sessions/<pid>.json` for the foreground pid (partly verified: the file for the block's foreground pid named the session its hooks reported; collied's lookup, which also checks the pid's start time, was not run); collied's Codex SessionStart hook reading `REX_BLOCK` (not run) | built |
| `task_core` (where New task starts) | herdr | herdr-core when herdr is not running | Rex when neither answers | built ([Tasks](#8-tasks)) |

## Questions

### 1. Agent status

- Rex keeps it. `terminal.program_status` lists the records programs reported with OSC 7501: `state` (`idle`, `working`, `done`, `blocked`, `error`), `kind` on a blocked record (`permission`, `question`, `auth`), `app`, `msg`, `updated_at`, and `owner`, the foreground process (pid and name) when the report arrived. `program_status_changed` carries each report, even one that changes only `updated_at`; `program_status_removed` drops records, with a reason (`clear`, `child_exited`, `reset`, `prompt`, `foreground`, `evicted`). Claude Code reports the root record (no `id`).
- Claude Code 2.1.322 in a Rex block: `idle` with `app: claude-code` once started, then `working`, `blocked` (`permission`, with its `msg`) and `done` (verified). At its folder trust prompt it reported nothing (no records), so it stays read-only from the phone until it reports, as under herdr-core.
- Rex dropped a record when its program was suspended (Ctrl+Z: `program_status_removed`, reason `foreground`), unlike herdr-core 0.1.16, but kept the record of a program that exited with its shell leading the terminal again (both checked with a stand-in reporting `app=codex`, under a zsh without shell integration, so no `prompt` reason). collied keeps herdr-core's rule (no terminal whose foreground process is a shell is an agent) and adds one Rex allows: a record whose `owner` is not the current foreground process (pid and name) is stale.
- No hook is needed for status. Claude Code's hooks ran in the Rex block with `REX_BLOCK` set (SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest and Stop, from a scratch `--settings` file), so a hook could report it, but the server already has the status. Codex needs collied's SessionStart hook for its session, as under herdr-core, reading `REX_BLOCK` as it reads `HC_TERMINAL_ID` (not run).
- Copilot CLI reports no OSC 7501 (herdr-core spike, 1.0.105), so a Copilot agent in Rex is listed `unknown` and is read-only, as in Herdr 2.

### 2. Identity and kind

- `terminal.process` describes the `foreground` process, its `ancestors` up to the terminal's `child`: `pid`, `name` (the executable's file name; Claude Code's is its version, `2.1.322`), `path`, `invoked_path` (`~/.local/bin/claude`), `argv0` (the only argument Rex returns, cut to 256 bytes) and `cwd`.
- A block is an agent once it reports OSC 7501 or its foreground process runs an agent CLI, as for herdr-core. Kind: the status record's `app` (`claude-code` is `claude`), else the basename of `argv0` among `claude`, `codex` and `copilot` (verified: `claude`). A shell is the basename of `argv0`, a leading `-` aside, in herdr-core's list (`zsh` verified). Plain shells and other programs are not listed.
- `process_changed` pushes each foreground change, so the kind needs no polling. It is also sent each time a client starts receiving a terminal's output.
- A block whose `host_id` differs from the server's runs on another machine and is skipped, as herdr-core's `box/t7` (not verified: every block here was local).

### 3. Transport

- The CLI works, one process per call: `rex events -s SESSION --json` held open streams that session's block events, `rex block call -b BLOCK METHOD ARGS` makes a call. A call took 41 to 49 ms under `sandbox-exec`, whose own start costs 15 ms, so 25 to 35 ms. Without `-s`, `rex events --json` carries no block events; with `--all`, every client's coming and going. With no server, calls exit 4 as documented, except `rex server status`, which exits 1; a missing block exits 3 and a usage error 2.
- The control connection: 0.19 ms for `program_status`, 0.25 ms for `process` or `format` of a small screen, 0.34 ms to connect, say hello and make one call (measured over a Unix socket from Python). Status, process, title and size changes are pushed, so nothing is polled for them; screen output has no event (only the data connection carries it).
- collied therefore follows Rex on one long-lived control connection: `tokio-tungstenite` (a dependency already, 0.30.0) over a `UnixStream`, without compression, answering the server's pings (every 120 s). It subscribes before it reads, so nothing that happens in between is missed: `session.attach_all` with `event_scope: session` first (sessions created and destroyed, not `event_scope: all`), then `session.list`, then for each session `session.attach` (its block events from then on) followed by `session.list_blocks` and the reads of each block. An event whose `seq` is at most a read's response `seq` is already in that read; a later one is applied on top, and a session created meanwhile is attached and read the same way. After a reconnect `seq` starts at 1, so collied does all of it again.
- Method check: the `hello` sends `api_revision: 13` and `api_min_revision: 13`, so an older server closes the connection (1008) instead of answering in older shapes. Then the server methods collied calls must be in `help.all`'s `methods`, and the terminal methods in `block_creator.list`'s `com.superlogical.terminal` creator, each against the set the built phases need ([Effort and phases](#effort-and-phases)); otherwise collied does not follow that server and `doctor` names what is missing, as for herdr-core.
- collied never starts a server and never runs the `rex` CLI: an absent socket means Rex is not running, while any `rex` command would start one (`--autostart` defaults to true).

### 4. Screen

- `format` returns the screen and the whole scrollback as text, HTML or VT (SGR, truecolor, cursor moves for runs of blanks), with `trim` and `unwrap` (soft-wrapped lines joined, herdr's `recent_unwrapped`). The visible screen is the last `rows` lines of the text (verified: 27 lines for 3 of history and 24 rows). There is no visible-only or tail-only option.
- Text: approvals, menus and startup checks read it unchanged ([Approvals](#6-approvals)). ANSI: collied's `Screen` (`alacritty_terminal` 0.26.0) fed with `format`'s VT output, its final line break dropped, drew the same 24 rows as the text (verified on Claude Code's permission dialog). So `agent.read`, `pane.read`, drafts, the jump banner and notices can come from an emulator fed by one `format` read.
- Size limits: with 100,000 lines of scrollback (4.3 MB of text), `format` failed with `too_large` (the message limit is 1 MiB). With `max_bytes` set, `format` answers the first part and keeps the whole result for `blob_read`, which reads it from any byte offset. A Rex terminal keeps up to 64 MiB of scrollback.
- `rex block stream` answered "not implemented" in this build. A data connection (`data.connect`, then `/api/data` with `rex.data.terminal.v1`) delivered its first bytes 0.5 to 2.9 ms after the ticket: a binary Ghostty snapshot (`GHOSTSNP`), then the program's output with Rex's own messages as `ESC _ rex1;<nonce>;<json> ESC \` (`resize` right after the snapshot, `theme`, `child_exited`). `alacritty_terminal` cannot read the snapshot, and nothing lines the stream up with `format`: `format` answers `content`, `id`, `total` and `eof`, and the stream carries only those messages and, for a resume, a byte count of its own. Seeding an emulator from `format` and then following the stream could apply output twice or drop it.
- So reads and watches poll `format` VT, as collied polls herdr's reads for a watch (every 250 ms, every 1 s once the screen has not changed for 5 s): one `format` with `max_bytes`, then `blob_read` of the tail the phone's lines need, cut at a line break, emulated afresh. Each poll is the terminal's whole current state, so output is never applied twice and never lost from what the screen and its history hold; output a program overwrites between two polls is never seen, as with herdr.
- Cost, measured against a terminal filled with numbered lines (one `format` with `max_bytes: 65536` plus one `blob_read` of the last 64 KB): 0.9 to 1.2 ms at 1,000 lines (0.04 MB of VT), 10 to 14 ms at 10,000 (0.45 MB) and 39 to 46 ms at 100,000 (4.4 MB). What crosses the socket stays bounded; the server's work grows with the scrollback. At 250 ms that is about a sixth of a core while a phone watches a 100,000-line terminal that keeps changing, a twentieth once it is quiet. The data connection's snapshot, by comparison, sent 0.05 MB in 1 ms at 1,000 lines and 4.5 MB in 65 ms at 100,000 before any live output. If polling proves too dear, two later options remain: a data connection used only as a sign that output arrived (its bytes discarded, the snapshot paid once per watch), or libghostty-vt, which the app already builds from pinned tarballs, to restore the snapshot itself (a new native dependency for collied; whether its C API reads Rex's snapshots is unknown).
- Size: neither `format` nor a data connection changed a terminal's size or owner (verified on a terminal resized to 100x30 with no owner). collied never calls `resize`, `own_block` or `set_theme`, and opens no data connection. A block Rex.app owns has Rex.app's size; not verified with Rex.app attached.

### 5. Input

- `rex send TEXT` (and `terminal.write`) types the text as raw bytes, with no bracketed paste and no Enter (verified with a raw-mode reader: `w1\nw2` arrived as is). `block.event` with a `paste` event pastes: inside `ESC[200~` and `ESC[201~` when the program turned bracketed paste on, newlines as `\r` otherwise, and an `ESC` in the text arrives as a space, so a pasted `ESC[201~` cannot end the paste (verified). A prompt is a paste and then a `key` event for Enter, which run in order on one connection (verified: Claude Code took the prompt and answered).
- Keys: on the socket a `key` event takes W3C key codes (`key_a`, `arrow_up`, `escape`, `bracket_left`), a modifier string (`ctrl+alt`, with `cmd` as `super`), the text the key types and its unshifted code point, so collied maps its names. For 39 names, the named `Key`s and combinations of each kind, a raw-mode reader received the same bytes from those `key` events as from `rex send-key` with collie's name, and the bytes herdr sends (verified): `enter`, `esc`, the arrows, `shift+left`, `tab`, `shift+tab`, `backspace`, `ctrl+c`, `ctrl+enter` as `ESC[27;5;13~`, `ctrl+a`, `ctrl+b`, `ctrl+e`, `ctrl+k`, `ctrl+l`, `ctrl+u`, `ctrl+w`, `ctrl+end` as `ESC[1;5F`, `end`, `space`, `a`, `1`, `/`, `` ` ``, `[`, `alt+x`, `ctrl+alt+x`, `shift+f5`, `f1`, `f12`, `alt+enter`, `shift+enter`, `ctrl+space`. Lost on a Rex agent: `cmd+k` (any `cmd` combination), `ctrl+[` and `ctrl+shift+x` send nothing either way to a program without a keyboard protocol, and `ctrl+alt+[` sends `ESC [` (Alt's `ESC`, then `[` without its Control); `ESC [` followed by the next keys could spell an escape sequence, which is why collie allows `[` only with Control. collied refuses these four kinds for a Rex agent, as herdr-core refuses `cmd`, rather than send nothing or `ESC [`. A program that turned on a keyboard protocol was not tried. `rex send-key` refuses an unknown name (exit 2). Phase 3 keeps a live test of every key over the socket.
- No write can be held to a status: `block.event` and `write` take no condition, and Rex holds nothing while someone types on the Mac. collied's ready check, `program_status` right before the write, is the only guard: the gap herdr leaves, wider than herdr-core's.
- Text without Enter (the slash mirror, an approval note) is a paste alone.

### 6. Approvals

- The status record's `kind` stands in for herdr's rule id, as for herdr-core, so the menu gate holds (`prompt::uses_menu("claude", Some("permission"))`). On the Rex text of Claude Code 2.1.322's Bash permission prompt, `Menu::parse` found the four options with the cursor on "Yes" and the decisions Approve, ApproveAlways and Deny, and `fingerprint` took its region (verified). Esc denied it (`idle`); Enter on "Yes" approved the next one (the probe file appeared in the scratch folder, then `working`, `done`).
- One dialog, one `blocked` report: Claude Code 2.1.322 sent none while one dialog stayed on screen for over five minutes (it sent a `desktop_notification`, "Claude needs your permission"), nor while another was resized from 80x24 to 100x30 and back, and each dialog brought its own report. Between two identical prompts in a row (`touch twice.txt`, twice, approving the first) it reported `working`: `blocked` (seq 75), `working` (100), `blocked` (113), with the same `msg`.
- `state_change_seq`: Rex's `seq` counts per connection and restarts after a reconnect, so collied keeps its own counter, and moves it on every `program_status_changed` for the root record, not only when the `state` changes. Two identical dialogs in a row then never share a seq, which with the same fingerprint would let `decide` (which requires both to match, `approvals.rs`) answer the second with a decision meant for the first. A report repeated during one dialog would only supersede the pending approval, and the phone would ask again; none was seen. After a reconnect, a record whose `updated_at` is unchanged keeps its value and any other takes a new one. Phase 3 tests two identical prompts in a row.
- `msg` is the agent's own text: logged at most, never trusted over the screen.
- Found on the way, not Rex's: at 80 columns Claude Code wraps its "Tip: ... choose \"switch to auto mode\" below" line, and `prompt::context` took the wrapped `below` as the description (`Bash: below`). A Rex block no client sized is 80 columns; a narrow herdr pane would do the same.

### 7. Grouping

- A Rex session is a group of related work, as a herdr workspace is: one group per session, `rex:session:<uuid>`, with the session's label (`session_label_changed` follows renames), numbered after herdr's and herdr-core's. Windows (tabs) are not shown. One group per server would hide the user's own grouping.
- `pane.close` is `block.close`, which ends the program; a window whose last block closes closes too. A session outlives its last block and window (verified).
- `workspace.close` on a Rex group is refused ("close its agents one at a time"), as for a herdr-core group. It would be `session.destroy`, which ends every block in the session, Rex.app's included, while the phone shows only agents, and Rex offers no conditional destroy (`server.stop` has `if_idle`; `session.destroy` takes only the session), so a check that the session holds only agents could be overtaken by a block Rex.app opens before the destroy.

### 8. Tasks

- New task starts in herdr while herdr answers, else in a followed herdr-core, else in a followed Rex: no setting, so a machine that runs both herdr and Rex changes nothing.
- A task is a session of its own, as herdr's `workspace.create` makes a workspace: `session.create` with the task's name as label and one window whose block runs `/usr/bin/env -u <name> ... <CLI path> <args>` (`options.command`, `options.cwd`), the API form of `rex new NAME -- COMMAND` (verified with `shell: none`: Claude Code started there and reported). With Rex's default shell (a login shell, not interactive) the agent gets the user's profile `PATH`, as Rex.app's terminals do; with `shell: none` it gets the server's environment. The default was not run (the sandbox), so phase 5 checks it. `rex run` and `session.new_window` would put a task in an existing session instead.
- The `-u` list is herdr-core's sixteen names: a Rex server passes its environment to every terminal (verified), and a server started by a `rex` command inside a herdr pane would carry that pane's variables (not verified: `rex server start --env` suggests only named variables pass).
- `session.create` makes collied the owner of the new block, which would keep collied's size (80x24) and keep Rex.app from sizing it. collied disowns it at once (`session.disown_block`), so Rex.app sizes it as it does the user's own blocks (not checked with Rex.app).
- A block has one label, which Rex.app shows, and nothing like herdr-core's `collie.kind` and `collie.worktree`. collied records the blocks it starts (kind, worktree checkout) by block id in its data folder; ids are never reused.
- Worktrees and archive as for herdr-core: `git worktree`, then `block.close` of the task's own block by id, then `git worktree remove`. Archive never destroys a session, for the reason `workspace.close` is refused: an archived task's session stays, empty, until the user closes it in Rex.app. The check that nothing else works in the checkout, read again after closing the task's block, has the same short window herdr-core's has.

### 9. Focus

- `rex focus BLOCK` (`session.focus_block`) makes the block's window the session's active window and focuses the block, on the server (verified with `session.view`), with no Rex.app setting. Whether Rex.app then shows that session and comes forward is not verified.
- Rex.app's own actions (`client.tab.goto`, `client.window.*`, through `client.perform` or `rex -C`) are refused while Remote Control is off in its settings: its binary holds "Rex.app is not accepting actions. Turn on Remote Control in its Settings under Rex Server." collied never calls them: they drive the app, and that setting is the user's.
- Plan: `agent.focus` on a Rex agent is refused, as for herdr-core, until a check with Rex.app shows `session.focus_block` brings the agent forward.

### 10. Alongside the others

- Ids: a block is `rex:block:<uuid>` (46 bytes) and a group `rex:session:<uuid>` (48 bytes), within `TerminalId`'s `[A-Za-z0-9_:.-]{1,64}`. Rex ids are never reused, so a restarted or wiped server never aliases old ids. collied routes by the `rex:` prefix, as it routes `hc:`; the phone parses no id.
- Discovery as the CLI does it, from collied's own environment ([What Rex is](#what-rex-is)), never `REX_SERVER`, which can name another machine, and never the hosts list. Rex missing or stopping behaves as herdr-core does: a server that never answered is quiet, one that stops after answering logs once and its agents leave with one `flock.changed`, and `herdr_unavailable` is answered only when no runtime answers.
- `collied status`: a `rex:` line with the version and API revision (or `not running`), the socket and its folder's mode, any method the server lacks, and remote access when it is on.
- `collied doctor`: informational when Rex is absent; else the version and the socket, and a warning for a server it does not follow (API revision, missing methods, the personal-server checks, a listener admitting anonymous callers), for Tailscale mode or a tailnet listener, and for a server whose environment (read by name only, from `server.status`'s `pid`, as for herdr-core's holder) holds `HERDR_PANE_ID`, `HERDR_SOCKET_PATH` or `CLAUDE_CODE_CHILD_SESSION`.
- Platforms: macOS, verified. Linux, where collied and CollieTray also run, is not verified; `rex server install ssh://...` installs a server on a Linux machine, and a collied there would follow that machine's own server. Remote hosts are never followed.
- Threat model, the supported boundary: a personal server. collied follows a server only when its socket is owned by the user and has no group or other permission bits, its folder is owned by the user with mode `0700`, and the peer on the socket (the server) runs as the user; otherwise it refuses, logs once and `doctor` says why. That keeps out Rex's shared server on Linux, whose group members all see and type into every session as the `rex` account, and a personal socket opened to a group with `--socket-mode`. Only the scratch personal server was checked here; Linux was not.
- Remote access, off by default and off in the issue's assumption: built-in Tailscale (the tailnet owner, or every member the ACLs allow), tailnet and Tailscale Serve listeners (the owner, or every identified caller), plain HTTP with `--allow-anonymous` (anyone who reaches the port, with full access) and SSH forwards. collied reads `server.status` (`listeners` with their `transport` and `admission`, and `tailscale.state`) when it connects: a listener admitting `anonymous` callers makes it refuse to follow, since anyone on that network types into the same agents; Tailscale `running` or a tailnet listener lets it follow, with one log line and a warning in `collied status` and `doctor`, since those reach the same terminals outside collie's allowlist and audit.
- What collied calls: the API is wider than herdr-core's (Tailscale, Lua, Rex.app's actions, files), so the client refuses anything but `hello`, `help.all`, `block_creator.list`, `server.status`, `session.list`, `session.attach`, `session.attach_all`, `session.list_blocks`, `block.close`, `block.event` (`paste` and `key` only), the terminal methods `program_status`, `process`, `title`, `size`, `format` and `blob_read`, and for tasks `session.create` and `session.disown_block`; never `session.destroy`, `data.connect`, `server.stop`, `server.tailscale.*`, `hosts.*`, `server.apple.*`, `client.*`, `own_block`, `resize`, `set_theme`, `write`, `list_dir`, `upload_begin` or `download_begin`. The paste neutralizes `ESC`, so a prompt cannot break out of bracketed paste. Screens are kept on disk under the state folder, same-user only. The threat model gains a Rex section in phase 1.

## Shape

- `crates/collied/src/runtime/rex.rs` beside `herdr.rs` and `herdr_core.rs`, behind the same `Runtime` calls, building the shared model (`AgentInfo`, `PaneInfo`, `WorkspaceInfo`) as herdr-core does: `terminal_id` and `pane_id` both `rex:block:<uuid>`, `workspace_id` `rex:session:<uuid>`, `state_change_seq` from collied's own counter, `agent_session` from Claude Code's pid file and the Codex hook, `read_only` while no status was reported.
- Phone protocol unchanged: `claude-code` is `claude`, `error` is `unknown`.
- Client: one control connection per followed server on `tokio-tungstenite`, reconnecting with herdr-core's backoff (1 s to 30 s); no data connection.
- Emulator: collied's `runtime::screen::Screen`, unchanged, fed a fresh `format` VT tail for each read.

## Testing against Rex

The AGENTS.md rule this plan adds: a test or check that needs a live Rex starts a server of its own (`rex server run --listen unix://<scratch>/rex.sock --state-dir <scratch> --config <empty file> --restore=false`) from a clean environment with scratch `HOME`, `XDG_RUNTIME_DIR` and `REX_RUNTIME_DIR`, passes `--autostart=false` and `-S` with that socket on every call, stops only that server by its pid and deletes its folder. `--state-dir` alone is not enough: the server also writes under its runtime folder, and with neither `HOME` nor a runtime folder set aside the default socket is the user's. The socket path must stay under 104 bytes, so the scratch folder is a short one under `/tmp`.

## Effort and phases

Each phase adds the methods it needs to the set the method check requires ([Transport](#3-transport)); `hello` and `help.all` always.

1. Agents and status (M): discovery, the personal-server and remote-access checks, the control connection, sessions as groups, status and kind from events, `collied status` and `doctor`, quiet absence, `pane.close`, the refusal of `workspace.close`, the threat model section. Server: `server.status`, `session.list`, `session.attach`, `session.attach_all`, `session.list_blocks`, `block.close`; terminal: `program_status`, `process`, `title`.
2. Screens (M): reads and watches polled from `format` VT tails into the emulator. Terminal: `format`, `blob_read`, `size`.
3. Input and approvals (M): paste and key events behind the allowlist, the ready check, read-only agents without status, approvals gated on `kind`, collied's own `state_change_seq` moved by every report; tests for two identical prompts in a row and for every key over the socket. Server: `block.event`.
4. Sessions (S): Claude Code's pid file through collied's lookup, the Codex SessionStart hook with `REX_BLOCK`, then Chat, costs, context, resume and the permission hook's binding. No new method.
5. Tasks (M): New task, resume, worktrees and archive in Rex when neither herdr nor herdr-core answers, with the sixteen variables stripped, the block disowned and task records kept by block id. Server: `session.create`, `session.disown_block`.
6. Focus (S, only if Rex.app follows): `agent.focus` through `session.focus_block`, added to the set only then.

What a Rex agent cannot do that a herdr one can, after all phases: take a prompt held to the status collied saw (no `if`), wait for the Mac's typing to pause, take `cmd`, `ctrl+[`, `ctrl+alt+[` or `ctrl+shift` keys, close with its whole group, probably be focused on the desktop, and, for a CLI without OSC 7501 (Copilot CLI), report a status or take input from the phone.

## Risks

- **Preview API.** Rex is 0.1.0-preview.16, its API at revision 13, and the user's running server is an older build than the installed CLI. collied pins the revision and fails closed per missing method.
- **Polling cost.** `format` always renders the whole scrollback (up to 64 MiB), so a watched terminal with 100,000 lines costs the server about 40 ms per poll, four polls a second while it changes ([Screen](#4-screen)).
- **Empty sessions.** Archived tasks leave empty Rex sessions behind until the user closes them; a conditional `session.destroy` in Rex would remove them.
- **The environment.** A Rex server passes its own environment to every terminal; one started from inside a herdr pane or a Claude Code session would hand its variables to every agent, as herdr-core's core did ([herdr-core risks](herdr-core.md#risks)). collied strips them from the agents it starts and `doctor` warns.
- **No input guard.** Without `if`, a dialog that opens between the ready check and the write takes the prompt's Enter, as with herdr.
- **Remote access.** A Rex reachable over the tailnet is a second way into the same agents, outside collie's audit; collied follows it with a warning, and refuses a server that admits anonymous callers.

## Prototype

A throwaway outside the repository (scratchpad, not committed): `spike.py`, a Python client of the control and data WebSockets (standard library only), `run.sh`, which drives it end to end, and `menu-check`, a Rust binary depending on collied by path for the menu parser, fingerprint and emulator. `run.sh` refuses to start while its socket exists, starts its own server with a scratch `HOME`, runtime and state folders and an empty `REX_CONFIG`, under `sandbox-exec` with a profile denying the user's two Rex folders and socket; runs Claude Code (`--model haiku`, with `--setting-sources project,local` so the user's hooks do not fire, and a scratch `--settings` file whose hooks log `REX_BLOCK`) in a scratch session, answering its folder trust prompt there; lists blocks with their status, reads the agent's screen, sends it a prompt, prints `collied status`'s herdr lines before and after; then kills the session, stops its server by pid, deletes its folders and compares the user's Rex folders with what they were when it started.

```text
spike.py --socket SOCK list | watch SECONDS | read BLOCK [vt] | send BLOCK TEXT | keys BLOCK KEY... | stream BLOCK SECONDS FILE | bench
keytest.py, costbench.py, resizetest.py       the review checks: keys over the socket, poll and snapshot costs, a resize during a dialog
run.sh                                        the end-to-end run below
menu-check TEXT VT COLS ROWS [ID SESSION]     collied's parser, gate, fingerprint and emulator on a Rex screen
```

Rerun from the scratchpad's `rex-prototype` folder: `./run.sh` (macOS, Rex Beta.app installed, Claude Code at `~/.local/bin/claude`; it uses `--model haiku` and adds a `~/.claude.json` project entry for its scratch folder, which is removed by hand). Run (abridged):

```text
== herdr before:  herdr 0.9.5, herdr-core 0.1.19, 9 agents: 8 idle, 1 working
== scratch server pid 72706: 0.1.0-preview.16 api 13 unix:///tmp/.../rs.sock tailscale disabled
== list
rex:01a1d0bb-...-c0201d843899  [spike703/shell]  fg=['claude', 'claude', '2.1.322']  claude idle  title='Claude Code'
== read (last rows)
❯ Try "fix lint errors"
  ⏵⏵ auto mode on (shift+tab to cycle) · ← for agents       ◐ medium · /effort
== send
sent after ready check (idle) in 0.3 ms
== done
❯ Reply with just the word pong. Do not use any tools.
⏺ pong
== events
[1]  process gen=2 fg='/Users/.../.local/bin/claude' name='2.1.322' pid=72716
[11] status idle app=claude-code owner={'pid': 72716, 'name': '2.1.322'} state_change_seq=11
[33] status working app=claude-code state_change_seq=33
[43] status done app=claude-code state_change_seq=43
== hooks: SessionStart, UserPromptSubmit, Stop, each with REX_BLOCK=block:01a1d0bb-...
== herdr after:   herdr 0.9.5, herdr-core 0.1.19, the same 9 agents: 7 idle, 2 working
== scratch server stopped; dirs removed; the user's Rex folders identical to the baseline; pid 8884 running
```

Exploratory runs (a scratch server of their own):

```text
blocked:  status blocked kind=permission msg='approve Bash: touch rex-spike-probe.txt' (seq 84);
          Menu::parse: ["Yes", "Yes, and always allow access to ... from this project",
          "Yes, and switch to auto mode · ...", "No"], cursor 0, decisions Approve, ApproveAlways, Deny;
          context "Bash: below\nCreate empty file rex-spike-probe.txt\ntouch rex-spike-probe.txt"
          emulator fed with format vt: 24 rows, identical to format text
          esc -> idle (seq 434); next dialog blocked (seq 452); enter -> working, done; probe file created
trust:    Claude Code at its folder trust prompt: no status records ("claude unknown")
latency:  CLI 41 to 49 ms per call (sandbox-exec alone 15 ms); socket 0.19 to 0.25 ms per call, 0.34 ms connect+call
scroll:   100,000 lines: format too_large; max_bytes 64 KB 110 ms (text), 58 ms (vt); blob_read tail 20 to 35 ms
stream:   first bytes 0.5 to 2.9 ms; GHOSTSNP snapshot 5 KB (Claude Code) to 4.5 MB (100,000 lines); size kept
keys:     paste bracketed when asked, ESC neutralized; write raw
stale:    Ctrl+Z removes the record (reason foreground); a normal exit back to zsh keeps it

review checks (another scratch server):
keys:     39 names, block.event key over the socket vs rex send-key: 39 of 39 identical bytes
twice:    touch twice.txt twice: blocked (75), enter, working (100), blocked (113), same msg; esc -> idle
resize:   own_block, resize 100x30 and back to 80x24 during a dialog: no status report; disowned after
poll:       1,000 lines: 0.04 MB vt, poll 0.9 to 1.2 ms, snapshot 0.05 MB in 1 ms
           10,000 lines: 0.45 MB vt, poll 10.1 to 14.0 ms, snapshot 0.46 MB in 2 ms
          100,000 lines: 4.43 MB vt, poll 38.7 to 45.7 ms, snapshot 4.53 MB in 65 ms
```

## Follow-up issues

### 1. List Rex agents and their status beside herdr's (#704)

Connect to a running personal Rex server (never start one) over its control WebSocket, follow each session's block events, and show its agents in one group per Rex session, with OSC 7501 status, status times, alerts and pushes. herdr, herdr-core and Rex work in any combination with no setting.

- [ ] Agents get ids `rex:block:<uuid>` and groups `rex:session:<uuid>`; blocks of other hosts are skipped.
- [ ] A missing Rex logs nothing and changes nothing for a machine without it.
- [ ] collied follows only a personal server: a socket owned by the user with no group or other bits, its folder `0700` and the user's, the server running as the user. A listener admitting anonymous callers makes it refuse; Tailscale mode or a tailnet listener makes it warn.
- [ ] It subscribes before it reads (`session.attach_all`, then `session.list`, then `session.attach` and the reads per session), so a session or report in between is not missed.
- [ ] The client sends `api_revision` 13 as its minimum, checks this phase's server methods in `help.all` and terminal methods in `block_creator.list`, and calls nothing else.
- [ ] A status record whose owner is not the foreground process, or a shell in the foreground, lists no agent.
- [ ] `collied status` and `collied doctor` report Rex (version, API revision, socket and folder checks, missing methods, listeners, Tailscale mode, the server's inherited variables).
- [ ] The phone's `pane.close` closes a Rex agent's block, and `workspace.close` on a Rex group is refused, ending no block.
- [ ] The threat model and architecture pages describe Rex.
- [ ] Live tests run a Rex server of their own, as AGENTS.md says.

### 2. Read Rex agent screens through collied's emulator (#705)

Reads and watches poll `format` VT (`max_bytes`, then `blob_read` of the tail the phone's lines need) into `runtime::screen::Screen` at herdr's watch periods, for `agent.read`, `pane.read`, `agent.watch`, drafts, the jump banner and notices. No data connection.

- [ ] Output and patches match herdr's format, so the phone needs no change.
- [ ] Emulated text equals `format` text for Claude Code fixtures.
- [ ] A read or watch poll of a terminal with 100,000 lines of scrollback succeeds and moves at most the tail.
- [ ] No read or watch changes a terminal's size or owner.
- [ ] Live tests run a Rex server of their own.

### 3. Send prompts, keys and approval decisions to Rex agents (#706)

Paste and key events behind the existing allowlist and ready check; approvals gated on the status `kind`, with collied's own `state_change_seq`, moved by every status report.

- [ ] A Claude Code permission prompt in Rex raises an approval that Approve and Deny answer.
- [ ] Two identical prompts in a row raise two approvals, and a decision meant for the first never answers the second.
- [ ] An approval's fingerprint and seq stay while one dialog stays on screen, and after a reconnect when its record's `updated_at` did not change.
- [ ] An agent with no status refuses prompts, typed answers and keys (`agent_not_ready`).
- [ ] Every `Key` is checked live over the socket by the bytes a program receives; `cmd` combinations, `ctrl+[`, `ctrl+alt+[` and `ctrl+shift` combinations are refused for a Rex agent.
- [ ] Live tests run a Rex server of their own.

### 4. Bind Claude Code and Codex sessions to Rex blocks (#707)

Claude Code's session from `sessions/<pid>.json` for the block's foreground pid, through collied's own lookup; Codex's from collied's SessionStart hook reporting `REX_BLOCK`, as for herdr-core's `HC_TERMINAL_ID`. Check Codex's OSC 7501 reports in a Rex block.

- [ ] collied binds a Rex Claude Code agent's session from its pid file (the spike saw the file only by hand).
- [ ] Chat shows a Rex Claude Code agent's conversation, and `task.sessions` leaves out sessions a Rex agent runs.
- [ ] A Codex agent in Rex reports its status and binds its session.
- [ ] Live tests run a Rex server of their own.

### 5. Start, resume and archive tasks in Rex when neither herdr nor herdr-core answers (#708)

`session.create` with the agent's command, the new block disowned, task records by block id, `git worktree` for worktree tasks, and `block.close` of the task's own block on archive. No session is ever destroyed.

- [ ] With herdr and herdr-core stopped, New task starts Claude Code and Codex in a Rex session of its own, in a worktree when asked.
- [ ] Agents start through `/usr/bin/env -u` with herdr-core's sixteen names and get the user's profile `PATH` from Rex's login shell (checked live).
- [ ] The task's block has no owner once started.
- [ ] Archive closes only the task's block and removes its recorded worktree; it destroys no session.
- [ ] With herdr or herdr-core running, tasks still start there.
- [ ] Live tests run a Rex server of their own.

### 6. Check focusing a Rex agent with Rex.app (#709)

With the owner's Rex.app, check whether `session.focus_block` brings the agent's session forward; if it does, `agent.focus` uses it, else focus stays refused. No Remote Control action is used.

- [ ] docs/rex.md and architecture.md say what Rex.app does.

### 7. Keep Claude Code's wrapped tip line out of an approval's context (#710)

At 80 columns Claude Code wraps the tip line of its permission prompt, which ends `choose "switch to auto mode" below`, and `prompt::context` takes the wrapped `below` as the call's description: `Bash: below` instead of `Bash: Create empty file ...`. The spike saw it on a Rex block at its default 80x24; a herdr pane that narrow shows the same screen.

- [ ] The context of that 80-column permission prompt names the call's own description.
- [ ] A scrubbed fixture of the screen covers it.
