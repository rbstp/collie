# Apple Watch

CollieWatch installs with the iPhone app. It holds no keys and never talks to `collied`: everything goes through the iPhone, which works while it is locked.

## In the watch app

- **Pending approvals**, with the command and a countdown.
- **Agents**, as the iPhone last saw them, like the Inbox.
- **Usage**, with each window's percentage and reset countdown.

The watch asks the iPhone for fresh approvals when it opens, at most once a minute unless an approval alert opened it.

## Deciding from the watch

Approvals are read-only on the watch until you turn on **Settings > Apple Watch > Decide from Apple Watch** on the iPhone, which needs Face ID or the passcode. Then:

- the watch must be unlocked and on your wrist;
- decisions are made in the watch app, never from the alert itself;
- there is no Approve always on the watch.

The iPhone checks the setting again and only sends an answer it showed the watch.

## Complication and widget

- The **circular complication** has three rings, outer to inner: Claude 5-hour, Claude weekly and Codex monthly. A ring's length is the percentage used, and its color moves from green through yellow, orange and red as usage grows. A white tick marks the elapsed share of each window.
- The **Plan usage widget** in the Smart Stack, also available as a rectangular complication, shows three gauges with the provider's logo and a reset countdown. A small orange dot marks readings at least 30 minutes old.

Tap either one to open a compact usage list. They refresh about every 15 minutes when watchOS allows it.
