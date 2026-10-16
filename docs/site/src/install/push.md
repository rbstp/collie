# Push notifications

Push notifications are optional. Without them, the app still shows everything while it is open. With them, you get approval alerts, done alerts, Live Activity updates and Apple Watch alerts.

## Publisher relay

`collied setup` offers the publisher relay when no push mode is configured. It writes this section and restarts the daemon, without importing an APNs key or creating a Keychain credential:

```toml
[push.relay]
```

Open the publisher app on the paired phone to register this machine's grant. The daemon sends only to `https://collie-push.rbstp.dev/v1/send`; the phone cannot choose another endpoint. App Attest enrollment and the publisher's deployment/real-device validation gates must pass before public relay delivery is available. Merely configuring the daemon does not establish enrollment or prove delivery.

Relay and direct APNs configuration are mutually exclusive. Remove `[push.relay]` before configuring `[apns]`, or remove `[apns]` before choosing the relay. Tailnet sessions, approvals, terminal/chat and desktop controls work independently of push availability.

Relay routine Live Activity attempts are coalesced to one per 60 seconds per followed agent and phone, including unchanged refreshes. Urgent updates, ends, alerts and approval clears bypass this cadence. All machines share the phone installation's relay quotas. Outages retain only bounded in-memory work, so restarting discards it. `collied doctor` reports registrations, earliest grant expiry, last actual send error, pending work and retry/quota-reset times. Remaining server budget is not reported.

## Direct APNs for a self-built app

Use `collied setup --direct-apns` for your own Apple team and bundle IDs. When `[apns]` is absent, setup asks for the `.p8` file and the IDs, imports the key and restarts `collied`. The `.p8` must already be readable only by you:

```sh
chmod 0600 AuthKey_<KEY_ID>.p8
collied setup --direct-apns
```

Each direct-mode machine uses its own APNs key, so you can revoke one key without touching the others. The relay does not accept self-built app credentials.

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

On macOS the key goes into the login Keychain, readable without a prompt only by the signed `collied`. Always reinstall with `just collied-install`, never `cargo install`, or the daemon cannot read the key. On Linux it becomes a systemd user credential (or a file readable only by you, as a fallback); `collied doctor` says which, and how it is sealed.

## Test a notification that opens a session

```sh
collied status
collied apns test --terminal-id <terminal-id>
```

Tapping that alert should open the agent's session. Test alerts carry no approval actions. More on testing in [Notifications](../reference/notifications.md), and the full Apple setup in [Release](../reference/release.md).

> [!TIP]
> `collied apns test` uses the selected relay or direct mode and reports that the send was accepted, not that the phone showed it. Check the phone too.
