import { readFile } from 'node:fs/promises';
import { Miniflare } from 'miniflare';
import { base64url, digest } from '../src/crypto.ts';
import { signGrant } from '../src/index.ts';
import type { Grant, Send } from '../src/contract.ts';

export const now = 1792152000;
export const secret = base64url(new Uint8Array(32).fill(7));
export async function privateKey(): Promise<string> {
  const key = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
  return `-----BEGIN PRIVATE KEY-----\n${Buffer.from(await crypto.subtle.exportKey('pkcs8', key.privateKey)).toString('base64')}\n-----END PRIVATE KEY-----`;
}
export async function fixture() {
  const runtime = new Miniflare({ modules: true, script: 'export default { fetch() { return new Response("unused"); } };', compatibilityDate: '2026-08-06', d1Databases: ['DB'] });
  const db = await runtime.getD1Database('DB');
  const schema = await readFile(new URL('../migrations/0001_relay.sql', import.meta.url), 'utf8');
  const statements = schema.split(/;\s*(?:\n|$)/);
  let current = '';
  for (const statement of statements) {
    if (!statement.trim()) continue;
    current += statement + ';\n';
    if (current.includes('CREATE TRIGGER') && !/\nEND;\s*$/.test(current)) continue;
    await db.prepare(current).run();
    current = '';
  }
  const grant: Grant = { v: 1, route_id: 'route', generation: 1, installation_id: 'phone', installation_epoch: 0,
    pairing_id: 'machine', pairing_epoch: 0, destination_digest: await digest('a'.repeat(64)),
    push_classes: ['alert', 'background'], environment: 'development', expires_at: now + 86400 };
  await db.batch([
    db.prepare('INSERT INTO installations VALUES (?, 0)').bind('phone'),
    db.prepare('INSERT INTO pairings VALUES (?, ?, 0)').bind('machine', 'phone'),
    db.prepare('INSERT INTO routes VALUES (?, ?, 1, ?, ?, ?, ?, ?, 0)').bind('route', 'machine', 'a'.repeat(64), grant.destination_digest, 'app', 'development', grant.expires_at),
    db.prepare('INSERT INTO installation_usage VALUES (?, ?, 0, 0)').bind('phone', Math.floor(now / 86400))
  ]);
  const env = { DB: db, APNS_ENVIRONMENT: 'development' as const, SEND_HMAC_KEYS: JSON.stringify({ first: secret }),
    APNS_TEAM_ID: 'TESTTEAM01', APNS_KEY_ID: 'TESTKEY001', APNS_PRIVATE_KEY: await privateKey(),
    SEND_LIMITER: { async limit() { return { success: true }; } } };
  return { runtime, db, grant, env };
}
export const alert: Send = { v: 1, push_type: 'alert', priority: 10, expiration: 0, collapse_id: 'terminal',
  payload: { aps: { alert: { title: 'Collie', body: 'An agent needs attention' }, 'mutable-content': 1,
    sound: 'default', category: 'APPROVAL', 'thread-id': 'terminal' },
  node_id: 'node', event_id: 'event', approval_id: 'approval', enc: base64url(new Uint8Array(40).fill(1)) } };
export async function request(grant: Grant, body: unknown = alert, extra: Record<string, string> = {}) {
  return new Request('https://relay.example/v1/send', { method: 'POST', headers: { 'content-type': 'application/json',
    authorization: `Bearer ${await signGrant(grant, 'first', secret)}`, ...extra }, body: JSON.stringify(body) });
}
