# Security overview

Security is collie's first requirement: the phone can drive every agent on your machine, so `collied` treats every request as hostile until proven otherwise. This page is a summary. The [threat model](../reference/threat-model.md) covers each threat, its mitigations and what remains.

## Network

- **Tailnet only.** `collied` accepts connections only through its embedded Tailscale node, on one port. A test asserts it opens no kernel TCP listener.
- **Whois gate.** Every connection is checked with Tailscale whois before anything else. The peer must be untagged, not shared in from another tailnet, owned by the collie owner, and a paired phone whose node still belongs to the user it was paired with. Authorization is checked again before every message, and revoking a phone cuts its live sessions.
- **Mutual TLS.** Inside the tunnel, the phone pins the machine's TLS key from the pairing QR code, and `collied` pins the phone's key, which lives in the Secure Enclave. A copied Tailscale node key, or a node injected by a compromised control plane, gets no session.

## Pairing

A pairing needs the QR code shown by `collied pair` and your `y` on the machine. The window lasts 120 seconds and the first attempt closes it.

## What the phone can do

`collied` accepts a fixed list of requests and rejects anything else before reading it. herdr's privileged methods cannot be reached from the phone.

- **Approvals** carry a one-time nonce and a fingerprint of the prompt on screen. `collied` re-reads the screen and answers only if the fingerprint still matches.
- **Apple Watch** decisions are off by default, need the watch unlocked and on your wrist, and go through the iPhone's same checks.
- **Terminals** are off unless `collied.toml` turns them on. Each unlock is signed by a second Secure Enclave key after Face ID or the passcode, for one terminal and 5 minutes, and audited without the command text.
- **Attachments** are size-capped, checksummed, stored privately and deleted after 24 hours.

## Notifications

All private alert and Live Activity display content, including initial state and static labels, is authenticated ChaCha20-Poly1305 ciphertext under a per-machine key generated on the phone. Apple sees generic alerts, opaque IDs, urgency, timing, frequency, sizes and activity lifecycle. The phone shows generic state without approval controls when it cannot authenticate and decrypt the content.

## Secrets on the machine

- **macOS**: the APNs key is in the login Keychain, readable without a prompt only by the Developer ID signed `collied`.
- **Linux**: it is a systemd user credential, encrypted with the host key and sealed to the TPM2 when one is usable, with a file readable only by you as the fallback. Any process running as your user can decrypt it.

## Your part

- Keep the [tailnet policy](../install/tailnet-policy.md) tight, and check the login name when you pair.
- When a phone is lost, revoke it with `collied peers revoke` and remove its node in the Tailscale admin console. See [Revocation](../reference/tailnet.md#revocation).
- Revoke a machine's APNs key in the Apple Developer portal if that machine is compromised.
