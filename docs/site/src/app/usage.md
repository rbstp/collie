# Usage

To see plan usage and token cost, switch the Agents tab to **Usage**, or swipe left. Each machine gets a card.

| Agent | Plan usage | Source |
|---|---|---|
| Claude Code | 5-hour and weekly limits | The [status line tap](../install/claude-code.md#plan-usage) |
| Codex | Monthly credits and reset date, for a credit-metered workspace | The local [Codex](../agents/codex.md#plan-usage) app-server |
| Copilot | Monthly premium usage and reset date, or "Unlimited" | GitHub, with the machine's `gh` login ([Copilot CLI](../agents/copilot.md#plan-usage)) |

<div class="screenshots">
<img class="screenshot-light" src="../images/usage-light.png" width="300" height="652" loading="lazy" alt="The Usage view: Claude Code's 5-hour and weekly usage limits, each with a bar and its reset time">
<img class="screenshot-dark" src="../images/usage-dark.png" width="300" height="652" loading="lazy" alt="The Usage view: Claude Code's 5-hour and weekly usage limits, each with a bar and its reset time">
</div>

A tick on each bar marks the time elapsed in its window: usage past the tick means you are using the plan faster than an even pace. The [Apple Watch](../notifications/apple-watch.md) complication and widget show the same readings.

<div class="screenshots">
<img src="../images/watch-widget.png" width="208" height="248" loading="lazy" alt="The Plan usage widget on the Apple Watch: three gauges, Claude with its 5-hour and weekly halves, Codex monthly and Copilot monthly, each with a reset countdown">
</div>

## Tokens and cost

Each card also shows the tokens the machine's agents used **Today** or over **7 days** or **30 days**, and their cost at API prices, by token type and by model.

`collied` counts tokens in the machine's Claude Code, Codex and Copilot CLI transcripts, subagents included. Days end at midnight where your phone is.

The cost is an estimate, not your bill: a subscription charges differently, and Copilot bills premium requests. Prices come from LiteLLM's price table, as shipped with `collied`. [Update collied](../operate/updating.md#collied) for newer prices, or when the card asks because the machine's release is too old to count tokens.
