import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import * as asn from 'asn1js';
import { base64url, bytes } from '../src/crypto.ts';
import { concat, sha256, verifyAssertion, verifyAttestation } from '../src/attestation.ts';
import { CborReader, cbor, map } from '../src/cbor.ts';
import { fixture } from './helpers.ts';
import { createWorker } from '../src/index.ts';
import { canonical, cleanup } from '../src/control.ts';
import type { Operation } from '../src/control.ts';

export function encode(value: unknown): Uint8Array<ArrayBuffer> {
  const header = (kind: number, n: number) => n < 24 ? new Uint8Array([kind * 32 + n])
    : n < 256 ? new Uint8Array([kind * 32 + 24, n]) : new Uint8Array([kind * 32 + 25, n >> 8, n & 255]);
  if (typeof value === 'number') return value < 0 ? header(1, -1 - value) : header(0, value);
  if (typeof value === 'string') { const data = bytes(value); return concat(header(3, data.length), data); }
  if (value instanceof Uint8Array) return concat(header(2, value.length), value);
  if (Array.isArray(value)) return concat(header(4, value.length), ...value.map(encode));
  const entries = value instanceof Map ? [...value] : Object.entries(value as object);
  return concat(header(5, entries.length), ...entries.flatMap(([k, v]) => [encode(k), encode(v)]));
}
const policy = { appId: 'TESTTEAM01.dev.rbstp.collie', environment: 'development' as const, bundleVersions: ['1'] };
async function keyPair() {
  return crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
}
async function assertion(key: CryptoKey, challenge: string, counter: number, extensions?: unknown, app = policy.appId) {
  const count = new Uint8Array(4); new DataView(count.buffer).setUint32(0, counter);
  const authData = concat(await sha256(bytes(app)), new Uint8Array([extensions ? 128 : 0]), count,
    ...(extensions ? [encode(extensions)] : []));
  const raw = new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, key, await sha256(concat(authData, await sha256(bytes(challenge))))));
  const integer = (value: Uint8Array) => {
    let start = 0; while (start < value.length - 1 && value[start] === 0) start++;
    const data = value.slice(start); return new asn.Integer({ valueHex: data[0] & 128 ? concat(new Uint8Array([0]), data).buffer : data.buffer });
  };
  const signature = new Uint8Array(new asn.Sequence({ value: [integer(raw.slice(0, 32)), integer(raw.slice(32))] }).toBER(false));
  return base64url(encode({ signature, authenticatorData: authData }));
}
async function controlFixture() {
  const result = await fixture();
  const schema = await readFile(new URL('../migrations/0002_attestation.sql', import.meta.url), 'utf8');
  let current = '';
  for (const statement of schema.split(/;\s*(?:\n|$)/)) {
    if (!statement.trim()) continue;
    current += statement + ';\n';
    if (current.includes('CREATE TRIGGER') && !/\nEND;\s*$/.test(current)) continue;
    await result.db.prepare(current).run(); current = '';
  }
  const keys = await keyPair();
  const publicKey = base64url(new Uint8Array(await crypto.subtle.exportKey('spki', keys.publicKey)));
  const keyId = base64url(await sha256(new Uint8Array(await crypto.subtle.exportKey('raw', keys.publicKey))));
  await result.db.prepare(`INSERT INTO attestations (installation_id,key_id,public_key,receipt,environment,last_nonce,retain_until)
    VALUES ('phone', ?, ?, 'test-receipt', 'development', 'initial', ?)`)
    .bind(keyId, publicKey, Math.floor(Date.now() / 1000) + 37 * 86400).run();
  const env = { ...result.env, ATTEST_BUNDLE_VERSIONS: '["1"]',
    CONTROL_LIMITER: { async limit() { return { success: true }; } }, ENROLL_LIMITER: { async limit() { return { success: true }; } } };
  const worker = createWorker();
  const post = (path: string, body: unknown) => worker.fetch(new Request(`https://relay.example/v1/${path}`, {
    method: 'POST', headers: { 'content-type': 'application/json', 'cf-connecting-ip': '192.0.2.1' }, body: JSON.stringify(body)
  }), env);
  const common = { v: 1 as const, installation_id: 'phone', key_id: keyId, environment: 'development' as const };
  let counter = 0;
  async function execute(op: Operation) {
    const response = await post('challenge', op); assert.equal(response.status, 200);
    const challenge = (await response.json()).challenge;
    return post('control', { operation: op, challenge, assertion: await assertion(keys.privateKey, challenge, ++counter) });
  }
  return { ...result, keys, keyId, env, worker, post, common, execute };
}
test('assertions verify actual signatures and reject stale counters, wrong app, wrong challenge and wrong keys', async () => {
  const keys = await keyPair(), other = await keyPair();
  const publicKey = base64url(new Uint8Array(await crypto.subtle.exportKey('spki', keys.publicKey)));
  const signed = await assertion(keys.privateKey, 'one-time', 7);
  assert.equal(await verifyAssertion(signed, publicKey, 6, await sha256(bytes('one-time')), policy), 7);
  await assert.rejects(verifyAssertion(signed, publicKey, 7, await sha256(bytes('one-time')), policy));
  await assert.rejects(verifyAssertion(signed, publicKey, 6, await sha256(bytes('other')), policy));
  await assert.rejects(verifyAssertion(await assertion(other.privateKey, 'one-time', 8), publicKey, 6, await sha256(bytes('one-time')), policy));
  await assert.rejects(verifyAssertion(await assertion(keys.privateKey, 'one-time', 8, undefined, 'OTHERTEAM1.dev.rbstp.collie'), publicKey, 6, await sha256(bytes('one-time')), policy));
});
test('fixed assertions require the Apple nonce and allow the retained AT flag without credential bytes', async () => {
  const fixture = JSON.parse(await readFile(new URL('./fixtures/assertions.json', import.meta.url), 'utf8'));
  const hash = await sha256(bytes(fixture.challenge));
  for (const proof of fixture.proofs) {
    const verified = verifyAssertion(proof.assertion, fixture.publicKey, 0, hash, { ...policy, appId: fixture.appId });
    if (proof.accepted) assert.equal(await verified, 1, proof.name);
    else await assert.rejects(verified, undefined, proof.name);
  }
});
test('documented authenticator extensions are parsed and enforced without assuming a fixed length', async () => {
  const keys = await keyPair();
  const publicKey = base64url(new Uint8Array(await crypto.subtle.exportKey('spki', keys.publicKey)));
  const values = { apple_validation_category_01: new Uint8Array([3, 0, 0, 0]), apple_bundle_version_01: '1' };
  const hash = await sha256(bytes('challenge'));
  assert.equal(await verifyAssertion(await assertion(keys.privateKey, 'challenge', 1, values), publicKey, 0, hash, policy), 1);
  await assert.rejects(verifyAssertion(await assertion(keys.privateKey, 'challenge', 1, { ...values, apple_bundle_version_01: '2' }), publicKey, 0, hash, policy));
  await assert.rejects(verifyAssertion(await assertion(keys.privateKey, 'challenge', 1, values), publicKey, 0, hash, { ...policy, environment: 'production' }));
});
test('bounded CBOR rejects duplicate keys, trailing data, indefinite lengths and excessive nesting', () => {
  for (const value of [new Uint8Array([0xa2, 1, 2, 1, 3]), new Uint8Array([1, 1]), new Uint8Array([0x9f, 0xff]), new Uint8Array(10).fill(0x81)]) {
    assert.throws(() => cbor(value));
  }
  assert.throws(() => new CborReader(new Uint8Array(16385)));
});
test('Apple published attestation remains untrusted with wrong app, environment, nonce or expired certificates', async () => {
  const sample = base64url(new Uint8Array(Buffer.from(await readFile(new URL('./fixtures/apple-attestation.b64', import.meta.url), 'utf8'), 'base64')));
  for (const variant of [policy, { ...policy, environment: 'production' as const }, { ...policy, appId: '1234567890.com.example.myapp' }]) {
    await assert.rejects(verifyAttestation(sample.trim(), 'zgSY9YSD-7TaDXssY6WlOPVS1K3Lmk-pFhlcSWE-ZV0', await sha256(bytes('unrelated')), variant, Math.floor(Date.now() / 1000)));
  }
});
test('canonical operations bind exact destination and reject unknown fields or changed challenge operations', async () => {
  const f = await controlFixture();
  try {
    const op: Operation = { ...f.common, op: 'route', pairing_id: 'second', route_id: 'second_app', expected_generation: 0, kind: 'app', destination: 'b'.repeat(64) };
    assert.equal(canonical(op), canonical(Object.fromEntries(Object.entries(op).reverse()) as Operation));
    assert.equal((await f.post('challenge', { ...op, topic: 'foreign' })).status, 400);
    const challenge = (await (await f.post('challenge', op)).json()).challenge;
    const result = await f.post('control', { operation: { ...op, destination: 'c'.repeat(64) }, challenge, assertion: await assertion(f.keys.privateKey, challenge, 1) });
    assert.equal(result.status, 401);
    assert.equal((await f.db.prepare('SELECT counter FROM attestations').first()).counter, 0);
  } finally { await f.runtime.dispose(); }
});
test('atomic counter guard permits one concurrent control and rejects re-signing a burned challenge', async () => {
  const f = await controlFixture();
  try {
    const op: Operation = { ...f.common, op: 'route', pairing_id: 'second', route_id: 'second_app', expected_generation: 0, kind: 'app', destination: 'b'.repeat(64) };
    const challenge = (await (await f.post('challenge', op)).json()).challenge;
    const body = { operation: op, challenge, assertion: await assertion(f.keys.privateKey, challenge, 1) };
    const results = await Promise.all(Array.from({ length: 10 }, () => f.post('control', body)));
    assert.equal(results.filter(r => r.status === 200).length, 1);
    assert.equal((await f.db.prepare('SELECT COUNT(*) n FROM routes WHERE id = ?').bind('second_app').first()).n, 1);
    assert.equal((await f.post('control', { ...body, assertion: await assertion(f.keys.privateKey, challenge, 2) })).status, 401);
    assert.equal((await f.db.prepare('SELECT counter FROM attestations').first()).counter, 1);
  } finally { await f.runtime.dispose(); }
});
test('route limits roll back counters and independent rotation and revocation retain other machines', async () => {
  const f = await controlFixture();
  try {
    for (let i = 0; i < 7; i++) assert.equal((await f.execute({ ...f.common, op: 'route', pairing_id: `pair_${i}`, route_id: `app_${i}`, expected_generation: 0, kind: 'app', destination: 'b'.repeat(64) })).status, 200);
    const failed = await f.execute({ ...f.common, op: 'route', pairing_id: 'ninth', route_id: 'ninth_app', expected_generation: 0, kind: 'app', destination: 'c'.repeat(64) });
    assert.equal(failed.status, 429);
    assert.equal((await f.db.prepare('SELECT counter FROM attestations').first()).counter, 7);
    assert.equal((await f.db.prepare("SELECT COUNT(*) n FROM pairings WHERE id = 'ninth'").first()).n, 0);
    const rotated = await f.execute({ ...f.common, op: 'route', pairing_id: 'pair_0', route_id: 'app_0', expected_generation: 1, kind: 'app', destination: 'd'.repeat(64) });
    assert.equal(rotated.status, 200); assert.equal((await rotated.json()).generation, 2);
    assert.equal((await f.execute({ ...f.common, op: 'revoke_route', pairing_id: 'pair_0', route_id: 'app_0', expected_generation: 1 })).status, 200);
    assert.equal((await f.db.prepare("SELECT revoked FROM routes WHERE id = 'app_0'").first()).revoked, 0);
    assert.equal((await f.execute({ ...f.common, op: 'revoke_pairing', pairing_id: 'pair_0' })).status, 200);
    assert.equal((await f.db.prepare("SELECT epoch FROM pairings WHERE id = 'pair_0'").first()).epoch, 1);
    assert.equal((await f.db.prepare("SELECT epoch FROM pairings WHERE id = 'pair_1'").first()).epoch, 0);
    assert.equal((await f.execute({ ...f.common, op: 'route', pairing_id: 'pair_0', route_id: 'app_0', expected_generation: 2, kind: 'app', destination: 'e'.repeat(64) })).status, 410);
  } finally { await f.runtime.dispose(); }
});
test('another attested installation cannot adopt, rotate or revoke foreign routes and pairings', async () => {
  const f = await controlFixture();
  try {
    const keys = await keyPair();
    const keyId = base64url(await sha256(new Uint8Array(await crypto.subtle.exportKey('raw', keys.publicKey))));
    const publicKey = base64url(new Uint8Array(await crypto.subtle.exportKey('spki', keys.publicKey)));
    await f.db.batch([
      f.db.prepare("INSERT INTO installations VALUES ('other', 0)"),
      f.db.prepare(`INSERT INTO attestations (installation_id,key_id,public_key,receipt,environment,last_nonce,retain_until)
        VALUES ('other', ?, ?, 'test-receipt', 'development', 'initial', ?)`)
        .bind(keyId, publicKey, Math.floor(Date.now() / 1000) + 37 * 86400),
      f.db.prepare("INSERT INTO installation_usage VALUES ('other', 0, 0, 0)")
    ]);
    const common = { ...f.common, installation_id: 'other', key_id: keyId };
    let counter = 0;
    const execute = async (operation: Operation) => {
      const response = await f.post('challenge', operation); assert.equal(response.status, 200);
      const challenge = (await response.json()).challenge;
      return f.post('control', { operation, challenge, assertion: await assertion(keys.privateKey, challenge, ++counter) });
    };
    const owned: Operation = { ...common, op: 'route', pairing_id: 'other_machine', route_id: 'other_app',
      expected_generation: 0, kind: 'app', destination: 'b'.repeat(64) };
    assert.equal((await execute(owned)).status, 200);
    const originalRoute = await f.db.prepare("SELECT * FROM routes WHERE id = 'route'").first();
    const originalPairing = await f.db.prepare("SELECT * FROM pairings WHERE id = 'machine'").first();
    for (const operation of [
      { ...owned, pairing_id: 'machine', route_id: 'adopted' },
      { ...owned, route_id: 'route', expected_generation: 1 }
    ]) assert.equal((await execute(operation)).status, 410);
    assert.equal((await f.db.prepare("SELECT COUNT(*) n FROM routes WHERE id = 'adopted'").first())!.n, 0);
    assert.equal((await f.db.prepare("SELECT counter FROM attestations WHERE installation_id = 'other'").first())!.counter, 1);
    assert.equal((await execute({ ...common, op: 'revoke_route', pairing_id: 'machine', route_id: 'route', expected_generation: 1 })).status, 200);
    assert.equal((await execute({ ...common, op: 'revoke_pairing', pairing_id: 'machine' })).status, 200);
    assert.deepEqual(await f.db.prepare("SELECT * FROM routes WHERE id = 'route'").first(), originalRoute);
    assert.deepEqual(await f.db.prepare("SELECT * FROM pairings WHERE id = 'machine'").first(), originalPairing);
    const victim: Operation = { ...f.common, op: 'revoke_pairing', pairing_id: 'machine' };
    const challenge = (await (await f.post('challenge', victim)).json()).challenge;
    assert.equal((await f.post('control', { operation: victim, challenge, assertion: await assertion(keys.privateKey, challenge, ++counter) })).status, 401);
    assert.equal((await f.db.prepare("SELECT counter FROM attestations WHERE installation_id = 'phone'").first())!.counter, 0);
    assert.equal((await f.db.prepare("SELECT revoked FROM pairings WHERE id = 'machine'").first())!.revoked, 0);
    assert.equal((await execute({ ...owned, expected_generation: 1, destination: 'c'.repeat(64) })).status, 200);
    assert.equal((await f.db.prepare("SELECT generation FROM routes WHERE id = 'other_app'").first())!.generation, 2);
  } finally { await f.runtime.dispose(); }
});
test('eight active activities are bounded and rejected registration rolls back every state change', async () => {
  const f = await controlFixture();
  try {
    for (let i = 0; i < 8; i++) assert.equal((await f.execute({ ...f.common, op: 'route', pairing_id: 'machine', route_id: `activity_${i}`, expected_generation: 0, kind: 'liveactivity', destination: 'b'.repeat(64) })).status, 200);
    assert.equal((await f.execute({ ...f.common, op: 'route', pairing_id: 'machine', route_id: 'activity_9', expected_generation: 0, kind: 'liveactivity', destination: 'c'.repeat(64) })).status, 429);
    assert.equal((await f.db.prepare('SELECT counter FROM attestations').first()).counter, 8);
    assert.equal((await f.execute({ ...f.common, op: 'route', pairing_id: 'machine', route_id: 'activity_0', expected_generation: 1, kind: 'liveactivity', destination: 'd'.repeat(64) })).status, 200);
  } finally { await f.runtime.dispose(); }
});
test('cleanup deletes expired routes, keys, raw destinations and zeroed counters without removing active replacements', async () => {
  const f = await controlFixture();
  try {
    const now = Math.floor(Date.now() / 1000);
    await f.db.prepare('UPDATE routes SET expires_at = ?').bind(now - 8 * 86400).run();
    await f.db.prepare('UPDATE attestations SET retain_until = ?').bind(now - 1).run();
    await cleanup(f.db as unknown as D1Database, now);
    assert.equal((await f.db.prepare('SELECT COUNT(*) n FROM installations').first()).n, 0);
    assert.equal((await f.db.prepare('SELECT COUNT(*) n FROM attestations').first()).n, 0);
    assert.equal((await f.db.prepare('SELECT COUNT(*) n FROM installation_usage').first()).n, 0);
  } finally { await f.runtime.dispose(); }
});
test('registration recovers a lost successful response without rotating or widening its destination grant', async () => {
  const f = await controlFixture();
  try {
    const op: Operation = { ...f.common, op: 'route', pairing_id: 'second', route_id: 'second_app', expected_generation: 0, kind: 'app', destination: 'b'.repeat(64) };
    const first = await f.execute(op); assert.equal(first.status, 200); const original = await first.json();
    const repeated = await f.execute(op); assert.equal(repeated.status, 200); const recovered = await repeated.json();
    assert.equal(recovered.generation, original.generation);
    assert.equal(recovered.grant, original.grant);
    assert.equal((await f.execute({ ...op, destination: 'c'.repeat(64) })).status, 410);
  } finally { await f.runtime.dispose(); }
});
test('an existing attested key recovers enrollment through an actual assertion and cannot re-enroll with another key', async () => {
  const f = await controlFixture();
  try {
    assert.equal((await f.execute({ ...f.common, op: 'enroll' })).status, 200);
    assert.equal((await f.post('challenge', { ...f.common, op: 'enroll', key_id: 'a'.repeat(43) })).status, 401);
    assert.equal((await f.db.prepare('SELECT COUNT(*) n FROM attestations').first()).n, 1);
  } finally { await f.runtime.dispose(); }
});
test('control quota rejection rolls back the replay counter and grant mutation', async () => {
  const f = await controlFixture();
  try {
    await f.db.prepare('UPDATE control_usage SET day = CAST(unixepoch() / 86400 AS INTEGER), controls = 1000').run();
    const response = await f.execute({ ...f.common, op: 'revoke_pairing', pairing_id: 'machine' });
    assert.equal(response.status, 429);
    assert.equal((await f.db.prepare('SELECT counter FROM attestations').first()).counter, 0);
    assert.equal((await f.db.prepare("SELECT epoch FROM pairings WHERE id = 'machine'").first()).epoch, 0);
  } finally { await f.runtime.dispose(); }
});
test('complete Apple fixture verifies pinned certificate chains, authenticator nonce and independent CMS receipt in its published context', async () => {
  const sample = base64url(new Uint8Array(Buffer.from(await readFile(new URL('./fixtures/apple-attestation.b64', import.meta.url), 'utf8'), 'base64')));
  const context = { appId: '1234567890.com.example.myapp', environment: 'production' as const,
    bundleVersions: ['1'], validationCategories: [1] };
  const key = 'zgSY9YSD-7TaDXssY6WlOPVS1K3Lmk-pFhlcSWE-ZV0';
  const time = Date.parse('2026-04-21T18:13:13Z') / 1000;
  // Apple's fixture uses these raw bytes as clientDataHash and an OS-executable validation category.
  const hash = bytes('example_server_challenge');
  const verified = await verifyAttestation(sample, key, hash, context, time);
  assert.equal(verified.publicKey, 'MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEQzJUSs8yPbd0RDyq8zn1bn6VxyT6wsFCWfNl4kRWULK1-yhbz1Sby2BZRBLnaCokJ-6tqftS3-0LGrF-0J-pvQ');
  assert.ok(verified.receipt.length > 1000);
  await assert.rejects(verifyAttestation(sample, key, hash, { ...context, appId: policy.appId }, time));
  await assert.rejects(verifyAttestation(sample, key, hash, { ...context, environment: 'development' }, time));
  await assert.rejects(verifyAttestation(sample, key, hash, { ...context, validationCategories: [2, 4] }, time));
  await assert.rejects(verifyAttestation(sample, key, hash, context, time + 301));
  const object = map(cbor(new Uint8Array(Buffer.from(await readFile(new URL('./fixtures/apple-attestation.b64', import.meta.url), 'utf8'), 'base64'))));
  const statement = map(object.get('attStmt')!);
  const receipt = statement.get('receipt') as Uint8Array;
  receipt[receipt.length - 12] ^= 1;
  await assert.rejects(verifyAttestation(base64url(encode(object)), key, hash, context, time));
});

test('shared control and enrollment admission rejects rotating sources before database or crypto work', async () => {
  for (const path of ['challenge', 'control']) for (const enrollment of [false, true]) {
    const shared = enrollment ? 'shared:enrollment' : 'shared:control';
    const counts = new Map<string, number>();
    let databaseReads = 0;
    let cryptoCalls = 0;
    const limiter = { async limit({ key }: { key: string }) {
      const count = (counts.get(key) ?? 0) + 1;
      counts.set(key, count);
      return { success: key !== shared || count <= 1 };
    } };
    const env = {
      APNS_ENVIRONMENT: 'development', SEND_HMAC_KEYS: JSON.stringify({ first: base64url(new Uint8Array(32)) }),
      CONTROL_LIMITER: limiter, ENROLL_LIMITER: limiter,
      DB: { prepare() { databaseReads++; return { bind() { return { async first() { return null; } }; } }; } },
      CRYPTO: { getByName() { cryptoCalls++; throw new Error('unexpected crypto'); } }
    };
    const worker = createWorker();
    const post = (ip: string) => {
      const operation = { v: 1, op: enrollment ? 'enroll' : 'revoke_pairing', ...(enrollment ? {} : { pairing_id: 'machine' }),
        installation_id: ip.replaceAll('.', '_'), key_id: 'A'.repeat(43), environment: 'development' };
      return worker.fetch(new Request(`https://relay.example/v1/${path}`, {
        method: 'POST', headers: { 'content-type': 'application/json', 'cf-connecting-ip': ip },
        body: JSON.stringify(path === 'challenge' ? operation : { operation, challenge: 'unused', attestation: 'AQ' })
      }), env as never);
    };
    assert.equal((await post('192.0.2.1')).status, path === 'challenge' && enrollment ? 200 : 401);
    assert.equal(databaseReads, 1);
    const rejected = await post('192.0.2.2');
    assert.equal(rejected.status, 429);
    assert.deepEqual(await rejected.json(), { v: 1, result: 'rate_limited', retry_after: 60 });
    assert.equal(databaseReads, 1);
    assert.equal(cryptoCalls, 0);
    assert.equal(counts.get(shared), 2);
  }
});
