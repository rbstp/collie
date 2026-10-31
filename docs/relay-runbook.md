# Relay operator runbook

[Relay](relay.md) holds the design, contracts and quotas of the production push relay at `collie-push.rbstp.dev`. Never put Cloudflare account or database IDs, secret values, grants, destinations or IP addresses in the repository, an issue or an incident record.

## Deployment

`relay/cloudflare.config.ts` deploys `relay/src/worker.ts`, the public Worker and the `RelayCrypto` Durable Object, with `limits.cpuMs: 250`, `observability.enabled: false`, `workersDev: false`, `previewUrls: false`, the hourly cleanup Cron trigger (`0 * * * *`) and these bindings:

| Binding | Kind | Purpose |
|---|---|---|
| `DB` | D1 | Registrations, attestations, counters and quotas |
| `CRYPTO` | Durable Object `RelayCrypto` | App Attest verification and APNs provider-token signing |
| `APNS_ENVIRONMENT` | Text | `production` or `development`; must match every grant and operation |
| `APNS_TEAM_ID`, `APNS_KEY_ID`, `APNS_PRIVATE_KEY` | Secrets | Publisher APNs key; the team ID also forms the App Attest App ID |
| `SEND_HMAC_KEYS` | Secret | One or two HMAC keys for send grants and control challenges |
| `ATTEST_MIN_BUNDLE_VERSION` | Text | Lowest accepted app build number, 1 to 999999999; the file holds the production value |
| `SEND_LIMITER` | Rate limit, 60 a minute | Sends, per source IP, installation and route generation |
| `CONTROL_LIMITER` | Rate limit, 30 a minute | Challenges and controls, per source IP, installation and one shared key |
| `ENROLL_LIMITER` | Rate limit, 5 a minute | Enrollments, per source IP and one shared key |

For production, replace the placeholder database ID and `development` environment with the real ID and `production` locally, and never commit them. The Custom Domain is attached in the dashboard, not in the file.

To deploy, unset `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_API_KEY` and `CLOUDFLARE_EMAIL`, check the account with `cf auth whoami`, set the verified `CLOUDFLARE_ACCOUNT_ID`, and run from `relay/`:

```sh
npm ci
npm run check
npm test
cf build
cf deploy --prebuilt --no-provision --secrets-file /secure/path/secrets.json
```

The secrets file is JSON, mode 0600, outside the repository; delete it after the deploy. `cf` merges it over the existing secrets, so include only those you change. Never pass a secret value as a command argument.

## Monitoring

The relay keeps no request logs. Never enable invocation logs, traces or header capture to investigate real traffic. Read aggregates only:

| Source | What to read |
|---|---|
| Worker metrics | Requests, errors and CPU time percentiles against the 250 ms limit; a request over it fails with Cloudflare error 1102 |
| `RelayCrypto` metrics | Requests and duration against the included GB-s |
| D1 metrics | Rows read and written; storage against the 10 GB per-database limit |
| `rbstp.dev` HTTP analytics for the relay host | Status codes, read with [Relay's result table](relay.md#send-contract-v1) |

Query the D1 counters read-only, in the D1 console or with `cf`, selecting counts only, never identifiers, destinations, keys or receipts:

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

`day` counts days since 1970-01-01 UTC. Counter rows reset at their first charge after 00:00 UTC, so record values shortly before midnight UTC for a daily series. A growing `cleanup_backlog` means hourly cleanup is failing or falling behind.

A steady trickle of `410 invalid_route` (tokens Apple invalidated) is normal; every `502 configuration_error` needs [action](#apns-configuration-errors).

To reproduce a failure, run `collied apns test` on a machine paired with an operator-owned phone; it prints each relay result. In `collied doctor`, "relay temporarily unavailable" means any 429 or 5xx without a more specific message, Cloudflare error pages included.

Before raising the public TestFlight tester limit, check seven days of counters against [Public link capacity](app-review.md#public-link-capacity) and [Relay's review points](relay.md#operations).

## Refuse old app builds

App Attest checks refuse bundle versions below `ATTEST_MIN_BUNDLE_VERSION` when Apple's attestation or assertion carries its bundle-version extension. TestFlight bundle versions are the release workflow's run number, so new builds pass unchanged. A refused build gets `401 unauthorized` for enrollment, renewal and revocation; grants it already holds keep sending until they expire.

To refuse older builds, raise the minimum in `relay/cloudflare.config.ts` through a reviewed pull request and deploy, once no tester should still run them.

## APNs key rotation

1. In the Apple Developer portal, under Keys, create a key with Apple Push Notifications service and note its Key ID.
2. Put the new `APNS_KEY_ID` and `APNS_PRIVATE_KEY` together in the secrets file, without the other secrets, and deploy.
3. On a machine paired with an operator-owned phone running a production build, run `collied apns test` and confirm the alert arrives.
4. Once that passes and at least 45 minutes after the deploy, revoke the old key in the portal; instances cache provider tokens for 45-minute windows.

If the old key leaked, deploy a new key at once, then revoke the old one. If you cannot deploy quickly, revoke it anyway: every send returns `502 configuration_error` until the new key is live, while tailnet sessions keep working.

## HMAC secret rotation

`SEND_HMAC_KEYS` is a JSON object mapping one or two IDs (1 to 32 letters, digits, underscores or hyphens) to 32 cryptographically secure random bytes in canonical unpadded base64url. The repository has no generator. The first entry signs new grants and challenges; both verify. Use non-integer IDs such as `hmac_20261020`: JavaScript orders integer-like keys first, which would change the signing key.

Planned rotation:

1. Deploy with the new entry first and the current one second.
2. Force-quit and relaunch the app on an operator-owned phone to renew its app route under the new key, then run `collied apns test` on a paired machine.
3. Wait 30 days, the grant lifetime, so no unexpired old-key grant remains; a phone not launched meanwhile renews at its next launch.
4. Deploy with only the new entry.

Emergency rotation, when a key leaked: deploy with only a new entry. Old-key grants and pending challenges fail at once with `401 unauthorized`, and users must force-quit and relaunch the app to renew (`collied doctor` tells them to open it). A running Live Activity keeps failing until it ends and a new one starts, since its route renews only on a new ActivityKit token.

## Revoke a pairing or an installation

The app revokes a removed machine's pairing (`revoke_pairing`), never a whole installation.

With no administrative endpoint, accounts or logs, you cannot trace traffic to an installation or pairing. A user reporting a compromised machine can read the IDs from the claims segment (base64url JSON) of the grant `v1.<key_id>.<claims>.<signature>` in that machine's private `push.json`. Never ask for or copy the whole grant.

Run reviewed statements with exact IDs against the production database; each must change exactly one row. A pairing, with the app's own statement from `relay/src/control.ts`:

```sql
UPDATE pairings SET epoch = epoch + 1, revoked = 1
WHERE id = '<pairing_id>' AND installation_id = '<installation_id>' AND revoked = 0;
```

An installation, as one file run in one batch, in this order:

```sql
UPDATE attestations SET revoked = 1 WHERE installation_id = '<installation_id>' AND revoked = 0;
UPDATE installations SET epoch = epoch + 1 WHERE id = '<installation_id>';
```

An installation needs both: the epoch bump alone stops existing grants, but the next challenge carries the new epoch and registers fresh routes; the `revoked` flag alone refuses controls, but sends do not check it.

Revocation is not a ban: once the phone's last known grant has expired, the app enrolls a new installation, within the 30-a-day enrollment cap. It cuts a phone's relay pushes only until its grants lapse, at most 30 days.

Never delete rows: revocation relies on the epoch and revoked flags, and a deleted pairing could be recreated at epoch 0, which its old grants carry. To revoke everything at once, use the emergency HMAC rotation.

## Stop enrollment or disable the public route

No setting stops enrollment alone. In order of reach:

1. **Stop invitations.** In App Store Connect, disable the public TestFlight link or lower its tester limit. Testers already in keep the app.
2. **Refuse App Attest for every build.** Set `ATTEST_MIN_BUNDLE_VERSION` to `999999999`, deploy, and commit it so later deploys keep it until the incident closes; a minimum just above the newest build would let the next TestFlight run number through. As with [old builds](#refuse-old-app-builds), proofs without the extension still pass and existing grants keep sending. A missing or invalid value fails every enrollment, renewal and revocation with `503 unavailable`.
3. **Lower the daily enrollment cap.** The 30-a-day cap is the `enrollment_limit` trigger in `relay/migrations/0002_attestation.sql`; changing it takes a new, reviewed migration.
4. **Disable the public route.** Remove the `collie-push.rbstp.dev` Custom Domain from the Worker in the dashboard (Settings, Domains & Routes). Deploys do not add it back, and with workers.dev and preview URLs off no other public path remains. All relay traffic stops; sessions, approvals, Chat and terminals over the tailnet keep working, and D1 state and cleanup continue. Add the domain back to restore service.

## Rollback

Stop invitations, then roll back in one of two unscripted ways:

- **Cloudflare version rollback**, under the Worker's Deployments in the dashboard. A version includes its bindings and secrets, so rolling back past an APNs or HMAC rotation restores the old secrets. Cloudflare refuses a rollback across a Durable Object class change.
- **Redeploy a reviewed commit** with the current secrets file; prefer this when a secret changed since that version. A commit from before `ATTEST_MIN_BUNDLE_VERSION` reads the `ATTEST_BUNDLE_VERSIONS` secret instead, a JSON array of exact bundle versions: add it to the secrets file with every build testers run, or enrollment, renewal and revocation fail with `503 unavailable`.

D1 is not rolled back: migrations only go forward, so the target code must run on the current schema. Never use D1 Time Travel as a rollback: it can bring back revoked grants, spent quotas and the assertion counters that replay protection relies on. Afterwards, run `collied apns test` and watch the status codes; if no safe version exists, disable the public route.

## Incidents

### Cloudflare error 1027

Error 1027 means the account passed Workers Free's 100,000 requests in a UTC day: it is no longer on Workers Paid, which has no daily limit. collied gets a Cloudflare error page, not relay JSON, and backs off.

1. Check the plan and billing in the dashboard and restore Workers Paid.
2. Stop invitations until then: the relay's 250 ms CPU limit needs Paid, and the [Free-plan CPU gate](reviews/relay-free-tier.md) failed.

### HTTP 429

Rate limits count per Cloudflare location.

| Result | Cause | Response |
|---|---|---|
| `rate_limited` from `/v1/send` | `SEND_LIMITER` | Transient; collied retries after 60 s. Machines behind one IP share the allowance. Sustained without legitimate traffic, treat as abuse |
| `rate_limited` from `/v1/challenge` or `/v1/control` | `CONTROL_LIMITER` or `ENROLL_LIMITER` | Brief when many testers enroll together; phones retry. Sustained, stop invitations |
| `daily_quota` | An installation's 1,000 attempts (700 routine), or the global 20,000 (16,000 routine) | If `global_usage` is at its cap, every phone waits until `reset_at` (00:00 UTC): stop invitations and lower the tester limit. Otherwise one phone spent its own budget |
| `route_limit` (control) | A control, enrollment, machine, Live Activity or record limit ([Relay](relay.md#attested-control-contract)) | Read `control_usage`. At 30 enrollments, new phones wait until 00:00 UTC |

Apple's own 429 reaches collied as `503 upstream_unavailable`, not 429. Raising any cap is a code change with a capacity review; never reset a counter by hand.

### D1 exhaustion or unavailability

The relay fails closed with `503 unavailable` when D1 errors or takes over five seconds, a secret is missing, or the crypto actor is busy; Durable Object metrics tell D1 and actor causes apart.

Workers Paid has no daily D1 cap, but D1 can still run out at the 10 GB database limit, under a query burst (a database runs queries one at a time), or at Free's daily limits if the account fell back.

1. Check D1 metrics for storage, rows and errors, and the account's plan.
2. Run the cleanup backlog count and confirm the hourly trigger runs.
3. Stop invitations while the cause is unknown; disable the public route during abuse.
4. Never delete security rows, reset quotas, restore from Time Travel or bypass validation to restore delivery.

### APNs configuration errors

`502 configuration_error` means Apple refused the request for a reason other than an invalid token; the relay keeps the registration and records no reason. Check that `APNS_KEY_ID` and `APNS_PRIVATE_KEY` belong to the same unrevoked key, then that `APNS_TEAM_ID` is the publisher team. Fix the secrets, deploy and run `collied apns test`.

## Incident record

Record the UTC interval, deployment version and source commit, aggregate counts and status codes, the affected feature and the mitigation; never IPs, installation, pairing or route IDs, destinations, grants, payloads or secrets. Tell users notifications were degraded while tailnet sessions stayed available; an APNs acceptance never proves a notification was displayed.

## Cost

Workers Paid has a $5 monthly minimum. The application quotas cap no spending: rejected requests, challenges, invalid proofs and other account projects still count. [Budget alerts](https://developers.cloudflare.com/billing/manage/budget-alerts/) notify but never pause usage; only disabling the public route stops it.
