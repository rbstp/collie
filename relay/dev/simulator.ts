import { createServer } from 'node:http';
import { connect } from 'node:http2';
import { chmod, mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Miniflare } from 'miniflare';
import { base64url } from '../src/crypto.ts';
import { simulatorWorker } from './worker.ts';

const [mode, directory, team = 'SIMULATOR1'] = process.argv.slice(2);
if (!['init', 'serve'].includes(mode) || !directory || !/^[A-Z0-9]{10}$/.test(team)) {
  throw new Error('usage: node dev/simulator.ts init|serve PRIVATE_DIRECTORY [APPLE_TEAM_ID]');
}
const root = resolve(directory);
const identityPath = resolve(root, 'simulator-relay.json');
await mkdir(root, { recursive: true, mode: 0o700 });
await chmod(root, 0o700);
if (mode === 'init') {
  const keys = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
  const jwk = await crypto.subtle.exportKey('jwk', keys.privateKey);
  const identity = { installationId: crypto.randomUUID(), keyId: base64url(new Uint8Array(await crypto.subtle.digest('SHA-256', await crypto.subtle.exportKey('raw', keys.publicKey)))),
    privateKey: jwk.d, publicKey: base64url(new Uint8Array(await crypto.subtle.exportKey('spki', keys.publicKey))), teamId: team };
  await writeFile(identityPath, JSON.stringify(identity), { mode: 0o600, flag: 'wx' });
  await writeFile(resolve(root, 'send-secret'), base64url(crypto.getRandomValues(new Uint8Array(32))), { mode: 0o600, flag: 'wx' });
  console.log('Created a new local-only identity. Copy simulator-relay.json only to the DEBUG Simulator app container.');
} else {
  const identity = JSON.parse(await readFile(identityPath, 'utf8'));
  const secret = await readFile(resolve(root, 'send-secret'), 'utf8');
  const runtime = new Miniflare({ modules: true, script: 'export default {fetch(){return new Response("unused")}}',
    compatibilityDate: '2026-08-06', d1Databases: ['DB'], d1Persist: resolve(root, 'd1') });
  const db = await runtime.getD1Database('DB');
  const installed = await db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='attestations'").first();
  if (!installed) {
    for (const migration of ['0001_relay.sql', '0002_attestation.sql']) {
      let current = '';
      for (const statement of (await readFile(new URL(`../migrations/${migration}`, import.meta.url), 'utf8')).split(/;\s*(?:\n|$)/)) {
        if (!statement.trim()) continue;
        current += `${statement};\n`;
        if (current.includes('CREATE TRIGGER') && !/\nEND;\s*$/.test(current)) continue;
        await db.prepare(current).run(); current = '';
      }
    }
    await db.batch([
      db.prepare('INSERT INTO installations VALUES (?, 0)').bind(identity.installationId),
      db.prepare(`INSERT INTO attestations (installation_id,key_id,public_key,receipt,environment,last_nonce,retain_until)
        VALUES (?, ?, ?, 'simulator-provisioned', 'development', 'initial', ?)`).bind(identity.installationId, identity.keyId,
          identity.publicKey, Math.floor(Date.now() / 1000) + 37 * 86400),
      db.prepare('INSERT INTO installation_usage VALUES (?, ?, 0, 0)').bind(identity.installationId, Math.floor(Date.now() / 86400))
    ]);
  }
  const real = process.env.COLLIE_SIMULATOR_APNS_KEY_FILE;
  const signingKey = real ? await readFile(real, 'utf8') : await (async () => {
    const keys = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
    return `-----BEGIN PRIVATE KEY-----\n${Buffer.from(await crypto.subtle.exportKey('pkcs8', keys.privateKey)).toString('base64')}\n-----END PRIVATE KEY-----`;
  })();
  if (real && !/^[A-Z0-9]{10}$/.test(process.env.COLLIE_SIMULATOR_APNS_KEY_ID ?? '')) throw new Error('APNs key ID required');
  const env = { DB: db, APNS_ENVIRONMENT: 'development' as const, SEND_HMAC_KEYS: JSON.stringify({ simulator: secret }),
    APNS_TEAM_ID: identity.teamId, APNS_KEY_ID: real ? process.env.COLLIE_SIMULATOR_APNS_KEY_ID! : 'SIMULATOR1', APNS_PRIVATE_KEY: signingKey,
    ATTEST_BUNDLE_VERSIONS: '["1"]', SEND_LIMITER: { async limit() { return { success: true }; } },
    CONTROL_LIMITER: { async limit() { return { success: true }; } }, ENROLL_LIMITER: { async limit() { return { success: false }; } } };
  const sandbox: typeof fetch = async (input, init) => {
    const url = new URL(String(input));
    if (url.origin !== 'https://api.sandbox.push.apple.com' || !/^\/3\/device\/[a-f0-9]{64,512}$/.test(url.pathname)) throw new Error('sandbox_required');
    return new Promise<Response>((resolve, reject) => {
      const connection = connect(url.origin);
      connection.once('error', reject);
      connection.setTimeout(10000, () => connection.destroy(new Error('timeout')));
      const stream = connection.request({ ...Object.fromEntries(new Headers(init?.headers)), ':method': 'POST', ':path': url.pathname });
      let status = 503, size = 0;
      const chunks: Buffer[] = [];
      stream.on('response', headers => { status = Number(headers[':status']); });
      stream.on('data', (chunk: Buffer) => {
        size += chunk.length;
        if (size > 1024) stream.destroy(new Error('oversize_response')); else chunks.push(chunk);
      });
      stream.once('error', error => { connection.close(); reject(error); });
      stream.once('end', () => { connection.close(); resolve(new Response(Buffer.concat(chunks), { status })); });
      stream.end(String(init?.body));
    });
  };
  const upstream: typeof fetch = real ? sandbox : async (_url, init) => {
    const payload = JSON.parse(String(init?.body));
    await writeFile(resolve(root, 'last-push.apns'), JSON.stringify(payload), { mode: 0o600 });
    return process.env.COLLIE_SIMULATOR_APNS_OUTAGE === '1' ? new Response('', { status: 503 }) : new Response('', { status: 200 });
  };
  const worker = simulatorWorker(upstream);
  const server = createServer(async (incoming, outgoing) => {
    try {
      if (incoming.method !== 'POST' || !['/v1/challenge', '/v1/control', '/v1/send'].includes(incoming.url ?? '')) {
        outgoing.writeHead(404).end(); return;
      }
      const chunks: Buffer[] = []; let size = 0;
      for await (const chunk of incoming) {
        size += chunk.length; if (size > 16384) { outgoing.writeHead(413).end(); return; }
        chunks.push(chunk);
      }
      const headers = new Headers({ 'content-type': 'application/json', 'cf-connecting-ip': '127.0.0.1' });
      if (incoming.headers.authorization) headers.set('authorization', incoming.headers.authorization);
      const result = await worker.fetch(new Request(`http://127.0.0.1:8788${incoming.url}`, {
        method: 'POST', headers, body: Buffer.concat(chunks) }), env);
      outgoing.writeHead(result.status, Object.fromEntries(result.headers)).end(await result.text());
    } catch { outgoing.writeHead(503).end(); }
  });
  server.requestTimeout = 5000;
  server.listen(8788, '127.0.0.1', () => console.log(`Local Simulator relay: ${real ? 'REAL SANDBOX APNs' : 'MOCK APNs'}, http://127.0.0.1:8788`));
  for (const signal of ['SIGINT', 'SIGTERM'] as const) process.on(signal, () => server.close(() => { void runtime.dispose().then(() => process.exit()); }));
}
