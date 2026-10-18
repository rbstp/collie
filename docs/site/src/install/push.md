# Push notifications

Push notifications are optional. With them, you get approval alerts, done alerts, Live Activity updates and Apple Watch alerts. Without them, the app still shows everything while it is open.

## Publisher relay

When no push mode is configured, `collied setup` offers the publisher relay. It writes this section and restarts `collied`; you import no APNs key and create no Keychain credential:

```toml
[push.relay]
```

Then open the publisher app on the paired phone, which registers this machine with the relay. `collied` sends only to `https://collie-push.rbstp.dev/v1/send`, and the phone cannot choose another endpoint. The relay is not yet open to the public, so configuring `collied` does not guarantee that notifications reach your phone.

You can use either the relay or direct APNs, not both. Remove `[push.relay]` before you configure `[apns]`, or remove `[apns]` before you choose the relay. Sessions, approvals, Chat, terminals and desktop controls keep working whether or not push works.

Through the relay, a followed agent's Live Activity updates at most once a minute on each phone. This limit does not hold back urgent updates, ends, alerts or cleared approvals. All your machines share one relay quota for the phone. During an outage, `collied` keeps pending notifications in memory and retries them for a limited time, but a restart drops them. `collied doctor` reports registrations, the earliest grant expiry, the last send error, pending notifications and when it will retry or the quota resets. It cannot report how much quota is left.

## Direct APNs for a self-built app

If you build the app with your own Apple team and bundle IDs, use `collied setup --direct-apns`. When `[apns]` is absent, setup asks for the `.p8` file and the IDs, imports the key and restarts `collied`. The `.p8` must already be readable only by you:

```sh
chmod 0600 AuthKey_<KEY_ID>.p8
collied setup --direct-apns
```

Give each machine its own APNs key, so you can revoke one key without touching the others. The relay does not accept a self-built app.

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

3. Check the configuration, then send a test alert:

   ```sh
   collied doctor
   collied apns test
   ```

On macOS, `collied apns import` puts the key in the login Keychain, where only the signed `collied` can read it without a prompt. Always reinstall with `just collied-install`, never `cargo install`, or `collied` cannot read the key. On Linux, the key becomes a systemd user credential, or a file readable only by you when that is not possible. `collied doctor` says which, and how it is sealed.

## Test a notification that opens a session

```sh
collied status
collied apns test --terminal-id <terminal-id>
```

Tapping that alert should open the agent's session. Test alerts carry no approval actions. [Notifications](../reference/notifications.md) has more on testing, and [Release](../reference/release.md) has the full Apple setup.

> [!TIP]
> `collied apns test` uses the configured relay or direct mode. It reports that the send was accepted, not that the phone showed it, so check the phone too.
