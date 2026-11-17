# iOS release notes

Text for 0.2.0. Confirm its final build in App Store Connect once the upload is processed.

## App Store release notes

- Watching a Rex agent no longer loads your Mac. While you have a Rex agent open on your phone, collied now follows its terminal as Rex streams it, instead of asking Rex to draw the whole terminal again up to four times a second. On a busy terminal with a long history, Rex now uses under 1% of a CPU core while you watch, down from about 30%. herdr and Herdr Core agents are unaffected.
- Use your Rex shells from the phone. With terminals turned on, each Rex session's shells appear under Terminals, and you can type commands and keys into them as into herdr's. A line you send reaches a Rex shell as a paste. A shell that keeps an agent in front, such as `sh -c 'claude'`, stays out of the list.
- On a Mac, you no longer get a done alert for a herdr agent you are looking at: its pane focused, its terminal in front, and you typing or moving the pointer in the last 30 seconds. The app then shows that agent idle rather than done. Herdr Core and Rex agents, and agents on Linux, alert as before.
- When Chat cannot show a Rex agent's conversation, it now says what to check in Rex rather than giving herdr's advice. Copilot CLI has Chat in herdr only.
- collied no longer passes its tailnet connections to the programs it starts, such as git and gh.
- Terminal now uses libghostty-vt 1.0.1, up from 1.0.0.

Update collied to 0.2.0 on each machine: every change above except the last needs it. Rex shells also need `[terminals] enabled = true` in `collied.toml` and a restart of collied. collied's service log now keeps warnings, and logs a warning that repeats once when its cause starts. You can update the app and collied in either order.

Collie Herd has been tested only with Claude Code CLI, Codex CLI and GitHub Copilot CLI; herdr's other agent integrations have not been validated. Rex support was tested with Rex 0.1.0-preview.20, a closed preview, on an Apple silicon Mac, with Claude Code and Codex. On Linux, only Arch Linux with Omarchy has been tested.

## What to Test

- On the Mac that runs Rex, with collied updated, open a Claude Code or Codex agent in Rex while it works, in Terminal and in Chat. Check that the screen keeps up with the agent as before: colors, wide characters such as emoji, wrapped lines, scrolling back, and rotating the phone.
- Keep a Claude Code agent in Rex open while it asks for an approval. Check that the approval card appears, approve it and check that the agent goes on, then deny the next one.
- Go back to the Agents tab while the agent works, wait a few seconds, then open it again. Check that its screen shows what it printed meanwhile, with no lines missing or repeated.
- In Activity Monitor on the Mac, find Rex Server (its process is `rex`). Open a working Rex agent with a long history, tens of thousands of lines, on the phone, and check that Rex Server's % CPU stays near what it shows with no agent open, rather than climbing toward 30% as it did with collied 0.1.22.
- Turn on terminals in `collied.toml` on the Mac that runs Rex and restart collied. Check that a Rex.app terminal appears under Terminals in its session's group and that a session started with `rex new demo -- top` does not. Open the terminal, unlock it, run `ls`, then `sleep 30`, and check that the terminal stays listed while `sleep` runs and that ⌃C from the key strip stops it. Start `claude` in it and check that it leaves Terminals and shows as an agent.
- On a Mac with herdr, give an agent a task of 40 seconds or more, keep its pane focused and its terminal in front, and move the mouse now and then. Check that no done alert arrives and that the app shows the agent idle. Repeat with another app in front, then with a Rex agent: each time the alert arrives. With another tab of the terminal showing, the alert arrives too if the terminal tells herdr when a tab loses focus.
- Run Copilot CLI in Rex and open its Chat. Check that it says "Chat is not available for this Rex agent."

Use Claude Code CLI, Codex CLI or GitHub Copilot CLI for these checks; other agents have not been validated with Collie.
