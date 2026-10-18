# Configuration

`collied` keeps its settings in `collied.toml`, next to its node state, paired phones and audit log.

## Where things live

| | macOS | Linux |
|---|---|---|
| `collied.toml`, paired phones, audit log | `~/Library/Application Support/collie/` | `$XDG_DATA_HOME/collie`, else `~/.local/share/collie` |
| Tailscale node state | `tsnet/` in that folder | `tsnet/` in that folder |
| Service | launchd agent | systemd user unit `~/.config/systemd/user/collied.service` |
| Logs | | `journalctl --user -u collied` |

`collied` refuses a node state folder that other users can read.

## collied.toml

Every section is optional. After a change, restart `collied` with `collied stop`, then `collied start`.

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

- **`owner_user_id`** is the numeric Tailscale user ID of the only person allowed to use this `collied`. It must match the account `collied` is signed in with, or `collied` refuses to start. Without it, `collied` still takes its signed-in account as the owner before the first pairing, and a phone cannot choose another. Read the ID from the pairing prompt or `collied peers list`. Setting it here keeps the owner pinned even if the paired phones file is deleted.
- **`[tasks] agents`** limits which agents New task can start. The first one is the default.
- **`[terminals]`**: see [Terminals](../install/terminals.md).
- **`[push.relay]` or `[apns]`**, never both: see [Push notifications](../install/push.md).
