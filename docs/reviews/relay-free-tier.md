# Relay Free-plan validation, 2026-10-16

The production rollout gate failed. Full Apple App Attest verification exceeded Workers Free CPU limits, and the deployed run produced 198 confirmed CPU-limit responses across 400 valid/invalid full-verification attempts. No certificate, receipt/CMS, nonce, environment or replay checks were removed. No paid plan or production notification delivery was enabled. The temporary resources were removed after measurement.

## Environment and method

The account's subscriptions API returned no subscriptions. More decisively, Cloudflare rejected the explicit CPU setting with error `100328`: CPU limits are unsupported on the Free plan. The setting was omitted and the benchmark used the native Free limit. The zone's plan was not used as evidence for the Workers plan. Rate-limit bindings deployed and returned success on this real Free account.

All Cloudflare operations used `cf` 1.0.0-beta.17 with its OAuth login, inherited token/key/email variables cleared, and the verified account selected explicitly. The benchmark used reviewed foundation/control source at `5d69123`, an isolated D1 database with both schema files, and a separate test-only entrypoint. Its guard rejected missing, wrong and unconfigured secrets before database access. It used random synthetic signing keys, generic encrypted-shape payloads and mocked APNs responses, with no outbound APNs fetch. The valid Apple fixture was checked in its documented historical app/environment/time context. This does not substitute for a fresh Collie-device enrollment.

The account had no workers.dev subdomain. An approved temporary single-level Custom Domain was used instead. No account-wide workers.dev registration, production hostname, existing Worker or billing setting changed. Requests reached the Toronto edge. The benchmark enabled invocation CPU logs only for synthetic traffic; production observability remains disabled.

## Observed CPU and outcomes

Cloudflare invocation logs supplied integer CPU milliseconds. The following are nearest-rank p99 values of returned invocation samples, not Node wall times. Adaptive sampling intervals were 1 or 10; some invocations were absent from the fetched sample, so these are approximate observed quantiles. The CPU-limit failures independently establish that the required p99 below 8 ms with zero limit errors was not met.

| Case | Observed samples | Observed p99 CPU | Maximum CPU |
|---|---:|---:|---:|
| New provider-token signer/cache miss | 199 | 25 ms | 27 ms |
| Warm provider-token cache | 200 | 1 ms | 1 ms |
| Full valid Apple attestation | 192 | 118 ms | 134 ms |
| Full invalid-nonce Apple attestation | 157 | 19 ms | 21 ms |
| Valid assertion verification | 154 | 2 ms | 3 ms |
| Replayed assertion rejection | 159 | 2 ms | 7 ms |
| Mocked complete send | 120 | 11 ms | 16 ms |
| Unknown public-envelope field rejection | 100 | 5 ms | 6 ms |
| Challenge | 37 | 7 ms | 7 ms |
| Assertion-authorized route control | 32 | 16 ms | 16 ms |

The client completed 200 valid-attestation attempts: 145 succeeded and 55 hit the CPU limit. Of 200 invalid-nonce attempts, 57 completed rejection and 143 hit the CPU limit. Both include all completed groups from the interrupted initial run and the repeated matrix. The 40 control operations completed successfully. One invalidation-inclusive send observed 81 ms CPU, a single sample rather than a quantile. Free CPU rollover can permit an invocation above the nominal limit without an error; it does not make these measurements pass the launch target.

Signer-cache misses are not proof of a fresh isolate on every request. The send benchmark included synthetic grant issuance and an extra route read, making its CPU and read count conservative. Real APNs network latency and fresh physical-device enrollment were not measured. Remote concurrent last-slot quota exhaustion was not measured; local D1 regression tests cover it.

## D1 lifecycle observations

These are actual D1 response metadata, including the schema's indexes and triggers, rather than a four-write assumption.

| Operation | Samples | Rows read | Rows written |
|---|---:|---:|---:|
| Synthetic verified-installation storage, three primary inserts | 1 | 1 | 9 |
| Challenge | 40 | 2 | 0 |
| First pairing/app-route registration | 1 | 12 | 14 |
| App-route rotation | 39 | 17 | 12 |
| Accepted send, including the benchmark's extra route lookup | 100 complete matrix samples | 11 | 2 |
| Send plus exact-generation invalidation | 1 | 12 | 3 |
| No-expiry cleanup | 1 | 8 | 0 |
| Cleanup of both synthetic installations and their routes/state | 1 | 41 | 11 |

The initial combined fixture seed cost 6 reads/17 writes. A duplicate synthetic key was correctly rejected by the unique key index; the control fixture was then provisioned with a distinct key. The final exact counters showed 131 charged attempts, 40 controls, two enrollments and zero remaining installations. D1 analytics returned 604 sampled backend events totaling 2,050 reads/628 writes at collection time. That sampled aggregate is not an exact account billing reconciliation and excludes subsequent read-only verification queries.

Keep the conservative ceiling of 20,000 sends/day and the 20,000-write reserve. At four writes per send, 30 phones at 624 sends use at most 74,880 send writes. Even treating all 12 daily controls per phone as 14-write mutations costs another 5,040 writes, before enrollment and cleanup. The measured examples fit that reserve, but maximum-cardinality cleanup, rejected-operation accounting, account-wide traffic and actual enrollment still require measurement. The CPU failure prevents a capacity approval regardless of this arithmetic. Do not raise capacity based on the two-write success path.

## Request accounting failure and prevention

The implementing agent exceeded the approved 1,000-attempt experiment cap by rerunning a matrix without first accounting for its interrupted initial run. The overwritten partial counter prevents an exact one-attempt reconstruction. The partial run attempted 634 to 635 calls, the repeated matrix 805, the control supplement 84, and standalone probes two: **1,525 to 1,526 controlled HTTP attempts**, including the failed network attempt. The conservative recorded total is 1,526. Unsolicited unauthenticated scans are separate account traffic and are not used to reduce that count. All load processes had ended when the accounting error was identified; no further Worker probes were made.

The benchmark client now reserves and fsyncs each attempt in one shared counter before HTTP, including network failures and error responses. Only explicit first-run initialization creates the journal, refusing existing state or a held lock. Normal, resumed and supplemental clients require the same existing file and never recreate a missing or moved journal. Automatic redirects are rejected so they cannot create uncounted requests. It fails closed at 1,000, on missing/corrupt/empty existing state, or when another allocator holds its lock. Local mocked tests prove these properties. The old temporary drivers were patched to use the shared exhausted counter and were not run again.

## Cleanup and remaining work

The temporary resource IDs were recorded before deletion:

| Resource | ID |
|---|---|
| Worker | `93e19d67ba184bc0ace4ed43350b71bd` |
| Custom Domain | `170f9f8736fc289ec82d441210be74e9d288562a` |
| Managed DNS record | `829c376abeb7efd9a6a3759c7965de44` |
| D1 database | `b6689c2a-90e7-4107-96c4-1f877d933d65` |

Automatic approval review initially rejected the combined cleanup because ownership was not established. Fresh creation timestamps, exact IDs, original DNS absence and zero installation/route/key rows established that these were this session's synthetic-only resources. The same exact cleanup then passed review. CF API verification found no temporary DNS/domain or Worker, and D1 returned not-found. The preexisting Worker inventory matched its before snapshot exactly. Only synthetic aggregate logs remain subject to Cloudflare retention.

Before production, revise and remeasure the enrollment/verifier design without weakening verification. One unimplemented candidate is a single internal SQLite Durable Object for bounded verification, retaining D1 authorization and public Worker checks: official [Durable Object limits](https://developers.cloudflare.com/durable-objects/platform/limits/) specify a 30-second default CPU budget, and [Free pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/) lists 100,000 requests and 13,000 GB-s per day with failure at exhausted quotas. That is a design candidate, not a demonstrated Free-tier fit; no Durable Object was created or deployed. A paid deployment requires a separate explicit decision. Do not claim full #350 acceptance, enable public enrollment, or connect production APNs based on this foundation. Later revocation/control changes after `5d69123` were not included in this benchmark.

Development tooling audit also found three high findings: Miniflare via sharp (`GHSA-rgj7-g3m4-5g8c`, `GHSA-wq5f-xc86-pv6w`) and undici (the 7.29.1 fixes, including `GHSA-rfgv-xxqx-mfg5` and `GHSA-w293-vg96-wgc3`). They affect local tooling, not the deployed Worker dependency graph. The latest stable Miniflare remains 4.20260730.0; the newer release is beta. Do not apply npm's proposed downgrade or unreviewed transitive overrides. Wait for an upstream stable fix or separately review a tooling change.

Sources: [Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/), [CPU limits and rollover](https://developers.cloudflare.com/workers/platform/limits/), [D1 row accounting](https://developers.cloudflare.com/d1/platform/pricing/), [cf account selection and build/deploy](https://developers.cloudflare.com/cf/projects/), [cf configuration](https://developers.cloudflare.com/cf/projects/cloudflare-config/), [Worker CPU analytics](https://developers.cloudflare.com/workers/observability/metrics-and-analytics/).
