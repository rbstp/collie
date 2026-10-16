import { open, unlink } from 'node:fs/promises';

export function benchmarkClient(counterPath: string, upstream: typeof fetch = fetch): typeof fetch {
  return async (...args: Parameters<typeof fetch>) => {
    const lock = await open(`${counterPath}.lock`, 'wx', 0o600);
    try {
      let counter;
      let created = false;
      try { counter = await open(counterPath, 'r+'); }
      catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
        counter = await open(counterPath, 'wx+', 0o600);
        created = true;
      }
      try {
        const encoded = await counter.readFile('utf8');
        if (!encoded && !created) throw new Error('Benchmark request budget invalid');
        const state = encoded ? JSON.parse(encoded) : { attempts: 0 };
        if (!Number.isSafeInteger(state.attempts) || state.attempts < 0 || state.attempts >= 1000) throw new Error('Benchmark request budget exhausted or invalid');
        await counter.truncate(0);
        await counter.write(Buffer.from(JSON.stringify({ attempts: state.attempts + 1 })), 0, undefined, 0);
        await counter.sync();
      } finally { await counter.close(); }
    } finally {
      await lock.close();
      await unlink(`${counterPath}.lock`);
    }
    return upstream(args[0], { ...args[1], redirect: 'error' });
  };
}
