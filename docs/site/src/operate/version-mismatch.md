# Version mismatch

The app and `collied` use the highest protocol revision both support. When they share none, the app stops connecting to that machine and says what to update:

| Message | Fix |
|---|---|
| "Update collied on \<machine>: install the latest release…" | [Update collied](updating.md#collied) on that machine |
| "Update the app." | [Update the app](updating.md#the-app) |
| "This app and collied on \<machine> use different versions. Update both." | [Update both](updating.md) |

Then pull to refresh or reopen the app; you do not need to pair again.

Current releases support revisions 26 to 29. Each new revision keeps at least the two previous ones; release notes say when support for one ends. From a release before revision 26, update both sides once. Until both sides support the revision a feature needs, the app leaves it out or asks you to update `collied`. See [Negotiated protocol compatibility](../reference/architecture.md#negotiated-protocol-compatibility).
