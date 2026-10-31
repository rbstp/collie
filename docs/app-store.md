# App Store release plan

Plan for [#290](https://github.com/rbstp/collie/issues/290), researched on 2026-10-16. It proposed work and changed no code or infrastructure. Its follow-up issues, #350 to #359, are closed; [Relay](relay.md), [Architecture](architecture.md) and [Desktop releases](desktop-releases.md) describe what shipped and take precedence where they differ from this plan. The main differences: the relay runs on Workers Paid with an internal crypto Durable Object because the [Free-plan CPU gate](reviews/relay-free-tier.md) failed; collied selects it with an empty `[push.relay]` section and a fixed endpoint; cleanup deletes records within 14 days of the last grant expiry, not seven; and the App Store submission went first, with no public TestFlight link, because TestFlight cannot exclude France ([#359](https://github.com/rbstp/collie/issues/359)).

## Decision

Ship the iOS app through external TestFlight, then the App Store, with a publisher-operated Cloudflare Worker forwarding encrypted pushes. Users install collied and the desktop companion from this repository's GitHub Releases: publisher-signed and notarized macOS downloads, also in a Homebrew tap, and Linux downloads with collied and the CollieTray Omarchy widget.

An App Store user needs a Tailscale account, the tailnet policy, a supported machine running herdr and collied, and a paired phone, but no Apple Developer account, APNs key, signing certificate or local build. The phone authorizes each paired machine to send through the relay; only the publisher holds the APNs key. Commands, approvals, terminal output, conversations and files still travel directly between phone and machine over Tailscale, inside mutual TLS.

Builds under another Apple team keep direct `[apns]` mode with their own key: deploying the auditable relay source does not let another team push to the App Store app. Firebase would also need publisher-side sending credentials, so it does not remove the relay.

## Findings on 2026-10-16

- `docs/release.md` assumed local builds and per-machine APNs keys; the macOS Keychain ACL trusted a Developer ID signed collied. Relay mode needs no APNs Keychain item on users' Macs, but downloads still need publisher signing.
- `crates/collied/src/push.rs` already abstracted sending with `Sender`. Approval context was sealed per paired machine's notification key; alert names, workspace labels, done alerts and agent alerts (PR watch wakes, archive-on-merge) were clear.
- `crates/collied/src/activity.rs` allowed routine updates every 2 seconds, refreshes every 600 seconds and eight activities per phone. Only approval context was encrypted; the widget decrypted it with the App Group key mirror.
- `crates/collied/src/server.rs` and `crates/collie-core/src/session.rs` required an exact protocol match, then version 25. [#327](https://github.com/rbstp/collie/issues/327) improved the error, not compatibility, and strict request decoding meant even an extra field could break an older daemon.
- `AgentDemo` was DEBUG only. Simulator TLS and terminal identities had software-key paths, but a simulator cannot establish genuine App Attest trust.
- `Mac/` built CollieBar for macOS 26+; `Linux/CollieTray` is an Omarchy/Quickshell widget, not a generic Linux tray executable. Neither had a release package.

## Relay and authorization

Use a small TypeScript Worker in this repository with one D1 database: no accounts, stored payloads, Queues, KV or external database. KV's [Free write allowance](https://developers.cloudflare.com/kv/platform/limits/) of 1,000 a day rules it out for registrations and counters, and synchronous forwarding avoids queue budgets and durable payloads. Serve the relay at the Worker Custom Domain `collie-push.rbstp.dev`, which Universal SSL covers, and keep `collie.rbstp.dev` as the DNS-only GitHub Pages CNAME; `push.collie.rbstp.dev` would need other certificate coverage ([Custom Domains](https://developers.cloudflare.com/workers/configuration/routing/custom-domains/), [Universal SSL coverage](https://developers.cloudflare.com/ssl/edge-certificates/universal-ssl/limitations/)).

### Phone enrollment and machine grants

1. Enable App Attest for the App ID and regenerate provisioning profiles. The phone attests an installation key against a short-lived relay challenge bound to the enrollment body. The Worker validates Apple's chain, nonce, app identity, key identity, environment and initial counter, then stores the verified public key and receipt, following [Apple's validation procedure](https://developer.apple.com/documentation/devicecheck/validating-apps-that-connect-to-your-server) and [fixtures](https://developer.apple.com/documentation/devicecheck/attestation-object-validation-guide), including documented authenticator-data extensions.
2. Later registration, rotation and revocation carry an App Attest assertion over a canonical operation body and one-time challenge, binding operation, installation, machine pairing, exact destination token, push class and APNs environment. Consume the challenge and advance the counter atomically, and serialize the phone's control operations so they cannot arrive out of counter order. Challenges carry a five-minute HMAC expiry and allocate nothing in D1 unauthenticated; replay rejection uses durable state at commit.
3. After the user confirms pairing, register that machine's routes: one shared app-token destination for alert and background delivery, and a route per Live Activity update token. A machine cannot substitute another destination, activity token, environment or topic. Cap an installation at eight paired machines and eight active Live Activities, and rate-limit enrollment and control operations.
4. Return a versioned, HMAC-authenticated bearer send token per route, expiring after 30 days, whose claims bind installation, pairing, destination digest, push classes, environment, expiry and revocation epochs. D1 keeps raw destinations in bounded records, never notification keys or payloads. Each send verifies the HMAC once and checks epochs and route validity in D1, with no eventually consistent revocation cache. Each machine gets only its own grants, over the paired mutual-TLS session, and keeps them private and out of diagnostics.
5. Renew on launch and reconnection from seven days before expiry, and re-register whenever iOS or ActivityKit supplies a changed token. A late APNs failure for an old route must not delete its replacement. A phone that never renews loses relay pushes at expiry while direct sessions keep working; settings and doctor must show this.
6. Removing a machine revokes only its grants by incrementing its epoch; revoking a whole installation is an operator action ([runbook](relay-runbook.md#revoke-a-pairing-or-an-installation)). Revoke at the relay before discarding local credentials. Offline, remove the pairing locally at once and keep a minimal pending revocation, with an explicit pending status, for retry. The machine may push until revocation succeeds or its grant expires; never claim offline revocation is immediate.

App Attest proves app integrity, not the user's identity, APNs token ownership or a daemon's trustworthiness; pairing remains the authorization boundary. A stolen send token sends bounded pushes to its destination until revoked or expired, and cannot approve anything. Attestation keys do not survive reinstall or restore, so the phone enrolls anew and re-pairs, without migration ([App Attest integrity](https://developer.apple.com/documentation/devicecheck/establishing-your-app-s-integrity)).

Production accepts only production App Attest and APNs routes, and TestFlight attests in production too. Debug device builds use a separate development deployment and attestation, or direct mode for their own team. A client's claim that attestation is unsupported never authenticates in production ([App Attest environments](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.devicecheck.appattest-environment)).

### Sending to Apple

Expose a versioned send endpoint. Bound bodies before expensive parsing: 16 KiB for attestation and control, 8 KiB for a send envelope, 4 KiB for the serialized APNs payload. Authenticate before D1 quota work or forwarding. The Worker resolves the destination from the authorized route, sets the fixed Collie topic and permits only alert, background or liveactivity delivery. Callers supply only validated priority, expiration and opaque collapse IDs: no URL, topic, provider authorization, header passthrough or redirects. The payload's public envelope must match its kind's allowlist, including fixed generic alert text, with no unknown fields.

Forward synchronously with a bounded timeout and return a small normalized result. `410 Unregistered` or `BadDeviceToken` invalidates only the matching route generation. `DeviceTokenNotForTopic`, signing errors and wrong-environment responses are configuration failures, never grounds to erase every registration. Never log APNs response bodies containing destinations.

Reuse an APNs provider JWT in memory per isolate. T3 Code's [APNs client](https://github.com/pingdotgg/t3code/blob/cfafb4bb54d244e4612f5681aad3832ea64cb0fb/infra/relay/src/agentActivity/ApnsClient.ts) delivers through Worker `fetch`, and its [provider-token cache](https://github.com/pingdotgg/t3code/blob/cfafb4bb54d244e4612f5681aad3832ea64cb0fb/infra/relay/src/agentActivity/ApnsProviderTokens.ts) and [RFC 6979 ES256 signer](https://github.com/pingdotgg/t3code/blob/cfafb4bb54d244e4612f5681aad3832ea64cb0fb/infra/relay/src/agentActivity/apnsJwt.ts) derive identical JWTs in 45-minute windows across isolates. Adopt that after benchmarking cold signing (ordinary ECDSA need not be deterministic), keep JWTs out of D1, and skip T3's accounts, Postgres and queues. Apple's [provider-token guidance](https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns) requires reuse and refresh within the validity window.

## Push privacy

Keep the per-machine notification key on the phone and collied, and extend the authenticated encrypted envelope to all private display state, with a fresh random nonce and domain-separated authenticated data binding envelope version, push kind, machine, destination or activity identity and event ID. Shared Rust and Swift vectors cover tampering, wrong key, wrong event and wrong kind. Neither the relay nor Apple gets decryption keys.

| Push | Encrypted data | Public data and fallback |
|---|---|---|
| Approval alert | Agent/workspace names, title, body and command/question context | Generic title/body such as "An agent needs attention", opaque routing/approval IDs, category, mutable-content flag and delivery metadata |
| Done alert | Names and completion text | Generic "An agent finished", opaque routing IDs and delivery metadata |
| Agent alert | PR watch wake, archive-on-merge and other agent text and labels | Generic "An agent needs attention", opaque routing IDs and delivery metadata |
| Background approval clear | No display text exists | Opaque machine/approval lookup IDs and content-available flag; its timing reveals that something cleared |
| Live Activity update/end | Names, workspace, kind, status, status-since, approval count and context | Encrypted envelope plus ActivityKit timestamp, event, stale/dismissal dates and opaque routing identifiers |

ColliePush decrypts every alert's title and body through `mutable-content: 1`; the original alert must stay useful and private if the extension times out or lacks keys ([notification service extension guidance](https://developer.apple.com/documentation/usernotifications/modifying-content-in-newly-delivered-notifications)).

The widget decrypts the encrypted `ContentState` envelope at render time with the protected App Group mirror; initial local state and attributes are ciphertext too. Relevance scores and alert text stay generic, with no status-dependent public field. A missing, invalid or pre-first-unlock key, which the widget cannot fetch, shows neutral text without approval controls. Decisions keep tailnet fetch-and-validate; no push carries a nonce or decision authority. Respect the APNs payload limit and ActivityKit's combined 4 KiB static and dynamic limit ([Live Activity data constraints](https://developer.apple.com/documentation/activitykit/displaying-live-data-with-live-activities)).

The publisher and Cloudflare see IP addresses, device and activity tokens, route relationships, push class, generic fallback text, opaque IDs, timing, frequency and ciphertext sizes. Apple sees APNs metadata, generic fallback and ciphertext, knows when activities update or end, and takes part in App Attest. Categories and urgency can reveal that attention is needed: encryption hides names and detailed status, not traffic patterns or what iOS shows after decryption. The App Group mirror keeps its protection, weaker than a ThisDeviceOnly Keychain item.

Update `docs/threat-model.md`, `docs/notifications.md`, `docs/architecture.md` and the privacy page in the implementing PRs, before deploying the relay or shipping a relay-enabled build. Publish retention: no stored payloads or authorization/body logs; active registration, key and epoch records only, with revoked or expired records kept until the last grant expires, then deleted within seven days. Budget the deletion writes. Disable request-header capture and invocation logs that keep sensitive URLs; tokens go in bodies or Authorization, never paths or query strings. Monitor with aggregate counters and error codes.

## Free-tier budget and capacity

Researched limits and proposed budgets, not load-test results:

| Resource | Free allowance | Design budget |
|---|---|---|
| Worker requests | 100,000/day across the account, reset at 00:00 UTC | Reserve at least 20,000 for control, retries, probes and other account use; count rejected traffic too |
| Worker CPU | 10 ms per invocation; network waits excluded | Benchmark cold/warm enrollment, assertion checks, signing, send and hostile input; target p99 below 8 ms with no CPU-limit errors |
| D1 | 5 million rows read/day; 100,000 rows written/day; 5 GB/account, 500 MB/database | Indexed lookups; at most 4 billed writes per authorized send attempt as a launch gate; reserve 20,000 writes/day for control, failed quota checks, cleanup and other account use |
| Burst limiter | Workers binding supports 10/60-second windows | Soft per-send-token, installation and source-IP limits, before expensive work; never the authority for global quota or replay protection |

Sources: [Workers limits](https://developers.cloudflare.com/workers/platform/limits/), [D1 pricing and row accounting](https://developers.cloudflare.com/d1/platform/pricing/), [D1 limits](https://developers.cloudflare.com/d1/platform/limits/), [rate limiting binding](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/). D1 index changes and deletes count as writes. Binding counters are per location and eventually consistent. The binding is GA with no documented paid-plan prerequisite; deploying it on a real Free account is an acceptance check. If it is unavailable there, keep exact D1 quotas and use the Free zone's [one IP-based WAF rate rule](https://developers.cloudflare.com/waf/rate-limiting-rules/). Neither can keep the account available under distributed abuse.

Start with a hard 1,000 authorized APNs attempts per installation per UTC day across all its machines and activity routes, at most 700 of them routine Live Activity updates, leaving 300 for alerts, clears and ends. Enforce this in D1 with atomic conditional updates, plus global ceilings of 20,000 total and 16,000 routine attempts a day, leaving at least 4,000 global slots for alerts, clears and ends, which may also use unused routine capacity. Concurrent requests must not overspend either cap. Charge before forwarding, including retries and ambiguous network failures; never refund an attempt that may have reached Apple. Reject over-budget routine updates without losing the newest pending state. Daily quota responses name the UTC reset time.

Exact D1 transactions, schema and billed row costs are an implementation gate: quota enforcement must be atomic, indexed and fail closed, with no all-row scans or per-push assertion-counter updates. Prototype one device-counter update whose global-counter trigger aborts the whole statement when either budget would be exceeded, and prove it on D1 with concurrent last-slot and rollback tests. The 20,000 ceiling is `(100,000 - 20,000) / 4`; count the complete lifecycle, including outcome writes, index maintenance and cleanup, and if a send bills more than four writes, lower the ceiling and recompute capacity before launch. No per-send deduplication table or stored payload is budgeted. In-memory event and collapse IDs reduce duplicates, but an accepted send whose response is lost can arrive twice; never promise exactly-once delivery.

In relay mode, coalesce routine Live Activity updates to the latest state at most once per 60 seconds per agent, sharing the device budget across machines. Unchanged state needs only the existing ten-minute refresh, counted as routine. Approval alerts, clears and ends bypass this cadence but count toward the reserved budget. Direct mode keeps its cadence. A lost routine update must not suppress a needed approval alert; keep the fallback to an alert when the activity cannot be notified.

A planning example per active phone per day:

| Traffic | Requests/attempts |
|---|---:|
| 40 approvals plus their background clears | 80 |
| 40 completed turns | 40 |
| 10 PR wakes plus 10 archive alerts | 20 |
| One continuously changing followed agent for 8 hours, every 60 seconds | 480 |
| Four activity ends | 4 |
| Enrollment/registration/renewal/revocation requests, including challenges | 12 |
| Total | 636 |

That is 624 send attempts and 12 control requests before retries. Thirty such daily active phones use 18,720 attempts, 360 control requests and at most 74,880 send-path D1 writes. The 20,000 ceiling supports roughly 32 such phones without retries, so start external TestFlight at **30 daily active phones**, not 100,000 divided by alert count. The 20,000 reserved D1 writes must also cover control bursts and cleanup, and enrollment CPU and writes need their own measured budget before invitations expand. One phone following eight continuously changing agents for eight hours would request 3,840 routine updates; its 700 routine cap deliberately degrades freshness.

Without coalescing, one followed agent at today's two-second gap for eight hours could cost 14,400 attempts, and eight agents 115,200, more than the account's whole daily Worker allowance. Per-device quotas cannot restore availability once unauthenticated traffic has exhausted it.

Never enable billing automatically. Review a move to Workers Paid when seven-day peak usage exceeds 70% of the Worker or D1 daily allowances, more than 1% of legitimate sends hit budgets, or forecast use exceeds 30 active phones. A failed CPU validation stops the plan even at one user: optimize, or split enrollment into individually verified, bounded stages on the same Free stack, and remeasure without skipping checks. If it still fails, report the free-tier requirement unmet and revise the plan before shipping. Paid Workers is an explicit future decision, not the assumed answer to this issue.

## collied behavior and outages

Add a relay mode, `[push] relay = "https://collie-push.rbstp.dev"`, mutually exclusive with direct `[apns]`. Setup for the published app defaults to relay, skips APNs import and needs no Keychain APNs item. Custom endpoints stay explicit operator configuration; the phone never silently picks an endpoint for production grants. Self-built apps keep direct setup.

`collied apns test` exercises the selected sender and reports the mode. `collied doctor` checks relay health, local registration and expiry, last send result, per-device budget and backoff or reset time, without printing credentials. A health response cannot prove APNs delivery or remaining account quota; actual send failures are authoritative.

The send path then removed invalid tokens and logged other failures, with no outage retry policy. Add bounded in-memory exponential backoff with jitter for network errors, APNs transient failures, 429, relay 5xx and Cloudflare's quota error 1027, a Cloudflare error code rather than an HTTP status, possibly without relay JSON. Honor a bounded Retry-After, avoid probe and retry storms, and distinguish daily device and global quotas from short burst limits. A known daily exhaustion waits for the reset, never a fast retry loop.

Keep only the newest pending routine update per activity and bounded unexpired alerts. Recheck pairing, revocation, route generation and approval expiry before each retry. Drop stale approvals, use current state after reconnect, and let tailnet synchronization repair missed clears and ends. No disk queue of payloads. When the relay, Apple or quota is unavailable, pushes degrade and doctor explains why; sessions, tailnet approvals, terminal, chat and desktop controls keep working. APNs acceptance never guarantees background execution or display.

## Protocol compatibility before App Store distribution

Create a negotiated baseline before external TestFlight, then keep the current and previous two negotiated revisions as new ones arrive. There are no adapters for older versions: existing exact-match peers may need one explicit update to reach the baseline.

Negotiate the highest common revision from advertised ranges and a small capability set; both hello implementations, pairing and bootstrap paths and serializers must agree. Feature-gate methods, parameters, response and event fields and enum variants, including relay registration and encrypted push formats; a range check in place of equality is not enough. Keep strict request validation and every pairing and authorization check. Unsupported peers keep #327's directional update message, with release-download instructions beyond `just collied-install`.

Test app N against daemon N, N-1 and N-2, and daemon N against each retained older app, with serialized fixtures, both upgrade orders and below-floor cases. Never send a method or field a peer cannot decode. Publish the baseline, floor and removal policy in the architecture doc and release notes. App Store review delay must never strand an installed app when the daemon advances.

## Desktop releases

Publish these assets in this repository's GitHub Releases with checksums, exact source SHA, protocol range, platform and runtime requirements, and install and update instructions. Pin build inputs and downloaded tools, and test the packaged artifacts, not just source builds.

| Platform | Initial release assets | Installation and checks |
|---|---|---|
| macOS 26+, Apple silicon | Signed collied archive and signed/notarized CollieBar app archive | Publisher Developer ID Application, hardened runtime and notarization for daemon distribution and app; staple supported containers/app and verify Gatekeeper on a clean Mac; Homebrew formula for collied and cask for CollieBar |
| Linux x86_64 | collied binary archive and architecture-independent CollieTray widget archive | State glibc/runtime floor; install daemon plus systemd user service through setup; widget instructions name Omarchy 4/Quickshell, `qrencode`, manifest and all QML/JS/icon files |

Do not advertise Intel macOS or Linux arm64 until native builds, bundled Go and libtailscale dependencies and packaged smoke tests pass for them. Do not describe CollieTray as a generic GNOME or KDE tray app; a portable replacement and AUR publishing are later optional work.

Signing and notary credentials live in CI secrets, isolated from untrusted pull requests. Never redistribute the APNs key. See [Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution), [Homebrew formulae](https://docs.brew.sh/Formula-Cookbook) and [casks](https://docs.brew.sh/Cask-Cookbook).

Reuse the release tag and SHA, stamp desktop versions from the tag, and give one workflow ownership of creating the release. The TestFlight workflow creates `v*` tags with `GITHUB_TOKEN` after upload, and a tag-push workflow does not run for such tags ([GitHub's trigger restriction](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow)), so invoke a reusable desktop release workflow explicitly, with manual dispatch for desktop-only releases and failed-job retries at the same immutable tag and SHA. Never move a tag or publish a version twice.

Homebrew upgrades must not leave launchd pointing into a deleted versioned Cellar or Caskroom path. Service installers canonicalize `current_exe`, so a stable symlink alone does not help: point the existing collied service definition at the new executable and restart it only if it was running. Keep one service label, with no competing `brew services` daemon. Test first install, upgrade, rollback, uninstall, desktop companion discovery and preservation of node identity, configuration and pairings.

## Simulator and review testing

Simulator support is a follow-up, never a production attestation bypass. It has three kinds of coverage:

1. In iOS Simulator, run pairing, authenticated session and UI tests with the existing software identity keys. Add local Worker and D1 tests with Apple attestation fixtures and invalid, replay and concurrent-budget cases, and mock APNs for deterministic failure and outage tests, with no Apple credentials in CI.
2. Give DEBUG simulator builds an explicitly provisioned development relay identity, with separate storage and send secrets, sandbox APNs only and no production enrollment fallback. A local relay may use a maintainer's sandbox-capable APNs credential for real delivery, never embedded or distributed. On supported Macs the simulator gets real alert, notification-extension and Live Activity pushes from APNs Sandbox; other hosts get mocked transport and local payload and UI tests ([Xcode Simulator APNs support](https://developer.apple.com/documentation/xcode-release-notes/xcode-14-release-notes), [ActivityKit push testing](https://developer.apple.com/documentation/activitykit/starting-and-updating-live-activities-with-activitykit-push-notifications)). `simctl push` checks rendering but does not replace a real APNs-to-extension test.
3. On a physical iPhone, validate genuine App Attest, production rejection of simulator and development credentials, Secure Enclave behavior, locked and pre-first-unlock decryption, background clears and notification actions; validate production APNs and App Attest with external TestFlight. Simulator success cannot satisfy these gates.

Document a repeatable simulator command sequence that separates mocked from real sandbox results. The public app and the published desktop binaries must never gain a development trust bypass.

For App Review, expose a clearly labeled sample workspace in Release builds: bundled synthetic data, simulated approvals and terminal controls with visible local outcomes, no real machine effects. It must work without a Tailscale login or a Mac, cover the main screens and reset without touching real pairings. Arbitrary fixture loading and developer launch flags must not become the user-facing demo.

Start with external TestFlight through a public link after Beta App Review, capped at the measured capacity, then submit to the App Store with review notes on the sample workspace, the Tailscale dependency, pairing, optional desktop software and background pushes, plus a real-hardware video. [App Review notes](app-review.md) holds the draft, the shot list, the submission checklist and the capacity cap. A video alone does not establish compliance. Apple's [guidelines 2.1 and 4.8](https://developer.apple.com/app-store/review/guidelines/) require review access and address third-party login: Tailscale appears to fit the exception for a client of a specific third-party service, but confirm it with App Review, since demo access is no guaranteed waiver, and obtain any prior approval needed to replace review credentials with a built-in demo ([review information](https://developer.apple.com/help/app-review/before-submitting-for-review/complete-review/)).

Rechecked October 16, 2026 against Apple's [guidelines](https://developer.apple.com/app-store/review/guidelines/) and [update summary](https://developer.apple.com/news/?id=92th7e5i): assess 1.2 for this private coding-agent client without assuming an exemption; keep 2.3.1 feature notes separate from 2.5.19 disclosure of shipped capabilities and availability; apply 2.3.14 if creative assets are used. The [submission draft](public-submission.md#guideline-assessment) records the capability boundaries and open review questions.

Prepare free pricing, age rating, screenshots, description, support and privacy URLs and App Privacy answers, checked against the actual deployment: device identifiers, relay IP and timing metadata, retention, Cloudflare and Apple processing, diagnostics and the absence of payload storage. Replace the privacy page's "no server" statement before a relay-enabled build ships. Follow the [export checklist](release.md#export-compliance), including the France restriction and external-tester caveat, without silently changing the encryption declaration. Personal embedded Tailscale sign-in remains required. [#389](https://github.com/rbstp/collie/issues/389) is a mandatory pre-submission gate: independently review the same-user ownership checks and validate real iPhone and Mac pairing, reconnect and background recovery, expiry and revocation without an OS VPN profile, including with a separate corporate VPN active on the Mac. Compatible personal policies need no edit; restrictive ones may need a TCP 8457 grant. Static, mocked and Simulator checks cannot establish physical corporate coexistence.

## Operations and launch gates

Use [free Workers metrics and logs](https://developers.cloudflare.com/workers/observability/logs/workers-logs/) for aggregate counts by push class and result, CPU, D1 row costs, quota rejections and APNs errors. Free logs are bounded: sample or disable successful invocation logs, keep only redacted errors and metrics, and build no logging backend or log-based queue. Track development and other Workers and D1 usage in the account too.

Keep the APNs key and HMAC signing secret as Worker secrets. Rotate APNs credentials by deploying the new key ID and key together and testing before revoking the old key. Rotate versioned HMAC secrets with an overlap of at most the grant lifetime; emergency rotation invalidates grants at once and phones re-register. The [relay runbook](relay-runbook.md) covers monitoring, revoking an installation or pairing, stopping enrollment, diagnosing 1027, 429 and D1 exhaustion, rollback and payload-free incident reports.

Before public distribution, record evidence for deployed Free-plan App Attest CPU and D1 budgets; rejected unauthorized, replayed and revoked routes; every encrypted push kind and its safe fallback; bounded outage recovery; protocol upgrade combinations; clean desktop install and upgrade; simulator sandbox tests; and physical-device and TestFlight tests. This planning issue deployed no prototype and measured no CPU or delivery.

Open questions, assigned to the follow-ups below: cold App Attest verification cost, the indexed D1 transaction write count, Free-account limiter availability, clean-machine runtime floors, physical locked-widget behavior and App Review's view of Tailscale and demo access.

## Ordered follow-up work

Each item was tracked in the Phase 7 milestone, in dependency order; desktop packaging and the demo could proceed independently. Production relay sending waited for privacy, authorization and compatibility.

1. **[Build and measure the free-tier relay (#350)](https://github.com/rbstp/collie/issues/350).** Implement the bounded Worker, deterministic APNs JWT, D1 schema and quotas and a mocked APNs adapter; deploy an isolated Free-plan validation environment to measure CPU and row budgets. Production sending and public enrollment wait for items 2 through 4.
2. **[Establish protocol negotiation and the compatibility window (#351)](https://github.com/rbstp/collie/issues/351).** Land a safe baseline and capability and serialization gates before changing phone and daemon relay messages.
3. **[Encrypt every alert and Live Activity display value (#352)](https://github.com/rbstp/collie/issues/352).** Cover all alert producers, initial state, widget decryption, safe fallbacks and shared crypto vectors; update the security and privacy docs.
4. **[Add attested installation registration and per-machine grants (#353)](https://github.com/rbstp/collie/issues/353).** Integrate phone enrollment, D1 validation, route-specific tokens, rotation and revocation, using items 1 and 2; requires physical production-attestation acceptance.
5. **[Add collied relay mode and bounded outage handling (#354)](https://github.com/rbstp/collie/issues/354).** Using items 1 through 4, update setup, doctor, `apns test`, coalescing and shared-device budgets, keeping direct mode.
6. **[Make the push flow repeatable in iOS Simulator (#355)](https://github.com/rbstp/collie/issues/355).** Add isolated DEBUG development trust and local, mocked and real sandbox procedures alongside items 1 through 5, with production rejection tests.
7. **[Publish signed and notarized macOS daemon and menu app releases (#356)](https://github.com/rbstp/collie/issues/356).** Build GitHub assets, Homebrew distribution, tag integration and clean upgrade checks; relay-first setup requires item 5.
8. **[Publish Linux daemon and CollieTray widget releases (#357)](https://github.com/rbstp/collie/issues/357).** Build GitHub assets, runtime declarations and Omarchy install and upgrade checks; relay-first setup requires item 5.
9. **[Provide a Release-build sample workspace for review (#358)](https://github.com/rbstp/collie/issues/358).** Implement the inert, no-login demo with device and simulator checks, and no developer fixture interfaces in Release.
10. **[Prepare external TestFlight and App Store submission (#359)](https://github.com/rbstp/collie/issues/359).** After the earlier gates, complete privacy and App Privacy, metadata, review notes, export checks and runbooks, then request Beta App Review and a capacity-limited public TestFlight link before App Store review.
