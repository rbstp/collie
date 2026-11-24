# ADR-0015: Copilot CLI takes input from the phone only while its input box shows

- Status: accepted
- Date: 2026-11-24 (#742, #743)

## Context

Copilot CLI 1.0.106 and later reports OSC 7501. Copilot CLI 1.0.115, run in a scratch Rex 0.1.0-preview.23 and a scratch herdr-core 0.1.22 from a scratch home, sandboxed, against a stand-in model served on localhost, reports `blocked` (kind `permission`) at its permission dialog and `blocked` (kind `question`) at its question form, but keeps `idle` (or the `done` it had) at its folder trust prompt and with a picker open (`/model`, `/theme`, `/permissions`, `/resume`). A prompt or a key sent then answers the dialog: in the #741 spike, a send at the trust prompt trusted the folder. The status cannot tell those dialogs from the input box, so a Copilot CLI agent in Herdr Core and Rex took no input from the phone, and New task did not offer it there. In herdr, herdr's own screen rules set Copilot's status, and none was checked against every picker.

## Decision

- Before every write to a Copilot CLI agent that does not report `blocked` (a prompt, a typed answer, keys, typed text, a new task's first prompt), in every runtime, herdr included, collied reads the screen and refuses unless it shows Copilot's input box at the bottom. A screen collied cannot read refuses too. The refusal says to answer the dialog in the terminal.
- The input box, as Copilot CLI 1.0.112 to 1.0.115 draw it (`prompt::copilot_box`): from the screen's last row up, one to three footer rows, each indented one column and without the word `cancel` (every dialog's key hint has it), then the box's bottom edge, at least one input row and its top edge. In Rex and herdr-core terminals, which announce truecolor and their own `TERM_PROGRAM`, the edges are `╻` then `▄` and `╹` then `▀` from column 0 to the width of the screen's widest row, and every input row starts with `┃`. In a herdr 0.9.5 pane (`TERM_PROGRAM=herdr`, truecolor announced too), and with neither `COLORTERM` nor `TERM_PROGRAM` set, Copilot draws full-width rules of `─`, its first input row starts with `❯` and the others with two spaces. Anything else refuses: a picker's search field is indented and has no `╻`, a sent prompt in the history is indented, the question form ends on its rule with no footer under it, the sidebar moves the box off column 0, and the Sessions tab and the help overlay leave no box with its footer.
- A write that adds Enter (a prompt, a typed answer, a task's first prompt) also needs the box empty: one input row with nothing after `┃` or `❯`. With `/` typed, Copilot fills the box with the command its completion list highlights, and the Enter would run it. Keys and typed text need only the box, so the phone's keyboard can type a draft and send it with Return.
- A Copilot CLI that does not report `blocked` takes one key per call: a batch could open a picker with its first key (Enter on a `/model` draft) and answer it with the next, past the one read. Typed text cannot hold Return, a newline or Tab.
- The screen is read last before the write: only the authorization check comes after it. In Rex the read is a polled `format`, never the watched stream, which can lag or replay an older screen. Herdr Core holds a `terminal.send` or `terminal.keys` until nobody has typed for about 3 s, and collied's own typed text counts, so before a prompt, a typed answer or a key collied waits until 3.5 s after it last typed into that terminal, then reads.
- Copilot CLI and the Antigravity CLI take input, `blocked` or not, only while their own program leads the terminal (`cli` is `copilot` or `agy`): a report a core keeps after the program stopped would otherwise make a program collied does not recognise, such as an editor, writable under their name. Claude Code and Codex keep taking input under an unrecognised program, for their npm installs, which run as `node`.
- Without truecolor, an empty continuation row inside the box counts as an input row, and a box of more than one row is not empty.
- At the folder trust prompt, ADR-0010's keys still go. A blocked Copilot CLI takes input as any blocked agent does, with an approval that offers no decision.
- Copilot CLI is a task kind in Herdr Core and Rex (`TASK_KINDS`), so New task and Resume start it there. Before a task's first prompt, collied waits up to 10 s for the empty input box, in herdr once herdr has started the agent and in Herdr Core and Rex once its `idle` report has held for a second, since Copilot may still be drawing its banner; a numbered option, folder trust wording, an unreadable screen or the end of the wait returns `startup_blocked` and the prompt is not sent.

## Consequences

- A dialog that opens between the read and the write receives the input: one opened at the terminal in those milliseconds, or one the phone's previous key opened that Copilot has not drawn yet. Herdr Core and Rex check the status the write was decided on, which a picker does not change. In Herdr Core, typing at the machine after the read holds the write until the typing stops, up to the call's 15 s timeout, and a dialog opened in that time receives it.
- A Copilot CLI started through another program, such as an npm install's `node`, is read-only in Herdr Core and Rex.
- A picker the phone opens, with a slash command typed and sent from the keyboard, takes no key from the phone, Esc included; it is closed in the terminal. So do Copilot's sidebar (Left at an empty input box), its Sessions tab (Tab) and its help (`?`).
- A Copilot CLI that draws its input box differently takes no input from the phone until its screens are captured and the rule follows. A Copilot CLI older than 1.0.106 reports nothing and stays read-only.

## Code

- `crates/collied/src/prompt.rs` (`copilot_box`), `crates/collied/src/drive.rs` (`copilot_input`, `copilot_started`, `keystroke_agent`, `prompt_agent`, `send_keys`, `type_text`, `send_text`, `prompt_started`, `host_agent_ready`), `crates/collied/src/runtime/agents.rs` (`read_only`, `TASK_KINDS`), `crates/collied/src/runtime/mod.rs` (`typed_at`, `input_screen`), `crates/collied/src/runtime/herdr_core.rs` (`typed_at`), `crates/collied/src/runtime/rex/mod.rs` (`polled_screen_text`)
- `crates/collied/tests/fixtures/copilot-1.0.115`; the tests in `prompt.rs`, `drive.rs`, `runtime/rex/tests/input.rs` and `tests/drive.rs`
