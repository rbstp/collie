# App Review notes

Material for Beta App Review and App Store review of the first public release ([#359](https://github.com/rbstp/collie/issues/359)). Nothing here has been sent to Apple. Check every statement against the exact build before pasting it into App Store Connect. Keep the review contact, phone number and any reviewer credentials out of this repository.

## Review notes

Paste into App Review Information, Notes (and, for TestFlight, Beta App Review Information, Review Notes):

> Collie is a client for coding agents (Claude Code, Codex, Copilot CLI) that run on the user's own Mac or Linux machine. The phone reaches that machine through the user's own Tailscale account. Collie runs Tailscale inside the app: it installs no VPN profile and does not need the Tailscale app.
>
> To review without an account or a computer, tap Explore sample workspace on the Welcome screen. It is also in Settings. A bar at the bottom reads "Sample workspace" and "Simulated actions only". Open Agents and an agent's Chat and Terminal, send a prompt, use a terminal key, approve or deny the blocked sample agent, and look at Usage and Machines. The sample's Settings tab has Reset sample workspace and Exit sample workspace. Sample data is bundled with the app; no sample action leaves the device.
>
> Real use needs three things: herdr, which runs the agents, and the free collied daemon on the user's Mac or Linux machine (https://collie.rbstp.dev/install/requirements.html), a sign-in to the same personal Tailscale account in Collie and on that machine, and pairing. To pair, the machine shows a QR code, the user scans it in Machines, Add machine, and confirms on the machine. A paired phone sends prompts, terminal input and files, starts tasks and worktrees, clones repositories, shows Git changes and pull requests, and approves or denies agent actions, all on the user's own machine as that user. The agents use the AI services already configured on the machine. Collie sells no account, subscription or AI access. The CollieBar menu bar app for macOS and the CollieTray widget for Linux are optional desktop companions, published on GitHub Releases.
>
> Collie has no account of its own. Its only sign-in is Tailscale, because Tailscale carries every connection between the phone and the user's machine. We believe this is the guideline 4.8 case of an app that is a client for a specific third-party service; please tell us if you see it differently. We cannot give reviewers access to a personal machine or tailnet, so the sample workspace and the attached video show the real workflows.
>
> Background use: remote notifications and Live Activities only. The user's machine encrypts the notification content, Collie's push relay forwards it to Apple, and the phone decrypts it. Pushes carry approval alerts, alerts when an agent finishes, silent pushes that clear an approval answered elsewhere, and Live Activity updates for agents the user follows. App Attest authenticates the app to the relay. A push never approves anything: an approval goes back over the authenticated connection, after Face ID or the passcode.

The sample does not show pairing, background pushes, Live Activities or App Attest; the video covers them.

## Guideline 4.8 and review access

**Unconfirmed.** Guideline 4.8 asks apps that use a third-party login for the user's primary account to offer an equivalent login option, with exceptions that include an app that is a client for a specific third-party service. Tailscale appears to fit, since it supplies the connection to the user's own machine, but App Review decides. The notes above ask the question; record the answer here, with its date and channel, before relying on it.

Guideline 2.1 requires full review access. The sample is the proposed substitute for credentials, and Apple may require prior approval for that. If App Review asks for live access instead, there is nothing to offer yet: it would need a publisher-operated demo tailnet and machine, which is a separate decision. Never offer the owner's personal account, machine or tailnet.

See [Apple's guidelines](https://developer.apple.com/app-store/review/guidelines/), [review information](https://developer.apple.com/help/app-review/before-submitting-for-review/complete-review/) and the [guideline assessment](public-submission.md#guideline-assessment).

## Real-device video

Record one continuous take on a physical iPhone running the exact submitted build, with a Mac running the matching collied release. Use a scratch herdr workspace and a demo repository. Blur or avoid personal names, the Tailscale account, paths and machine names. Note the iPhone model, iOS version, app build and collied version for the submission.

1. Launch the app on Welcome and show Explore sample workspace.
2. Sign in to Tailscale inside Collie.
3. On the Mac, run `collied pair`; scan the QR code in Machines, Add machine; show the login and device on the Mac's confirmation and confirm.
4. In Agents, open the scratch agent; send a prompt in Chat and type in Terminal.
5. Follow the agent, lock the iPhone and show its Live Activity change state as the agent works.
6. Trigger an approval: show the notification arriving on the lock screen with its decrypted text, open it, approve with Face ID, and show the agent continue on the Mac.
7. Show the done alert and the Live Activity ending when the agent finishes.
8. Remove the machine in Machines and show that the Mac no longer lists the phone.

## Submission checklist

What remains manual in App Store Connect, in order. Record each result separately: uploaded, beta-approved, submitted, approved and released are different states.

1. **Build.** Confirm the exact TestFlight build is processed and installable, its bundle version is allowed by the relay ([Allow a new app build](relay-runbook.md#allow-a-new-app-build)), and the desktop release with the same tag is published and verified ([Release](release.md#cutting-a-release)).
2. **Screenshots**, from that build, checked against [Apple's specifications](https://developer.apple.com/help/app-store-connect/reference/app-information/screenshot-specifications/) on 2026-10-20. Required: iPhone with Dynamic Island (medium display), 1206 x 2622 or 1179 x 2556 portrait, for example the iPhone 18 Pro Simulator; and, because the app ships a watch app, Apple Watch at one size for every localization, such as 416 x 496 (Series 12, the size the documentation screenshots use). Optional: the large display set, 1320 x 2868 (iPhone 18 Pro Max). No iPad set: the app is iPhone only. No alpha channel. iPhone Duo screenshots become required from April 2027 for builds made with the iOS 27.1 SDK or later. Show Agents, Chat, Terminal, an approval, Usage and the sample bar, scrubbed as [Public submission](public-submission.md#metadata-draft) describes.
3. **Review information.** Contact name, phone and email; no demo account, with the notes pointing to the sample; the notes above; the video as an attachment.
4. **Export compliance**, as [Release](release.md#export-compliance) describes. `ITSAppUsesNonExemptEncryption` is `false`, so builds need no per-build answer. For the App Store, choose specific countries and regions without France, and answer any encryption question with standard algorithms and France no. External TestFlight is not limited by App Store availability: invite no testers located in France until that caveat is settled.
5. **App Privacy, listing text and age rating.** Done; recheck them against the final build and the privacy page ([#541](https://github.com/rbstp/collie/issues/541)).
6. **Beta App Review.** In TestFlight, fill in Test Information (beta description, feedback email, Beta App Review Information with these notes) and What to Test from [iOS release notes](ios-release-notes.md). Create an external group, add the build and submit it.
7. **Public TestFlight link.** After beta approval, enable the group's public link with a tester limit no higher than the cap below. Its criteria filter devices and OS versions only; it cannot exclude France.
8. **App Store review**, as a separate submission after the beta is accepted: select the same or a later accepted build, set the price to free, the availability from step 4, the review information from step 3, and manual release so that approval and release stay separate.

## Public link capacity

The cap comes from the relay's daily limits in [Relay](relay.md#atomic-quotas-and-storage-ownership): 20,000 push attempts across all phones (16,000 routine Live Activity updates), 1,000 per phone (700 routine), 1,000 control operations and 30 new enrollments. At these caps the account stays well inside Workers Paid's inclusions, so the quotas, not Cloudflare, bound capacity.

The documented planning case is 624 attempts and 12 controls a day per active phone: 40 approvals with their clears, 40 finished turns, 20 pull request alerts, one agent followed for eight hours and four Live Activity ends. That gives `floor(20,000 / 624) = 32` phones, and [the plan](app-store.md#free-tier-budget-and-capacity) starts at **30 daily active phones**. If every phone used its full 1,000 attempts, 20 phones would exhaust the global budget; the per-phone cap keeps one heavy user to 5% of it.

Apple's public link counts testers, not installations. Each iPhone a tester installs on enrolls separately, a reinstall enrolls again, and the owner's and internal testers' phones share the same budget. So:

- **Tester limit = 30 minus the installations already active**, counted with the active-phones query in the [runbook](relay-runbook.md#monitoring). With only the owner's iPhone active, that is 29.
- Enrollment is capped at 30 phones a UTC day, reinstalls included, so when a link fills fast some phones wait until 00:00 UTC to register.

No production per-phone usage is recorded yet. Measure it before raising the limit: for seven days, record the end-of-day `global_usage` total and the active-phones query's phone count and busiest phone. Raise the limit only while the seven-day peak stays under 70% of 20,000 (14,000) and `daily_quota` rejections stay under 1% of sends, which are [Relay](relay.md#operations)'s review points. At the planning case, 30 phones already use 94% of the budget, so the model leaves no room above 30: only measured usage below 624 a phone can justify more, at roughly `floor(14,000 / measured attempts per active phone)` phones in total. Raising the global caps instead is a code change to the migrations with a cost review.
