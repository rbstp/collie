# Version mismatch

When the app and `collied` share no protocol revision, the app stops dialing that machine and says what to update, both in the Machines tab and in the machine's header on the Agents tab.

The two use the highest revision both support. Revision 26 was the first such revision; current releases support revisions 26 to 29. A release older than revision 26 needed an exact match, so coming from one, update each side once.

| Message | Meaning | Fix |
|---|---|---|
| "Update collied on Studio: install the latest release from github.com/rbstp/collie/releases." (with your machine's name) | The app is newer than `collied` | [Update collied](updating.md#collied) on that machine |
| "Update the app." | `collied` is newer than the app | [Update the app](updating.md#the-app) |
| "This app and collied on Studio use different versions. Update both." | The versions differ but `collied` did not say which it runs | Update [both](updating.md) |

You do not need to pair again: after the update, pull to refresh or reopen the app, and it connects again.

When the protocol gets a new revision, the app and `collied` will keep supporting up to two earlier negotiated revisions, starting with revision 26, and release notes will say when support for an older one ends. [Negotiated protocol compatibility](../reference/architecture.md#negotiated-protocol-compatibility) has the details. A feature that changes what the two sides exchange works only when both support it; until then, the app and `collied` simply do not use it.
