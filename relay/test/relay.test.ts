import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { createWorker, signGrant } from '../src/index.ts';
import { base64url, bytes, decode, ProviderToken } from '../src/crypto.ts';
import { validSend, readBody } from '../src/contract.ts';
import { charge, invalidate, lookupRoute } from '../src/storage.ts';
import { alert, fixture, now, privateKey, request, secret } from './helpers.ts';

let f: Awaited<ReturnType<typeof fixture>>;
before(async () => { f = await fixture(); });
after(async () => { await f?.runtime.dispose(); });
async function reset() {
  await f.db.prepare('DELETE FROM installation_usage').run();
  await f.db.prepare('INSERT INTO installation_usage VALUES (?, ?, 0, 0)').bind('phone', Math.floor(now / 86400)).run();
  await f.db.prepare('UPDATE global_usage SET day = ?, total = 0, routine = 0').bind(Math.floor(now / 86400)).run();
}
test('deterministic ES256 JWTs match across isolates, verify, and rotate only at a 45-minute boundary', async () => {
  const pem = await privateKey();
  const a = new ProviderToken('TESTTEAM01', 'TESTKEY001', pem);
  const b = new ProviderToken('TESTTEAM01', 'TESTKEY001', pem);
  const token = await a.token(now);
  assert.equal(token, await b.token(now));
  assert.equal(token, await a.token(now + 1));
  assert.notEqual(token, await a.token(now + 2700));
  const parts = token.split('.');
  const der = Buffer.from(pem.replace(/-----[^-]+-----|\s/g, ''), 'base64');
  const key = await crypto.subtle.importKey('pkcs8', der, { name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign']);
  const jwk = await crypto.subtle.exportKey('jwk', key);
  delete jwk.d; jwk.key_ops = ['verify'];
  const publicKey = await crypto.subtle.importKey('jwk', jwk, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);
  assert.equal(await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, publicKey, decode(parts[2]), bytes(parts.slice(0, 2).join('.'))), true);
});
test('forwarding constructs only fixed APNs headers and returns no upstream data', async () => {
  await reset();
  let calls = 0;
  const worker = createWorker(async (url, init) => {
    calls++;
    assert.equal(url, `https://api.sandbox.push.apple.com/3/device/${'a'.repeat(64)}`);
    assert.equal(init?.redirect, 'manual');
    assert.deepEqual(Object.keys(init!.headers!).sort(), ['apns-collapse-id', 'apns-expiration', 'apns-priority', 'apns-push-type', 'apns-topic', 'authorization', 'content-type']);
    assert.equal((init!.headers as Record<string, string>)['apns-topic'], 'dev.rbstp.collie');
    return new Response('', { status: 200, headers: { 'apns-id': 'private' } });
  }, () => now);
  const result = await worker.fetch(await request(f.grant), f.env as never);
  assert.deepEqual(await result.json(), { v: 1, result: 'accepted' });
  assert.equal(calls, 1);
});
test('rejects invalid authentication, mismatched environment and expired grants before quota or APNs', async () => {
  await reset();
  const worker = createWorker(async () => { throw new Error('unexpected APNs'); }, () => now);
  for (const change of [{ expires_at: now }, { environment: 'production' }, { generation: -1 }]) {
    const result = await worker.fetch(await request({ ...f.grant, ...change } as never), f.env as never);
    assert.equal(result.status, 401);
  }
  const req = await request(f.grant);
  req.headers.set('authorization', `${req.headers.get('authorization')}x`);
  assert.equal((await worker.fetch(req, f.env as never)).status, 401);
  assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first())!.total, 0);
});
test('key ID selects one HMAC key and supports bounded rotation overlap', async () => {
  const worker = createWorker(async () => new Response('', { status: 200 }), () => now);
  const req = await request(f.grant);
  req.headers.set('authorization', `Bearer ${await signGrant(f.grant, 'next', secret)}`);
  assert.equal((await worker.fetch(req, { ...f.env, SEND_HMAC_KEYS: JSON.stringify({ first: secret, next: secret }) } as never)).status, 200);
  const removed = createWorker(async () => new Response('', { status: 200 }), () => now);
  assert.equal((await removed.fetch(await request(f.grant), { ...f.env, SEND_HMAC_KEYS: JSON.stringify({ next: secret }) } as never)).status, 401);
});
test('strict public allowlists reject plaintext, arbitrary headers, malformed envelopes and oversized ciphertext', () => {
  assert.equal(validSend(alert), true);
  for (const mutate of [
    (x: any) => { x.topic = 'other'; }, (x: any) => { x.payload.title = 'private'; },
    (x: any) => { x.payload.aps.alert.body = 'private'; }, (x: any) => { x.payload.enc = 'a'.repeat(4000); },
    (x: any) => { x.payload.aps.badge = 3; }, (x: any) => { x.collapse_id = '\r\ninjected'; },
    (x: any) => { x.payload.enc = 'invalid='; }
  ]) { const body = structuredClone(alert); mutate(body); assert.equal(validSend(body), false); }
});
test('stream body limit rejects absent length, misleading lengths, compression and invalid UTF8', async () => {
  for (const body of ['x'.repeat(8193), new Uint8Array([255])]) {
    const req = new Request('https://relay.example', { method: 'POST', headers: { 'content-type': 'application/json', 'content-length': '1' }, body });
    await assert.rejects(readBody(req));
  }
  await assert.rejects(readBody(new Request('https://relay.example', { method: 'POST', headers: { 'content-type': 'application/json', 'content-encoding': 'gzip' }, body: '{}' })));
});
test('installation routine cap reserves alerts and installation total cap is exact under concurrency', async () => {
  await reset();
  await f.db.prepare('UPDATE installation_usage SET total = 699, routine = 699').run();
  assert.equal(await charge(f.db as never, f.grant, now, true), 'charged');
  assert.equal(await charge(f.db as never, f.grant, now, true), 'daily_quota');
  assert.equal(await charge(f.db as never, f.grant, now, false), 'charged');
  await f.db.prepare('UPDATE installation_usage SET total = 999').run();
  const results = await Promise.all(Array.from({ length: 20 }, () => charge(f.db as never, f.grant, now, false)));
  assert.equal(results.filter(result => result === 'charged').length, 1);
  assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first())!.total, 1000);
});
test('global cap rolls back the installation counter under concurrent last-slot requests', async () => {
  await reset();
  await f.db.prepare('UPDATE global_usage SET total = 19999, routine = 15999').run();
  const results = await Promise.all(Array.from({ length: 20 }, () => charge(f.db as never, f.grant, now, true)));
  assert.equal(results.filter(result => result === 'charged').length, 1);
  assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first())!.total, 1);
  assert.equal((await f.db.prepare('SELECT total FROM global_usage').first())!.total, 20000);
});
test('global routine cap reserves total capacity for nonroutine sends', async () => {
  await reset();
  await f.db.prepare('UPDATE global_usage SET total = 16000, routine = 16000').run();
  assert.equal(await charge(f.db as never, f.grant, now, true), 'daily_quota');
  assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first())!.total, 0);
  assert.equal(await charge(f.db as never, f.grant, now, false), 'charged');
});
test('UTC midnight resets both counters, including a device with yesterday routine count', async () => {
  await reset();
  await f.db.prepare('UPDATE installation_usage SET total = 1000, routine = 700').run();
  await f.db.prepare('UPDATE global_usage SET total = 20000, routine = 16000').run();
  const tomorrow = (Math.floor(now / 86400) + 1) * 86400;
  const grant = { ...f.grant, expires_at: tomorrow + 100 };
  await f.db.prepare('UPDATE routes SET expires_at = ?').bind(grant.expires_at).run();
  assert.equal(await charge(f.db as never, grant, tomorrow, true), 'charged');
  assert.deepEqual(await f.db.prepare('SELECT total, routine FROM global_usage').first(), { total: 1, routine: 1 });
  assert.deepEqual(await f.db.prepare('SELECT total, routine FROM installation_usage').first(), { total: 1, routine: 1 });
  await f.db.prepare('UPDATE routes SET expires_at = ?').bind(f.grant.expires_at).run();
});
test('revocation between route lookup and quota charge fails closed', async () => {
  await reset();
  assert.ok(await lookupRoute(f.db as never, f.grant, now));
  await f.db.prepare('UPDATE pairings SET epoch = epoch + 1').run();
  assert.equal(await charge(f.db as never, f.grant, now, false), 'invalid_route');
  await f.db.prepare('UPDATE pairings SET epoch = 0').run();
  await f.db.prepare('UPDATE routes SET revoked = 1').run();
  assert.equal(await lookupRoute(f.db as never, f.grant, now), null);
  await f.db.prepare('UPDATE routes SET revoked = 0, expires_at = ?').bind(now).run();
  assert.equal(await lookupRoute(f.db as never, f.grant, now), null);
  await f.db.prepare('UPDATE routes SET expires_at = ?').bind(f.grant.expires_at).run();
});
test('old invalidation cannot revoke a replacement route generation', async () => {
  await f.db.prepare('UPDATE routes SET generation = 2').run();
  await invalidate(f.db as never, f.grant);
  assert.equal((await f.db.prepare('SELECT revoked FROM routes').first())!.revoked, 0);
  await f.db.prepare('UPDATE routes SET generation = 1').run();
});
test('normalizes APNs rejection and never erases a route for configuration failures', async () => {
  await reset();
  for (const [status, reason, expected] of [[400, 'DeviceTokenNotForTopic', 502], [403, 'InvalidProviderToken', 502], [429, 'TooManyRequests', 503], [410, 'Unregistered', 410], [400, 'BadDeviceToken', 410]] as const) {
    await f.db.prepare('UPDATE routes SET revoked = 0').run();
    const worker = createWorker(async () => Response.json({ reason, destination: 'secret' }, { status }), () => now);
    const result = await worker.fetch(await request(f.grant), f.env as never);
    assert.equal(result.status, expected);
    assert.equal((await result.text()).includes('secret'), false);
    assert.equal((await f.db.prepare('SELECT revoked FROM routes').first())!.revoked, expected === 410 ? 1 : 0);
  }
  await f.db.prepare('UPDATE routes SET revoked = 0').run();
});

test('validates Live Activity encrypted state, routine classification and reserved end capacity', async () => {
  await reset();
  const grant = { ...f.grant, push_classes: ['liveactivity'] as const };
  await f.db.prepare("UPDATE routes SET kind = 'liveactivity'").run();
  const body = { v: 1, push_type: 'liveactivity', priority: 5, expiration: now + 300,
    payload: { node_id: 'node', event_id: 'event', aps: { timestamp: now, event: 'update',
      'content-state': { v: 1, enc: base64url(new Uint8Array(40).fill(1)) } } } };
  const worker = createWorker(async () => new Response('', { status: 200 }), () => now);
  assert.equal((await worker.fetch(await request(grant as never, body), f.env as never)).status, 200);
  assert.equal((await f.db.prepare('SELECT routine FROM installation_usage').first())!.routine, 1);
  await f.db.prepare('UPDATE global_usage SET total = 16000, routine = 16000').run();
  assert.equal((await worker.fetch(await request(grant as never, body), f.env as never)).status, 429);
  body.payload.aps.event = 'end';
  assert.equal((await worker.fetch(await request(grant as never, body), f.env as never)).status, 200);
  assert.equal((await f.db.prepare('SELECT routine FROM installation_usage').first())!.routine, 1);
  await f.db.prepare("UPDATE routes SET kind = 'app'").run();
});
test('malformed requests and soft rate limits never reach APNs or consume daily quota', async () => {
  await reset();
  const worker = createWorker(async () => { throw new Error('unexpected APNs'); }, () => now);
  assert.equal((await worker.fetch(await request(f.grant, { ...alert, url: 'https://attacker.example' }), f.env as never)).status, 400);
  const limited = { ...f.env, SEND_LIMITER: { async limit() { return { success: false }; } } };
  assert.equal((await worker.fetch(await request(f.grant), limited as never)).status, 429);
  assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first())!.total, 0);
});

test('a delayed previous-day invocation cannot roll counters backward after midnight', async () => {
  await reset();
  const tomorrow = (Math.floor(now / 86400) + 1) * 86400;
  const grant = { ...f.grant, expires_at: tomorrow + 100 };
  await f.db.prepare('UPDATE routes SET expires_at = ?').bind(grant.expires_at).run();
  assert.equal(await charge(f.db as never, grant, tomorrow, true), 'charged');
  assert.equal(await charge(f.db as never, grant, now, false), 'daily_quota');
  assert.deepEqual(await f.db.prepare('SELECT total, routine FROM global_usage').first(), { total: 1, routine: 1 });
  await f.db.prepare('UPDATE routes SET expires_at = ?').bind(f.grant.expires_at).run();
});

async function streamedRequest(grant: typeof f.grant, finish: () => void | Promise<void>) {
  const req = await request(grant);
  const body = JSON.stringify(alert);
  let first = true;
  const stream = new ReadableStream({
    async pull(controller) {
      if (first) { first = false; controller.enqueue(bytes(body)); }
      else { await finish(); controller.close(); }
    }
  });
  return new Request(req.url, { method: 'POST', headers: req.headers, body: stream, duplex: 'half' } as RequestInit);
}
function interceptCharge(after: () => void | Promise<void>) {
  return {
    prepare(sql: string) {
      const statement = f.db.prepare(sql);
      if (!sql.startsWith('UPDATE installation_usage')) return statement;
      return { bind(...values: unknown[]) {
        const bound = statement.bind(...values);
        return { async first() { const result = await bound.first(); await after(); return result; } };
      } };
    }
  };
}
test('a grant expiring during streamed body parsing never reaches quota or APNs', async () => {
  await reset();
  let time = Math.floor(Date.now() / 1000);
  const grant = { ...f.grant, expires_at: time + 1 };
  let calls = 0;
  const worker = createWorker(async () => { calls++; return new Response('', { status: 200 }); }, () => time);
  const req = await streamedRequest(grant, () => { time += 2; });
  assert.equal((await worker.fetch(req, f.env as never)).status, 401);
  assert.equal(calls, 0);
  assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first())!.total, 0);
});
test('streamed requests crossing midnight use the new quota day and JWT window', async () => {
  await reset();
  const midnight = (Math.floor(now / 86400) + 1) * 86400;
  let time = midnight - 1;
  const grant = { ...f.grant, expires_at: midnight + 100 };
  await f.db.prepare('UPDATE routes SET expires_at = ?').bind(grant.expires_at).run();
  await f.db.prepare('UPDATE installation_usage SET total = 1000').run();
  await f.db.prepare('UPDATE global_usage SET total = 20000').run();
  let calls = 0;
  const worker = createWorker(async (_url, init) => {
    calls++;
    const token = (init!.headers as Record<string, string>).authorization.slice(7);
    const payload = JSON.parse(new TextDecoder().decode(decode(token.split('.')[1])));
    assert.equal(payload.iat, midnight);
    return new Response('', { status: 200 });
  }, () => time);
  const req = await streamedRequest(grant, () => { time = midnight + 1; });
  assert.equal((await worker.fetch(req, f.env as never)).status, 200);
  assert.equal(calls, 1);
  assert.deepEqual(await f.db.prepare('SELECT day, total FROM installation_usage').first(), { day: midnight / 86400, total: 1 });
  assert.equal((await f.db.prepare('SELECT total FROM global_usage').first())!.total, 1);
  await f.db.prepare('UPDATE routes SET expires_at = ?').bind(f.grant.expires_at).run();
});
test('grant expiry during an awaited quota response stops forwarding after consuming the reservation', async () => {
  await reset();
  let time = Math.floor(Date.now() / 1000);
  const grant = { ...f.grant, expires_at: time + 60 };
  let calls = 0;
  const worker = createWorker(async () => { calls++; return new Response('', { status: 200 }); }, () => time);
  const db = interceptCharge(() => { time += 61; });
  assert.equal((await worker.fetch(await request(grant), { ...f.env, DB: db } as never)).status, 401);
  assert.equal(calls, 0);
  assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first())!.total, 1);
});
test('an epoch change between lookup and charge returns invalid_route rather than daily_quota', async () => {
  await reset();
  let calls = 0;
  const worker = createWorker(async () => { calls++; return new Response('', { status: 200 }); }, () => now);
  const db = { prepare(sql: string) {
    const statement = f.db.prepare(sql);
    if (!sql.startsWith('UPDATE installation_usage')) return statement;
    return { bind(...values: unknown[]) { const bound = statement.bind(...values); return { async first() {
      await f.db.prepare('UPDATE pairings SET epoch = epoch + 1').run();
      return bound.first();
    } }; } };
  } };
  const result = await worker.fetch(await request(f.grant), { ...f.env, DB: db } as never);
  assert.equal(result.status, 410);
  assert.deepEqual(await result.json(), { v: 1, result: 'invalid_route' });
  assert.equal(calls, 0);
  assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first())!.total, 0);
  await f.db.prepare('UPDATE pairings SET epoch = 0').run();
});
test('body duration is bounded even when the stream never finishes or cancellation stalls', async () => {
  const stream = new ReadableStream({ start(controller) { controller.enqueue(bytes('{}')); }, cancel() { return new Promise(() => {}); } });
  const req = new Request('https://relay.example', { method: 'POST', headers: { 'content-type': 'application/json' }, body: stream, duplex: 'half' } as RequestInit);
  await assert.rejects(readBody(req, 20), /invalid_request/);
});
test('route expiry during a streamed request returns invalid_route without consuming quota', async () => {
  await reset();
  let time = Math.floor(Date.now() / 1000);
  const grant = { ...f.grant, expires_at: time + 100 };
  await f.db.prepare('UPDATE routes SET expires_at = ?').bind(grant.expires_at).run();
  let calls = 0;
  const worker = createWorker(async () => { calls++; return new Response('', { status: 200 }); }, () => time);
  const req = await streamedRequest(grant, async () => {
    await f.db.prepare('UPDATE routes SET expires_at = ?').bind(time + 1).run();
    time += 2;
  });
  assert.equal((await worker.fetch(req, f.env as never)).status, 410);
  assert.equal(calls, 0);
  assert.equal((await f.db.prepare('SELECT total FROM installation_usage').first())!.total, 0);
  await f.db.prepare('UPDATE routes SET expires_at = ?').bind(f.grant.expires_at).run();
});
test('real quota exhaustion returns daily_quota but an absent counter returns unavailable', async () => {
  await reset();
  const worker = createWorker(async () => { throw new Error('unexpected APNs'); }, () => now);
  await f.db.prepare('UPDATE global_usage SET total = 20000').run();
  const exhausted = await worker.fetch(await request(f.grant), f.env as never);
  assert.equal(exhausted.status, 429);
  assert.deepEqual(await exhausted.json(), { v: 1, result: 'daily_quota', reset_at: (Math.floor(now / 86400) + 1) * 86400 });
  await f.db.prepare('DELETE FROM installation_usage').run();
  const missing = await worker.fetch(await request(f.grant), f.env as never);
  assert.equal(missing.status, 503);
  assert.deepEqual(await missing.json(), { v: 1, result: 'unavailable' });
});
