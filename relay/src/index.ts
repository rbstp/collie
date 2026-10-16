import { base64url, bytes, decode, hmacKey, ProviderToken } from './crypto.ts';
import { readBody, validGrant, validSend } from './contract.ts';
import type { Environment, Grant } from './contract.ts';
import { charge, invalidate, lookupRoute } from './storage.ts';

export interface Env {
  DB: D1Database;
  SEND_LIMITER: { limit(options: { key: string }): Promise<{ success: boolean }> };
  APNS_ENVIRONMENT: Environment;
  SEND_HMAC_KEYS: string;
  APNS_TEAM_ID: string;
  APNS_KEY_ID: string;
  APNS_PRIVATE_KEY: string;
}
const response = (status: number, result: string, extra: Record<string, unknown> = {}): Response =>
  Response.json({ v: 1, result, ...extra }, { status, headers: { 'cache-control': 'no-store' } });
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
  let signing: ProviderToken | undefined;
  let keys: Map<string, Promise<CryptoKey>> | undefined;
  return {
    async fetch(request: Request, env: Env): Promise<Response> {
      const url = new URL(request.url);
      if (url.pathname !== '/v1/send' || url.search || request.method !== 'POST') return response(404, 'not_found');
      try {
        const now = clock();
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
        const route = await lookupRoute(env.DB, grant, now);
        if (!route || !/^[a-f0-9]{64,512}$/.test(route.destination)
          || ((route.kind === 'liveactivity') !== (body.push_type === 'liveactivity'))) return response(410, 'invalid_route');
        const aps = body.payload.aps as Record<string, unknown>;
        const routine = body.push_type === 'liveactivity' && aps.event === 'update' && aps.alert === undefined;
        signing ??= new ProviderToken(env.APNS_TEAM_ID, env.APNS_KEY_ID, env.APNS_PRIVATE_KEY);
        const jwt = await signing.token(now);
        if (!await charge(env.DB, grant, now, routine)) return response(429, 'daily_quota', { reset_at: (Math.floor(now / 86400) + 1) * 86400 });
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
        if (result.status === 200) { await result.body?.cancel(); return response(200, 'accepted'); }
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
            } finally { await reader.cancel(); reader.releaseLock(); }
          }
        } catch { reason = undefined; }
        if ((result.status === 410 && reason === 'Unregistered') || (result.status === 400 && reason === 'BadDeviceToken')) {
          await invalidate(env.DB, grant);
          return response(410, 'invalid_route');
        }
        if (result.status === 429 || result.status >= 500) return response(503, 'upstream_unavailable', { retry_after: 60 });
        return response(502, 'configuration_error');
      } catch { return response(503, 'unavailable'); }
    }
  };
}
export default createWorker();
