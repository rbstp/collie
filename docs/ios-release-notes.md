# iOS release notes

Text for 0.2.6, covering everything since 0.2.0, the last version on the App Store. Confirm its final build in App Store Connect once the upload is processed.

## App Store release notes

- Follow Antigravity CLI agents: their status, Chat, Terminal, approval alerts, plan usage and token cost, and start Antigravity tasks in herdr, Herdr Core or Rex.
- Choose Code Mode when you start a Claude Code task. Turn it off for a session you want to open with Remote Control.
- Answer a Claude Code question from the phone, with or without Code Mode, once the whole question is on screen. A form only partly on screen, or one an agent could have faked, is left to the terminal.
- Answer Codex's folder trust prompt with the terminal keys when a task starts in a new folder, and Copilot CLI's in a session you run in Herdr Core or Rex.
- Approve and Deny in a notification check the dialog again before they act, and Always allow appears only when its whole scope is shown.
- Agent cards show each agent's folder when several share a workspace, Herdr Core agents included.
- Usage shows plan limits again after an early reset, and prices Claude Fable 5.2 and Mythos 5.2.
- The app covers its window in the app switcher, checks links before it opens them, and clears a locked shell's screen.
- The keys that identify your phone stay in the app's own Keychain group. If a brief Secure Enclave or Keychain failure leaves a key unusable, Settings says why and lets you replace it.
- The watch shows an approval's whole command, reason and options, marks one too long for it to answer, and refreshes after an approval is decided elsewhere.
- On Linux, Chat no longer breaks Claude Code's replies at the terminal's line ends.

Update collied to 0.2.6 on each machine: most changes above need it. If a machine refuses this phone's key after the update, run `collied peers revoke` for the phone there and pair again.

Collie Herd has been tested only with Claude Code CLI, Codex CLI, GitHub Copilot CLI and Antigravity CLI; herdr's other agent integrations have not been validated. Rex support was tested with Rex 0.1.0-preview.20, a closed preview, on an Apple silicon Mac. On Linux, only Arch Linux with Omarchy has been tested.

## What to Test

- Start a Claude Code task with Code Mode off, and check that the session offers Remote Control. Start another with Code Mode on.
- Ask a Claude Code agent a question with options, once in Code Mode and once without, in a terminal wider than 75 columns. Check that the phone lists the options and that choosing one answers the question.
- Approve an Edit with a note, a Bash command and a plan from the phone.
- Start a Claude Code task in a folder Claude has not trusted, and check that Approve answers the trust prompt. Start a Codex task in a new folder and answer its trust prompt with the terminal's arrow keys and Return.
- Run Antigravity CLI in herdr or Rex with collied's hook set up, and check its card, Chat, Terminal and Usage.
- With your paired machines, check approvals, notifications, Live Activities, and that the agents load without a manual retry after the app sits in the background for about 15 minutes.

Use Claude Code CLI, Codex CLI, GitHub Copilot CLI or Antigravity CLI for these checks; other agents have not been validated with Collie.
