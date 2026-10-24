import { DurableObject } from 'cloudflare:workers';
import { bytes, ProviderToken } from '../src/crypto.ts';
import { verifyAttestation } from '../src/attestation.ts';
export { RelayCrypto } from '../src/compute.ts';

// Yield in this test bundle so overlapping admissions are deterministic even when WebCrypto completes immediately.
const sign = ProviderToken.prototype.token;
ProviderToken.prototype.token = async function(now: number) {
  await new Promise(resolve => setTimeout(resolve, 20));
  return sign.call(this, now);
};

// Apple's historical sample has a different publisher and raw client challenge.
export class HistoricalFixture extends DurableObject {
  async attest(encoded: string, invalid: boolean) {
    return verifyAttestation(encoded, 'zgSY9YSD-7TaDXssY6WlOPVS1K3Lmk-pFhlcSWE-ZV0',
      bytes(invalid ? 'invalid_server_challenge' : 'example_server_challenge'),
      { appId: '1234567890.com.example.myapp', environment: 'production', minBundleVersion: 1, validationCategories: [1] },
      Date.parse('2026-04-21T18:13:13Z') / 1000);
  }
}
export default { async fetch(request: Request, env: Record<string, DurableObjectNamespace>) {
  if (new URL(request.url).pathname !== '/test' || request.method !== 'POST') return new Response('not_found', { status: 404 });
  const body: any = await request.json();
  try {
    if (body.method === 'attest' && body.actor !== 'HISTORICAL') body.args[2] = new Uint8Array(body.args[2]);
    const stub: any = env[body.actor ?? 'CRYPTO'].getByName(body.actor === 'HISTORICAL' ? 'fixture' : 'relay-crypto');
    if (body.method === 'concurrency') return Response.json({ value: await Promise.all(Array.from({ length: 12 }, async () => {
      try { return { value: await stub.providerToken(Math.floor(Date.now() / 1000)) }; }
      catch (error) { return { error: String(error) }; }
    })) });
    return Response.json({ value: await stub[body.method](...body.args) });
  } catch (error) { return Response.json({ error: String(error) }, { status: 503 }); }
} };

