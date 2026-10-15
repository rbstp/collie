# Install the app

Collie is not on the App Store yet. You build it and install it on your own iPhone.

1. Turn on Developer Mode on the iPhone and connect it to the Mac.
2. Install the app and its watch app:

   ```sh
   just ios-run-device
   ```

3. Open Collie and sign in to Tailscale inside the app with your own account. This is the account that owns `collied`, not a tagged key.

> [!NOTE]
> The watch app installs with the iPhone app. To sign it, register the paired Apple Watch with your team once: connect it in Xcode's Devices and Simulators window, or add its UDID in the developer portal.

The app runs its own Tailscale node, `collie-phone`. It does not need the Tailscale app, and a Tailscale app already on the phone plays no part in collie.

## Next

[Pair your phone](pair.md) with each machine.
