import { appendFile, mkdir, open, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { base64url } from '../src/crypto.ts';
import { alert } from './helpers.ts';
import { benchmarkClient, initializeBenchmarkBudget } from './benchmark-client.ts';
import { signedAssertion } from './benchmark-assertion.ts';

export async function initialize(directory: string): Promise<void> {
  await mkdir(directory, { mode: 0o700 });
  await initializeBenchmarkBudget(join(directory, 'attempts.json'), 50);
  const provider = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
  const assertion = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
  const secrets = {
    BENCHMARK_SECRET: base64url(crypto.getRandomValues(new Uint8Array(32))),
    APNS_PRIVATE_KEY: `-----BEGIN PRIVATE KEY-----\n${Buffer.from(await crypto.subtle.exportKey('pkcs8', provider.privateKey)).toString('base64')}\n-----END PRIVATE KEY-----`,
    SEND_HMAC_KEYS: JSON.stringify({ benchmark: base64url(crypto.getRandomValues(new Uint8Array(32))) })
  };
  await writeFile(join(directory, 'secrets.json'), JSON.stringify(secrets), { mode: 0o600, flag: 'wx' });
  await writeFile(join(directory, 'assertion-key.json'), JSON.stringify(await crypto.subtle.exportKey('jwk', assertion.privateKey)), { mode: 0o600, flag: 'wx' });
}
export async function run(directory: string, origin: string, upstream: typeof fetch = fetch): Promise<void> {
  if (new URL(origin).origin !== origin || !origin.startsWith('https://')) throw new Error('Expected exact HTTPS origin');
  const client = benchmarkClient(join(directory, 'attempts.json'), upstream);
  const secrets = JSON.parse(await readFile(join(directory, 'secrets.json'), 'utf8'));
  const key = await crypto.subtle.importKey('jwk', JSON.parse(await readFile(join(directory, 'assertion-key.json'), 'utf8')),
    { name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign']);
  const publicJwk = await crypto.subtle.exportKey('jwk', key); delete publicJwk.d; publicJwk.key_ops = ['verify'];
  const publicKey = await crypto.subtle.importKey('jwk', publicJwk, { name: 'ECDSA', namedCurve: 'P-256' }, true, ['verify']);
  const keyId = base64url(new Uint8Array(await crypto.subtle.digest('SHA-256', await crypto.subtle.exportKey('raw', publicKey))));
  const logPath = join(directory, 'responses.jsonl');
  const log = await open(logPath, 'wx', 0o600); await log.close();
  let reservedWrites = 40;
  let measuredWrites = 0;
  async function record(value: unknown) {
    await appendFile(logPath, JSON.stringify(value) + '\n');
    const log = await open(logPath, 'r+'); try { await log.sync(); } finally { await log.close(); }
  }
  async function post(label: string, path: string, body: unknown, expected: number, mutation = false,
    extra: Record<string, string> = {}, guard: string | null = secrets.BENCHMARK_SECRET) {
    // The fixed one-phone/one-route fixture bounds each mutation to 64 writes, including indexes and triggers.
    if (mutation) reservedWrites += 64;
    if (reservedWrites > 1000) throw new Error('D1 experiment budget exhausted');
    const start = new Date().toISOString();
    try {
      const headers = { 'content-type': 'application/json', ...extra } as Record<string, string>;
      if (guard !== null) headers.authorization = `Bearer ${guard}`;
      const response = await client(`${origin}${path}`, { method: 'POST', headers, body: JSON.stringify(body), signal: AbortSignal.timeout(15000) });
      const encoded = await response.text();
      const value = encoded ? JSON.parse(encoded) : null;
      measuredWrites += value?.d1?.rows_written ?? 0;
      const result = value?.result;
      const safeResult = result && typeof result === 'object'
        ? Object.fromEntries(Object.entries(result).filter(([name]) => !['grant', 'challenge'].includes(name))) : result;
      await record({ label, path, start, end: new Date().toISOString(), status: response.status, result: safeResult,
        d1: value?.d1, reserved_writes: reservedWrites, measured_writes: measuredWrites });
      if (response.status !== expected || measuredWrites > 1000) throw new Error('Unexpected benchmark response');
      return result;
    } catch (error) {
      await record({ label, start, end: new Date().toISOString(), error: error instanceof Error ? error.name : 'Error' });
      throw error;
    }
  }
  await post('missing_guard', '/benchmark/jwt', { cold: false }, 404, false, {}, null);
  await post('wrong_guard', '/benchmark/jwt', { cold: false }, 404, false, {}, 'wrong');
  await post('seed', '/benchmark/seed', { key_id: keyId, public_key: base64url(new Uint8Array(await crypto.subtle.exportKey('spki', publicKey))) }, 200, true);
  for (let n = 0; n < 4; n++) await post('cold_jwt', '/benchmark/jwt', { cold: true }, 200);
  for (let n = 0; n < 4; n++) await post(n === 0 ? 'provider_cache_first' : 'warm_jwt', '/benchmark/jwt', { cold: false }, 200);
  const attestation = base64url(new Uint8Array(Buffer.from(await readFile(new URL('./fixtures/apple-attestation.b64', import.meta.url), 'utf8'), 'base64')));
  for (const invalid of [false, true]) for (let n = 0; n < 6; n++) {
    const result = await post(invalid ? 'invalid_attestation' : 'valid_attestation', '/benchmark/attestation', { attestation, invalid }, 200);
    if (invalid ? result !== 'rejected' : !result?.receipt_bytes) throw new Error('Unexpected verification result');
  }
  let grant = '', challenge = '', operation: unknown;
  for (let generation = 0; generation < 4; generation++) {
    operation = { v: 1, op: 'route', installation_id: 'benchmark_phone', key_id: keyId, environment: 'development',
      pairing_id: 'benchmark_pair', route_id: 'benchmark_route', expected_generation: generation, kind: 'app', destination: 'a'.repeat(64) };
    challenge = (await post('challenge', '/v1/challenge', operation, 200)).challenge;
    const result = await post('control', '/v1/control', { operation, challenge, assertion: await signedAssertion(key, challenge, generation + 1) }, 200, true);
    if (result.result !== 'registered' || !result.grant) throw new Error('Expected registered route');
    grant = result.grant;
  }
  await post('replay', '/v1/control', { operation, challenge, assertion: await signedAssertion(key, challenge, 5) }, 401, true);
  for (let n = 0; n < 8; n++) await post(n === 0 ? 'send_first' : 'send_warm', '/v1/send', alert, 200, true, { 'x-benchmark-grant': grant });
  await post('hostile_payload', '/v1/send', { ...alert, payload: { ...alert.payload, title: 'private' } }, 400, false, { 'x-benchmark-grant': grant });
  await post('cleanup', '/benchmark/cleanup', {}, 200, true);
  await record({ complete: true, attempts: JSON.parse(await readFile(join(directory, 'attempts.json'), 'utf8')).attempts,
    reserved_writes: reservedWrites, measured_writes: measuredWrites });
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.argv[2] === 'initialize' && process.argv.length === 4) await initialize(process.argv[3]);
  else if (process.argv[2] === 'run' && process.argv.length === 5) await run(process.argv[3], process.argv[4]);
  else throw new Error('Expected initialize <new-private-directory> or run <existing-private-directory> <https-origin>');
}
