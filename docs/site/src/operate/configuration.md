# Configuration

`collied` keeps its settings in `collied.toml`, in its data folder.

## Where things live

| | macOS | Linux |
|---|---|---|
| Data folder: `collied.toml`, paired phones, audit log | `~/Library/Application Support/collie/` | `$XDG_DATA_HOME/collie`, else `~/.local/share/collie` |
| Tailscale node state | `tsnet/` in the data folder | `tsnet/` in the data folder |
| Service | launchd agent `~/Library/LaunchAgents/dev.rbstp.collied.plist` | systemd user unit `~/.config/systemd/user/collied.service` |
| Logs | `collied.err.log` and `collied.out.log` in the data folder | `journalctl --user -u collied` |

`collied` refuses a node state folder that other users can access.

## collied.toml

Every section is optional. After a change, restart `collied` with `collied stop`, then `collied start`.

```toml
[tailnet]
hostname = "collie-studio"     # defaults to collie-<host name>
port = 8457                    # change the tailnet grant to match
owner_user_id = 123456789      # pins the owner; see below

[herdr]
session = "work"               # a named herdr session; herdr's default otherwise

[tasks]
roots = ["/Users/you/git"]     # where New task may work; your home folder by default
agents = ["claude", "codex", "copilot"]

[terminals]
enabled = false                # plain shells from the phone; see Terminals

[push.relay]                   # publisher app; no APNs key
```

- **`owner_user_id`** is the numeric Tailscale user ID of the only person who may use this `collied`. It must match the account `collied` is signed in with, or `collied` refuses to start. Without it, that account is still the owner, and no phone can choose another; setting it keeps the owner pinned even if the paired phones file is deleted. The pairing prompt and `collied peers list` show the ID.
- **`[tasks] agents`** limits which agents New task can start. The first one herdr knows is the default.
- **`[terminals]`**: see [Terminals](../install/terminals.md).
- **`[push.relay]` or `[apns]`**, never both: see [Push notifications](../install/push.md).
