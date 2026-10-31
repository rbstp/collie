# collie

collie lets you watch and steer your [herdr](https://github.com/herdrdev/herdr) coding agents from an iPhone and an Apple Watch. A small daemon, `collied`, runs on the Mac or Linux machine where herdr runs, and the Collie app reaches it over your own tailnet.

<div class="screenshots">
<img class="screenshot-light" src="images/agents-light.png" width="300" height="652" loading="lazy" alt="The Agents tab: six agents, one blocked on an approval">
<img class="screenshot-dark" src="images/agents-dark.png" width="300" height="652" loading="lazy" alt="The Agents tab: six agents, one blocked on an approval">
<img class="screenshot-light" src="images/chat-light.png" width="300" height="652" loading="lazy" alt="An agent's Chat view with its prompt field">
<img class="screenshot-dark" src="images/chat-dark.png" width="300" height="652" loading="lazy" alt="An agent's Chat view with its prompt field">
</div>

## What you can do

- See every agent on every paired machine, with its status, a live preview and remaining context.
- Read an agent's conversation or terminal screen, and reply by typing, dictating or sending keys.
- Approve or deny a blocked Claude Code agent from a notification, a Live Activity or the watch.
- Start Claude Code, Codex or Copilot CLI in a folder, Git worktree, fresh clone or pull request branch.
- Review an agent's Git changes and send it line comments.
- Follow plan usage on the phone and on a watch complication.

## Security and privacy

- `collied` and the app each run their own Tailscale node, signed in as you, so you need no Tailscale app.
- `collied` listens on TCP port 8457 inside the tailnet only, and the app only dials out. On your network, only Tailscale's WireGuard UDP socket listens.
- `collied` accepts only an untagged phone that you own and paired, and the two prove their keys to each other with mutual TLS.
- herdr, your agents, their transcripts and your files stay on the machine. There is no collie account.
- Notifications reach Apple through the publisher's push relay, or directly for a self-built app, encrypted so that only your phone can read their content. Apple and the relay see generic alerts and metadata such as timing and size.

The [Security overview](security/overview.md) and the [privacy policy](privacy.md) have the details.

## The parts

- **`collied`** runs on an Apple silicon Mac or an x86_64 Linux machine, talks to herdr and serves the phone over the tailnet.
- **Collie** is the iPhone app, with notifications and Live Activities.
- **CollieWatch** shows approvals, agents and a usage complication, all through the iPhone.
- **[CollieBar](desktop/colliebar.md)** on macOS and **[CollieTray](desktop/collietray.md)** on Omarchy are optional desktop companions.

## Where to start

1. Check the [requirements](install/requirements.md) and the [tailnet policy](install/tailnet-policy.md).
2. Set up `collied` on [macOS](install/collied-macos.md) or [Linux](install/collied-linux.md).
3. [Install the app](install/app.md) and [pair your phone](install/pair.md).

Source, issues and releases: [GitHub](https://github.com/rbstp/collie).
