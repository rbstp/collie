import { performance } from 'node:perf_hooks';
import { ProviderToken } from '../src/crypto.ts';
import { createWorker } from '../src/index.ts';
import { invalidate } from '../src/storage.ts';
import { fixture, now, privateKey, request } from './helpers.ts';

const summary = (values: number[]) => {
  const sorted = values.toSorted((a, b) => a - b);
  return { samples: sorted.length, median_wall_ms: sorted[Math.floor(sorted.length / 2)], p99_wall_ms: sorted[Math.ceil(sorted.length * 0.99) - 1] };
};
const key = await privateKey();
const cold: number[] = [];
const warm: number[] = [];
for (let n = 0; n < 100; n++) {
  const provider = new ProviderToken('TESTTEAM01', 'TESTKEY001', key);
  let start = performance.now();
  await provider.token(now);
  cold.push(performance.now() - start);
  start = performance.now();
  await provider.token(now);
  warm.push(performance.now() - start);
}
const f = await fixture();
const measurements: Record<string, unknown> = {};
let rowsRead = 0;
let rowsWritten = 0;
const wrap = (statement: any): any => ({
  bind(...values: unknown[]) { return wrap(statement.bind(...values)); },
  async first() { const result = await statement.all(); rowsRead += result.meta.rows_read; rowsWritten += result.meta.rows_written; return result.results[0] ?? null; },
  async run() { const result = await statement.run(); rowsRead += result.meta.rows_read; rowsWritten += result.meta.rows_written; return result; }
});
const db = { prepare(sql: string) { return wrap(f.db.prepare(sql)); } };
try {
  const worker = createWorker(async () => new Response('', { status: 200 }), () => now);
  const elapsed: number[] = [];
  for (let n = 0; n < 100; n++) {
    const req = await request(f.grant);
    const start = performance.now();
    const result = await worker.fetch(req, { ...f.env, DB: db } as never);
    if (result.status !== 200) throw new Error(`Unexpected status ${result.status}`);
    elapsed.push(performance.now() - start);
  }
  measurements.successful_send = { ...summary(elapsed), rows_read_per_send: rowsRead / 100, rows_written_per_send: rowsWritten / 100 };
  rowsRead = 0; rowsWritten = 0;
  await invalidate(db as never, f.grant);
  measurements.route_invalidation = { rows_read: rowsRead, rows_written: rowsWritten };
  const cleanup = await f.db.prepare('DELETE FROM routes WHERE id = ? AND generation = ?').bind('route', 1).run();
  measurements.route_cleanup = { rows_read: cleanup.meta.rows_read, rows_written: cleanup.meta.rows_written };
  const cleanupUsage = await f.db.prepare('DELETE FROM installation_usage WHERE installation_id = ?').bind('phone').run();
  measurements.counter_cleanup = { rows_read: cleanupUsage.meta.rows_read, rows_written: cleanupUsage.meta.rows_written };
  console.log(JSON.stringify({ runtime: 'Node wall time and local Miniflare D1; not deployed Worker CPU or billed production D1', deterministic_jwt_cold: summary(cold), deterministic_jwt_warm: summary(warm), ...measurements,
    deployment_gate: 'Await intended Free account, full App Attest verifier and genuine fixtures, remote D1 accounting, enrollment/control/cleanup reserve, and p99 Worker CPU < 8ms' }, null, 2));
} finally { await f.runtime.dispose(); }
