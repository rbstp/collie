# App Store release plan

Plan for [#290](https://github.com/rbstp/collie/issues/290), researched on 2026-10-16; it changed no code or infrastructure. Its follow-ups, #350 to #359, are closed. Where they differ from this plan, [Relay](relay.md), [Architecture](architecture.md) and [Desktop releases](desktop-releases.md) describe what shipped. The main differences:

- The relay runs on Workers Paid with an internal crypto Durable Object, because the [Free-plan CPU gate](reviews/relay-free-tier.md) failed.
- collied selects it with an empty `[push.relay]` section and a fixed endpoint.
- Cleanup deletes records within 14 days of the last grant expiry, not seven.
- The App Store submission went first, with no public TestFlight link, because TestFlight cannot exclude France ([#359](https://github.com/rbstp/collie/issues/359)).

## Decision

Ship the iOS app through external TestFlight, then the App Store, with a publisher-operated Cloudflare Worker forwarding encrypted pushes. Users install collied and the desktop companion from GitHub Releases: signed and notarized macOS downloads, also in a Homebrew tap, and Linux downloads of collied and the CollieTray Omarchy widget.

An App Store user needs a Tailscale account, the tailnet policy, a supported machine running herdr and collied, and a paired phone, but no Apple Developer account, APNs key, signing certificate or local build. The phone authorizes each paired machine to send through the relay; only the publisher holds the APNs key. Commands, approvals, terminal output, conversations and files still travel directly between phone and machine over Tailscale, inside mutual TLS.

Builds under another Apple team keep direct `[apns]` mode with their own key: deploying the relay source does not let another team push to the App Store app. Firebase would also need publisher-side sending credentials, so it does not remove the relay.

## Findings on 2026-10-16

- `docs/release.md` assumed local builds and per-machine APNs keys; the macOS Keychain ACL trusted a Developer ID signed collied. Relay mode needs no APNs Keychain item on users' Macs, but downloads still need publisher signing.
- `crates/collied/src/push.rs` already abstracted sending with `Sender`. Approval context was sealed per paired machine's notification key; alert names, workspace labels, done alerts and agent alerts (PR watch wakes, archive-on-merge) were clear.
- `crates/collied/src/activity.rs` allowed routine updates every 2 seconds, refreshes every 600 seconds and eight activities per phone. Only approval context was encrypted; the widget decrypted it with the App Group key mirror.
- `crates/collied/src/server.rs` and `crates/collie-core/src/session.rs` required an exact protocol match, then version 25. [#327](https://github.com/rbstp/collie/issues/327) improved the error, not compatibility, and strict request decoding meant even an extra field could break an older daemon.
- `AgentDemo` was DEBUG only. Simulator TLS and terminal identities had software-key paths, but a simulator cannot establish genuine App Attest trust.
- `Mac/` built CollieBar for macOS 26+; `Linux/CollieTray` is an Omarchy/Quickshell widget, not a generic Linux tray executable. Neither had a release package.

## Relay and authorization

Use a small TypeScript Worker in this repository with one D1 database: no accounts, stored payloads, Queues, KV or external database. KV's [Free write allowance](https://developers.cloudflare.com/kv/platform/limits/) of 1,000 a day rules it out for registrations and counters, and synchronous forwarding avoids queue budgets and durable payloads. Serve it at `collie-push.rbstp.dev` ([domain choice](relay.md#operations)).

### Phone enrollment and machine grants

[Relay](relay.md#attested-control-contract) has the shipped contract. The plan:

- **Enrollment:** enable App Attest for the App ID and regenerate provisioning profiles. The phone attests an installation key against a short-lived challenge bound to the enrollment body; the Worker checks it as [Apple's procedure](https://developer.apple.com/documentation/devicecheck/validating-apps-that-connect-to-your-server) and [fixtures](https://developer.apple.com/documentation/devicecheck/attestation-object-validation-guide) describe and stores the verified key and receipt.
- **Control operations:** registration, rotation and revocation carry an assertion over the canonical operation and a one-time challenge (five-minute HMAC expiry, no D1 allocation before authentication), binding installation, pairing, exact destination token, push class and APNs environment. Consuming the challenge and advancing the counter are atomic, and the phone serializes its operations.
- **Routes:** after the user confirms pairing, one shared app-token route for alert and background delivery and one per Live Activity token, which a machine cannot swap for another destination, token, environment or topic. Caps: eight paired machines and eight active Live Activities per installation, plus rate-limited enrollment and control.
- **Grants:** a versioned HMAC bearer token per route, valid 30 days, binding installation, pairing, destination digest, push classes, environment, expiry and revocation epochs. Each send verifies the HMAC and checks epochs and route in D1, with no eventually consistent revocation cache. D1 keeps raw destinations in bounded records, never notification keys or payloads. Each machine receives only its own grants, over mutual TLS, and keeps them out of diagnostics.
- **Renewal:** from seven days before expiry, on launch and reconnection, and whenever iOS or ActivityKit supplies a changed token. A late APNs failure for an old route must not delete its replacement. A phone that never renews loses relay pushes at expiry, which settings and doctor show; direct sessions keep working.
- **Revocation:** removing a machine revokes only its grants, by incrementing its epoch at the relay before local credentials are discarded; revoking an installation is an operator action ([runbook](relay-runbook.md#revoke-a-pairing-or-an-installation)). Offline, the pairing goes at once and a pending revocation, shown as pending, retries. Until it succeeds or the grant expires the machine can still push, so offline revocation is never claimed immediate.

App Attest proves app integrity, not the user's identity, APNs token ownership or a daemon's trustworthiness; pairing remains the authorization boundary. A stolen send token sends bounded pushes to its destination until revoked or expired, and approves nothing. Attestation keys do not survive reinstall or restore, so the phone enrolls anew and re-pairs ([App Attest integrity](https://developer.apple.com/documentation/devicecheck/establishing-your-app-s-integrity)). Production accepts only production App Attest and APNs routes, TestFlight included; Debug device builds use a separate development deployment, or direct mode for their own team. A client's claim that attestation is unsupported never authenticates in production ([App Attest environments](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.devicecheck.appattest-environment)).

### Sending to Apple

Bound bodies before expensive parsing: 16 KiB for attestation and control, 8 KiB for a send envelope, 4 KiB for the APNs payload. Authenticate before quota work. The Worker resolves the destination from the route, sets the fixed topic and permits only alert, background or liveactivity delivery; callers supply only validated priority, expiration and opaque collapse IDs, in a public envelope that matches its kind's allowlist. `410 Unregistered` or `BadDeviceToken` invalidates only the matching route generation; `DeviceTokenNotForTopic`, signing and wrong-environment errors are configuration failures that never erase registrations. Never log APNs response bodies containing destinations.

Cache the provider JWT in memory, never in D1. T3 Code's [APNs client](https://github.com/pingdotgg/t3code/blob/cfafb4bb54d244e4612f5681aad3832ea64cb0fb/infra/relay/src/agentActivity/ApnsClient.ts) delivers through Worker `fetch`, and its [token cache](https://github.com/pingdotgg/t3code/blob/cfafb4bb54d244e4612f5681aad3832ea64cb0fb/infra/relay/src/agentActivity/ApnsProviderTokens.ts) and [RFC 6979 ES256 signer](https://github.com/pingdotgg/t3code/blob/cfafb4bb54d244e4612f5681aad3832ea64cb0fb/infra/relay/src/agentActivity/apnsJwt.ts) derive identical JWTs in 45-minute windows across isolates ([Apple's guidance](https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns)). Adopt that after benchmarking cold signing, without T3's accounts, Postgres and queues.

## Push privacy

Keep the per-machine notification key on the phone and collied, and extend the authenticated encrypted envelope to all private display state ([format](architecture.md#encrypted-display-envelope)). Neither the relay nor Apple gets decryption keys.

| Push | Encrypted data | Public data and fallback |
|---|---|---|
| Approval alert | Agent and workspace names, title, body, command or question context | Generic "An agent needs attention", opaque routing and approval IDs, category, mutable-content flag |
| Done alert | Names and completion text | Generic "An agent finished", opaque routing IDs |
| Agent alert | PR watch wake, archive-on-merge and other agent text and labels | Generic "An agent needs attention", opaque routing IDs |
| Background approval clear | No display text exists | Opaque machine and approval lookup IDs, content-available flag; its timing reveals that something cleared |
| Live Activity update or end | Names, workspace, kind, status, status-since, approval count and context | Encrypted envelope, ActivityKit timestamp, event, stale and dismissal dates, opaque routing IDs |

Every push also carries APNs delivery metadata. The original alert must stay useful and private when ColliePush times out or lacks keys ([extension guidance](https://developer.apple.com/documentation/usernotifications/modifying-content-in-newly-delivered-notifications)). The widget decrypts at render time with the App Group key mirror; initial state and attributes are ciphertext too, and relevance and alert text stay generic. Without a usable key, including before first unlock, it shows neutral text without approval controls. No push carries a nonce or decision authority. Fit ActivityKit's combined 4 KiB static and dynamic limit ([data constraints](https://developer.apple.com/documentation/activitykit/displaying-live-data-with-live-activities)).

The publisher, Cloudflare and Apple still see delivery metadata ([Relay](relay.md#operations) lists it), and Apple takes part in App Attest. Encryption hides names and detailed status, not traffic patterns or what iOS shows after decryption: categories and urgency can reveal that attention is needed. The App Group mirror is weaker than a ThisDeviceOnly Keychain item.

Planned retention: no stored payloads or authorization or body logs; only registration, key and epoch records, with revoked or expired ones deleted within seven days of the last grant expiry and the deletion writes budgeted. Tokens go in bodies or Authorization, never paths or query strings, and header capture and invocation logs that keep sensitive URLs stay off.

## Free-tier budget and capacity

Researched limits and proposed budgets, not load-test results:

| Resource | Free allowance | Design budget |
|---|---|---|
| Worker requests | 100,000/day across the account, reset at 00:00 UTC | Reserve at least 20,000 for control, retries, probes and other account use; count rejected traffic too |
| Worker CPU | 10 ms per invocation; network waits excluded | Benchmark cold and warm enrollment, assertion checks, signing, send and hostile input; target p99 below 8 ms with no CPU-limit errors |
| D1 | 5 million rows read/day; 100,000 rows written/day; 5 GB/account, 500 MB/database | Indexed lookups; at most 4 billed writes per authorized send attempt as a launch gate; reserve 20,000 writes/day for control, failed quota checks, cleanup and other account use |
| Burst limiter | Workers binding supports 10/60-second windows | Soft per-send-token, installation and source-IP limits, before expensive work; never the authority for global quota or replay protection |

Sources: [Workers limits](https://developers.cloudflare.com/workers/platform/limits/), [D1 pricing](https://developers.cloudflare.com/d1/platform/pricing/), [D1 limits](https://developers.cloudflare.com/d1/platform/limits/), [rate limiting binding](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/). D1 index changes and deletes count as writes. Binding counters are per location and eventually consistent; if the binding fails its acceptance check on a real Free account, keep exact D1 quotas and use the Free zone's [one IP-based WAF rate rule](https://developers.cloudflare.com/waf/rate-limiting-rules/). Neither keeps the account available under distributed abuse.

Quotas per UTC day, enforced in D1 with atomic conditional updates that concurrent requests cannot overspend:

- Per installation, across all its machines and activity routes: 1,000 authorized APNs attempts, at most 700 of them routine Live Activity updates, leaving 300 for alerts, clears and ends.
- Global: 20,000 attempts, at most 16,000 routine, leaving at least 4,000 for alerts, clears and ends, which may also use unused routine capacity.
- Charge before forwarding, retries and ambiguous network failures included, and never refund an attempt that may have reached Apple. Reject over-budget routine updates without losing the newest pending state. Quota responses name the UTC reset time.

Enforcement must be atomic, indexed and fail closed, with no all-row scans or per-push assertion-counter updates: one device-counter update whose global-counter trigger aborts the statement when either budget would be exceeded, proven on D1 with concurrent last-slot and rollback tests. The 20,000 ceiling is `(100,000 - 20,000) / 4`; if a send's complete lifecycle, including outcome writes, index maintenance and cleanup, bills more than four writes, lower it and recompute capacity before launch. No per-send deduplication table or stored payload is budgeted; in-memory event and collapse IDs reduce duplicates, but an accepted send whose response is lost can arrive twice, so never promise exactly-once delivery.

In relay mode, coalesce routine Live Activity updates to the latest state at most once per 60 seconds per agent, sharing the device budget across machines; unchanged state needs only the ten-minute refresh, counted as routine. Approval alerts, clears and ends bypass this cadence but count toward the reserved budget. Direct mode keeps its cadence. A lost routine update must not suppress a needed approval alert.

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

That is 624 send attempts and 12 control requests before retries; thirty such phones use 18,720 attempts, 360 control requests and at most 74,880 send-path D1 writes. The 20,000 ceiling supports about 32 such phones without retries, so start external TestFlight at **30 daily active phones**. The reserved 20,000 D1 writes must also cover control bursts and cleanup, and enrollment CPU and writes need their own measured budget before invitations expand. Eight continuously changing agents followed for eight hours would request 3,840 routine updates; the 700 routine cap deliberately degrades freshness. Without coalescing, one agent at the two-second gap found on 2026-10-16 would cost 14,400 attempts in eight hours, and eight agents 115,200, more than the account's whole daily Worker allowance.

Never enable billing automatically. Review Workers Paid when seven-day peak usage exceeds 70% of the Worker or D1 daily allowances, more than 1% of legitimate sends hit budgets, or forecast use exceeds 30 active phones. A failed CPU validation stops the plan even at one user: optimize, or split enrollment into bounded, individually verified stages, and remeasure without skipping checks; if it still fails, revise the plan before shipping.

## collied behavior and outages

[Architecture](architecture.md#relay-sender) has the shipped sender. The plan:

- Relay mode, exclusive with direct `[apns]`, is the published app's setup default and needs no APNs Keychain item. Custom endpoints stay explicit operator configuration, never picked by the phone for production grants. Self-built apps keep direct setup.
- `collied apns test` exercises the selected sender; `collied doctor` shows relay health, registration and expiry, last send result, per-device budget and backoff or reset time, without credentials. Send failures, not health responses, are authoritative.
- Bounded in-memory exponential backoff with jitter covers network errors, APNs transient failures, 429, relay 5xx and Cloudflare error 1027 (a Cloudflare code, not an HTTP status, possibly without relay JSON), honoring a bounded Retry-After. A known daily exhaustion waits for the reset; short burst limits only back off, and probes and retries never storm.
- Only the newest routine update per activity and bounded unexpired alerts wait, with no disk queue. Each retry rechecks pairing, revocation, route generation and approval expiry; tailnet synchronization repairs missed clears and ends.
- An unavailable relay, Apple or quota degrades only pushes, and doctor explains why; sessions, tailnet approvals, terminal, chat and desktop controls keep working. APNs acceptance never guarantees background execution or display.

## Protocol compatibility before App Store distribution

[Architecture](architecture.md#negotiated-protocol-compatibility) has the shipped rules. The plan: before external TestFlight, create a negotiated baseline, then keep the current and previous two revisions, with no adapters for older exact-match peers, which may need one update to reach it. Peers negotiate the highest common revision and a small capability set, and gate every method, field and enum variant a peer cannot decode, including relay registration and encrypted push formats; a range check in place of equality is not enough. Strict validation and every pairing and authorization check stay. Test app N against daemon N, N-1 and N-2 and the reverse, in both upgrade orders and below the floor. App Store review delay must never strand an installed app when the daemon advances.

## Desktop releases

[Desktop release downloads](desktop-releases.md) has the shipped assets and procedures. The plan:

- Release assets carry checksums, source SHA, protocol range, platform and runtime requirements, and install and update instructions; build inputs and tools are pinned, and packaged artifacts are tested.
- macOS 26+, Apple silicon: Developer ID signed, hardened and notarized collied and CollieBar, stapled where supported and checked with Gatekeeper on a clean Mac, plus a Homebrew formula for collied and a cask for CollieBar.
- Linux x86_64: collied with a stated glibc and runtime floor and a systemd user service, and a CollieTray archive whose instructions name Omarchy 4/Quickshell, `qrencode`, the manifest and all QML, JS and icon files.
- No Intel macOS or Linux arm64 until native builds and packaged smoke tests pass. CollieTray is not a generic GNOME or KDE tray app.
- Signing and notary credentials stay in CI secrets, out of reach of untrusted pull requests; the APNs key is never redistributed.
- One workflow creates the release at the TestFlight tag and SHA. Tags created with `GITHUB_TOKEN` start no tag-push workflow ([GitHub's restriction](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow)), so TestFlight invokes the desktop workflow explicitly; manual dispatch covers desktop-only releases and retries. Never move a tag or publish a version twice.
- Service installers canonicalize `current_exe`, so a Homebrew upgrade must point the existing collied service at the new executable, restarting it only if it was running, rather than leave launchd on a deleted Cellar or Caskroom path. One service label, no competing `brew services` daemon. Test install, upgrade, rollback, uninstall, companion discovery and preservation of node identity, configuration and pairings.

## Simulator and review testing

Simulator support is a follow-up, never a production attestation bypass:

1. In iOS Simulator, run pairing, session and UI tests with the existing software identity keys. Test the Worker and D1 locally with Apple attestation fixtures and invalid, replay and concurrent-budget cases, and mock APNs for failure and outage tests, with no Apple credentials in CI.
2. DEBUG simulator builds get an explicitly provisioned development relay identity: separate secrets, sandbox APNs only, no production enrollment fallback. A local relay may use a maintainer's sandbox APNs credential, never distributed. Supported Macs get real sandbox alert, extension and Live Activity pushes in the simulator; other hosts use mocked transport ([Simulator APNs](https://developer.apple.com/documentation/xcode-release-notes/xcode-14-release-notes), [ActivityKit push testing](https://developer.apple.com/documentation/activitykit/starting-and-updating-live-activities-with-activitykit-push-notifications)). `simctl push` checks rendering only.
3. A physical iPhone validates genuine App Attest, rejection of simulator and development credentials, Secure Enclave behavior, locked and pre-first-unlock decryption, background clears and notification actions; external TestFlight validates production APNs and App Attest. Simulator success cannot satisfy these gates.

Document a repeatable simulator sequence that separates mocked from real sandbox results. The public app and desktop binaries never gain a development trust bypass.

For App Review, Release builds expose a clearly labeled, inert [sample workspace](architecture.md#release-sample-workspace) that needs no Tailscale login or Mac and resets without touching real pairings; developer fixtures and launch flags never become the demo.

The plan starts distribution with a public TestFlight link after Beta App Review, capped at the measured capacity, then App Store review with notes and a real-hardware video; a video alone does not establish compliance. [App Review notes](app-review.md) and [Public submission preparation](public-submission.md) hold the submission material, including the [#389](https://github.com/rbstp/collie/issues/389) connectivity gate and the [guideline assessment](public-submission.md#guideline-assessment).

## Operations and launch gates

Monitor with [free Workers metrics and logs](https://developers.cloudflare.com/workers/observability/logs/workers-logs/): aggregate counts by push class and result, CPU, D1 row costs, quota rejections and APNs errors, only redacted errors, no logging backend, and the account's other Workers and D1 usage.

The APNs key and HMAC signing secret are Worker secrets. Rotate versioned HMAC secrets with an overlap of at most the grant lifetime; the [relay runbook](relay-runbook.md) has the rotation and incident procedures.

Before public distribution, record evidence for every [release gate](public-submission.md#release-gates). Open questions at planning time, assigned to the follow-ups: cold App Attest verification cost, the indexed D1 transaction write count, Free-account limiter availability, clean-machine runtime floors, physical locked-widget behavior and App Review's view of Tailscale and demo access.

## Ordered follow-up work

Tracked in the Phase 7 milestone in this order, all closed. Production relay sending and public enrollment (1) waited for 2 to 4; 4 used 1 and 2, and 5 used 1 to 4; relay-first desktop setup (7, 8) needed 5; desktop packaging and the demo could proceed independently.

1. [Build and measure the free-tier relay (#350)](https://github.com/rbstp/collie/issues/350)
2. [Establish protocol negotiation and the compatibility window (#351)](https://github.com/rbstp/collie/issues/351)
3. [Encrypt every alert and Live Activity display value (#352)](https://github.com/rbstp/collie/issues/352)
4. [Add attested installation registration and per-machine grants (#353)](https://github.com/rbstp/collie/issues/353), with physical production-attestation acceptance
5. [Add collied relay mode and bounded outage handling (#354)](https://github.com/rbstp/collie/issues/354), keeping direct mode
6. [Make the push flow repeatable in iOS Simulator (#355)](https://github.com/rbstp/collie/issues/355), with production rejection tests
7. [Publish signed and notarized macOS daemon and menu app releases (#356)](https://github.com/rbstp/collie/issues/356)
8. [Publish Linux daemon and CollieTray widget releases (#357)](https://github.com/rbstp/collie/issues/357)
9. [Provide a Release-build sample workspace for review (#358)](https://github.com/rbstp/collie/issues/358)
10. [Prepare external TestFlight and App Store submission (#359)](https://github.com/rbstp/collie/issues/359): Beta App Review and a capacity-limited public link before App Store review
