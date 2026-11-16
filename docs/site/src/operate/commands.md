# Commands

Run these on the machine; [CollieBar](../desktop/colliebar.md) and [CollieTray](../desktop/collietray.md) offer the everyday ones from a menu.

| Command | What it does |
|---|---|
| `collied setup` | Signs in, installs the service, runs doctor, sets up relay push and pairs a phone, skipping finished steps |
| `collied setup --direct-apns` | The same, with your own APNs key for a self-built app |
| `collied status` | Shows the running daemon's state |
| `collied doctor` | Checks the setup; see below |
| `collied pair` | Shows a pairing QR code; `--show-uri` also prints the invite, one-time code included, for the Simulator |
| `collied peers list` | Lists the owner's user ID and paired phones |
| `collied peers revoke <label or StableID>` | Revokes a phone and closes its live sessions |
| `collied stop` / `start` | Turns `collied` off (it stays off across reboots) or on again |
| `collied login` | Signs Collie's node in to Tailscale; stop `collied` first |
| `collied service install` / `uninstall` | Installs and starts, or stops and removes, the service |
| `collied apns import <file>` | Imports a direct APNs `.p8` key |
| `collied apns test` | Sends a test alert to every phone registered for push; `--terminal-id <id>`, from `collied status`, makes it open that agent. Success means the send was accepted, not shown |

Commands that read `collied.toml` take `--config <path>` to use another file. `collied --version` prints the version.

## Reading `collied doctor`

Each line is `ok`, `warn` or `fail`, the check and a detail. [Troubleshooting](../troubleshooting.md) names the line to read for each symptom.

The `push` line shows the last send error and next retry; with the publisher relay, also the earliest grant expiry and when a reached daily quota resets. All machines paired with a phone share that quota: 1,000 sends per UTC day, at most 700 of them routine Live Activity updates. Times are Unix timestamps: grant expiry in seconds, the others in milliseconds. The line never prints a grant, push token or payload.

The `herdr-core` lines show the Herdr Core `collied` would follow, found from your environment as `hc` finds it. `not running` is fine when you do not use Herdr Core, and `herdr` stays `ok` while Herdr Core answers in its place. A warning names what keeps `collied` from following the core (an older version, another protocol generation, a missing method), or session variables the core's terminals inherit (`HERDR_PANE_ID`, `HERDR_SOCKET_PATH`, `CLAUDE_CODE_CHILD_SESSION`) because the core was started inside herdr or Claude Code; start it from a plain terminal instead. The socket's folder must be yours with mode `0700`, else the line fails. `collied status` prints a `herdr-core:` line with the version, socket and folder mode, or `not running`.

The `rex` lines show the Rex server `collied` would follow, found from your environment as the `rex` command finds it: on macOS, the one Rex.app runs. `not running` is fine when you do not use Rex, and `herdr` stays `ok` while Rex answers in its place. The first line names Rex's version and API revision, or warns what keeps `collied` from following the server: an API revision `collied` does not speak (a Rex server keeps the revision it started with, so restart it after you update Rex), a missing method, a listener that lets in callers who do not sign in, or a socket that is not yours alone (its folder or the socket open to other users, or a server another user runs). Once `collied` can follow the server, two more lines show the modes of the socket's folder and of the socket. A warning says when other machines reach the server through Rex's built-in Tailscale or a tailnet listener: they then reach the agents collie follows outside its allowlist and audit. `collied` still follows the server; turn those off in Rex unless you want other machines to reach the agents. A last warning names session variables the server's terminals inherit, as for Herdr Core. `collied status` prints a `rex:` line with the version and API revision, the socket and its folder's mode, why `collied` does not follow the server, and how other machines reach it, or `not running`.

Once Codex is set up, a second `codex hook` line checks `collied`'s own SessionStart hook in `hooks.json`, which [Codex in Herdr Core and Rex](../agents/codex.md#herdr-core-and-rex) needs; it warns only while Herdr Core or Rex runs.
