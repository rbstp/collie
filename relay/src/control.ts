import { base64url, bytes, decode, digest, hmacKey } from './crypto.ts';
import { exact, object, readBody } from './contract.ts';
import type { Environment, Grant } from './contract.ts';
import { compute, validCryptoInput, validKeyId } from './compute-client.ts';
import type { CryptoBinding } from './compute-client.ts';

export interface ControlEnv extends CryptoBinding {
  DB: D1Database;
  APNS_ENVIRONMENT: Environment;
  APNS_TEAM_ID: string;
  SEND_HMAC_KEYS: string;
  ATTEST_MIN_BUNDLE_VERSION?: string;
  CONTROL_LIMITER?: { limit(options: { key: string }): Promise<{ success: boolean }> };
  ENROLL_LIMITER?: { limit(options: { key: string }): Promise<{ success: boolean }> };
}
interface Common { v: 1; op: string; installation_id: string; key_id: string; environment: Environment }
export type Operation = Common & (
  { op: 'enroll' }
  | { op: 'revoke_pairing'; pairing_id: string }
  | { op: 'revoke_route'; pairing_id: string; route_id: string; expected_generation: number }
  | { op: 'route'; pairing_id: string; route_id: string; expected_generation: number; kind: 'app' | 'liveactivity'; destination: string }
);
interface State { public_key: string; counter: number; epoch: number; revoked: number; environment: Environment; key_id: string }
interface Challenge {
  v: 1; operation_digest: string; installation_id: string; key_id: string;
  previous_counter: number; epoch: number; nonce: string; expires_at: number;
}
const opaque = (v: unknown): v is string => typeof v === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(v);
const integer = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) >= 0 && (v as number) < 4294967295;
export function operation(value: unknown): Operation | null {
  if (!object(value) || value.v !== 1 || !opaque(value.installation_id)
    || typeof value.key_id !== 'string' || !/^[A-Za-z0-9_-]{43}$/.test(value.key_id)
    || !['production', 'development'].includes(value.environment as string)) return null;
  const common = ['v', 'op', 'installation_id', 'key_id', 'environment'];
  if (value.op === 'enroll') {
    if (!exact(value, common)) return null;
  } else if (value.op === 'revoke_pairing') {
    if (!exact(value, [...common, 'pairing_id']) || !opaque(value.pairing_id)) return null;
  } else if (value.op === 'route' || value.op === 'revoke_route') {
    const fields = [...common, 'pairing_id', 'route_id', 'expected_generation'];
    if (value.op === 'route') fields.push('kind', 'destination');
    if (!exact(value, fields) || !opaque(value.pairing_id) || !opaque(value.route_id) || !integer(value.expected_generation)) return null;
    if (value.op === 'route' && (!['app', 'liveactivity'].includes(value.kind as string)
      || typeof value.destination !== 'string' || !/^[a-f0-9]{64,512}$/.test(value.destination))) return null;
  } else return null;
  return value as unknown as Operation;
}
export function canonical(value: Operation): string {
  return JSON.stringify(Object.fromEntries(Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)));
}
const response = (status: number, result: string, extra: Record<string, unknown> = {}) =>
  Response.json({ v: 1, result, ...extra }, { status, headers: { 'cache-control': 'no-store' } });
function configured(env: ControlEnv): [string, string][] {
  const value: unknown = JSON.parse(env.SEND_HMAC_KEYS);
  if (!object(value) || Object.keys(value).length < 1 || Object.keys(value).length > 2) throw new Error('invalid_keys');
  return Object.entries(value).map(([id, secret]) => {
    if (!/^[A-Za-z0-9_-]{1,32}$/.test(id) || typeof secret !== 'string' || decode(secret).length !== 32) throw new Error('invalid_keys');
    return [id, secret];
  });
}
async function state(env: ControlEnv, value: Operation): Promise<State | null> {
  return env.DB.prepare(`SELECT a.public_key, a.counter, a.revoked, a.environment, a.key_id, i.epoch
    FROM attestations a JOIN installations i ON i.id = a.installation_id WHERE a.installation_id = ?`)
    .bind(value.installation_id).first<State>();
}
async function signChallenge(value: Challenge, env: ControlEnv): Promise<string> {
  const [id, secret] = configured(env)[0];
  const message = `c1.${id}.${base64url(bytes(JSON.stringify(value)))}`;
  return `${message}.${base64url(new Uint8Array(await crypto.subtle.sign('HMAC', await hmacKey(secret), bytes(message))))}`;
}
async function challenge(token: unknown, op: Operation, env: ControlEnv, now: number): Promise<Challenge | null> {
  if (typeof token !== 'string' || token.length > 2048) return null;
  const parts = token.split('.');
  if (parts.length !== 4 || parts[0] !== 'c1') return null;
  const key = configured(env).find(([id]) => id === parts[1]);
  if (!key || !await crypto.subtle.verify('HMAC', await hmacKey(key[1]), decode(parts[3]), bytes(parts.slice(0, 3).join('.')))) return null;
  const value = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(decode(parts[2]))) as Challenge;
  if (!object(value) || !exact(value, ['v', 'operation_digest', 'installation_id', 'key_id', 'previous_counter', 'epoch', 'nonce', 'expires_at'])
    || value.v !== 1 || value.operation_digest !== await digest(canonical(op)) || value.installation_id !== op.installation_id
    || value.key_id !== op.key_id || !integer(value.previous_counter) || !integer(value.epoch)
    || typeof value.nonce !== 'string' || !/^[A-Za-z0-9_-]{43}$/.test(value.nonce)
    || !Number.isSafeInteger(value.expires_at) || value.expires_at <= now || value.expires_at > now + 300) return null;
  return value;
}
export async function commitControl(env: ControlEnv, op: Exclude<Operation, { op: 'enroll' }>, proof: Challenge, counter: number, now: number): Promise<Grant | null> {
  const db = env.DB;
  const statements = [db.prepare(`INSERT OR REPLACE INTO control_commits
    (installation_id, key_id, previous_counter, counter, epoch, nonce, now, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)`)
    .bind(op.installation_id, op.key_id, proof.previous_counter, counter, proof.epoch, proof.nonce, now, proof.expires_at)];
  let grant: Grant | null = null;
  if (op.op === 'route') {
    const pairing = await db.prepare('SELECT installation_id, epoch, revoked FROM pairings WHERE id = ?').bind(op.pairing_id)
      .first<{ installation_id: string; epoch: number; revoked: number }>();
    if (pairing && (pairing.installation_id !== op.installation_id || pairing.revoked)) throw new Error('invalid_route');
    const prior = await db.prepare('SELECT pairing_id, generation, destination_digest, kind, expires_at, revoked FROM routes WHERE id = ?').bind(op.route_id)
      .first<{ pairing_id: string; generation: number; destination_digest: string; kind: string; expires_at: number; revoked: number }>();
    const destinationDigest = await digest(op.destination);
    const recovered = prior && prior.generation === op.expected_generation + 1 && prior.pairing_id === op.pairing_id
      && prior.destination_digest === destinationDigest && prior.kind === op.kind && !prior.revoked && prior.expires_at > now;
    if ((!recovered && (prior ? prior.generation : 0) !== op.expected_generation) || (prior && prior.pairing_id !== op.pairing_id)) throw new Error('invalid_route');
    if (!pairing) statements.push(db.prepare('INSERT INTO pairings (id, installation_id, epoch) VALUES (?, ?, 0)').bind(op.pairing_id, op.installation_id));
    const expiresAt = recovered ? prior.expires_at : now + 30 * 86400;
    // Delete and insert occur in the same guarded D1 transaction, so quota triggers check the replacement too.
    if (prior && !recovered) statements.push(db.prepare('DELETE FROM routes WHERE id = ? AND pairing_id = ? AND generation = ?').bind(op.route_id, op.pairing_id, op.expected_generation));
    if (!recovered) statements.push(db.prepare(`INSERT INTO routes (id, pairing_id, generation, destination, destination_digest, kind, environment, expires_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?)`)
      .bind(op.route_id, op.pairing_id, op.expected_generation + 1, op.destination, destinationDigest, op.kind, op.environment, expiresAt));
    statements.push(db.prepare('UPDATE attestations SET retain_until = MAX(retain_until, ?) WHERE installation_id = ?')
      .bind(expiresAt + 7 * 86400, op.installation_id));
    grant = { v: 1, route_id: op.route_id, generation: op.expected_generation + 1, installation_id: op.installation_id,
      installation_epoch: proof.epoch, pairing_id: op.pairing_id, pairing_epoch: pairing?.epoch ?? 0,
      destination_digest: destinationDigest, push_classes: op.kind === 'app' ? ['alert', 'background'] : ['liveactivity'],
      environment: op.environment, expires_at: expiresAt };
  } else if (op.op === 'revoke_pairing') {
    statements.push(db.prepare('UPDATE pairings SET epoch = epoch + 1, revoked = 1 WHERE id = ? AND installation_id = ? AND revoked = 0')
      .bind(op.pairing_id, op.installation_id));
  } else {
    statements.push(db.prepare(`UPDATE routes SET revoked = 1 WHERE id = ? AND pairing_id = ? AND generation = ?
      AND EXISTS (SELECT 1 FROM pairings WHERE id = ? AND installation_id = ?)`)
      .bind(op.route_id, op.pairing_id, op.expected_generation, op.pairing_id, op.installation_id));
  }
  await db.batch(statements);
  return grant;
}
export async function control(request: Request, env: ControlEnv, now: number,
  signer: (grant: Grant, id: string, secret: string) => Promise<string>): Promise<Response> {
  const path = new URL(request.url).pathname;
  try {
    const source = request.headers.get('cf-connecting-ip');
    if (!env.CONTROL_LIMITER || !env.ENROLL_LIMITER) return response(503, 'unavailable');
    if (!source || !(await env.CONTROL_LIMITER.limit({ key: `ip:${source}` })).success
      || !(await env.CONTROL_LIMITER.limit({ key: 'shared:control' })).success) return response(429, 'rate_limited', { retry_after: 60 });
    let body: unknown;
    try { body = await readBody(request, 5000, 16384); } catch { return response(400, 'invalid_request'); }
    if (!object(body)) return response(400, 'invalid_request');
    const op = operation(path === '/v1/challenge' ? body : body.operation);
    if (!op || op.environment !== env.APNS_ENVIRONMENT) return response(400, 'invalid_request');
    if (!(await env.CONTROL_LIMITER.limit({ key: `installation:${op.installation_id}` })).success) return response(429, 'rate_limited', { retry_after: 60 });
    if (op.op === 'enroll' && (!(await env.ENROLL_LIMITER.limit({ key: `ip:${source}` })).success
      || !(await env.ENROLL_LIMITER.limit({ key: 'shared:enrollment' })).success)) return response(429, 'rate_limited', { retry_after: 60 });
    const current = await state(env, op);
    if ((op.op !== 'enroll' && !current) || (current && (current.revoked || current.key_id !== op.key_id || current.environment !== op.environment))) return response(401, 'unauthorized');
    if (path === '/v1/challenge') {
      const value: Challenge = { v: 1, operation_digest: await digest(canonical(op)), installation_id: op.installation_id,
        key_id: op.key_id, previous_counter: current?.counter ?? 0, epoch: current?.epoch ?? 0,
        nonce: base64url(crypto.getRandomValues(new Uint8Array(32))), expires_at: now + 300 };
      return response(200, 'challenge', { challenge: await signChallenge(value, env), expires_at: value.expires_at });
    }
    if (!exact(body, ['operation', 'challenge', 'attestation', 'assertion']) || typeof body.challenge !== 'string') return response(400, 'invalid_request');
    const proof = await challenge(body.challenge, op, env, now);
    if (!proof) return response(401, 'unauthorized');
    const clientHash = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes(body.challenge)));
    if (op.op === 'enroll' && current) {
      if (typeof body.assertion !== 'string' || body.attestation !== undefined || current.counter !== proof.previous_counter
        || current.epoch !== proof.epoch) return response(401, 'unauthorized');
      let counter;
      if (!validCryptoInput(body.assertion, clientHash)) return response(401, 'unauthorized');
      counter = await compute(env).assertion(body.assertion, current.public_key, current.counter, clientHash);
      if (counter === null) return response(401, 'unauthorized');
      await env.DB.prepare(`INSERT OR REPLACE INTO control_commits
        (installation_id, key_id, previous_counter, counter, epoch, nonce, now, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)`)
        .bind(op.installation_id, op.key_id, proof.previous_counter, counter, proof.epoch, proof.nonce, now, proof.expires_at).run();
      return response(200, 'enrolled');
    }
    if (op.op === 'enroll') {
      if (typeof body.attestation !== 'string' || body.assertion !== undefined) return response(400, 'invalid_request');
      let verified;
      if (!validCryptoInput(body.attestation, clientHash) || !validKeyId(op.key_id)) return response(401, 'unauthorized');
      verified = await compute(env).attest(body.attestation, op.key_id, clientHash);
      if (!verified) return response(401, 'unauthorized');
      await env.DB.batch([
        env.DB.prepare('INSERT INTO installations (id, epoch) VALUES (?, 0)').bind(op.installation_id),
        env.DB.prepare(`INSERT INTO attestations (installation_id, key_id, public_key, receipt, environment, last_nonce, retain_until)
          VALUES (?, ?, ?, ?, ?, ?, ?)`)
          .bind(op.installation_id, op.key_id, verified.publicKey, verified.receipt, op.environment, proof.nonce, now + 37 * 86400),
        env.DB.prepare('INSERT INTO installation_usage (installation_id, day, total, routine) VALUES (?, ?, 0, 0)')
          .bind(op.installation_id, Math.floor(now / 86400))
      ]);
      return response(200, 'enrolled');
    }
    if (typeof body.assertion !== 'string' || body.attestation !== undefined || !current
      || current.counter !== proof.previous_counter || current.epoch !== proof.epoch) return response(401, 'unauthorized');
    let counter;
    if (!validCryptoInput(body.assertion, clientHash)) return response(401, 'unauthorized');
    counter = await compute(env).assertion(body.assertion, current.public_key, current.counter, clientHash);
    if (counter === null) return response(401, 'unauthorized');
    const grant = await commitControl(env, op, proof, counter, now);
    if (!grant) return response(200, 'revoked');
    const [id, secret] = configured(env)[0];
    return response(200, 'registered', { grant: await signer(grant, id, secret), route_id: grant.route_id,
      generation: grant.generation, expires_at: grant.expires_at, renew_at: grant.expires_at - 7 * 86400 });
  } catch (error) {
    const message = error instanceof Error ? error.message : '';
    if (['unauthorized_control', 'revoked_pairing', 'UNIQUE constraint failed'].some(v => message.includes(v))) return response(401, 'unauthorized');
    if (['pairing_limit', 'activity_limit', 'route_limit', 'control_quota'].some(v => message.includes(v))) return response(429, 'route_limit');
    if (message.includes('invalid_route')) return response(410, 'invalid_route');
    return response(503, 'unavailable');
  }
}
export async function cleanup(db: D1Database, now: number): Promise<void> {
  await db.batch([
    db.prepare('DELETE FROM routes WHERE id IN (SELECT id FROM routes WHERE expires_at < ? ORDER BY expires_at LIMIT 64)').bind(now - 7 * 86400),
    // 64 installations' worth of the 64 pairing tombstones each may hold; oldest retention first bounds deletion delay.
    db.prepare(`DELETE FROM pairings WHERE id IN (SELECT p.id FROM pairings p JOIN attestations a ON a.installation_id = p.installation_id
      WHERE a.retain_until < ? AND NOT EXISTS (SELECT 1 FROM routes WHERE pairing_id = p.id) ORDER BY a.retain_until LIMIT 4096)`).bind(now),
    db.prepare(`DELETE FROM installation_usage WHERE installation_id IN (SELECT a.installation_id FROM attestations a
      WHERE a.retain_until < ? AND NOT EXISTS (SELECT 1 FROM pairings WHERE installation_id = a.installation_id) ORDER BY a.retain_until LIMIT 64)`).bind(now),
    db.prepare(`DELETE FROM installations WHERE id IN (SELECT a.installation_id FROM attestations a
      WHERE a.retain_until < ? AND NOT EXISTS (SELECT 1 FROM pairings WHERE installation_id = a.installation_id)
      AND NOT EXISTS (SELECT 1 FROM installation_usage WHERE installation_id = a.installation_id) ORDER BY a.retain_until LIMIT 64)`).bind(now)
  ]);
}
