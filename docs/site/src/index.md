# collie

collie lets you watch and steer your [herdr](https://github.com/herdrdev/herdr) coding agents from an iPhone and an Apple Watch. A small daemon, `collied`, runs on the Mac or Linux machine where herdr runs. The Collie app reaches it over your own tailnet, so you can check every agent, read its screen, send it a prompt, answer an approval from the lock screen and start new work while you are away from the keyboard.

## What you can do

- See every herdr agent on every paired machine, with its status, live terminal preview and remaining context.
- Read an agent's conversation in Chat or its raw screen in Terminal, then reply by typing, dictating or sending keys.
- Approve or deny a blocked Claude Code agent from a notification, a Live Activity or the Apple Watch.
- Start Claude Code, Codex or Copilot CLI in a folder, a new folder, a Git worktree, a fresh clone or a pull request branch.
- Review an agent's Git changes and send line comments back to it.
- Follow plan usage for Claude Code and Codex on the phone and on a watch complication.

## How the phone reaches your machine

collie never opens a port on your network and does not use the Tailscale app.

1. `collied` embeds its own Tailscale node, tagged `tag:collie-mac` or `tag:collie-linux`. It listens on TCP port 8457 inside the tailnet only.
2. The Collie app embeds a second Tailscale node that belongs to you. It only dials out to the machine.
3. Every connection is checked with Tailscale's whois before anything else: the phone must be untagged, owned by you and paired with this machine.
4. Inside the tunnel, both sides prove their keys with mutual TLS. The machine's key is pinned from the pairing QR code; the phone's key lives in its Secure Enclave and never leaves it.

The only socket on the real network is Tailscale's own WireGuard UDP socket. See the [Security overview](security/overview.md) for the full picture.

## What stays on your machine

herdr, your agents, their transcripts and your files stay on the machine. The phone fetches conversation history over the encrypted session only while Chat is open, and none of it goes into logs or notifications. Push notifications go through Apple, so `collied` keeps them short: the agent and workspace names in clear, and the pending command encrypted end to end for your phone. There is no collie server and no account to create.

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
