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
