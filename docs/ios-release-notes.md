# iOS release notes

Text for 0.2.0. Confirm its final build in App Store Connect once the upload is processed.

## App Store release notes

- Watching a Rex agent no longer loads your Mac. While you have a Rex agent open on your phone, collied now follows its terminal as Rex streams it, instead of asking Rex to draw the whole terminal again up to four times a second. On a busy terminal with a long history, Rex now uses under 1% of a CPU core while you watch, down from about 30%.
- Agents in herdr and Herdr Core work as before.
- Terminal now uses libghostty-vt 1.0.1, up from 1.0.0.

Update collied to 0.2.0 on the Mac that runs Rex: the lighter watch comes from collied alone, and the app needs no setting. You can update the app and collied in either order.

Collie Herd has been tested only with Claude Code CLI, Codex CLI and GitHub Copilot CLI; herdr's other agent integrations have not been validated. Rex support was tested with Rex 0.1.0-preview.20, a closed preview, on an Apple silicon Mac, with Claude Code and Codex. On Linux, only Arch Linux with Omarchy has been tested.

## What to Test

- On the Mac that runs Rex, with collied updated, open a Claude Code or Codex agent in Rex while it works, in Terminal and in Chat. Check that the screen keeps up with the agent as before: colors, wide characters such as emoji, wrapped lines, scrolling back, and rotating the phone.
- Keep a Claude Code agent in Rex open while it asks for an approval. Check that the approval card appears, approve it and check that the agent goes on, then deny the next one.
- Go back to the Agents tab while the agent works, wait a few seconds, then open it again. Check that its screen shows what it printed meanwhile, with no lines missing or repeated.
- In Activity Monitor on the Mac, find Rex Server (its process is `rex`). Open a working Rex agent with a long history, tens of thousands of lines, on the phone, and check that Rex Server's % CPU stays near what it shows with no agent open, rather than climbing toward 30% as it did with collied 0.1.22.

Use Claude Code CLI or Codex CLI for these checks: in Rex, only they take prompts and keys from the phone.
