# Relay Free-plan validation, 2026-10-16

The production rollout gate failed: full Apple App Attest verification exceeded the Workers Free CPU limit, with 198 confirmed CPU-limit responses across 400 valid/invalid full-verification attempts. No certificate, receipt/CMS, nonce, environment or replay check was removed, no paid plan or production notification delivery was enabled, and the temporary resources were removed. The publisher account later moved to Workers Paid ([Relay](../relay.md#paid-plan-and-cost-controls)).

## Environment and method

The account's subscriptions API returned none and, more decisively, Cloudflare rejected the explicit CPU setting with error `100328`, since Free does not support CPU limits; the zone's plan was not used as evidence. The benchmark ran under the native Free limit without that setting. Rate-limit bindings deployed and returned success.

All Cloudflare operations used `cf` 1.0.0-beta.17 with its OAuth login, the verified account selected explicitly and inherited token/key/email variables cleared. The benchmark ran reviewed foundation/control source at `5d69123` against an isolated D1 database with both schema files, through a test-only entrypoint whose guard rejected missing, wrong and unconfigured secrets before database access, with random synthetic signing keys, generic encrypted-shape payloads and mocked APNs (no outbound fetch). The valid Apple fixture was checked in its documented historical app/environment/time context, no substitute for a fresh Collie-device enrollment.

An approved temporary single-level Custom Domain served the benchmark, since the account had no workers.dev subdomain; no workers.dev registration, production hostname, existing Worker or billing setting changed. Requests reached the Toronto edge. Invocation CPU logs were on for this synthetic traffic only.

## Observed CPU and outcomes

CPU values are integer milliseconds from Cloudflare invocation logs: nearest-rank p99s of the returned samples, not Node wall times, and approximate because of adaptive sampling intervals of 1 or 10 and invocations missing from the fetched sample. The CPU-limit failures alone show that the gate, p99 below 8 ms with zero limit errors, was not met.

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

Of 200 valid-attestation attempts, 145 succeeded and 55 hit the CPU limit; of 200 invalid-nonce attempts, 57 completed rejection and 143 hit the limit, counting every completed group from the interrupted initial run and the repeated matrix. All 40 control operations succeeded. One send including invalidation used 81 ms CPU, a single sample. Free CPU rollover can let an invocation exceed the nominal limit without an error, but that does not make these results meet the launch target.

A signer-cache miss does not prove a fresh isolate. The send benchmark added synthetic grant issuance and an extra route read, so its CPU and read counts are conservative. Real APNs latency, fresh physical-device enrollment and remote concurrent last-slot quota exhaustion were not measured; local D1 regression tests cover the last.

## D1 lifecycle observations

Actual D1 response metadata, including the schema's indexes and triggers, rather than a four-write assumption:

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

The initial combined fixture seed cost 6 reads/17 writes. The unique key index correctly rejected a duplicate synthetic key, so the control fixture used a distinct key. Final exact counters showed 131 charged attempts, 40 controls, two enrollments and zero remaining installations. D1 analytics returned 604 sampled backend events totaling 2,050 reads/628 writes at collection time, a sampled aggregate, not a billing reconciliation, excluding later read-only verification queries.

Keep the conservative ceiling of 20,000 sends a day and the 20,000-write reserve. At four writes per send, 30 phones at 624 sends use at most 74,880 send writes; counting all 12 daily controls per phone as 14-write mutations adds 5,040, before enrollment and cleanup. The measured examples fit that reserve, but maximum-cardinality cleanup, rejected-operation accounting, account-wide traffic and actual enrollment still need measuring, and the CPU failure blocks capacity approval regardless. Do not raise capacity on the two-write success path.

## Request accounting failure and prevention

The implementing agent exceeded the approved 1,000-attempt cap: it reran a matrix without counting its interrupted initial run, whose partial counter was then overwritten, so the count cannot be reconstructed to one attempt. The partial run attempted 634 to 635 calls, the repeated matrix 805, the control supplement 84 and standalone probes two: **1,525 to 1,526 controlled HTTP attempts**, including the failed network attempt; the conservative recorded total is 1,526. Unsolicited unauthenticated scans are separate account traffic and do not reduce it. All load processes had ended when the error was found; no further Worker probes were made.

The benchmark client now reserves and fsyncs each attempt in one shared journal before HTTP, so network failures and error responses count. Only explicit first-run initialization creates the journal. The client rejects redirects, which would make uncounted requests, and fails closed at 1,000 attempts or on missing, corrupt, empty or locked state; local mocked tests prove this. The old temporary drivers were patched to use the shared, exhausted counter and were not run again.

## Cleanup and remaining work

The temporary resource IDs were recorded before deletion:

| Resource | ID |
|---|---|
| Worker | `93e19d67ba184bc0ace4ed43350b71bd` |
| Custom Domain | `170f9f8736fc289ec82d441210be74e9d288562a` |
| Managed DNS record | `829c376abeb7efd9a6a3759c7965de44` |
| D1 database | `b6689c2a-90e7-4107-96c4-1f877d933d65` |

Automatic approval review rejected the combined cleanup until fresh creation timestamps, exact IDs, original DNS absence and zero installation/route/key rows proved these were this session's synthetic-only resources. Cloudflare API checks then found no temporary DNS record, domain or Worker, D1 returned not-found, and the preexisting Worker inventory matched its before snapshot exactly. Only synthetic aggregate logs remain, subject to Cloudflare retention.

Before production, revise and remeasure the enrollment/verifier design without weakening verification. One candidate, then unimplemented, is a single internal SQLite Durable Object for bounded verification, keeping D1 authorization and the public Worker checks: [Durable Object limits](https://developers.cloudflare.com/durable-objects/platform/limits/) give a 30-second default CPU budget, and [Free pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/) lists 100,000 requests and 13,000 GB-s a day, failing at exhausted quotas. It is a candidate, not a demonstrated Free-tier fit; no Durable Object was created or deployed. A paid deployment requires a separate explicit decision. Do not claim full #350 acceptance, enable public enrollment or connect production APNs on this foundation. Revocation/control changes after `5d69123` were not benchmarked.

The development tooling audit also found three high findings in local tooling, not the deployed Worker's dependency graph: Miniflare via sharp (`GHSA-rgj7-g3m4-5g8c`, `GHSA-wq5f-xc86-pv6w`) and undici (the 7.29.1 fixes, including `GHSA-rfgv-xxqx-mfg5` and `GHSA-w293-vg96-wgc3`). The latest stable Miniflare was 4.20260730.0; the newer release was beta. Do not apply npm's proposed downgrade or unreviewed transitive overrides. Reviewed scoped overrides in `relay/package.json` pin Miniflare's sharp to 0.35.5 and undici to 7.30.1.

Sources: [Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/), [CPU limits and rollover](https://developers.cloudflare.com/workers/platform/limits/), [D1 row accounting](https://developers.cloudflare.com/d1/platform/pricing/), [cf account selection and build/deploy](https://developers.cloudflare.com/cf/projects/), [cf configuration](https://developers.cloudflare.com/cf/projects/cloudflare-config/), [Worker CPU analytics](https://developers.cloudflare.com/workers/observability/metrics-and-analytics/).

## Follow-up: internal SQLite crypto actor, 2026-10-16

A separately approved follow-up ran the frozen prototype at `d881372e2c55a0d76283c33688c528edde30746f` on the same unchanged Free account. Its single guarded matrix produced the expected results in **42 controlled HTTP attempts**, below its durable 50-attempt ceiling. No paid plan, production endpoint, publisher secret or real APNs delivery was enabled, and all temporary resources were removed.

The public Worker kept its admission/body/challenge checks and transactional D1 replay/epoch/quota policy. One internal SQLite `BenchmarkCrypto` actor inherited the production `RelayCrypto` verification/signing methods, with the verifier, pinned roots and schema unchanged; only this guarded test subclass added Apple's fixed historical app/time/category context and a new-signer method, which production RPCs lack. Six genuine historical attestations completed, six invalid-nonce attempts were rejected, and four signed route controls, the rejection of a re-signed consumed challenge, eight mocked sends and a hostile plaintext-field rejection returned their expected results.

### Observed CPU and coverage

Returned invocation logs held all 42 controlled public fetches at sampling interval 1, with no error fields or CPU-limit responses. The largest observed CPU time, **6 ms**, is below the Worker's 10 ms Free limit. These are small-sample maxima, not production p99 estimates:

| Public Worker case | Observations | Maximum CPU |
|---|---:|---:|
| Missing/wrong bearer guard | 2 | 0 ms |
| Synthetic verified-key seed | 1 | 6 ms |
| New/cached actor provider-token proxy | 8 | 1 ms |
| Full historical verification proxy | 12 | 2 ms |
| Challenge | 4 | 3 ms |
| Signed control and replay rejection | 5 | 5 ms |
| Mocked send and hostile-envelope rejection | 9 | 4 ms |
| Database cleanup | 1 | 1 ms |

Returned actor logs held **21 aggregate `jsrpc` observations** with maximum CPU **101 ms**, against the documented 30-second default SQLite actor budget; the matrix makes 25 actor calls, so four went unobserved. All returned actor sampling intervals were 1 and all error fields empty. Generic actor labels, missing parent ray IDs and skewed actor timestamps prevent reliable per-case attribution, so no per-case actor quantiles, complete actor coverage or billed GB-s are claimed. Invocation wall time is not billed actor duration, and the active CLI returned no namespace duration analytics.

A later read-only log collection also returned five unattributed unauthenticated GETs after the matrix ended, one for a favicon; all returned 404 before actor or database work, used 0 to 1 ms CPU, and came from neither the driver nor the implementing agent's tools. The observed public total is therefore **47 fetch rows: 42 controlled POSTs and five unattributed GETs**, and the shared journal remains the exact controlled-attempt count. No controlled Worker request followed the cycle, cleanup verification included.

### Actual D1 accounting

The initial 22 schema statements used 38 reads and 36 writes, within the 40-write setup reserve. Matrix response metadata, including indexes and triggers:

| Operation | Samples | Reads per operation | Writes per operation |
|---|---:|---:|---:|
| Synthetic verified-installation seed | 1 | 1 | 9 |
| Challenge | 4 | 2 | 0 |
| First pairing/app-route registration | 1 | 11 | 14 |
| Route rotation | 3 | 16 | 12 |
| Consumed-challenge replay rejection | 1 | 2 | 0 |
| Accepted mocked send | 8 | 10 | 2 |
| Hostile-envelope rejection | 1 | 0 | 0 |
| Expiry cleanup | 1 | 21 | 6 |

The matrix used 171 reads and 81 writes; setup plus matrix used **209 reads and 117 writes**, excluding later read-only inspection. The driver conservatively reserved 40 setup writes plus 64 for each of 15 potentially mutating calls, exactly its 1,000-write ceiling. Its shared journal persisted each attempt before network access, a second matrix run could not recreate its response log, and no retry or supplemental matrix ran. Final read-only state showed zero installation, route, pairing, attestation and control-commit rows; singleton counters recorded eight sends, four controls and one synthetic seed enrollment.

### Cleanup and evidence

| Temporary resource | Recorded ID |
|---|---|
| Worker | `47310591a2574dbe9ba94d85374fb1ef` |
| Custom Domain | `956f4325d4818a47918afef71dffbf6c4a877601` |
| Managed DNS | `a5923390f78ad1112f1b56d32d8cdd48` |
| D1 | `72ac2a4d-9cea-4442-b310-2b5c2f7f5607` |
| SQLite namespace | `4da8514848b840b1841c3554c16bd046` |

The initial cleanup command, with an empty domain list, reported no targets and did not detach the domain. With the orchestrator's approval, cleanup instead deployed the reviewed inert, binding-free bundle with an explicit deleted-class tombstone, verified the exact namespace was gone and the Worker had only its own temporary domain and no outside references, then natively deleted that Worker, which removed its domain and managed DNS, and finally the empty D1 database, using the CLI's noninteractive confirmation and no broad reference-override flag. `relay/test/cloudflare-crypto-plan.md` records this tested order.

Final Cloudflare API inventories of Workers, D1, namespaces and the exact DNS name matched their original snapshots. The owned load/CLI processes ended, and the synthetic private keys and guard secret were deleted after byte-identical redacted journal/response copies were preserved.

The committed `docs/reviews/relay-free-crypto-evidence.json` records 59 raw evidence artifact hashes; its SHA-256 is `b152eb3240a01c4dc91193ceb4d5629bb25768628250d3683de7be84535d2bd9`. Raw redacted responses, invocation logs, creation/absence/cleanup metadata and process exit evidence remain under `/private/tmp/collie-push-crypto-bench-20261016/`. The OAuth profile was not altered.

The cycle supports continuing the Free prototype investigation; it does not complete #350 or authorize rollout. Keep the account Free. Fresh physical-device/TestFlight App Attest enrollment, complete actor duration/capacity accounting, account-wide quotas, real APNs behavior and statistically meaningful cold/warm CPU validation remain open gates. The earlier direct-Worker failure stands, and this small run does not prove production reliability.
