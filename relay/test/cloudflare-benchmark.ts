import { bytes, decode } from '../src/crypto.ts';
import { readBody, object, exact } from '../src/contract.ts';
import { createWorker } from '../src/index.ts';
import type { Env } from '../src/index.ts';
import { cleanup } from '../src/control.ts';
import { verifyAttestation } from '../src/attestation.ts';

interface BenchmarkEnv extends Env { BENCHMARK_SECRET: string }
interface Meter { rows_read: number; rows_written: number; statements: number }
function measured(db: D1Database, meter: Meter): D1Database {
  const originals = new WeakMap<D1PreparedStatement, D1PreparedStatement>();
  const record = <T>(result: D1Result<T>): D1Result<T> => {
    meter.rows_read += result.meta.rows_read; meter.rows_written += result.meta.rows_written; meter.statements++;
    return result;
  };
  function statement(original: D1PreparedStatement): D1PreparedStatement {
    const wrapped = {
      bind(...values: unknown[]) { return statement(original.bind(...values)); },
      async first<T>(column?: string): Promise<T | null> {
        const result = record(await original.all<Record<string, unknown>>());
        const row = result.results[0]; return (row ? column ? row[column] : row : null) as T | null;
      },
      async all<T>() { return record(await original.all<T>()); },
      async run<T>() { return record(await original.run<T>()); },
      raw: original.raw.bind(original)
    } as D1PreparedStatement;
    originals.set(wrapped, original); return wrapped;
  }
  return {
    prepare(sql: string) { return statement(db.prepare(sql)); },
    async batch<T>(statements: D1PreparedStatement[]) {
      return (await db.batch<T>(statements.map(s => originals.get(s) ?? s))).map(record);
    },
    exec: db.exec.bind(db), dump: db.dump.bind(db), withSession: db.withSession.bind(db)
  } as D1Database;
}
export function createAttestationBenchmark() {
  const worker = createWorker(async () => new Response(null, { status: 200 }));
  return {
    async fetch(request: Request, env: BenchmarkEnv): Promise<Response> {
      if (!env.BENCHMARK_SECRET || env.BENCHMARK_SECRET.length < 32
        || request.headers.get('authorization') !== `Bearer ${env.BENCHMARK_SECRET}`) return new Response(null, { status: 404 });
      const meter: Meter = { rows_read: 0, rows_written: 0, statements: 0 };
      const scoped = { ...env, DB: measured(env.DB, meter) };
      const path = new URL(request.url).pathname;
      const result = (body: unknown, status = 200) => Response.json({ result: body, d1: meter }, { status, headers: { 'cache-control': 'no-store' } });
      try {
        if (path === '/benchmark/attestation' && request.method === 'POST') {
          const body = await readBody(request, 5000, 16384);
          if (!object(body) || !exact(body, ['attestation', 'invalid']) || typeof body.attestation !== 'string') return result('invalid_request', 400);
          try {
            await verifyAttestation(body.attestation, 'zgSY9YSD-7TaDXssY6WlOPVS1K3Lmk-pFhlcSWE-ZV0', bytes('example_server_challenge'),
              { appId: body.invalid ? 'TESTTEAM01.dev.rbstp.collie' : '1234567890.com.example.myapp', environment: 'production',
                bundleVersions: ['1'], validationCategories: [1] }, Date.parse('2026-04-21T18:13:13Z') / 1000);
            return result('verified');
          } catch { return result('rejected'); }
        }
        if (path === '/benchmark/seed' && request.method === 'POST') {
          const body = await readBody(request, 5000, 16384);
          if (!object(body) || !exact(body, ['key_id', 'public_key']) || typeof body.key_id !== 'string'
            || typeof body.public_key !== 'string' || decode(body.key_id).length !== 32 || body.public_key.length > 256) return result('invalid_request', 400);
          await crypto.subtle.importKey('spki', decode(body.public_key), { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);
          const now = Math.floor(Date.now() / 1000);
          await scoped.DB.batch([
            scoped.DB.prepare("INSERT INTO installations (id, epoch) VALUES ('benchmark_phone', 0)"),
            scoped.DB.prepare(`INSERT INTO attestations (installation_id, key_id, public_key, receipt, environment, last_nonce, retain_until)
              VALUES ('benchmark_phone', ?, ?, 'isolated-benchmark-only', 'development', 'initial', ?)`)
              .bind(body.key_id, body.public_key, now + 37 * 86400),
            scoped.DB.prepare("INSERT INTO installation_usage (installation_id, day, total, routine) VALUES ('benchmark_phone', ?, 0, 0)")
              .bind(Math.floor(now / 86400))
          ]);
          return result('seeded');
        }
        if (path === '/benchmark/cleanup' && request.method === 'POST') {
          await cleanup(scoped.DB, Math.floor(Date.now() / 1000) + 40 * 86400);
          return result('cleaned');
        }
        if (['/v1/challenge', '/v1/control'].includes(path)) {
          const response = await worker.fetch(request, scoped);
          return result(await response.json(), response.status);
        }
        return result('not_found', 404);
      } catch { return result('unavailable', 503); }
    }
  };
}
export default createAttestationBenchmark();
