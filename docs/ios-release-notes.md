# iOS release notes

Text for the next combined release. Confirm its final version and build in App Store Connect once the upload is processed.

## App Store release notes

- Rex: follow your agents in Rex beside herdr's. See their status, read the terminal or Chat, send prompts, keys and approval decisions, and start, resume and archive tasks. The main gain for Rex users: Collie Herd runs Tailscale inside the app, so your phone reaches your Mac with no Tailscale app and no VPN to switch on.
- Choose where new tasks start: herdr, Herdr Core or Rex, in Settings > Sessions > Start tasks in. New task offers only the agents that runtime can start.
- Each agent shows the mark of what it runs in: herdr, Herdr Core or Rex. Hide the marks in Settings > Sessions > Runtime icons.
- Choose how opaque the app's glass is: Clear, Regular or Opaque, in Settings > Appearance > Glass.
- See Claude Code's suggested next prompt in the prompt field, and accept it with the Tab key.
- An approval no longer shows Claude Code's tip line as part of its context.
- The New task prompt stays below the navigation bar when you rotate with the keyboard open.
- Herdr 2 is now named Herdr Core everywhere in the app.
- In Herdr Core and Rex, only Claude Code and Codex take prompts and keys from the phone. Other agents show their status and screen, read-only.

Update collied on each machine to follow Rex agents, choose where tasks start and see the runtime marks. You can update the app and collied in either order.

Collie Herd has been tested only with Claude Code CLI, Codex CLI and GitHub Copilot CLI; herdr's other agent integrations have not been validated. Rex support was tested with Rex 0.1.0-preview.19, a closed preview, on a Mac, with Claude Code and Codex. On Linux, only Arch Linux with Omarchy has been tested.

## What to Test

- On a Mac running Rex, open a Claude Code or Codex agent in a Rex session. Check that it appears in a group named after its session, that its status follows the agent, and that Terminal and Chat show it. Send it a prompt and a few keys, then approve one Claude Code approval and deny another from the phone.
- Set Settings > Sessions > Start tasks in to Rex. Check that New task offers only Claude Code and Codex. Start a worktree task and check that it runs in a Rex session named after the task. Resume a Claude Code or Codex session in the same folder, then archive the worktree task and check that its checkout and its Rex session go away.
- Set Start tasks in to Herdr Core and repeat: start a worktree task, resume a session and archive the task. Check that the agents appear in the Herdr Core group.
- Choose a runtime that is not running on the machine. Check that a red line under the location says so, and that Start explains why and starts nothing elsewhere. Set Start tasks in back to Automatic.
- Check that cards, rows and the session header show herdr's mark for herdr and Herdr Core agents and Rex's mark for Rex agents, and that a Herdr Core card or row reads "Core" beside the mark. Turn off Settings > Sessions > Runtime icons and check that the marks go away and a Herdr Core card or row reads "Herdr Core" again.
- Set Settings > Appearance > Glass to Clear, Regular and Opaque in turn, over Terminal and over Chat. Check that the session header's title and status stay readable under Clear, and that under Clear the Agents and Usage tabs scroll under the top bar with only the picker and its buttons on glass.
- After a Claude Code turn that leaves a suggestion grayed out in its input box, check that Terminal shows a ⇥ key before return and that Chat's empty prompt field shows the suggestion in gray. Tap ⇥ in each and check that it takes the suggestion.
- Run Copilot CLI in Herdr Core or Rex. Check that it shows its status and screen, and that the machine refuses a prompt or a key sent to it.
- Approve a Claude Code Bash command whose dialog shows a tip that wraps onto a second line. Check that the approval's context leaves the tip out.
- Open New task, show the keyboard and rotate the phone. Check that the prompt row stays below the navigation bar.
- Check that the app says Herdr Core, never Herdr 2.
- Connect to a machine still running collied 0.1.18. Check that the app connects without a version message, that no runtime marks and no Rex agents appear, and that New task with Start tasks in set to Rex says collied is too old and keeps Start dimmed.
- Check session visibility, question answers, approvals, notifications, Live Activities and reconnection with your paired machines.

Use Claude Code CLI, Codex CLI or GitHub Copilot CLI for these checks. Other herdr integrations and other Linux distributions have not been validated with Collie.
