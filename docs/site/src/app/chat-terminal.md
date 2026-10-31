# Chat and Terminal

A Claude Code, Codex or Copilot CLI session has two views: **Chat** shows the conversation, and **Terminal** shows the agent's screen. Switch between them with the toggle at the top of the session. **Settings > Sessions > Default view** picks the view that opens first (Chat by default) and what the [grid cards](agents-tab.md#grid) preview; the toggle in a session does not change it. Plain terminals have only Terminal.

<div class="screenshots">
<img class="screenshot-light" src="../images/chat-light.png" width="300" height="652" loading="lazy" alt="An agent's Chat view: the conversation, with the prompt field at the bottom">
<img class="screenshot-dark" src="../images/chat-dark.png" width="300" height="652" loading="lazy" alt="An agent's Chat view: the conversation, with the prompt field at the bottom">
<img class="screenshot-light" src="../images/terminal-light.png" width="300" height="652" loading="lazy" alt="An agent's Terminal view: the agent's raw screen, with the key strip button and the prompt field">
<img class="screenshot-dark" src="../images/terminal-dark.png" width="300" height="652" loading="lazy" alt="An agent's Terminal view: the agent's raw screen, with the key strip button and the prompt field">
</div>

## Chat

Chat reads the agent's real conversation from its transcript on the machine.

- Replies render as Markdown. Long-press a message to **Copy** or **Quote** it; tool records and code blocks have their own Quote and Copy buttons.
- Consecutive tool calls collapse into one **Activity** row; expand it to see each call with its result.
- A shell command you run in Claude Code with `!`, such as `! git status`, shows as your message, with its output collapsed below it.
- A prompt that [PR watch](pull-requests.md#watch-a-pull-request) typed to wake the agent shows as one row with an eye, such as "PR watch: CI failed on #123". Tap it to read the full prompt.
- A recent page loads first; tap **Load older messages** for more. The search button finds text across the history, with previous and next buttons. New messages arrive without moving you while you read older ones.
- File paths the agent mentions are links: absolute paths, relative ones such as `src/main.rs` or `src/main.rs:42`, common file names such as `README.md`, and Markdown links to a path. Tap one to [open the file](#open-a-file).

When an interactive prompt needs terminal keys, Chat offers **Continue in Terminal**.

> [!NOTE]
> Chat needs the session ID that herdr's SessionStart integration reports. Until the ID arrives, Chat says so and retries; it never guesses a transcript from the folder. Codex needs herdr's hook: see [Codex](../agents/codex.md). Copilot CLI needs no hook, but Chat waits until it can tell which conversation the pane shows: see [Copilot CLI](../agents/copilot.md).

Chat shortens very long text and marks the cut. The phone fetches history only in the foreground, while Chat is open or a grid card previews it, and `collied` never writes it to logs.

### Open a file

Tap a file link in Chat, or choose **Open** on a selected path in Terminal, to preview the file on the machine, read-only:

- images (PNG, JPEG, GIF, HEIC and WebP) and PDF files up to 20 MB;
- text and source files up to 1 MB, in the monospaced style of Git diffs, with the same wrap button;
- any other file up to 20 MB, as its name, its size and "No preview".

**Copy** puts the file on the clipboard; **Share** saves or sends it. The phone keeps the file only while the preview is open.

`collied` opens only regular files inside the task folders (`roots` in [collied.toml](../operate/configuration.md#colliedtoml), your home folder by default). It refuses hidden files and folders such as `.env`, symlinks, folders, paths with `..` and larger files, and the preview says why; an agent working inside a hidden folder, such as a worktree in `.worktree`, can still show its own files. If the connection drops while a file loads, tap **Try Again**. With an older `collied`, the preview asks you to update it.

## Terminal

Terminal shows the last 200 lines of the agent's pane (500 or 1000 with **Settings > Terminal > Scrollback**), rendered with libghostty-vt in JetBrains Mono, Ghostty's default font, with its ligatures and Nerd Font glyphs.

- Lines wrap to the phone's width by default; turn off the wrap button to scroll long lines sideways.
- Long-press to select text, then copy it (Universal Clipboard included), quote it into the prompt or open an http or https link. In an agent session, **Open** opens a selected file path as a file link in Chat does. Plain terminals have no **Quote**.
- Terminal updates up to four times a second, and once a second after 5 seconds without change or while the iPhone is in Low Data Mode.

## The prompt

The composer at the bottom works in both views.

- When a prompt cannot reach the machine, the send button spins while the phone tries again, up to 5 tries in all, a minute apart or as soon as the connection is back. To stop, edit the prompt or tap the spinner.
- **Quote** adds the text to the end of the prompt as a code block, without sending it. If the prompt would grow past 32 KiB, Collie shows an error and leaves the prompt unchanged.
- To hide the keyboard, tap the hide-keyboard button or swipe down on the prompt field; in a long prompt, the first swipe scrolls back to the top. **Settings > Prompt > Keep keyboard open after sending** keeps it up after you send.
- With the keyboard down, drag the prompt field to scroll a long prompt, tap it to edit and long-press it to copy it.
- Text you left unsent in Claude Code's input box on the machine fills an empty prompt field when you open the agent.
- When a background task or agent below Claude Code's input box is selected on the machine (after ↓ on the box's last line), the phone keeps your prompt and says so, because Claude Code would not send it. Press ↑ on the key strip or on the machine to return to the input box, then send again.
- An unsent prompt stays with its agent on this phone, and its attachments for up to 23 hours.

See also [Key strip and gestures](keys.md), [Dictation and attachments](dictation-attachments.md) and [Saved prompts](saved-prompts.md).
