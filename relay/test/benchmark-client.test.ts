import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, unlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { benchmarkClient, initializeBenchmarkBudget } from './benchmark-client.ts';

async function fixture(run: (path: string) => Promise<void>) {
  const directory = await mkdtemp(join(tmpdir(), 'collie-request-budget-'));
  try { await run(join(directory, 'attempts.json')); } finally { await rm(directory, { recursive: true, force: true }); }
}
test('benchmark counter is shared across clients and records network failures before the attempt', async () => {
  await fixture(async path => {
    await initializeBenchmarkBudget(path);
    let calls = 0;
    const failed = benchmarkClient(path, async () => { calls++; throw new Error('network failed'); });
    await assert.rejects(failed('https://benchmark.invalid'), /network failed/);
    assert.deepEqual(JSON.parse(await readFile(path, 'utf8')), { attempts: 1 });
    const resumed = benchmarkClient(path, async (_url, init) => { calls++; assert.equal(init?.redirect, 'error'); return new Response('', { status: 503 }); });
    assert.equal((await resumed('https://benchmark.invalid')).status, 503);
    assert.equal(calls, 2);
    assert.deepEqual(JSON.parse(await readFile(path, 'utf8')), { attempts: 2 });
  });
});
test('benchmark hard cap survives resumes and rejects the next request without calling fetch', async () => {
  await fixture(async path => {
    await writeFile(path, JSON.stringify({ attempts: 999 }));
    let calls = 0;
    const first = benchmarkClient(path, async () => { calls++; return new Response(''); });
    await first('https://benchmark.invalid');
    const resumed = benchmarkClient(path, async () => { calls++; return new Response(''); });
    await assert.rejects(resumed('https://benchmark.invalid'), /budget exhausted/);
    assert.equal(calls, 1);
    assert.deepEqual(JSON.parse(await readFile(path, 'utf8')), { attempts: 1000 });
  });
});
test('corrupt state and concurrent allocation fail closed without calling fetch', async () => {
  await fixture(async path => {
    let calls = 0;
    const client = benchmarkClient(path, async () => { calls++; return new Response(''); });
    await writeFile(path, 'corrupt');
    await assert.rejects(client('https://benchmark.invalid'));
    await writeFile(path, '');
    await assert.rejects(client('https://benchmark.invalid'));
    await writeFile(path, JSON.stringify({ attempts: 0 }));
    await writeFile(`${path}.lock`, '');
    await assert.rejects(client('https://benchmark.invalid'));
    assert.equal(calls, 0);
  });
});

test('resume requires its journal and never recreates deleted state', async () => {
  await fixture(async path => {
    let calls = 0;
    const client = benchmarkClient(path, async () => { calls++; return new Response(''); });
    await assert.rejects(client('https://benchmark.invalid'), { code: 'ENOENT' });
    await assert.rejects(readFile(path), { code: 'ENOENT' });
    await initializeBenchmarkBudget(path);
    await client('https://benchmark.invalid');
    await unlink(path);
    const resumed = benchmarkClient(path, async () => { calls++; return new Response(''); });
    await assert.rejects(resumed('https://benchmark.invalid'), { code: 'ENOENT' });
    await assert.rejects(readFile(path), { code: 'ENOENT' });
    assert.equal(calls, 1);
  });
});
test('explicit first-run initialization refuses existing state and held locks', async () => {
  await fixture(async path => {
    await initializeBenchmarkBudget(path);
    await assert.rejects(initializeBenchmarkBudget(path), { code: 'EEXIST' });
    assert.deepEqual(JSON.parse(await readFile(path, 'utf8')), { attempts: 0 });
    await unlink(path);
    await writeFile(`${path}.lock`, '');
    await assert.rejects(initializeBenchmarkBudget(path), { code: 'EEXIST' });
    await assert.rejects(readFile(path), { code: 'ENOENT' });
  });
});
