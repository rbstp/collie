# Security overview

Your phone can drive every agent on your machine, so `collied` treats every request as hostile ([threat model](../reference/threat-model.md)).

## Network and pairing

- **Tailnet only.** `collied` listens only on its own Tailscale node, on one port; on your local network it opens only Tailscale's UDP socket, no TCP listener ([details](../reference/threat-model.md#lan-attacker)).
- **Your paired phone only.** Pairing needs the QR code from `collied pair`, CollieBar or CollieTray, valid for 120 seconds and one attempt, and your confirmation on the machine. `collied` then serves only your paired phones, rechecking before every message that each is untagged, yours, in your tailnet and on its paired account. Revoking a phone cuts its live sessions.
- **Mutual TLS.** Each side pins the other's key; the phone's never leaves its Secure Enclave. A stolen node key, or a node a compromised Tailscale control plane injects, gets no session.

## What the phone can do

`collied` accepts only a fixed set of requests, never herdr's privileged methods.

- **Approvals** answer only the prompt you saw, once, while it is still on screen; typing keys or text into a prompt skips these checks and Face ID ([details](../reference/threat-model.md#keys-and-text-on-a-blocked-prompt)).
- **Apple Watch** decisions are off by default and need the watch unlocked and on your wrist.
- **Terminals** are off unless `collied.toml` turns them on. Each Face ID or passcode unlock covers one terminal for 5 minutes; the audit log records each command run, never its text.
- **Attachments** are size-limited, integrity-checked, private and deleted after 24 hours.

## Notifications

Only your phone can read what alerts and Live Activities show, labels included: a key it made for this machine encrypts it all. Apple and the default publisher relay see only generic alert text and metadata such as timing, sizes and opaque IDs; the relay also sees IP addresses ([privacy policy](../privacy.md#push-relay)).

## Secrets on the machine

- **Publisher relay (default):** no APNs key; the private `push.json` holds relay grants that can send to your phone until they expire or the app revokes them, which it does when you remove the machine.
- **Direct APNs** (`collied setup --direct-apns`): an APNs key, which on macOS only the signed `collied` reads from the login Keychain without a prompt, and on Linux any process running as you can decrypt.

## Your part

- Keep the [tailnet policy](../install/tailnet-policy.md) tight and check the login name when you pair.
- If you lose a phone, run `collied peers revoke` and remove its node in the Tailscale admin console ([Revocation](../reference/tailnet.md#revocation)).
- If a machine is compromised, remove it in the app and, with direct APNs, revoke its key in the Apple Developer portal.
