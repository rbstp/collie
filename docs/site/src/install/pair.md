# Pair a phone

This page describes the next matching phone and desktop candidate; user-owned setup is unreleased. Published desktop v0.1.0 still uses tagged nodes. Follow the [v0.1.0 tailnet procedure](https://github.com/rbstp/collie/blob/v0.1.0/docs/tailnet.md) until both peers are updated.

Pairing records the phone's keys on the machine and the machine's keys on the phone. Each machine is paired on its own.

1. On the machine, start a pairing window. Use one of:
   - the last step of `collied setup`;
   - `collied pair`;
   - **Pair a Phone** in [CollieBar](../desktop/colliebar.md) or [CollieTray](../desktop/collietray.md).
2. In the app, open **Machines**, tap **Add machine** and scan the QR code.
3. Back on the machine, check the login name and device shown, then confirm with `y` (or **Pair** in the companion window).

The window stays open for 120 seconds, and the first attempt closes it, right or wrong. If you do not answer within 60 seconds, the answer is no.

> [!WARNING]
> The owner is bound to collied's verified embedded Tailscale user before the first pairing. Sign in inside Collie with that same personal account and check that the confirmation prompt shows your login. The pairing code and local confirmation are still required.

## Pair again

Pair again after you delete and reinstall the app, or when the app asks you to (for example to use [terminals](terminals.md) with a phone paired before they existed). Remove the old phone with `collied peers revoke` and in the Tailscale admin console.

## Remove a machine

Removing a machine in the app also revokes the phone on that machine when the machine is reachable. Otherwise run `collied peers revoke <label or StableID>` on the machine. See [Commands](../operate/commands.md).
