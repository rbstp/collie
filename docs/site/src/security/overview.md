# Security overview

Security is collie's first requirement: the phone can drive every agent on your machine, so `collied` treats every request as hostile until proven otherwise. This page sums up what protects you and what is left to you. The [threat model](../reference/threat-model.md) covers each threat, its mitigations and what remains.

## Network

- **Tailnet only.** `collied` accepts application connections only through its own Tailscale node, on one port, and exposes no TCP listener on your local network. Tailscale's own UDP socket stays reachable, as the [threat model](../reference/threat-model.md) describes.
- **Your paired phone only.** Before anything else, `collied` asks Tailscale who is connecting. It accepts only an untagged phone from your own tailnet, owned by you, that you paired and that still belongs to the account it was paired with. `collied` checks this again before every message, and revoking a phone cuts its live sessions.
- **Mutual TLS.** Inside the tunnel, the phone pins the machine's key from the pairing QR code, and `collied` pins the phone's key, which never leaves its Secure Enclave. A stolen Tailscale node key, or a node injected by a compromised Tailscale control plane, gets no session.

## Pairing

Nobody can pair without the QR code that `collied pair` shows and your `y` on the machine. The pairing window lasts 120 seconds, and the first attempt closes it.

## What the phone can do

`collied` accepts a fixed list of requests and rejects anything else before reading it. The phone cannot reach herdr's privileged methods.

- **Approvals** answer only the prompt you saw. `collied` reads the screen again and answers only if the same prompt is still there, and each approval can be used once.
- **Apple Watch** decisions are off by default, need the watch unlocked and on your wrist, and go through the same checks as the iPhone.
- **Terminals** are off unless `collied.toml` turns them on. Each unlock needs Face ID or the passcode, is signed by a separate key in the phone's Secure Enclave, and covers one terminal for 5 minutes. The audit log records it without the command text.
- **Attachments** are limited in size, checked for integrity, stored privately and deleted after 24 hours.

## Notifications

Only your phone can read what alerts and Live Activities show, including their labels: `collied` encrypts all of it with a key your phone made for this machine. Apple sees generic alerts, opaque IDs, urgency, timing, frequency, sizes and when activities start and end. When the phone cannot verify and decrypt the content, it shows a generic state without approval controls.

## Secrets on the machine

- **macOS**: the APNs key is in the login Keychain, where only the Developer ID signed `collied` can read it without a prompt.
- **Linux**: the APNs key is a systemd user credential, encrypted with the host key and sealed to the TPM2 when one is usable, or else a file readable only by you. Any process running as your user can decrypt it.

## Your part

- Keep the [tailnet policy](../install/tailnet-policy.md) tight, and check the login name when you pair.
- When a phone is lost, revoke it with `collied peers revoke` and remove its node in the Tailscale admin console. See [Revocation](../reference/tailnet.md#revocation).
- If a machine is compromised, revoke its APNs key in the Apple Developer portal.
