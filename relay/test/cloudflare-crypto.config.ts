import { bindings, defineConfig, exports } from 'cf/config';

export default defineConfig({
  accountId: 'e0d20426f1862227c675befdcc4d1e7e',
  worker: {
    name: 'collie-push-crypto-bench-20261016',
    compatibilityDate: '2026-08-06',
    entrypoint: './test/cloudflare-crypto-benchmark.ts',
    workersDev: false,
    domains: ['collie-push-crypto-bench-20261016.rbstp.dev'],
    previewUrls: false,
    observability: { enabled: true, headSamplingRate: 1, redactQueryString: true,
      logs: { enabled: true, invocationLogs: true, headSamplingRate: 1, persist: true }, traces: { enabled: false } },
    exports: { BenchmarkCrypto: exports.durableObject({ storage: 'sqlite' }) },
    env: {
      CRYPTO: bindings.durableObject({ worker: 'collie-push-crypto-bench-20261016', exportName: 'BenchmarkCrypto' }),
      DB: bindings.d1({ id: '00000000-0000-0000-0000-000000000000', name: 'collie-push-crypto-bench-20261016' }),
      BENCHMARK_SECRET: bindings.secret(),
      APNS_PRIVATE_KEY: bindings.secret(),
      SEND_HMAC_KEYS: bindings.secret(),
      APNS_TEAM_ID: bindings.text('TESTTEAM01'),
      APNS_KEY_ID: bindings.text('TESTKEY001'),
      APNS_ENVIRONMENT: bindings.text('development'),
      ATTEST_BUNDLE_VERSIONS: bindings.text('["1"]'),
      SEND_LIMITER: bindings.rateLimit({ namespace: '350', simple: { limit: 60, period: 60 } }),
      CONTROL_LIMITER: bindings.rateLimit({ namespace: '353', simple: { limit: 30, period: 60 } }),
      ENROLL_LIMITER: bindings.rateLimit({ namespace: '354', simple: { limit: 5, period: 60 } })
    }
  }
});
