# ADR-0009: Antigravity CLI is read like the other agents, through its own hook, files and layout

- Status: accepted
- Date: 2026-11-20 (recorded from the comments that came with Antigravity CLI support, #890)

## Context

Antigravity CLI (`agy`) joined Claude Code, Codex and Copilot CLI as an agent collied lists, binds, prices and reflows. docs/architecture.md (Agent kinds, Reading screens, Plan usage) says what users get; this records the decisions inside the code.

## Decision

- `collied hook` serves Codex's SessionStart hook and Antigravity CLI's SessionStart and PreInvocation hooks (camelCase `hookEventName` and `conversationId`). It only reports the session to the daemon over the control socket and never injects anything into the agent or blocks it: for Codex it prints nothing; for Antigravity CLI it prints `{}`, which agy's hook contract requires. Run as Claude Code's PermissionRequest hook, which earlier versions asked for, it reads its input and sends nothing.
- The hook binds a session only when it runs under the process that leads the terminal, as for Codex: `agy`, possibly under a `sh` it started.
- `collied doctor` reads Antigravity CLI's config from `ANTIGRAVITY_CLI_CONFIG_DIR` (default `~/.gemini/config`, where `hooks.json` lives) and its data from `ANTIGRAVITY_APP_DATA_DIR` (default `~/.gemini/antigravity-cli`, holding `brain/`). In `hooks.json` top-level keys name hooks, a hook counts unless `enabled` is false, and its session and invocation events hold flat handler arrays. herdr 0.9.5 writes version 4 of its Antigravity CLI hook at `<config>/hooks/herdr-agent-state.sh`, the version doctor expects.
- Costs: each conversation's transcript is `<brain>/<uuid>/.system_generated/logs/transcript.jsonl`, under a hidden folder, so it is scanned directly rather than through `walk`, which skips hidden folders. One record counts per planner step (`PLANNER_RESPONSE`), and its `input_tokens` excludes `cache_read_tokens`. Prices come from LiteLLM's Claude, OpenAI and Gemini chat prices (`just prices`).
- Transcripts: Antigravity CLI wraps a user prompt in `<USER_REQUEST>...</USER_REQUEST>` followed by metadata tags; the wrapper is removed when present, so Inbox and Chat show only what the user wrote.
- Subagents: Antigravity CLI writes each spawned subagent and swarm worker to `<brain>/<parent>/.system_generated/subagents/<child>.json` with `state` (`SUBAGENT_STATE_ALIVE` while kept, `SUBAGENT_STATE_KILLED` once killed), `conversationId` and `subagentDescriptor` (`typeName` and `role`). Because an idle subagent or waiting swarm worker stays `SUBAGENT_STATE_ALIVE`, collied reads `<brain>/<child>/.system_generated/logs/transcript.jsonl` incrementally: step 0, a `USER_INPUT`, or a `SYSTEM_MESSAGE` carrying `[Message]` starts a run; a `PLANNER_RESPONSE` with active `tool_calls` keeps it running, updates activity (`toolSummary`, file basename, query or tool name) and sums `input_tokens + cache_read_tokens` with the run's `output_tokens`; a `PLANNER_RESPONSE` with no `tool_calls`, calling `swarm_wait`, or with status `ERROR` or `CANCELLED` ends the turn (a turn paused on a backgrounded `run_command` shows idle until its completion message wakes the subagent). Swarm worker titles strip the `Swarm Worker (...)` wrapper when non-empty.
- Reflow: Antigravity draws assistant prose, expanded thinking and Glamour lists at indent 2 with no column-0 marker, tool output at indent 4, and its input box between full-width `─` rules around a column-0 `>` prompt at the bottom. Prose wraps at `cols - ind - 3`: its alt-screen viewport keeps 1 column for its scrollbar and Glamour pads markdown with 2 columns on each side (an 80-column pane gives 75 columns of text at indent 2). The input box wraps at the rule width minus 4: 2 for the `> ` prompt or its indent, 1 for the cursor and 1 right margin (an 80-column terminal gives 76, a 40-column rule 36). A sent prompt stays in the scrollback as a `>` row and indent-2 rows; its rows join at the rule width minus 3, inferred rather than measured, since a wider guess only misses joins while a narrower one would join typed newlines.

## Consequences

- A change to agy's hook payload, files or layout is checked against these rules and the fixtures in the tests that cover them.

## Code

- `crates/collied/src/hooks.rs`, `crates/collied/src/doctor.rs`, `crates/collied/src/costs.rs`, `crates/collied/src/reflow.rs`, `crates/collied/src/runtime/agents.rs`, `crates/collied/src/transcript.rs`, `crates/collied/src/transcript/provider_subagents.rs`
