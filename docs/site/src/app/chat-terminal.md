# Chat and Terminal

A Claude Code or Codex session has two views: **Chat** and **Terminal**. Switch between them with the toggle at the top of the session. **Settings > Sessions > Default view** picks which one opens first; Chat is the default. Copilot CLI and plain terminals use Terminal only.

## Chat

Chat reads the agent's real conversation from its transcript on the machine.

- Replies render as Markdown, with selectable code blocks and Copy actions.
- Consecutive tool calls collapse into one row; expand it to see each call with its result.
- A recent page loads first. Tap **Load older messages** for more.
- The search button finds text across the history, with highlighted matches and previous and next buttons.
- New messages arrive while you read older ones, without jumping.

When an interactive prompt needs terminal keys, Chat offers **Continue in Terminal**.

> [!NOTE]
> Chat needs the agent's session ID from herdr's SessionStart integration. If it is missing, Chat says so and retries when the ID arrives; it never guesses a transcript from the folder. For Codex, install herdr's hook: see [Codex](../agents/codex.md).

Each read is capped at 2 MiB and 40 message blocks, and long text is shortened with a mark. History is fetched only while Chat is open, never written to logs or notifications, and loading pauses when you switch to Terminal or leave the app.

## Terminal

Terminal shows the last 200 lines of the agent's pane (500 or 1000 in **Settings > Terminal > Scrollback**), rendered with libghostty-vt in the MesloLGS NF font.

- Turn on word wrapping to fit lines to the phone's width.
- Long-press to select and copy text (Universal Clipboard included) or to open an http or https link.
- The machine checks the pane four times a second, and once a second after 5 seconds without change or while the iPhone is in Low Data Mode.

## The prompt

The composer at the bottom works in both views.

- A prompt that could not reach the machine is sent again up to 5 times, a minute apart or as soon as the connection is back, while the send button spins. Editing the prompt or tapping the spinner stops it.
- **Settings > Prompt** can keep the keyboard open after sending.
- With the keyboard down, a long press on the prompt field copies it.
- An unsent prompt stays with its agent on this phone, and its attachments for up to 23 hours.

See also [Key strip and gestures](keys.md), [Dictation and attachments](dictation-attachments.md) and [Saved prompts](saved-prompts.md).
