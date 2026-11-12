# Rex backend plan

Spike for [#703](https://github.com/rbstp/collie/issues/703), 2026-11-12. This page is the plan, the evidence behind it and the follow-up issues for "Phase 8: Rex integration". Nothing here is built yet. Most of the design reuses herdr-core's: its plan is [herdr-core.md](herdr-core.md), and what collied does with it is in [architecture.md](architecture.md#herdr-core-herdr-2).

## Summary

- Rex 0.1.0-preview.16 (API revision 13) already gives collied what it needs to follow Claude Code agents beside herdr's, with no hook and no polling for status. Each terminal block keeps the OSC 7501 status its programs report (`program_status`, pushed as `program_status_changed`) and its foreground process (`process`, pushed as `process_changed`), reads its screen as text or escape sequences (`format`), and takes pastes and named keys (`block.event`). The issue's list of what Rex leaves undocumented predates this build: the server documents its whole API, both WebSocket protocols included, at `/llms.txt` on its own socket.
- Claude Code 2.1.322 reports `idle`, `working`, `blocked` (`kind: permission`, `msg: approve Bash: touch rex-spike-probe.txt`) and `done` in a Rex block, as it does under herdr-core, and nothing at its folder trust prompt (verified). Codex and Copilot CLI were not run here: Codex reports OSC 7501 under herdr-core and Copilot CLI does not, so a Copilot agent would be read-only from the phone.
- collied should talk to the control WebSocket (`/api/control`, subprotocol `rex.control.v1`), not run the CLI. A call costs 0.2 to 0.3 ms on an open connection against 25 to 35 ms for each `rex` process, and status, process and title changes arrive as events. collied already depends on `tokio-tungstenite`.
- collied's approval parser, menu gate and fingerprint read Rex's `format` text unchanged, and its `alacritty_terminal` emulator fed with `format`'s VT output draws the same screen (verified). Reads are settled; watches are not: `format` returns the whole scrollback, and the data stream a watch would follow starts with a binary Ghostty snapshot that `alacritty_terminal` cannot read ([Screen](#4-screen)).
- Lost or weaker for a Rex agent: `terminal.send`'s `if` and its typing hold, which Rex has no equivalent for, and probably desktop focus. Built in collied: the agent kind, the session binding (Claude Code's `sessions/<pid>.json`, as for herdr-core), `state_change_seq`, task records and worktrees.
- Shape: a third runtime beside herdr and herdr-core, `crates/collied/src/runtime/rex.rs`, with ids `rex:block:<uuid>`, one group per Rex session (`rex:session:<uuid>`), and the phone protocol unchanged.

## Evidence

Verified by running, on macOS (Darwin 27.3), with the `rex` CLI inside Rex Beta.app (`Contents/Helpers/Rex Server.rex/Contents/MacOS/rex`: 0.1.0-preview.16, build 1940, API revision 13; nothing was installed), Claude Code 2.1.322 (`--model haiku`), and the user's herdr 0.9.5 and herdr-core 0.1.19 running their agents:

- The throwaway prototype [below](#prototype), against a Rex server it starts for itself, and a longer exploratory session against a second scratch server.
- The server's self-description: `rex api list --json` and `help.all` (74 methods with JSON Schemas), `block_creator.list`, `session.describe_block`, and `/llms.txt`, which the server serves on its socket.
- collied's `prompt::Menu::parse`, `prompt::context`, `approvals::fingerprint` and `runtime::screen::Screen` on Rex screens, through a throwaway binary depending on collied by path.

How the spike kept off the user's Rex, whose server (Rex.app's, build 1858) ran throughout: apart from the first `--help` listings, every `rex` process, both scratch servers and everything they started ran under `sandbox-exec` with a profile denying reads and writes in `~/Library/Application Support/rex` and `~/Library/Application Support/com.superlogical.rex` and any connection to the user's socket. Each started from an empty environment with a scratch `HOME`, `XDG_RUNTIME_DIR`, `REX_RUNTIME_DIR` and `REX_CONFIG`, and every CLI call passed `--autostart=false` and `-S` with the scratch socket. The user's two folders were compared (names, sizes, modes and mtimes) before and after every phase: unchanged, apart from files the user's own server rewrites by itself (its block snapshots, its log, its Tailscale node's logs and netmap cache), which also changed before the first scratch server started. The user's server was never queried, so nothing here says what build 1858 supports. The sandbox kept the scratch servers from running `/usr/bin/login`, a setuid binary, so their blocks started with `shell: none`; Rex.app's terminals start through `login` on macOS.

Read, not run:

- The four Rex pages the issue links. They document neither the WebSocket protocols, nor OSC 7501, nor where the socket lives; `/llms.txt` documents the first two, and the CLI's own errors on scratch paths showed the third.
- Rex.app's binary, for its client actions and the message it answers them with while Remote Control is off. Rex.app was never opened.
- collied's runtime (`crates/collied/src/runtime`), [herdr-core.md](herdr-core.md) and [architecture.md](architecture.md#herdr-core-herdr-2).

Not verified: Codex and Copilot CLI in Rex, Linux, Rex.app (the size it gives a block it owns, whether it follows a focus change made on the server), a server started by `--autostart` (what environment it passes on), sessions restored after a server restart, and builds other than 1940.

## What Rex is

- One server per user, owning sessions, their windows (tabs) and their blocks (today, terminals). Clients such as Rex.app and the CLI connect to it. Ids (`session:<uuid>`, `window:<uuid>`, `block:<uuid>`, UUIDv7) never change and are never reused; labels can change and repeat.
- The CLI finds the local server at `$REX_RUNTIME_DIR/server.sock`, else `$XDG_RUNTIME_DIR/rex/server.sock`, else, on macOS, `~/Library/Application Support/rex/server.sock`, where Rex.app's server listens; `REX_SERVER` and `-S` name another server (verified from the CLI's errors on scratch paths). Linux without `XDG_RUNTIME_DIR` was not checked (the API examples in the binary show `/run/user/1000/rex/server.sock`). A socket path over 103 bytes is refused. The socket is `0600` in a `0700` folder, and the server admits callers by their peer credentials.
- Two WebSockets on that socket. `/api/control` (`rex.control.v1`): a `hello` naming the API revisions the client speaks, a `welcome`, then JSON requests, responses and events; each response carries the `seq` of the last event it already reflects. `/api/data`: one block's output per connection, opened with a single-use ticket from `data.connect`.
- Events: a connection receives a session's events, block events included, once it calls `session.attach` (or `session.bootstrap`); `session.attach_all` with `event_scope: session` adds sessions being created and destroyed but no block events, and `event_scope: all` adds every client joining and leaving (each CLI call is four events). `seq` counts per connection and starts again at 1 on a new one.
- A terminal block's methods include `format` (text, HTML or VT, screen and scrollback), `program_status`, `process`, `title`, `size`, `modes`, `write`, `resize`, `blob_read`, `list_dir`, `upload_begin` and `download_begin`. Its events include `program_status_changed`, `program_status_removed`, `process_changed`, `title_changed`, `size_changed`, `child_exited` and `desktop_notification`.
- Every terminal gets `REX_SESSION`, `REX_BLOCK`, `REX_BIN_DIR`, `TERM_PROGRAM=rex`, `COLORTERM=truecolor` and the server's own environment (verified: the scratch server's `REX_CONFIG` and `XDG_*` reached its blocks). The docs add `REX_SERVER`; a server started with `rex server run` did not set it.
- A client may own a block, which then takes that client's size and colors. A block no client owns or sized is 80x24.
- The server saves each terminal's screen to disk (`<state>/terminal/<pid>-<id>/<uuid>.snap`, `0600` in `0700` folders), even with `--restore=false`.
- Beyond terminals, the API stops the server, turns Tailscale on and signs it in, edits the host list, grants macOS file access, loads and runs Lua (`client.set_script`, `client.action`), asks Rex.app to perform its actions (`client.perform`), lists folders and moves files (`list_dir`, `upload_begin`, `download_begin`).

## Mapping

Every runtime call collied makes today (`crates/collied/src/runtime/mod.rs`), with herdr's and herdr-core's call, mapped to Rex. "Built" means collied does it itself; "lost" means a Rex agent goes without. `terminal.x` stands for the block method `com.superlogical.terminal.x`, called on the control connection with `session_id`, `block_id` and `args`.

| collied call | herdr | herdr-core | Rex | |
|---|---|---|---|---|
| `ping` | `ping` | `hello` | `hello` (API revision), `server.status` (version, pid, listeners, Tailscale) | equivalent |
| `session_snapshot`, `agent_list`, `poll` | `session.snapshot`, `agent.list`, `workspace.list`, polled every 1 s | `events.subscribe` | `session.list`, `session.list_blocks`, then `terminal.program_status`, `terminal.process` and `terminal.title` per terminal; then events | equivalent, push instead of poll; groups built |
| `agent_get` (ready check) | `agent.get` | `terminal.get` | `terminal.program_status` and `terminal.process` | equivalent |
| `pane_get` (worktree occupancy, `still_new`) | `pane.get` | `terminal.get` | `terminal.process` (`cwd` of the foreground process and the child) | equivalent |
| `pane_columns` | `pane.layout` | `terminal.get` `cols` | `terminal.size` | equivalent, exact |
| `detection_text` (approvals, menus, startup checks) | `pane.read` `detection` | `terminal.read` | `terminal.format {format: text}`, its last `rows` lines | equivalent (verified) |
| `agent_read`, `pane_read` (ANSI, `visible`, `recent`) | `agent.read`, `pane.read` | emulator on a watch-only attach | `terminal.format {format: vt}` into collied's emulator | built (verified for the screen) |
| `watch` | nothing to hold | watch-only attach | data connection, or `format` polled | built; open ([Screen](#4-screen)) |
| `agent_explain` | `agent.explain` | `status.kind` | the status record's `kind` | replaced (verified) |
| `agent_prompt`, `type_text` | `agent.prompt`, `pane.send_input` | `terminal.send` (`if`, `force`) | `block.event` paste, then `block.event` key Enter | equivalent, without `if` (verified) |
| `agent_send_keys`, `scroll_bottom`, `notice_digit` | `agent.send_keys`, `pane.send_text` | `terminal.keys` | `block.event` key | equivalent (verified) |
| `pane_send_text` (slash mirror, notes) | `pane.send_text` | typing attach | `block.event` paste, no Enter | equivalent (verified) |
| `pane_send_keys`, `pane_send_input` | as named | herdr only | `block.event` | herdr only, as for herdr-core |
| `agent_focus` | `agent.focus` | lost | `session.focus_block` | unverified ([Focus](#9-focus)) |
| `agent_start`, `workspace_create` | `agent.start`, `workspace.create` | `terminal.run` | `session.create` with one window running the agent | equivalent; kind and task record built |
| `tab_create` (keeps an archived worktree's workspace open) | `tab.create` | not needed | not needed: a session outlives its last block (verified) | |
| `worktree_list`, `worktree_create`, `worktree_open`, `worktree_remove` | `worktree.*` | `git worktree` | `git worktree` | built, as for herdr-core |
| `workspace_close` | `workspace.close` | refused | `session.destroy`, only for a session holding nothing but listed agents | equivalent with a guard ([Grouping](#7-grouping)) |
| `pane_processes` (`claude attach` target, transcript pids) | `pane.process_info` | herdr only | `terminal.process` (`foreground`, `ancestors`, `child`) | equivalent |
| `pane_close` | `pane.close` | `terminal.kill`, `terminal.rm` | `block.close` | equivalent |
| `agent_manifests` | `server.agent_manifests` | built | built (the CLIs on the machine) | built |
| `agent_session` (Chat, costs, context, resume, hook binding) | herdr's hooks | pid file; Codex hook | Claude Code's `sessions/<pid>.json` for the foreground pid (verified); collied's Codex SessionStart hook reading `REX_BLOCK` | built |
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
- collied therefore follows Rex on one long-lived control connection: `tokio-tungstenite` (a dependency already, 0.30.0) over a `UnixStream`, without compression, answering the server's pings (every 120 s). It calls `session.attach` for each session (block events) and `session.attach_all` with `event_scope: session` (sessions created and destroyed), not `event_scope: all`. After a reconnect `seq` starts at 1, so collied reads every session again.
- Method check: the `hello` sends `api_revision: 13` and `api_min_revision: 13`, so an older server closes the connection (1008) instead of answering in older shapes. Then `help.all`, or `block_creator.list` for the terminal's methods, must list every method collied calls, or collied does not follow that server and `doctor` names what is missing, as for herdr-core.
- collied never starts a server and never runs the `rex` CLI: an absent socket means Rex is not running, while any `rex` command would start one (`--autostart` defaults to true).

### 4. Screen

- `format` returns the screen and the whole scrollback as text, HTML or VT (SGR, truecolor, cursor moves for runs of blanks), with `trim` and `unwrap` (soft-wrapped lines joined, herdr's `recent_unwrapped`). The visible screen is the last `rows` lines of the text (verified: 27 lines for 3 of history and 24 rows). There is no visible-only or tail-only option.
- Text: approvals, menus and startup checks read it unchanged ([Approvals](#6-approvals)). ANSI: collied's `Screen` (`alacritty_terminal` 0.26.0) fed with `format`'s VT output, its final line break dropped, drew the same 24 rows as the text (verified on Claude Code's permission dialog). So `agent.read`, `pane.read`, drafts, the jump banner and notices can come from an emulator seeded by one `format` call.
- Cost: with 100,000 lines of scrollback (4.3 MB of text), `format` failed with `too_large` (the message limit is 1 MiB); with `max_bytes: 65536` it took 110 ms (text) or 58 ms (VT) and kept the whole result for `blob_read`, whose read of the last 64 KB took 20 to 35 ms. A Rex terminal keeps up to 64 MiB of scrollback. A read therefore takes the tail through `blob_read` (bytes, cut at the first line break), which bounds what crosses the socket but not the server's work.
- `rex block stream` answered "not implemented" in this build. A data connection (`data.connect`, then `/api/data` with `rex.data.terminal.v1`) delivered its first bytes 0.5 to 2.9 ms after the ticket: a binary Ghostty snapshot (`GHOSTSNP`; 5 KB for Claude Code's screen, 4.5 MB with the 100,000 lines), then the program's output with Rex's own messages as `ESC _ rex1;<nonce>;<json> ESC \` (`resize` right after the snapshot, `theme`, `child_exited`). `alacritty_terminal` cannot read the snapshot.
- Watches need a choice in phase 2: (a) follow the data connection, drop everything up to the first `rex1` message with the connection's nonce, and seed the emulator with `format` VT read at that moment: simple, but output between the snapshot and the read is applied twice or not at all, a glitch while the agent writes that lasts until it redraws; (b) poll `format` VT with `max_bytes` and `blob_read` at the watch period (250 ms), up to about 100 ms of server work per poll on a long session; (c) restore the snapshot with libghostty-vt, which the app already builds from pinned tarballs: exact, but a new native dependency for collied on macOS and Linux, and whether its C API restores Rex's snapshots is unknown. The plan takes (a), and (c) if the glitch shows.
- Size: neither `format` nor a data connection changed a terminal's size or owner (verified on a terminal resized to 100x30 with no owner). collied never calls `resize`, `own_block` or `set_theme`. A block Rex.app owns has Rex.app's size; not verified with Rex.app attached.

### 5. Input

- `rex send TEXT` (and `terminal.write`) types the text as raw bytes, with no bracketed paste and no Enter (verified with a raw-mode reader: `w1\nw2` arrived as is). `block.event` with a `paste` event pastes: inside `ESC[200~` and `ESC[201~` when the program turned bracketed paste on, newlines as `\r` otherwise, and an `ESC` in the text arrives as a space, so a pasted `ESC[201~` cannot end the paste (verified). A prompt is a paste and then a `key` event for Enter, which run in order on one connection (verified: Claude Code took the prompt and answered).
- `rex send-key` takes every named key collie sends, under the same name, and the program receives what herdr sends (verified byte by byte: `enter`, `esc`, the arrows, `shift+left`, `tab`, `shift+tab`, `backspace`, `ctrl+c`, `ctrl+enter` as `ESC[27;5;13~`, `ctrl+a`, `ctrl+b`, `ctrl+e`, `ctrl+k`, `ctrl+l`, `ctrl+u`, `ctrl+w`, `ctrl+end` as `ESC[1;5F`, `space`, characters, `alt+x`, `ctrl+alt+x`, `shift+f5`, `f12`, `alt+enter`, `shift+enter`, `ctrl+space`). `cmd+k`, `ctrl+[` and `ctrl+shift+x` send nothing to a program without a keyboard protocol, and an unknown name fails (exit 2). On the socket, a `key` event takes W3C key codes (`key_a`, `arrow_up`, `escape`) and a modifier string, so collied maps its names (checked for `esc`, `up`, `down`, `shift+left`, `ctrl+c` and `enter`).
- No write can be held to a status: `block.event` and `write` take no condition, and Rex holds nothing while someone types on the Mac. collied's ready check, `program_status` right before the write, is the only guard: the gap herdr leaves, wider than herdr-core's.
- Text without Enter (the slash mirror, an approval note) is a paste alone.

### 6. Approvals

- The status record's `kind` stands in for herdr's rule id, as for herdr-core, so the menu gate holds (`prompt::uses_menu("claude", Some("permission"))`). On the Rex text of Claude Code 2.1.322's Bash permission prompt, `Menu::parse` found the four options with the cursor on "Yes" and the decisions Approve, ApproveAlways and Deny, and `fingerprint` took its region (verified). Esc denied it (`idle`); Enter on "Yes" approved the next one (the probe file appeared in the scratch folder, then `working`, `done`).
- One dialog, one `blocked` event: status and screen stayed put while it waited (Claude Code also sent a `desktop_notification`, "Claude needs your permission"), and the next dialog brought its own event.
- `state_change_seq`: Rex's `seq` counts per connection and restarts after a reconnect, so collied keeps its own counter, moved when an event changes a block's `state`. After a reconnect, a block whose state and `updated_at` are unchanged keeps its value and any other takes a new one. Rex has no turn counter, so a dialog Claude Code re-reported during the gap is superseded, which errs on the safe side.
- `msg` is the agent's own text: logged at most, never trusted over the screen.
- Found on the way, not Rex's: at 80 columns Claude Code wraps its "Tip: ... choose \"switch to auto mode\" below" line, and `prompt::context` took the wrapped `below` as the description (`Bash: below`). A Rex block no client sized is 80 columns; a narrow herdr pane would do the same.

### 7. Grouping

- A Rex session is a group of related work, as a herdr workspace is: one group per session, `rex:session:<uuid>`, with the session's label (`session_label_changed` follows renames), numbered after herdr's and herdr-core's. Windows (tabs) are not shown. One group per server would hide the user's own grouping.
- `pane.close` is `block.close`, which ends the program; a window whose last block closes closes too. A session outlives its last block and window (verified).
- `workspace.close` would be `session.destroy`, which ends every block in the session, Rex.app's included, while the phone shows only agents. So it is allowed only while every block in the session is an agent collied lists, and refused otherwise ("close its agents one at a time"), as for a herdr-core group.

### 8. Tasks

- New task starts in herdr while herdr answers, else in a followed herdr-core, else in a followed Rex: no setting, so a machine that runs both herdr and Rex changes nothing.
- A task is a session of its own, as herdr's `workspace.create` makes a workspace: `session.create` with the task's name as label and one window whose block runs `/usr/bin/env -u <name> ... <CLI path> <args>` (`options.command`, `options.cwd`), the API form of `rex new NAME -- COMMAND` (verified with `shell: none`: Claude Code started there and reported). With Rex's default shell (a login shell, not interactive) the agent gets the user's profile `PATH`, as Rex.app's terminals do; with `shell: none` it gets the server's environment. The default was not run (the sandbox), so phase 5 checks it. `rex run` and `session.new_window` would put a task in an existing session instead.
- The `-u` list is herdr-core's sixteen names: a Rex server passes its environment to every terminal (verified), and a server started by a `rex` command inside a herdr pane would carry that pane's variables (not verified: `rex server start --env` suggests only named variables pass).
- A block has one label, which Rex.app shows, and nothing like herdr-core's `collie.kind` and `collie.worktree`. collied records the blocks it starts (kind, worktree checkout) by block id in its data folder; ids are never reused.
- Worktrees and archive as for herdr-core: `git worktree`, then `block.close`, then `git worktree remove`; archive also destroys the task's session once it holds no other block.

### 9. Focus

- `rex focus BLOCK` (`session.focus_block`) makes the block's window the session's active window and focuses the block, on the server (verified with `session.view`), with no Rex.app setting. Whether Rex.app then shows that session and comes forward is not verified.
- Rex.app's own actions (`client.tab.goto`, `client.window.*`, through `client.perform` or `rex -C`) are refused while Remote Control is off in its settings: its binary holds "Rex.app is not accepting actions. Turn on Remote Control in its Settings under Rex Server." collied never calls them: they drive the app, and that setting is the user's.
- Plan: `agent.focus` on a Rex agent is refused, as for herdr-core, until a check with Rex.app shows `session.focus_block` brings the agent forward.

### 10. Alongside the others

- Ids: a block is `rex:block:<uuid>` (46 bytes) and a group `rex:session:<uuid>` (48 bytes), within `TerminalId`'s `[A-Za-z0-9_:.-]{1,64}`. Rex ids are never reused, so a restarted or wiped server never aliases old ids. collied routes by the `rex:` prefix, as it routes `hc:`; the phone parses no id.
- Discovery as the CLI does it, from collied's own environment ([What Rex is](#what-rex-is)), never `REX_SERVER`, which can name another machine, and never the hosts list. Rex missing or stopping behaves as herdr-core does: a server that never answered is quiet, one that stops after answering logs once and its agents leave with one `flock.changed`, and `herdr_unavailable` is answered only when no runtime answers.
- `collied status`: a `rex:` line with the version and API revision (or `not running`), the socket and its folder's mode, and any method the server lacks.
- `collied doctor`: informational when Rex is absent; else the version and the socket, a warning for a server it cannot follow (API revision, missing methods), for a folder that is not `0700` and the user's, for a listener admitting anonymous callers, and for a server whose environment (read by name only, from `server.status`'s `pid`, as for herdr-core's holder) holds `HERDR_PANE_ID`, `HERDR_SOCKET_PATH` or `CLAUDE_CODE_CHILD_SESSION`. It reports Rex's own Tailscale mode as information.
- Platforms: macOS, verified. Linux, where collied and CollieTray also run, is not verified; `rex server install ssh://...` installs a server on a Linux machine, and a collied there would follow that machine's own server. Remote hosts are never followed.
- Threat model: the socket admits only the user (`0600`, peer credentials), so collied's allowlist stays the only boundary between the phone and the machine, as for herdr. What changes: the API is wider than herdr-core's (Tailscale, Lua, Rex.app's actions, files), so collied calls only `hello`, `server.status`, `help.all` or `block_creator.list`, `session.list`, `session.attach`, `session.attach_all`, `session.list_blocks`, `session.create`, `session.destroy` (guarded), `block.close`, `block.event` (`paste` and `key` only), `data.connect`, and the terminal methods `program_status`, `process`, `title`, `size`, `format` and `blob_read`; never `server.stop`, `server.tailscale.*`, `hosts.*`, `server.apple.*`, `client.*`, `resize`, `set_theme`, `write`, `list_dir`, `upload_begin` or `download_begin`. The paste neutralizes `ESC`, so a prompt cannot break out of bracketed paste. Screens are kept on disk under the state folder, same-user only. With Rex's Tailscale mode on, the user's tailnet devices (or every allowed member) reach the same terminals through Rex itself, outside collie's allowlist and audit; collie neither needs nor changes that, and `doctor` says so. The threat model gains a Rex section in phase 1.

## Shape

- `crates/collied/src/runtime/rex.rs` beside `herdr.rs` and `herdr_core.rs`, behind the same `Runtime` calls, building the shared model (`AgentInfo`, `PaneInfo`, `WorkspaceInfo`) as herdr-core does: `terminal_id` and `pane_id` both `rex:block:<uuid>`, `workspace_id` `rex:session:<uuid>`, `state_change_seq` from collied's own counter, `agent_session` from Claude Code's pid file and the Codex hook, `read_only` while no status was reported.
- Phone protocol unchanged: `claude-code` is `claude`, `error` is `unknown`.
- Client: one control connection per followed server on `tokio-tungstenite`, reconnecting with herdr-core's backoff (1 s to 30 s), and data connections only while a phone watches.
- Emulator: collied's `runtime::screen::Screen`, unchanged, seeded from `format` VT.

## Testing against Rex

The AGENTS.md rule this plan adds: a test or check that needs a live Rex starts a server of its own (`rex server run --listen unix://<scratch>/rex.sock --state-dir <scratch> --config <empty file> --restore=false`) from a clean environment with scratch `HOME`, `XDG_RUNTIME_DIR` and `REX_RUNTIME_DIR`, passes `--autostart=false` and `-S` with that socket on every call, stops only that server by its pid and deletes its folder. `--state-dir` alone is not enough: the server also writes under its runtime folder, and with neither `HOME` nor a runtime folder set aside the default socket is the user's. The socket path must stay under 104 bytes, so the scratch folder is a short one under `/tmp`.

## Effort and phases

1. Agents and status (M): discovery, the control connection, the method check, sessions as groups, status and kind from events, `collied status` and `doctor`, quiet absence, `pane.close` and the guarded `workspace.close`, the threat model section.
2. Screens (M to L): reads from `format` into the emulator, `blob_read` tails, watches on the data connection ([Screen](#4-screen) (a)).
3. Input and approvals (M): paste and key events behind the allowlist, the ready check, read-only agents without status, approvals gated on `kind` with collied's own `state_change_seq`.
4. Sessions (S): Claude Code's pid file, the Codex SessionStart hook with `REX_BLOCK`, then Chat, costs, context, resume and the permission hook's binding.
5. Tasks (M): New task, resume, worktrees and archive in Rex when neither herdr nor herdr-core answers, with the sixteen variables stripped and task records kept by block id.

What a Rex agent cannot do that a herdr one can, after all phases: take a prompt held to the status collied saw (no `if`), wait for the Mac's typing to pause, probably be focused on the desktop, and, for a CLI without OSC 7501 (Copilot CLI), report a status or take input from the phone.

## Risks

- **Preview API.** Rex is 0.1.0-preview.16, its API at revision 13, and the user's running server is an older build than the installed CLI. collied pins the revision and fails closed per missing method.
- **Scrollback cost.** `format` always renders the whole scrollback (up to 64 MiB); a long session makes each read cost tens of milliseconds on the server, and a watch that polled would cost that four times a second.
- **The snapshot gap.** Watches seeded by `format` beside the data stream can misdraw output written in between ([Screen](#4-screen) (a)).
- **The environment.** A Rex server passes its own environment to every terminal; one started from inside a herdr pane or a Claude Code session would hand its variables to every agent, as herdr-core's core did ([herdr-core risks](herdr-core.md#risks)). collied strips them from the agents it starts and `doctor` warns.
- **No input guard.** Without `if`, a dialog that opens between the ready check and the write takes the prompt's Enter, as with herdr.
- **Tailscale mode.** A Rex reachable over the tailnet is a second way into the same agents, outside collie's audit.

## Prototype

A throwaway outside the repository (scratchpad, not committed): `spike.py`, a Python client of the control and data WebSockets (standard library only), `run.sh`, which drives it end to end, and `menu-check`, a Rust binary depending on collied by path for the menu parser, fingerprint and emulator. `run.sh` refuses to start while its socket exists, starts its own server with a scratch `HOME`, runtime and state folders and an empty `REX_CONFIG`, under `sandbox-exec` with a profile denying the user's two Rex folders and socket; runs Claude Code (`--model haiku`, with `--setting-sources project,local` so the user's hooks do not fire, and a scratch `--settings` file whose hooks log `REX_BLOCK`) in a scratch session, answering its folder trust prompt there; lists blocks with their status, reads the agent's screen, sends it a prompt, prints `collied status`'s herdr lines before and after; then kills the session, stops its server by pid, deletes its folders and compares the user's Rex folders with what they were when it started.

```text
spike.py --socket SOCK list | watch SECONDS | read BLOCK [vt] | send BLOCK TEXT | keys BLOCK KEY... | stream BLOCK SECONDS FILE | bench
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

Exploratory runs (second scratch server):

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
keys:     send-key bytes as listed in Input; paste bracketed when asked, ESC neutralized; write raw
stale:    Ctrl+Z removes the record (reason foreground); a normal exit back to zsh keeps it
```

## Follow-up issues

Drafts, to be created in "Phase 8: Rex integration" after review.

### 1. List Rex agents and their status beside herdr's

Connect to a running Rex server (never start one) over its control WebSocket, follow each session's block events, and show its agents in one group per Rex session, with OSC 7501 status, status times, alerts and pushes. herdr, herdr-core and Rex work in any combination with no setting.

- [ ] Agents get ids `rex:block:<uuid>` and groups `rex:session:<uuid>`; blocks of other hosts are skipped.
- [ ] A missing Rex logs nothing and changes nothing for a machine without it.
- [ ] The client sends `api_revision` 13 as its minimum, checks every method it calls, and calls only the methods docs/rex.md lists.
- [ ] A status record whose owner is not the foreground process, or a shell in the foreground, lists no agent.
- [ ] `collied status` and `collied doctor` report Rex (version, API revision, socket, folder mode, missing methods, anonymous listeners, the server's inherited variables, Tailscale mode).
- [ ] The phone's `pane.close` closes a Rex agent's block, and `workspace.close` destroys a Rex session only while it holds nothing but listed agents.
- [ ] The threat model and architecture pages describe Rex.
- [ ] Live tests run a Rex server of their own, as AGENTS.md says.

### 2. Read Rex agent screens through collied's emulator

`format` VT into `runtime::screen::Screen` for `agent.read`, `pane.read`, drafts, the jump banner and notices, with the tail through `max_bytes` and `blob_read`; watches on the data connection, dropping the Ghostty snapshot and seeding from `format`.

- [ ] Output and patches match herdr's format, so the phone needs no change.
- [ ] Emulated text equals `format` text for Claude Code fixtures.
- [ ] A read of a terminal with 100,000 lines of scrollback succeeds and moves at most the tail.
- [ ] No read or watch changes a terminal's size or owner.
- [ ] Live tests run a Rex server of their own.

### 3. Send prompts, keys and approval decisions to Rex agents

Paste and key events behind the existing allowlist and ready check; approvals gated on the status `kind`, with collied's own `state_change_seq`.

- [ ] A Claude Code permission prompt in Rex raises an approval that Approve and Deny answer.
- [ ] An approval's fingerprint and seq stay while one dialog stays on screen, change for the next, and survive a reconnect when nothing changed.
- [ ] An agent with no status refuses prompts, typed answers and keys (`agent_not_ready`).
- [ ] Every `Key` maps to a Rex key event, checked live by the bytes a program receives.
- [ ] Live tests run a Rex server of their own.

### 4. Bind Claude Code and Codex sessions to Rex blocks

Claude Code's session from `sessions/<pid>.json` for the block's foreground pid; Codex's from collied's SessionStart hook reporting `REX_BLOCK`, as for herdr-core's `HC_TERMINAL_ID`. Check Codex's OSC 7501 reports in a Rex block.

- [ ] Chat shows a Rex Claude Code agent's conversation, and `task.sessions` leaves out sessions a Rex agent runs.
- [ ] A Codex agent in Rex reports its status and binds its session.
- [ ] Live tests run a Rex server of their own.

### 5. Start, resume and archive tasks in Rex when neither herdr nor herdr-core answers

`session.create` with the agent's command, task records by block id, `git worktree` for worktree tasks, `block.close` and `session.destroy` on archive.

- [ ] With herdr and herdr-core stopped, New task starts Claude Code and Codex in a Rex session of its own, in a worktree when asked.
- [ ] Agents start through `/usr/bin/env -u` with herdr-core's sixteen names and get the user's profile `PATH`.
- [ ] Archive closes the task's block, removes its recorded worktree and destroys its emptied session.
- [ ] With herdr or herdr-core running, tasks still start there.
- [ ] Live tests run a Rex server of their own.

### 6. Check focusing a Rex agent with Rex.app

With the owner's Rex.app, check whether `session.focus_block` brings the agent's session forward; if it does, `agent.focus` uses it, else focus stays refused. No Remote Control action is used.

- [ ] docs/rex.md and architecture.md say what Rex.app does.
