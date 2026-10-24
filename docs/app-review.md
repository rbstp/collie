# App Review notes

Material for Beta App Review and App Store review of the first public release ([#359](https://github.com/rbstp/collie/issues/359)). Nothing here has been sent to Apple. Check every statement against the exact build before pasting it into App Store Connect. Keep the review contact, phone number and any reviewer credentials out of this repository.

## Review notes

Paste into App Review Information, Notes (and, for TestFlight, Beta App Review Information, Review Notes):

> Collie is a client for coding agents (Claude Code, Codex, Copilot CLI) that run on the user's own Mac or Linux machine. The phone reaches that machine through the user's own Tailscale account. Collie runs Tailscale inside the app: it installs no VPN profile, does not need the Tailscale app and does not change a Mac's separate corporate Tailscale account. A restrictive tailnet policy may need an administrator to grant TCP port 8457.
>
> To review without an account or a computer, tap Explore sample workspace on the Welcome screen. It is also in Settings. A bar at the bottom reads "Sample workspace" and "Simulated actions only". Open Agents and an agent's Chat and Terminal, send a prompt, use a terminal key, approve or deny the blocked sample agent, and look at Usage and Machines. The sample's Settings tab has Reset sample workspace and Exit sample workspace. Sample data is bundled with the app; no sample action leaves the device.
>
> Real use needs three things: herdr, which runs the agents, and the free collied daemon on the user's Mac or Linux machine (https://collie.rbstp.dev/install/requirements.html), a sign-in to the same personal Tailscale account in Collie and on that machine, and pairing. To pair, the machine shows a QR code, the user scans it in Machines, Add machine, and confirms on the machine. A paired phone sends prompts, terminal input and files, starts tasks and worktrees, clones repositories, shows Git changes and pull requests, and approves or denies agent actions, all on the user's own machine as that user. The agents use the AI services already configured on the machine. Collie sells no account, subscription or AI access. The CollieBar menu bar app for macOS and the CollieTray widget for Linux are optional desktop companions, published on GitHub Releases.
>
> Collie has no account of its own. Users sign in directly to their own Tailscale account inside Collie to reach their own machine; Tailscale supplies the connection, not the content, which comes from the user's agents. We believe this is the guideline 4.8 case of an app that is a client for a specific third-party service, but please confirm it. We cannot give reviewers access to a personal machine or tailnet. As App Review approved beforehand (date to fill in), the built-in sample workspace stands in for a demo account; the attached video shows the real workflows.
>
> Background use: remote notifications and Live Activities only. The user's machine encrypts the notification content, Collie's push relay forwards it to Apple, and the phone decrypts it. Pushes carry approval alerts, alerts when an agent finishes, silent pushes that clear an approval answered elsewhere, and Live Activity updates for agents the user follows. App Attest authenticates the app to the relay. A push never approves anything: an approval goes back over the authenticated connection, after Face ID or the passcode.

The sample does not show pairing, background pushes, Live Activities or App Attest; the video covers them.

## Guideline 4.8 and review access

**Unconfirmed.** Guideline 4.8 asks apps that use a third-party login for the user's primary account to offer an equivalent login option. Its exceptions include an app that is a client for a specific third-party service, where users sign in to that account directly to access their content. Tailscale supplies connectivity to the user's own machine, not the content itself, so the fit is a question for App Review, not an assumption. The notes above ask the question; record the answer here, with its date and channel, before relying on it.

Guideline 2.1(a) requires full review access, normally through a demo account. A demo mode in place of a demo account is allowed only with Apple's prior approval, and only when a demo account cannot be provided, here because review would need a personal machine and tailnet. **Prior approval is required and not yet obtained;** record it here, with its date and channel, like the 4.8 answer. If App Review asks for live access instead, there is nothing to offer yet: it would need a publisher-operated demo tailnet and machine, which is a separate decision. Never offer the owner's personal account, machine or tailnet.

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
2. **Screenshots**, from that build, checked against [Apple's specifications](https://developer.apple.com/help/app-store-connect/reference/app-information/screenshot-specifications/) on 2026-10-20. Supply the iPhone with Dynamic Island (large display) set at 1320 x 2868 portrait and the Apple Watch set at 416 x 496 (Series 12), the same size for every localization; `just ios-screenshots` writes both to `target/ios/app-store`. For version 0.1.8, App Store Connect asked this app for iPhone 6.5" Display, 1242 x 2688 or 1284 x 2778; `just screenshot_device="iPhone 14 Plus" ios-screenshots` writes it to `target/ios/app-store/1284x2778` ([Release](release.md#app-store-screenshots)). Apple's page also names iPhone with Dynamic Island (medium display), 1206 x 2622 or 1179 x 2556, as the required size; if App Store Connect still asks for it, capture it on an iPhone 18 Pro Simulator. No iPad set: the app is iPhone only. No alpha channel. iPhone Duo screenshots become required from April 2027 for builds made with the iOS 27.1 SDK or later; leave that size empty until a native iPhone Duo capture is possible. Show Agents, Chat, Terminal, an approval, Usage and the sample bar, scrubbed as [Public submission](public-submission.md#metadata-draft) describes.
3. **Review information.** Contact name, phone and email; the notes above; the video as an attachment. Ask App Review for prior approval of the sample before submitting, and fill in its date in the notes. Leave the demo account empty and point to the sample only once that approval is recorded above, as the 4.8 answer will be; until then, the submission waits.
4. **Export compliance**, as [Release](release.md#export-compliance) describes. `ITSAppUsesNonExemptEncryption` is `false`, so builds need no per-build answer. Decision (owner, 2026-10-20): App Store availability is every country or region except France. Set it under Specific Countries or Regions, as [Manage availability for your app on the App Store](https://developer.apple.com/help/app-store-connect/manage-your-apps-availability/manage-availability-for-your-app-on-the-app-store/) describes, and answer any encryption question with standard algorithms and France no. External TestFlight is not limited by App Store availability: invite no testers located in France until that caveat is settled.
5. **App Privacy, listing text and age rating.** Done; recheck them against the final build and the privacy page ([#541](https://github.com/rbstp/collie/issues/541)).
6. **Beta App Review.** In TestFlight, fill in Test Information (beta description, feedback email, Beta App Review Information with these notes) and What to Test from [iOS release notes](ios-release-notes.md). Create an external group, add the build and submit it.
7. **Public TestFlight link**, only after beta approval and once the France caveat in step 4 is resolved. Apple's TestFlight help, read on 2026-10-20, documents no country or region setting for external testing: public link criteria cover devices and OS versions ([Invite external testers](https://developer.apple.com/help/app-store-connect/test-a-beta-version/invite-external-testers/), [TestFlight overview](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview/)), and the country or region availability in step 4 applies to the App Store only. So a public link cannot exclude France, and an unrestricted link could admit testers located there; do not publish one until the caveat is settled. Then enable the group's public link with a tester limit no higher than the cap below.
8. **App Store review**, as a separate submission after the beta is accepted: select the same or a later accepted build, set the price to free, the availability from step 4, the review information from step 3, and manual release so that approval and release stay separate.

## Public link capacity

The cap comes from the relay's daily limits in [Relay](relay.md#atomic-quotas-and-storage-ownership): 20,000 push attempts across all phones (16,000 routine Live Activity updates), 1,000 per phone (700 routine), 1,000 control operations and 30 new enrollments. At these caps the account stays well inside Workers Paid's inclusions, so the quotas, not Cloudflare, bound capacity.

The documented planning case is 624 attempts and 12 controls a day per active phone: 40 approvals with their clears, 40 finished turns, 20 pull request alerts, one agent followed for eight hours and four Live Activity ends. That gives `floor(20,000 / 624) = 32` phones, and [the plan](app-store.md#free-tier-budget-and-capacity) starts at **30 daily active phones**. If every phone used its full 1,000 attempts, 20 phones would exhaust the global budget; the per-phone cap keeps one heavy user to 5% of it.

Apple's public link counts testers, not installations. Each iPhone a tester installs on enrolls separately, a reinstall enrolls again, and the owner's and internal testers' phones share the same budget. So:

- **Tester limit = 30 minus the installations already active.** Count them as the seven-day maximum of the daily `phones` value, the number of `installation_usage` rows charged that UTC day, from the query in the [runbook](relay-runbook.md#monitoring). With only the owner's iPhone active, that is 29.
- Enrollment is capped at 30 phones a UTC day, reinstalls included, so when a link fills fast some phones wait until 00:00 UTC to register.

No production per-phone usage is recorded yet. Measure it before raising the limit: for seven days, record the end-of-day `global_usage` total and the `installation_usage` query's `phones` and `busiest` values. Raise the limit only while the seven-day peak stays under 70% of 20,000 (14,000) and `daily_quota` rejections stay under 1% of sends, which are [Relay](relay.md#operations)'s review points. At the planning case, 30 phones already use 94% of the budget, so the model leaves no room above 30: only measured usage below 624 a phone can justify more, at roughly `floor(14,000 / measured attempts per active phone)` phones in total. Raising the global caps instead is a code change to the migrations with a cost review.
