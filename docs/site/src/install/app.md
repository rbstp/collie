# Install the app

Collie is not on the App Store yet. You build it and install it on your own iPhone.

1. Turn on Developer Mode on the iPhone and connect it to the Mac.
2. Install the app and its watch app:

   ```sh
   just ios-run-device
   ```

3. Open Collie and sign in to Tailscale inside the app with the personal account you used for `collied`.

> [!NOTE]
> The watch app installs with the iPhone app. To sign it, register the paired Apple Watch with your team once: connect it in Xcode's Devices and Simulators window, or add its UDID in the developer portal.

The app runs its own Tailscale node, `collie-phone`. It does not need the Tailscale app, and a Tailscale app already on the phone plays no part in collie.

## Next

[Pair your phone](pair.md) with each machine.

## Try the sample workspace

You can try Collie without a Tailscale login or a desktop. Tap **Explore sample workspace** on Welcome, or in Settings when you are already signed in. The sample works in the distributed app on iPhone and in the iOS Simulator.

A bar at the bottom of the screen labels the sample. It holds made-up agents, Chat and Terminal output, usage and an approval. Send a prompt, use the terminal keys, or approve or deny the sample request to see what happens. Nothing runs and nothing reaches a real machine. The sample does not include new tasks, pairing, attachments, dictation, Live Activities or remote notifications.

To restore the initial data, use **Reset sample workspace** in the sample's Settings tab. To return to your real setup, tap **Exit** in that bar. The sample lives only in memory and leaves your accounts, pairings, saved drafts and preferences unchanged.
