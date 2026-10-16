import assert from 'node:assert/strict';
import { createPrivateKey, sign } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { provisionSimulator, simulatorRuntime, simulatorWorker } from '../dev/worker.ts';
import { createWorker, signGrant } from '../src/index.ts';
import { base64url, bytes, decode } from '../src/crypto.ts';
import { fixture, privateKey, secret } from './helpers.ts';

const now = Math.floor(Date.now() / 1000);

function item(data: Uint8Array, kind: number): Buffer {
  return Buffer.concat([Buffer.from(data.length < 24 ? [kind | data.length] : [kind | 24, data.length]), data]);
}
async function migration(db: D1Database) {
  let current = '';
  for (const statement of (await readFile(new URL('../migrations/0002_attestation.sql', import.meta.url), 'utf8')).split(/;\s*(?:\n|$)/)) {
    if (!statement.trim()) continue;
    current += statement + ';\n';
    if (current.includes('CREATE TRIGGER') && !/\nEND;\s*$/.test(current)) continue;
    await db.prepare(current).run(); current = '';
  }
}
async function setup() {
  const keys = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
  const keyId = base64url(new Uint8Array(await crypto.subtle.digest('SHA-256', await crypto.subtle.exportKey('raw', keys.publicKey))));
  const publicKey = base64url(new Uint8Array(await crypto.subtle.exportKey('spki', keys.publicKey)));
  const key = createPrivateKey({ key: await crypto.subtle.exportKey('jwk', keys.privateKey), format: 'jwk' });
  const cryptoEnv = { APNS_TEAM_ID: 'TESTTEAM01', APNS_KEY_ID: 'TESTKEY001', APNS_PRIVATE_KEY: await privateKey(),
    APNS_ENVIRONMENT: 'development' as const, ATTEST_BUNDLE_VERSIONS: '["1"]' };
  const runtime = await simulatorRuntime(cryptoEnv);
  const db = await runtime.getD1Database('DB');
  await provisionSimulator(db, { installationId: 'phone', keyId, publicKey });
  const env = { ...cryptoEnv, DB: db, CRYPTO: await runtime.getDurableObjectNamespace('CRYPTO'),
    SEND_HMAC_KEYS: JSON.stringify({ first: secret }), SEND_LIMITER: { async limit() { return { success: true }; } },
    CONTROL_LIMITER: { async limit() { return { success: true }; } }, ENROLL_LIMITER: { async limit() { return { success: true }; } } };
  const common = { v: 1, installation_id: 'phone', key_id: keyId, environment: 'development' };
  const upstream: string[] = []; let outage = false;
  const worker = simulatorWorker(async (url, init) => {
    upstream.push(String(url));
    const token = new Headers(init?.headers).get('authorization')!.slice(7).split('.');
    assert.equal(JSON.parse(new TextDecoder().decode(decode(token[0]))).kid, 'TESTKEY001');
    assert.equal(JSON.parse(new TextDecoder().decode(decode(token[1]))).iss, 'TESTTEAM01');
    return new Response('', { status: outage ? 503 : 200 });
  }, () => now);
  const post = (path: string, body: unknown) => worker.fetch(new Request(`http://127.0.0.1:8788/v1/${path}`, {
    method: 'POST', headers: { 'content-type': 'application/json', 'cf-connecting-ip': '127.0.0.1' }, body: JSON.stringify(body)
  }), env);
  let counter = 0;
  async function proof(challenge: string) {
    const count = Buffer.alloc(4); count.writeUInt32BE(++counter);
    const auth = Buffer.concat([Buffer.from(await crypto.subtle.digest('SHA-256', bytes('TESTTEAM01.dev.rbstp.collie'))), Buffer.from([0]), count]);
    const signature = sign('sha256', Buffer.concat([auth, Buffer.from(await crypto.subtle.digest('SHA-256', bytes(challenge)))]), key);
    return base64url(Buffer.concat([Buffer.from([0xa2]), item(bytes('signature'), 0x60), item(signature, 0x40),
      item(bytes('authenticatorData'), 0x60), item(auth, 0x40)]));
  }
  async function signed(operation: unknown) {
    const challenge = (await (await post('challenge', operation)).json()).challenge;
    return { operation, challenge, assertion: await proof(challenge) };
  }
  return { runtime, db, env, common, upstream, worker, post, signed, proof, outage: () => { outage = true; } };
}
test('local identity uses strict encrypted control/send, replay, quota, revocation and outage paths', async () => {
  const f = await setup();
  try {
    assert.equal((await f.post('challenge', { ...f.common, op: 'enroll' })).status, 403);
    assert.equal((await f.runtime.dispatchFetch('http://local/v1/challenge', { method: 'POST', body: '{}' })).status, 404);
    await assert.rejects(simulatorRuntime({ ...f.env, APNS_ENVIRONMENT: 'production' }), /sandbox_required/);
    const operation = { ...f.common, op: 'route', pairing_id: 'machine', route_id: 'route', expected_generation: 0,
      kind: 'app', destination: 'a'.repeat(64) };
    const body = await f.signed(operation);
    const registration = await f.post('control', body); assert.equal(registration.status, 200);
    const grant = (await registration.json()).grant;
    assert.equal((await f.post('control', body)).status, 401);
    assert.equal((await f.post('control', { ...body, assertion: await f.proof(body.challenge) })).status, 401);
    const vectors = JSON.parse(await readFile(new URL('../../docs/protocol/push-display-vectors.json', import.meta.url), 'utf8'));
    const payload = vectors.vectors.find((v: { kind: number; payload?: unknown }) => v.kind === 1 && v.payload).payload;
    const send = async (value = payload, env = f.env) => f.worker.fetch(new Request('http://127.0.0.1:8788/v1/send', {
      method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${grant}` },
      body: JSON.stringify({ v: 1, push_type: 'alert', priority: 10, expiration: 0, payload: value })
    }), env);
    assert.equal((await send()).status, 200);
    assert.match(f.upstream[0], /^https:\/\/api\.sandbox\.push\.apple\.com\/3\/device\//);
    assert.equal((await f.runtime.listDurableObjectIds('CRYPTO')).length, 1);
    assert.equal((await send(payload, { ...f.env, CRYPTO: undefined })).status, 503);
    assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first()).total, 1);
    assert.equal((await send({ ...payload, title: 'Private name' })).status, 400);
    f.outage(); assert.equal((await send()).status, 503);
    await f.db.prepare('UPDATE installation_usage SET total = 1000 WHERE installation_id = ?').bind('phone').run();
    assert.equal((await send()).status, 429);
    assert.equal((await f.post('control', await f.signed({ ...f.common, op: 'revoke_pairing', pairing_id: 'machine' }))).status, 200);
    assert.equal((await send()).status, 410);
    assert.equal(f.upstream.length, 2);
  } finally { await f.runtime.dispose(); }
});
test('production entry rejects local identity enrollment, assertions and separately signed local grants', async () => {
  const local = await setup(), production = await fixture();
  try {
    await migration(production.db);
    const env = { ...production.env, SEND_HMAC_KEYS: JSON.stringify({ production: base64url(new Uint8Array(32).fill(9)) }),
      APNS_ENVIRONMENT: 'production' as const, ATTEST_BUNDLE_VERSIONS: '["1"]',
      CONTROL_LIMITER: { async limit() { return { success: true }; } }, ENROLL_LIMITER: { async limit() { return { success: true }; } } };
    let forwarded = false;
    const worker = createWorker(async () => { forwarded = true; return new Response(); }, () => now);
    const post = (path: string, body: unknown) => worker.fetch(new Request(`https://publisher.example/v1/${path}`, {
      method: 'POST', headers: { 'content-type': 'application/json', 'cf-connecting-ip': '192.0.2.1' }, body: JSON.stringify(body)
    }), env);
    assert.equal((await post('challenge', { ...local.common, op: 'route', pairing_id: 'machine', route_id: 'new',
      expected_generation: 0, kind: 'app', destination: 'a'.repeat(64) })).status, 400);
    const op = { ...local.common, environment: 'production', op: 'enroll' };
    const challenge = (await (await post('challenge', op)).json()).challenge;
    assert.equal((await post('control', { operation: op, challenge, attestation: await local.proof(challenge) })).status, 401);
    const token = await signGrant({ ...production.grant, environment: 'production' }, 'simulator', secret);
    assert.equal((await worker.fetch(new Request('https://publisher.example/v1/send', {
      method: 'POST', headers: { authorization: `Bearer ${token}` }, body: '{}'
    }), env)).status, 401);
    assert.equal(forwarded, false);
    assert.equal((await production.db.prepare('SELECT COUNT(*) AS n FROM attestations').first()).n, 0);
    assert.equal((await local.worker.fetch(new Request('http://127.0.0.1:8788/v1/send', { method: 'POST' }),
      { ...local.env, APNS_ENVIRONMENT: 'production' })).status, 403);
  } finally { await local.runtime.dispose(); await production.runtime.dispose(); }
});
