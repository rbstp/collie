# Pair a phone

Pair your phone with each machine once. Pairing records the phone's keys on the machine and the machine's keys on the phone.

1. On the machine, start a pairing window. Use one of:
   - the last step of `collied setup`;
   - `collied pair`;
   - **Pair a Phone** in [CollieBar](../desktop/colliebar.md) or [CollieTray](../desktop/collietray.md).
2. In the app, open **Machines**, tap **Add machine** and scan the QR code.
3. Back on the machine, check the login name and device shown, then confirm with `y` (or **Pair** in the companion window).

The pairing window stays open for 120 seconds, and the first attempt closes it, right or wrong. If you do not answer within 60 seconds, `collied` refuses the pairing.

> [!WARNING]
> `collied` accepts only a phone signed in to its own Tailscale account. Sign in inside Collie with that same personal account, and check that the confirmation prompt shows your login before you confirm.

## Pair again

Pair again after you delete and reinstall the app, or when the app asks you to (for example, to use [terminals](terminals.md) with a phone paired before they existed).

- **After a reinstall**, the app has a new Tailscale node, and the machine still lists the old one. Before you pair, remove the old phone with `collied peers revoke` and remove its node in the Tailscale admin console.
- **When the app asks you to pair again**, the phone keeps its node, and the new pairing replaces the old one. Do not revoke the phone or remove its node.

## Remove a machine

When you remove a machine in the app, the app also revokes the phone on that machine if it can reach it. Otherwise, run `collied peers revoke <label or StableID>` on the machine. See [Commands](../operate/commands.md).
