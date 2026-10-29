# Privacy policy

Last updated: October 29, 2026.

This policy covers the Collie app for iPhone and Apple Watch, its notification extension and widgets, and `collied`, the daemon you run on your own Mac or Linux machine.

## In short

- Collie has no publisher account. Your machine sends notifications through the publisher's push relay, which `collied setup` chooses by default, or directly to Apple for a self-built app; [Push relay](#push-relay) describes what the relay sees and keeps.
- The app talks to your own machines over your own tailnet, end to end encrypted.
- The app contains no advertising or tracking code. To use the push relay, it proves to the relay with Apple App Attest that it is a genuine copy; this is separate from the sessions with your machines.
- The services involved are Tailscale, to connect your devices; Apple's push notification and attestation services; Cloudflare, which hosts the push relay; and, from your machine and with your own credentials, GitHub through `gh` and OpenAI through Codex, for the features described below.

## What stays on your devices

**On your machine**, `collied` keeps in its data folder:

- its configuration and its Tailscale node keys;
- the paired phones, and each phone's push notification token and notification key;
- the audit log, and its service log (the user journal on Linux);
- recorded plan usage, and token counts from your agents' transcripts for the last 32 days, by transcript, model and quarter hour, without any message text;
- starred agents, the pull requests you watch, with their URLs, and the last 20 prompts that watching typed into each agent, so Chat can tell them from yours;
- attachments you sent from the phone, deleted after 24 hours.

Your agents' transcripts and your files stay where the agents keep them; `collied` reads them only to answer the phone.

**On your iPhone**, the app keeps:

- its Tailscale node keys and the machines you paired, including a Tailscale user ID ownership pin in the user-owned setup, in a state folder excluded from iCloud and Finder backups;
- its signing keys and per-machine notification keys, in the iPhone's Keychain, available on this device only and not synced. The app generates the TLS signing key in the Secure Enclave, and the key cannot be exported;
- your settings, saved prompts, key shortcuts and other preferences, and unsent drafts (with their attachments for up to 23 hours). Your device and iCloud backups can include these.

Deleting the app removes its files, but iOS can keep the app's Keychain items on the device, and the app uses them again if you reinstall it.

**On your Apple Watch**, the watch app keeps the last approvals, agent list and plan usage that the iPhone sent it.

## What goes between your phone and your machine

The phone and `collied` talk only over your tailnet, inside mutual TLS. This traffic carries what you see and do in the app: the agent list, terminal screens, conversation history while Chat is open or grid cards preview it, Git diffs, pull request status, plan usage, token counts and estimated costs (numbers, model names and dates only, for which the phone sends its current time zone offset so days end at your midnight), your prompts, keys and attachments, files you open from a Chat link or a Terminal selection, and approval decisions. The phone also gives `collied` its push notification token and a notification key it generated, so the machine can send it encrypted alerts.

`collied` never writes conversation history or the summary lines shown in the Inbox to logs, and never puts them in notifications.

When you tap a file link in Chat, or choose Open on a selected path in Terminal, `collied` sends that file to the phone and writes only its path to the audit log; it never logs, caches or pushes the file's content. The phone keeps the file in memory and in a temporary copy for Share, and deletes the copy when you close the preview.

## Tailscale

Both the app and `collied` embed their own Tailscale client. Desktop v0.1.0 uses a tagged node; desktop v0.1.1 and later use user-owned nodes signed in with the same personal account as the phone. Neither flow installs or requires an OS VPN profile, and a separate corporate VPN client keeps its own account and state.

You sign in to your own Tailscale account, and Tailscale's coordination service learns about these devices as it does for any device on your tailnet. WireGuard encrypts the traffic between your devices. When a direct connection is not possible, that traffic may pass through Tailscale's relay servers, which cannot read it. Both embedded clients have log upload to Tailscale turned off. [Tailscale's privacy policy](https://tailscale.com/privacy-policy) covers its handling of your account and device data.

## Push notifications

If you set up notifications, `collied` on your machine sends them through Apple Push Notification service, by way of the push relay or directly. Alert titles, bodies, agent and workspace names, pull request details, pending commands, and the private attributes and initial/update/end state of Live Activities are encrypted with a key generated on your phone. Only your paired machine and phone hold that key. Apple receives generic alert text and ciphertext.

Apple can still see notification delivery destinations, opaque machine/thread/approval/event/activity identifiers, generic categories, notification priority, timing, frequency, ciphertext size and Live Activity lifecycle. A generic alert can distinguish an agent finishing from an agent needing attention. Initial and local activity content use the same encrypted storage shape. Your phone's notification extension and widget decrypt the content locally. Before the first unlock after reboot, or if decryption fails, they show generic content without approval controls. Approval nonces and decisions travel only over your authenticated tailnet connection.

The app keeps a protected App Group mirror of notification keys so the widget can read them. Only Collie's own targets can use this mirror, and only after the first unlock; it stays on this device and is excluded from backups. [Push relay](#push-relay) describes what the relay sees and keeps. [Apple's privacy policy](https://www.apple.com/legal/privacy/) covers Apple's handling of APNs.

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

Changes to this policy are published on this page with a new date. If a future version of collie adds a service that handles your data, this policy will describe it before that version is released.

## Contact

Questions about this policy: open an issue on [GitHub](https://github.com/rbstp/collie/issues), or see [Support](support.md).

## Push relay

The publisher runs a push relay on Cloudflare so that your machine can send notifications to the publisher's app without holding the publisher's Apple sending key. Your machine uses the relay when its `collied.toml` has `[push.relay]`, which `collied setup` chooses by default. A machine that you set up with `[apns]` for a self-built app sends directly to Apple and sends nothing to the relay.

To use the relay, the app proves with Apple App Attest that it is a genuine copy, and then authorizes each paired machine to send to it. Your machine then sends encrypted notification content through the relay. Commands, chats, files and approval decisions still travel only over your tailnet. The relay never receives a notification key, so it cannot read the encrypted names, text or activity details.

The publisher and Cloudflare see source IP addresses, the app's and each Live Activity's notification tokens, which installations, pairings and routes belong together, generic push categories, opaque event identifiers, timing, frequency and ciphertext sizes. Even though the content is encrypted, this metadata shows when and how often you receive notifications. Apple receives delivery metadata and ciphertext, and checks device and app integrity for App Attest.

The relay keeps, in Cloudflare D1, the app's verified public key and attestation receipt, replay counters, revocation records, delivery routes with their notification tokens, and daily send counts. It keeps no notification content, no decryption keys and no IP addresses; it uses IP addresses only for short-lived abuse limits.

The relay deletes these records on a schedule. A grant lasts 30 days, and the app renews it before it expires. When the app registers a new notification token, the relay deletes the old one at once. An expired or revoked route becomes eligible for deletion seven days after its grant expires. Your installation's remaining records, including its key and receipt, become eligible seven days after its last grant expires, and no earlier than 37 days after the app first enrolled. A cleanup job runs every hour and deletes eligible records within seven days, even when the relay runs at its daily limits. So the relay deletes your records within 14 days after your last grant expires. Cloudflare D1 [Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/) keeps a restorable history of the database for 30 days, so a deleted record can remain in that history for up to 30 days after its deletion.

If the relay is unreachable when you remove a pairing, the phone keeps a minimal pending revocation and retries it. Until the revocation succeeds or the grant expires, the removed machine can still send to your phone, and removing it locally does not delete its relay records at once. If you stop opening the app until your grants expire, your phone stops receiving relay notifications until the app renews them; your direct sessions keep working.

The relay runs with Cloudflare's invocation logs and traces turned off, and its code logs no request bodies, authorization, notification tokens or attestations. Cloudflare's own service and security processing is separate; [Cloudflare's privacy policy](https://www.cloudflare.com/privacypolicy/) covers it. The relay has no marketing or advertising use.

When `collied` is configured with `[push.relay]`, it forwards encrypted notification envelopes using this machine's destination grants from the paired phone. It keeps the grants only in its private local registration file. For retries, it holds a bounded amount of encrypted work in RAM for a limited time, and loses it on restart; it stores no payload queue on disk. Its diagnostic output contains counts, expiry/error codes and retry times, with no grants, destination tokens or notification contents.
