# Usage

To see plan usage and token cost, switch the Agents tab to **Usage**, or swipe left. Each machine gets a card that says how long ago it recorded its plan usage.

| Agent | Shows | Source |
|---|---|---|
| Claude Code | 5-hour and weekly limits | The [status line tap](../install/claude-code.md#plan-usage) |
| Codex | Monthly credits and the reset date, for a credit-metered workspace | The local Codex app-server ([Codex](../agents/codex.md#plan-usage)) |
| Copilot | Monthly premium usage and the reset date, or "Unlimited" | GitHub, through the machine's `gh` login ([Copilot CLI](../agents/copilot.md#plan-usage)) |

<div class="screenshots">
<img class="screenshot-light" src="../images/usage-light.png" width="300" height="652" loading="lazy" alt="The Usage view: Claude Code's 5-hour and weekly usage limits, each with a bar and its reset time">
<img class="screenshot-dark" src="../images/usage-dark.png" width="300" height="652" loading="lazy" alt="The Usage view: Claude Code's 5-hour and weekly usage limits, each with a bar and its reset time">
</div>

Each block shows the amount used, the reset time and a bar with a tick for the time elapsed in the window. When usage passes the tick, you are using the plan faster than an even pace. A Copilot seat that allows overage can pass its allowance: the bar stays full and the amount shows what you used.

## Tokens and cost

Below the plan usage, each card shows the tokens the machine's agents used and what they would have cost at API prices, for **Today**, **7 days** or **30 days**.

- The total is the estimated cost, with what cache reads saved compared with the input price.
- Input, output, cache read and cache write give the tokens.
- For 7 and 30 days, a bar chart gives each day's cost.
- Each model gets a row with its agent, tokens and cost. A model without a known price shows "No price".

`collied` counts the tokens in the Claude Code, Codex and Copilot CLI transcripts on the machine, subagents included. The first time, it reads them in the background, and the card says "Scanning" while the totals grow; after that it reads only what the agents wrote since. Days end at midnight where your phone is now.

The cost is an estimate, not your bill: a subscription charges differently, and Copilot bills premium requests, so its cost is what the same tokens would cost from the API. The prices come from a snapshot of LiteLLM's price table that ships with `collied`, so updating `collied` updates them.

If the card asks you to update `collied`, the machine runs a release too old to count tokens: [update collied](../operate/updating.md#collied) there.

The plan readings also feed the [Apple Watch](../notifications/apple-watch.md) complication and widget.

<div class="screenshots">
<img src="../images/watch-widget.png" width="208" height="248" loading="lazy" alt="The Plan usage widget on the Apple Watch: three gauges, Claude with its 5-hour and weekly halves, Codex monthly and Copilot monthly, each with a reset countdown">
</div>
