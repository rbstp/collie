# Commands

You control `collied` from a terminal on the machine. [CollieBar](../desktop/colliebar.md) and [CollieTray](../desktop/collietray.md) offer the everyday ones from a menu.

| Command | What it does |
|---|---|
| `collied setup` | Interactive setup: login, service, doctor, publisher relay push and pairing. Safe to run again |
| `collied setup --direct-apns` | Set up push with your own APNs key for a self-built app |
| `collied status` | The running daemon's state: its node and tags, sessions, paired phones, herdr version and the agents it sees |
| `collied doctor` | Checks the config, service, herdr, the tailnet node, its tag and reach, push, hooks, plan usage and `gh` |
| `collied pair` | Opens a pairing window and shows the QR code |
| `collied peers list` | Lists paired phones |
| `collied peers revoke <label or StableID>` | Revokes a phone and closes its live sessions |
| `collied stop` | Turns `collied` off and keeps it off, across reboots, until `collied start` |
| `collied start` | Starts it again |
| `collied login` | Signs the machine's node in to Tailscale |
| `collied service install` | Installs the launchd agent (macOS) or systemd user unit (Linux) |
| `collied service uninstall` | Stops and removes the launchd agent or systemd user unit |
| `collied apns import <file>` | Imports the APNs key |
| `collied apns test` | Sends an encrypted test alert through the configured relay or direct mode. It reports that the send was accepted, not that the phone showed it |

## Reading `collied doctor`

When something goes wrong, start with `collied doctor`. Each line is `ok`, `warn` or `fail`, with a check name and a detail. [Troubleshooting](../troubleshooting.md) is organized by symptom and names the line to look at.

The push line reports the active mode, registration counts, the earliest relay grant expiry, the last send error and pending retries. Grant expiry is in Unix seconds, and attempt and retry times are in Unix milliseconds. The line never prints a grant, a destination token or a payload. All machines paired with a phone share its daily relay quotas. `collied` cannot know how much quota is left, but when the relay reports a quota reached, the push line shows when it resets.
