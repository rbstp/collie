# Usage

To see plan usage, switch the Agents tab to **Usage**, or swipe left. Each machine that has recorded a plan gets a card, which says how long ago it recorded it.

| Agent | Shows | Source |
|---|---|---|
| Claude Code | 5-hour and weekly limits | The [status line tap](../install/claude-code.md#plan-usage) |
| Codex | Monthly used and allowed credits, and the reset date | The local Codex app-server, every five minutes, for a credit-metered workspace |

<div class="screenshots">
<img class="screenshot-light" src="../images/usage-light.png" width="300" height="652" loading="lazy" alt="The Usage view: Claude Code's 5-hour and weekly usage limits, each with a bar and its reset time">
<img class="screenshot-dark" src="../images/usage-dark.png" width="300" height="652" loading="lazy" alt="The Usage view: Claude Code's 5-hour and weekly usage limits, each with a bar and its reset time">
</div>

Each block shows the amount used, the reset time and a bar with a tick for the time elapsed in the window. When usage passes the tick, you are using the plan faster than an even pace.

The same readings feed the [Apple Watch](../notifications/apple-watch.md) complication and widget.

<div class="screenshots">
<img src="../images/watch-widget.png" width="208" height="248" loading="lazy" alt="The Plan usage widget on the Apple Watch: three gauges, Claude 5-hour, Claude weekly and Codex monthly, each with a reset countdown">
</div>
