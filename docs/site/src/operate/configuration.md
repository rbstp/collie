# Configuration

## Files

| | macOS | Linux |
|---|---|---|
| Data folder: `collied.toml`, paired phones, audit log, node state (`tsnet/`) | `~/Library/Application Support/collie/` | `$XDG_DATA_HOME/collie`, else `~/.local/share/collie` |
| Service | `~/Library/LaunchAgents/dev.rbstp.collied.plist` | `~/.config/systemd/user/collied.service` |
| Logs | `collied.err.log`, `collied.out.log` in the data folder | `journalctl --user -u collied` |

`collied` refuses a `tsnet/` folder that other users can access.

## collied.toml

Every section is optional. After a change, run `collied stop`, then `collied start`.

```toml
[tailnet]
hostname = "collie-studio"     # default: collie-<host name>
port = 8457                    # change the tailnet grant to match
owner_user_id = 123456789

[herdr]
session = "work"               # default: herdr's default

[tasks]                        # for New task
roots = ["/Users/you/git"]     # where it may work; default: your home folder
agents = ["claude", "codex", "copilot"]  # the first one herdr knows is the default
```

**`owner_user_id`** pins the owner, the only Tailscale user allowed to use `collied`, by the ID the pairing prompt and `collied peers list` show. `collied` refuses to start under another account, and the pin survives deleting the paired phones file. Without it, the signed-in account is the owner.

[Terminals](../install/terminals.md) covers `[terminals]`, and [Push notifications](../install/push.md) covers `[push.relay]` and `[apns]`.
