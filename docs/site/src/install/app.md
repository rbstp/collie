# Install the app

Collie is not on the App Store yet. You build it on your Mac and install it on your own iPhone.

1. Turn on Developer Mode on the iPhone and connect it to the Mac.
2. With [your own identifiers](requirements.md#your-own-identifiers) set, install the app and its watch app:

   ```sh
   just ios-run-device
   ```

3. Open Collie, tap **Sign in to Tailscale** and use the personal account you used for `collied`.

> [!NOTE]
> To sign the watch app, register the paired Apple Watch with your team once: connect it in Xcode's Devices and Simulators window, or add its UDID in the developer portal.

The app runs its own Tailscale node, `collie-phone`. It does not need the Tailscale app, and a Tailscale app already on the phone plays no part in collie.

## Try the sample workspace

You can try Collie without a Tailscale login or a machine. Tap **Explore sample workspace** on Welcome, or at the bottom of Settings once you are signed in. The sample works on iPhone and in the iOS Simulator.

The sample holds made-up agents, Chat and Terminal output, usage and an approval, and a bar at the bottom of the screen labels it. Send a prompt, use the terminal keys, or approve or deny the sample request: nothing runs and nothing reaches a real machine. The sample does not include new tasks, pairing, attachments, dictation, Live Activities or remote notifications.

**Reset sample workspace**, in the sample's Settings tab, restores its initial data, and **Exit**, in the bar, returns to your real setup. The sample lives only in memory and leaves your accounts, pairings, saved drafts and preferences unchanged.

## Next

[Pair your phone](pair.md) with each machine.
