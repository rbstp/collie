import { bytes, decode } from './crypto.ts';

export type PushType = 'alert' | 'background' | 'liveactivity';
export type Environment = 'production' | 'development';
export interface Grant {
  v: 1;
  route_id: string;
  generation: number;
  installation_id: string;
  installation_epoch: number;
  pairing_id: string;
  pairing_epoch: number;
  destination_digest: string;
  push_classes: PushType[];
  environment: Environment;
  expires_at: number;
}
export interface Send {
  v: 1;
  push_type: PushType;
  priority: 5 | 10;
  expiration: number;
  collapse_id?: string;
  payload: Record<string, unknown>;
}
export const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
export const exact = (v: Record<string, unknown>, keys: string[]) => Object.keys(v).every(key => keys.includes(key));
const integer = (v: unknown, max = Number.MAX_SAFE_INTEGER): v is number => Number.isSafeInteger(v) && (v as number) >= 0 && (v as number) <= max;
const opaque = (v: unknown): v is string => typeof v === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(v);
const encrypted = (v: unknown): boolean => {
  if (typeof v !== 'string' || v.length > 3800) return false;
  try { return decode(v).length >= 29; } catch { return false; }
};
const fallback = (v: unknown): boolean => object(v) && exact(v, ['title', 'body']) && v.title === 'Collie'
  && ['An agent needs attention', 'An agent finished'].includes(v.body as string);
export function validGrant(v: unknown, now: number): v is Grant {
  if (!object(v) || !exact(v, ['v', 'route_id', 'generation', 'installation_id', 'installation_epoch', 'pairing_id', 'pairing_epoch', 'destination_digest', 'push_classes', 'environment', 'expires_at'])) return false;
  return v.v === 1 && opaque(v.route_id) && opaque(v.installation_id) && opaque(v.pairing_id)
    && integer(v.generation) && v.generation > 0 && integer(v.installation_epoch) && integer(v.pairing_epoch)
    && typeof v.destination_digest === 'string' && /^[A-Za-z0-9_-]{43}$/.test(v.destination_digest)
    && Array.isArray(v.push_classes) && v.push_classes.length > 0 && v.push_classes.length <= 2
    && new Set(v.push_classes).size === v.push_classes.length
    && v.push_classes.every(c => ['alert', 'background', 'liveactivity'].includes(c))
    && !(v.push_classes.includes('liveactivity') && v.push_classes.length !== 1)
    && ['production', 'development'].includes(v.environment as string)
    && integer(v.expires_at) && v.expires_at > now && v.expires_at <= now + 30 * 86400;
}
export function validSend(v: unknown): v is Send {
  if (!object(v) || !exact(v, ['v', 'push_type', 'priority', 'expiration', 'collapse_id', 'payload'])) return false;
  if (v.v !== 1 || !['alert', 'background', 'liveactivity'].includes(v.push_type as string)
    || ![5, 10].includes(v.priority as number) || !integer(v.expiration, 4294967295)
    || (v.collapse_id !== undefined && !opaque(v.collapse_id)) || !object(v.payload)
    || bytes(JSON.stringify(v.payload)).length > 4096) return false;
  const p = v.payload;
  if (!object(p.aps) || !opaque(p.node_id)) return false;
  const aps = p.aps;
  if (v.push_type === 'background') return v.priority === 5 && v.expiration === 0 && v.collapse_id === undefined
    && exact(p, ['aps', 'node_id', 'approval_id']) && opaque(p.approval_id)
    && exact(aps, ['content-available']) && aps['content-available'] === 1;
  if (v.push_type === 'alert') return exact(p, ['aps', 'node_id', 'event_id', 'approval_id', 'enc'])
    && opaque(p.event_id) && (p.approval_id === undefined || opaque(p.approval_id)) && encrypted(p.enc)
    && exact(aps, ['alert', 'mutable-content', 'sound', 'category', 'thread-id'])
    && fallback(aps.alert) && aps['mutable-content'] === 1 && aps.sound === 'default'
    && (aps.category === undefined || aps.category === 'APPROVAL')
    && opaque(aps['thread-id']);
  return v.collapse_id === undefined && exact(p, ['aps', 'node_id', 'event_id']) && opaque(p.event_id)
    && exact(aps, ['timestamp', 'event', 'content-state', 'stale-date', 'dismissal-date', 'alert'])
    && integer(aps.timestamp, 4294967295) && ['update', 'end'].includes(aps.event as string)
    && object(aps['content-state']) && exact(aps['content-state'], ['v', 'enc'])
    && aps['content-state'].v === 1 && encrypted(aps['content-state'].enc)
    && (aps['stale-date'] === undefined || integer(aps['stale-date'], 4294967295))
    && (aps['dismissal-date'] === undefined || (aps.event === 'end' && integer(aps['dismissal-date'], 4294967295)))
    && (aps.alert === undefined || fallback(aps.alert));
}
export async function readBody(request: Request, timeoutMs = 5000): Promise<unknown> {
  const length = request.headers.get('content-length');
  if (length !== null && (!/^\d+$/.test(length) || Number(length) > 8192)) throw new Error('invalid_request');
  if (request.headers.get('content-type')?.split(';')[0].trim() !== 'application/json'
    || request.headers.has('content-encoding') || !request.body) throw new Error('invalid_request');
  const reader = request.body.getReader();
  let timer: ReturnType<typeof setTimeout>;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new Error('invalid_request'));
      void reader.cancel().catch(() => {});
    }, timeoutMs);
  });
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await Promise.race([reader.read(), timeout]);
      if (done) break;
      size += value.length;
      if (size > 8192) { void reader.cancel().catch(() => {}); throw new Error('invalid_request'); }
      chunks.push(value);
    }
  } finally { clearTimeout(timer!); reader.releaseLock(); }
  const data = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { data.set(chunk, offset); offset += chunk.length; }
  return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(data));
}
