# Commands

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
| `collied apns test` | Sends an encrypted test through the selected relay or direct mode; acceptance does not confirm display |

## Reading `collied doctor`

Each line is `ok`, `warn` or `fail` with a check name and a detail. Start every investigation here: [Troubleshooting](../troubleshooting.md) is organized by symptom and names the doctor line to look at.

The push detail reports the active mode, registration counts, earliest relay grant expiry, last actual send error and bounded in-memory retry state. Relay daily quotas are shared across the installation's machines. Remaining budget is unknown locally; a quota response supplies the reset time. Times are Unix seconds for grant expiry and Unix milliseconds for attempts/retries. No grant, destination token or payload is printed.
