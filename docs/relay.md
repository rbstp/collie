# Push relay foundation

The Worker in `relay/` is an isolated validation foundation for [#350](https://github.com/rbstp/collie/issues/350). It is not deployed, has no enrollment endpoint, and is not enabled in the app or collied. Existing releases keep direct APNs delivery. Public rollout remains blocked on verified App Attest control operations (#353), encrypted phone payloads (#352), compatible peers (#351), the daemon sender (#354), and the Free-account measurements below.

## Development

Use Node 24 or newer. `relay/package-lock.json` locks tooling and dependency integrity; direct versions are exact stable releases checked on 2026-10-16. The compatibility date is 2026-08-06, supported by the pinned stable Miniflare runtime. Wrangler currently bundles a newer beta Miniflare separately.

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
| 503 | `upstream_unavailable` | Network, timeout, Apple 429/5xx; includes `retry_after: 60` |
| 503 | `unavailable` | D1, secret or other internal failure; fails closed |

Only Apple's `410 Unregistered` and `400 BadDeviceToken` invalidate the exact matching route generation and destination digest. Late failures cannot erase a replacement. APNs reason strings, response headers, destinations and payloads are never returned. There is no automatic retry or exactly-once delivery guarantee. A lost response after Apple accepts can lead to duplicate delivery if the sender retries.

## Atomic quotas and storage ownership

`relay/migrations/0001_relay.sql` owns the foundation tables. The future attestation/control implementation adds its verified key, challenge/counter and receipt schema separately and must create installation, pairing, route and zeroed `installation_usage` records atomically only after authenticating authorization. There is no production provisioning command in this foundation. It must enforce the plan's eight-machine/eight-activity limits and bound/clean registration records before rollout.

A single indexed installation-counter UPDATE rechecks current route, pairing and installation authorization and uses the later of the current request clock and D1 UTC clock for expiry and day boundaries. The Worker refreshes grant validity after body parsing and signing, and immediately before forwarding after the bounded D1 charge await. An expiry during that final await stops delivery without refunding the reservation. Route revocation or expiry during quota work returns `invalid_route`; only a valid route with exhausted capacity returns `daily_quota`, and a missing counter returns `unavailable`. Its trigger updates the singleton global counter, aborting the entire statement when global capacity is exhausted. No per-send counter record is inserted, and no daily counter deletion is needed: each row resets lazily at UTC midnight. Delayed previous-day invocations cannot move either counter backward. Registration must pre-create its zeroed counter row; missing counters fail closed. Never use INSERT ON CONFLICT with a BEFORE INSERT quota trigger, since both insert and update triggers could charge an upsert.

The installation limits are 1,000 total and 700 routine attempts per UTC day, across all paired machines and activity routes. Global limits are 20,000 total and 16,000 routine attempts. A Live Activity `update` without an alert is routine. Alerts, clears and activity ends can use the reserved remainder. All accepted attempts are charged before forwarding, including network ambiguity and Apple rejections. Counters are never refunded. Soft IP/installation/route rate limits precede D1 quota work; they do not replace exact counters. Revocation after the final atomic charge may race an already authorized in-flight APNs attempt.

Send lookups use primary-key indexes. The local suite proves 20 concurrent last-slot attempts allow exactly one commit and roll back all rejected counter changes, total/routine reservation, UTC reset, held streams crossing expiry/midnight, expiry during the charge response, stream duration limits, expired/revoked routes and an epoch change between lookup and charge. Signing and D1 waits have five-second bounds. Production D1 concurrency and billed costs still require remote verification. Bounded indexed expiry cleanup and enrollment costs belong to the control lifecycle and are not implemented here.

## Local measurements and release gates

A 2026-10-16 macOS run, 100 samples, measured Node wall time, not deployed Worker CPU:

| Operation | Median wall time | p99 wall time |
|---|---:|---:|
| New deterministic provider signer | 0.639 ms | 1.378 ms |
| Warm cached provider JWT | 0.000375 ms | 0.00221 ms |
| Full mocked send through local D1 | 21.166 ms | 143.555 ms |

Local D1 metadata reports 10 rows read and 2 rows written per accepted send, 1/1 for route invalidation, and 1/1 each for explicit route and counter deletion. These are local runtime observations, not verified production billing, index-maintenance or complete lifecycle costs. Reproduce with `npm run benchmark`; timings depend on the host and load.

Using the conservative four-write launch gate, `(100000 - 20000) / 4 = 20000` attempts/day supports `floor(20000 / 624) = 32` phones at the plan's 624-send/12-control example. Thirty phones consume 18,720 attempts, at most 74,880 send writes and 360 controls, leaving the reserved 20,000 writes for enrollment, controls, failed quota work, cleanup and other account use. The local two-write result does not justify raising that ceiling. Record remote outcome writes, index costs, zero-counter creation, enrollment/assertion replay updates, expiry cleanup and other account traffic before claiming the reserve is sufficient. Lower the SQL global caps and recompute capacity if complete-lifecycle send costs exceed four billed writes.

Outstanding launch gates: deploy only to the intended confirmed Free account; verify rate-limit binding availability there; use genuine valid/invalid App Attest certificate, nonce, app/environment and replay fixtures; measure cold/warm enrollment, assertions, JWTs, full sends and hostile input with p99 Worker CPU below 8 ms and zero CPU-limit errors; measure remote D1 complete lifecycle and quota rollback. The verifier and controls are #353 work. No attestation CPU result is claimed by this foundation. No public traffic or real phone sends are authorized by its validation configuration.

## Operations

The checked-in configuration has no workers.dev URL, preview URL, Custom Domain, Cron trigger, real database ID or production enrollment. Do not enable a public route while validation fixtures or incomplete authorization are in use. For an authenticated isolated deployment, confirm the Free plan and account, create its isolated D1 database, apply the schema, configure an authenticated access boundary, then replace the placeholder database binding locally. Keep workers.dev/preview URLs disabled. Test rate-limit availability on Free before deciding whether the binding can be required.

Provision `APNS_TEAM_ID`, `APNS_KEY_ID`, `APNS_PRIVATE_KEY` and `SEND_HMAC_KEYS` with Wrangler secrets, never committed vars, command arguments or logs. `SEND_HMAC_KEYS` is a JSON map from key ID to a canonical base64url 32-byte random key, with at most two active keys. New grants use the new ID; retain the old verification key only until its last 30-day grant expires, then remove it. Emergency removal immediately rejects that key's grants. APNs key rotation deploys the new key ID/private key together and revokes the old Apple key only after validation. Provider tokens are deterministic RFC 6979 ES256 in 45-minute windows, cached only per isolate and never persisted.

When production gates pass, use the single-level Worker Custom Domain `collie-push.rbstp.dev`; keep `collie.rbstp.dev` as the DNS-only GitHub Pages CNAME. Universal SSL covers the former. Do not use `push.collie.rbstp.dev` without suitable certificate coverage. Never automatically upgrade to Paid. If the rate-limit binding cannot be deployed on Free, keep exact D1 quotas and configure the Free zone's one IP-based WAF rate-limit rule as coarse protection before deployment. The current source requires its limiter binding and fails closed without it; a separately reviewed fallback configuration is required. A distributed flood can exhaust the account request allowance despite either soft control.

Invocation observability is disabled and source code logs no requests or payloads. Keep request headers, authorization, bodies, destination URLs and APNs response bodies out of platform logging, tracing and dashboards. Use only aggregate outcome codes and account-wide Worker/D1 metrics. Diagnose 401 as grant/key/environment, 410 as route/renewal, 429 as limiter or UTC quota, 502 as APNs configuration, and 503 as D1/secret/upstream availability. Inspect account-wide traffic before increasing invitations: other Workers and D1 databases share Free allowances. Reconsider the design at seven-day peak usage above 70%, legitimate budget rejection above 1%, or forecast use above 30 daily active phones.

Cloudflare sees source IPs, raw APNs destinations, route relationships, generic alerts, opaque identifiers, timing, frequency and ciphertext size. Apple receives APNs metadata and ciphertext, but neither party gets notification decryption keys. D1 stores registration metadata, revocation state and counters, with no notification payloads, provider JWTs or approval authority. The control implementation must establish the plan's retention and deletion guarantee before any live enrollment.

Sources: [Cloudflare Worker limits](https://developers.cloudflare.com/workers/platform/limits/), [D1 pricing and row accounting](https://developers.cloudflare.com/d1/platform/pricing/), [D1 transactions](https://developers.cloudflare.com/d1/worker-api/d1-database/), [rate-limit binding](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/), [Apple provider tokens](https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns).
