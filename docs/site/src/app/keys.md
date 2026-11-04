# Key strip and gestures

## The key strip

The key strip sends keys an iPhone keyboard lacks. The **Keys** button at the bottom of Terminal shows or hides it; it stays above the keyboard while you type.

```text
⇧  ⌃  esc  ←  ↑  ↓  →  ⇥  ⌫  ⏎
```

With larger text, the arrows move to a row of their own under the other keys.

**esc** sends Escape. Tap ⇧ or ⌃, then a key, to send the pair: ⇧⇥, ⌃⏎ or, in Codex, ⇧←. With ⌃ selected, a letter typed on the keyboard sends Control and that letter (A, B, C, E, K, L, U or W).

The second row scrolls: `⇧⇥ ⌃⏎ ^C`, with `⇧←` before `^C` in Codex sessions to answer a queued question, then your key shortcuts and **+** to add one.

The round joystick at the end of the second row moves the cursor: press it and drag toward an arrow to send that arrow, and keep holding to repeat it, about 12 times a second after a short pause. The direction it sends lights up; let go to stop. With ⇧ selected in Codex, the first left arrow goes out as ⇧←.

## Key shortcuts

Add your own buttons in **Settings > Prompt > Key shortcuts**, or with **+** on the strip. A shortcut sends a key or two-key combination, such as Shift+Tab, or text, such as `/effort high`, to every session or to one kind: Claude Code, Codex, Copilot CLI or Terminal.

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
