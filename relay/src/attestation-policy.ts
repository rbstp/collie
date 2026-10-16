import type { AttestationPolicy } from './attestation.ts';
import type { Environment } from './contract.ts';

export interface PolicyEnv {
  APNS_TEAM_ID: string;
  APNS_ENVIRONMENT: Environment;
  ATTEST_BUNDLE_VERSIONS?: string;
}
export function attestationPolicy(env: PolicyEnv): AttestationPolicy {
  if (!/^[A-Z0-9]{10}$/.test(env.APNS_TEAM_ID)) throw new Error('invalid_app_id');
  const versions: unknown = JSON.parse(env.ATTEST_BUNDLE_VERSIONS ?? '[]');
  if (!Array.isArray(versions) || !versions.length || versions.length > 16
    || versions.some(v => typeof v !== 'string' || !/^[A-Za-z0-9.]{1,32}$/.test(v))) throw new Error('invalid_bundle_versions');
  return { appId: `${env.APNS_TEAM_ID}.dev.rbstp.collie`, environment: env.APNS_ENVIRONMENT, bundleVersions: versions };
}
