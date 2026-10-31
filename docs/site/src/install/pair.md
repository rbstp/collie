# Pair a phone

Pair your phone once with each machine.

1. On the machine, open a pairing window: answer `y` at the end of `collied setup`, run `collied pair`, or choose **Pair a Phone…** in [CollieBar](../desktop/colliebar.md) or [CollieTray](../desktop/collietray.md).
2. In the app, open **Machines**, tap **Add machine** and scan the QR code.
3. On the machine, check that the login and device shown are yours, then confirm with `y`, or **Pair** in CollieBar or CollieTray.

The window closes after 120 seconds or at the first attempt, right or wrong, and you have 60 seconds to confirm.

## Pair again

- **After a reinstall**, the app has a new Tailscale node: first revoke the old phone with `collied peers revoke` and remove the old `collie-phone` node in the Tailscale admin console.
- **When the app asks you to**, for example for [terminals](terminals.md), pair without revoking the phone or removing its node: the new pairing replaces the old one.

## Remove a machine

Removing a machine in the app also revokes the phone there when the app can reach it. Otherwise, run `collied peers revoke <label or StableID>` there.
