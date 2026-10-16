import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { test } from 'node:test';
import { build } from 'esbuild';
import { Miniflare } from 'miniflare';
import { initialize, run } from './cloudflare-crypto-driver.ts';

async function directoryFixture(operation: (directory: string) => Promise<void>) {
  const parent = await mkdtemp(join(tmpdir(), 'collie-crypto-cycle-'));
  try { await operation(join(parent, 'private')); } finally { await rm(parent, { recursive: true, force: true }); }
}
test('one local complete cycle uses exactly42 attempts with real actor/control/send and no outbound APNs', async () => {
  await directoryFixture(async directory => {
    await initialize(directory);
    assert.equal((await stat(directory)).mode & 0o777, 0o700);
    assert.equal((await stat(join(directory, 'secrets.json'))).mode & 0o777, 0o600);
    await assert.rejects(initialize(directory), { code: 'EEXIST' });
    const secrets = JSON.parse(await readFile(join(directory, 'secrets.json'), 'utf8'));
    const bundled = await build({ entryPoints: [new URL('./cloudflare-crypto-benchmark.ts', import.meta.url).pathname], bundle: true,
      format: 'esm', platform: 'neutral', mainFields: ['module', 'main'], target: 'es2024', external: ['cloudflare:workers'], write: false });
    const runtime = new Miniflare({ name: 'crypto-benchmark', unsafeInspectDurableObjects: true, modules: true, script: bundled.outputFiles[0].text,
      compatibilityDate: '2026-08-06', d1Databases: ['DB'], durableObjects: { CRYPTO: { className: 'BenchmarkCrypto', useSQLite: true } },
      bindings: { ...secrets, APNS_TEAM_ID: 'TESTTEAM01', APNS_KEY_ID: 'TESTKEY001', APNS_ENVIRONMENT: 'development', ATTEST_BUNDLE_VERSIONS: '["1"]' },
      ratelimits: { SEND_LIMITER: { namespace_id: '350', simple: { limit: 60, period: 60 } },
        CONTROL_LIMITER: { namespace_id: '353', simple: { limit: 30, period: 60 } }, ENROLL_LIMITER: { namespace_id: '354', simple: { limit: 5, period: 60 } } } });
    try {
      const db = await runtime.getD1Database('DB');
      for (const file of ['0001_relay.sql', '0002_attestation.sql']) {
        let current = '';
        const schema = await readFile(new URL(`../migrations/${file}`, import.meta.url), 'utf8');
        for (const statement of schema.split(/;\s*(?:\n|$)/)) {
          if (!statement.trim()) continue;
          current += statement + ';\n';
          if (current.includes('CREATE TRIGGER') && !/\nEND;\s*$/.test(current)) continue;
          await db.prepare(current).run(); current = '';
        }
      }
      let calls = 0;
      await run(directory, 'https://benchmark.invalid', async (url, options) => {
        calls++;
        const headers = new Headers(options?.headers); headers.set('cf-connecting-ip', '192.0.2.1');
        return runtime.dispatchFetch(String(url), { ...options, headers }) as never;
      });
      assert.equal(calls, 42);
      assert.deepEqual(JSON.parse(await readFile(join(directory, 'attempts.json'), 'utf8')), { attempts: 42, limit: 50 });
      const log = await readFile(join(directory, 'responses.jsonl'), 'utf8');
      const rows = log.trim().split('\n').map(row => JSON.parse(row));
      assert.equal(rows.at(-1).complete, true);
      assert.equal(rows.at(-1).reserved_writes, 1000);
      assert.ok(rows.at(-1).measured_writes < 1000);
      assert.equal(log.includes(secrets.BENCHMARK_SECRET), false);
      assert.equal(log.includes('"grant"'), false);
      assert.equal(log.includes('"challenge":"'), false);
      assert.equal((await db.prepare('SELECT COUNT(*) n FROM installations').first())!.n, 0);
      assert.equal((await runtime.listDurableObjectIds('CRYPTO')).length, 1);
      const storage = await runtime.unsafeGetDurableObjectStorage('crypto-benchmark', 'BenchmarkCrypto', { name: 'relay-crypto' });
      assert.deepEqual(await storage.exec("SELECT name FROM sqlite_master WHERE name NOT LIKE '_cf_%' AND name NOT LIKE '__miniflare_%'"), []);
      await assert.rejects(run(directory, 'https://benchmark.invalid', async () => { calls++; return new Response(''); }), { code: 'EEXIST' });
      assert.equal(calls, 42);
    } finally { await runtime.dispose(); }
  });
});
test('a failed network attempt is durable, stops the cycle and prevents accidental rerun', async () => {
  await directoryFixture(async directory => {
    await initialize(directory);
    let calls = 0;
    await assert.rejects(run(directory, 'https://benchmark.invalid', async () => {
      calls++;
      if (calls <= 2) return new Response(null, { status: 404 });
      throw new Error('network failure');
    }), /network failure/);
    assert.equal(calls, 3);
    assert.deepEqual(JSON.parse(await readFile(join(directory, 'attempts.json'), 'utf8')), { attempts: 3, limit: 50 });
    await assert.rejects(run(directory, 'https://benchmark.invalid', async () => { calls++; return new Response(''); }), { code: 'EEXIST' });
    assert.equal(calls, 3);
  });
});
