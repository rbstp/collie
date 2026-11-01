# herdr-core (Herdr 2) backend plan

Spike for [#629](https://github.com/rbstp/collie/issues/629), 2026-11-01. Nothing here is built: this page is the plan, the evidence behind it and the follow-up issues. herdr's own behavior is in [architecture.md](architecture.md#herdr-integration-verified-against-093-source-and-live-socket).

## Summary

- herdr-core 0.1.16 is enough to follow Claude Code and Codex agents beside herdr's. It already reads a screen as text (`terminal.read`), pastes a prompt with Enter (`terminal.send`, refusing a blocked program) and presses named keys (`terminal.keys`); the issue's list, taken from Herdr UI's client, predates these or leaves them out. collied still needs its own emulator, but only for what text cannot carry: ANSI output for watches and the reads that look at style (the jump banner, the draft box's dim placeholder).
- Status comes only from the agent's OSC 7501 report. Claude Code 2.1.316 and Codex 0.166.0 report it (verified); Copilot CLI 1.0.105 does not (verified), nor does Devin CLI (herdrdev/herdr#5700). A Copilot agent in herdr-core shows as `unknown`, with no approvals and no done alert.
- A `blocked` report carries `kind: permission` and a one-line `msg` (`approve Bash: touch hc-spike-probe.txt`). It replaces herdr's rule id and `agent.explain` as the gate before collied reads a menu; collied's menu parser reads the emulated screen unchanged (verified).
- Lost for herdr-core agents: desktop focus (`agent.focus`), the herdr workspace a Herdr UI tab sits in, and status for agents without OSC 7501. Moved into collied: groups, worktrees, the agent's session id and the agent kind.
- Shape: one runtime boundary in collied with a herdr and a herdr-core implementation, agents of both in one flock, ids kept apart by prefix, the phone protocol unchanged. Write a small tokio client in collied with Herdr UI's `herdr-core-client` as a reference, not a dependency, and emulate with `alacritty_terminal` directly.

## Evidence

Verified by running, on macOS with herdr 0.9.5, herdr-core 0.1.16 (`hc`, already installed in `~/.local/bin`; nothing was installed), Claude Code 2.1.316, codex-cli 0.166.0 and Copilot CLI 1.0.105:

- The throwaway prototype below, against the live core, with the installed collied and its two herdr agents running beside it.
- `hc schema` (the OpenRPC document), `hc run`, `hc ls`, `hc get`, `hc exec env` and the core's state folder.
- Copilot's and Codex's own bundles, searched for OSC 7501 strings.

Read, not run:

- Herdr UI (penso/herdr-gpui at `7bc7090`): `crates/herdr-core-client` (endpoint, framing, credit, the methods it requires), `crates/herdr-terminal` (the emulator) and `crates/herdr-gpui/src/core_terminals` (how it groups and shows Herdr 2 tabs).
- herdrdev/herdr#5700 (Devin CLI and OSC 7501, closed as a feature request).
- collied's herdr calls (`crates/collied/src/herdr.rs` and its callers).

Not verified: Herdr.app's behavior (only the state it leaves in the core was read), herdr-core versions other than 0.1.16, Linux, and key names other than `enter` and `esc`.

## What herdr-core is

- One core per user and session (`HC_SESSION`, `default` when unset). The socket is `HC_SOCKET`, else `socket` in `~/.config/herdr-core/config.toml`, else `$XDG_RUNTIME_DIR/herdr-core/<session>.sock` (or `/run/user/<uid>/...` when that folder exists), else `~/.local/state/herdr-core/<session>/<session>.sock`. On this Mac it is the last: a `0755` socket in a `0700` folder.
- JSON-RPC 2.0, one message per line; a connection keeps answering, unlike herdr's one request per connection. `hello` with `framed: true` switches it to frames (`[channel u32 LE][kind u8][len u32 LE][payload]`, kinds DATA, CREDIT, CLOSE, RESIZE, FOCUS, FACTS); RPC stays on channel 0 and each attach gets a channel. The core grants 256 KiB of credit and repaints past 4 MiB of lag.
- `hello` answers generation 1, version 0.1.16, 32 methods and the features `framed`, `attach`, `events`, `status`, `shell`.
- A terminal's address (`t7`) is stable "forever on this core"; the core has a permanent random id that resets only when its state is wiped. Terminals on machines the core added are `machine/t7`.
- Every terminal gets `HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET` and `HC_SCOPE`, plus the environment of whatever started the core (see [Risks](#risks)).
- Beyond terminals, the API also runs programs and edits files on this and other machines (`machine.exec`, `file.*`, `machine.add` over ssh), stores opaque documents (`doc.*`) and stops the core (`server.stop`).

## Mapping

Every herdr call collied makes today, by caller (`herdr.rs` wrappers). "Built" means collied does it itself; "lost" means a herdr-core agent goes without.

| herdr call | collied uses it for | herdr-core | |
|---|---|---|---|
| `ping` | doctor, status, reachability | `hello` (version, generation, core id, methods) | equivalent |
| `session.snapshot` | workspaces, panes, agents, stars, worktree settling | `terminal.list`, or the snapshot `events.subscribe` starts with | equivalent; groups built |
| `workspace.list` | reconcile, `flock.changed` on renames | none | built ([groups](#workspaces-and-tabs)) |
| `workspace.create` | New task | none: a group is a label on the terminals collied starts | built |
| `workspace.close` | archive | `terminal.kill`, then `terminal.rm` | equivalent |
| `tab.create` | new shell pane | `terminal.run` without argv (the machine's shell) | equivalent |
| `worktree.list` | New task, Git changes, archive | none | built (`git worktree list --porcelain`) |
| `worktree.create`, `worktree.open` | New task in a worktree, pull request worktree | none | built (`git worktree add`), then `terminal.run` in it |
| `worktree.remove` | archive | none | built (`git worktree remove --force`), after `terminal.kill` |
| `agent.list` | 1 s reconcile, approvals, Drive | `events.subscribe` (status, turn, title, foreground, created, finished, removed, `missed`) | equivalent, push instead of poll |
| `agent.get` | ready checks before writes | `terminal.get` (`status`, `status_ms`, `turn`, `foreground`) | equivalent |
| `agent.start` | New task, resume | `terminal.run {argv, cwd, name, labels, no_parent, no_notify}` | equivalent; kind built |
| `agent.focus` | focus on the desktop | none: Herdr UI and Herdr.app own their windows | lost |
| `agent.read` (`recent_unwrapped`, ANSI) | `agent.read`, `agent.watch` | `terminal.read` is text only | built (emulator; [screen](#screen)) |
| `pane.read` (`visible`, `recent`, ANSI) | drafts, banner, notices, watches | as above | built (emulator) |
| `pane.read` (`detection`, text) | approvals, menus, startup checks | `terminal.read` (screen only) | equivalent (identical text, verified) |
| `agent.explain` | the rule id that gates menu reading | `status.kind` (`permission`, `question`, `auth`) and `status.app` | replaced ([approvals](#approvals)) |
| `agent.prompt` | prompts | `terminal.send` (paste, Enter, refuses `blocked`) | equivalent (verified) |
| `agent.send_keys`, `pane.send_keys` | key strip, decisions, draft clearing | `terminal.keys` (named keys, encoded for the program's keyboard modes) | equivalent (`esc` verified) |
| `pane.send_input` (text and Enter in one write) | `agent.type_text`, `terminal.run` | `terminal.send`, with `force` for a blocked agent | equivalent |
| `pane.send_text` (raw, no Enter) | notice digits, Ctrl+End, slash mirror, approval notes | `terminal.keys` for single keys; raw bytes only through a typing attach | built ([input](#input)) |
| `pane.get` | shells, star cleanup | `terminal.get` | equivalent |
| `pane.layout` | wrap width for Claude Code and Codex | `terminal.get` `cols`, the terminal's own width | equivalent, exact |
| `pane.process_info` | the `claude attach` target, transcript pids | `terminal.get` `foreground` (leader's argv and pid only) | equivalent for the leader |
| `pane.close` | archive, failed starts | `terminal.kill`, `terminal.rm` | equivalent |
| `server.agent_manifests` | which kinds New task offers | none | built (the CLI on the machine) |
| `agent_session` (from herdr's hooks) | Chat, costs, context, resume, the Claude Code hook's binding | none: herdr's hooks exit without `HERDR_PANE_ID` | built ([sessions](#the-claude-code-hook-and-herdr_pane_id)) |

The architecture page describes a herdr event subscription; the code has none and polls `agent.list` and `workspace.list` every second. herdr-core's `events.subscribe` replaces that poll for its terminals: a snapshot with `seq`, then every change in order, a heartbeat every 15 s, and `missed` with a fresh snapshot for a subscriber that fell behind.

## Questions

### Workspaces and tabs

herdr-core has neither, and the two apps keep their grouping apart from each other:

- Herdr UI keeps each Herdr 2 tab in its own tab store (`browser-tabs.json` in its state folder), as `Location::CoreTerminal { panes, observe }` under a herdr workspace. It sets no label and writes no document in the core, so the core cannot tell which herdr workspace a tab sits in.
- Herdr.app writes its layout to the core: the document `herdr-app` holds `spaces` (`id`, `name`, `folder`, `tabs`, each a tree of panes naming terminals), and each terminal carries the labels `herdr.space` and `herdr.tab`.

So a herdr-core agent goes in a group of its own, never in a herdr workspace. Phase 2 makes one group per core (`hc:<core id>`, labelled "Herdr 2", numbered after herdr's workspaces). Herdr.app spaces as groups (`doc.get herdr-app`, read only) can follow if wanted; reading Herdr UI's private tab store is not worth it.

### Agent identity and kind

- `status.app` once the agent has reported: `claude-code` (Claude Code 2.1.316) and `codex` (codex-cli 0.166.0). collied maps those to its kinds `claude` and `codex`.
- Before the first report, and for agents that never report, the basename of `foreground.argv[0]` (`foreground.name` is no help: Claude Code sets its process name to its version, `2.1.316`). Copilot is then `copilot` with status `unknown`.
- Labels only on terminals collied starts (`collie.kind`), as a tie-breaker.
- Codex reports OSC 7501 (its binary holds `]7501;state=`; live, it reported `idle`, then `working` while starting). Copilot CLI 1.0.105 does not: its bundle has no 7501 string, and live it reported nothing (`reported: false`) at its folder trust prompt. Codex also redraws its title with a spinner, one `title` event every 100 ms while it works; collied ignores title events for the flock.

### Screen

- `terminal.read` answers the screen as text in about 0.14 ms and matched the emulated screen exactly for Claude Code (idle, blocked, after a turn) and Codex. That covers approvals, menus, notices and the startup checks, which read text.
- Watches, drafts and the jump banner need ANSI: the banner is a label on an SGR background, the draft's placeholder is dim, and the phone renders colored output. collied would emulate those terminals itself: `terminal.attach {observe: true}`, bytes into `alacritty_terminal`, rows rendered back to SGR. alacritty marks soft-wrapped rows (`WRAPLINE`), which gives `recent_unwrapped` directly.
- Cost per agent (measured, 200 columns): parsing runs at 160 to 180 MB/s, so CPU is negligible next to an agent's output (about 23 KB in a 14 s Claude Code session with two turns). Memory grows with history: about 6 MB more at 2,000 lines and 37 MB at 10,000 than with none.
- collied need not stay attached: a watch-only attach paints the current screen at once (first bytes in 0.85 to 1.2 ms, 3 to 13 KB). It attaches while a phone watches an agent, or for one read (a draft, a banner check), and otherwise follows events only. Approvals read text, so a blocked agent needs no attach.
- A watch-only attach does not change the size: requested at 60x20, the terminal stayed 200x120. A typing attach does: at 100x30 the terminal became 100x30 and went back to 200x120 when it closed. A terminal no client has sized runs at 200x120.
- `terminal.read` with history (`lines: 500`) on Claude Code answered `how: stitched` (a full-screen app read by scrolling it) after 2.5 s. collied keeps history from its own emulator and never asks the core for it.

### Approvals

- `blocked` with `kind: permission` replaces herdr's rule id and `agent.explain` as the gate: collied parses a menu only for `app: claude-code` with kind `permission` or `question` (forms). The rule id in the fingerprint becomes the kind; `state_change_seq` becomes `turn` plus `status_ms` (Claude Code's turn moved from 0 to 1 when it blocked, and `status_ms` moves on every word).
- Verified: 2.7 s after the prompt, the terminal reported `blocked`, `kind: permission`, `msg: "approve Bash: touch hc-spike-probe.txt"`, while the screen showed the dialog. `prompt::Menu::parse` on the emulated screen found the four options with the cursor on "Yes", and `prompt::context` gave `Bash: Create empty probe file\ntouch hc-spike-probe.txt`. `terminal.send` was refused (`kind: blocked`, "typing would answer its dialog"), and `terminal.keys ["esc"]` denied it: the agent went back to `idle`.
- Everything else in [Approvals](architecture.md#approvals) holds, since it reads the screen text herdr-core gives unchanged. The `msg` is the agent's own text: collied may log it like the snippet, never trust it over the screen.

### Input

- `terminal.send` replaces `agent.prompt` and `agent.type_text` (`force: true` for a blocked agent). It waits until nobody has typed for about 3 s and holds keys pressed meanwhile, so a prompt sent while someone types on the Mac lands after them instead of inside their text. A send to an idle Claude Code took 230 ms; with `wait: true` it also returns how the turn ended (`done`, turn 2, after 1.4 s for a one-word answer).
- `terminal.keys` replaces `send_keys` for the closed `Key` list, encoded for the program's keyboard modes (the core knows the kitty and application modes; collied would not). Key names for Ctrl+End and every `Key` must be checked against `hc keys`; only `enter` and `esc` were run.
- `pane.send_text` has no RPC equivalent. Single characters (notice digits) go through `terminal.keys`; the slash mirror and approval notes, which paste without Enter, need DATA frames on a typing attach opened at the terminal's current size, since a typing attach resizes the terminal to the size it asks for (verified). That attach is opened for the write and closed after.
- The allowlist and every check before a write stay as they are; only the last call changes.

### New task, resume and worktrees

- `terminal.run {argv, cwd, name, labels, env, no_parent: true, no_notify: true}` starts an agent; `argv[0]` is the absolute path of the CLI, never a bare name, since a name that matches a template starts the template with its body as the first message. Resume and ultracode keep their arguments; Codex keeps `--no-daemon` (hc requires it too, or Codex runs commands outside the terminal).
- Worktree create, open and remove move into collied (`git worktree`), as do the checks it already runs around them; archiving a herdr-core task kills and removes its terminals, then the worktree.
- When both run, New task starts in herdr, which gives the agent herdr's screen rules, focus and workspace. It starts in herdr-core only when herdr does not answer, so a machine moved to Herdr 2 needs no setting.
- A terminal collied starts shows in Herdr.app's agent list (which lists every terminal that reports a status) but in no Herdr UI tab: Herdr UI shows only the tabs it saved.

### The Claude Code hook and `HERDR_PANE_ID`

- herdr's integration hooks (`herdr-agent-state.sh` for Claude Code, Codex's SessionStart hook) exit unless `HERDR_ENV=1` and `HERDR_PANE_ID` are set, and herdr-core has no `pane.report_agent_session`, so a herdr-core agent has no `agent_session`: no Chat, costs, context or resume exclusion, and the `collied hook` report (keyed by session id) binds to nothing.
- Claude Code: collied already reads `~/.claude/sessions/<pid>.json` (`pid`, `sessionId`, `cwd`) for subagents. The terminal's `foreground.pid` gives the session id from it without a hook (read from an existing session file; not run under herdr-core).
- Codex: collied's own SessionStart hook, reading `HC_TERMINAL_ID` from its environment and reporting it with the session id over the control socket, like `collied hook` does for permissions.
- Focus on the desktop is lost; everything else keyed on `HERDR_PANE_ID` follows from the session id.

### Socket discovery, access and the threat model

- Discovery as `hc` does it ([above](#what-herdr-core-is)), from collied's own environment. collied never starts a core: an absent socket means herdr-core is not running. The CLI does start one: `hc server status` with no core running started one (uptime 6 ms).
- Access is same-UID, as for herdr: the socket is `0755` but sits in a `0700` folder, and there is no authentication. collied's allowlist stays the only boundary between the phone and the machine.
- What changes: the API is wider than herdr's. It runs programs and writes files on this and every added machine, stores documents other apps read, and stops the core. collied's client calls only `hello`, `events.subscribe`, `terminal.list/get/read/attach/send/keys/run/kill/rm`; never `machine.*`, `file.*`, `doc.set`, `server.*`, `terminal.put`, `line`, `on` or a template. It skips addresses with `/` (other machines), so the phone never reaches another machine through the core. `doctor` checks the folder is `0700` and owned by the user. The threat model gains a herdr-core section with these limits when phase 2 lands.

## Both at once

- Ids: a herdr-core terminal is `hc:<core id>:<address>` (the core id is 32 hex digits, so `hc:<id>:t7` is 38 bytes), within `TerminalId`'s `[A-Za-z0-9_:.-]{1,64}`; herdr's are `term_<hex>`. The group is `hc:<core id>`. The phone never parses ids, so it needs no change and no runtime field; collied routes each call by prefix. A wiped core gets a new id, so old addresses never alias new ones.
- Each runtime has its own loop, outage state and backoff. A runtime that never answered is quiet: no warning, no `flock.changed`, no error to the phone. A runtime that stops after answering logs once and drops its agents with one `flock.changed`, as herdr's outage does today. `herdr_unavailable` is answered only when neither answers, or for an agent whose runtime is down.
- `collied status`: a `herdr:` line and a `herdr-core:` line (version, or "not running"), and agents from both.
- `collied doctor`: herdr stays a failure only when herdr-core does not answer either; herdr-core is informational when absent, else `herdr-core 0.1.16 (generation 1)` with the socket path, and a warning when the core is older than the minimum, speaks another generation, or lacks a method collied calls.

## Shape

- One boundary, `crates/collied/src/runtime`, with `herdr` (today's `herdr.rs`) and `core` implementations behind the calls server, drive and approvals make: list and watch agents, read text, read ANSI, prompt, type, keys, start, close. Callers stop naming herdr; the herdr implementation keeps today's behavior exactly.
- Phone protocol unchanged. `Agent.kind` maps `claude-code` to `claude`; herdr-core's `error` state maps to `unknown`, so no new `AgentStatus` value.
- Client: a tokio client in collied (line mode for RPC and events, frames only for attaches), about the size of `herdr.rs`. Herdr UI's `herdr-core-client` is the reference, not a dependency: it is `publish = false`, pulls in Herdr UI's backend, client, protocol and terminal crates, and runs on blocking threads. Generation 1, minimum 0.1.16 (`terminal.read`, `send` and `keys` are required; when they first shipped is unknown).
- Emulator: `alacritty_terminal` 0.26.0 (Apache-2.0, allowed by `cargo deny`), as Herdr UI's `herdr-terminal` uses it. It also brings its pty modules, which collied does not use.

## Effort and phases

1. Runtime boundary (M): herdr calls behind `runtime`, no behavior change.
2. Agents and status (M): discovery, `hello`, `events.subscribe`, the herdr-core group, cards, status times, push alerts, done alerts, `collied status` and `doctor`, quiet absence both ways, the threat model section.
3. Screen (M to L): watch-only attach on demand, ANSI rendering with soft wraps, `agent.read`, `agent.watch`, grid previews, banner, notices and draft reads.
4. Input and approvals (M): `terminal.send` and `terminal.keys` behind the allowlist, approvals gated on `kind`, decisions, typed answers, the slash mirror and notes through a same-size typing attach.
5. Sessions (S to M): Claude Code's pid file, collied's Codex SessionStart hook, then Chat, costs, context, resume and the hook binding.
6. Tasks (M): New task, resume, worktrees and archive in herdr-core when herdr does not answer.

What a herdr-core agent cannot do that a herdr one can, after all phases: be focused on the desktop, sit in the herdr workspace Herdr UI shows it in, and report a status when the CLI does not emit OSC 7501 (Copilot CLI, Devin CLI today). Plain shells in herdr-core are not planned.

## Risks

- **The core inherits its starter's environment.** The core used here was started from a shell inside a herdr pane running Claude Code, and every terminal it ran received that pane's `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_SOCKET_PATH` and `CLAUDE_CODE_*` variables (a messaging token included). herdr's hooks in a herdr-core agent then report its session against that herdr pane, and collied would bind the wrong session to a herdr agent. The prototype stripped them with `env -u`. collied cannot fix the core's environment; `doctor` should warn when a herdr-core terminal sees `HERDR_PANE_ID`, and herdr-core's makers could start terminals without the `HERDR_*` variables.
- **History reads scroll the program.** `terminal.read` with `lines` scrolls a full-screen app (`stitched`) and took 2.5 s; collied never asks for history.
- **Paint and read disagree by a row.** For a shell whose output filled the screen, the attach's first paint (and the emulator) showed rows 182 to 300 and an empty cursor row, while `terminal.read` showed rows 181 to 300. Claude Code and Codex screens matched. Phase 3 compares the two before relying on either for row positions.
- **API growth.** Generation 1 is young, and Herdr UI's client calls only eight methods. collied checks `hello`'s method list and fails closed per feature.

## Prototype

A throwaway Rust binary outside the repository (scratchpad, not committed), depending on collied and protocol by path for the menu parser. It resolves the socket like `hc`, opens one line connection for RPC, one for `events.subscribe` filtered on its own label `collie.spike=629`, and framed connections for attaches; it runs each agent with `/usr/bin/env -u HERDR_ENV -u HERDR_PANE_ID ...` and kills and removes every terminal it started. It never sent input to, read or attached to any other terminal (the one other live terminal was listed and skipped).

```text
hc-spike claude <worktree>    Claude Code: list, read, permission prompt, deny, prompt, sizes
hc-spike codex <scratch dir>  Codex: does it report OSC 7501
hc-spike copilot <scratch>    Copilot CLI: does it report OSC 7501
hc-spike paint <scratch>      a shell: one-shot attach and read latency
hc-spike bench                emulator throughput and memory, no core
```

Claude Code run (abridged):

```text
[ 0.117s] hello: version="0.1.16" generation=1 features=["framed","attach","events","status","shell"] methods=32
[ 0.135s] terminal.run -> t2 in 17.6ms
[ 1.172s] first report: status="idle" app="claude-code"
[ 1.982s] attach observe=true at 60x20: 3164 bytes; terminal size now 200x120 (was 200x120)
[ 1.988s] screen after attach: emulator 120 rows vs terminal.read 120 rows: identical
[ 2.217s] terminal.send (permission prompt) -> {"started":true,"status":"idle","turn":0} in 228ms
[ 6.321s] after send: status="blocked" kind="permission" app="claude-code" msg="approve Bash: touch hc-spike-probe.txt"
[ 6.328s] blocked screen: emulator 25 rows vs terminal.read 25 rows: identical
[ 6.328s] prompt::Menu::parse on emulated screen: options=["Yes", "Yes, and always allow access to <worktree> from this project",
          "Yes, and switch to auto mode · ...", "No"] cursor=0 context="Bash: Create empty probe file\ntouch hc-spike-probe.txt"
[10.123s] terminal.send while blocked -> {"code":-32000,"data":{"kind":"blocked"},"message":"t2 is blocked (permission)"}
[10.156s] terminal.keys esc -> {"status":"blocked","turn":1}; then status idle
[11.574s] terminal.send wait=true (pong) -> {"started":true,"wait":{"status":"done","turn":2,...}} in 1.42s; screen shows pong
[15.083s] terminal.read lines=500: 120 rows, how="stitched" (2.5 s)
[15.596s] attach observe=false at 100x30: terminal size now 100x30
[16.107s] after closing that attach: terminal size 200x120
[17.615s] terminal.kill t2, terminal.rm t2: {"removed":"t2"}; spike terminals left: 0
events: foreground, title, status idle, working, blocked(permission), turn 1, idle, working, done, turn 2, size 100x30, size 200x120, finished, removed
collied status before and after every run: herdr 0.9.5, the same 2 agents, unchanged
```

Other runs:

```text
codex:   first report status="working" app="codex" after 1.6 s (idle, then working); screen identical to terminal.read
copilot: status=null reported=false after 25 s at its folder trust prompt (not answered); screen identical to terminal.read
paint:   one-shot attach to first paint 0.85 to 1.2 ms (10 runs); terminal.read 0.13 to 0.20 ms
bench:   200x50, 4 MB of output: 156 to 181 MB/s; RSS +4.8 MB (no history, includes the 4 MB input), +10.4 MB (2,000 lines), +41.5 MB (10,000 lines)
```

Clean-up: no spike terminal is left (`hc ls --all`); Claude Code's, Codex's and Copilot's configuration files are unchanged (compared with copies taken before the runs). Copilot left one empty session folder for the scratch directory.

## Follow-up issues

Drafts, to be created in "Phase 7: Improvements" after review.

### 1. Put collied's herdr calls behind a runtime boundary

Move every `herdr::` call in server, drive, approvals, transcript, doctor and status behind a `runtime` module with a herdr implementation, so a second runtime can sit beside it. No behavior change.

- [ ] No module outside `runtime` calls `herdr::` directly.
- [ ] `just test` passes with no test changed except imports.
- [ ] Nothing in `docs/protocol/` changes.

### 2. List herdr-core agents and their status beside herdr's

Connect to a running herdr-core (never start one), follow `events.subscribe`, and show its agents in one "Herdr 2" group per core, with OSC 7501 status, status times, alerts and pushes. herdr only, herdr-core only and both work with no setting.

- [ ] Agents get ids `hc:<core id>:<address>`; other machines' terminals (`box/t7`) are skipped.
- [ ] A missing herdr-core logs nothing and changes nothing for a herdr-only machine, and a missing herdr does the same for a herdr-core-only machine.
- [ ] `collied status` and `collied doctor` report herdr-core (version, generation, socket, folder mode, missing methods, an inherited `HERDR_PANE_ID`).
- [ ] The client calls only the methods this plan lists; the threat model and architecture pages describe herdr-core.

### 3. Read herdr-core agent screens through collied's own emulator

Watch-only attaches into `alacritty_terminal`, rendered back to ANSI with soft wraps, for `agent.read`, `agent.watch`, grid previews, the jump banner, notices and draft reads. Attach only while needed.

- [ ] Watch output and patches match herdr's format, so the phone needs no change.
- [ ] No attach resizes a terminal.
- [ ] Screen text from the emulator equals `terminal.read` for Claude Code and Codex fixtures.

### 4. Send prompts, keys and approval decisions to herdr-core agents

`terminal.send` and `terminal.keys` behind the existing allowlist and checks; approvals gated on `status.kind` and bound to `turn` and `status_ms`; the slash mirror and notes through a typing attach at the terminal's current size.

- [ ] A Claude Code permission prompt in herdr-core raises an approval that Approve and Deny answer.
- [ ] Every `Key` maps to an `hc` key name, checked live.
- [ ] Draft sync and the slash mirror work on a herdr-core Claude Code agent.

### 5. Bind Claude Code and Codex sessions to herdr-core terminals

Claude Code's session from `~/.claude/sessions/<pid>.json` for the terminal's foreground pid; Codex's from a collied SessionStart hook reporting `HC_TERMINAL_ID`. Chat, costs, context, resume and the permission hook then work for herdr-core agents.

- [ ] Chat shows a herdr-core Claude Code agent's conversation.
- [ ] `task.sessions` leaves out sessions a herdr-core agent runs.
- [ ] `collied doctor` reports the Codex hook.

### 6. Start, resume and archive tasks in herdr-core when herdr is not running

New task, resume, worktree and pull request worktree tasks, and archive, through `terminal.run`, `git worktree` and `terminal.kill`, used only when herdr does not answer.

- [ ] With herdr stopped, New task starts Claude Code and Codex in herdr-core, in a worktree when asked.
- [ ] Archive kills the task's terminals and removes its worktree.
- [ ] With both running, tasks still start in herdr.
