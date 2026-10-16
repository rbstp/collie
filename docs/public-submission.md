# Public submission preparation

Offline preparation for [#359](https://github.com/rbstp/collie/issues/359), based on the reviewed source at `190c6ed`. This is not a submission record. No public TestFlight link, App Review message, App Store Connect change or production deployment is authorized by this document.

## Release gates

**Public rollout capacity is UNAPPROVED.** The original direct-Worker verifier failed the [Free-plan CPU gate](reviews/relay-free-tier.md). A separate internal Durable Object prototype subsequently passed a small 42-call controlled check with no errors, a maximum observed public-Worker CPU of 6 ms and actor CPU of 101 ms. That is prototype evidence, not full capacity approval or fresh-hardware validation. Its temporary benchmark cleanup is in progress; it is not a production relay. The plan's 30 daily active phones and SQL limits remain planning bounds. Keep public enrollment and distribution closed until the revised design passes its complete release measurements.

Record the exact build, source SHA, deployment version, operator and evidence link for each gate below. An upload, installation, successful APNs response and user acceptance are separate results.

| Gate | Evidence needed before submission |
|---|---|
| Free-plan runtime and capacity | Cold/warm enrollment, assertion, signing, send and hostile-input CPU; actual account-wide request/read/write costs; concurrency and bounded cleanup; approved enrollment/device capacity |
| Production trust | Current physical-device and TestFlight App Attest under the publisher App ID; reject development/simulator proofs; actual profile/signature entitlements and retained replay/revocation state |
| Push privacy and delivery | Every alert, background clear and Live Activity update/end; safe extension/widget failure; locked/pre-first-unlock behavior; real production APNs receipt on the submitted build |
| Compatibility and recovery | Both supported app/daemon upgrade orders, route renewal/revocation and 1027/429/D1 outage behavior; direct sessions remain usable |
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
> Collie connects to your machines through your Tailscale account. Real use requires Tailscale, its documented tailnet policy, and herdr plus collied on a supported machine. Explore the built-in sample workspace without an account or computer. Sample actions are simulated.
>
> Commands, conversations and files travel between your devices over an authenticated encrypted connection. Notification content is encrypted on your machine and decrypted on your phone. The push service handles delivery metadata as described in the privacy policy.

Use this description only after real notifications and the publisher relay pass the release gates. Do not advertise unfinished delivery as available. Complete Apple's [age-rating questionnaire](https://developer.apple.com/help/app-store-connect/manage-app-information/set-an-app-age-rating/) against the actual capabilities, including access to agent-generated/user-controlled content, external links and terminal behavior; do not invent an age rating here. The publisher must supply the private review contact and feedback email before submission.

Capture final-build screenshots of Agents, Chat, Terminal, an approval, Usage and the sample banner. Existing site assets are candidates, not evidence of the submitted binary. Scrub paths, account data, tokens and personal machine names; verify current [screenshot specifications](https://developer.apple.com/help/app-store-connect/reference/app-information/screenshot-specifications/).

## Review notes draft

The following is a draft for the final validated binary, not an instruction to send it now:

> Collie is a client for coding agents running on a user's own machine. To explore without a personal computer or Tailscale account, choose **Explore sample workspace** on Welcome, or open the same option in Settings. A persistent sample banner identifies simulated data and actions.
>
> In the sample, open Agents and inspect Chat and Terminal. Send a sample prompt, use a terminal key, approve or deny the blocked sample agent, and inspect Usage and Machines. Reset sample workspace is in sample Settings. Exit returns to onboarding or the existing setup. Sample actions run on this device only and cannot operate a real machine.
>
> Real use requires the user's Tailscale account, tailnet policy, herdr and collied. Desktop software and installation requirements are documented at https://collie.rbstp.dev/ . The macOS menu app and Linux Omarchy widget are desktop companions. Collie does not sell accounts or AI access. Agents use the services and credentials configured on the user's machine.
>
> The sample demonstrates interface workflows. It does not reproduce genuine device attestation, pairing, background pushes or Live Activities. The submission's attached real-device video and validation record cover those workflows. Approval decisions always use the authenticated device connection; a push does not authorize a command.

Before sending, attach the actual video and build/device/OS evidence, verify the desktop links, and include any review resources Apple requests. Keep reviewer credentials or private support contacts out of this repository. Never offer access to the user's personal Mac or tailnet.

**Guideline 4.8 treatment is UNCONFIRMED.** Ask App Review whether signing in directly to Tailscale qualifies for the specific-third-party-service client exception, given that Tailscale supplies connectivity rather than the agent content. Obtain any required prior approval to use the sample instead of credentials. Neither the sample, a video nor this draft establishes a waiver. [Apple's guidelines 2.1 and 4.8](https://developer.apple.com/app-store/review/guidelines/) and [review information](https://developer.apple.com/help/app-review/before-submitting-for-review/complete-review/) control the final submission.

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

The code sets an enrollment retention deadline 37 days after enrollment and extends it to at least the latest issued grant expiry plus seven days. Grants last 30 days. Replacing a route deletes its old destination in the same transaction. Otherwise, expiry/revocation does not immediately erase a route: cleanup selects routes only after expiry plus seven days, in batches of at most 64, then removes eligible parent state. Revocation does not shorten the attestation deadline. No live automatic-deletion deadline is established while the handler has no schedule.

Before enabling enrollment, configure and measure cleanup cadence/backlog, state the resulting active-database deletion bound and publish it in the policy. Do not repeat the plan's deletion-within-seven-days promise: this implementation only starts deletion after that delay. Account for provider backups separately: [D1 Free Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/) can restore up to seven days of history. Restoring retained authorization state can resurrect revoked grants and rolled-back counters, so the rollback procedure below forbids an ordinary D1 restore.

Production observability remains disabled. No request bodies, Authorization values, destinations, full URLs, attestation objects or secret values may be logged. Cloudflare's own service/security processing is covered by its [privacy policy](https://www.cloudflare.com/privacypolicy/); verify the actual account products and retention rather than promising universal zero logs.

## Operator runbook

These procedures are for a separately authorized production operator after the gates pass. They do not authorize deployment, secret access, billing or account changes now. Use the confirmed personal account and `cf` workflow in [Relay](relay.md#operations). Record source/deployment IDs and redacted outcome counts, never credentials or payloads.

| Event | Response |
|---|---|
| Routine monitoring | Use aggregate Cloudflare request/error/CPU and D1 read/write/storage metrics, plus aggregate SQL usage counts. Current source has no per-result metrics backend. Do not enable invocation/body/header capture to investigate real traffic. Reproduce detail with synthetic traffic in an isolated reviewed deployment |
| Worker error 1027 | Confirm the account-wide daily Worker allowance and UTC reset using aggregate metrics. Stop invitations and retries that cannot succeed; preserve direct sessions. Do not assume the response is relay JSON or interpret 1027 as an HTTP status |
| 429 | Distinguish relay `rate_limited`/retry delay, `daily_quota`/UTC reset, control `route_limit`, and upstream APNs throttling normalized as temporary failure. Honor bounded backoff and avoid probe storms; an exhausted daily budget waits for reset |
| D1 exhaustion/unavailability | Fail closed on registration/send authorization and quotas. Stop enrollment/invitations, inspect aggregate allowance/error state and cleanup backlog, and preserve existing epochs/counters. Never delete security rows, reset quotas or bypass validation to restore delivery |
| Enrollment stop | Disable invitations/public link first. No enrollment-only operator switch exists in source. If new enrollment must stop immediately, remove the relay's public routing as an explicitly authorized outage; sends/control requests also stop. Keep cleanup active. Restore access only after the cause and capacity are reviewed |
| Pairing/installation revocation | Prefer the phone's authenticated revoke operation. For emergency operator action, use a separately reviewed exact-ID D1 transaction that advances the relevant epoch and marks the matching pairing/attestation revoked. No administrative endpoint exists. Verify grants fail; do not dump destinations or key records |
| APNs credential rotation | Provision the new team/key ID/private key together through private Worker secrets, deploy the reviewed artifact and validate with an owned test destination. Revoke the old Apple key after successful delivery. A compromised key requires emergency revocation; never copy it to users or paste it into diagnostics |
| Send/challenge HMAC rotation | `SEND_HMAC_KEYS` accepts at most two entries; the first signs new grants/challenges. Add the new key first and retain the old verifier only through its last possible 30-day grant expiry, then remove it. Emergency removal invalidates old grants immediately; phones must register again. Verify old challenges/grants fail without logging them |
| Rollback | Stop invitations and select a previously reviewed source/deployment compatible with current schema and protocol. Keep current D1 authorization, counters and quotas. Never restore D1 history as a routine rollback: it can undo revocations and replay protection. If no compatible safe version exists, leave the relay unavailable while direct sessions continue |
| Incident report | Record UTC interval, deployment SHA, aggregate counts/error classes, affected feature and mitigation. Exclude IPs, raw identifiers/destinations, tokens, payloads and secrets. Tell users pushes degraded and direct sessions remain available; APNs acceptance alone never proves display |

Follow the plan's measured upgrade thresholds, with current account traffic included. Do not enable Paid automatically. A failed CPU gate blocks rollout even below every daily allowance.

## Distribution sequence after approval

1. Assemble the gate evidence and reconcile policy, App Privacy, privacy manifest, metadata and runtime requirements with the exact build. Complete the existing export checklist without changing its declarations or France policy.
2. Obtain the App Review decision on Tailscale/demo access through a separately authorized communication. Supply real review resources if requested; do not assume the proposed exception.
3. With explicit distribution authorization, upload the exact signed build and request Beta App Review for an external group. Record uploaded and beta-approved separately. No public link is created by this preparation.
4. After beta approval and measured capacity approval, authorize the public link with a tester limit no higher than the approved remaining capacity. Budget all existing installations, multiple devices per tester, background use and other account traffic. Apple's [public-link controls](https://developer.apple.com/help/app-store-connect/test-a-beta-version/invite-external-testers/) limit testers, not daily active installations. Supported criteria are device/platform/OS, not a documented France exclusion. Until the existing external-tester export caveat is resolved, do not publish an unrestricted link that could admit testers located in France.
5. Record actual beta outcomes and close any runtime/review gaps before a separately authorized App Store submission. Preserve France exclusion and the existing encryption declaration. Track submitted, approved and released as separate states. Stop invitations whenever capacity or trust/retention gates regress.
