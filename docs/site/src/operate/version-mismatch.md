# Version mismatch

The app and `collied` use the highest protocol revision both support; current releases support revisions 26 to 29. When they share none, the app stops connecting to that machine and says what to update, on the Machines tab and in the machine's header on the Agents tab.

| Message | Meaning | Fix |
|---|---|---|
| "Update collied on Studio: install the latest release from github.com/rbstp/collie/releases." (with your machine's name) | The app is newer than `collied` | [Update collied](updating.md#collied) on that machine |
| "Update the app." | `collied` is newer than the app | [Update the app](updating.md#the-app) |
| "This app and collied on Studio use different versions. Update both." | The versions differ, but `collied` did not say which one it runs | Update [both](updating.md) |

You do not need to pair again: after the update, pull to refresh or reopen the app, and it connects.

A release older than revision 26 needed an exact match, so coming from one, update both sides once.

When the protocol gets a new revision, both sides keep supporting at least the two previous ones, and release notes say when support for an older one ends. [Negotiated protocol compatibility](../reference/architecture.md#negotiated-protocol-compatibility) has the details. A feature that needs a newer revision works only when both sides support it; until then, the app leaves it out or asks you to update `collied`.
