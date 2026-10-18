# Apple Watch

CollieWatch shows your pending approvals, agents and plan usage on your wrist. It installs with the iPhone app, holds no keys and never talks to `collied`: everything goes through the iPhone, which works while it is locked.

## In the watch app

- **Pending approvals**, with the command and a countdown.
- **Agents**, as the iPhone last saw them, like the Inbox.
- **Usage**, with each window's percentage and reset countdown.

When the watch app opens, it asks the iPhone for fresh approvals, at most once a minute unless an approval alert opened it.

## Deciding from the watch

The watch shows approvals read-only until you turn on **Settings > Apple Watch > Decide from Apple Watch** on the iPhone, which needs Face ID or the passcode. Then:

- the watch must be unlocked and on your wrist;
- you decide in the watch app, never from the alert itself;
- the watch offers no Approve always.

The iPhone checks the setting again, and sends only an answer it showed the watch.

## Complication and widget

- The **circular complication** has three rings, outer to inner: Claude 5-hour, Claude weekly and Codex monthly. A ring's length shows the percentage used, and its color moves from green through yellow, orange and red as usage grows. A white tick marks how much of each window has passed.
- The **Plan usage widget** in the Smart Stack, also available as a rectangular complication, shows three gauges, each with the provider's logo and a reset countdown. A small orange dot marks readings at least 30 minutes old.

Tap either one to open a compact usage list. Both refresh about every 15 minutes, when watchOS allows it.

<div class="screenshots">
<img src="../images/watch-widget.png" width="208" height="248" loading="lazy" alt="The Plan usage widget: three gauges, Claude 5-hour, Claude weekly and Codex monthly, each with a reset countdown">
<img src="../images/watch-usage.png" width="208" height="248" loading="lazy" alt="The watch usage list: Claude 5-hour at 38%, Claude weekly at 61% and Codex monthly at 24%, each with the time left">
</div>
