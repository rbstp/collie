# Public submission preparation

Offline preparation for [#359](https://github.com/rbstp/collie/issues/359), based on the reviewed source at `190c6ed`. This is not a submission record. No public TestFlight link, App Review message, App Store Connect change or production deployment is authorized by this document.

## Release gates

The publisher relay is deployed on Workers Paid and serves `https://collie-push.rbstp.dev` with production APNs; released builds deliver through it, and Live Activity update and end through the relay are confirmed on a physical iPhone ([Relay](relay.md)). It runs the single crypto actor with a 250 ms CPU limit because the original direct-Worker verifier failed the [Free-plan CPU gate](reviews/relay-free-tier.md). Observability stays disabled and workers.dev and preview URLs are off. Self-built apps keep direct APNs. **Public capacity is not yet measured:** keep the existing SQL quotas, start the public link at the cap in [App Review notes](app-review.md#public-link-capacity) and raise it only on measured usage.

Record the exact build, source SHA, deployment version, operator and evidence link for each gate below. An upload, installation, successful APNs response and user acceptance are separate results.

| Gate | Evidence needed before submission |
|---|---|
| Paid runtime and capacity | Cold/warm enrollment, assertion, signing, send and hostile-input CPU; actual account-wide Worker/Durable Object requests, CPU/duration and D1 read/write/storage costs; concurrency and bounded cleanup; approved enrollment/device capacity |
| Production trust | Current physical-device and TestFlight App Attest under the publisher App ID; reject development/simulator proofs; actual profile/signature entitlements and retained replay/revocation state |
| Push privacy and delivery | Every alert, background clear and Live Activity update/end; safe extension/widget failure; locked/pre-first-unlock behavior; real production APNs receipt on the submitted build |
| Compatibility and recovery | Both supported app/daemon upgrade orders, route renewal/revocation and 1027/429/D1 outage behavior; direct sessions remain usable |
| Personal embedded connectivity ([#389](https://github.com/rbstp/collie/issues/389)) | Independently reviewed same-user pairing/reconnect/background ownership gates; real iPhone and Mac use without an OS VPN profile or standalone Tailscale client, including a Mac with its separate corporate VPN active; expiry/sign-in recovery and revocation. Static/mocked/Simulator checks do not satisfy physical coexistence acceptance |
| Review access | Release sample on simulator and signed iPhone, reset/exit without real effects, real-hardware video; App Review decision about the sample and Tailscale login |
| Desktop installation | Published signed/notarized macOS downloads and Linux packages with tested requirements/install/update instructions; verify the exact linked assets |
| Disclosures and operation | Published policy matches actual deployed retention/logging; App Privacy and privacy-manifest review; tested incident and credential rotation procedures |
| Export and metadata | Existing [export checklist](release.md#export-compliance), France restriction and external-tester caveat; actual screenshots, age-rating questionnaire, contact details and complete metadata |

Do not close #359 until its actual distribution and review gates are complete. Preparation PRs use `Refs #359`.

## Metadata draft

Review these values against the final submitted build. This draft has not been entered into App Store Connect.

| Field | Draft |
|---|---|
| Name | Collie |
| Subtitle | Your coding agents, on iPhone |
| Category | Developer Tools |
| Price | Free |
| Support URL | <https://collie.rbstp.dev/support.html> |
| Privacy URL | <https://collie.rbstp.dev/privacy.html> |
| Marketing URL | <https://collie.rbstp.dev/> |
| Keywords | coding,agents,terminal,developer,approvals,remote |

Description:

> Follow coding agents on your own Mac or Linux machine from iPhone. Read conversations and terminal output, send prompts, review Git changes and respond to pending approvals. Keep track of work with notifications, Live Activities and Apple Watch.
>
> Collie connects to your machines through your Tailscale account. Real use requires signing in inside Collie with the same personal Tailscale account on both devices, plus herdr and collied on a supported machine. No OS VPN profile or standalone Tailscale app is required. Compatible personal tailnet policies need no edits; restrictive administrator policies may still need a TCP 8457 grant. Explore the built-in sample workspace without an account or computer. Sample actions are simulated.
>
> Commands, conversations and files travel between your devices over an authenticated encrypted connection. Notification content is encrypted on your machine and decrypted on your phone. The push service handles delivery metadata as described in the privacy policy.

Use this description only after real notifications and the publisher relay pass the release gates. Do not advertise unfinished delivery as available. Complete Apple's [age-rating questionnaire](https://developer.apple.com/help/app-store-connect/manage-app-information/set-an-app-age-rating/) against the actual capabilities, including access to agent-generated/user-controlled content, external links and terminal behavior; do not invent an age rating here. The publisher must supply the private review contact and feedback email before submission.

Capture final-build screenshots of Agents, Chat, Terminal, an approval, Usage and the sample banner. Existing site assets are candidates, not evidence of the submitted binary. Scrub paths, account data, tokens and personal machine names; verify current [screenshot specifications](https://developer.apple.com/help/app-store-connect/reference/app-information/screenshot-specifications/).

## Review notes

The review notes, the guideline 4.8 question, the real-device video and the submission checklist are in [App Review notes](app-review.md).

## Guideline assessment

Checked October 16, 2026 against Apple's [guidelines](https://developer.apple.com/app-store/review/guidelines/) and [update summary](https://developer.apple.com/news/?id=92th7e5i). This is preparation, not an App Review determination.

- **1.2:** Collie sends prompts/files to private coding agents and displays user/agent content. The revised scope includes AI content creation apps; private use without a public feed is not an automatic exemption. Assess actual filtering, content reporting/response, abusive-user blocking and contact safeguards; ask App Review how these apply where uncertain. Authenticated pairing/revocation are security controls, not content-moderation evidence. This assessment authorizes no cloud scanning or other data-processing change.
- **2.3.1 and 2.5.19:** Describe all new features, functionality and product changes specifically and disclose all shipped capabilities and actual availability. The sample must not conceal real remote controls. Reconcile compiled but unavailable relay paths before submission; this draft approves neither dormant behavior nor undisclosed activation after review.
- **2.3.14, if creative assets are used:** Follow Apple's [asset guidance](https://developer.apple.com/app-store/asset-best-practices/): accurate functionality/brand, all-audience presentation, no misleading associations, unverifiable claims or specific/discounted prices. Retain app-in-use screenshots and do not advertise unfinished delivery.

**Separate open decisions:** 2.1 requires full review access and any prior approval needed to substitute a demo for credentials; the sample does not exercise every real backend/device flow. The 4.8 Tailscale login exception remains independently unconfirmed. Supply actual tested-device, external-service and hardware information under the complete-review guidance above.

### Release capability and review access

Verify this record against the exact submitted phone build.

| Capability | Current preparation state and review boundary |
|---|---|
| Visible bundled sample | Welcome/Settings entry, simulated prompts, terminal keys and decisions, reset/exit. Release Simulator acceptance passed; physical iPhone sample acceptance remains open. No real machine action |
| Paired remote controls | Terminal/chat, prompts/files, tasks/worktrees, repository/Git and approvals on confirmed paired computers. Requires Tailscale, its policy, herdr and collied; provide approved review resources, never the owner's personal account/machine |
| Genuine push and attestation | Released builds enroll with App Attest and deliver encrypted alerts and Live Activities through the publisher relay; Live Activity update and end are confirmed on a physical iPhone. Record the same evidence for the exact submitted build. Sample/mock results cannot prove delivery |
| Debug VM and Simulator hooks | Excluded from ordinary Release app/core artifacts; retain the submitted build's exclusion proof. These cannot supply production trust or review credentials |

Use Apple's [review-status](https://developer.apple.com/help/app-review/after-submitting-for-review/review-status) and [submission-contact](https://developer.apple.com/help/app-review/after-submitting-for-review/contact-app-review) guidance after submission. Submitted, approved and released remain separate states.

## App Privacy draft

These are proposed answers for a future relay-enabled build, not the current App Store Connect record. Reconcile the final binary, Tailscale SDK behavior, Cloudflare configuration and every third-party practice before entering them. Apple's [App Privacy definitions](https://developer.apple.com/app-store/app-privacy-details/) include information retained beyond serving a request, even when used only for functionality; linkage through a device counts. Encryption and no account do not justify a blanket Data Not Collected answer.

| Data and proposed category | Purpose and linkage | Source/retention check |
|---|---|---|
| Device/activity destinations and installation/key/pairing/route identifiers: Device ID | App Functionality; linked through installation/device; not tracking | Stored in D1 while routes/grants and retention state remain |
| App Attest public key, receipt, environment and integrity signals: Other Data Types, with device identifiers also covered above | App Functionality, including authentication/fraud prevention; linked; not tracking | D1 verified attestation record; Apple processes attestation independently |
| Per-installation send/routine/control counters: Other Usage Data | App Functionality for quotas/security; linked; not tracking | Daily buckets reset lazily; current row remains with installation state |
| Source IPs, request timing and sizes | Classify any retained data according to actual use; no location inference intended | Worker uses IP for transient abuse limiting, without D1 IP/request-event storage. Confirm Cloudflare's retained service/security data before final answers; observability disabled is not proof the provider retains nothing |
| Crash/feedback and support submissions | Reassess Diagnostics/Customer Support for what the developer actually receives and retains | Review optional Apple/TestFlight reports and chosen support flow; do not infer exemption from optional use alone |

No advertising, advertising identifier or cross-company advertising tracking is implemented. Private command/chat/file plaintext is not accessible to the publisher relay; its transient ciphertext is not persisted. Do not label transient relay payload handling as retained readable user content. Tailscale account/device processing and optional support/diagnostic collection still need the final third-party audit. The checked-in privacy manifest currently has an empty collected-data array; review it alongside App Privacy before a relay-enabled binary ships, without confusing required-reason API declarations with the store label.

### Actual retention and logging constraints

The #353 source stores verified keys/receipts, counters, replay state, revocation epochs, route relationships and raw destinations. It stores no notification keys or payloads. Source IPs are limiter keys, not D1 fields. `relay/cloudflare.config.ts` disables observability and contains no production database/domain or cleanup schedule.

The code sets an enrollment retention deadline 37 days after enrollment and extends it to at least the latest issued grant expiry plus seven days. Grants last 30 days. Replacing a route deletes its old destination in the same transaction. Otherwise, expiry/revocation does not immediately erase a route: cleanup selects routes only after expiry plus seven days, in batches of at most 64, then removes eligible parent state. Revocation does not shorten the attestation deadline. The private runtime has an hourly cleanup schedule, but native execution and its backlog bound remain unverified, so no live automatic-deletion deadline is established.

Before enabling enrollment, verify native cleanup execution and measure cadence/backlog, state the resulting active-database deletion bound and publish it in the policy. Do not repeat the plan's deletion-within-seven-days promise: this implementation only starts deletion after that delay. Account for provider backups separately: [D1 Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/) is always on and can restore up to 30 days of history on the active Workers Paid plan. Removing a live record does not immediately remove it from recoverable history. Restoring retained authorization state can resurrect revoked grants and rolled-back counters, so the [rollback procedure](relay-runbook.md#rollback) forbids an ordinary D1 restore.

Production observability remains disabled. No request bodies, Authorization values, destinations, full URLs, attestation objects or secret values may be logged. Cloudflare's own service/security processing is covered by its [privacy policy](https://www.cloudflare.com/privacypolicy/); verify the actual account products and retention rather than promising universal zero logs.

## Operator runbook

The relay's operating and incident procedures are in the [relay runbook](relay-runbook.md). The distribution sequence and the public link's capacity cap are in [App Review notes](app-review.md#submission-checklist).
