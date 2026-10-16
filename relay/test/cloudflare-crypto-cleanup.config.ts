import { defineConfig, exports } from 'cf/config';

export default defineConfig({
  accountId: 'e0d20426f1862227c675befdcc4d1e7e',
  worker: {
    name: 'collie-push-crypto-bench-20261016',
    compatibilityDate: '2026-08-06',
    entrypoint: './test/cloudflare-crypto-cleanup.ts',
    workersDev: false,
    domains: [],
    previewUrls: false,
    observability: { enabled: false },
    exports: { BenchmarkCrypto: exports.durableObject({ state: 'deleted' }) }
  }
});
