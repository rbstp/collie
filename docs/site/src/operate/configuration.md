# Configuration

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

[apns]
key_id = "<KEY_ID>"
team_id = "<TEAM_ID>"
bundle_id = "<bundle ID>"
```

- **`owner_user_id`** is the numeric Tailscale user ID of the only person allowed to use this `collied`. Without it, the first phone you confirm at pairing sets the owner. Read the ID from the pairing prompt or `collied peers list`; copying it here keeps the owner pinned even if the paired phones file is deleted.
- **`[tasks] agents`** limits which agents New task can start, the first being the default.
- **`[terminals]`**: see [Terminals](../install/terminals.md).
- **`[apns]`**: see [Push notifications](../install/push.md).
