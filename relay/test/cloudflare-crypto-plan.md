# Isolated Free crypto experiment plan

Status: the single reviewed 42-attempt cycle ran on 2026-10-16 and its exact temporary resources were removed; `docs/reviews/relay-free-tier.md` records the results, and production now runs this `RelayCrypto` design on Workers Paid (`docs/relay.md`). A rerun needs a separate explicit decision. The experiment uses no billing, Paid plan, production hostname, real APNs secret or fresh device enrollment.

## Source and limits

The experiment keeps production's `RelayCrypto` methods, attestation/assertion verifier, Apple roots, and D1 schema and transactions. `test/cloudflare-crypto-benchmark.ts` exports a guarded Worker and `BenchmarkCrypto`, a test subclass with a fixed historical Apple-fixture method and a new-signer method that production RPCs lack. Before any database or actor access, the Worker answers 404 to an absent, wrong or unconfigured bearer secret. It seeds one synthetic verified key, fakes APNs with no outbound fetch, runs challenge, control and send through the production implementation, and logs no bodies, grants, challenges or secrets. The local-test admission-delay helper `compute-fixture.ts` is not bundled.

One explicit initialization creates a new private directory, ephemeral signing keys and a shared fsynced `{attempts:0,limit:50}` journal. Every Worker-origin request, probes, failed network attempts and authorized supplemental clients included, goes through `benchmarkClient` with that journal. Redirects are errors; missing, corrupt or locked state fails closed. The driver never retries, refuses a second matrix once its response log exists, and gives each attempt a 15-second deadline. A failure stops the matrix and authorizes no recovery traffic, directory or counter.

| Calls | Case |
|---:|---|
| 2 | Missing and wrong guard, both 404 |
| 1 | Synthetic installation seed |
| 4 | New provider signer in the actor |
| 4 | Actor provider cache: first use, three warm |
| 6 | Genuine historical Apple attestation |
| 6 | Same chain/receipt, invalid nonce |
| 8 | Four challenges and four signed controls: first route, three rotations |
| 1 | Re-signed consumed challenge rejection |
| 8 | Mocked sends: first public cache use, seven warm |
| 1 | Hostile plaintext-field rejection |
| 1 | Bounded database cleanup |
| **42** | **Total; hard ceiling 50 for all clients combined** |

Exactly 15 calls can mutate; the one-phone, one-pairing, one-route fixture bounds each to 64 D1 writes, indexes and triggers included, so with 40 reserved for the two schemas the ceiling is 1,000. The driver reserves before each mutation and checks returned cumulative metadata; the remote schema must fit its 40-write reserve. A network failure consumes its reservation and ends the run; nothing is refunded, resumed or supplemented.

## Reviewed resources and commands

Use only the verified personal account `e0d20426f1862227c675befdcc4d1e7e` and zone `203b54799bb609edd64c2f702fe431cb`. Name the new Worker and D1 database `collie-push-crypto-bench-20261016` and the Custom Domain `collie-push-crypto-bench-20261016.rbstp.dev`; stop on any collision. Never register an account-wide workers.dev subdomain or overwrite an existing record. In each Cloudflare shell:

```sh
unset CLOUDFLARE_API_TOKEN CLOUDFLARE_API_KEY CLOUDFLARE_EMAIL
export CLOUDFLARE_ACCOUNT_ID=e0d20426f1862227c675befdcc4d1e7e
cf auth whoami
```

Before any write, record fresh evidence of the account and Free plan (empty subscriptions or a rejected CPU setting, not the zone plan), snapshot Workers, D1, Durable Object namespaces, exact-domain attachments and exact-name DNS records, and verify every proposed name is absent. Record creation outputs, exact IDs, timestamps and absence evidence, actor namespace/object and managed DNS IDs included. Never print secret files.

From the reviewed `relay/` directory, with pinned Node and tooling:

```sh
node test/cloudflare-crypto-driver.ts initialize /private/tmp/collie-crypto-bench-20261016-private
cf d1 create --name collie-push-crypto-bench-20261016
```

Apply only the reviewed `migrations/0001_relay.sql` and `0002_attestation.sql` with `cf d1 query <owned-database-id> --batch @<schema-batch.json>`, keeping trigger statements whole; keep the returned metadata and require at most 40 writes. Stage a private build directory with the reviewed `src/`, `test/`, package and lock files and installed dependencies, using `test/cloudflare-crypto.config.ts` as `cloudflare.config.ts` with only its zero D1 placeholder replaced by the new database ID. From there:

```sh
cf build
cf deploy --prebuilt --no-provision --secrets-file /private/tmp/collie-crypto-bench-20261016-private/secrets.json
```

Before deploying, check that the prebuilt `worker.config.json` and bundle hold only the exact Worker, SQLite `BenchmarkCrypto` class, fixed actor binding, new D1 database and domain, with no CPU/Paid setting, admission-delay helper, real credential or outbound APNs code; `--no-provision` disables unrelated binding resource creation, so the class and Custom Domain are the intended creations. Record exact IDs at once and check readiness through Cloudflare API metadata, never uncounted HTTP probes.

Only after independent review and orchestration authorization, run the one matrix:

```sh
node test/cloudflare-crypto-driver.ts run /private/tmp/collie-crypto-bench-20261016-private https://collie-push-crypto-bench-20261016.rbstp.dev
```

Capture durable stdout/stderr and an exit sidecar, and wait for exit. `responses.jsonl` holds redacted case timestamps, outcomes and D1 metadata; `attempts.json` is the authoritative attempt count. Never rerun on failure; cleanup verification needs no Worker request.

## Separate CPU collection

A read-only `cf sql datasets --account-tag <verified-account> --include-columns --include-wae=false --include-lex=false` catalogue confirmed these `logs.workersLogs` columns. Query only the experiment script and captured window:

```sh
cf sql query "SELECT timestamp, scriptName, invocationType, invocationLabel, cpuTimeMs, wallTimeMs, sampleInterval, error, httpStatus, requestId, rayId, url FROM logs.workersLogs WHERE scriptName = 'collie-push-crypto-bench-20261016' ORDER BY timestamp LIMIT 2000 FORMAT JSON" --scope-account e0d20426f1862227c675befdcc4d1e7e --time-since '<experiment-start-RFC3339>' --time-until '<experiment-end-RFC3339>'
```

Split public `fetch` from actor invocations (`jsrpc` in the observed logs) by `invocationType` and `invocationLabel`, and record raw sampling intervals, error counts and small-sample maxima or quantiles per driver case. Generic actor labels, missing parent ray IDs and clock skew prevented per-case actor attribution in the actual run: report aggregate actor samples unless a reliable join exists. Capture actor namespace/object metadata separately; never hold actor CPU against the public Worker's 10 ms limit. Label first uses, not cold calls: a new signer does not prove a fresh isolate, and a small run cannot establish a production p99.

Per-request `wallTimeMs` is not billed actor duration; Cloudflare documents `durableObjectsInvocationsAdaptiveGroups` and `durableObjectsPeriodicGroups` for namespace request, CPU and duration analytics. If `cf` cannot expose them, record that gate as unmeasured; never substitute wall time for GB-s, invent field names, extend Worker load or claim full Free viability.

## Exact cleanup, CF API only

After all owned load processes exit, collect metrics, read-only synthetic state and counters, owned IDs and fresh before snapshots. Stage `test/cloudflare-crypto-cleanup.config.ts` as `cloudflare.config.ts`: an inert 404 handler with no bindings, an empty domain list and only the tombstone `BenchmarkCrypto: exports.durableObject({state:'deleted'})`.

```sh
cf build
cf deploy --prebuilt --no-provision
cf durable-objects namespaces list
```

`cf workers triggers deploy --prebuilt` with `domains: []` reported no targets but did not detach the Custom Domain. The orchestrator approved this order instead, which removed the exact owned resources:

1. Confirm the recorded namespace ID is missing from the list. The tombstone deletes only this Worker's new class, namespace and data; [Cloudflare's class lifecycle contract](https://developers.cloudflare.com/durable-objects/reference/durable-objects-migrations/) requires an absent class and binding and rejects external references.
2. Require fresh Worker metadata to show only the recorded temporary domain and no external Worker, Durable Object, queue or outbound references, then delete this exact Worker, which removes its domain and managed DNS:

   ```sh
   cf workers delete <exact-new-worker-id> --force
   ```

3. Confirm through the API that the domain and managed DNS record are gone (if either remains, stop and report: deleting it needs an exact ownership decision), then delete only the new D1 database:

   ```sh
   cf d1 delete <exact-new-database-id> --force
   ```

4. Confirm through the API that the domain, managed DNS, Worker, D1 database and namespace are absent and that preexisting inventories match their before snapshots.

Stop and report any unexpected identity, reference or cleanup failure. Never use the broad `--delete-with-references`, modify existing resources or delete unrelated ones. Preserve the redacted resource-ID list, before/create/after metadata, logs, actual attempt count and process exit sidecars.

Sources: [DO limits](https://developers.cloudflare.com/durable-objects/platform/limits/), [Free pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/), [DO metrics](https://developers.cloudflare.com/durable-objects/observability/metrics-and-analytics/), [cf builds/deployments](https://developers.cloudflare.com/cf/projects/).
