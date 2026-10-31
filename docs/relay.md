# Push relay

The Worker in `relay/` is the publisher push relay at `https://collie-push.rbstp.dev`, on the publisher's Workers Paid account with production APNs. Released builds use it: `collied setup` selects `[push.relay]` by default, and the publisher's distributed app (TestFlight builds) enrolls with App Attest once a paired daemon reports relay mode. `collied` refuses a config with both `[push.relay]` and `[apns]`, and sends only to the fixed `/v1/send` endpoint with the grants a paired phone registered over mutual TLS ([Relay sender](architecture.md#relay-sender)). Self-built apps use direct APNs (`collied setup --direct-apns`, `[apns]`); a daemon in direct mode never triggers enrollment, so nothing reaches the relay. Live Activity update and end through the relay are confirmed on a physical iPhone.

## Development

Use Node 24 or newer. Direct dependencies are exact pins of the latest compatible stable releases as of 2026-10-16, except the beta-only deployment CLI `cf`. Miniflare 4.20260730.0 supports the compatibility date 2026-08-06. Scoped overrides give every Miniflare copy, including the betas inside `cf` and Wrangler, sharp 0.35.5 and Undici 7.30.1, keeping the Undici 7 API Miniflare expects.

```sh
cd relay
npm ci
npm run check
npm test
npm run build
npm run benchmark
```

The `relay` workflow runs the locked install, typecheck, tests and dry-run build for pull requests and `master` pushes that touch `relay/`, on Node 26.12.0 pinned by SHA-256, without Cloudflare credentials. Tests use local workerd D1, real SQLite-backed actors and mocked APNs, and never enroll a real phone or send a push. Fixture registration, ephemeral signing keys and Apple's historical attestation context exist only in test code; production rejects the historical fixture.

## Internal crypto prototype

One internal SQLite-backed Durable Object, `RelayCrypto` under the fixed name `relay-crypto`, verifies App Attest proofs and signs APNs provider tokens over RPC. It has no HTTP handler, writes no storage and keeps only a busy flag and a provider-token cache in memory. D1 owns authorization, replay counters, epochs, grants and quotas.

- Control requests reach the actor only after the rate limiters, the bounded body parser, the operation allowlist and the challenge checks. Verification input is bounded to 16,384 encoded characters and a 32-byte client hash, and the actor rechecks both.
- The actor takes the App ID, environment and minimum bundle version from its own bindings and the time from its own clock: no RPC accepts a caller-chosen trust root, app policy, receipt category or validation time.
- The Worker waits at most five seconds for the actor and returns `503 unavailable` on missing bindings, actor overload, exhausted platform quotas or unavailable compute, with no inline fallback. The actor rejects overlapping operations instead of queueing them, and a caller timeout does not cancel admitted work, which parsing and platform CPU limits still bound.
- [Provider tokens](https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns) are deterministic RFC 6979 ES256 signatures over 45-minute windows, cached in memory by the Worker and the actor and never persisted; the actor signs only for its own clock's window.

A guarded 42-attempt cycle on a Free account measured public Worker CPU up to 6 ms and aggregate actor CPU up to 101 ms, against the default 30 seconds for SQLite actors ([Durable Object limits](https://developers.cloudflare.com/durable-objects/platform/limits/)); its [validation report](reviews/relay-free-tier.md#follow-up-internal-sqlite-crypto-actor-2026-10-16) lists the gaps. A new cloud experiment needs a separate decision and follows `relay/test/cloudflare-crypto-plan.md`.

## Paid plan and cost controls

The publisher account moved to Workers Paid on 2026-10-16. Keep the single crypto actor and D1 design: the actor has its own included request and duration allowances and spares public Worker CPU, and inline verification has not shown a lower bill. [`worker.limits.cpuMs: 250`](https://developers.cloudflare.com/workers/platform/limits/), shared by the Worker and its actor, is a provisional ceiling, not a target or a reserved charge; 100 ms would be below the historical direct-verification p99 of 118 ms and maximum of 134 ms.

Keep the caps of 20,000 sends, 1,000 controls and 30 new enrollments per UTC day; the send cap comes from the Free-plan [capacity plan](app-store.md#free-tier-budget-and-capacity), and the local two-write result below does not justify raising it. The planning model at those caps for 31 days, with one challenge per control and enrollment:

| Resource | Model | Monthly inclusion |
|---|---:|---:|
| HTTP requests | 683,860, plus 744 hourly cleanup invocations | 10 million |
| D1 writes at four per send, 14 per control and nine per enrollment, before cleanup and rejected operations | 2,922,370 | 50 million |
| One fixed 128 MB actor billed continuously (arithmetic, not measured) | 342,835.2 GB-s | 400,000 GB-s |

Idle hibernation-eligible time is free. Workers Paid also includes 30 million Worker CPU ms and one million actor RPCs a month, and the account's other projects, actors and environments share every inclusion ([Workers](https://developers.cloudflare.com/workers/platform/pricing/), [D1](https://developers.cloudflare.com/d1/platform/pricing/) and [Durable Object](https://developers.cloudflare.com/durable-objects/platform/pricing/) pricing).

Besides source and installation keys, the control and enrollment limiters each use one shared key, 30 and five calls a minute per Cloudflare location, counting challenge and commit, so rotating IPs within a location does not evade them. The counters are permissive and eventually consistent, and exhausting a shared key can briefly reject legitimate enrollment or revocation ([rate-limit behavior](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/)). Daily quotas count only committed operations, after verification, so challenges, invalid proofs and a distributed flood still consume requests, CPU, D1 reads and actor RPCs. Neither the quotas nor the CPU ceiling cap spending: keep the ability to disable the public route during abuse.

## Send contract v1

`POST /v1/send` takes `Content-Type: application/json` and `Authorization: Bearer v1.<key_id>.<claims>.<signature>`, with no query string. The authorization value is at most 2,048 characters before decoding, in canonical unpadded base64url segments. HMAC-SHA256 authenticates the exact ASCII bytes `v1.<key_id>.<claims>` before the UTF-8 JSON claims are parsed, so property order does not matter. `key_id` selects one of at most two configured keys. Claims have exactly these fields:

```json
{
  "v": 1,
  "route_id": "opaque_route",
  "generation": 1,
  "installation_id": "opaque_installation",
  "installation_epoch": 0,
  "pairing_id": "opaque_pairing",
  "pairing_epoch": 0,
  "destination_digest": "base64url_SHA256_of_lowercase_destination_hex",
  "push_classes": ["alert", "background"],
  "environment": "production",
  "expires_at": 1794744000
}
```

- Identifiers are 1 to 64 ASCII letters, digits, underscores or hyphens; integers are nonnegative safe JSON integers, and `generation` starts at 1.
- `expires_at` is Unix seconds, in the future and at most 30 days away. `destination_digest` is 43 base64url characters; a send never carries the raw destination.
- An app route permits `alert`, `background` or both; a Live Activity route permits only `liveactivity`.
- `environment` is `production` or `development` and must equal the deployment's. The route, pairing and installation must match the current D1 generation, epochs, digest, environment and expiry.

The body has exactly these fields:

```json
{
  "v": 1,
  "push_type": "alert",
  "priority": 10,
  "expiration": 0,
  "collapse_id": "opaque_terminal",
  "payload": {}
}
```

`priority` is 5 or 10, `expiration` is unsigned 32-bit Unix seconds, and the optional `collapse_id` is an identifier. The Worker rejects unknown fields at any level, a compressed or invalid UTF-8 body, and a body over 8 KiB or still streaming after five seconds, whatever Content-Length says. The serialized APNs payload is at most 4 KiB and must match these allowlists:

| Push type | Payload fields | `aps` fields and values |
|---|---|---|
| `alert` | `aps`, `node_id`, `event_id`, optional `approval_id`, `enc` | Required `alert`, `mutable-content: 1`, `sound: "default"`, `thread-id`; optional `category: "APPROVAL"` |
| `background` | `aps`, `node_id`, `approval_id` | Exactly `content-available: 1`; requires priority 5, expiration 0, no collapse ID |
| `liveactivity` | `aps`, `node_id`, `event_id` | Required `timestamp`, `event: "update"` or `"end"`, `content-state`; optional `stale-date`, `dismissal-date` for end only, `alert`; no collapse ID |

Every alert is exactly `{"title":"Collie","body":"An agent needs attention"}` or `{"title":"Collie","body":"An agent finished"}`. Routing, event, approval and thread identifiers are opaque identifiers. `enc` is canonical base64url of at least a 12-byte nonce, one ciphertext byte and a 16-byte authentication tag, at most 3,800 characters. Live Activity `content-state` is exactly `{"v":1,"enc":"..."}`, and the timestamp and dates are unsigned 32-bit Unix seconds. The relay holds no notification key, so the sender and the phone enforce ciphertext authenticity and encryption. `npm test` checks the shared Rust and Swift display vectors in `docs/protocol/push-display-vectors.json` against these allowlists.

The Worker chooses only Apple's production or sandbox host, the registered destination and the fixed topic, `dev.rbstp.collie` or `dev.rbstp.collie.push-type.liveactivity`, and builds every header itself. It follows no redirects, waits at most 10 seconds for Apple and reads at most 1 KiB of Apple's response. Responses are `{ "v": 1, "result": "..." }`, with `Cache-Control: no-store`:

| HTTP | Result | Meaning |
|---|---|---|
| 200 | `accepted` | Apple accepted the attempt; display is not confirmed |
| 400 | `invalid_request` | Body, envelope or public payload rejected |
| 401 | `unauthorized` | Invalid, expired, wrong-environment or wrong-class grant |
| 404 | `not_found` | Unknown method, path or query string |
| 410 | `invalid_route` | Revoked/expired/mismatched registration, or matching Apple invalidation |
| 429 | `rate_limited` | Soft limiter, includes `retry_after: 60` |
| 429 | `daily_quota` | No reserved slot, includes next UTC midnight as `reset_at` Unix seconds |
| 502 | `configuration_error` | APNs configuration/other rejection; registration retained |
| 503 | `upstream_unavailable` | Apple 5xx includes `retry_after: 900`; network, timeout or Apple 429 includes `retry_after: 60` |
| 503 | `unavailable` | D1, secret or other internal failure; fails closed |

Apple's `410 ExpiredToken`, `410 Unregistered` and `400 BadDeviceToken` invalidate the exact route generation and destination digest; once that commits, later attempts stop before APNs, and a failed or timed-out invalidation returns `503 unavailable`. A late failure never erases a replacement, even a newer generation for the same token, and invalidation deletes nothing before [retention cleanup](#retention-and-cleanup) ([Apple's response guidance](https://developer.apple.com/documentation/usernotifications/handling-notification-responses-from-apns)). The Worker never retries, and never returns APNs reason strings, response headers, destinations or payloads. A response lost after Apple accepts can make the daemon's bounded retries deliver twice.

## Atomic quotas and storage ownership

Only attested enrollment creates an installation, with its attestation and zeroed `installation_usage` row, in one [batch](https://developers.cloudflare.com/d1/worker-api/d1-database/); authenticated route registration creates a pairing and route in one guarded batch.

A send charges quota with one indexed UPDATE of the installation counter, which rechecks route, pairing and installation authorization and takes expiry and day boundaries from the later of the request clock and D1's UTC clock. Its trigger charges the singleton global counter and aborts the whole statement when global capacity is exhausted. Counters reset lazily at UTC midnight, and a delayed previous-day invocation cannot move them backward. Never use INSERT ON CONFLICT with a BEFORE INSERT quota trigger: both insert and update triggers could charge one upsert. The Worker rechecks grant validity after parsing and signing and again after the charge; a grant that expires during that last wait stops delivery without a refund.

| Attempts per UTC day | Total | Routine |
|---|---:|---:|
| Per installation, across all its machines and activity routes | 1,000 | 700 |
| Global | 20,000 | 16,000 |

Only a priority-5 Live Activity `update` without an alert is routine. Everything else, including priority-10 updates such as silent approval clears, alerts and activity ends, counts only against the total. Every accepted attempt is charged before forwarding, including network ambiguity and Apple rejections, and never refunded. A definite quota rejection whose response crosses UTC midnight is retried once against the new day; successful or ambiguous charges never are, and quota responses report the reset of the window attempted. Soft IP, installation and route rate limits run first and never replace the exact counters. A revocation after the final charge can race an already authorized APNs attempt.

Workers Paid allows [10 GB per database](https://developers.cloudflare.com/d1/platform/limits/) and processes a database's queries one at a time. Send lookups use primary-key indexes, every D1 wait is bounded to five seconds, and local tests show 20 concurrent last-slot attempts commit exactly one charge and roll back every rejected counter change.

## Measurements and release gates

In the [deployed Free-plan report](reviews/relay-free-tier.md), the direct-Worker verifier failed the CPU gate: 198 of 400 full Apple verifications hit the CPU limit. The response was the crypto actor on Workers Paid, with no verification check removed or weakened.

A 2026-10-16 macOS run of 100 samples measured Node wall time, not deployed Worker CPU:

| Operation | Median wall time | p99 wall time |
|---|---:|---:|
| New deterministic provider signer | 0.639 ms | 1.378 ms |
| Warm cached provider JWT | 0.000375 ms | 0.00221 ms |
| Full mocked send through local D1 | 21.166 ms | 143.555 ms |

Local D1 metadata reports 10 rows read and 2 written per accepted send, 1 and 1 for route invalidation, and 1 and 1 each for explicit route and counter deletion: local observations, not production billing, index maintenance or complete lifecycle costs. `npm run benchmark` reproduces them.

A 2026-10-16 local Node run of `npm run benchmark:attestation` verified the complete historical Apple attestation and CMS receipt 100 times: 21.946 ms for the first verification, 9.133 ms median and 12.039 ms p99 wall time. That includes local scheduling and WebCrypto waits, is not Worker CPU, and misses the deployed gate of a p99 below 8 ms: keep optimizing, never skip receipt or chain checks.

Local, mock and Free-plan results do not replace the [release gates](public-submission.md#release-gates): on the intended, confirmed Paid account, with genuine valid and invalid App Attest fixtures, cold and warm enrollment, assertions, JWTs, full sends and hostile input must show zero CPU-limit errors and headroom below the CPU ceiling. Still unmeasured: production CPU distributions, actor duration and capacity, remote D1 concurrency, quota rollback, complete-lifecycle costs, populated retention and throughput, burst capacity and shared account headroom.

## Operations

`relay/cloudflare.config.ts` is the deployment template; the [runbook](relay-runbook.md#deployment) covers its bindings, secrets, the `cf` deploy and [HMAC](relay-runbook.md#hmac-secret-rotation) and [APNs key](relay-runbook.md#apns-key-rotation) rotation. Inspect the `cf build` output before deploying, never put secret values in committed vars, command arguments or logs, and keep workers.dev and preview URLs disabled. Wrangler only builds local bundles and dry runs. Any other isolated deployment needs its own D1 database with the schema and an authenticated access boundary, and one for Debug builds, whose bundle version is 1, needs `ATTEST_MIN_BUNDLE_VERSION` 1. After each deployment, verify the limiter bindings, CPU setting and Cron trigger, and recheck cold and warm CPU.

The relay uses the single-level Worker Custom Domain `collie-push.rbstp.dev`, which Universal SSL covers; `collie.rbstp.dev` stays the DNS-only GitHub Pages CNAME. `push.collie.rbstp.dev` would need its own certificate coverage. Before public TestFlight or App Store distribution, review edge protection and route shutdown: distributed traffic can exceed the inclusions despite soft controls.

Invocation observability is disabled, and the source logs no requests or payloads. Keep headers, authorization, bodies, destination URLs and APNs response bodies out of platform logs, traces and dashboards; watch only aggregate outcome codes and account-wide Worker and D1 metrics. Reconsider the design at seven-day peak usage above 70%, legitimate budget rejection above 1%, or forecast use above 30 daily active phones.

Cloudflare sees source IPs, raw APNs destinations, route relationships, generic alerts, opaque identifiers, timing, frequency and ciphertext size. Apple receives APNs metadata and ciphertext. Neither gets notification decryption keys. D1 stores registration metadata, revocation state and counters, with no notification payloads, provider JWTs or approval authority, until [Retention and cleanup](#retention-and-cleanup) deletes them.

## Attested control contract

`POST /v1/challenge` takes one operation. `POST /v1/control` takes exactly `operation`, `challenge`, and either `attestation` for a new installation or `assertion` for an enrolled key. Both reject query strings, unknown fields, compressed bodies, invalid UTF-8, and bodies over 16 KiB or still streaming after five seconds; replies never include destinations, receipts or verification details. Missing limiter or bundle-version configuration fails closed.

Every operation contains `v: 1`, `op`, an opaque `installation_id`, a `key_id` (canonical base64url of 32 bytes) and `environment`:

| Operation | Additional fields |
|---|---|
| `enroll` | None |
| `route` | `pairing_id`, `route_id`, `expected_generation`, `kind` (`app` or `liveactivity`), `destination` (lowercase hex) |
| `revoke_pairing` | `pairing_id` |
| `revoke_route` | `pairing_id`, `route_id`, `expected_generation` |

The challenge, `c1.<key_id>.<claims>.<signature>`, holds a random 32-byte nonce, the digest of the canonical operation (sorted ASCII property names, compact JSON), the installation and key, the current durable counter and epoch, and a five-minute expiry. HMAC-SHA256 with the send-grant keys authenticates its first three segments, under the separate `c1` domain. Issuing a challenge writes nothing to D1, and the phone passes the whole challenge's SHA-256 to App Attest. Durable key uniqueness protects the first enrollment. Every later operation must present the challenge's exact prior counter and epoch in the D1 transaction that advances the counter and changes authorization, so a used challenge fails even when re-signed with a larger counter. A rejected trigger aborts every write, including the global and installation control quotas.

The verifier trusts only the pinned Apple App Attestation root and, for the initial CMS receipt, Apple Root CA G3. Bounded CBOR and ASN.1 parsing rejects duplicate keys and trailing objects.

- Attestation: certificate validity, issuer signatures, CA, path-length and key-usage constraints, duplicate or unknown critical extensions, the leaf nonce, agreement of COSE key, public key and key ID, the exact publisher App ID, a zero initial counter, the credential ID and the attestation environment.
- Receipt: the independent CMS signature and G3 chain, the App ID, the attested certificate, the client hash, the `ATTEST` type and creation within five minutes. No receipt or key is trusted because the client supplies it.
- Assertion: the ECDSA signature over the SHA-256 nonce of authenticator data plus client-data hash, the App ID and a strictly increasing counter. An assertion may keep Apple's AT flag without credential data, but unexpected credential bytes and trailing data are rejected.
- Authenticator extensions, parsed whenever present, even without the extensions flag: production accepts only TestFlight and App Store launch categories 2 and 4, development only category 3, and the bundle version must be a decimal whole number of at most nine digits, at or above `ATTEST_MIN_BUNDLE_VERSION`. Without them, every certificate, app and environment check still applies.

Grants last 30 days, and the phone renews them from seven days before expiry. A pairing has one app route and independent activity routes; replacement generations and conditional revocation keep old failures from affecting a new registration. A genuine APNs registration callback or Live Activity token event renews its relay generation even when the token is unchanged, which recovers from server invalidation; polling and reconnects reuse fresh grants. A fresh assertion can recover a lost registration response, but only for the same next generation, destination and kind with the original expiry; an enrolled key recovers an ambiguous enrollment response the same way, never with a second, unverified attestation.

A normal renewal costs one control operation (two HTTP requests, one assertion, four SQL reads and a four-statement batch, by source count), so an app registration event with eight paired machines needs eight.

Per installation, the schema allows eight active machines and eight active activities, and bounds tombstones to 64 pairing and 128 route records until retention cleanup. Daily control limits are 128 per installation and 1,000 globally, and enrollments 30 globally, separate from the send quotas. An exhausted allocation or control budget fails closed with `route_limit`.

On the phone:

- Challenge, App Attest and commit run under an explicit FIFO gate, because Swift actors can reenter during awaits.
- A ThisDeviceOnly Keychain record holds the installation and key ID, exact grants and minimal pending revocations; a backup-excluded installation marker stops a new install from reusing it.
- The transport pins `https://collie-push.rbstp.dev`, disables cookies, cache and redirects, bounds responses and never takes a relay URL from a machine. Without App Attest support there is no enrollment fallback; tailnet sessions are unaffected.
- Before posting a route proof, the phone durably records the challenge expiry plus 30 days as the latest possible grant expiry, so the bound survives a paused App Attest or a lost response.
- Removing a pairing keeps its revocation authority before discarding grants. An offline revocation stays pending, and the machine can send until it succeeds or the last grant expires.
- A phone inactive past its grant expiry gets no relay notifications until it renews; its tailnet sessions keep working.
- Enrollment needs a paired daemon that negotiates both `relay_grants_v1` and `encrypted_display_v1` and reports relay mode. A phone whose server state is missing or expired enrolls afresh only once every known grant and ambiguous registration has expired; an invalid App Attest key requires pairing again.
- Wire pairing IDs hash the installation and the confirmed local pairing, so a fresh installation never collides with retained old pairings.

### Retention and cleanup

Superseding a route deletes its previous destination in the same transaction. An expired or revoked route becomes eligible for deletion seven days after its grant expiry; an installation's pairing tombstones, usage counter, key, receipt and replay record, seven days after its last grant expiry and no earlier than 37 days after enrollment.

The hourly Cron trigger deletes eligible records oldest first, at most 64 routes, 4,096 pairings (64 installations' worth of 64 tombstones), 64 usage counters and 64 installations per run; keys, receipts and replay records cascade. Each control or enrollment creates at most one route and sets at most one installation's deadline, so the daily global caps of 1,000 controls and 30 enrollments bound eligibility. The worst case, two days made eligible at once across one UTC midnight, is 2,000 routes and 2,060 installations holding 131,840 pairing tombstones. It drains in 32 runs for routes, 33 for pairings and 33 for counters and installations, so with up to an hour before the first run every record is deleted within 99 hours (about four days) of becoming eligible, inside the seven-day backlog bound; `npm test` checks this worst case in local D1. A sustained day at the caps (1,000 routes, 65,920 pairings, 1,030 installations) fits 24 runs' capacity (1,536, 98,304, 1,536). Live records are therefore gone within 14 days of the last grant expiry; a missed or failed run only delays deletion.

D1 [Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/) is always on and keeps 30 days of restorable history on Workers Paid, so a deleted record stays restorable for up to 30 days.
