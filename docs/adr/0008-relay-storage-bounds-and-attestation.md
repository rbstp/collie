# ADR-0008: The relay checks App Attest flags strictly and bounds its storage in guarded batches

- Status: accepted
- Date: 2026-11-20 (recorded from the comments of `relay/`)

## Context

The push relay (docs/relay.md) accepts App Attest enrollments and assertions, stores routes and pairing tombstones in D1 and enforces quotas with triggers. docs/relay.md's "Atomic quotas and storage ownership" and "Retention and cleanup" sections give the quotas and the retention arithmetic; this records the code-level decisions.

## Decision

- The trust anchors are Apple's public roots, from https://www.apple.com/certificateauthority/.
- Authenticator flags: an attestation must carry AT (attested credential data); an assertion may keep AT without including credential data, which Apple's assertions do, so AT is not refused there. The reserved flags (`0x3e`) must be clear.
- A route is replaced by a delete and an insert in the same guarded D1 transaction, so the quota triggers check the replacement too.
- The hourly cleanup deletes eligible records oldest retention first, in bounded batches: 64 routes, 4,096 pairings (64 installations' worth of the 64 pairing tombstones each may hold), 64 usage counters and 64 installations. Each control or enrollment sets at most one installation's retention deadline and creates at most one route, so a window across one UTC midnight can make two days eligible at once; sustained daily arrivals stay below hourly capacity, so a later day never adds more delay than that burst. `relay/test/attestation.test.ts` checks the worst case: one hour to the first run, plus the route, pairing and installation stages each draining a two-day burst.
- Test fixtures: Apple's sample attestation uses raw bytes as `clientDataHash` and an OS-executable validation category; its historical sample has another publisher and a raw client challenge. The Cloudflare crypto benchmark reserves 64 writes per mutation, the bound of its one-phone, one-route fixture, indexes and triggers included.

## Consequences

- A new table or trigger touched by a route change joins the same batch, or the quota no longer covers it.
- A larger cap on daily controls or enrollments needs the cleanup bounds and the worst-case test redone.

## Code

- `relay/src/attestation.ts`, `relay/src/apple-roots.ts`, `relay/src/control.ts`
- `relay/test/attestation.test.ts`, `relay/test/cloudflare-crypto-driver.ts`, `relay/test/compute-fixture.ts`
