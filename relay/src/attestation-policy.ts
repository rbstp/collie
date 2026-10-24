import type { AttestationPolicy } from './attestation.ts';
import type { Environment } from './contract.ts';

export interface PolicyEnv {
  APNS_TEAM_ID: string;
  APNS_ENVIRONMENT: Environment;
  ATTEST_MIN_BUNDLE_VERSION?: string;
}
export function attestationPolicy(env: PolicyEnv): AttestationPolicy {
  if (!/^[A-Z0-9]{10}$/.test(env.APNS_TEAM_ID)) throw new Error('invalid_app_id');
  if (!/^[1-9][0-9]{0,8}$/.test(env.ATTEST_MIN_BUNDLE_VERSION ?? '')) throw new Error('invalid_min_bundle_version');
  return { appId: `${env.APNS_TEAM_ID}.dev.rbstp.collie`, environment: env.APNS_ENVIRONMENT, minBundleVersion: Number(env.ATTEST_MIN_BUNDLE_VERSION) };
}
