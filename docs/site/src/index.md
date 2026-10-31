# collie

collie lets you watch and steer your [herdr](https://github.com/herdrdev/herdr) coding agents from an iPhone and an Apple Watch. A small daemon, `collied`, runs on the Mac or Linux machine where herdr runs, and the Collie app reaches it over your own tailnet. Away from the keyboard, you can check every agent, read its screen, send it a prompt, answer an approval from the lock screen and start new work.

<div class="screenshots">
<img class="screenshot-light" src="images/agents-light.png" width="300" height="652" loading="lazy" alt="The Agents tab: a grid of six agents in three workspaces on one Mac, each card a preview of the agent's screen, one blocked on an approval">
<img class="screenshot-dark" src="images/agents-dark.png" width="300" height="652" loading="lazy" alt="The Agents tab: a grid of six agents in three workspaces on one Mac, each card a preview of the agent's screen, one blocked on an approval">
<img class="screenshot-light" src="images/chat-light.png" width="300" height="652" loading="lazy" alt="An agent's Chat view: the conversation, with the prompt field at the bottom">
<img class="screenshot-dark" src="images/chat-dark.png" width="300" height="652" loading="lazy" alt="An agent's Chat view: the conversation, with the prompt field at the bottom">
</div>

## What you can do

- See every herdr agent on every paired machine, with its status, live terminal preview and remaining context.
- Read an agent's conversation in Chat or its raw screen in Terminal, then reply by typing, dictating or sending keys.
- Approve or deny a blocked Claude Code agent from a notification, a Live Activity or the Apple Watch.
- Start Claude Code, Codex or Copilot CLI in a folder, a new folder, a Git worktree, a fresh clone or a pull request branch.
- Review an agent's Git changes and send line comments back to it.
- Follow plan usage for Claude Code, Codex and Copilot CLI on the phone, and for Claude Code and Codex on a watch complication.

Collie has been tested only with Claude Code CLI, Codex CLI and GitHub Copilot CLI. herdr supports more agents, but Collie has not been validated with them. On Linux, Collie has been tested only on Arch Linux with Omarchy. See [Requirements](install/requirements.md).

## How the phone reaches your machine

collie never opens a port on your network and does not use the Tailscale app.

1. `collied` runs its own Tailscale node, signed in with your personal Tailscale account. It listens on TCP port 8457, inside the tailnet only.
2. The Collie app runs a second Tailscale node, also signed in as you. It only dials out to the machine.
3. Before anything else, `collied` asks Tailscale who is connecting. It accepts only an untagged phone that you own and that is paired with this machine.
4. Inside the tunnel, the phone and the machine prove their keys to each other with mutual TLS. The phone pins the machine's key from the pairing QR code, and the phone's own key never leaves its Secure Enclave.

The only socket on your real network is Tailscale's own WireGuard UDP socket. The [Security overview](security/overview.md) gives the full picture.

## What stays on your machine

herdr, your agents, their transcripts and your files stay on the machine. The phone fetches conversation history over the encrypted session only while Chat is open, and none of it goes into logs or notifications. Push notifications go through Apple, so `collied` encrypts their content: only your phone can read the agent and workspace names, pull request details and pending commands. Apple sees generic alert text, timing and size. There is no collie server and no account to create.

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
2. Add the [tailnet policy](install/tailnet-policy.md).
3. Set up `collied` on [macOS](install/collied-macos.md) or [Linux](install/collied-linux.md).
4. [Install the app](install/app.md) and [pair your phone](install/pair.md).

The source, issues and releases are on [GitHub](https://github.com/rbstp/collie).
