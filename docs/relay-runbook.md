# Relay operator runbook

Procedures for the publisher operating the production push relay at `collie-push.rbstp.dev`. [Relay](relay.md) holds the design, contracts and quotas. Never put Cloudflare account IDs, database IDs, secret values, grants, destinations or IP addresses in the repository, an issue or an incident record.

## Deployment

`relay/src/worker.ts` exports the public handler and the single `RelayCrypto` Durable Object, which verifies App Attest and signs APNs provider tokens. `relay/cloudflare.config.ts` sets `limits.cpuMs: 250`, `observability.enabled: false`, `workersDev: false`, `previewUrls: false` and these bindings:

| Binding | Kind | Purpose |
|---|---|---|
| `DB` | D1 | Installations, pairings, routes, attestations, counters and quotas |
| `CRYPTO` | Durable Object `RelayCrypto` | Attestation, assertion and provider-token signing |
| `APNS_ENVIRONMENT` | Text | `production` or `development`; must match every grant and operation |
| `APNS_TEAM_ID`, `APNS_KEY_ID`, `APNS_PRIVATE_KEY` | Secrets | Publisher APNs key; the team ID also forms the App Attest App ID |
| `SEND_HMAC_KEYS` | Secret | One or two HMAC keys for send grants and control challenges |
| `ATTEST_MIN_BUNDLE_VERSION` | Text | Lowest app build number accepted by App Attest checks, a whole number from 1 to 999999999 |
| `SEND_LIMITER` | Rate limit, 60 per 60 s | Sends, keyed by source IP, installation and route generation |
| `CONTROL_LIMITER` | Rate limit, 30 per 60 s | Challenges and controls, keyed by source IP, installation and one shared key |
| `ENROLL_LIMITER` | Rate limit, 5 per 60 s | Enrollments, keyed by source IP and one shared key |

The checked-in configuration is the deployment template: the hourly cleanup Cron trigger (`0 * * * *`), a placeholder database ID, `APNS_ENVIRONMENT` set to `development` and the production `ATTEST_MIN_BUNDLE_VERSION`. For production, set the real database ID and `production` locally and never commit them. The `collie-push.rbstp.dev` Custom Domain is attached in the Cloudflare dashboard, not in the configuration.

Deploy as [Relay](relay.md#operations) describes: unset `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_API_KEY` and `CLOUDFLARE_EMAIL`, check the account with `cf auth whoami`, set the verified `CLOUDFLARE_ACCOUNT_ID`, then from `relay/`:

```sh
npm ci
npm run check
npm test
cf build
cf deploy --prebuilt --no-provision --secrets-file /secure/path/secrets.json
```

The secrets file is private JSON, mode 0600, outside the repository. `cf` merges it over the Worker's existing secrets, so include only the secrets you change, and delete the file after the deploy. Never pass a secret value as a command argument.

## Monitoring

The relay keeps no request logs. Keep observability disabled: never enable invocation logs, traces or header capture to investigate real traffic. Monitor these aggregate sources:

| Source | What to read |
|---|---|
| Worker metrics in the Cloudflare dashboard | Requests, errors and CPU time percentiles against the 250 ms limit. A request over the limit fails with Cloudflare error 1102 |
| `RelayCrypto` Durable Object metrics | Requests and duration; one actor running continuously uses at most 342,835.2 GB-s a month against 400,000 included |
| D1 metrics | Rows read and written, storage against the 10 GB per-database limit |
| HTTP analytics of the `rbstp.dev` zone, filtered to the relay host | Responses by status code, read with the result table in [Relay](relay.md#send-contract-v1) |

Account usage counts against Workers Paid's monthly inclusions, shared with every other project in the account: 10 million requests, 30 million CPU ms and 50 million D1 rows written. At its daily caps, the relay model in [Relay](relay.md#paid-plan-and-cost-controls) uses about 7% of the requests and 6% of the writes.

D1 holds the aggregate counters. No script queries them: run read-only statements in the dashboard's D1 console or with `cf`, and select counts only, never identifiers, destinations, keys or receipts.

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

`day` counts days since 1970-01-01 UTC. Each counter row keeps only its latest day and resets at the first charge after 00:00 UTC, so record the values shortly before midnight UTC to keep a daily series. `global_usage` caps at 20,000 attempts and 16,000 routine; `control_usage` at 1,000 controls and 30 enrollments. A growing `cleanup_backlog` means the hourly trigger is not running or not keeping up.

The relay records no APNs reason. It maps Apple's answers to three results: `410 invalid_route` when Apple invalidates a token (a steady trickle is normal), `502 configuration_error` for any other Apple rejection, and `503 upstream_unavailable` for Apple 5xx, 429 or network failures. Any 502 needs action; see [APNs configuration errors](#apns-configuration-errors). To reproduce a failure, use a machine paired with an operator-owned phone: `collied apns test` sends through the relay and prints each result, and `collied doctor` shows the daemon's last send error.

| `collied doctor` last error | Relay result |
|---|---|
| relay daily quota exhausted | `429 daily_quota` |
| relay temporarily unavailable | `429 rate_limited`, any other 429 or 5xx, including `503 upstream_unavailable`, `503 unavailable` and Cloudflare error pages with those statuses |
| relay returned an unexpected response | Any other response without relay JSON, such as another Cloudflare error page |
| relay APNs configuration error; registration retained | `502 configuration_error` |
| relay grant unauthorized; open the paired app to renew | `401 unauthorized` |

Before raising the public TestFlight tester limit, read the last seven days against the review points in [Relay](relay.md#operations): peak `global_usage` total above 70% of 20,000, more than 1% of legitimate sends rejected with `daily_quota`, or more than 30 daily active phones. [App Review notes](app-review.md#public-link-capacity) set the capacity cap.

## Refuse old app builds

App Attest checks refuse an app whose bundle version is not a whole number of at most nine digits at or above `ATTEST_MIN_BUNDLE_VERSION`, whenever Apple's attestation or assertion carries its bundle-version extension. TestFlight builds take the release workflow's run number as their bundle version, so each new build passes without a relay change. A refused build gets `401 unauthorized` for enrollment, renewal and revocation; its grants already issued keep sending until they expire.

To refuse older builds, raise the minimum in `relay/cloudflare.config.ts` through a reviewed pull request and deploy. Raise it only once no tester should still run the builds below it: their phones can then fail to renew or revoke, and keep relay pushes only until their grants expire.

## APNs key rotation

The relay signs every push, Live Activities included, with the publisher's one APNs key.

1. In the Apple Developer portal, under Keys, create a key with Apple Push Notifications service and note its Key ID.
2. Put the new `APNS_KEY_ID` and `APNS_PRIVATE_KEY` together in the secrets file, leave the other secrets unchanged, and deploy.
3. From a machine paired with an operator-owned phone on a production build, run `collied apns test` and confirm the alert arrives.
4. Revoke the old key in the portal only after that test passes and at least 45 minutes after the deploy: provider tokens are cached in memory for 45-minute windows, so an instance can sign with the old key until its window ends.

If the old key leaked, deploy a new key at once and revoke the old one right after. If you cannot deploy a new key quickly, revoke the old key anyway: every send returns `502 configuration_error` until the new key is deployed, and sessions over the tailnet keep working.

## HMAC secret rotation

`SEND_HMAC_KEYS` is a JSON object of one or two entries. Each ID is 1 to 32 letters, digits, underscores or hyphens; each value is 32 bytes from a cryptographically secure random source, in canonical unpadded base64url. The repository has no generator. The first entry signs new grants and challenges; both entries verify. Use IDs that are not integers, such as `hmac_20261020`: JavaScript orders integer-like keys first, which would change the signing key.

Planned rotation:

1. Deploy with the new entry first and the current one second.
2. On an operator-owned phone, force-quit and relaunch the app, which renews its app route under the new key, then run `collied apns test` on a paired machine.
3. Wait 30 days. Grants last 30 days and phones renew from seven days before expiry, so by then no unexpired grant signed with the old key remains. A phone not launched in that time has already lost relay pushes and renews at its next launch.
4. Deploy with only the new entry.

Emergency rotation, when a key leaked: deploy with only a new entry. Every grant and pending challenge signed with the old key fails at once: sends return `401 unauthorized`, and `collied doctor` tells the user to open the paired app. The app renews at launch, so users must force-quit and relaunch it. A Live Activity route renews only when ActivityKit supplies a new token, so a running Live Activity keeps failing until it ends and a new one starts.

## Revoke a pairing or an installation

A user revokes a machine by removing it in the app, which sends an authenticated `revoke_pairing`. The app never revokes a whole installation.

There is no administrative endpoint or script, and with no accounts or logs the operator cannot trace traffic to an installation or pairing. The IDs exist only on the phone and in grants. A user who reports a compromised machine can read them from the grant in that machine's private `push.json`: the grant is `v1.<key_id>.<claims>.<signature>`, and only the claims segment, base64url JSON, is needed. Never ask for or copy the whole grant.

With an exact ID, run reviewed statements against the production database and check that each changes exactly one row.

A pairing, with the app's own `revoke_pairing` statement from `relay/src/control.ts`:

```sql
UPDATE pairings SET epoch = epoch + 1, revoked = 1
WHERE id = '<pairing_id>' AND installation_id = '<installation_id>' AND revoked = 0;
```

An installation, as one file run in one batch, in this order:

```sql
UPDATE attestations SET revoked = 1 WHERE installation_id = '<installation_id>' AND revoked = 0;
UPDATE installations SET epoch = epoch + 1 WHERE id = '<installation_id>';
```

An installation needs both statements. The epoch bump alone stops existing grants, but the next challenge carries the new epoch and registers fresh routes. The `revoked` flag alone refuses controls, but sends do not check it, so existing grants keep sending.

Revoking a pairing makes its routes return `410 invalid_route`, and collied drops the grant. Revoking an installation does the same for all its routes and refuses its controls with `401 unauthorized`. It is not a ban: once the phone's last known grant has expired, the app treats `401 unauthorized` as lost state, creates a new installation and enrolls again, within the 30-a-day enrollment cap. Revocation therefore cuts a phone's relay pushes until its grants lapse, at most 30 days. Never delete rows: revocation relies on the epoch and revoked flags, and a deleted pairing could be recreated at epoch 0, the epoch its old grants carry. Cleanup removes revoked records after the retention delay.

To revoke everything at once, use the emergency HMAC rotation.

## Stop enrollment or disable the public route

No setting stops enrollment alone. In order of reach:

1. **Stop invitations.** In App Store Connect, disable the public TestFlight link or lower its tester limit. Testers already in keep the app.
2. **Refuse App Attest for every build.** Set `ATTEST_MIN_BUNDLE_VERSION` to `999999999` in `relay/cloudflare.config.ts`, deploy, and commit the change so later deploys keep it until the incident closes. A minimum just above the newest build would let the next TestFlight run number through. As with [old builds](#refuse-old-app-builds), an attestation or assertion without Apple's bundle-version extension still passes, and existing grants keep sending until they expire. A missing or invalid value fails every enrollment, renewal and revocation with `503 unavailable`.
3. **Lower the daily enrollment cap.** The 30-a-day cap is in the `enrollment_limit` trigger of `relay/migrations/0002_attestation.sql`. Changing it takes a new, reviewed migration; there is no runtime setting.
4. **Disable the public route.** In the dashboard, remove the `collie-push.rbstp.dev` Custom Domain from the Worker (Settings, Domains & Routes). A deploy does not add it back, since the configuration does not declare it, and with workers.dev and preview URLs off no other public path remains. Enrollment, renewal, revocation and sends all stop; collied backs off and doctor reports the relay unavailable, while sessions, approvals, Chat and terminals over the tailnet keep working. D1 state stays and the cleanup trigger keeps running. Add the domain back to restore service.

## Rollback

Stop invitations first. Then roll back in one of two ways, neither scripted:

- **Cloudflare version rollback**, in the dashboard under the Worker's Deployments. A Worker version includes its bindings, secrets among them, so rolling back past an APNs or HMAC rotation restores the old secret values. Cloudflare refuses a rollback across a Durable Object class change.
- **Redeploy a reviewed commit**: check out the earlier commit and deploy as above with the current secrets file. Prefer this when a secret has changed since the target version. A commit from before `ATTEST_MIN_BUNDLE_VERSION` reads the `ATTEST_BUNDLE_VERSIONS` secret instead, a JSON array of exact bundle versions: put it in the secrets file with every build testers run, or enrollment, renewal and revocation fail with `503 unavailable`.

D1 is not rolled back. The schema has only forward migrations, `0001_relay.sql` and `0002_attestation.sql`, so the target code must run on the current schema. Never restore D1 with Time Travel as a rollback: it can bring back revoked grants, spent quotas and the assertion counters that replay protection relies on. After a rollback, run `collied apns test` from an operator machine and watch the status codes. If no safe version exists, disable the public route.

## Incidents

### Cloudflare error 1027

Cloudflare returns error 1027 when an account on Workers Free passes its 100,000 daily requests; the count resets at 00:00 UTC. Workers Paid has no daily request limit, so a 1027 from the relay means the account is no longer on Paid. The response is a Cloudflare error page, not relay JSON; collied treats it as a temporary failure and backs off.

1. Check the account's plan and billing in the dashboard, and restore Workers Paid.
2. Stop invitations until it is restored. The relay cannot run as configured on Free: its 250 ms CPU limit needs Paid, and the [Free-plan CPU gate](reviews/relay-free-tier.md) failed.

### HTTP 429

Identify the result in the JSON body.

| Result | Cause | Response |
|---|---|---|
| `rate_limited` on `/v1/send` | `SEND_LIMITER`: 60 a minute per source IP, installation or route, per Cloudflare location | Transient; collied retries after `retry_after` (60 s). Machines behind one IP share the allowance. Sustained without legitimate traffic, treat as abuse |
| `rate_limited` on `/v1/challenge` or `/v1/control` | `CONTROL_LIMITER` (30 a minute) or `ENROLL_LIMITER` (5 a minute), per limiter key and Cloudflare location | Expected briefly when many testers enroll together; phones retry. Sustained, stop invitations |
| `daily_quota` | An installation's 1,000 attempts (700 routine), or the global 20,000 (16,000 routine) | Read `global_usage`. At its cap, every phone waits for `reset_at` (00:00 UTC): stop invitations and lower the tester limit. Otherwise one phone used its own budget, as designed |
| `route_limit` (control) | 128 controls a day per installation; 1,000 controls or 30 enrollments a day globally; 8 machines, 8 Live Activities, 64 pairing or 128 route records per installation | Read `control_usage`. At 30 enrollments, new phones wait until 00:00 UTC |

Apple's own 429 reaches collied as `503 upstream_unavailable` with a 60-second `retry_after`, not as 429. Raising any cap is a code change with a capacity review; never reset a counter by hand.

### D1 exhaustion or unavailability

The relay fails closed: sends and controls return `503 unavailable` when D1 errors or takes more than five seconds, or a secret is missing. The crypto actor refuses overlapping operations, so a busy actor also yields `503 unavailable`; Durable Object metrics tell the two causes apart.

Workers Paid has no daily D1 cap; usage past the inclusions is billed. What can still exhaust D1 is the 10 GB database limit, a burst of queries that one database processes one at a time, or the Free daily limits if the account fell back to Free.

1. Check D1 metrics for storage, rows and errors, and the account's plan.
2. Run the cleanup backlog count and confirm the hourly trigger runs.
3. Stop invitations while the cause is unknown; disable the public route during abuse.
4. Never delete security rows, reset quotas, restore from Time Travel or bypass validation to restore delivery.

### APNs configuration errors

`502 configuration_error` means Apple refused the relay's request for a reason other than an invalid token. The relay keeps the registration and returns no reason. Check, in order: `APNS_KEY_ID` and `APNS_PRIVATE_KEY` belong to the same key and that key is not revoked; `APNS_TEAM_ID` is the publisher team. Fix the secrets, deploy and run `collied apns test`.

## Incident record

Record the UTC interval, deployment version and source commit, aggregate counts and status codes, the affected feature and the mitigation. Leave out IPs, installation, pairing and route IDs, destinations, grants, payloads and secrets. Tell users that notifications were degraded and that sessions over the tailnet stayed available; an APNs acceptance never proves that a notification was displayed.

## Cost

Workers Paid has a $5 monthly minimum. The application quotas are not a spending cap: rejected requests, challenges, invalid proofs and other account projects still count. [Budget alerts](https://developers.cloudflare.com/billing/manage/budget-alerts/) notify but never pause usage; the only hard stop is disabling the public route. A failed CPU or capacity gate still blocks rollout.
