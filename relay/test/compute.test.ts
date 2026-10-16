import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { after, before, test } from 'node:test';
import { build } from 'esbuild';
import { Miniflare } from 'miniflare';
import { base64url, bytes, decode } from '../src/crypto.ts';
import { createWorker } from '../src/index.ts';
import { validCryptoInput } from '../src/compute-client.ts';
import { signedAssertion } from './benchmark-assertion.ts';
import { alert, fixture, now, privateKey, request } from './helpers.ts';

let runtime: Miniflare;
let actor: any;
let historical: any;
let sample: string;
async function invoke(method: string, args: unknown[] = [], actor = 'CRYPTO'): Promise<any> {
  return (await runtime.dispatchFetch('https://local/test', { method: 'POST', body: JSON.stringify({ method, args, actor }) })).json();
}
before(async () => {
  const bundled = await build({ entryPoints: [new URL('./compute-fixture.ts', import.meta.url).pathname], bundle: true,
    format: 'esm', platform: 'neutral', mainFields: ['module', 'main'], target: 'es2024', external: ['cloudflare:workers'], write: false });
  runtime = new Miniflare({ modules: true, script: bundled.outputFiles[0].text, compatibilityDate: '2026-08-06',
    durableObjects: { CRYPTO: { className: 'RelayCrypto', useSQLite: true }, HISTORICAL: { className: 'HistoricalFixture', useSQLite: true } },
    bindings: { APNS_TEAM_ID: 'TESTTEAM01', APNS_KEY_ID: 'TESTKEY001', APNS_PRIVATE_KEY: await privateKey(),
      APNS_ENVIRONMENT: 'development', ATTEST_BUNDLE_VERSIONS: '["1"]' } });
  actor = (await runtime.getDurableObjectNamespace('CRYPTO')).getByName('relay-crypto');
  historical = (await runtime.getDurableObjectNamespace('HISTORICAL')).getByName('fixture');
  sample = base64url(new Uint8Array(Buffer.from(await readFile(new URL('./fixtures/apple-attestation.b64', import.meta.url), 'utf8'), 'base64')));
});
after(async () => { await runtime?.dispose(); });
test('actor input bounds and fail-fast concurrency reject work without durable allocation or queue helpers', async () => {
  assert.equal(validCryptoInput('A'.repeat(16385), new Uint8Array(32)), false);
  assert.equal(await actor.attest('A'.repeat(16385), 'A'.repeat(43), new Uint8Array(32)), null);
  assert.equal(await actor.assertion('AQ', 'A'.repeat(257), 0, new Uint8Array(32)), null);
  assert.match((await invoke('run')).error, /method/i);
  const results = (await invoke('concurrency')).value;
  assert.ok(results.some(result => /crypto_busy/.test(result.error)));
  assert.equal((await runtime.listDurableObjectIds('CRYPTO')).length, 1);
});
test('a real SQLite actor verifies the genuine historical Apple chain and receipt without changing production policy', async () => {
  const verified = await historical.attest(sample, false);
  assert.ok(verified.publicKey.length > 0);
  assert.ok(verified.receipt.length > 0);
  assert.match((await invoke('attest', [sample, true], 'HISTORICAL')).error, /invalid_attestation/);
  const hash = new Uint8Array(32);
  assert.equal(await actor.attest(sample, 'zgSY9YSD-7TaDXssY6WlOPVS1K3Lmk-pFhlcSWE-ZV0', hash), null);
  assert.equal((await runtime.dispatchFetch('https://local/v1/attest')).status, 404);
  assert.match((await invoke('fetch', ['https://internal/'])).error, /fetch|method/i);
});
test('real actor signs deterministic provider windows and rejects caller-selected stale windows', async () => {
  const current = Math.floor(Date.now() / 1000);
  const token = await actor.providerToken(current);
  assert.equal(token, await actor.providerToken(current));
  const claims = JSON.parse(new TextDecoder().decode(decode(token.split('.')[1])));
  assert.equal(claims.iss, 'TESTTEAM01');
  assert.equal(claims.iat, Math.floor(current / 2700) * 2700);
  assert.match((await invoke('providerToken', [current + 2700])).error, /invalid_jwt_window/);
});
test('a real actor cold token reaches mocked APNs with quota policy intact', async () => {
  const f = await fixture();
  try {
    const current = Math.floor(Date.now() / 1000);
    const namespace = await runtime.getDurableObjectNamespace('CRYPTO');
    let calls = 0;
    const worker = createWorker(async (_url, init) => {
      calls++;
      const jwt = (init!.headers as Record<string, string>).authorization.slice(7);
      assert.equal(JSON.parse(new TextDecoder().decode(decode(jwt.split('.')[1]))).iss, 'TESTTEAM01');
      return new Response('', { status: 200 });
    }, () => current);
    const result = await worker.fetch(await request(f.grant), { ...f.env, CRYPTO: namespace } as never);
    assert.equal(result.status, 200);
    assert.equal(calls, 1);
    assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first())!.total, 1);
  } finally { await f.runtime.dispose(); }
});
test('missing, overloaded and exhausted compute fail closed before charge or APNs', async () => {
  const f = await fixture();
  try {
    const worker = createWorker(async () => { throw new Error('unexpected APNs'); }, () => now);
    for (const CRYPTO of [undefined, { getByName() { throw new Error('overloaded'); } },
      { getByName() { return { async providerToken() { throw new Error('daily request quota exceeded'); } }; } }]) {
      const result = await worker.fetch(await request(f.grant), { ...f.env, CRYPTO } as never);
      assert.equal(result.status, 503);
      assert.deepEqual(await result.json(), { v: 1, result: 'unavailable' });
    }
    assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first())!.total, 0);
    let calls = 0;
    const denied = createWorker();
    const bad = { ...alert, payload: { ...alert.payload, title: 'private' } };
    const result = await denied.fetch(await request(f.grant, bad), { ...f.env, CRYPTO: { getByName() { calls++; throw new Error('unexpected compute'); } } } as never);
    assert.equal(result.status, 400);
    assert.equal(calls, 0);
  } finally { await f.runtime.dispose(); }
});

test('real actor assertion verification retains D1 replay guards and never accepts caller policy context', async () => {
  const f = await fixture();
  try {
    const schema = await readFile(new URL('../migrations/0002_attestation.sql', import.meta.url), 'utf8');
    let current = '';
    for (const statement of schema.split(/;\s*(?:\n|$)/)) {
      if (!statement.trim()) continue;
      current += statement + ';\n';
      if (current.includes('CREATE TRIGGER') && !/\nEND;\s*$/.test(current)) continue;
      await f.db.prepare(current).run(); current = '';
    }
    const keys = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
    const publicKey = base64url(new Uint8Array(await crypto.subtle.exportKey('spki', keys.publicKey)));
    const keyId = base64url(new Uint8Array(await crypto.subtle.digest('SHA-256', await crypto.subtle.exportKey('raw', keys.publicKey))));
    await f.db.prepare(`INSERT INTO attestations (installation_id,key_id,public_key,receipt,environment,last_nonce,retain_until)
      VALUES ('phone', ?, ?, 'fixture', 'development', 'initial', ?)`).bind(keyId, publicKey, now + 37 * 86400).run();
    const env = { ...f.env, CRYPTO: await runtime.getDurableObjectNamespace('CRYPTO'), ATTEST_BUNDLE_VERSIONS: '["1"]',
      CONTROL_LIMITER: { async limit() { return { success: true }; } }, ENROLL_LIMITER: { async limit() { return { success: true }; } } };
    const worker = createWorker();
    const post = (path: string, body: unknown) => worker.fetch(new Request(`https://local/v1/${path}`, {
      method: 'POST', headers: { 'content-type': 'application/json', 'cf-connecting-ip': '192.0.2.1' }, body: JSON.stringify(body)
    }), env as never);
    const op = { v: 1, op: 'route', installation_id: 'phone', key_id: keyId, environment: 'development',
      pairing_id: 'second', route_id: 'second_app', expected_generation: 0, kind: 'app', destination: 'b'.repeat(64) };
    const challenge = (await (await post('challenge', op)).json()).challenge;
    const proof = await signedAssertion(keys.privateKey, challenge, 1);
    assert.equal((await post('control', { operation: op, challenge, assertion: proof })).status, 200);
    assert.equal((await post('control', { operation: op, challenge, assertion: await signedAssertion(keys.privateKey, challenge, 2) })).status, 401);
    assert.equal((await f.db.prepare('SELECT counter FROM attestations').first())!.counter, 1);
    assert.equal(await actor.assertion(proof, publicKey, 1, new Uint8Array(await crypto.subtle.digest('SHA-256', bytes(challenge)))), null);
    assert.equal(await actor.assertion(proof, publicKey, 0, new Uint8Array(32)), null);
    assert.equal((await post('control', { operation: op, challenge, assertion: proof, policy: { appId: 'foreign' } })).status, 400);
  } finally { await f.runtime.dispose(); }
});
