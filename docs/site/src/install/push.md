# Push notifications

Push notifications are optional. Without them, the app still shows everything while it is open. With them, you get approval alerts, done alerts, Live Activity updates and Apple Watch alerts.

Each machine sends its own pushes with its own APNs key, so you can revoke one machine's key without touching the others.

## With setup

When `[apns]` is not configured, `collied setup` asks for the `.p8` file and the team and bundle IDs, adds the `[apns]` section, imports the key and restarts `collied`. The `.p8` must already be readable only by you:

```sh
chmod 0600 AuthKey_<KEY_ID>.p8
collied setup
```

## By hand

1. Add an `[apns]` section to `collied.toml`:

   ```toml
   [apns]
   key_id = "<KEY_ID>"
   team_id = "<TEAM_ID>"
   bundle_id = "<your app's bundle ID>"
   ```

2. Import the key:

   ```sh
   collied apns import AuthKey_<KEY_ID>.p8
   ```

3. Check the configuration, then send a test alert:

   ```sh
   collied doctor
   collied apns test
   ```

On macOS the key goes into the login Keychain, readable without a prompt only by the signed `collied`. Always reinstall with `just collied-install`, never `cargo install`, or the daemon cannot read the key. On Linux it becomes a systemd user credential (or a file readable only by you, as a fallback); `collied doctor` says which, and how it is sealed.

## Test a notification that opens a session

```sh
collied status
collied apns test --terminal-id <terminal-id>
```

Tapping that alert should open the agent's session. Test alerts carry no approval actions. More on testing in [Notifications](../reference/notifications.md), and the full Apple setup in [Release](../reference/release.md).

> [!TIP]
> `collied apns test` reports that Apple accepted the push, not that the phone showed it. Check the phone too.
