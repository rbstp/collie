# ADR-0011: Code Mode is set on Claude Code's own command, or as a setting through herdr

- Status: accepted
- Date: 2026-11-21 (recorded from the comments of #888)

## Context

New task can start Claude Code with Code Mode on or off (`CLAUDE_CODE_EXPERIMENTAL_CODE_MODE`). The value has to win over whatever the user's login profile exports, and each runtime starts the agent differently.

## Decision

- The app offers the choice for Claude Code only, and only when collied says it takes it (`task.options`): an older collied refuses the field.
- herdr-core and Rex: the variable is set on the agent's own command line (`task_argv`, through `env`), after the login profile ran, so it wins over a value the profile exports. It is never put in `--settings` there.
- herdr: `agent.start` takes no environment and types the command into a shell whose start-up files already ran, so Code Mode can only reach Claude Code as a setting, in the `env` of Claude Code's `--settings` (`agent_args`). herdr-core and Rex pass `None` to `agent_args`.

## Code

- `crates/collied/src/runtime/agents.rs` (`CODE_MODE`, `task_argv`), `crates/collied/src/runtime/herdr.rs` (`agent_args`)
- `Collie/Sources/Collie/NewTaskModel.swift` (`offersCodeMode`)
- `crates/collied/tests/drive.rs`, `crates/collied/src/runtime/rex/tests/tasks.rs`
