# Isolated Free crypto experiment plan

Status: the independently reviewed single 42-attempt cycle completed on 2026-10-16, and its exact temporary resources were removed. This is the reproducible plan and its corrected cleanup procedure, not authorization to rerun. A new experiment requires a separate explicit decision. This does not replace the failed direct-Worker result in `docs/reviews/relay-free-tier.md`. No billing, Paid plan, production hostname, real APNs secret or fresh device enrollment is part of this experiment.

## Source and limits

Production is `src/worker.ts`, exporting only the ordinary Worker and `RelayCrypto`. The fixed internal object name is `relay-crypto`. Its full attestation/assertion verifier, Apple roots and D1 schema/transactions remain unchanged. Policy and current attestation time come from trusted actor bindings.

The isolated entrypoint `test/cloudflare-crypto-benchmark.ts` exports only `BenchmarkCrypto`, a test subclass with a fixed historical Apple-fixture method and a new-signer method. Historical context never enters production RPCs. The public benchmark rejects absent/wrong/unconfigured bearer secrets before any database or actor access. It provisions one synthetic verified key only, uses fake APNs with no outbound fetch, and forwards ordinary challenge/control/send operations through the production implementation. It does not log bodies, grants, challenges or secrets. The separate `compute-fixture.ts` admission-delay helper is local-test-only and is not in this cloud bundle.

One explicit initialization creates a new private directory, ephemeral signing keys and a shared fsynced `{attempts:0,limit:50}` journal. All Worker-origin requests, including probes, failed network attempts and any authorized supplemental client, must use that same journal through `benchmarkClient`. Redirects are errors; missing/corrupt/locked state fails closed. The driver performs no retries and refuses a second matrix run because its durable response log already exists. Every HTTP attempt has a 15-second deadline. A failure stops the matrix; it does not authorize recovery traffic or a new directory/counter.

| Calls | Case |
|---:|---|
| 2 | Missing and wrong guard, both 404 |
| 1 | Synthetic installation seed |
| 4 | New provider signer in the actor |
| 4 | Actor provider cache: first use, then three warm calls |
| 6 | Genuine historical Apple attestation |
| 6 | Same complete chain/receipt with invalid nonce |
| 8 | Four production challenges and four signed controls, first route plus three rotations |
| 1 | Re-signed consumed challenge rejection |
| 8 | Production mocked sends: first public cache use, then seven warm calls |
| 1 | Hostile plaintext-field rejection |
| 1 | Bounded database cleanup |
| **42** | **Total planned attempts; hard ceiling 50 for every client combined** |

There are exactly 15 potentially mutating calls. The one-phone/one-pairing/one-route fixture bounds each to 64 D1 writes including indexes/triggers; reserve 40 writes for the two initial schemas, for a conservative total of 1,000. The driver reserves before each mutation and checks returned cumulative metadata. The initial remote schema metadata must fit its 40-write reserve before running the matrix. A network failure consumes its reservation and ends the run. No counters are refunded. There is no matrix resume or supplemental mutation plan.

## Reviewed resources and commands

Use only the verified personal account `e0d20426f1862227c675befdcc4d1e7e` and zone `203b54799bb609edd64c2f702fe431cb`. Exact new names are Worker/D1 `collie-push-crypto-bench-20261016` and Custom Domain `collie-push-crypto-bench-20261016.rbstp.dev`. Stop on any collision. Do not register an account-wide workers.dev subdomain or overwrite an existing record.

In each Cloudflare shell:

```sh
unset CLOUDFLARE_API_TOKEN CLOUDFLARE_API_KEY CLOUDFLARE_EMAIL
export CLOUDFLARE_ACCOUNT_ID=e0d20426f1862227c675befdcc4d1e7e
cf auth whoami
```

Before writes, record fresh intended-account/Free-plan evidence and snapshots of Workers, D1, Durable Object namespaces, exact-domain attachments and exact-name DNS records. Verify all proposed names are absent. The previously observed Free-plan rejection and empty subscriptions are evidence for the account plan; the zone plan alone is not. Record creation outputs, exact IDs, created timestamps and original absence evidence, including the actor namespace/object IDs and managed DNS ID. Do not print secret files.

From the reviewed `relay/` directory, with pinned Node/tooling:

```sh
node test/cloudflare-crypto-driver.ts initialize /private/tmp/collie-crypto-bench-20261016-private
cf d1 create --name collie-push-crypto-bench-20261016
```

Record the returned new database ID. Apply only the reviewed `migrations/0001_relay.sql` and `0002_attestation.sql` to that ID using `cf d1 query <owned-database-id> --batch @<schema-batch.json>`, preserving whole trigger statements. Retain all returned D1 metadata and require at most 40 writes. Stage a private build directory containing the reviewed `src/`, `test/`, package/lock and installed dependencies. Copy `test/cloudflare-crypto.config.ts` as its `cloudflare.config.ts`, replacing only the zero D1 placeholder with the newly recorded database ID. The account, Worker, actor export, exact domain and native limiter caps stay as reviewed.

From that staged directory:

```sh
cf build
cf deploy --prebuilt --no-provision --secrets-file /private/tmp/collie-crypto-bench-20261016-private/secrets.json
```

Inspect the prebuilt `worker.config.json` and bundle before deployment: one exact Worker, one SQLite `BenchmarkCrypto` class, one fixed actor binding, exact new D1/domain, no CPU/Paid setting, no test admission-delay helper, no real credentials, no outbound APNs implementation. `--no-provision` disables unrelated binding resource creation; the explicitly declared class lifecycle and exact Custom Domain are the intended creations. Record exact IDs immediately and use CF API metadata for readiness, with no uncounted HTTP health probes.

Only after independent review and orchestration authorization, run the one matrix:

```sh
node test/cloudflare-crypto-driver.ts run /private/tmp/collie-crypto-bench-20261016-private https://collie-push-crypto-bench-20261016.rbstp.dev
```

Capture durable stdout/stderr and an exit sidecar; wait for that process to exit. `responses.jsonl` records redacted case timestamps, outcomes and D1 metadata; `attempts.json` is the authoritative attempted-HTTP count. Do not rerun on failure. No additional Worker request is needed for cleanup verification.

## Separate CPU collection

A fresh read-only `cf sql datasets --account-tag <verified-account> --include-columns --include-wae=false --include-lex=false` catalogue confirmed `logs.workersLogs` has the columns below. Query only the exact experiment script and captured time window:

```sh
cf sql query "SELECT timestamp, scriptName, invocationType, invocationLabel, cpuTimeMs, wallTimeMs, sampleInterval, error, httpStatus, requestId, rayId, url FROM logs.workersLogs WHERE scriptName = 'collie-push-crypto-bench-20261016' ORDER BY timestamp LIMIT 2000 FORMAT JSON" --scope-account e0d20426f1862227c675befdcc4d1e7e --time-since '<experiment-start-RFC3339>' --time-until '<experiment-end-RFC3339>'
```

Separate `fetch` public Worker invocations from actor invocations (`jsrpc` in the observed logs) using `invocationType` and `invocationLabel`. Record raw sampling intervals, error counts and small-sample maxima/quantiles, compared with the driver's timestamps and cases. Generic actor labels, missing parent ray IDs and clock skew prevented reliable per-case actor attribution in the actual run; report aggregate actor samples only unless a reliable join is established. There is only one deployed actor class/object; capture namespace/object metadata separately. Do not combine actor CPU with the public Worker's 10 ms limit. A new signer is not proof of a fresh isolate, and the small run cannot establish a production p99. Preserve first-use labels rather than claiming arbitrary calls are cold isolates.

Per-request `wallTimeMs` is not billed actor duration. Official metrics describe `durableObjectsInvocationsAdaptiveGroups` and `durableObjectsPeriodicGroups` for namespace request/CPU/duration analytics. If the active `cf` CLI cannot expose those metrics, record that exact unmeasured gate; do not substitute Node wall time or invocation wall time for GB-s, invent field names, extend Worker load, or claim full Free viability. The public Worker must also avoid CPU-limit failures on real control and send paths. Fresh Collie-device enrollment, real APNs latency, account-wide capacity and statistical p99 validation remain separate gates.

## Exact cleanup, CF API only

After all owned load processes have exited, collect metrics and read-only exact synthetic state/counters. Record owned IDs and before snapshots again. Stage `test/cloudflare-crypto-cleanup.config.ts` as the isolated `cloudflare.config.ts`. Its code exports only an inert 404 handler, has no bindings or class, explicitly declares an empty domain list, and declares only `BenchmarkCrypto: exports.durableObject({state:'deleted'})`.

```sh
cf build
cf deploy --prebuilt --no-provision
cf durable-objects namespaces list
```

Observed correction: `cf workers triggers deploy --prebuilt` with `domains: []` reported no targets but did not detach this Custom Domain. Do not treat that output as domain absence. The orchestrator approved the following narrow order adjustment, which removed the exact owned resources successfully.

The tombstone deletes only this Worker's exact new class/namespace and its data; [the official class lifecycle contract](https://developers.cloudflare.com/durable-objects/reference/durable-objects-migrations/) requires absent class/binding and rejects external references. Verify the recorded namespace ID is absent from the returned namespace list before deleting Worker/D1. Read fresh Worker attachment metadata and require that its only domain is the exact newly recorded temporary domain, with no external Worker, Durable Object, queue or outbound references. Native deletion of this exact new Worker removes its own associated domain and managed DNS. Stop and report any unexpected identity/reference or cleanup failure; do not use broad `--delete-with-references` or modify existing resources.

```sh
cf workers delete <exact-new-worker-id> --force
```

Verify the exact temporary domain and managed DNS are absent through CF API reads before proceeding. Stop and report if either remains; deleting any leftover record requires an exact ownership decision. Then confirm deletion of only the exact new D1 database:

```sh
cf d1 delete <exact-new-database-id> --force
```

Finally verify exact domain, managed DNS, Worker, D1 and namespace absence through CF API reads and compare preexisting inventories with their before snapshots. Preserve the redacted resource-ID list, before/create/after metadata, logs, actual attempt count and process exit sidecars. Delete no unrelated resources.

Sources: [DO limits](https://developers.cloudflare.com/durable-objects/platform/limits/), [Free pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/), [DO metrics](https://developers.cloudflare.com/durable-objects/observability/metrics-and-analytics/), [cf builds/deployments](https://developers.cloudflare.com/cf/projects/).
