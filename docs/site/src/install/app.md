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

## Try the sample workspace

Tap **Explore sample workspace** on Welcome, or in Settings when already signed in. It works in the distributed app on iPhone and iOS Simulator, without a Tailscale login or a desktop.

The labeled sample contains synthetic agents, Chat and Terminal output, usage and an approval. Send a prompt, use the terminal keys or approve or deny the sample request to see a local result. These actions do not run commands or reach a real machine. New tasks, pairing, attachments, dictation, Live Activities and remote notifications are not part of the sample.

Use **Reset sample workspace** in its Settings tab to restore the initial data, or **Exit** to return to your real setup. Sample state is held in memory and does not change accounts, pairings, saved drafts or preferences.
