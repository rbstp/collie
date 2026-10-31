# Privacy policy

Last updated: October 31, 2026.

This policy covers the Collie app for iPhone and Apple Watch, its notification extension and widgets, and `collied`, the daemon you run on your own Mac or Linux machine.

## In short

- Collie needs no account with the publisher. Your machine sends notifications through the publisher's [push relay](#push-relay), the `collied setup` default, or directly to Apple for a self-built app.
- The app talks to your own machines over your own tailnet, end to end encrypted.
- The app contains no advertising or tracking code.
- The services involved are Tailscale, to connect your devices; Apple's push notification and attestation services; Cloudflare, which hosts the push relay; and, from your machine with your own credentials, GitHub through `gh` and OpenAI through Codex.

## What stays on your devices

**On your machine**, `collied` keeps in its data folder:

- its configuration and its Tailscale node keys;
- the paired phones, with each phone's push notification token and notification key;
- the audit log, and its service log (the user journal on Linux);
- recorded plan usage, and token counts from your agents' transcripts for the last 32 days, by transcript, model and quarter hour, without any message text;
- starred agents, the pull requests you watch with their URLs, and the last 20 prompts that watching typed into each agent, so Chat can tell them from yours;
- attachments you sent from the phone, deleted after 24 hours.

Your agents' transcripts and your files stay where the agents keep them; `collied` reads them only to answer the phone.

**On your iPhone**, the app keeps:

- its Tailscale node keys and the machines you paired, including a Tailscale user ID as an ownership pin, in a state folder excluded from iCloud and Finder backups;
- its signing keys and per-machine notification keys, in the iPhone's Keychain, on this device only and never synced; its TLS signing key is generated in the Secure Enclave and cannot be exported;
- your settings, saved prompts, key shortcuts and other preferences, and unsent drafts (with their attachments for up to 23 hours); device and iCloud backups can include all of these.

Deleting the app removes its files, but iOS can keep its Keychain items, which the app uses again if you reinstall it.

**On your Apple Watch**, the watch app keeps the last approvals, agent list and plan usage the iPhone sent it.

## What goes between your phone and your machine

The phone and `collied` talk only over your tailnet, inside mutual TLS. This traffic carries what you see and do in the app: the agent list, terminal screens, conversation history while Chat is open or grid cards preview it, Git diffs, pull request status, plan usage, token counts and estimated costs (numbers, model names and dates only; the phone sends its time zone offset so days end at your midnight), your prompts, keys and attachments, files you open from a Chat link or a Terminal selection, and approval decisions. The phone also sends its push notification token and a notification key it generated, for encrypted alerts.

`collied` never logs conversation history or the Inbox's summary lines, and notifications leave them out, except that a blocked Codex agent's alert can quote the agent's latest response from its screen.

For a file you open, `collied` writes only its path to the audit log and never logs, caches or pushes its content. The phone keeps the file in memory, plus a temporary copy for Share that it deletes when you close the preview.

## Tailscale

The app and `collied` each embed their own Tailscale client, a node owned by your personal Tailscale account, and install no OS VPN profile; a separate corporate VPN client keeps its own account and state.

Tailscale's coordination service learns about these devices as about any device on your tailnet. WireGuard encrypts the traffic between them; when no direct connection is possible, it may pass through Tailscale's relay servers, which cannot read it. Both embedded clients have log upload to Tailscale turned off. [Tailscale's privacy policy](https://tailscale.com/privacy-policy) covers its handling of your account and device data.

## Push notifications

If you set up notifications, `collied` sends them through Apple Push Notification service (APNs), via the push relay or directly. A key generated on your phone encrypts alert titles and bodies, agent and workspace names, pull request details, pending commands, and Live Activities' private attributes and every state they show. Only your paired machine and phone hold that key; Apple receives generic alert text and ciphertext.

Apple still sees delivery destinations; opaque machine, thread, approval, event and activity identifiers; generic categories; notification priority, timing, frequency and ciphertext size; and Live Activity lifecycle. The generic text still tells Apple whether an agent finished or needs attention. Live Activity content the phone starts or updates itself is stored encrypted the same way. Your phone's notification extension and widget decrypt the content locally. Approval nonces and decisions travel only over your authenticated tailnet connection.

For the widget, the app keeps a protected copy of the notification keys in its App Group, usable only by Collie's own targets after the first unlock, on this device only and excluded from backups. [Apple's privacy policy](https://www.apple.com/legal/privacy/) covers Apple's handling of APNs.

## GitHub and Codex, from your machine

`collied` runs these tools on your machine, as you, with your existing credentials; collie has none of its own.

- **GitHub CLI (`gh`)**: while the app is connected, or with archiving merged pull requests turned on, `collied` checks the pull request of each agent's branch on GitHub about every 2 minutes. With or without the app, it polls the pull requests you watch at the interval you chose, and every five minutes asks GitHub for your Copilot premium usage, keeping only the used and allowed amounts, whether the seat is unlimited, how it is billed and the reset date. When you ask, it clones repositories, fetches pull request branches and runs `gh poi` to clean up merged branches.
- **Codex**: every five minutes, `collied` asks the local Codex app-server for your account's usage limits, which Codex reads from OpenAI.

GitHub's and OpenAI's own policies cover their handling of these requests.

## Device features

- **Camera**: only scans the pairing QR code. Images are neither stored nor sent.
- **Microphone and speech**: dictation uses Apple's on-device speech recognition, and audio never leaves the phone. iOS may download the speech model for your language from Apple.
- **Face ID or passcode**: handled by iOS. The app learns only whether you passed.
- **Photos and files**: the app reads only those you attach and sends them only to the machine you chose.

## Diagnostics

The app writes connection diagnostics to the iPhone's own system log, where they stay unless you share them, for example in a sysdiagnose. If you opt in to sharing analytics with app developers in iOS, or use a TestFlight build, Apple may share crash reports, and any feedback you choose to send, with the developer under Apple's terms.

## Your agents

The coding agents you run (Claude Code, Codex, Copilot CLI) talk to their own services under their own terms, whether or not you use collie. What you send them from the phone, such as prompts, attachments and answers, reaches them as if typed on the machine. Answering a Claude Code tip's feedback question from the phone, for example, makes Claude Code send Anthropic the same feedback signal as on the machine.

## Children

collie is a developer tool and is not directed at children.

## Changes

Changes to this policy appear on this page with a new date. If a collie version adds a service that handles your data, this page will describe it before that version is released.

## Contact

Questions about this policy: open an issue on [GitHub](https://github.com/rbstp/collie/issues), or see [Support](support.md).

## Push relay

The publisher runs the push relay on Cloudflare so that your machine can notify the publisher's app without holding the publisher's Apple sending key. A machine uses it when its `collied.toml` has `[push.relay]`, the `collied setup` default; one set up with `[apns]` for a self-built app sends directly to Apple and nothing to the relay.

To use the relay, the app proves with Apple App Attest that it is a genuine copy, separately from its sessions with your machines, then authorizes each paired machine to send to it. Your machine then sends encrypted notification content through the relay; commands, chats, files and approval decisions still travel only over your tailnet. The relay never receives a notification key, so it cannot read names, text or activity details.

The publisher and Cloudflare see source IP addresses, the app's and each Live Activity's notification tokens, which installations, pairings and routes belong together, generic push categories, opaque event identifiers, timing, frequency and ciphertext sizes. This metadata shows when and how often you receive notifications. For App Attest, Apple checks device and app integrity.

The relay keeps, in Cloudflare D1, the app's verified public key and attestation receipt, replay counters, revocation records, delivery routes with their notification tokens, and daily send counts. It keeps no notification content, decryption keys or IP addresses; it uses IP addresses only for short-lived abuse limits.

The relay deletes these records on a schedule:

- A grant lasts 30 days, and the app renews it before it expires.
- When the app registers a new notification token, the relay deletes the old one at once.
- An expired or revoked route becomes eligible for deletion seven days after its grant expires.
- Your installation's remaining records, including its key and receipt, become eligible seven days after its last grant expires, and no earlier than 37 days after the app first enrolled.
- An hourly cleanup job deletes eligible records within seven days, even at the relay's daily limits.

So the relay deletes your records within 14 days after your last grant expires. Cloudflare D1 [Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/) keeps a restorable history of the database, so a deleted record can remain there for up to 30 days after its deletion.

If the relay is unreachable when you remove a pairing, the phone keeps a minimal pending revocation and retries it. Until the revocation succeeds or the grant expires, the removed machine can still send to your phone, and its relay records are not deleted at once. If you stop opening the app until your grants expire, relay notifications stop until the app renews them; your direct sessions keep working.

The relay runs with Cloudflare's invocation logs and traces turned off, and its code logs no request bodies, authorization, notification tokens or attestations. Cloudflare's own service and security processing is separate; [Cloudflare's privacy policy](https://www.cloudflare.com/privacypolicy/) covers it. The relay has no marketing or advertising use.

`collied` keeps the destination grants the phone gives it only in its private local registration file. For retries, it holds a bounded number of encrypted notifications in memory for a limited time, never on disk, and loses them on restart. Its diagnostic output shows counts, expiry, error codes and retry times, never grants, destination tokens or notification content.
