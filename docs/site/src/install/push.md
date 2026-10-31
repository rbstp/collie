# Push notifications

Push notifications bring approval and done alerts, Live Activity updates and Apple Watch alerts. Without them, the app shows everything only while it is open.

The publisher relay serves only the publisher's app. For an app you build yourself, use [direct APNs](#direct-apns-for-a-self-built-app).

## Publisher relay

When no push mode is configured, `collied setup` offers the relay, which needs no APNs key. Setup adds this section to `collied.toml` and restarts `collied`:

```toml
[push.relay]
```

Then open the app on the paired phone to register this machine. `collied doctor` lists the registration.

`collied` sends only to `https://collie-push.rbstp.dev/v1/send`. All your machines share one daily relay quota per phone.

## Direct APNs for a self-built app

Make your `.p8` key readable only by you, then run setup with `--direct-apns`. If no push mode is configured, setup asks for the key file and your IDs, imports the key and restarts `collied`:

```sh
chmod 0600 AuthKey_<KEY_ID>.p8
collied setup --direct-apns
```

A machine uses the relay or direct APNs, never both: remove one section before adding the other. Give each machine its own APNs key, so you can revoke one alone.

## Direct configuration by hand

1. Add an `[apns]` section to `collied.toml`:

   ```toml
   [apns]
   key_id = "<KEY_ID>"
   team_id = "<TEAM_ID>"
   bundle_id = "<your app's bundle ID>"
   ```

2. Import the key. The command then offers to delete the `.p8`:

   ```sh
   collied apns import AuthKey_<KEY_ID>.p8
   ```

3. Restart `collied` (`collied stop`, then `collied start`): it reads push settings only at start.

4. Run `collied doctor`, then [send a test](#test-a-notification-that-opens-a-session).

On macOS, the key goes into the login Keychain, which only the signed `collied` reads without a prompt, so reinstall with `just collied-install`, never `cargo install`. On Linux, it becomes a systemd user credential or a private file: see [collied on Linux](collied-linux.md).

## Test a notification that opens a session

With either push mode, use an agent's terminal ID from `collied status`:

```sh
collied apns test --terminal-id <terminal-id>
```

Tapping the alert should open that agent's session; it carries no approval actions. The command reports only that the send was accepted: check the phone. See [Notifications](../reference/notifications.md) for more tests and [Release](../reference/release.md) for the Apple setup.
