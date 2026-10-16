import { decode } from './crypto.ts';
import type { VerifiedAttestation } from './attestation.ts';

export interface CryptoStub {
  attest(encoded: string, keyId: string, clientHash: Uint8Array<ArrayBuffer>): Promise<VerifiedAttestation | null>;
  assertion(encoded: string, publicKey: string, previousCounter: number, clientHash: Uint8Array<ArrayBuffer>): Promise<number | null>;
  providerToken(now: number): Promise<string>;
}
export interface CryptoBinding { CRYPTO?: { getByName(name: string): CryptoStub } }
export function compute(env: CryptoBinding): CryptoStub {
  if (!env.CRYPTO) throw new Error('crypto_unavailable');
  return env.CRYPTO.getByName('relay-crypto');
}
export function validCryptoInput(encoded: string, clientHash: Uint8Array<ArrayBuffer>): boolean {
  return typeof encoded === 'string' && encoded.length > 0 && encoded.length <= 16384
    && /^[A-Za-z0-9_-]+$/.test(encoded) && clientHash instanceof Uint8Array && clientHash.length === 32;
}
export function validKeyId(keyId: string): boolean {
  try { return typeof keyId === 'string' && keyId.length === 43 && decode(keyId).length === 32; }
  catch { return false; }
}
