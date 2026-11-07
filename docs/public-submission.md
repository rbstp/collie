# Public submission preparation

Preparation for [#359](https://github.com/rbstp/collie/issues/359) from the reviewed source at `190c6ed`; #359 records the submission results. [App Review notes](app-review.md) has the review material, checklist and [capacity cap](app-review.md#public-link-capacity); the [relay runbook](relay-runbook.md) has operating procedures.

## Release gates

For each gate, record the exact build, source SHA, deployment version, operator and evidence link. An upload, an installation, a successful APNs response and user acceptance are separate results.

| Gate | Evidence needed before submission |
|---|---|
| Paid runtime and capacity | Cold and warm CPU for enrollment, assertions, signing, sends and hostile input; account-wide Worker, Durable Object and D1 usage; concurrency and bounded cleanup; approved enrollment and device capacity |
| Production trust | App Attest under the publisher App ID on a physical device and in TestFlight; rejected development and simulator proofs; actual profile and signature entitlements; retained replay and revocation state |
| Push privacy and delivery | Every alert, background clear and Live Activity update and end; safe extension and widget failure; locked and pre-first-unlock behavior; real production APNs receipt on the submitted build |
| Compatibility and recovery | Both app and daemon upgrade orders; route renewal and revocation; 1027, 429 and D1 outages with direct sessions still usable |
| Personal embedded connectivity ([#389](https://github.com/rbstp/collie/issues/389)) | Independently reviewed same-user ownership checks; pairing, reconnect, background use, expiry, sign-in recovery and revocation on a real iPhone and Mac with no OS VPN profile or standalone Tailscale client, including beside a corporate VPN on the Mac. Static, mocked and Simulator checks do not prove physical coexistence |
| Review access | Release sample on simulator and signed iPhone, reset and exit without real effects; real-hardware video; App Review's decision on the sample and the Tailscale login |
| Desktop installation | The exact linked macOS (signed, notarized) and Linux downloads verified, with tested requirements and install and update instructions |
| Disclosures and operation | Published policy matches deployed retention and logging; App Privacy and privacy-manifest review; tested incident and credential rotation procedures |
| Export and metadata | The [export checklist](release.md#export-compliance), France restriction and external-tester caveat included; actual screenshots, age rating, contact details and metadata |

## Metadata draft

| Field | Draft |
|---|---|
| Name | Collie Herd |
| Subtitle | Remote for your coding agents |
| Primary category | Developer Tools |
| Price | Free |
| Mac availability | Off: the app is not offered on Apple silicon Macs |
| Support URL | <https://collie.rbstp.dev/support.html> |
| Privacy URL | <https://collie.rbstp.dev/privacy.html> |
| Marketing URL | <https://collie.rbstp.dev/> |
| Keywords | herdr,ai,terminal,cli,developer,programming,code,git,diff,review,assistant,llm,monitor,approve |

Promotional text:

> Approve a blocked Claude Code agent from your Lock Screen, review its Git changes and send key combinations. Now redesigned, with Herdr 2 support.

Description:

```text
Collie Herd puts the coding agents on your Mac or Linux machine in your pocket. Follow every agent that herdr runs, see which one needs you, and answer it from your iPhone or Apple Watch.

WHAT YOU CAN DO
• See every agent on every paired machine: its status, a live preview and, for Claude Code and Codex, the context it has left.
• Read an agent's conversation or its live terminal, and reply by typing, dictating, attaching files or sending keys.
• Add key shortcuts that send a key combination, such as ⌃⌥X, or a command, such as /compact.
• Approve or deny a blocked Claude Code agent from its notification, a Live Activity or the app.
• Start a task in a folder, a new Git worktree, a fresh clone or a pull request branch.
• Review an agent's Git changes and pull request diffs, and send it line comments.
• Track plan limits and estimated token cost for Claude Code, Codex and Copilot.
• Follow agents on the Lock Screen and in the Dynamic Island, and check agents, approvals and usage on Apple Watch.

WHAT YOU NEED
Collie Herd is a remote control: your agents keep running on your own machine. Real use needs:
• herdr or herdr-core (Herdr 2) on an Apple silicon Mac or an x86_64 Linux machine
• the free collied daemon on that machine (collie.rbstp.dev)
• a personal Tailscale account, signed in inside the app and on that machine

The app runs Tailscale itself, so you need no Tailscale app and no VPN profile. A restrictive tailnet policy may need an administrator to allow TCP port 8457.

No machine yet? Explore the built-in sample workspace without an account. Its actions are simulated and never leave your phone.

PRIVACY
Your agents, transcripts and files stay on your machine, and there is no Collie account. The phone reaches the machine over your own tailnet, inside mutual TLS. Your machine encrypts notification content so that only your phone can read it; the push service sees generic alerts and delivery metadata, as the privacy policy describes.

TESTED WITH
Collie Herd has been tested only with Claude Code CLI, Codex CLI and GitHub Copilot CLI. herdr supports more agents, but those have not been validated with Collie Herd. On Linux, only Arch Linux with Omarchy has been tested. In Herdr 2, some actions are limited; collie.rbstp.dev lists them.
```

Recheck these against App Store Connect and the submitted build; use the description only once relay notifications pass the release gates. Answer Apple's [age-rating questionnaire](https://developer.apple.com/help/app-store-connect/manage-app-information/set-an-app-age-rating/) from actual capabilities: agent-generated and user-controlled content, external links, terminal behavior.

## Guideline assessment

Checked 2026-10-16 against Apple's [guidelines](https://developer.apple.com/app-store/review/guidelines/) and [update summary](https://developer.apple.com/news/?id=92th7e5i); not an App Review determination.

- **1.2:** Collie sends prompts and files to private coding agents and shows user and agent content. The revised scope covers AI content creation apps; private use without a public feed is no automatic exemption. Assess filtering, content reporting and response, abusive-user blocking and contact safeguards, and ask App Review where unsure. Pairing and revocation are security controls, not moderation. This assessment authorizes no cloud scanning or other data-processing change.
- **2.3.1 and 2.5.19:** Describe changes specifically and disclose every shipped capability and its actual availability; the sample must not conceal real remote controls. Reconcile compiled but unavailable relay paths before submission: no dormant behavior, no undisclosed activation after review.
- **2.3.14, if creative assets are used:** Follow Apple's [asset guidance](https://developer.apple.com/app-store/asset-best-practices/): accurate functionality and brand, all-audience presentation, no misleading associations, unverifiable claims or specific or discounted prices. Keep app-in-use screenshots.
- **2.1 and 4.8:** open, tracked in [App Review notes](app-review.md). The sample does not exercise every real flow, so supply the tested devices, external services and hardware Apple's [review information](https://developer.apple.com/help/app-review/before-submitting-for-review/complete-review/) asks for.

### Release capability and review access

Verify against the exact submitted build:

- **Sample:** Welcome and Settings entry; simulated actions only. Release Simulator acceptance passed; physical iPhone acceptance is open.
- **Paired remote controls** need Tailscale, its policy, herdr and collied; review uses approved resources, never the owner's personal account or machine.
- **Push and attestation:** released builds enroll with App Attest and deliver encrypted alerts and Live Activities through the relay, with Live Activity update and end confirmed on a physical iPhone.
- **Debug VM and Simulator hooks:** excluded from Release app and core artifacts (keep the submitted build's proof); no production trust or review credentials.

## App Privacy draft

Draft for the relay-enabled build; the answers entered since in App Store Connect ([submission checklist](app-review.md#submission-checklist), step 5) are authoritative. Recheck them against the final binary, Tailscale SDK, Cloudflare configuration and third-party practices. Apple's [definitions](https://developer.apple.com/app-store/app-privacy-details/) count data kept beyond serving a request, even for functionality only, and linkage through a device: neither encryption nor having no account justifies a blanket Data Not Collected.

Each proposed answer is App Functionality, linked, not tracking:

| Data | Proposed category and use | Where it lives |
|---|---|---|
| Device and activity destinations; installation, key, pairing and route identifiers | Device ID, linked through installation and device | D1, while routes, grants and retention state remain |
| App Attest public key, receipt, environment and integrity signals | Other Data Types, used for authentication and fraud prevention | D1; Apple processes attestation independently |
| Per-installation send, routine and control counters | Other Usage Data, for quotas and security | Daily buckets reset lazily; the current row stays with installation state |

Still to classify:

- **Source IPs, request timing and sizes:** classify any retained data by actual use, with no location inference. Only transient abuse limits use IPs and D1 stores none; confirm what Cloudflare retains, since disabled observability does not prove it keeps nothing.
- **Crash, feedback and support submissions:** reassess Diagnostics and Customer Support for what the developer receives and retains from optional Apple and TestFlight reports and the support flow; optional use is no exemption.

The app has no advertising, advertising identifier or cross-company tracking. The relay cannot read private plaintext and does not persist its transient ciphertext, so payloads are not retained readable content. Tailscale account and device processing still needs the final third-party audit. The privacy manifest (`Collie/Resources/PrivacyInfo.xcprivacy`) declares no collected data types; its required-reason API declarations are not the store label.

### Actual retention and logging constraints

D1 holds no notification keys, payloads or source IPs. Hourly cleanup deletes records within 14 days of the last grant expiry, the bound the privacy policy publishes, not the plan's seven days: records become eligible seven days after grant expiry, and an installation's no earlier than 37 days after enrollment, even after revocation ([deadlines](relay.md#retention-and-cleanup)). Confirm cleanup with the runbook's `cleanup_backlog` count. [D1 Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/) keeps deleted records restorable for up to 30 days on Workers Paid; the [rollback procedure](relay-runbook.md#rollback) forbids a restore.

Observability stays disabled (`relay/cloudflare.config.ts`); never log request bodies, Authorization values, destinations, full URLs, attestation objects or secret values. Cloudflare's own service and security processing falls under its [privacy policy](https://www.cloudflare.com/privacypolicy/); verify the account's products and retention rather than promise zero logs.
