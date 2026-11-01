# Apple Watch

CollieWatch installs with the iPhone app, holds no keys and never talks to `collied`: everything goes through the iPhone, even while it is locked (after its first unlock).

## In the watch app

- **Plan usage**: each window's percentage and reset countdown.
- **Approvals**: pending approvals with countdowns; open one for its command.
- **Working** and **Done**: your agents as the iPhone last saw them.

## Deciding from the watch

Approvals are read-only until you turn on **Settings > Apple Watch > Decide from Apple Watch** on the iPhone, which asks for Face ID or the passcode. Then:

- the watch must be unlocked and on your wrist;
- you decide in the watch app, never from the alert;
- you can approve, deny or pick a menu option, never Approve always.

The iPhone rechecks the setting for each decision and sends only an answer it showed the watch.

## Complication and widget

- **Circular complication**: four rings, outer to inner, for Claude 5-hour, Claude weekly, Codex monthly and Copilot monthly premium usage. A white tick marks elapsed time in each window.
- **Plan usage widget**, in the Smart Stack or as a rectangular complication: a gauge with a reset countdown for each provider, Claude's split into 5-hour (left) and weekly (right). An orange dot marks a reading at least 30 minutes old.

Tap either for a usage list. Both refresh about every 15 minutes, as watchOS allows.

<div class="screenshots">
<img src="../images/watch-widget.png" width="208" height="248" loading="lazy" alt="The Plan usage widget: Claude's 5-hour and weekly halves, Codex monthly and Copilot monthly, each with a reset countdown">
<img src="../images/watch-usage.png" width="208" height="248" loading="lazy" alt="The usage list: Claude 5-hour at 38%, Claude weekly at 61% and Codex monthly at 24%, each with the time left">
</div>

### Choosing the agents

Each widget and complication shows all three agents until you turn some off in its options: edit the watch face or Smart Stack on the watch, or the watch face in the iPhone's Watch app.
