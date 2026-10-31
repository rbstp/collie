# Push relay

The Worker in `relay/` is the publisher push relay, deployed on the publisher's Workers Paid account at `https://collie-push.rbstp.dev` with production APNs. Released builds use it: `collied setup` selects `[push.relay]` by default, and the publisher's distributed app (TestFlight builds) enrolls with App Attest once a paired daemon reports relay mode. Self-built apps use direct APNs (`collied setup --direct-apns`, `[apns]`), and a daemon in direct mode never triggers enrollment, so nothing reaches the relay. Live Activity update and end through the relay are confirmed on a physical iPhone. The relay runs on Paid because a temporary benchmark deployment, since removed, [failed the Free-plan CPU gate](reviews/relay-free-tier.md).

## Development

Use Node 24 or newer. `relay/package-lock.json` locks tooling and dependency integrity. Direct versions are exact pins of the latest compatible stable releases as of 2026-10-16, except the beta-only deployment CLI, `cf` 1.0.0-beta.18. Miniflare 4.20260730.0, then the latest stable release, supports the compatibility date 2026-08-06; the pinned `cf` and Wrangler each bundle a newer beta Miniflare. Scoped overrides patch every Miniflare copy to sharp 0.35.5 and Undici 7.30.1, which keeps the Undici 7 API Miniflare expects.

```sh
cd relay
npm ci
npm run check
npm test
npm run build
npm run benchmark
```

The `relay` GitHub Actions workflow runs the locked install, typecheck, tests and dry-run build when a pull request or a `master` push touches `relay/`. It pins Node 26.12.0 by the official archive's SHA-256, uses no Cloudflare credentials and deploys nothing.

Tests use local workerd D1, real SQLite-backed actors and mocked APNs, and cover real assertion signatures, D1 replay rejection, input bounds and fail-closed compute errors; they never enroll a real phone or send a push. Fixture registration, ephemeral signing keys and Apple's historical attestation context exist only in test code: the production entrypoint has no registration bypass, fixture endpoint or test signing key, and rejects the historical fixture.

## Internal crypto prototype

Apple verification and APNs signing run in one internal SQLite-backed `RelayCrypto` Durable Object with the fixed name `relay-crypto`, which `relay/src/worker.ts` exports with the public Worker. The actor has RPC methods for full attestation, signed assertions and deterministic provider tokens, no HTTP handler and no storage writes; it keeps only a busy flag and a provider-token cache in memory. D1 keeps authorization, replay counters, epochs, grants and quotas.

Before the actor is called, a control request passes the source, installation and enrollment limiters, the bounded body parser, the operation allowlist and the challenge checks, and its verification input is bounded to 16,384 encoded characters and a 32-byte client hash, which the actor checks again. The actor takes the App ID, environment and minimum bundle version from its own trusted bindings and the time from its own clock; no RPC accepts a caller-chosen trust root, app policy, receipt category or validation time. The atomic D1 guard commits assertion counters after verification.

The public Worker waits at most five seconds for the actor and returns `503 unavailable` on missing bindings, actor overload, exhausted platform quotas or unavailable compute; there is no inline fallback. The actor rejects overlapping operations instead of queueing them, though the platform may queue calls before they arrive. A caller timeout does not cancel admitted work, which parsing and platform CPU limits still bound. Provider tokens are deterministic RFC 6979 ES256 signatures over 2,700-second (45-minute) windows, cached in memory by both the Worker and the actor and never persisted; the actor refuses any window but its own clock's.

A guarded 42-attempt cycle on a Free account completed and its temporary resources were removed. The [validation report](reviews/relay-free-tier.md#follow-up-internal-sqlite-crypto-actor-2026-10-16) records public Worker CPU up to 6 ms and aggregate actor CPU up to 101 ms, against the 30 seconds of CPU that [Durable Object limits](https://developers.cloudflare.com/durable-objects/platform/limits/) give SQLite actors by default, and lists the remaining gaps. On Free, [Durable Object pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/) provides 100,000 requests and 13,000 GB-s per day and fails once they are exhausted.

A new cloud experiment needs a separate decision and follows `relay/test/cloudflare-crypto-plan.md`. One shared attempt journal records every request, including probes, resumed clients and network failures. Only explicit initialization creates it, refusing existing state or a held lock; resumed clients never recreate missing state, and resuming or reinitializing cannot raise its limit (at most 1,000; 50 for the crypto experiment). A temporary actor namespace is retired by deploying an explicit `exports.durableObject({ state: 'deleted' })` tombstone without its class or binding and verifying the namespace is gone before deleting the exact owned Worker ([class lifecycle contract](https://developers.cloudflare.com/durable-objects/reference/durable-objects-migrations/)).

## Paid plan and cost controls

The publisher account moved to Workers Paid on 2026-10-16. Keep the single crypto actor and D1 design: the actor draws on its own included request and duration allowances and leaves public Worker CPU headroom, and moving verification inline has not shown a lower bill.

`relay/cloudflare.config.ts` sets `worker.limits.cpuMs: 250`, shared by the public Worker and its actor. It is a provisional ceiling, not a target or a reserved charge; 100 ms would be below the historical direct-verification p99 of 118 ms and maximum of 134 ms.

Keep the caps of 20,000 sends, 1,000 controls and 30 new enrollments per UTC day. At those caps for 31 days, with one challenge per control and enrollment, the model has 683,860 HTTP requests plus 744 hourly cleanup invocations and, at four writes per send, 14 per control and nine per enrollment, 2,922,370 D1 writes before cleanup and rejected operations. These are planning assumptions. Workers Paid includes 10 million Worker requests, 30 million Worker CPU ms and 50 million D1 writes per month, shared with the account's other projects ([Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/), [D1 pricing](https://developers.cloudflare.com/d1/platform/pricing/)).

One fixed 128 MB actor billed continuously for 31 days comes to 342,835.2 GB-s against 400,000 included; idle hibernation-eligible time is free. This is arithmetic, not measured duration, and other actors, duplicate environments or extra instances draw on the same pool. Actor RPCs have their own inclusion of one million a month. Keep the single fixed name and no application storage ([Durable Object pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/)).

The control and enrollment limiters also use one shared key each, 30 and five calls per minute per Cloudflare location, besides the source and installation keys. Challenge and commit each count, so rotating IPs within a location does not get around them. The counters are permissive and eventually consistent, coarse rather than exact, and exhausting them can briefly reject legitimate enrollment or revocation before D1 or crypto ([rate-limit behavior](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/)).

Committed-operation quotas do not cap challenges, invalid proofs or every billed HTTP request, and verification runs before the committed quota checks. A distributed flood can still consume requests, CPU, D1 reads and actor RPCs, and neither the CPU ceiling nor the quotas cap spending: keep the ability to disable the public route during abuse.

## Send contract v1

`POST /v1/send`, with no query string, `Content-Type: application/json` and `Authorization: Bearer v1.<key_id>.<claims>.<signature>`. Tokens never belong in paths or queries. The authorization value is bounded to 2,048 characters before decoding, and its segments use canonical unpadded base64url. `claims` is UTF-8 JSON; HMAC-SHA256 authenticates the exact ASCII bytes `v1.<key_id>.<claims>` before parsing, so JSON property order does not matter. `key_id` selects one of at most two configured keys.

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

Identifiers are 1 to 64 ASCII letters, digits, underscores or hyphens. Integers are nonnegative safe JSON integers, and `generation` starts at 1. `expires_at` is Unix seconds, in the future and at most 30 days away. `destination_digest` is 43 base64url characters. An app route permits `alert`, `background` or both; a Live Activity route permits only `liveactivity`. `environment` is `production` or `development` and must equal the deployment's. The route, pairing and installation must match the current D1 generation, epochs, digest, environment and expiry. A send request never carries the raw destination.

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

`priority` is 5 or 10, `expiration` is unsigned 32-bit Unix seconds, and the optional `collapse_id` uses the identifier syntax. The Worker rejects a compressed or invalid UTF-8 body, or one over 8 KiB or still streaming after five seconds whatever Content-Length says, and bounds the serialized APNs payload to 4 KiB. Unknown fields are rejected at every level of the public envelope.

Public payload allowlists:

| Push type | Payload fields | `aps` fields and values |
|---|---|---|
| `alert` | `aps`, `node_id`, `event_id`, optional `approval_id`, `enc` | Required `alert`, `mutable-content: 1`, `sound: "default"`, `thread-id`; optional `category: "APPROVAL"` |
| `background` | `aps`, `node_id`, `approval_id` | Exactly `content-available: 1`; requires priority 5, expiration 0, no collapse ID |
| `liveactivity` | `aps`, `node_id`, `event_id` | Required `timestamp`, `event: "update"` or `"end"`, `content-state`; optional `stale-date`, `dismissal-date` for end only, `alert`; no collapse ID |

Every alert is exactly `{"title":"Collie","body":"An agent needs attention"}` or `{"title":"Collie","body":"An agent finished"}`. Routing, event, approval and thread identifiers are opaque and use the identifier syntax. `enc` is canonical base64url of at least a 12-byte nonce, one ciphertext byte and a 16-byte authentication tag, at most 3,800 characters. Live Activity `content-state` is exactly `{"v":1,"enc":"..."}`. The timestamp and optional dates are unsigned 32-bit Unix seconds. The relay has no notification key, so the sender and the phone enforce ciphertext authenticity and encryption. `npm test` checks that the shared Rust and Swift display vectors in `docs/protocol/push-display-vectors.json` pass these allowlists.

The Worker chooses only Apple's production or sandbox host, the registered destination and the fixed topic `dev.rbstp.collie` or `dev.rbstp.collie.push-type.liveactivity`, and builds every header itself, never taking a caller's URL, topic, provider authorization or extra headers. It never follows redirects. It waits at most 10 seconds for Apple and reads at most 1 KiB of the response body.

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

Apple's `410 ExpiredToken`, `410 Unregistered` and `400 BadDeviceToken` invalidate the exact matching route generation and destination digest; once that commits, later attempts fail before reaching APNs. A failed or timed-out invalidation returns `503 unavailable` without claiming durable revocation. A late failure cannot erase a replacement, even a newer generation for the same token, and invalidation deletes nothing: the tombstone and destination stay until [retention cleanup](#retention-and-cleanup) ([Apple's response guidance](https://developer.apple.com/documentation/usernotifications/handling-notification-responses-from-apns)). The Worker never returns APNs reason strings, response headers, destinations or payloads, and never retries. The daemon's bounded in-memory retries do not guarantee exactly-once delivery: if a response is lost after Apple accepts, a retry can deliver twice.

## Atomic quotas and storage ownership

`relay/migrations/0001_relay.sql` creates the foundation tables; `relay/migrations/0002_attestation.sql` adds verified keys, receipts, atomic replay and control counters, and bounded lifecycle state. There is no provisioning command: only attested enrollment creates an installation, inserting the installation, attestation and zeroed `installation_usage` rows in one batch. Authenticated route registration creates its pairing and route in one guarded batch. Schema triggers enforce the limits of eight machines and eight activities, and [Retention and cleanup](#retention-and-cleanup) bounds and deletes registration records.

A send charges quota with one indexed UPDATE of the installation counter. The UPDATE rechecks route, pairing and installation authorization and takes expiry and day boundaries from the later of the request clock and D1's UTC clock; its trigger charges the singleton global counter and aborts the whole statement when global capacity is exhausted. A route revoked or expired during quota work returns `invalid_route`, a valid route with exhausted capacity `daily_quota`, and a missing counter `unavailable`. Counters reset lazily at UTC midnight, so no per-send record or daily deletion is needed, and a delayed previous-day invocation cannot move them backward. Never use INSERT ON CONFLICT with a BEFORE INSERT quota trigger: both insert and update triggers could charge one upsert. The Worker rechecks grant validity after body parsing and signing and again after the bounded D1 charge, just before forwarding; a grant that expires during that last wait stops delivery without a refund.

The installation limits are 1,000 total and 700 routine attempts per UTC day, across all its machines and activity routes; the global limits are 20,000 and 16,000. Only a priority-5 Live Activity `update` without an alert is routine. Everything else, including priority-10 updates such as silent approval clears, alerts and activity ends, counts only against the total, so it can use the capacity above the routine limit. Every accepted attempt is charged before forwarding, including network ambiguity and Apple rejections, and never refunded. A definite quota rejection whose response crosses UTC midnight is retried once against the new day; successful or ambiguous charges never are. Quota responses report the reset of the window actually attempted, not one derived from the response time. Soft IP, installation and route rate limits run before D1 quota work and do not replace the exact counters. A revocation after the final charge can race an APNs attempt already authorized.

D1 stores registration, authorization and counters, not notification history. Workers Paid allows [10 GB per database](https://developers.cloudflare.com/d1/platform/limits/), with queries processed one at a time per database; that limit is separate from included account storage and does not cap spending.

Send lookups use primary-key indexes, and every D1 wait is bounded to five seconds. Local tests prove that 20 concurrent last-slot attempts commit exactly one charge and roll back every rejected counter change, and cover total and routine reservation, UTC reset, held streams crossing expiry or midnight, expiry during the charge response, stream duration limits, expired and revoked routes, and an epoch change between lookup and charge.

## Measurements and release gates

The [deployed Free-plan report](reviews/relay-free-tier.md) records CPU-limit failures, D1 lifecycle costs, a request-accounting failure and verified resource cleanup. Its direct-Worker verifier failed the CPU gate: 198 of 400 full Apple verifications hit the CPU limit, 55 of 200 valid and 143 of 200 invalid-nonce attempts. The response was the crypto actor on Workers Paid, never removing or weakening verification checks.

A 2026-10-16 macOS run of 100 samples measured Node wall time, not deployed Worker CPU:

| Operation | Median wall time | p99 wall time |
|---|---:|---:|
| New deterministic provider signer | 0.639 ms | 1.378 ms |
| Warm cached provider JWT | 0.000375 ms | 0.00221 ms |
| Full mocked send through local D1 | 21.166 ms | 143.555 ms |

Local D1 metadata reports 10 rows read and 2 written per accepted send, 1 and 1 for route invalidation, and 1 and 1 each for explicit route and counter deletion. These are local observations, not production billing, index maintenance or complete lifecycle costs. Reproduce with `npm run benchmark`; timings depend on the host and its load.

A 2026-10-16 local Node run of `npm run benchmark:attestation` verified the complete historical Apple attestation and CMS receipt 100 times: 21.946 ms for the first verification, 9.133 ms median and 12.039 ms p99 wall time. That includes local scheduling and WebCrypto waits, is not Worker CPU, and does not meet the deployed gate of a p99 below 8 ms. It is a reason to keep measuring and optimizing, never to skip receipt or chain checks.

The original Free-plan capacity model used a conservative launch gate of four writes per send: `(100000 - 20000) / 4 = 20000` attempts a day supports `floor(20000 / 624) = 32` phones at the plan's example of 624 sends and 12 controls per phone. Thirty phones use 18,720 attempts, at most 74,880 send writes and 360 controls, leaving the reserved 20,000 writes for enrollment, controls, failed quota work, cleanup and other account use. The local two-write result does not justify raising that ceiling. Before claiming the reserve is enough, record remote outcome writes, index costs, zero-counter creation, enrollment and assertion replay updates, expiry cleanup and other account traffic. If complete-lifecycle sends cost more than four billed writes, lower the SQL global caps and recompute capacity.

Local, mock and Free-plan results do not replace production checks. Deploy only to the intended, confirmed Paid account. With genuine valid and invalid App Attest certificate, nonce, app, environment and replay fixtures, measure cold and warm enrollment, assertions, JWTs, full sends and hostile input, with enough headroom below the CPU ceiling and zero CPU-limit errors. Still unmeasured: production CPU distributions, actor duration and capacity, remote D1 concurrency, quota rollback and complete-lifecycle costs (registration and rotation, failed transactions, index writes, expiry cleanup, enrollment), populated retention and throughput, burst capacity and shared account headroom. Physical-device and TestFlight enrollment evidence remains a release gate for each submitted build ([public submission](public-submission.md)).

## Operations

`cloudflare.config.ts` has the hourly cleanup Cron trigger (`0 * * * *`), no workers.dev or preview URL, a placeholder D1 database ID, the development APNs environment and the production minimum app build number in `ATTEST_MIN_BUNDLE_VERSION`. The owner deploys it with `cf`, supplying the real database ID, the production APNs environment and the `collie-push.rbstp.dev` Custom Domain outside the file; never commit them. Another isolated deployment needs its own D1 database with the schema and an authenticated access boundary, and one for Debug builds, whose bundle version is 1, needs a minimum of 1. Keep workers.dev and preview URLs disabled. After each deployment, verify the limiter bindings, CPU setting and Cron trigger, and recheck cold and warm CPU.

Use the `cf` CLI with its confirmed OAuth account for all Cloudflare operations. Wrangler is only a locked dependency for local bundles and dry runs, and local tests and builds change no deployment, account, credential, profile or Apple portal setting. Unset inherited `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_API_KEY` and `CLOUDFLARE_EMAIL` so they cannot override the `cf` login, run `cf auth whoami` and set the verified `CLOUDFLARE_ACCOUNT_ID`. Build with `cf build` and inspect the output before deploying. Put `APNS_TEAM_ID`, `APNS_KEY_ID`, `APNS_PRIVATE_KEY` and `SEND_HMAC_KEYS` in a private JSON file with mode 0600 and pass it with `cf deploy --prebuilt --no-provision --secrets-file /secure/path/secrets.json`; never put secret values in committed vars, command arguments or logs.

`SEND_HMAC_KEYS` maps key IDs to canonical base64url 32-byte random keys, at most two. New grants and challenges use the first entry; JavaScript lists integer-like IDs first, so never use integers as IDs ([HMAC secret rotation](relay-runbook.md#hmac-secret-rotation)). Keep an old key only until its last 30-day grant expires. Removing a key rejects its grants at once. To rotate the APNs key, deploy the new key ID and private key together and revoke the old Apple key only after validation.

The relay uses the single-level Worker Custom Domain `collie-push.rbstp.dev`, which Universal SSL covers; `collie.rbstp.dev` stays the DNS-only GitHub Pages CNAME. Do not use `push.collie.rbstp.dev` without suitable certificate coverage. The source requires its limiter bindings and fails closed without them. Review edge protection and route shutdown before public TestFlight or App Store distribution: distributed traffic can exceed included account usage despite soft controls.

Invocation observability is disabled, and the source logs no requests or payloads. Keep headers, authorization, bodies, destination URLs and APNs response bodies out of platform logs, traces and dashboards; watch only aggregate outcome codes (see the [response table](#send-contract-v1)) and account-wide Worker and D1 metrics. Other Workers, actors and D1 databases share the Paid allowances, so check account-wide traffic before increasing invitations. Reconsider the design at seven-day peak usage above 70%, legitimate budget rejection above 1%, or forecast use above 30 daily active phones.

Cloudflare sees source IPs, raw APNs destinations, route relationships, generic alerts, opaque identifiers, timing, frequency and ciphertext size. Apple receives APNs metadata and ciphertext. Neither gets notification decryption keys. D1 stores registration metadata, revocation state and counters, with no notification payloads, provider JWTs or approval authority, for as long as [Retention and cleanup](#retention-and-cleanup) allows.

Sources: [Cloudflare Worker limits](https://developers.cloudflare.com/workers/platform/limits/), [D1 pricing and row accounting](https://developers.cloudflare.com/d1/platform/pricing/), [D1 transactions](https://developers.cloudflare.com/d1/worker-api/d1-database/), [rate-limit binding](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/), [Apple provider tokens](https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns).

## Attested control contract

`POST /v1/challenge` accepts one operation object. `POST /v1/control` accepts exactly `operation`, `challenge`, and either `attestation` for a new installation or `assertion` for an enrolled key. Both reject query strings, unknown fields, compressed bodies, invalid UTF-8, and bodies over 16 KiB or still streaming after five seconds. Replies never include destinations, receipts or verification details. The source-IP and installation control limiter and the enrollment limiter run before any expensive verification, and missing limiter or bundle-version configuration fails closed.

Every operation contains `v: 1`, `op`, an opaque `installation_id`, a `key_id` (canonical base64url of 32 bytes) and `environment`:

| Operation | Additional fields |
|---|---|
| `enroll` | None |
| `route` | `pairing_id`, `route_id`, `expected_generation`, `kind` (`app` or `liveactivity`), `destination` (lowercase hex) |
| `revoke_pairing` | `pairing_id` |
| `revoke_route` | `pairing_id`, `route_id`, `expected_generation` |

Canonicalization sorts the operation's ASCII property names and serializes compact JSON. The server returns `challenge: c1.<key_id>.<claims>.<signature>`. Its claims hold a random 32-byte nonce, the canonical operation's digest, the installation and key, the current durable counter and epoch, and a five-minute expiry; HMAC-SHA256 authenticates the first three segments with the send-grant key set, under the separate `c1` domain. The phone passes the SHA-256 of the whole challenge string to App Attest. Issuing a challenge writes nothing to D1. Durable key uniqueness protects the first enrollment; every later operation must present the challenge's exact prior counter and epoch in the D1 transaction that advances the counter and changes authorization, so a used challenge fails even when re-signed with a larger counter. A rejected trigger aborts every write, including the global and installation control quotas.

The verifier trusts only the pinned Apple App Attestation root and, separately for the initial CMS receipt, Apple Root CA G3. It checks certificate validity, issuer signatures, CA and path-length constraints, key usage, duplicate or unknown critical extensions, the leaf nonce, agreement of COSE key, public key and key ID, the exact publisher App ID, a zero initial counter, the credential ID and the attestation environment. Assertions verify the ECDSA signature over the SHA-256 nonce of authenticator data plus client-data hash, the App ID and a strictly increasing counter. The parser accepts Apple assertions that keep the AT flag without credential data, but rejects unexpected credential bytes and trailing data, and bounded CBOR and ASN.1 parsing rejects duplicate keys and trailing objects. Documented authenticator extensions are parsed when present, including the suffix in Apple's published fixture, which lacks the extensions flag. Production accepts only TestFlight and App Store launch categories 2 and 4, development only category 3, and the bundle version must be a decimal whole number of at most nine digits, at or above `ATTEST_MIN_BUNDLE_VERSION`. Authenticator data without these optional extensions still has to pass every certificate, app and environment check.

Receipt verification checks the independent CMS signature and Apple G3 chain, the App ID, the attested certificate, the client hash, the `ATTEST` type and creation within five minutes; no receipt or key is trusted just because the client supplies it. Apple's checked-in example verifies only in its explicit historical fixture context, alongside real ECDSA assertion and invalid, tampered and replay tests; production policy never accepts it.

Grants last 30 days; the phone renews them from seven days before expiry. A pairing has one app route and independent activity routes. Replacement generations and conditional revocation keep old failures from affecting a new registration. A genuine APNs registration callback or Live Activity token event renews its relay generation even when the token is unchanged, which recovers from server invalidation; polling, reconnect delivery, preference changes and foreground token snapshots reuse fresh grants. Pending authenticated operations recover their exact prior generation before newer registration work, and a failed delivery retries the issued grant instead of rotating. A fresh assertion can recover a lost registration response, but only for the same next generation, destination and kind with the original expiry. An enrolled key can likewise recover an ambiguous enrollment response with an assertion, never with a second, unverified attestation.

A normal existing-route renewal, without enrollment, pending recovery or revocation, is one control operation: two HTTP requests, one App Attest assertion, four explicit SQL reads and a four-statement atomic batch, plus trigger and index work. These are counts from the source, not measured billed rows. One genuine app registration event with eight paired machines needs eight controls; ordinary polling forces no renewals.

The schema enforces eight active machines and eight active activities per installation, and bounds tombstones to 64 pairing records and 128 route records per installation until retention cleanup. Exact daily control limits are 128 per installation and 1,000 globally, and initial enrollments are capped at 30 a day globally. These limits are separate from the APNs send quotas. Exhausted allocation or control budgets fail closed with `route_limit`.

The phone client runs challenge, App Attest and commit under an explicit FIFO gate, because Swift actors can reenter during awaits. A ThisDeviceOnly Keychain record stores its installation and key ID, exact grants and minimal pending revocations, and an installation marker excluded from backup prevents reusing a previous install's record. The transport pins `https://collie-push.rbstp.dev`, disables cookies, cache and redirects, bounds responses and never accepts a relay URL from a machine. A device without App Attest support has no enrollment fallback, and its tailnet sessions are unaffected.

Before posting a route proof, the phone durably records the challenge expiry plus 30 days as the latest possible grant expiry, so the bound survives a paused App Attest or a lost response. Removing a pairing keeps its pending revocation authority before discarding grants. A revocation that fails offline stays pending, and the machine can keep sending until it succeeds or the last grant expires; the client never claims immediate remote revocation. A phone inactive past its grant expiry gets no relay notifications until it renews, while paired sessions keep working.

The phone requests publisher grants only after a paired daemon negotiates both `relay_grants_v1` and `encrypted_display_v1` and reports relay mode; direct, disabled and older senders never trigger enrollment. If server state is missing or expired, the phone enrolls afresh only when every locally known grant and ambiguous registration has expired; transient failures keep the identity. An invalid App Attest key requires pairing again, keeps the last possible grant expiry and claims nothing about earlier remote authority. Wire pairing IDs hash the installation and the confirmed local pairing, so a fresh installation cannot collide with retained old pairing records.

### Retention and cleanup

Superseding a route deletes its previous destination in the same transaction. An expired or revoked route becomes eligible for deletion seven days after its grant expiry. An installation's pairing tombstones, usage counter, key, receipt and replay record become eligible seven days after its last grant expiry, and no earlier than 37 days after enrollment.

The hourly Cron trigger deletes eligible records oldest first, in bounded indexed batches per run: 64 routes, 4,096 pairings (64 installations' worth of 64 tombstones), 64 usage counters and 64 installations, whose keys, receipts and replay records cascade. Each control or enrollment creates at most one route and sets at most one installation's deadline, so the exact global caps of 1,000 controls and 30 enrollments per UTC day bound eligibility. At worst, a window straddling one UTC midnight makes two days of them eligible at once: 2,000 routes and 2,060 installations holding 131,840 pairing tombstones. Draining them takes 32 runs for routes, 33 for pairings and 33 for counters and installations; with up to an hour before the first run, every record is deleted within 99 hours (about four days) of becoming eligible, inside the seven-day backlog bound. Sustained caps add less: a day at the caps brings 1,000 routes, 65,920 pairings and 1,030 installations, against 24 runs' capacity of 1,536, 98,304 and 1,536. `npm test` seeds this worst case in local D1 and checks the per-run bounds and the drain. Live records are therefore gone within 14 days of the last grant expiry; a missed or failed run only delays deletion to a later one.

D1 [Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/) is always on and keeps restorable database history for 30 days on Workers Paid, so a deleted record can be restored for up to 30 days after deletion.

## Daemon sender

`collied` selects `[push.relay]` or `[apns]`, never both. Relay sends use the fixed publisher endpoint `https://collie-push.rbstp.dev/v1/send` and only the machine or activity grant persisted through the paired mutual-TLS registration. [Relay sender](architecture.md#relay-sender) covers coalescing, expiry, quota backoff, invalidation and diagnostics.
