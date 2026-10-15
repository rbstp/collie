# Pair a phone

Pairing records the phone's keys on the machine and the machine's keys on the phone. Each machine is paired on its own.

1. On the machine, start a pairing window. Use one of:
   - the last step of `collied setup`;
   - `collied pair`;
   - **Pair a Phone** in [CollieBar](../desktop/colliebar.md) or [CollieTray](../desktop/collietray.md).
2. In the app, open **Machines**, tap **Add machine** and scan the QR code.
3. Back on the machine, check the login name and device shown, then confirm with `y` (or **Pair** in the companion window).

The window stays open for 120 seconds, and the first attempt closes it, right or wrong. If you do not answer within 60 seconds, the answer is no.

> [!WARNING]
> Until the first pairing, `collied` has no owner yet. Check that the login name on the confirmation prompt is yours. The first phone you confirm sets the owner, unless `owner_user_id` is set in `collied.toml`.

## Pair again

Pair again after you delete and reinstall the app, or when the app asks you to (for example to use [terminals](terminals.md) with a phone paired before they existed). Remove the old phone with `collied peers revoke` and in the Tailscale admin console.

## Remove a machine

Removing a machine in the app also revokes the phone on that machine when the machine is reachable. Otherwise run `collied peers revoke <label or StableID>` on the machine. See [Commands](../operate/commands.md).
