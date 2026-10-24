import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';
import { Miniflare } from 'miniflare';
import { readBody } from '../src/contract.ts';
import { createWorker } from '../src/index.ts';
import type { Env } from '../src/index.ts';

export function simulatorWorker(upstream: typeof fetch, clock?: () => number) {
  const worker = createWorker(upstream, clock);
  return {
    async fetch(request: Request, env: Env): Promise<Response> {
      if (env.APNS_ENVIRONMENT !== 'development') return new Response('sandbox_required', { status: 403 });
      const url = new URL(request.url);
      if (url.pathname === '/v1/challenge' || url.pathname === '/v1/control') {
        const body = await readBody(request.clone() as Request, 5000, 16384).catch(() => null);
        const value = body as { op?: string; operation?: { op?: string } } | null;
        const op = url.pathname === '/v1/challenge' ? value : value?.operation;
        if (op?.op === 'enroll') return new Response('local_provisioning_required', { status: 403 });
      }
      return worker.fetch(request, env);
    }
  };
}

export async function simulatorRuntime(env: Pick<Env,
  'APNS_TEAM_ID' | 'APNS_KEY_ID' | 'APNS_PRIVATE_KEY' | 'APNS_ENVIRONMENT' | 'ATTEST_MIN_BUNDLE_VERSION'>, d1Persist?: string) {
  if (env.APNS_ENVIRONMENT !== 'development') throw new Error('sandbox_required');
  const bundled = await build({ entryPoints: [new URL('./compute.ts', import.meta.url).pathname], bundle: true,
    format: 'esm', platform: 'neutral', mainFields: ['module', 'main'], target: 'es2024', external: ['cloudflare:workers'], write: false });
  return new Miniflare({ modules: true, script: bundled.outputFiles[0].text, compatibilityDate: '2026-08-06',
    durableObjects: { CRYPTO: { className: 'RelayCrypto', useSQLite: true } }, d1Databases: ['DB'], d1Persist,
    bindings: { ...env } });
}

export async function provisionSimulator(db: D1Database, identity: { installationId: string; keyId: string; publicKey: string }) {
  if (await db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='attestations'").first()) return;
  for (const migration of ['0001_relay.sql', '0002_attestation.sql']) {
    let current = '';
    for (const statement of (await readFile(new URL(`../migrations/${migration}`, import.meta.url), 'utf8')).split(/;\s*(?:\n|$)/)) {
      if (!statement.trim()) continue;
      current += `${statement};\n`;
      if (current.includes('CREATE TRIGGER') && !/\nEND;\s*$/.test(current)) continue;
      await db.prepare(current).run(); current = '';
    }
  }
  const now = Math.floor(Date.now() / 1000);
  await db.batch([
    db.prepare('INSERT INTO installations VALUES (?, 0)').bind(identity.installationId),
    db.prepare(`INSERT INTO attestations (installation_id,key_id,public_key,receipt,environment,last_nonce,retain_until)
      VALUES (?, ?, ?, 'simulator-provisioned', 'development', 'initial', ?)`).bind(identity.installationId, identity.keyId,
        identity.publicKey, now + 37 * 86400),
    db.prepare('INSERT INTO installation_usage VALUES (?, ?, 0, 0)').bind(identity.installationId, Math.floor(now / 86400))
  ]);
}
