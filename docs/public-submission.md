# Public submission preparation

Preparation for [#359](https://github.com/rbstp/collie/issues/359), based on the reviewed source at `190c6ed`: the release gates, the listing and privacy drafts, and the guideline assessment. #359 records the submission results. [App Review notes](app-review.md) holds the review notes, the guideline 4.8 question, the real-device video, the submission checklist and the public link's capacity cap; the [relay runbook](relay-runbook.md) holds the operating and incident procedures.

## Release gates

The publisher relay runs on Workers Paid at `https://collie-push.rbstp.dev` with production APNs, and released builds deliver through it; Live Activity update and end through the relay are confirmed on a physical iPhone ([Relay](relay.md)). It runs the single crypto actor with a 250 ms CPU limit because the direct-Worker verifier failed the [Free-plan CPU gate](reviews/relay-free-tier.md). Observability, workers.dev and preview URLs are off. Self-built apps keep direct APNs. **Public capacity is not yet measured:** keep the SQL quotas, start the public link at the [capacity cap](app-review.md#public-link-capacity) and raise it only on measured usage.

For each gate, record the exact build, source SHA, deployment version, operator and evidence link. An upload, an installation, a successful APNs response and user acceptance are separate results.

| Gate | Evidence needed before submission |
|---|---|
| Paid runtime and capacity | Cold and warm CPU for enrollment, assertions, signing, sends and hostile input; account-wide Worker and Durable Object requests, CPU and duration, and D1 reads, writes and storage; concurrency and bounded cleanup; approved enrollment and device capacity |
| Production trust | Physical-device and TestFlight App Attest under the publisher App ID; rejected development and simulator proofs; actual profile and signature entitlements; retained replay and revocation state |
| Push privacy and delivery | Every alert, background clear and Live Activity update and end; safe extension and widget failure; locked and pre-first-unlock behavior; real production APNs receipt on the submitted build |
| Compatibility and recovery | Both app and daemon upgrade orders, route renewal and revocation, and 1027, 429 and D1 outages, with direct sessions still usable |
| Personal embedded connectivity ([#389](https://github.com/rbstp/collie/issues/389)) | Independently reviewed same-user ownership checks for pairing, reconnect and background use; real iPhone and Mac use with no OS VPN profile or standalone Tailscale client, including a Mac with its separate corporate VPN active; expiry, sign-in recovery and revocation. Static, mocked and Simulator checks do not establish physical coexistence |
| Review access | Release sample on simulator and signed iPhone, reset and exit without real effects, real-hardware video, App Review's decision on the sample and the Tailscale login |
| Desktop installation | Published signed and notarized macOS downloads and Linux packages with tested requirements and install and update instructions; the exact linked assets verified |
| Disclosures and operation | Published policy matches deployed retention and logging; App Privacy and privacy-manifest review; tested incident and credential rotation procedures |
| Export and metadata | The [export checklist](release.md#export-compliance) with its France restriction and external-tester caveat; actual screenshots, age rating, contact details and complete metadata |

## Metadata draft

Recheck these values against App Store Connect and the submitted build.

| Field | Draft |
|---|---|
| Name | Collie Herd |
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

Use this description only after notifications through the publisher relay pass the release gates; never advertise unfinished delivery. Answer Apple's [age-rating questionnaire](https://developer.apple.com/help/app-store-connect/manage-app-information/set-an-app-age-rating/) from the actual capabilities, including agent-generated and user-controlled content, external links and terminal behavior; this page sets no rating. The publisher supplies the private review contact and feedback email.

## Guideline assessment

Checked October 16, 2026 against Apple's [guidelines](https://developer.apple.com/app-store/review/guidelines/) and [update summary](https://developer.apple.com/news/?id=92th7e5i). This is preparation, not an App Review determination.

- **1.2:** Collie sends prompts and files to private coding agents and displays user and agent content. The revised scope includes AI content creation apps, and private use without a public feed is no automatic exemption. Assess actual filtering, content reporting and response, abusive-user blocking and contact safeguards, and ask App Review where unsure. Pairing and revocation are security controls, not content moderation. This assessment authorizes no cloud scanning or other change in data processing.
- **2.3.1 and 2.5.19:** Describe new features, functionality and product changes specifically, and disclose every shipped capability and its actual availability. The sample must not conceal real remote controls. Reconcile compiled but unavailable relay paths before submission; this draft approves neither dormant behavior nor undisclosed activation after review.
- **2.3.14, if creative assets are used:** Follow Apple's [asset guidance](https://developer.apple.com/app-store/asset-best-practices/): accurate functionality and brand, all-audience presentation, no misleading associations, unverifiable claims or specific or discounted prices. Keep app-in-use screenshots.

**Separate open decisions:** 2.1 requires full review access, and a demo in place of credentials needs Apple's prior approval; the sample does not exercise every real backend or device flow. The 4.8 Tailscale login exception is still unconfirmed. [App Review notes](app-review.md) tracks both. Supply the tested devices, external services and hardware that Apple's [review information](https://developer.apple.com/help/app-review/before-submitting-for-review/complete-review/) asks for.

### Release capability and review access

Verify this record against the exact submitted build.

| Capability | Preparation state and review boundary |
|---|---|
| Visible bundled sample | Welcome and Settings entry, simulated prompts, terminal keys and decisions, reset and exit, no real machine action. Release Simulator acceptance passed; physical iPhone sample acceptance remains open |
| Paired remote controls | Terminal and chat, prompts and files, tasks and worktrees, repositories and Git, and approvals on confirmed paired computers. Requires Tailscale, its policy, herdr and collied; provide approved review resources, never the owner's personal account or machine |
| Genuine push and attestation | Released builds enroll with App Attest and deliver encrypted alerts and Live Activities through the relay; Live Activity update and end are confirmed on a physical iPhone. Record the same evidence for the submitted build; sample or mock results cannot prove delivery |
| Debug VM and Simulator hooks | Excluded from Release app and core artifacts; keep the submitted build's exclusion proof. They supply no production trust or review credentials |

After submission, follow Apple's [review-status](https://developer.apple.com/help/app-review/after-submitting-for-review/review-status) and [submission-contact](https://developer.apple.com/help/app-review/after-submitting-for-review/contact-app-review) guidance. Submitted, approved and released are separate states.

## App Privacy draft

Draft answers for the relay-enabled build. The answers entered in App Store Connect since ([App Review notes](app-review.md#submission-checklist), step 5) are authoritative; recheck them against the final binary, Tailscale SDK behavior, Cloudflare configuration and every third-party practice. Apple's [App Privacy definitions](https://developer.apple.com/app-store/app-privacy-details/) count data retained beyond serving a request, even for functionality only, and linkage through a device. Encryption and having no account do not justify a blanket Data Not Collected.

| Data and proposed category | Purpose and linkage | Source and retention check |
|---|---|---|
| Device and activity destinations; installation, key, pairing and route identifiers: Device ID | App Functionality; linked through installation and device; not tracking | D1, while routes, grants and retention state remain |
| App Attest public key, receipt, environment and integrity signals: Other Data Types (device identifiers covered above) | App Functionality, including authentication and fraud prevention; linked; not tracking | D1 attestation record; Apple processes attestation independently |
| Per-installation send, routine and control counters: Other Usage Data | App Functionality for quotas and security; linked; not tracking | Daily buckets reset lazily; the current row stays with installation state |
| Source IPs, request timing and sizes | Classify any retained data by actual use; no location inference intended | IPs serve only transient abuse limits; D1 stores no IPs or request events. Confirm what Cloudflare retains for service and security; disabled observability does not prove it retains nothing |
| Crash, feedback and support submissions | Reassess Diagnostics and Customer Support for what the developer receives and retains | Review optional Apple and TestFlight reports and the support flow; optional use alone is no exemption |

The app has no advertising, advertising identifier or cross-company tracking. The relay cannot read private plaintext and does not persist its transient ciphertext, so do not label relay payload handling as retained readable content. Tailscale account and device processing and optional support and diagnostic collection still need the final third-party audit. The privacy manifest (`Collie/Resources/PrivacyInfo.xcprivacy`) declares no collected data types; review it with App Privacy, without confusing its required-reason API declarations with the store label.

### Actual retention and logging constraints

D1 holds verified keys and receipts, counters, replay state, revocation epochs, route relationships and raw destinations, but no notification keys or payloads; source IPs are rate-limiter keys, not D1 fields. `relay/cloudflare.config.ts` disables observability and declares the hourly cleanup Cron trigger. [Retention and cleanup](relay.md#retention-and-cleanup) sets the deadlines. Replacing a route deletes its old destination at once. An expired or revoked route becomes eligible for deletion seven days after its grant expiry, and the installation's other records seven days after its last grant expiry but no earlier than 37 days after enrollment, a deadline revocation does not shorten. Cleanup deletes them within 14 days of the last grant expiry, the bound the privacy policy publishes. Do not repeat the plan's deletion-within-seven-days promise, since deletion only starts after that delay. Confirm the trigger runs on the deployment with the runbook's `cleanup_backlog` count.

[D1 Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/) is always on and can restore 30 days of history on Workers Paid, so a deleted record stays recoverable for up to 30 days. Restoring that state could resurrect revoked grants and rolled-back counters, so the [rollback procedure](relay-runbook.md#rollback) forbids an ordinary D1 restore.

Production observability stays disabled. Never log request bodies, Authorization values, destinations, full URLs, attestation objects or secret values. Cloudflare's own service and security processing falls under its [privacy policy](https://www.cloudflare.com/privacypolicy/); verify the account's actual products and retention rather than promising zero logs.
