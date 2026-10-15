# Privacy policy

Last updated: October 15, 2026.

This policy covers the Collie app for iPhone and Apple Watch, its notification extension and widgets, and `collied`, the daemon you run on your own Mac or Linux machine.

## In short

- collie has no server and no account. The developer does not receive your data.
- The app talks to your own machines over your own tailnet, end to end encrypted.
- The app contains no analytics, advertising or tracking code. Apart from its own connection to your machines, the only component that talks to the network is the embedded Tailscale client.
- Two outside services are involved: Tailscale, to connect your devices, and Apple's push notification service, if you turn on notifications.

## What stays on your devices

**On your machine**, `collied` keeps its configuration, its Tailscale node keys, the list of paired phones, the audit log, recorded plan usage and attachments you sent from the phone. Attachments are deleted after 24 hours. Your agents' transcripts and your files stay where the agents keep them; `collied` reads them only to answer the phone.

**On your iPhone**, the app keeps its Tailscale node keys, the machines you paired, your settings, saved prompts, key shortcuts and other preferences, and unsent drafts (with their attachments for up to 23 hours). Its signing keys live in the Secure Enclave and cannot be exported. The app's state folder, which holds the node keys and paired machines, is excluded from iCloud and Finder backups. Deleting the app deletes this data.

**On your Apple Watch**, the watch app keeps the last approvals, agent list and plan usage that the iPhone sent it.

## What goes between your phone and your machine

The phone and `collied` talk only over your tailnet, inside mutual TLS. This traffic carries what you see and do in the app: the agent list, terminal screens, conversation history while Chat is open, Git diffs, pull request status, plan usage, your prompts, keys and attachments, and approval decisions. The phone also gives `collied` its push notification token and a notification key it generated, so the machine can send it encrypted alerts.

Conversation history and the summary lines shown in the Inbox are never written to logs or put in notifications by `collied`.

## Tailscale

Both the app and `collied` embed a Tailscale client. You sign in to your own Tailscale account, and Tailscale's coordination service learns about these devices as it does for any device on your tailnet. Traffic between your devices is encrypted with WireGuard and may pass through Tailscale's relay servers when a direct connection is not possible; the relays cannot read it. Tailscale's handling of your account and device data is covered by [Tailscale's privacy policy](https://tailscale.com/privacy-policy).

## Push notifications

If you set up notifications, `collied` on your machine sends them through Apple Push Notification service. Apple can see:

- the agent and workspace names shown in the alert, and opaque identifiers;
- when alerts are sent, how often, and the size of the encrypted part;
- for agents you choose to follow in a Live Activity, their status changes and the time of each.

The pending command or question is encrypted with a key generated on your phone, and only the phone can read it. Apple's handling of this data is covered by [Apple's privacy policy](https://www.apple.com/legal/privacy/).

## Device features

- **Camera**: used only to scan the pairing QR code. Images are not stored or sent.
- **Microphone and speech**: dictation uses Apple's on-device speech recognition. Audio never leaves the phone. iOS may download the speech model for your language from Apple.
- **Face ID or passcode**: handled by iOS. The app only learns whether you passed.
- **Photos and files**: only the ones you attach are read, and they go only to the machine you send them to.

## Diagnostics

The app writes connection diagnostics to the iPhone's own system log. They stay on the device unless you share them, for example in a sysdiagnose. If you opt in to sharing analytics with app developers in iOS, or use a TestFlight build, Apple may share crash reports, and any feedback you choose to send, with the developer under Apple's terms.

## Your agents and other tools

The coding agents you run (Claude Code, Codex, Copilot CLI) and tools such as `gh` and `git` talk to their own services under their own terms, whether or not you use collie. collie does not change what they send. For example, answering a Claude Code tip's feedback question from the phone makes Claude Code send Anthropic the same feedback signal it would send if you answered on the machine.

## Children

collie is a developer tool and is not directed at children.

## Changes

Changes to this policy are published on this page with a new date. If a future version of collie adds a service that handles your data, such as a push relay for an App Store release, this policy will describe it before that version is released.

## Contact

Questions about this policy: open an issue on [GitHub](https://github.com/rbstp/collie/issues), or see [Support](support.md).
