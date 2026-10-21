# Relay operator runbook

Procedures for the publisher operating the production push relay at `collie-push.rbstp.dev`. [Relay](relay.md) holds the design, the send and control contracts and the quotas; this page covers routine operation and incident response. Never put Cloudflare account IDs, database IDs, secret values, grants, destinations or IP addresses in the repository, an issue or an incident record.

## Deployment

The Worker is `relay/src/worker.ts`: the public handler, the single `RelayCrypto` Durable Object for App Attest and APNs signing, the D1 binding `DB`, three rate limiters and five secrets. `relay/cloudflare.config.ts` sets `limits.cpuMs: 250`, `observability.enabled: false`, `workersDev: false` and `previewUrls: false`.

| Binding | Kind | Purpose |
|---|---|---|
| `DB` | D1 | Installations, pairings, routes, attestations, counters and quotas |
| `CRYPTO` | Durable Object `RelayCrypto` | Attestation, assertion and provider-token signing |
| `APNS_ENVIRONMENT` | Text | `production` or `development`; must match every grant and operation |
| `APNS_TEAM_ID`, `APNS_KEY_ID`, `APNS_PRIVATE_KEY` | Secrets | Publisher APNs key; the team ID also forms the App Attest App ID |
| `SEND_HMAC_KEYS` | Secret | One or two HMAC keys for send grants and control challenges |
| `ATTEST_BUNDLE_VERSIONS` | Secret | JSON array of 1 to 16 app bundle versions accepted by App Attest checks |
| `SEND_LIMITER` | Rate limit, 60 per 60 s | Sends, keyed by source IP, installation and route generation |
| `CONTROL_LIMITER` | Rate limit, 30 per 60 s | Challenges and controls, keyed by source IP, installation and one shared key |
| `ENROLL_LIMITER` | Rate limit, 5 per 60 s | Enrollments, keyed by source IP and one shared key |

The checked-in configuration is the validation one: a placeholder database ID, `APNS_ENVIRONMENT` set to `development` and no Custom Domain. The production deployment uses a private local copy with the production database, `production`, the `collie-push.rbstp.dev` Custom Domain and the hourly cleanup trigger ([#541](https://github.com/rbstp/collie/issues/541)). The repository does not hold that copy. Every procedure below that deploys uses it unchanged, apart from the change it describes.

Deploy as [Relay](relay.md#operations) describes. Unset `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_API_KEY` and `CLOUDFLARE_EMAIL`, check the account with `cf auth whoami` and set the verified `CLOUDFLARE_ACCOUNT_ID`. Then, from `relay/`:

```sh
npm ci
npm run check
npm test
cf build
cf deploy --prebuilt --no-provision --secrets-file /secure/path/secrets.json
```

The secrets file is private JSON, mode 0600, outside the repository, and holds all five secrets. Never pass a secret value as a command argument.

## Monitoring

The relay keeps no request logs and the repository adds no logging backend. Keep observability disabled; do not enable invocation logs, traces or header capture to investigate real traffic. Monitor from four aggregate sources.

| Source | What to read |
|---|---|
| Worker metrics in the Cloudflare dashboard | Requests, errors and CPU time percentiles against the 250 ms limit. A request over the limit fails with Cloudflare error 1102 |
| `RelayCrypto` Durable Object metrics | Requests and duration; one actor runs continuously at most 342,835.2 GB-s a month against 400,000 included |
| D1 metrics | Rows read and written, storage against the 10 GB per-database limit |
| HTTP analytics of the `rbstp.dev` zone, filtered to the relay host | Responses by status code, read with the result table in [Relay](relay.md#send-contract-v1) |

Account usage counts against Workers Paid's monthly inclusions: 10 million requests, 30 million CPU ms and 50 million D1 rows written, shared with every other project in the account. The relay model in [Relay](relay.md#paid-plan-and-cost-controls) uses about 7% of the requests and 6% of the writes at its daily caps.

D1 holds the aggregate counters. The repository has no script to query it; run read-only statements in the D1 console of the dashboard or with `cf`. Select counts only, never identifiers, destinations, keys or receipts.

```sql
SELECT day, total, routine FROM global_usage;
SELECT day, controls, enrollments FROM control_usage;
SELECT COUNT(*) AS phones, SUM(total) AS attempts, MAX(total) AS busiest,
  SUM(total >= 1000 OR routine >= 700) AS at_cap
FROM installation_usage
WHERE day = CAST(unixepoch() / 86400 AS INTEGER) AND total > 0;
SELECT COUNT(*) AS installations FROM attestations WHERE revoked = 0;
SELECT COUNT(*) AS cleanup_backlog FROM routes WHERE expires_at < unixepoch() - 7 * 86400;
```

`day` counts days since 1970-01-01 UTC. Each counter row keeps only its latest day and resets at the first charge after 00:00 UTC, so record the values shortly before midnight UTC to keep a daily series. `global_usage` caps at 20,000 attempts and 16,000 routine; `control_usage` caps at 1,000 controls and 30 enrollments. A growing `cleanup_backlog` means the hourly trigger is not keeping up or not running.

The relay records no APNs reason. It turns Apple's answers into three results: `410 invalid_route` when Apple invalidates a token (a steady trickle is normal), `502 configuration_error` for any other Apple rejection, and `503 upstream_unavailable` for Apple 5xx, 429 or network failures. Any 502 needs action; see [APNs configuration errors](#apns-configuration-errors). To reproduce a failure, use a machine paired with an operator-owned phone: `collied apns test` sends through the relay, and `collied doctor` shows the last send error.

| `collied doctor` last error | Relay result |
|---|---|
| relay daily quota exhausted | `429 daily_quota` |
| relay temporarily unavailable | `429 rate_limited`, any other 429 or 5xx, including `503 upstream_unavailable`, `503 unavailable` and Cloudflare error pages with those statuses |
| relay returned an unexpected response | Any other response without relay JSON, such as another Cloudflare error page |
| relay APNs configuration error; registration retained | `502 configuration_error` |
| relay grant unauthorized; open the paired app to renew | `401 unauthorized` |

Before raising the public TestFlight tester limit, read the last seven days. [Relay](relay.md#operations) sets the review points: peak `global_usage` total above 70% of 20,000, more than 1% of legitimate sends rejected with `daily_quota`, or more than 30 daily active phones. See [App Review notes](app-review.md#public-link-capacity) for the capacity cap.

## Allow a new app build

App Attest checks reject an app whose bundle version is not in `ATTEST_BUNDLE_VERSIONS`, whenever Apple's attestation or assertion carries its bundle-version extension. Enrollment, renewal or revocation from such a build then returns `401 unauthorized`. Sends are unaffected: grants already issued keep working until they expire.

Before a build reaches testers, add its bundle version, as the same kind of value as the entries already there, to the array and deploy. The array holds at most 16 entries of up to 32 letters, digits and dots. Remove a version only once no tester should still run that build, since its phones may then fail to renew or revoke and keep relay pushes only until their grants expire.

## APNs key rotation

The relay signs with the publisher's APNs key alone. Live Activity pushes use the same key.

1. In the Apple Developer portal, under Keys, create a key with Apple Push Notifications service and note its Key ID.
2. Put the new `APNS_KEY_ID` and `APNS_PRIVATE_KEY` in the secrets file together, keep `APNS_TEAM_ID` and the other secrets as they are, and deploy.
3. From a machine paired with an operator-owned phone on a production build, run `collied apns test` and see the alert arrive on the phone.
4. Revoke the old key in the portal only after that test passes and at least 45 minutes after the deploy. Provider tokens are cached in memory for 45-minute windows, so an instance can still sign with the old key until its window ends.

If the old key leaked, deploy a new key at once and revoke the old one right after. If a new key cannot be deployed quickly, revoke the old key anyway: every send then returns `502 configuration_error` until the new key is deployed, and sessions over the tailnet keep working.

## HMAC secret rotation

`SEND_HMAC_KEYS` is a JSON object of one or two entries. Each ID is 1 to 32 letters, digits, underscores or hyphens; each value is 32 random bytes in canonical unpadded base64url. The first entry signs new grants and challenges, and both entries verify. Use IDs that are not integers, such as `hmac_20261020`: JavaScript puts integer-like keys first, which would change the signing key. The repository has no generator; take the bytes from a cryptographically secure random source.

Planned rotation:

1. Deploy with the new entry first and the current one second.
2. From an operator-owned phone, open the app, which renews its app route with the new key, then run `collied apns test` on a paired machine.
3. Wait 30 days. Grants last 30 days and phones renew from seven days before expiry, so no unexpired grant signed with the old key remains after that. Phones that stayed closed longer have already lost relay pushes and renew when opened.
4. Deploy with only the new entry.

Emergency rotation, when a key leaked: deploy with only a new entry. Every grant and pending challenge signed with the old key fails at once. Sends return `401 unauthorized`, `collied doctor` tells the user to open the paired app, and opening the app registers again.

## Revoke a pairing or an installation

A user revokes a machine by removing it in the app, which sends an authenticated `revoke_pairing`. The app never revokes a whole installation.

There is no administrative endpoint or script. The relay has no accounts and no logs, so the operator cannot find the installation or pairing behind any traffic. The IDs exist only on the phone and in grants. A user who reports a compromised machine can read them from the grant in that machine's private `push.json`: the grant is `v1.<key_id>.<claims>.<signature>`, and only the claims segment, base64url JSON, is needed. Never ask for or copy the whole grant.

With an exact ID, run reviewed statements against the production database. Check that each changed exactly one row.

```sql
-- One pairing: the app's own revoke_pairing statement.
UPDATE pairings SET epoch = epoch + 1, revoked = 1 WHERE id = '<pairing_id>' AND revoked = 0;

-- A whole installation.
UPDATE installations SET epoch = epoch + 1 WHERE id = '<installation_id>';
UPDATE attestations SET revoked = 1 WHERE installation_id = '<installation_id>';
```

Revoking a pairing makes its routes return `410 invalid_route`, and collied drops the grant. Revoking an installation does the same for all its routes and refuses its controls with `401 unauthorized`; that installation can never register again, and only a reinstall, which enrolls a new installation, brings relay pushes back to the phone. Never delete rows: revocation relies on the epoch and revoked flags, and a deleted pairing could be created again at epoch 0, the epoch its old grants carry. Cleanup removes revoked records after the retention delay.

To revoke everything at once, use the emergency HMAC rotation.

## Stop enrollment or disable the public route

No setting stops enrollment alone. In order of reach:

1. **Stop invitations.** In App Store Connect, disable the public TestFlight link or lower its tester limit. Testers already in keep the app.
2. **Refuse App Attest for unknown builds.** Setting `ATTEST_BUNDLE_VERSIONS` to a version no build reports refuses enrollment, renewal and revocation from every phone whose attestation or assertion carries Apple's bundle-version extension. One without the extension still passes. Existing grants keep sending until they expire. The array cannot be empty: an empty or invalid value makes every enrollment, renewal and revocation fail with `503 unavailable`.
3. **Lower the daily enrollment cap.** The 30-a-day cap is in the `enrollment_limit` trigger of `relay/migrations/0002_attestation.sql`. Changing it needs a new migration and a review; there is no runtime setting.
4. **Disable the public route.** Remove the `collie-push.rbstp.dev` Custom Domain from the Worker in the dashboard (Settings, Domains & Routes), and, if the private configuration declares it, take it out there too so the next deploy does not restore it. workers.dev and preview URLs are already off, so no other public path remains. Enrollment, renewal, revocation and sends all stop; collied backs off and doctor reports the relay unavailable, while sessions, approvals, Chat and terminals over the tailnet keep working. D1 state is kept and the cleanup trigger keeps running. Add the domain back to restore service.

## Rollback

Two ways, neither scripted in the repository:

- **Cloudflare version rollback**, in the dashboard under the Worker's Deployments. A Worker version includes its bindings, secrets among them, so rolling back past an APNs or HMAC rotation restores the old secret values. Cloudflare refuses a rollback across a Durable Object class change.
- **Redeploy a reviewed commit**: check out the earlier commit and deploy as above with the current secrets file. Prefer this when a secret has changed since the target version.

D1 is not rolled back. The schema has only forward migrations, `0001_relay.sql` and `0002_attestation.sql`, so the target code must run on the current schema. Never restore D1 with Time Travel as a rollback: it can bring back revoked grants, spent quotas and assertion counters that replay protection relies on. After a rollback, run `collied apns test` from an operator machine and watch the status codes. If no safe version exists, disable the public route.

## Incidents

### Cloudflare error 1027

Cloudflare returns error 1027 when an account on Workers Free passes its 100,000 daily requests; the count resets at 00:00 UTC. Workers Paid has no daily request limit, so a 1027 from the relay means the account is no longer on Paid. The response is a Cloudflare error page, not relay JSON; collied treats it as a temporary failure and backs off.

1. Check the account's plan and billing in the dashboard, and restore Workers Paid.
2. Stop invitations until it is restored. The relay cannot run as configured on Free: its 250 ms CPU limit needs Paid, and the [Free-plan CPU gate](reviews/relay-free-tier.md) failed.

### HTTP 429

Identify the result in the JSON body.

| Result | Cause | Response |
|---|---|---|
| `rate_limited` on `/v1/send` | `SEND_LIMITER`: 60 a minute per source IP, installation or route, per Cloudflare location | Transient; collied retries after `retry_after` (60 s). Phones behind one shared IP share the allowance. Sustained without legitimate traffic, treat as abuse |
| `rate_limited` on `/v1/challenge` or `/v1/control` | `CONTROL_LIMITER` 30 a minute or `ENROLL_LIMITER` 5 a minute, per IP, installation and one shared key per location | Expected briefly when many testers enroll together; phones retry. Sustained, stop invitations |
| `daily_quota` | An installation's 1,000 attempts (700 routine), or the global 20,000 (16,000 routine) | Read `global_usage`. At its cap, every phone waits for `reset_at` (00:00 UTC): stop invitations and lower the tester limit. Otherwise one phone used its own budget, as designed |
| `route_limit` (control) | 128 controls per installation, 1,000 controls or 30 enrollments a day globally, 8 machines, 8 Live Activities, 64 pairing or 128 route records per installation | Read `control_usage`. At 30 enrollments, new phones wait until 00:00 UTC |

Apple's own 429 reaches collied as `503 upstream_unavailable` with a 60-second `retry_after`, not as 429. Raising any cap is a code change with a capacity review; never reset a counter by hand.

### D1 exhaustion or unavailability

The relay fails closed: sends and controls return `503 unavailable` when D1 errors, times out after five seconds or a secret is missing. The crypto actor also returns `503 unavailable` when it is busy, since it refuses overlapping operations; Durable Object metrics tell the two apart.

Workers Paid has no daily D1 cap; usage past the inclusions is billed. What can still exhaust D1 is the 10 GB database limit, a burst of queries that one database processes one at a time, or the Free daily limits if the account fell back to Free.

1. Check D1 metrics for storage, rows and errors, and the account's plan.
2. Run the cleanup backlog count and confirm the hourly trigger runs.
3. Stop invitations while the cause is unknown; disable the public route during abuse.
4. Never delete security rows, reset quotas, restore from Time Travel or bypass validation to restore delivery.

### APNs configuration errors

`502 configuration_error` means Apple refused the relay's request for a reason other than an invalid token. The relay keeps the registration and returns no reason. Check, in order: `APNS_KEY_ID` and `APNS_PRIVATE_KEY` belong to the same key and that key is not revoked; `APNS_TEAM_ID` is the publisher team; `APNS_ENVIRONMENT` is `production` for TestFlight and App Store builds. Fix the secrets, deploy and run `collied apns test`.

## Incident record

Record the UTC interval, deployment version and source commit, aggregate counts and status codes, the affected feature and the mitigation. Leave out IPs, installation, pairing and route IDs, destinations, grants, payloads and secrets. Tell users that notifications were degraded and that sessions over the tailnet stayed available; an APNs acceptance never proves that a notification was displayed.

## Cost

Workers Paid has a $5 monthly minimum. The application quotas are not a spending cap: rejected requests, challenges, invalid proofs and other account projects still count. [Budget alerts](https://developers.cloudflare.com/billing/manage/budget-alerts/) notify but never pause usage. The only hard stop is disabling the public route.
