# Isolated Free crypto experiment plan

Status: the single reviewed 42-attempt cycle ran on 2026-10-16 and its exact temporary resources were removed. Its results in `docs/reviews/relay-free-tier.md` do not replace the failed direct-Worker result there. Production has since moved to Workers Paid with this `RelayCrypto` design (`docs/relay.md`). This is the reproducible plan with its corrected cleanup order, not authorization to rerun: a new experiment needs a separate explicit decision. The experiment involves no billing, Paid plan, production hostname, real APNs secret or fresh device enrollment.

## Source and limits

Production `src/worker.ts` exports only the ordinary Worker and `RelayCrypto`, reached by the fixed object name `relay-crypto`, with the full attestation/assertion verifier, Apple roots and D1 schema/transactions unchanged. Policy comes from the actor's trusted bindings and attestation time from its own clock.

The isolated entrypoint `test/cloudflare-crypto-benchmark.ts` exports a guarded Worker and `BenchmarkCrypto`, a test subclass adding a fixed historical Apple-fixture method and a new-signer method; historical context never enters production RPCs. The Worker returns 404 for an absent, wrong or unconfigured bearer secret before any database or actor access. It provisions one synthetic verified key, uses fake APNs with no outbound fetch, forwards ordinary challenge/control/send operations through the production implementation and logs no bodies, grants, challenges or secrets. The local-test admission-delay helper `compute-fixture.ts` is not in this cloud bundle.

One explicit initialization creates a new private directory, ephemeral signing keys and a shared fsynced `{attempts:0,limit:50}` journal. Every Worker-origin request, including probes, failed network attempts and any authorized supplemental client, must go through `benchmarkClient` with that journal. Redirects are errors; missing, corrupt or locked state fails closed. The driver never retries, refuses a second matrix run because its durable response log already exists, and gives each HTTP attempt a 15-second deadline. A failure stops the matrix and authorizes no recovery traffic or new directory/counter.

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

Exactly 15 calls can mutate. The one-phone/one-pairing/one-route fixture bounds each to 64 D1 writes including indexes and triggers; with 40 writes reserved for the two initial schemas, the conservative total is 1,000. The driver reserves before each mutation and checks returned cumulative metadata, and the initial remote schema metadata must fit its 40-write reserve before the matrix runs. A network failure consumes its reservation and ends the run. Nothing is refunded; there is no matrix resume or supplemental mutation plan.

## Reviewed resources and commands

Use only the verified personal account `e0d20426f1862227c675befdcc4d1e7e` and zone `203b54799bb609edd64c2f702fe431cb`. The new Worker and D1 database are both named `collie-push-crypto-bench-20261016`, and the Custom Domain is `collie-push-crypto-bench-20261016.rbstp.dev`. Stop on any collision. Do not register an account-wide workers.dev subdomain or overwrite an existing record.

In each Cloudflare shell:

```sh
unset CLOUDFLARE_API_TOKEN CLOUDFLARE_API_KEY CLOUDFLARE_EMAIL
export CLOUDFLARE_ACCOUNT_ID=e0d20426f1862227c675befdcc4d1e7e
cf auth whoami
```

Before any write, record fresh intended-account/Free-plan evidence, such as empty subscriptions or a rejected CPU setting; the zone plan alone is not evidence. Snapshot Workers, D1, Durable Object namespaces, exact-domain attachments and exact-name DNS records, and verify every proposed name is absent. Record creation outputs, exact IDs, creation timestamps and original absence evidence, including the actor namespace/object IDs and managed DNS ID. Do not print secret files.

From the reviewed `relay/` directory, with pinned Node and tooling:

```sh
node test/cloudflare-crypto-driver.ts initialize /private/tmp/collie-crypto-bench-20261016-private
cf d1 create --name collie-push-crypto-bench-20261016
```

Record the new database ID. Apply only the reviewed `migrations/0001_relay.sql` and `0002_attestation.sql` to it with `cf d1 query <owned-database-id> --batch @<schema-batch.json>`, keeping trigger statements whole; retain all returned D1 metadata and require at most 40 writes. Stage a private build directory with the reviewed `src/`, `test/`, package/lock files and installed dependencies, and copy `test/cloudflare-crypto.config.ts` in as its `cloudflare.config.ts`, replacing only the zero D1 placeholder with the new database ID. Everything else in that configuration stays as reviewed.

From that staged directory:

```sh
cf build
cf deploy --prebuilt --no-provision --secrets-file /private/tmp/collie-crypto-bench-20261016-private/secrets.json
```

Before deploying, inspect the prebuilt `worker.config.json` and bundle: one exact Worker, one SQLite `BenchmarkCrypto` class, one fixed actor binding, the exact new D1 database and domain, no CPU/Paid setting, no test admission-delay helper, no real credentials and no outbound APNs implementation. `--no-provision` disables unrelated binding resource creation; the declared class lifecycle and exact Custom Domain are the intended creations. Record exact IDs immediately and check readiness through Cloudflare API metadata, never uncounted HTTP health probes.

Only after independent review and orchestration authorization, run the one matrix:

```sh
node test/cloudflare-crypto-driver.ts run /private/tmp/collie-crypto-bench-20261016-private https://collie-push-crypto-bench-20261016.rbstp.dev
```

Capture durable stdout/stderr and an exit sidecar, and wait for the process to exit. `responses.jsonl` records redacted case timestamps, outcomes and D1 metadata; `attempts.json` is the authoritative attempted-HTTP count. Do not rerun on failure. Cleanup verification needs no further Worker request.

## Separate CPU collection

A fresh read-only `cf sql datasets --account-tag <verified-account> --include-columns --include-wae=false --include-lex=false` catalogue confirmed that `logs.workersLogs` has the columns below. Query only the exact experiment script and captured time window:

```sh
cf sql query "SELECT timestamp, scriptName, invocationType, invocationLabel, cpuTimeMs, wallTimeMs, sampleInterval, error, httpStatus, requestId, rayId, url FROM logs.workersLogs WHERE scriptName = 'collie-push-crypto-bench-20261016' ORDER BY timestamp LIMIT 2000 FORMAT JSON" --scope-account e0d20426f1862227c675befdcc4d1e7e --time-since '<experiment-start-RFC3339>' --time-until '<experiment-end-RFC3339>'
```

Separate public `fetch` invocations from actor invocations (`jsrpc` in the observed logs) by `invocationType` and `invocationLabel`. Record raw sampling intervals, error counts and small-sample maxima or quantiles against the driver's timestamps and cases. In the actual run, generic actor labels, missing parent ray IDs and clock skew prevented per-case actor attribution, so report aggregate actor samples unless a reliable join exists. Capture the single actor's namespace/object metadata separately. Do not hold actor CPU against the public Worker's 10 ms limit. A new signer does not prove a fresh isolate and a small run cannot establish a production p99, so keep first-use labels rather than calling arbitrary calls cold.

Per-request `wallTimeMs` is not billed actor duration. Cloudflare documents `durableObjectsInvocationsAdaptiveGroups` and `durableObjectsPeriodicGroups` for namespace request, CPU and duration analytics. If the active `cf` CLI cannot expose them, record that gate as unmeasured: do not substitute Node or invocation wall time for GB-s, invent field names, extend Worker load or claim full Free viability. The public Worker must also avoid CPU-limit failures on real control and send paths. Fresh Collie-device enrollment, real APNs latency, account-wide capacity and statistical p99 validation remain separate gates.

## Exact cleanup, CF API only

After all owned load processes exit, collect metrics and read-only exact synthetic state and counters, and record owned IDs and before snapshots again. Stage `test/cloudflare-crypto-cleanup.config.ts` as the isolated `cloudflare.config.ts`: an inert 404 handler with no bindings, an explicit empty domain list and only the tombstone `BenchmarkCrypto: exports.durableObject({state:'deleted'})`.

```sh
cf build
cf deploy --prebuilt --no-provision
cf durable-objects namespaces list
```

Observed correction: `cf workers triggers deploy --prebuilt` with `domains: []` reported no targets but did not detach this Custom Domain, so that output does not prove the domain is gone. The orchestrator approved the narrow order below, which removed the exact owned resources.

The tombstone deletes only this Worker's new class, namespace and data; [Cloudflare's class lifecycle contract](https://developers.cloudflare.com/durable-objects/reference/durable-objects-migrations/) requires an absent class and binding and rejects external references. Verify the recorded namespace ID is missing from the returned list before deleting the Worker or D1. Read fresh Worker attachment metadata and require that its only domain is the exact recorded temporary domain, with no external Worker, Durable Object, queue or outbound references; natively deleting this exact Worker then removes its own domain and managed DNS. Stop and report any unexpected identity, reference or cleanup failure. Do not use the broad `--delete-with-references` or modify existing resources.

```sh
cf workers delete <exact-new-worker-id> --force
```

Verify through Cloudflare API reads that the exact temporary domain and managed DNS record are gone; if either remains, stop and report, since deleting it needs an exact ownership decision. Then delete only the exact new D1 database:

```sh
cf d1 delete <exact-new-database-id> --force
```

Finally, verify through Cloudflare API reads that the exact domain, managed DNS, Worker, D1 database and namespace are absent, and compare preexisting inventories with their before snapshots. Preserve the redacted resource-ID list, before/create/after metadata, logs, actual attempt count and process exit sidecars. Delete no unrelated resources.

Sources: [DO limits](https://developers.cloudflare.com/durable-objects/platform/limits/), [Free pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/), [DO metrics](https://developers.cloudflare.com/durable-objects/observability/metrics-and-analytics/), [cf builds/deployments](https://developers.cloudflare.com/cf/projects/).
