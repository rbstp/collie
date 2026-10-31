# App Review notes

Material for Beta App Review and App Store review of the first public release ([#359](https://github.com/rbstp/collie/issues/359)). Check every statement against the exact build before pasting it, and keep the review contact, phone number and reviewer credentials out of this repository.

## Review notes

Paste into App Review Information, Notes (and, for TestFlight, Beta App Review Information, Review Notes):

> Collie is a client for coding agents (Claude Code, Codex, Copilot CLI) running on the user's own Mac or Linux machine, which the phone reaches through the user's own Tailscale account. Tailscale runs inside the app: Collie installs no VPN profile, needs no Tailscale app and does not change a Mac's separate corporate Tailscale account. A restrictive tailnet policy may need an administrator to grant TCP port 8457.
>
> To review without an account or a computer, tap Explore sample workspace on the Welcome screen or in Settings. A bar at the bottom reads "Sample workspace" and "Simulated actions only". Open Agents and an agent's Chat and Terminal, send a prompt, use a terminal key, approve or deny the blocked sample agent, and look at Usage and Machines. Reset sample workspace and Exit sample workspace are in the sample's Settings tab. Sample data is bundled; no sample action leaves the device.
>
> Real use needs herdr (which runs the agents) and the free collied daemon on the user's Mac or Linux machine (https://collie.rbstp.dev/install/requirements.html), the same personal Tailscale account in Collie and on that machine, and pairing: the user scans the machine's QR code in Machines, Add machine, and confirms on the machine. A paired phone sends prompts, terminal input and files, starts tasks and worktrees, clones repositories, shows Git changes and pull requests, and approves or denies agent actions, all on the user's own machine as that user; the agents use the AI services configured there. Collie sells no account, subscription or AI access. Optional desktop companions (the CollieBar menu bar app for macOS, the CollieTray widget for Linux) are on GitHub Releases.
>
> Collie has no account of its own: users sign in directly to their own Tailscale account inside Collie to reach their own machine. Tailscale supplies the connection; the content comes from the user's agents. We believe this is the guideline 4.8 case of a client for a specific third-party service; please confirm. Since we cannot give reviewers a personal machine or tailnet, the built-in sample workspace stands in for a demo account, as App Review approved beforehand (date to fill in); the attached video shows the real workflows.
>
> Background use: remote notifications and Live Activities only. The user's machine encrypts notification content, Collie's push relay forwards it to Apple, and the phone decrypts it. Pushes carry approval, finished-agent and pull request alerts, silent clears of approvals answered elsewhere, and Live Activity updates for followed agents. App Attest authenticates the app to the relay. A push never approves anything: an approval goes back over the authenticated connection, after Face ID or the passcode.

The sample does not show pairing, background pushes, Live Activities or App Attest; the video covers them.

## Guideline 4.8 and review access

**4.8: unconfirmed.** The notes ask App Review to confirm that Collie, as a client of a specific third-party service, needs no equivalent login option; Tailscale supplies connectivity, not content, so the fit is App Review's call. Record the answer here, with its date and channel, before relying on it.

**2.1(a): prior approval required, not yet obtained.** The sample may replace a demo account only with Apple's prior approval, and only when no demo account can be provided, as here. Record it here like the 4.8 answer. Live access would need a publisher-operated demo tailnet and machine, a separate, open decision; never offer the owner's personal account, machine or tailnet.

## Real-device video

Record one continuous take on a physical iPhone with the exact submitted build, a Mac with the matching collied release, a scratch herdr workspace and a demo repository. Blur or avoid personal names, the Tailscale account, paths and machine names. Note the iPhone model, iOS version, app build and collied version.

1. On Welcome, show Explore sample workspace.
2. Sign in to Tailscale inside Collie.
3. On the Mac, run `collied pair`; scan the QR code in Machines, Add machine; show the login and device in the Mac's confirmation and confirm.
4. In Agents, open the scratch agent; send a prompt in Chat and type in Terminal.
5. Choose Follow on Lock Screen, lock the iPhone and show the Live Activity change state as the agent works.
6. Trigger an approval: show the notification arrive on the lock screen with its decrypted text, open it, approve with Face ID and show the agent continue on the Mac.
7. Show the done alert and the Live Activity ending when the agent finishes.
8. Remove the machine in Machines and show the Mac no longer lists the phone (`collied peers list`, or Paired Phones in CollieBar).

## Submission checklist

Manual App Store Connect steps, in order; record uploaded, beta-approved, submitted, approved and released as separate states. The App Store submission (0.1.9, build 284, 2026-10-24) went first; steps 6 and 7 wait because a public link cannot exclude France.

1. **Build:** the exact TestFlight build is processed, installable and at or above the relay's [minimum bundle version](relay-runbook.md#refuse-old-app-builds), and the same-tag desktop release is published and verified ([Release](release.md#cutting-a-release)).
2. **Screenshots:** the submitted commit's set for every iPhone size App Store Connect lists and Apple Watch, made as [Release](release.md#app-store-screenshots) describes (sizes checked against [Apple's specifications](https://developer.apple.com/help/app-store-connect/reference/app-information/screenshot-specifications/) on 2026-10-20); no iPad set (iPhone only); no personal paths, account data, tokens or machine names.
3. **Review information:** contact name, phone and email, the notes above and the video, with the demo account empty. Before submitting, get App Review's prior approval of the sample, record it above and fill in its date in the notes; until then, the submission waits.
4. **Export compliance**, as [Release](release.md#export-compliance) describes: every country or region except France (owner decision, 2026-10-20); invite no external testers in France, since TestFlight ignores App Store availability.
5. **App Privacy, listing text and age rating:** done; recheck against the final build and the privacy page ([#541](https://github.com/rbstp/collie/issues/541)).
6. **Beta App Review:** in TestFlight, fill in Test Information (beta description, feedback email, these notes) and What to Test from [iOS release notes](ios-release-notes.md); create an external group, add the build and submit it.
7. **Public TestFlight link**, after beta approval and only once the France caveat is settled (Apple's TestFlight help, read 2026-10-20, has no country setting for external testing), with a tester limit no higher than the [cap](#public-link-capacity).
8. **App Store review**, a separate submission after the beta is accepted: the same or a later accepted build, free price, step 4's availability, step 3's review information, and manual release so approval and release stay separate.

## Public link capacity

The cap comes from the relay's daily limits, not Workers Paid's inclusions ([Relay](relay.md#atomic-quotas-and-storage-ownership)): 20,000 push attempts across all phones, 1,000 per phone and 30 new enrollments. The [planning case](app-store.md#free-tier-budget-and-capacity) of 624 attempts a day per active phone gives `floor(20,000 / 624) = 32` phones; the plan starts at **30 daily active phones**.

The public link counts testers, not installations: every tester iPhone and reinstall enrolls separately, and the owner's and internal testers' phones share the budget. So:

- **Tester limit = 30 minus the installations already active**, the seven-day maximum of the daily `phones` value from the [runbook](relay-runbook.md#monitoring) query. With only the owner's iPhone active, that is 29.
- At most 30 phones enroll per UTC day, reinstalls included, so a fast-filling link leaves some phones waiting until 00:00 UTC to register.

No production per-phone usage is recorded yet. Before raising the limit, record seven days of end-of-day `global_usage` totals and the query's `phones` and `busiest` values. Raise it only while the seven-day peak stays under 14,000 (70% of 20,000) and `daily_quota` rejections under 1% of sends, [Relay](relay.md#operations)'s review points. At the planning case 30 phones already use 94% of the budget, so only measured usage below 624 per phone justifies more, up to about `floor(14,000 / measured attempts per active phone)` phones. Raising the global caps instead means a migration and a cost review.
