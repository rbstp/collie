# collie

collie lets you watch and steer your [herdr](https://github.com/herdrdev/herdr) coding agents from an iPhone and an Apple Watch. A small daemon, `collied`, runs on the Mac or Linux machine where herdr runs, and the Collie app reaches it over your own tailnet.

<div class="screenshots">
<img class="screenshot-light" src="images/agents-light.png" width="300" height="652" loading="lazy" alt="The Agents tab: a grid of six agents in three workspaces on one Mac, each card a preview of the agent's screen, one blocked on an approval">
<img class="screenshot-dark" src="images/agents-dark.png" width="300" height="652" loading="lazy" alt="The Agents tab: a grid of six agents in three workspaces on one Mac, each card a preview of the agent's screen, one blocked on an approval">
<img class="screenshot-light" src="images/chat-light.png" width="300" height="652" loading="lazy" alt="An agent's Chat view: the conversation, with the prompt field at the bottom">
<img class="screenshot-dark" src="images/chat-dark.png" width="300" height="652" loading="lazy" alt="An agent's Chat view: the conversation, with the prompt field at the bottom">
</div>

## What you can do

- See every herdr agent on every paired machine, with its status, a live preview and remaining context.
- Read an agent's conversation in Chat or its raw screen in Terminal, then reply by typing, dictating or sending keys.
- Approve or deny a blocked Claude Code agent from a notification, a Live Activity or the Apple Watch.
- Start Claude Code, Codex or Copilot CLI in a folder, a new folder, a Git worktree, a fresh clone or a pull request branch.
- Review an agent's Git changes and send line comments back to it.
- Follow plan usage for Claude Code, Codex and Copilot CLI on the phone and on a watch complication.

Collie has been tested only with Claude Code CLI, Codex CLI and GitHub Copilot CLI, and on Linux only on Arch Linux with Omarchy. See [Requirements](install/requirements.md).

## How the phone reaches your machine

collie opens no port on your network and does not use the Tailscale app. On your real network, only Tailscale's own WireGuard UDP socket listens.

1. `collied` runs its own Tailscale node, signed in with your personal Tailscale account, and listens on TCP port 8457 inside the tailnet only.
2. The Collie app runs a second node, also signed in as you, and only dials out to the machine.
3. `collied` first asks Tailscale who is connecting, and accepts only an untagged phone that you own and paired with this machine.
4. Inside the tunnel, the phone and the machine prove their keys to each other with mutual TLS. The phone pins the machine's key from the pairing QR code, and the phone's own key never leaves its Secure Enclave.

The [Security overview](security/overview.md) gives the full picture.

## What stays on your machine

herdr, your agents, their transcripts and your files stay on the machine. The phone fetches conversation history only while Chat is open or a grid card previews it, and `collied` never writes it to logs. Notifications reach Apple through the publisher's push relay, or directly for a self-built app, with their content encrypted: only your phone can read agent and workspace names, pull request details and pending commands. Apple sees generic alert text, timing and size. There is no collie account. The [privacy policy](privacy.md) has the details.

## The parts

| Part | Runs on | Role |
|---|---|---|
| `collied` | macOS on Apple silicon, Linux on x86_64 | Talks to herdr over its local socket and serves the phone over the tailnet |
| Collie | iPhone | The app, with a notification extension and Live Activities |
| CollieWatch | Apple Watch | Approvals, agents and a usage complication, all through the iPhone |
| CollieBar | macOS | Menu bar companion for `collied` |
| CollieTray | Linux with Omarchy | Bar widget with the same menu |

## Where to start

1. Check the [requirements](install/requirements.md).
2. Check the [tailnet policy](install/tailnet-policy.md).
3. Set up `collied` on [macOS](install/collied-macos.md) or [Linux](install/collied-linux.md).
4. [Install the app](install/app.md) and [pair your phone](install/pair.md).

The source, issues and releases are on [GitHub](https://github.com/rbstp/collie).
