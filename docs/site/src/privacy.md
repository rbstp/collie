# Privacy policy

Last updated: October 18, 2026.

This policy covers the Collie app for iPhone and Apple Watch, its notification extension and widgets, and `collied`, the daemon you run on your own Mac or Linux machine.

## In short

- Collie has no publisher account. Released clients use direct push delivery. A publisher push relay is under validation and is not open for public enrollment; [Publisher relay under validation](#publisher-relay-under-validation) describes its metadata handling.
- The app talks to your own machines over your own tailnet, end to end encrypted.
- The app contains no advertising or tracking code. Relay validation also includes a phone client for Apple App Attest and the publisher relay, separate from direct device sessions.
- The services involved are Tailscale, to connect your devices; Apple's push notification and attestation services; Cloudflare for the relay when enabled; and, from your machine and with your own credentials, GitHub through `gh` and OpenAI through Codex, for the features described below.

## What stays on your devices

**On your machine**, `collied` keeps in its data folder:

- its configuration and its Tailscale node keys;
- the paired phones, and each phone's push notification token and notification key;
- the audit log, and its service log (the user journal on Linux);
- recorded plan usage;
- starred agents, and the pull requests you watch, with their URLs;
- attachments you sent from the phone, deleted after 24 hours.

Your agents' transcripts and your files stay where the agents keep them; `collied` reads them only to answer the phone.

**On your iPhone**, the app keeps:

- its Tailscale node keys and the machines you paired, including a Tailscale user ID ownership pin in the user-owned setup, in a state folder excluded from iCloud and Finder backups;
- its signing keys and per-machine notification keys, in the iPhone's Keychain, available on this device only and not synced. The app generates the TLS signing key in the Secure Enclave, and the key cannot be exported;
- your settings, saved prompts, key shortcuts and other preferences, and unsent drafts (with their attachments for up to 23 hours). Your device and iCloud backups can include these.

Deleting the app removes its files, but iOS can keep the app's Keychain items on the device, and the app uses them again if you reinstall it.

**On your Apple Watch**, the watch app keeps the last approvals, agent list and plan usage that the iPhone sent it.

## What goes between your phone and your machine

The phone and `collied` talk only over your tailnet, inside mutual TLS. This traffic carries what you see and do in the app: the agent list, terminal screens, conversation history while Chat is open, Git diffs, pull request status, plan usage, your prompts, keys and attachments, and approval decisions. The phone also gives `collied` its push notification token and a notification key it generated, so the machine can send it encrypted alerts.

`collied` never writes conversation history or the summary lines shown in the Inbox to logs, and never puts them in notifications.

## Tailscale

Both the app and `collied` embed their own Tailscale client. Desktop v0.1.0 uses a tagged node; desktop v0.1.1 and later use user-owned nodes signed in with the same personal account as the phone. Neither flow installs or requires an OS VPN profile, and a separate corporate VPN client keeps its own account and state.

You sign in to your own Tailscale account, and Tailscale's coordination service learns about these devices as it does for any device on your tailnet. WireGuard encrypts the traffic between your devices. When a direct connection is not possible, that traffic may pass through Tailscale's relay servers, which cannot read it. Both embedded clients have log upload to Tailscale turned off. [Tailscale's privacy policy](https://tailscale.com/privacy-policy) covers its handling of your account and device data.

## Push notifications

If you set up notifications, `collied` on your machine sends them through Apple Push Notification service. Alert titles, bodies, agent and workspace names, pull request details, pending commands, and the private attributes and initial/update/end state of Live Activities are encrypted with a key generated on your phone. Only your paired machine and phone hold that key. Apple receives generic alert text and ciphertext.

Apple can still see notification delivery destinations, opaque machine/thread/approval/event/activity identifiers, generic categories, notification priority, timing, frequency, ciphertext size and Live Activity lifecycle. A generic alert can distinguish an agent finishing from an agent needing attention. Initial and local activity content use the same encrypted storage shape. Your phone's notification extension and widget decrypt the content locally. Before the first unlock after reboot, or if decryption fails, they show generic content without approval controls. Approval nonces and decisions travel only over your authenticated tailnet connection.

The app keeps a protected App Group mirror of notification keys so the widget can read them. Only Collie's own targets can use this mirror, and only after the first unlock; it stays on this device and is excluded from backups. Public relay enrollment remains closed pending validation; [Publisher relay under validation](#publisher-relay-under-validation) describes the relay data flow. [Apple's privacy policy](https://www.apple.com/legal/privacy/) covers Apple's handling of APNs.

## GitHub and Codex, from your machine

`collied` runs these tools on your machine, as you, with the credentials you already gave them. collie has no credentials of its own.

- **GitHub CLI (`gh`)**: while the app is connected, or when you turned on archiving merged pull requests, `collied` asks GitHub about the pull request of each agent's branch, about every 2 minutes. It also polls the pull requests you watch, at the interval you chose, with or without the app. When you ask, it clones repositories, fetches pull request branches and runs `gh poi` to clean up merged branches.
- **Codex**: `collied` asks the local Codex app-server for your account's usage limits every five minutes, which Codex reads from OpenAI.

GitHub's and OpenAI's own policies cover their handling of these requests.

## Device features

- **Camera**: used only to scan the pairing QR code. Images are not stored or sent.
- **Microphone and speech**: dictation uses Apple's on-device speech recognition. Audio never leaves the phone. iOS may download the speech model for your language from Apple.
- **Face ID or passcode**: handled by iOS. The app only learns whether you passed.
- **Photos and files**: the app reads only the ones you attach, and sends them only to the machine you send them to.

## Diagnostics

The app writes connection diagnostics to the iPhone's own system log. They stay on the device unless you share them, for example in a sysdiagnose. If you opt in to sharing analytics with app developers in iOS, or use a TestFlight build, Apple may share crash reports, and any feedback you choose to send, with the developer under Apple's terms.

## Your agents

The coding agents you run (Claude Code, Codex, Copilot CLI) talk to their own services under their own terms, whether or not you use collie. What you send them from the phone, such as prompts, attachments and answers, reaches them as if you had typed it on the machine. For example, answering a Claude Code tip's feedback question from the phone makes Claude Code send Anthropic the same feedback signal it would send if you answered on the machine.

## Children

collie is a developer tool and is not directed at children.

## Changes

Changes to this policy are published on this page with a new date. If a future version of collie adds a service that handles your data, such as a push relay for an App Store release, this policy will describe it before that version is released.

## Contact

Questions about this policy: open an issue on [GitHub](https://github.com/rbstp/collie/issues), or see [Support](support.md).

## Publisher relay under validation

The repository includes a publisher push relay and an attested phone client. They are not a publicly enabled service. Current direct notification delivery sends no data to this relay. Before a relay-enabled build is distributed, the deployment settings and this policy must be checked together. Local validation does not establish automatic deletion or production availability.

In relay mode, the phone enrolls with Apple App Attest and authorizes routes for confirmed paired machines. The machine sends encrypted notification content through the relay instead of holding the publisher's Apple sending key. Commands, chats, files and approval decisions still travel directly over your authenticated tailnet connection. The relay receives no notification decryption key and cannot read encrypted names, text or detailed activity state.

The publisher and Cloudflare process source IP addresses, app/activity notification tokens, installation/pairing/route relationships, generic push categories, opaque event identifiers, timing, frequency and ciphertext sizes. Even though the content is encrypted, this metadata exposes delivery patterns. Apple receives delivery metadata and ciphertext, and processes device/app integrity information for App Attest. The relay stores verified public keys and attestation receipts, counters, revocation state and delivery-route records in Cloudflare D1. It stores no notification payloads or decryption keys. The relay uses IP addresses for transient abuse limiting and does not store them in these D1 records.

The current code retains enrollment state for at least 37 days from enrollment, and extends its deadline to the latest issued grant expiry plus seven days. Grants last 30 days. Superseding a route removes the previous destination. Other expired or revoked destinations become eligible for bounded cleanup only after expiry plus seven days. Parent key/receipt and revocation records are removed once their retention deadline and child cleanup allow it.

The validation configuration has no cleanup schedule, so no live deletion deadline has been established. Before public enrollment, the cleanup schedule and backlog bound must be verified, and this policy updated to state that bound. The publisher account uses Workers Paid. Cloudflare D1 [Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/) is always on and can retain restorable database history for 30 days beyond an active-database change, so removing a live record does not immediately remove it from that history.

If the relay is unreachable when you remove a pairing, the phone keeps a minimal pending revocation. A previously authorized machine may still send until remote revocation succeeds or its grant expires. Local removal does not immediately delete relay records. An inactive phone whose grants expire stops receiving relay pushes until it renews them; direct sessions remain available.

The checked-in relay configuration disables observability, and the handler emits no request-body, authorization, destination or attestation logs. Provider service and security processing is separate from application logging; [Cloudflare's privacy policy](https://www.cloudflare.com/privacypolicy/) covers it. Before this policy can describe a live service, a future deployment must verify those settings and retention. No marketing or advertising use is implemented.

When `collied` is configured with `[push.relay]`, it forwards encrypted notification envelopes using this machine's destination grants from the paired phone. It keeps the grants only in its private local registration file. For retries, it holds a bounded amount of encrypted work in RAM for a limited time, and loses it on restart; it stores no payload queue on disk. Its diagnostic output contains counts, expiry/error codes and retry times, with no grants, destination tokens or notification contents. The relay remains subject to the deployment, attestation and real-device validation gates documented in the relay reference.
