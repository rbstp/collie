# Push notifications

Push notifications are optional: they bring approval and done alerts, Live Activity updates and Apple Watch alerts. Without them, the app still shows everything while it is open. The publisher relay serves only the publisher's app: for an app you build yourself, use [direct APNs](#direct-apns-for-a-self-built-app).

## Publisher relay

When no push mode is configured, `collied setup` offers the publisher relay. It adds this section to `collied.toml` and restarts `collied`, with no APNs key to import:

```toml
[push.relay]
```

Then open the publisher's app on the paired phone: it registers this machine with the relay, and notifications start. `collied doctor` then reports the registration. `collied` sends only to `https://collie-push.rbstp.dev/v1/send`, and the phone cannot choose another endpoint.

Through the relay:

- A followed agent's Live Activity updates at most once a minute on each phone. This limit never holds back urgent updates, ends, alerts or cleared approvals.
- All your machines share one daily relay quota for each phone.
- During an outage, `collied` keeps pending notifications in memory and retries them for a limited time. A restart drops them.

`collied doctor` reports registrations, the earliest grant expiry, the last send error, pending notifications, and when `collied` will retry or the quota resets. It cannot report how much quota is left.

## Direct APNs for a self-built app

The relay does not accept a self-built app. If you build the app with your own Apple team and bundle IDs, run `collied setup --direct-apns`. When no push mode is configured, setup asks for the `.p8` file and the IDs, imports the key and restarts `collied`. The `.p8` must be readable only by you:

```sh
chmod 0600 AuthKey_<KEY_ID>.p8
collied setup --direct-apns
```

You can use the relay or direct APNs, not both: remove one section before you add the other. Give each machine its own APNs key, so you can revoke one without touching the others.

## Direct configuration by hand

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

3. Restart `collied` with `collied stop`, then `collied start`: it reads push settings only when it starts.

4. Check the configuration, then send a test alert:

   ```sh
   collied doctor
   collied apns test
   ```

`collied apns import` stores the key, then offers to delete the `.p8`. On macOS, the key goes into the login Keychain, where only the signed `collied` can read it without a prompt, so always reinstall with `just collied-install`, never `cargo install`. On Linux, it becomes a systemd user credential, or, where that is not possible, a file readable only by you; `collied doctor` says which, and how it is sealed.

## Test a notification that opens a session

Take an agent's terminal ID from `collied status`, then send:

```sh
collied apns test --terminal-id <terminal-id>
```

Tapping that alert should open the agent's session. Test alerts carry no approval actions. [Notifications](../reference/notifications.md) has more on testing, and [Release](../reference/release.md) has the full Apple setup.

> [!TIP]
> `collied apns test` uses the configured mode. It reports that the send was accepted, not that the phone showed it, so check the phone too.
