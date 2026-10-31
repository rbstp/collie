# iOS release notes

Text for the next combined release. Confirm its final version and build in App Store Connect once the upload is processed.

## App Store release notes

- Read GitHub Copilot CLI sessions in Chat, with formatted replies, collapsed tool activity and search.
- When Chat is your default view, Agents grid cards show the latest messages instead of the terminal.
- A session's running subagent list now sits on Liquid Glass, with the terminal and Chat scrolling under it.
- The Agents and Usage toggle keeps its accent color after you tap it, and the wrap button shows its state with a white or accent icon on glass.

Update collied on each machine to get Copilot CLI Chat and the grid chat previews. You can update the app and collied in either order.

Collie has been tested only with Claude Code CLI, Codex CLI and GitHub Copilot CLI; herdr's other agent integrations have not been validated with Collie. On Linux, only Arch Linux with Omarchy has been tested.

## What to Test

- Open a Copilot CLI session in Chat. Check replies, tool activity, search and new messages as they arrive. Run `/new` in Copilot and check that Chat follows the new conversation.
- Set Settings > Sessions > Default view to Chat. Check that grid cards show your last two messages and the agent's last two replies, that starred cards show more of each message, and that a blocked agent keeps its terminal. Set it back to Terminal and check that the cards show the terminal again.
- Connect to a machine still running collied 0.1.5. Check that the app connects without a version message and that its cards keep the terminal preview.
- Open an agent with running subagents. Check the list over the terminal and over Chat, that scrolling to the top shows the first terminal rows, and that Latest messages does not appear when Chat is already at the bottom.
- Tap the Agents and Usage toggle and check that it keeps the accent color. Turn wrap on and off and check that the icon switches between accent and white.
- Check session visibility, question answers, approvals, notifications, Live Activities and reconnection with your paired machines.

Use Claude Code CLI, Codex CLI or GitHub Copilot CLI for these checks. Other herdr integrations and other Linux distributions have not been validated with Collie.
