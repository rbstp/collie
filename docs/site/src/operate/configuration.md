# Configuration

This page describes the next matching phone and desktop candidate; user-owned setup is unreleased. Published desktop v0.1.0 still uses tagged nodes. Follow the [v0.1.0 tailnet procedure](https://github.com/rbstp/collie/blob/v0.1.0/docs/tailnet.md) until both peers are updated.

## Where things live

| | macOS | Linux |
|---|---|---|
| `collied.toml`, paired phones, audit log | `~/Library/Application Support/collie/` | `$XDG_DATA_HOME/collie`, else `~/.local/share/collie` |
| Tailscale node state | `tsnet/` in that folder | `tsnet/` in that folder |
| Service | launchd agent | systemd user unit `~/.config/systemd/user/collied.service` |
| Logs | | `journalctl --user -u collied` |

`collied` refuses a node state folder that other users can read.

## collied.toml

Every section is optional. Restart `collied` (`collied stop`, then `collied start`) after a change.

```toml
[tailnet]
hostname = "collie-studio"     # defaults to collie-<host name>
port = 8457                    # change the tailnet grant to match
owner_user_id = 123456789      # pins the owner; see below

[tasks]
roots = ["/Users/you/git"]     # where New task may work; your home folder by default
agents = ["claude", "codex", "copilot"]

[terminals]
enabled = false                # plain shells from the phone; see Terminals

[push.relay]                  # publisher app; no APNs key
```

- **`owner_user_id`** is the numeric Tailscale user ID of the only person allowed to use this `collied`. It must match collied's actual positive embedded Tailscale user. Without it, that verified self user still binds the gate before first pairing; a phone cannot choose the owner. Read the ID from the pairing prompt or `collied peers list`; copying it here keeps the owner pinned even if the paired phones file is deleted.
- **`[tasks] agents`** limits which agents New task can start, the first being the default.
- **`[terminals]`**: see [Terminals](../install/terminals.md).
- **`[push.relay]` or `[apns]`** (mutually exclusive): see [Push notifications](../install/push.md).
