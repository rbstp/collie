# Privacy policy

Last updated: October 16, 2026.

This policy covers the Collie app for iPhone and Apple Watch, its notification extension and widgets, and `collied`, the daemon you run on your own Mac or Linux machine.

## In short

- collie has no server and no account. The developer does not receive your data.
- The app talks to your own machines over your own tailnet, end to end encrypted.
- The app contains no analytics, advertising or tracking code. Apart from its own connection to your machines, the only component that talks to the network is the embedded Tailscale client.
- The services involved are Tailscale, to connect your devices; Apple's push notification service, if you turn on notifications; and, from your machine and with your own credentials, GitHub through `gh` and OpenAI through Codex, for the features described below.

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

- its Tailscale node keys and the machines you paired, in a state folder excluded from iCloud and Finder backups;
- its signing keys and per-machine notification keys, in the iPhone's Keychain, available on this device only and not synced. The TLS signing key is generated in the Secure Enclave and cannot be exported;
- your settings, saved prompts, key shortcuts and other preferences, and unsent drafts (with their attachments for up to 23 hours). These can be included in your device and iCloud backups.

Deleting the app removes its files, but iOS can keep the app's Keychain items on the device, and the app uses them again if you reinstall it.

**On your Apple Watch**, the watch app keeps the last approvals, agent list and plan usage that the iPhone sent it.

## What goes between your phone and your machine

The phone and `collied` talk only over your tailnet, inside mutual TLS. This traffic carries what you see and do in the app: the agent list, terminal screens, conversation history while Chat is open, Git diffs, pull request status, plan usage, your prompts, keys and attachments, and approval decisions. The phone also gives `collied` its push notification token and a notification key it generated, so the machine can send it encrypted alerts.

Conversation history and the summary lines shown in the Inbox are never written to logs or put in notifications by `collied`.

## Tailscale

Both the app and `collied` embed a Tailscale client. You sign in to your own Tailscale account, and Tailscale's coordination service learns about these devices as it does for any device on your tailnet. Traffic between your devices is encrypted with WireGuard and may pass through Tailscale's relay servers when a direct connection is not possible; the relays cannot read it. Log upload to Tailscale is turned off in both embedded clients. Tailscale's handling of your account and device data is covered by [Tailscale's privacy policy](https://tailscale.com/privacy-policy).

## Push notifications

If you set up notifications, `collied` on your machine sends them through Apple Push Notification service. Alert titles, bodies, agent and workspace names, pull request details, pending commands, and the private attributes and initial/update/end state of Live Activities are encrypted with a key generated on your phone. Only your paired machine and phone hold that key. Apple receives generic alert text and ciphertext.

Apple can still see notification delivery destinations, opaque machine/thread/approval/event/activity identifiers, generic categories, notification priority, timing, frequency, ciphertext size and Live Activity lifecycle. A generic alert can distinguish an agent finishing from an agent needing attention. Initial and local activity content use the same encrypted storage shape. Your phone's notification extension and widget decrypt the content locally; before the first unlock after reboot, or if decryption fails, they show generic content without approval controls. Approval nonces and decisions travel only over your authenticated tailnet connection.

The app keeps a protected App Group mirror of notification keys so the widget can read them. It is available only to Collie's own targets after the first unlock, stays on this device and is excluded from backups. No push relay is enabled by this change. If a future release uses one, its metadata handling will be described here before release. Apple's handling of APNs is covered by [Apple's privacy policy](https://www.apple.com/legal/privacy/).

## GitHub and Codex, from your machine

`collied` runs these tools on your machine, as you, with the credentials you already gave them. collie has no credentials of its own.

- **GitHub CLI (`gh`)**: while the app is connected, or when you turned on archiving merged pull requests, `collied` asks GitHub about the pull request of each agent's branch, about every 2 minutes. It also polls the pull requests you watch, at the interval you chose, with or without the app. When you ask, it clones repositories, fetches pull request branches and runs `gh poi` to clean up merged branches.
- **Codex**: `collied` asks the local Codex app-server for your account's usage limits every five minutes, which Codex reads from OpenAI.

GitHub's and OpenAI's handling of these requests is covered by their own policies.

## Device features

- **Camera**: used only to scan the pairing QR code. Images are not stored or sent.
- **Microphone and speech**: dictation uses Apple's on-device speech recognition. Audio never leaves the phone. iOS may download the speech model for your language from Apple.
- **Face ID or passcode**: handled by iOS. The app only learns whether you passed.
- **Photos and files**: only the ones you attach are read, and they go only to the machine you send them to.

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
