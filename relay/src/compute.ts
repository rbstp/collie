import { DurableObject } from 'cloudflare:workers';
import { ProviderToken } from './crypto.ts';
import { attestationPolicy } from './attestation-policy.ts';
import type { PolicyEnv } from './attestation-policy.ts';
import { validCryptoInput, validKeyId } from './compute-client.ts';
import type { VerifiedAttestation } from './attestation.ts';

interface ComputeEnv extends PolicyEnv { APNS_KEY_ID: string; APNS_PRIVATE_KEY: string }
export class RelayCrypto extends DurableObject<ComputeEnv> {
  #busy = false;
  #signing?: ProviderToken;
  async #run<T>(operation: () => Promise<T>): Promise<T> {
    if (this.#busy) throw new Error('crypto_busy');
    this.#busy = true;
    try { return await operation(); } finally { this.#busy = false; }
  }
  async attest(encoded: string, keyId: string, clientHash: Uint8Array<ArrayBuffer>): Promise<VerifiedAttestation | null> {
    if (!validCryptoInput(encoded, clientHash) || !validKeyId(keyId)) return null;
    return this.#run(async () => {
      const policy = attestationPolicy(this.env);
      const { verifyAttestation } = await import('./attestation.ts');
      try { return await verifyAttestation(encoded, keyId, clientHash, policy, Math.floor(Date.now() / 1000)); }
      catch { return null; }
    });
  }
  async assertion(encoded: string, publicKey: string, previousCounter: number, clientHash: Uint8Array<ArrayBuffer>): Promise<number | null> {
    if (!validCryptoInput(encoded, clientHash) || typeof publicKey !== 'string' || publicKey.length > 256
      || !Number.isSafeInteger(previousCounter) || previousCounter < 0 || previousCounter >= 4294967295) return null;
    return this.#run(async () => {
      const policy = attestationPolicy(this.env);
      const { verifyAssertion } = await import('./attestation.ts');
      try { return await verifyAssertion(encoded, publicKey, previousCounter, clientHash, policy); }
      catch { return null; }
    });
  }
  async providerToken(now: number): Promise<string> {
    if (!Number.isSafeInteger(now) || Math.floor(now / 2700) !== Math.floor(Date.now() / 2700000)) throw new Error('invalid_jwt_window');
    return this.#run(async () => {
      this.#signing ??= new ProviderToken(this.env.APNS_TEAM_ID, this.env.APNS_KEY_ID, this.env.APNS_PRIVATE_KEY);
      return this.#signing.token(now);
    });
  }
}
