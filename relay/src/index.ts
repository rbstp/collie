import { base64url, bytes, decode, hmacKey } from './crypto.ts';
import { readBody, validGrant, validSend } from './contract.ts';
import type { Environment, Grant } from './contract.ts';
import { compute } from './compute-client.ts';
import type { CryptoBinding } from './compute-client.ts';
import { charge, invalidate, lookupRoute } from './storage.ts';

export interface Env extends CryptoBinding {
  DB: D1Database;
  SEND_LIMITER: { limit(options: { key: string }): Promise<{ success: boolean }> };
  APNS_ENVIRONMENT: Environment;
  SEND_HMAC_KEYS: string;
  APNS_TEAM_ID: string;
  APNS_KEY_ID: string;
  APNS_PRIVATE_KEY: string;
  ATTEST_MIN_BUNDLE_VERSION?: string;
  CONTROL_LIMITER?: Env['SEND_LIMITER'];
  ENROLL_LIMITER?: Env['SEND_LIMITER'];
}
const response = (status: number, result: string, extra: Record<string, unknown> = {}): Response =>
  Response.json({ v: 1, result, ...extra }, { status, headers: { 'cache-control': 'no-store' } });
async function bounded<T>(operation: Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout>;
  const timeout = new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('timeout')), 5000); });
  try { return await Promise.race([operation, timeout]); } finally { clearTimeout(timer!); }
}
async function authenticate(request: Request, keys: Map<string, Promise<CryptoKey>>, now: number): Promise<Grant | null> {
  const authorization = request.headers.get('authorization');
  if (!authorization || authorization.length > 2048 || !authorization.startsWith('Bearer v1.')) return null;
  const parts = authorization.slice(7).split('.');
  if (parts.length !== 4 || !/^[A-Za-z0-9_-]{1,32}$/.test(parts[1])) return null;
  const key = keys.get(parts[1]);
  if (!key) return null;
  try {
    const signature = decode(parts[3]);
    if (signature.length !== 32 || !await crypto.subtle.verify('HMAC', await key, signature, bytes(parts.slice(0, 3).join('.')))) return null;
    const grant: unknown = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(decode(parts[2])));
    return validGrant(grant, now) ? grant : null;
  } catch { return null; }
}
export async function signGrant(grant: Grant, keyId: string, secret: string): Promise<string> {
  if (!/^[A-Za-z0-9_-]{1,32}$/.test(keyId)) throw new Error('invalid_key_id');
  const message = `v1.${keyId}.${base64url(bytes(JSON.stringify(grant)))}`;
  return `${message}.${base64url(new Uint8Array(await crypto.subtle.sign('HMAC', await hmacKey(secret), bytes(message))))}`;
}
export function createWorker(upstream: typeof fetch = fetch, clock: () => number = () => Math.floor(Date.now() / 1000)) {
  let signing: { window: number; token: string } | undefined;
  let keys: Map<string, Promise<CryptoKey>> | undefined;
  return {
    async scheduled(_event: ScheduledEvent, env: Env): Promise<void> {
      const { cleanup } = await import('./control.ts');
      await cleanup(env.DB, clock());
    },
    async fetch(request: Request, env: Env): Promise<Response> {
      const url = new URL(request.url);
      if (['/v1/challenge', '/v1/control'].includes(url.pathname) && !url.search && request.method === 'POST') {
        try {
          const { control } = await import('./control.ts');
          return await bounded(control(request, env, clock(), signGrant));
        } catch { return response(503, 'unavailable'); }
      }
      if (url.pathname !== '/v1/send' || url.search || request.method !== 'POST') return response(404, 'not_found');
      try {
        let now = clock();
        const source = request.headers.get('cf-connecting-ip');
        if (source && !(await env.SEND_LIMITER.limit({ key: `ip:${source}` })).success) return response(429, 'rate_limited', { retry_after: 60 });
        if (!keys) {
          const configured = JSON.parse(env.SEND_HMAC_KEYS) as Record<string, string>;
          if (Object.keys(configured).length < 1 || Object.keys(configured).length > 2) throw new Error('invalid_keys');
          keys = new Map(Object.entries(configured).map(([id, secret]) => [id, hmacKey(secret)]));
          await Promise.all(keys.values());
        }
        const grant = await authenticate(request, keys, now);
        if (!grant || grant.environment !== env.APNS_ENVIRONMENT) return response(401, 'unauthorized');
        if (!(await env.SEND_LIMITER.limit({ key: `installation:${grant.installation_id}` })).success
          || !(await env.SEND_LIMITER.limit({ key: `route:${grant.route_id}:${grant.generation}` })).success) return response(429, 'rate_limited', { retry_after: 60 });
        let body: unknown;
        try { body = await readBody(request); } catch { return response(400, 'invalid_request'); }
        if (!validSend(body)) return response(400, 'invalid_request');
        if (!grant.push_classes.includes(body.push_type)) return response(401, 'unauthorized');
        now = clock();
        if (!validGrant(grant, now)) return response(401, 'unauthorized');
        const route = await bounded(lookupRoute(env.DB, grant, now));
        if (!route || !/^[a-f0-9]{64,512}$/.test(route.destination)
          || ((route.kind === 'liveactivity') !== (body.push_type === 'liveactivity'))) return response(410, 'invalid_route');
        const aps = body.payload.aps as Record<string, unknown>;
        const routine = body.push_type === 'liveactivity' && body.priority === 5 && aps.event === 'update' && aps.alert === undefined;
        now = clock();
        if (!env.CRYPTO) throw new Error('crypto_unavailable');
        if (signing?.window !== Math.floor(now / 2700)) signing = { window: Math.floor(now / 2700), token: await bounded(compute(env).providerToken(now)) };
        if (Math.floor(clock() / 2700) !== signing.window) {
          now = clock();
          signing = { window: Math.floor(now / 2700), token: await bounded(compute(env).providerToken(now)) };
        }
        const jwt = signing.token;
        now = clock();
        if (!validGrant(grant, now)) return response(401, 'unauthorized');
        let charged = await bounded(charge(env.DB, grant, now, routine));
        if (charged === 'daily_quota' && Math.floor(clock() / 86400) !== Math.floor(now / 86400)) {
          now = clock();
          if (!validGrant(grant, now)) return response(401, 'unauthorized');
          charged = await bounded(charge(env.DB, grant, now, routine));
        }
        if (charged === 'invalid_route') return response(410, 'invalid_route');
        if (charged === 'unavailable') return response(503, 'unavailable');
        if (charged === 'daily_quota') return response(429, 'daily_quota', { reset_at: (Math.floor(now / 86400) + 1) * 86400 });
        now = clock();
        if (!validGrant(grant, now)) return response(401, 'unauthorized');
        if (route.expires_at <= now) return response(410, 'invalid_route');
        const host = grant.environment === 'production' ? 'api.push.apple.com' : 'api.sandbox.push.apple.com';
        const headers: Record<string, string> = {
          authorization: `bearer ${jwt}`, 'content-type': 'application/json',
          'apns-topic': `dev.rbstp.collie${body.push_type === 'liveactivity' ? '.push-type.liveactivity' : ''}`,
          'apns-push-type': body.push_type, 'apns-priority': String(body.priority), 'apns-expiration': String(body.expiration)
        };
        if (body.collapse_id !== undefined) headers['apns-collapse-id'] = body.collapse_id;
        let result: Response;
        try {
          result = await upstream(`https://${host}/3/device/${route.destination}`, {
            method: 'POST', headers, body: JSON.stringify(body.payload), redirect: 'manual', signal: AbortSignal.timeout(10000)
          });
        } catch { return response(503, 'upstream_unavailable', { retry_after: 60 }); }
        if (result.status === 200) { void result.body?.cancel().catch(() => {}); return response(200, 'accepted'); }
        let reason: unknown;
        try {
          const reader = result.body?.getReader();
          if (reader) {
            let size = 0;
            let text = '';
            const decoder = new TextDecoder();
            try {
              while (true) {
                const chunk = await reader.read();
                if (chunk.done) break;
                size += chunk.value.length;
                if (size > 1024) break;
                text += decoder.decode(chunk.value, { stream: true });
              }
              reason = JSON.parse(text).reason;
            } finally { void reader.cancel().catch(() => {}); reader.releaseLock(); }
          }
        } catch { reason = undefined; }
        if ((result.status === 410 && (reason === 'Unregistered' || reason === 'ExpiredToken')) || (result.status === 400 && reason === 'BadDeviceToken')) {
          await bounded(invalidate(env.DB, grant));
          return response(410, 'invalid_route');
        }
        if (result.status >= 500) return response(503, 'upstream_unavailable', { retry_after: 900 });
        if (result.status === 429) return response(503, 'upstream_unavailable', { retry_after: 60 });
        return response(502, 'configuration_error');
      } catch { return response(503, 'unavailable'); }
    }
  };
}
export default createWorker();
