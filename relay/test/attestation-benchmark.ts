import { readFile } from 'node:fs/promises';
import { performance } from 'node:perf_hooks';
import { base64url, bytes } from '../src/crypto.ts';
import { verifyAttestation } from '../src/attestation.ts';

const encoded = base64url(new Uint8Array(Buffer.from(await readFile(new URL('./fixtures/apple-attestation.b64', import.meta.url), 'utf8'), 'base64')));
const policy = { appId: '1234567890.com.example.myapp', environment: 'production' as const,
  minBundleVersion: 1, validationCategories: [1] };
const samples: number[] = [];
for (let i = 0; i < 100; i++) {
  const start = performance.now();
  await verifyAttestation(encoded, 'zgSY9YSD-7TaDXssY6WlOPVS1K3Lmk-pFhlcSWE-ZV0', bytes('example_server_challenge'),
    policy, Date.parse('2026-04-21T18:13:13Z') / 1000);
  samples.push(performance.now() - start);
}
const first = samples[0];
samples.sort((a, b) => a - b);
console.log(JSON.stringify({ measurement: 'Local Node wall time, not deployed Worker CPU', samples: samples.length,
  first_ms: first, median_ms: samples[49], p99_ms: samples[98] }));
