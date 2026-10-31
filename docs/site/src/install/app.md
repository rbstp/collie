# Install the app

Collie is not on the App Store yet; build and install it from your Mac.

1. Set [your own identifiers](requirements.md#your-own-identifiers).
2. Turn on Developer Mode on the iPhone and connect it to the Mac.
3. Install the app and its watch app:

   ```sh
   just ios-run-device
   ```

4. Open Collie, tap **Sign in to Tailscale** and use the personal account you used for `collied`.

> [!NOTE]
> To sign the watch app, register the paired Apple Watch with your team once, in Xcode's Devices and Simulators window or by its UDID in the developer portal.

Next, [pair your phone](pair.md) with each machine.

## Try the sample workspace

To try Collie with made-up agents and no Tailscale login or machine, tap **Explore sample workspace** on Welcome, or at the bottom of Settings once signed in. Nothing you do there reaches a machine. The sample leaves out new tasks, pairing, attachments, dictation, Live Activities and remote notifications.

In the sample, **Reset sample workspace** in Settings restores it, and **Exit** returns to your setup. It lives only in memory and changes none of your accounts, pairings, drafts or preferences.
