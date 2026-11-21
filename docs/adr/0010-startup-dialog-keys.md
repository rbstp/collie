# ADR-0010: Keys reach a silent agent's startup dialog only while it shows

- Status: accepted
- Date: 2026-11-21 (recorded from the comments of #895, #897 and the Antigravity CLI sign-in test)

## Context

The phone sends keys and keystrokes only to an agent whose status says what it shows, since a key into an unseen dialog can answer it. Some agents report nothing, or report the wrong state, at their startup dialogs: Codex 0.171.0 reports nothing at its folder trust prompt, and Copilot CLI 1.0.112 reports `idle` at its folder trust box (Copilot stays read-only otherwise, #745). Without an exception the owner could not get past those dialogs from the phone.

## Decision

- Keys and keystrokes, which add no Enter, also reach such an agent as keys to a blocked agent, only while its startup dialog fills the screen:
  - a Codex that has reported nothing (`unreported`, its kind taken from its foreground CLI), never a Codex label that a retained report gives another CLI, nor a Claude Code with no status (#895);
  - a Copilot CLI whose own report (`reported`) and foreground process (`cli`) are both Copilot's, while its trust box is up, and only at the terminal's own size (`pane_size`, herdr-core and Rex only), with the box's rows counted in display cells, a wide character's two included (#897).
- The dialog is known by its screen alone (`prompt.rs`). Codex's folder trust prompt or update picker, as Codex 0.160.1 to 0.171.0 draw them: blank rows, the heading, indented text, a blank row, options numbered from 1 with one cursor, a blank row and the key hint, nothing else. Codex draws its composer under any transcript, so no screen where agent output shows passes. Copilot CLI's folder trust prompt, as Copilot 1.0.112 draws it: its tab bar on the first row, a box as wide as the terminal from its first column, its bottom border on the terminal's last row, `Confirm folder trust` on its first row over a rule, one `❯` option and the key hint on its last rows, widths in display cells. Copilot draws its input box and status under anything an agent or a tool writes, so their text never ends the screen.
- Only the dialog's own keys go: arrows, Enter, Esc and one digit. Enter, Esc or a digit can close the dialog (a digit picks its option at once), and a digit goes as a key, as one pressed in the dialog would. No Enter of collied's is added.
- After a key that can answer the dialog, collied keeps the dialog's screen and the time it answered. The screen read right after an answer can still show the dialog it answered, so the same screen within `SCREEN_SETTLE` of an answer counts as no dialog, and even the dialog's own keys wait until the screen moves on.
- Text sent to a blocked agent returns an audit target that never names the text.
- Antigravity CLI 1.5.13 reports `blocked` at its sign-in prompt, so it takes keys there as any blocked agent does.

## Consequences

- A new agent version that reports nothing, or something else, at a startup dialog needs its screen checked before it joins these rules.
- The tests in `drive.rs` and `runtime/rex/tests/input.rs` hold each rule, on herdr and on Rex.

## Code

- `crates/collied/src/drive.rs` (`keystroke_agent`, `send_text`, `startup_key`, `answers_startup`, the `answered` map)
- `crates/collied/src/runtime/mod.rs` (`AgentInfo::unreported`, `cli`, `reported`, `Runtime::pane_size`)
- `crates/collied/src/runtime/rex/tests/input.rs`, `crates/collied/src/prompt.rs` (the startup dialog screens)
