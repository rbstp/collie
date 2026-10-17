# iOS release notes

Prepared text for the next combined release. Confirm the final version and build in App Store Connect after the upload is processed.

## App Store release notes

- Reorder machines in the Machines tab; the Agents tab follows the same saved order.
- See the running daemon version in the macOS menu bar and Linux Omarchy bar companions.
- Restore GitHub pull request features when `gh` is installed in `~/.local/bin`, including Omarchy setups.

Collie has been tested only with Claude Code CLI, Codex CLI and GitHub Copilot CLI. herdr supports more agent integrations, but those have not been validated with Collie. Linux testing is limited to Arch Linux with Omarchy; other distributions have not been validated.

## What to Test

- Reorder two paired machines in Machines, check the same order in Agents, then restart Collie and confirm the order remains.
- Check that the macOS menu bar and Linux Omarchy bar companions show the running daemon version, clear it when stopped and refresh it after the daemon restarts. Replacing the executable alone does not change the running process.
- On Arch Linux with Omarchy and `gh` in `~/.local/bin`, check `collied doctor` and pull request status in Collie.
- Check session visibility, question answers, approvals, notifications, Live Activities and reconnection with your paired machines.

Use Claude Code CLI, Codex CLI or GitHub Copilot CLI for these checks. Other herdr integrations and other Linux distributions have not been validated with Collie.
