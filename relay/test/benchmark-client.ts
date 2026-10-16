import { open, unlink } from 'node:fs/promises';

export async function initializeBenchmarkBudget(counterPath: string, limit = 1000): Promise<void> {
  if (!Number.isSafeInteger(limit) || limit < 1 || limit > 1000) throw new Error('Invalid benchmark request budget');
  const lock = await open(`${counterPath}.lock`, 'wx', 0o600);
  try {
    const counter = await open(counterPath, 'wx', 0o600);
    try {
      await counter.writeFile(JSON.stringify({ attempts: 0, limit }));
      await counter.sync();
    } finally { await counter.close(); }
  } finally {
    await lock.close();
    await unlink(`${counterPath}.lock`);
  }
}

export function benchmarkClient(counterPath: string, upstream: typeof fetch = fetch): typeof fetch {
  return async (...args: Parameters<typeof fetch>) => {
    const lock = await open(`${counterPath}.lock`, 'wx', 0o600);
    try {
      const counter = await open(counterPath, 'r+');
      try {
        const encoded = await counter.readFile('utf8');
        const state = JSON.parse(encoded);
        if (!Number.isSafeInteger(state.limit) || state.limit < 1 || state.limit > 1000
          || !Number.isSafeInteger(state.attempts) || state.attempts < 0 || state.attempts >= state.limit) throw new Error('Benchmark request budget exhausted or invalid');
        await counter.truncate(0);
        await counter.write(Buffer.from(JSON.stringify({ attempts: state.attempts + 1, limit: state.limit })), 0, undefined, 0);
        await counter.sync();
      } finally { await counter.close(); }
    } finally {
      await lock.close();
      await unlink(`${counterPath}.lock`);
    }
    return upstream(args[0], { ...args[1], redirect: 'error' });
  };
}
