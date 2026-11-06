# Key strip and gestures

## The key strip

The key strip sends keys an iPhone keyboard lacks. The ⌘ button at the bottom left of Terminal shows or hides it; it stays above the keyboard while you type.

```text
⌃ | ⇧ | ⎋ | ⇥ | ⌫ | ⏎    joystick
```

The main keys share one bar of large symbols, with the joystick after it: control, shift, escape, tab, delete and return.

The escape key sends Escape. Tap control or shift, then a key, to send the pair: shift and tab, control and Return or, in Codex, shift and the left arrow. With control selected, a letter typed on the keyboard sends Control and that letter (A, B, C, E, K, L, U or W).

The second row scrolls, in symbols too: shift-tab (⇧⇥), control-return (⌃⏎) and control-C (⌃C), with shift-left (⇧←) before control-C in Codex sessions to answer a queued question, then your key shortcuts and **+** to add one. A key shortcut shows its keys' symbols, or its name or text when it has them.

The round joystick sends the arrows. With the strip hidden, it sits in the bottom row, beside the keyboard button, and a return key follows the keyboard button at the right edge, so you can send Return after dictation without opening the keyboard or the strip. Press it and drag toward an arrow to send that arrow, and keep holding to repeat it, about 12 times a second after a short pause. The direction it sends lights up; let go to stop. With shift selected in Codex, the first left arrow goes out as shift-left.

In a Claude Code, Codex or Copilot CLI session, you can also tap a spot in the input box to move the cursor there; the phone sends the keys for you, ↓ to the tapped line and → along it ([Terminal](chat-terminal.md#terminal) lists the few spots it skips).

## Key shortcuts

Add your own buttons in **Settings > Prompt > Key shortcuts**, or with **+** on the strip. A shortcut sends a key combination, such as ⌃⌥X, or text, such as `/effort high`, to every session or to one kind: Claude Code, Codex, Copilot CLI or Terminal.

For a key combination, turn on any of control (⌃), option (⌥), shift (⇧) and command (⌘), then type the key: a letter, a digit, one of `` ' , - . / ; = [ \ ] ` `` or f1 to f12, or pick return, escape, tab, delete, space or an arrow from the keyboard menu beside the field. The strip shows the combination in symbols. You can't save control with D, Z, R, J, M, O, T, 4 or `\`: those keys would end or suspend the agent, search its history, submit like Return or open its transcript.

Command reaches an agent only in herdr, and only one that turned on a keyboard protocol, as Claude Code and Codex do; Claude Code acts on it when you bind the key in its `keybindings.json`. A plain terminal ignores it, and herdr-core refuses it.

Text goes out at once and leaves your draft alone. Claude Code refuses it while its input box holds unsent text; Codex and Copilot CLI add it after that text. A plain terminal takes only Terminal text shortcuts, and runs them as commands.

Swipe to delete a shortcut, and tap **Edit** to reorder. Shortcuts stay on this phone.

## Gestures

Set Terminal gestures in **Settings > Terminal > Gestures**:

| Gesture | Action |
|---|---|
| Double-tap | Paste: types the clipboard into the terminal, new lines as spaces, never Return |
| Pinch | Change the font size |
| Swipe sideways | Switch agents (while lines wrap) |
| Triple-tap | Send Esc (off by default) |

Swipe left for the next agent and right for the previous one, in the order of the layout you opened the session from, as it was then.
