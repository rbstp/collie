# Version mismatch

The app and `collied` speak a versioned protocol. They negotiate the highest revision both support. Revision 26 is the first baseline, currently the only supported revision. Older exact-match installations need one update on each side. When their supported ranges do not overlap, the app stops dialing that machine and says what to update, both in the Machines tab and in the machine's header on the Agents tab.

| Message | Meaning | Fix |
|---|---|---|
| "Update collied on Studio: install the latest release from github.com/rbstp/collie/releases." (with your machine's name) | The app is newer than `collied` | [Update collied](updating.md#collied) on that machine |
| "Update the app." | `collied` is newer than the app | [Update the app](updating.md#the-app) |
| "This app and collied on Studio use different versions. Update both." | The versions differ but `collied` did not say which it runs | Update [both](updating.md) |

No new pairing is needed: after the update, pull to refresh or reopen the app and it connects again.

New revisions will retain the current and previous two implemented negotiated revisions. Release notes announce supported-floor increases. Features also require a shared capability when their wire format differs; unsupported methods and fields are not sent.
