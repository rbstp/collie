# Version mismatch

The app and `collied` speak a versioned protocol. When their versions do not match, the app stops dialing that machine and says which side to update, both in the Machines tab and in the machine's header on the Agents tab.

| Message | Meaning | Fix |
|---|---|---|
| "Update collied on Studio: run just collied-install there." (with your machine's name) | The app is newer than `collied` | [Update collied](updating.md#collied) on that machine |
| "Update the app." | `collied` is newer than the app | [Update the app](updating.md#the-app) |

No new pairing is needed: after the update, pull to refresh or reopen the app and it connects again.

Some features also need both sides to be recent even when the protocol matches, such as Chat, Git changes and typed answers to prompts. The app says so where it applies.
