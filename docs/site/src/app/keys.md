# Key strip and gestures

## The key strip

The key strip sends the keys an iPhone keyboard lacks, such as Escape, arrows and Control combinations. In Terminal, the **Keys** button next to the prompt field shows or hides it. The strip starts hidden, and your choice applies to every session on this phone.

The first row is:

```text
⇧  ⌃  E  ←  ↑  ↓  →  ⇥  ⌫  ⏎
```

`E` sends Escape. Tap ⇧ or ⌃, then a key, to send the pair: ⇧⇥, ⇧← or ⌃⏎. With ⌃ selected, a letter typed on the keyboard sends Control and that letter (A, B, C, E, K, L, U or W). A modifier applies to the next key only; tap it again to clear it.

The second row scrolls: `⇧⇥ ⌃⏎ ⇧← ^C` in one tap, then your key shortcuts, then **+** to add one. Keys that a session cannot use are turned off.

## Key shortcuts

Add your own buttons in **Settings > Prompt > Key shortcuts**, or with **+** on the strip. A shortcut sends:

- a key or a two-key combination, such as Shift+Tab;
- or text, such as `/effort high`.

A shortcut applies to every session or to one kind: Claude Code, Codex, Copilot CLI or Terminal. A text shortcut is sent at once and leaves your draft alone. Claude Code refuses it while its input box holds unsent text; Codex and Copilot CLI add it after any text there. A text shortcut runs as a command in a terminal only when its kind is Terminal.

Swipe to delete a shortcut, and tap **Edit** to reorder. Shortcuts stay on this phone.

## Gestures

Choose what each Terminal gesture does in **Settings > Terminal > Gestures**:

| Gesture | Action |
|---|---|
| Double-tap | Paste into the prompt field |
| Pinch | Change the font size |
| Swipe sideways | Switch agents (while lines wrap) |
| Triple-tap | Send Esc (off by default) |

Swipe left for the next agent and right for the previous one, in the order of the layout you opened the session from: the grid in your order with starred cards first, the list, or the inbox with its filter, its search and the **Snoozed** section when it is open. The session keeps that order even when statuses change: agents that close are skipped, and agents that start later join only when you open a session again. If you open an agent the layout does not show, for example from a notification, a Live Activity or New Task, swiping follows the current layout with every agent in it.
