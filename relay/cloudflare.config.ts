import { bindings, defineConfig } from 'cf/config';

export default defineConfig({
  worker: {
    name: 'collie-push-validation',
    compatibilityDate: '2026-08-06',
    entrypoint: './src/index.ts',
    workersDev: false,
    previewUrls: false,
    observability: { enabled: false },
    env: {
      DB: bindings.d1({ id: '00000000-0000-0000-0000-000000000000', name: 'collie-push-validation' }),
      APNS_ENVIRONMENT: bindings.text('development'),
      APNS_TEAM_ID: bindings.secret(),
      APNS_KEY_ID: bindings.secret(),
      APNS_PRIVATE_KEY: bindings.secret(),
      SEND_HMAC_KEYS: bindings.secret(),
      SEND_LIMITER: bindings.rateLimit({ namespace: '350', simple: { limit: 60, period: 60 } })
    }
  }
});
