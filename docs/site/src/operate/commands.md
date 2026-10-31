# Commands

You control `collied` from a terminal on the machine. [CollieBar](../desktop/colliebar.md) and [CollieTray](../desktop/collietray.md) offer the everyday commands from a menu.

| Command | What it does |
|---|---|
| `collied setup` | Runs login, service, doctor, push through the publisher relay, and pairing; it skips each step already done, so you can run it again |
| `collied setup --direct-apns` | The same, with your own APNs key for a self-built app |
| `collied status` | Shows the running daemon's node, port, tags, sessions, paired phones, herdr version and agents |
| `collied doctor` | Checks the config, permissions, daemon, service, herdr, the tailnet node with its tags and reach, push, hooks, plan usage, `gh` and Copilot |
| `collied pair` | Opens a pairing window and shows its QR code; `--show-uri` also prints the invite, one-time code included, for the Simulator |
| `collied peers list` | Lists the owner's user ID and the paired phones |
| `collied peers revoke <label or StableID>` | Revokes a phone and closes its live sessions |
| `collied stop` | Turns `collied` off and keeps it off, across reboots, until `collied start` |
| `collied start` | Starts it again |
| `collied login` | Signs Collie's own node in to Tailscale; stop `collied` first |
| `collied service install` | Installs and starts the launchd agent (macOS) or systemd user unit (Linux) |
| `collied service uninstall` | Stops and removes the service |
| `collied apns import <file>` | Stores the APNs `.p8` key in the login Keychain (macOS) or as a systemd credential (Linux), then offers to delete the file |
| `collied apns test` | Sends an encrypted test alert to every phone registered for push, in the configured mode. It reports that the send was accepted, not that the phone showed it |
| `collied apns test --terminal-id <id>` | Sends one that opens that agent's session; `collied status` lists the IDs |

Commands that read `collied.toml` take `--config <path>` to use another file, and `collied --version` prints the version. The service runs `collied run`, and Claude Code runs `collied hook` and `collied statusline` ([Claude Code hook and plan usage](../install/claude-code.md)).

## Reading `collied doctor`

Each line is `ok`, `warn` or `fail`, then the check and a detail. [Troubleshooting](../troubleshooting.md) names the line to read for each symptom.

The `push` line shows the mode, the registered devices and Live Activities, queued sends, the last attempt, the last send error and the next retry. With the publisher relay, it adds the earliest grant expiry and the daily budget that all machines paired with the phone share: 1,000 sends per UTC day, at most 700 of them routine Live Activity updates. `collied` cannot see how much is left, but when the relay reports the quota reached, the line shows when it resets. Grant expiry is in Unix seconds, other times in Unix milliseconds. The line never prints a grant, a push token or a payload.
