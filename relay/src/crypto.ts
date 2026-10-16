import { p256 } from '@noble/curves/nist.js';

const encoder = new TextEncoder();
export const bytes = (value: string): Uint8Array<ArrayBuffer> => encoder.encode(value);
export function base64url(value: Uint8Array): string {
  return btoa(String.fromCharCode(...value)).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '');
}
export function decode(value: string): Uint8Array<ArrayBuffer> {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) throw new Error('invalid_encoding');
  const result = Uint8Array.from(atob(value.replaceAll('-', '+').replaceAll('_', '/')), c => c.charCodeAt(0));
  if (base64url(result) !== value) throw new Error('invalid_encoding');
  return result;
}
export async function hmacKey(secret: string): Promise<CryptoKey> {
  const key = decode(secret);
  if (key.length !== 32) throw new Error('invalid_secret');
  return crypto.subtle.importKey('raw', key, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign', 'verify']);
}
export async function digest(value: string): Promise<string> {
  return base64url(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes(value))));
}

export class ProviderToken {
  private cached?: { window: number; token: string };
  private team: string;
  private keyId: string;
  private privateKey: string;
  constructor(team: string, keyId: string, privateKey: string) {
    this.team = team; this.keyId = keyId; this.privateKey = privateKey;
  }
  async token(now: number): Promise<string> {
    const window = Math.floor(now / 2700) * 2700;
    if (this.cached?.window === window) return this.cached.token;
    if (!/^[A-Z0-9]{10}$/.test(this.team) || !/^[A-Z0-9]{10}$/.test(this.keyId)) throw new Error('invalid_apns_identity');
    const pem = this.privateKey.replace(/-----BEGIN PRIVATE KEY-----|-----END PRIVATE KEY-----|\s/g, '');
    const der = Uint8Array.from(atob(pem), c => c.charCodeAt(0));
    const imported = await crypto.subtle.importKey('pkcs8', der, { name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign']);
    const jwk = await crypto.subtle.exportKey('jwk', imported);
    if (!jwk.d) throw new Error('invalid_apns_key');
    const header = base64url(bytes(JSON.stringify({ alg: 'ES256', kid: this.keyId })));
    const payload = base64url(bytes(JSON.stringify({ iss: this.team, iat: window })));
    const message = `${header}.${payload}`;
    const signature = p256.sign(bytes(message), decode(jwk.d), { prehash: true, lowS: false });
    const token = `${message}.${base64url(signature)}`;
    this.cached = { window, token };
    return token;
  }
}
