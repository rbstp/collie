# Push relay

The Worker in `relay/` is the publisher push relay. Its production deployment on the publisher's Workers Paid account serves `https://collie-push.rbstp.dev` with production APNs. Released builds deliver through it: `collied setup` selects `[push.relay]` by default, and the publisher's distributed app (TestFlight builds) enrolls with App Attest once a paired daemon reports relay mode. Self-built apps keep direct APNs (`[apns]`) and send nothing to the relay. Live Activity update and end through the relay are confirmed on a physical iPhone. The checked-in configuration is the deployment template: the owner deploys it with `cf`, adding the real D1 database ID, the production APNs environment and the `collie-push.rbstp.dev` Custom Domain outside the committed file. A separate guarded synthetic benchmark was deployed temporarily and removed; [the Free-plan CPU gate failed](reviews/relay-free-tier.md), which is why the relay runs on Paid.

## Development

Use Node 24 or newer. `relay/package-lock.json` locks tooling and dependency integrity; direct versions are exact pins checked on 2026-10-16, using the latest compatible stable releases and the existing beta-only `cf` tool. The compatibility date is 2026-08-06, supported by the pinned stable Miniflare runtime. The pinned local bundler includes a newer beta Miniflare separately. Scoped overrides patch every Miniflare copy to sharp 0.35.5 and Undici 7.30.1 while retaining its compatible Undici 7 API; Miniflare 4.20260730.0 remains the latest stable release. The deployment CLI is `cf` 1.0.0-beta.18, selected explicitly for this project.

```sh
cd relay
npm ci
npm run check
npm test
npm run build
npm run benchmark
```

The separate `relay` GitHub Actions workflow runs the locked dependency install, typecheck, regression tests and dry-run build on relay pull requests and master changes. It pins Node 26.12.0 with the official archive SHA-256 and requires no Cloudflare credentials or deployment.

Tests use local workerd D1 and mocked APNs. They never enroll a real phone or send a push. The benchmark generates an ephemeral test signing key and uses local fixture registration. Fixture provisioning exists only in test code; the deployed source has no registration bypass, administrative fixture endpoint or test signing key.

## Internal crypto prototype

The local prototype uses one internal SQLite-backed `RelayCrypto` Durable Object, selected by the fixed name `relay-crypto`. The production entrypoint is `relay/src/worker.ts`; it exports the ordinary public Worker and this class. The actor has RPC methods for full attestation, signed assertions and deterministic provider tokens, with no HTTP handler or storage writes. It retains only its busy flag and provider-token cache in memory. D1 continues to own authorization, replay counters, epochs, grants and quotas. The verifier and pinned Apple roots are unchanged.

Control requests pass the existing source/installation/enrollment limiters, bounded body parser, operation allowlist and authenticated challenge checks before invoking the actor. Encoded verification inputs are bounded to 16,384 characters and client hashes to 32 bytes before actor resolution; the actor repeats these checks. Publisher App ID, environment and minimum bundle version come from the actor's trusted bindings. Attestation validity uses the actor's current wall clock. No RPC accepts a caller-selected trust root, app policy, receipt category or validation time. Assertion counters are still committed by the existing atomic D1 guard after verification.

The public Worker bounds crypto waits to five seconds and returns `503 unavailable` on missing bindings, actor overload, exhausted platform quotas or unavailable compute. It has no inline verification/signing fallback. The actor rejects overlapping admitted operations instead of maintaining an application queue. A caller timeout does not cancel already admitted computation; parsing limits and the platform CPU limit still apply. Native platform queuing can occur before the actor receives a call. APNs tokens retain their existing 2,700-second deterministic windows; both actor and public Worker cache them, and the actor rejects a requested signing window different from its own clock.

`npm test` exercises real SQLite-backed actors in local workerd, actual assertion signatures and D1 replay rejection, mocked APNs delivery, input bounds and fail-closed compute errors. The separate test bundle uses Apple's historical fixture with a fixed historical publisher/time/category context; production RPCs reject that fixture under Collie's current policy. Its signer receives a test-only asynchronous delay to make overlap admission deterministic even when workerd resolves WebCrypto immediately. These test classes and context are absent from the production entrypoint.

A separately guarded 42-attempt Free-account cycle completed and its temporary resources were removed; the [validation report](reviews/relay-free-tier.md#follow-up-internal-sqlite-crypto-actor-2026-10-16) records observed public Worker CPU up to 6 ms, aggregate actor CPU up to 101 ms and the remaining gaps. The original failed direct-Worker result remains valid. Official [Durable Object limits](https://developers.cloudflare.com/durable-objects/platform/limits/) allow a default 30 seconds of CPU for SQLite actors; [Free pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/) provides 100,000 requests and 13,000 GB-s per day and fails at exhausted quotas. Production CPU distributions, complete actor duration/capacity and D1 lifecycle costs, and fresh Collie enrollment still require validation. The checked-in configuration creates no public hostname or production enrollment rollout.

Any separately approved cloud experiment must initialize one fresh shared benchmark journal explicitly, with its reviewed attempt limit, then use that same existing file for every probe, resumed client and network failure. Initialization refuses existing files or locks, and resumed clients never recreate missing state. The journal persists its immutable per-experiment limit, at most 1,000; the proposed crypto experiment uses 50. Local tests prove that resuming or reinitializing cannot raise that limit. A retired temporary actor namespace is removed by deploying code without its class/binding and an explicit `exports.durableObject({ state: 'deleted' })` tombstone, then verifying namespace absence before deleting its exact owned Worker. See [Cloudflare's class lifecycle contract](https://developers.cloudflare.com/durable-objects/reference/durable-objects-migrations/).

## Paid plan and cost controls

The publisher account moved to Workers Paid on 2026-10-16. Keep the reviewed single crypto actor and D1 design: it uses separate included crypto request/duration allowances and preserves public Worker CPU headroom. The historical Free tests above remain measurements of that plan, not a production acceptance result. Moving verification inline has not demonstrated a lower bill.

The checked-in configuration sets `worker.limits.cpuMs: 250`, shared by its public Worker and associated crypto actor. This is a provisional active CPU ceiling, not a target or reserved charge. A 100 ms ceiling is below the historical direct-verification p99 of 118 ms and maximum of 134 ms. Recheck the deployed limit and cold/warm CPU after each deployment; neither the configuration nor the small actor test proves fresh enrollment fits. Full Apple verification, input bounds, five-second crypto waits and fail-closed behavior remain unchanged.

Keep the existing 20,000 sends, 1,000 controls and 30 new enrollments per UTC day. At those committed-operation ceilings for 31 days, with one challenge per control/enrollment, the model has 683,860 HTTP requests plus 744 hourly cleanup invocations. Four writes per send, 14 per control and nine per enrollment total 2,922,370 D1 writes before cleanup and rejected operations. These are planning assumptions, not complete measured monthly usage. Workers Paid includes 10 million Worker requests, 30 million Worker CPU ms and 50 million D1 writes per month, shared with other account projects. [Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/), [D1 pricing](https://developers.cloudflare.com/d1/platform/pricing/).

One fixed 128 MB actor, even continuously billable for 31 days, represents 342,835.2 GB-s against 400,000 included GB-s. Idle hibernation-eligible time is free. This is arithmetic, not measured duration; other account actors, duplicate environments or additional instances consume the same pool. Actor RPCs have a separate one-million monthly inclusion. Preserve the single fixed name and no application storage. [Durable Object pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/).

The control and enrollment limiters also use fixed shared keys, respectively 30 and five calls per minute per Cloudflare location, in addition to source and installation keys. Challenge and commit each consume admission. Rotating IPs in one location does not bypass that shared allowance. Counters are permissive and eventually consistent, so this is coarse protection rather than exact global accounting. Shared exhaustion can temporarily reject legitimate enrollment or revocation before D1 or crypto. [Rate-limit behavior](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/).

Committed-operation quotas do not cap challenges, invalid proofs or all billed HTTP requests. Enrollment and assertion verification precede committed quota checks. A distributed flood can still consume requests, CPU, D1 reads and actor RPCs. Neither this CPU ceiling nor application quotas provide a dollar spending cap. Keep observability disabled, inspect aggregate account usage before increasing invitations, and retain the ability to disable the public route during abuse. Burst capacity and shared account headroom remain operational checks; maximum-cardinality cleanup is covered under [retention](#retention-and-cleanup).

## Send contract v1

`POST /v1/send`, with no query string, `Content-Type: application/json`, and `Authorization: Bearer v1.<key_id>.<claims>.<signature>`. The authorization value is bounded to 2,048 characters before decoding. Segments use canonical unpadded base64url. `claims` is UTF-8 JSON; HMAC-SHA256 authenticates the exact ASCII `v1.<key_id>.<claims>` bytes. JSON property order is not significant because verification authenticates the encoded bytes before parsing. `key_id` selects exactly one key from at most two configured keys. Tokens never belong in paths or queries.

Claims have exactly these fields:

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

Identifiers use 1 to 64 ASCII letters, digits, underscores or hyphens. Integers are nonnegative safe JSON integers; generation starts at 1. Expiry is Unix seconds and must be future and at most 30 days away. The destination digest is 43 base64url characters. An app route permits `alert`, `background`, or both. A Live Activity route permits only `liveactivity`. Environment is `production` or `development` and must equal the deployment's configured environment. The route, pairing and installation must match the current D1 generation, epochs, digest, environment and expiry. The raw destination is never supplied in a send request.

The exact body fields are:

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

`priority` is 5 or 10. `expiration` is unsigned 32-bit Unix seconds. Optional `collapse_id` follows the identifier syntax. The incoming stream is bounded to 8 KiB and five seconds regardless of Content-Length, rejects compressed or invalid UTF-8 bodies, and bounds the serialized APNs payload to 4 KiB. Unknown fields are rejected at every public envelope level.

Public payload allowlists:

| Push type | Payload fields | `aps` fields and values |
|---|---|---|
| `alert` | `aps`, `node_id`, `event_id`, optional `approval_id`, `enc` | Required `alert`, `mutable-content: 1`, `sound: "default"`, `thread-id`; optional `category: "APPROVAL"` |
| `background` | `aps`, `node_id`, `approval_id` | Exactly `content-available: 1`; requires priority 5, expiration 0, no collapse ID |
| `liveactivity` | `aps`, `node_id`, `event_id` | Required `timestamp`, `event: "update"` or `"end"`, `content-state`; optional `stale-date`, `dismissal-date` for end only, `alert`; no collapse ID |

Every alert is exactly `{"title":"Collie","body":"An agent needs attention"}` or `{"title":"Collie","body":"An agent finished"}`. Routing, event, approval and thread identifiers are opaque and use the identifier syntax. `enc` is canonical base64url of at least a 12-byte nonce, one ciphertext byte and 16-byte authentication tag, at most 3,800 encoded characters. Live Activity `content-state` is exactly `{"v":1,"enc":"..."}`. Timestamp and optional dates are unsigned 32-bit Unix seconds. Ciphertext authenticity and semantic encryption are enforced by the phone and sender, since the relay has no notification key. This contract intentionally does not accept today's cleartext direct-mode payloads. The encryption implementation must supply shared fixtures before these allowlists become a production interoperability guarantee.

The Worker chooses only Apple's production or sandbox hostname, the registered destination, and the fixed topics `dev.rbstp.collie` or `dev.rbstp.collie.push-type.liveactivity`. It constructs all headers. It never accepts a caller's URL, topic, provider authorization or extra APNs headers, and never follows redirects. Upstream wait is bounded to 10 seconds and response bodies to 1 KiB.

Responses are `{ "v": 1, "result": "..." }`, with `Cache-Control: no-store`:

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

Apple's `410 ExpiredToken`, `410 Unregistered` and `400 BadDeviceToken` invalidate the exact matching route generation and destination digest. Once revocation commits, subsequent attempts fail before contacting APNs; a failed or timed-out invalidation returns `503 unavailable` without claiming durable revocation. Late failures cannot erase a replacement, including a newer generation for the same token. Invalidation retains the route tombstone and destination until the documented retention cleanup; it does not delete an installation or unrelated routes. See [Apple's response guidance](https://developer.apple.com/documentation/usernotifications/handling-notification-responses-from-apns). APNs reason strings, response headers, destinations and payloads are never returned. The Worker does not retry. The daemon uses bounded in-memory retries and does not guarantee exactly-once delivery. A lost response after Apple accepts can lead to duplicate delivery if the sender retries.

## Atomic quotas and storage ownership

`relay/migrations/0001_relay.sql` owns the foundation tables. `relay/migrations/0002_attestation.sql` adds verified keys, receipts, atomic replay/control counters and bounded lifecycle state. Enrollment creates the installation, attestation and zeroed `installation_usage` rows in one batch; authenticated route registration creates its pairing and route in one guarded batch. There is no provisioning command; only attested enrollment creates an installation. Schema triggers enforce the eight-machine/eight-activity limits, and [Retention and cleanup](#retention-and-cleanup) bounds and deletes registration records.

A single indexed installation-counter UPDATE rechecks current route, pairing and installation authorization and uses the later of the current request clock and D1 UTC clock for expiry and day boundaries. The Worker refreshes grant validity after body parsing and signing, and immediately before forwarding after the bounded D1 charge await. An expiry during that final await stops delivery without refunding the reservation. Route revocation or expiry during quota work returns `invalid_route`; only a valid route with exhausted capacity returns `daily_quota`, and a missing counter returns `unavailable`. Its trigger updates the singleton global counter, aborting the entire statement when global capacity is exhausted. No per-send counter record is inserted, and no daily counter deletion is needed: each row resets lazily at UTC midnight. Delayed previous-day invocations cannot move either counter backward. Registration must pre-create its zeroed counter row; missing counters fail closed. Never use INSERT ON CONFLICT with a BEFORE INSERT quota trigger, since both insert and update triggers could charge an upsert.

The installation limits are 1,000 total and 700 routine attempts per UTC day, across all paired machines and activity routes. Global limits are 20,000 total and 16,000 routine attempts. A priority-5 Live Activity `update` without an alert is routine. Priority-10 updates, including silent approval clears, use the reserved total budget. Alerts, clears and activity ends can use the reserved remainder. All accepted attempts are charged before forwarding, including network ambiguity and Apple rejections. Counters are never refunded. A definitely rejected quota charge whose response crosses UTC midnight is re-evaluated once against the fresh day; successful or ambiguous charges are never retried. Quota responses retain the final attempted window rather than deriving a later reset from response time. Soft IP/installation/route rate limits precede D1 quota work; they do not replace exact counters. Revocation after the final atomic charge may race an already authorized in-flight APNs attempt.

D1 stores registration, authorization and counters rather than notification history. Workers Paid permits [10 GB per database](https://developers.cloudflare.com/d1/platform/limits/), with queries processed one at a time per database. This storage limit is separate from included account storage and does not cap spending. Indexed queries, bounded records and batched cleanup reduce work; populated retention and throughput still require measurement.

Send lookups use primary-key indexes. The local suite proves 20 concurrent last-slot attempts allow exactly one commit and roll back all rejected counter changes, total/routine reservation, UTC reset, held streams crossing expiry/midnight, expiry during the charge response, stream duration limits, expired/revoked routes and an epoch change between lookup and charge. Signing and D1 waits have five-second bounds. Production D1 concurrency and billed costs still require remote verification. Bounded indexed expiry cleanup and enrollment costs belong to the separate production lifecycle and cost validation.

## Measurements and release gates

The [deployed Free-plan report](reviews/relay-free-tier.md) records actual CPU-limit failures, D1 lifecycle costs, a request-accounting failure and verified resource cleanup.

A 2026-10-16 macOS run, 100 samples, measured Node wall time, not deployed Worker CPU:

| Operation | Median wall time | p99 wall time |
|---|---:|---:|
| New deterministic provider signer | 0.639 ms | 1.378 ms |
| Warm cached provider JWT | 0.000375 ms | 0.00221 ms |
| Full mocked send through local D1 | 21.166 ms | 143.555 ms |

Local D1 metadata reports 10 rows read and 2 rows written per accepted send, 1/1 for route invalidation, and 1/1 each for explicit route and counter deletion. These are local runtime observations, not verified production billing, index-maintenance or complete lifecycle costs. Reproduce with `npm run benchmark`; timings depend on the host and load.

The original Free-plan capacity model used the conservative four-write launch gate: `(100000 - 20000) / 4 = 20000` attempts/day supports `floor(20000 / 624) = 32` phones at the plan's 624-send/12-control example. Thirty phones consume 18,720 attempts, at most 74,880 send writes and 360 controls, leaving the reserved 20,000 writes for enrollment, controls, failed quota work, cleanup and other account use. The local two-write result does not justify raising that ceiling. Record remote outcome writes, index costs, zero-counter creation, enrollment/assertion replay updates, expiry cleanup and other account traffic before claiming the reserve is sufficient. Lower the SQL global caps and recompute capacity if complete-lifecycle send costs exceed four billed writes.

Production deployment checks: deploy only to the intended confirmed Paid account; verify the explicit CPU limit and rate-limit bindings; use genuine valid/invalid App Attest certificate, nonce, app/environment and replay fixtures; measure cold/warm enrollment, assertions, JWTs, full sends and hostile input with sufficient headroom below the configured CPU ceiling and zero CPU-limit errors; measure remote D1 complete lifecycle and quota rollback. No attestation CPU result is claimed by these local measurements.

## Operations

The `cloudflare.config.ts` configuration has the hourly cleanup Cron trigger (`0 * * * *`), no workers.dev or preview URL, a placeholder D1 database ID, the development APNs environment and the production minimum app build number, `ATTEST_MIN_BUNDLE_VERSION`. The production deployment supplies the database ID, APNs environment and Custom Domain outside this file; never commit them. Any other isolated deployment needs its own D1 database with the schema applied and an authenticated access boundary; a development deployment for Debug builds, whose bundle version is 1, also needs a minimum of 1. Keep workers.dev/preview URLs disabled. Verify the deployed limiter bindings, CPU setting and Cron trigger after each deployment.

Use `cf` for Cloudflare operations and deployments. Unset inherited `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_API_KEY` and `CLOUDFLARE_EMAIL` so they cannot override the active `cf` OAuth login; run `cf auth whoami` and explicitly set the verified `CLOUDFLARE_ACCOUNT_ID`. Provision `APNS_TEAM_ID`, `APNS_KEY_ID`, `APNS_PRIVATE_KEY` and `SEND_HMAC_KEYS` in a private JSON file with mode 0600, passed as `cf deploy --prebuilt --no-provision --secrets-file /secure/path/secrets.json`, never committed vars, command arguments containing secret values or logs. Build with `cf build` and inspect its output before deploying. `SEND_HMAC_KEYS` is a JSON map from key ID to a canonical base64url 32-byte random key, with at most two active keys. New grants use the new ID; retain the old verification key only until its last 30-day grant expires, then remove it. Emergency removal immediately rejects that key's grants. APNs key rotation deploys the new key ID/private key together and revokes the old Apple key only after validation. Provider tokens are deterministic RFC 6979 ES256 in 45-minute windows, cached in the public isolate and the single crypto actor, never persisted.

The relay uses the single-level Worker Custom Domain `collie-push.rbstp.dev`; keep `collie.rbstp.dev` as the DNS-only GitHub Pages CNAME. Universal SSL covers the former. Do not use `push.collie.rbstp.dev` without suitable certificate coverage. The current source requires its limiter bindings and fails closed without them. Review edge protection and route shutdown before public TestFlight or App Store distribution; distributed traffic can exceed included account usage despite soft controls.

Invocation observability is disabled and source code logs no requests or payloads. Keep request headers, authorization, bodies, destination URLs and APNs response bodies out of platform logging, tracing and dashboards. Use only aggregate outcome codes and account-wide Worker/D1 metrics. Diagnose 401 as grant/key/environment, 410 as route/renewal, 429 as limiter or UTC quota, 502 as APNs configuration, and 503 as D1/secret/upstream availability. Inspect account-wide traffic before increasing invitations: other Workers, actors and D1 databases share Paid allowances. Reconsider the design at seven-day peak usage above 70%, legitimate budget rejection above 1%, or forecast use above 30 daily active phones.

Cloudflare sees source IPs, raw APNs destinations, route relationships, generic alerts, opaque identifiers, timing, frequency and ciphertext size. Apple receives APNs metadata and ciphertext, but neither party gets notification decryption keys. D1 stores registration metadata, revocation state and counters, with no notification payloads, provider JWTs or approval authority. [Retention and cleanup](#retention-and-cleanup) bounds how long it keeps them.

Sources: [Cloudflare Worker limits](https://developers.cloudflare.com/workers/platform/limits/), [D1 pricing and row accounting](https://developers.cloudflare.com/d1/platform/pricing/), [D1 transactions](https://developers.cloudflare.com/d1/worker-api/d1-database/), [rate-limit binding](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/), [Apple provider tokens](https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns).

## Attested control contract

`POST /v1/challenge` accepts one operation object. `POST /v1/control` accepts exactly `operation`, `challenge`, and either `attestation` for a new installation or `assertion` for an enrolled key. Both paths reject query strings, unknown fields, compressed bodies, invalid UTF-8 and more than 16 KiB or five seconds of streaming. Replies never include destinations, receipts or verification details. The source-IP and installation control limiter and enrollment limiter run before expensive verification. Missing limiter or bundle-version configuration fails closed.

Every operation contains `v: 1`, `op`, opaque `installation_id`, canonical base64url 32-byte `key_id` and `environment`. Operations are:

| Operation | Additional fields |
|---|---|
| `enroll` | None |
| `route` | `pairing_id`, `route_id`, `expected_generation`, `kind` (`app` or `liveactivity`), lowercase destination hex |
| `revoke_pairing` | `pairing_id` |
| `revoke_route` | `pairing_id`, `route_id`, `expected_generation` |

Canonicalization sorts the operation's ASCII property names and serializes compact JSON. The server returns `challenge: c1.<key_id>.<claims>.<signature>`, with a random 32-byte nonce, canonical operation digest, installation/key identity, current durable counter/epoch and five-minute expiry. HMAC-SHA256 authenticates the exact first three segments with the same bounded key set as send grants, under the distinct `c1` domain. The phone passes SHA256 of the complete challenge string to App Attest. Issuing challenges allocates no unauthenticated D1 records. Durable key uniqueness protects initial enrollment. Later operations require the challenge's exact prior counter and epoch in the same D1 transaction that advances the counter and changes authorization. Re-signing a used challenge with a larger counter still fails. A rejected trigger aborts every write, including global and installation control quotas.

The verifier trusts only the pinned Apple App Attestation root and, independently for initial CMS receipts, Apple Root CA G3. It checks certificate validity, issuer signatures, CA/path-length and key-usage constraints, duplicate/unknown critical extensions, leaf nonce, COSE/public-key/key-ID agreement, exact publisher App ID, zero initial counter, credential ID and attestation environment. Assertions verify the ECDSA signature over the SHA-256 nonce of authenticator data plus client-data hash, the App ID and a strictly increasing counter. The parser accepts Apple assertions that retain the AT flag without credential data; it still rejects unexpected credential bytes and trailing data. Bounded CBOR/ASN.1 parsing rejects duplicate keys and trailing objects. Documented authenticator extensions are parsed when present, including Apple's published fixture's suffix without the extensions flag. Production accepts only TestFlight/App Store launch categories 2/4, development accepts category 3, and the bundle version must be a decimal whole number of at most nine digits, at or above the configured `ATTEST_MIN_BUNDLE_VERSION`. Older authenticator data without these optional extensions still requires every certificate, app and environment check.

Receipt verification checks the independent CMS signature and Apple G3 chain, App ID, attested certificate, client hash, `ATTEST` type and creation within five minutes. No receipt or key is trusted merely because the client supplies it. The checked-in Apple example is verified successfully only in its explicit historical fixture context, alongside actual ECDSA assertion and invalid/tampered/replay tests. It is never accepted by production Collie policy. Current physical-device and TestFlight proofs remain release gates.

An app route permits alert/background delivery; a Live Activity route permits only liveactivity. Grants last 30 days and the phone renews from seven days before expiry. A pairing has one app route and independent activity routes. Replacement generations and conditional revocation protect a new registration from old failures. A genuine APNs registration callback or Live Activity token event renews the corresponding relay generation even when the token is unchanged, allowing recovery after server invalidation. Ordinary polling, reconnect delivery, preference changes and foreground token snapshots reuse fresh grants. Pending authenticated operations recover their exact prior generation before newer registration work; failed delivery retries the issued grant instead of rotating again. If a registration response is lost, a fresh assertion can recover only the exact same next generation/destination/kind and original expiry. It cannot change the recovered grant's scope. An enrolled key can similarly recover an ambiguous enrollment response through an assertion, without accepting an unverified second attestation.

A normal successful existing-route renewal, without enrollment, pending recovery or revocations, consumes one control operation: two HTTP requests, one App Attest assertion, four explicit SQL reads and a four-statement atomic batch with additional trigger/index work. These are source operation counts, not measured billed rows. No extra route is retained. Eight paired machines require eight controls for one genuine app registration event; ordinary polling adds no forced renewals. The existing quotas and bounded retry behavior still apply.

The schema enforces eight active machines and eight active activities per installation. Tombstones are bounded to 64 pairing records and 128 route records per installation until retention cleanup. Exact daily control limits are 128 per installation and 1,000 globally; initial enrollments are capped at 30/day globally. These limits are separate from APNs send quotas. Exhausted allocation/control budgets fail closed with `route_limit`. Registration/rotation, failed transactions and index writes must be included in the remote complete-lifecycle cost gate; these caps do not claim measured Free-plan sufficiency.

The phone client uses an explicit FIFO gate spanning challenge, App Attest and commit, since Swift actors can reenter during awaits. A ThisDeviceOnly Keychain record stores its installation/key ID, exact grants and minimal pending revocations. An excluded-from-backup installation marker prevents reusing a previous install's record. The transport pins `https://collie-push.rbstp.dev`, disables cookies/cache and redirects, bounds responses and never accepts a machine-provided relay URL. Unsupported attestation has no public enrollment fallback and leaves tailnet sessions independent.

Before posting a route proof, the phone durably retains the server challenge expiry plus 30 days as the maximum possible grant expiry, including when App Attest pauses or a successful response is lost. Removing a pairing retains its pending revocation authority before discarding grants locally. Offline failures remain pending; a previously authorized machine may send until revocation succeeds or its last grant expires. The client does not claim immediate remote revocation. A phone that stays inactive past its grant expiry loses relay notifications until it renews, while paired sessions remain usable.

Cloud operations use the user's `cf` CLI with its confirmed OAuth account. Wrangler remains only a locked local bundle/dry-run dependency. No deployment, account, credential, profile or Apple portal changes are implied by a local test/build.

A 2026-10-16 local Node run of `npm run benchmark:attestation` verified the complete historical Apple attestation and CMS receipt 100 times: first verification 21.946 ms, median 9.133 ms and p99 12.039 ms wall time. This includes local scheduling/WebCrypto waits, is not Worker CPU, and does not satisfy the below-8-ms deployed p99 gate. It is a reason to keep measuring and optimizing, not to bypass receipt or chain checks.


The first isolated Free Worker enrollment benchmark failed its CPU acceptance gate: only 45 of 100 complete Apple verifications succeeded, with 55 CPU-limit errors; all 100 invalid-nonce cases exceeded CPU limits. These are failed validation results, not authorization to expose enrollment, remove verification checks or enable billing. The direct Worker verifier remains a validation implementation until a separately reviewed Free-plan design passes complete cold/warm and hostile-input measurements.

The phone integration requests publisher grants only after a paired daemon negotiates both concrete capabilities and reports actual relay mode. Direct/disabled/older senders do not trigger enrollment. Missing expired server state may enroll afresh only when every locally known grant and ambiguous registration is expired; transient failures preserve identity. Invalid App Attest keys require explicit pairing again, retain the last possible grant expiry and do not claim earlier remote authority disappeared. Wire pairing IDs hash the installation and local confirmed pairing, so fresh installations cannot collide with retained old pairing records.

### Retention and cleanup

Superseding a route deletes its previous destination in the same transaction. A route that expires or is revoked becomes eligible for deletion seven days after its grant expiry. An installation's pairing tombstones, usage counter, key, receipt and replay record become eligible seven days after its last grant expiry, and no earlier than 37 days after enrollment. No payloads or notification decryption keys enter D1. No authorization/body/destination logs are emitted.

The hourly Cron trigger runs the scheduled cleanup, which deletes eligible records oldest first in bounded indexed batches per run: 64 routes, 4,096 pairings (64 installations' worth of 64 tombstones), 64 usage counters and 64 installations, whose keys, receipts and replay records cascade. Each control or enrollment creates at most one route and sets at most one installation's deadline, so the exact global caps of 1,000 controls and 30 enrollments per UTC day bound eligibility. The worst case is a window straddling one UTC midnight that makes two days of them eligible at once: 2,000 routes and 2,060 installations holding 131,840 pairing tombstones. Draining that takes 32 runs for routes, 33 for pairings and 33 for counters and installations, so with one hour until the first run every record is deleted within 99 hours (about four days) of becoming eligible, below the seven-day backlog bound. Sustained caps add less: one day at the caps brings 1,000 routes, 65,920 pairings and 1,030 installations against 24 runs' capacity of 1,536, 98,304 and 1,536. `npm test` seeds this worst case in local D1 and checks the per-run batch bounds and the drain. Live records are therefore gone within 14 days of the last grant expiry. A missed or failed run only delays deletion to a later run.

D1 [Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/) is always on and keeps restorable database history for 30 days on Workers Paid, so a deleted record can be restored from that history for up to 30 days after deletion.

## Daemon sender

`collied` selects `[push.relay]` or `[apns]`, never both. Relay sends use the fixed publisher `/v1/send` endpoint and only the machine/activity grant persisted through the paired mutual-TLS registration. See [architecture.md](architecture.md#relay-sender) for bounded coalescing, expiry, quota/backoff, invalidation and diagnostic behavior. Source-level/mock validation is separate from actual Cloudflare deployment, successful App Attest enrollment, Apple acceptance and physical display confirmation.
