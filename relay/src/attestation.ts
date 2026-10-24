import 'reflect-metadata';
import { BasicConstraintsExtension, KeyUsagesExtension, KeyUsageFlags, X509Certificate } from '@peculiar/x509';
import * as asn from 'asn1js';
import { binary, cbor, CborReader, map } from './cbor.ts';
import type { Cbor } from './cbor.ts';
import { base64url, bytes, decode } from './crypto.ts';
import type { Environment } from './contract.ts';
import { attestationRoot, receiptRoot } from './apple-roots.ts';

const limits = { maxDepth: 16, maxNodes: 1024, maxContentLength: 16384 };
const options = { berOptions: limits };
export const sha256 = async (value: Uint8Array<ArrayBuffer>): Promise<Uint8Array<ArrayBuffer>> =>
  new Uint8Array(await crypto.subtle.digest('SHA-256', value));
export const concat = (...values: Uint8Array<ArrayBuffer>[]): Uint8Array<ArrayBuffer> => {
  const result = new Uint8Array(values.reduce((n, v) => n + v.length, 0));
  let offset = 0;
  for (const value of values) { result.set(value, offset); offset += value.length; }
  return result;
};
function equal(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && a.every((v, i) => v === b[i]);
}
function require(value: unknown): asserts value { if (!value) throw new Error('invalid_attestation'); }
function parsed(input: Uint8Array<ArrayBuffer>): asn.AsnType {
  const result = asn.fromBER(input, limits);
  require(result.offset === input.length && !result.result.error);
  return result.result;
}
function children(value: asn.AsnType): asn.AsnType[] {
  require(value instanceof asn.Sequence || value instanceof asn.Set || value instanceof asn.Constructed);
  return value.valueBlock.value;
}
function octets(value: asn.AsnType): Uint8Array<ArrayBuffer> {
  require(value instanceof asn.OctetString);
  return new Uint8Array(value.getValue());
}
function oid(value: asn.AsnType, expected: string): void {
  require(value instanceof asn.ObjectIdentifier && value.valueBlock.toString() === expected);
}
function signature(input: Uint8Array<ArrayBuffer>): Uint8Array<ArrayBuffer> {
  const parts = children(parsed(input));
  require(parts.length === 2);
  const result = new Uint8Array(64);
  parts.forEach((v, i) => {
    require(v instanceof asn.Integer);
    let value = v.valueBlock.valueHexView;
    require(value.length >= 1 && value.length <= 33 && !(value[0] & 128));
    if (value.length > 1 && value[0] === 0) { require((value[1] & 128) !== 0); value = value.slice(1); }
    require(value.length <= 32);
    result.set(value, (i + 1) * 32 - value.length);
  });
  return result;
}
function certificate(input: Uint8Array<ArrayBuffer> | string): X509Certificate {
  return new X509Certificate(input, options);
}
async function chain(leaf: X509Certificate, intermediates: X509Certificate[], root: X509Certificate, now: number): Promise<void> {
  const certificates = [leaf, ...intermediates, root];
  require(certificates.length >= 3 && certificates.length <= 4);
  for (let i = 0; i < certificates.length; i++) {
    const cert = certificates[i];
    require(cert.notBefore.getTime() <= now * 1000 && cert.notAfter.getTime() > now * 1000);
    require(new Set(cert.extensions.map(v => v.type)).size === cert.extensions.length);
    require(cert.extensions.every(v => !v.critical || ['2.5.29.19', '2.5.29.15'].includes(v.type)));
    const constraints = cert.getExtension(BasicConstraintsExtension);
    const usages = cert.getExtension(KeyUsagesExtension);
    require(constraints && usages);
    if (i === 0) require(!constraints.ca && (usages.usages & KeyUsageFlags.digitalSignature) !== 0);
    else require(constraints.ca && (usages.usages & KeyUsageFlags.keyCertSign) !== 0
      && (constraints.pathLength === undefined || constraints.pathLength >= i - 1));
    const issuer = certificates[Math.min(i + 1, certificates.length - 1)];
    require(cert.issuer === issuer.subject && await cert.verify({ publicKey: issuer.publicKey, date: new Date(now * 1000) }, crypto));
  }
}
export interface AttestationPolicy {
  appId: string;
  environment: Environment;
  minBundleVersion: number;
  validationCategories?: number[];
}
function extensions(reader: CborReader, flags: number, policy: AttestationPolicy): void {
  if ((flags & 128) !== 0 || reader.remaining > 0) {
    const values = map(reader.read(), ['apple_validation_category_01', 'apple_bundle_version_01']);
    const category = binary(values.get('apple_validation_category_01'));
    require(category.length === 4);
    const value = new DataView(category.buffer, category.byteOffset, 4).getUint32(0, true);
    require((policy.validationCategories ?? (policy.environment === 'production' ? [2, 4] : [3])).includes(value));
    const version = values.get('apple_bundle_version_01');
    require(typeof version === 'string' && /^[0-9]{1,9}$/.test(version) && Number(version) >= policy.minBundleVersion);
  }
  reader.finish();
}
async function authenticator(input: Uint8Array<ArrayBuffer>, policy: AttestationPolicy, attested: boolean): Promise<{ counter: number; reader: CborReader; flags: number }> {
  require(input.length >= 37 && equal(input.slice(0, 32), await sha256(bytes(policy.appId))));
  const flags = input[32];
  // Apple assertions can retain the AT flag without including credential data.
  require((!attested || (flags & 64) !== 0) && (flags & 0x3e) === 0);
  const counter = new DataView(input.buffer, input.byteOffset + 33, 4).getUint32(0);
  return { counter, reader: new CborReader(input.slice(37)), flags };
}
async function verifyReceipt(receipt: Uint8Array<ArrayBuffer>, leaf: X509Certificate, clientHash: Uint8Array<ArrayBuffer>, policy: AttestationPolicy, now: number): Promise<void> {
  const contentInfo = children(parsed(receipt));
  require(contentInfo.length === 2); oid(contentInfo[0], '1.2.840.113549.1.7.2');
  const explicit = children(contentInfo[1]); require(explicit.length === 1);
  const signed = children(explicit[0]); require(signed.length === 5);
  const algorithms = children(signed[1]); require(algorithms.length === 1); oid(children(algorithms[0])[0], '2.16.840.1.101.3.4.2.1');
  const encapsulated = children(signed[2]); require(encapsulated.length === 2); oid(encapsulated[0], '1.2.840.113549.1.7.1');
  const payloadWrapper = children(encapsulated[1]); require(payloadWrapper.length === 1);
  const payload = octets(payloadWrapper[0]);
  require(signed[3].idBlock.tagClass === 3 && signed[3].idBlock.tagNumber === 0);
  const certs = children(signed[3]).map(value => certificate(new Uint8Array(value.toBER(false))));
  require(certs.length >= 2 && certs.length <= 3);
  const signers = children(signed[4]); require(signers.length === 1);
  const signer = children(signers[0]); require(signer.length === 5 || signer.length === 6);
  const sid = children(signer[1]); require(sid.length === 2 && sid[1] instanceof asn.Integer);
  const serial = sid[1].valueBlock.valueHexView;
  const signingLeaf = certs.find(value => value.serialNumber.toLowerCase() === Array.from(serial, v => v.toString(16).padStart(2, '0')).join(''));
  require(signingLeaf);
  const remaining = certs.filter(value => value !== signingLeaf && value.subject !== certificate(receiptRoot).subject);
  require(remaining.length === 1 && new Uint8Array(sid[0].toBER(false)).length > 0);
  require(equal(new Uint8Array(sid[0].toBER(false)), new Uint8Array(signingLeaf.issuerName.toArrayBuffer())));
  await chain(signingLeaf, remaining, certificate(receiptRoot), now);
  oid(children(signer[2])[0], '2.16.840.1.101.3.4.2.1');
  let message = payload;
  let index = 3;
  if (signer.length === 6) {
    require(signer[3].idBlock.tagClass === 3 && signer[3].idBlock.tagNumber === 0);
    const attrs = children(signer[3]); require(attrs.length === 2);
    let content = false, digest = false;
    for (const attr of attrs) {
      const parts = children(attr); require(parts.length === 2 && parts[0] instanceof asn.ObjectIdentifier);
      const values = children(parts[1]); require(values.length === 1);
      const id = parts[0].valueBlock.toString();
      if (id === '1.2.840.113549.1.9.3') { require(!content); oid(values[0], '1.2.840.113549.1.7.1'); content = true; }
      else { require(id === '1.2.840.113549.1.9.4' && !digest && equal(octets(values[0]), await sha256(payload))); digest = true; }
    }
    require(content && digest);
    message = new Uint8Array(signer[3].valueBeforeDecodeView); message[0] = 0x31;
    index++;
  }
  oid(children(signer[index])[0], '1.2.840.10045.4.3.2');
  const key = await signingLeaf.publicKey.export({ name: 'ECDSA', namedCurve: 'P-256' }, ['verify'], crypto);
  require(await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, key, signature(octets(signer[index + 1])), message));
  const fields = new Map<number, Uint8Array<ArrayBuffer>>();
  for (const item of children(parsed(payload))) {
    const parts = children(item); require(parts.length === 3 && parts[0] instanceof asn.Integer && parts[1] instanceof asn.Integer);
    const field = parts[0].valueBlock.valueDec;
    require(!fields.has(field) && parts[1].valueBlock.valueDec === 1); fields.set(field, octets(parts[2]));
  }
  const text = (n: number) => new TextDecoder('utf-8', { fatal: true }).decode(fields.get(n));
  require(fields.has(2) && text(2) === policy.appId && fields.has(3) && equal(fields.get(3)!, new Uint8Array(leaf.rawData)));
  require(fields.has(4) && equal(fields.get(4)!, clientHash) && text(6) === 'ATTEST');
  const creation = Date.parse(text(12)) / 1000;
  require(Number.isFinite(creation) && creation <= now + 30 && creation > now - 300);
}
export interface VerifiedAttestation { publicKey: string; receipt: string }
export async function verifyAttestation(encoded: string, keyId: string, clientHash: Uint8Array<ArrayBuffer>, policy: AttestationPolicy, now: number): Promise<VerifiedAttestation> {
  const value = map(cbor(decode(encoded)), ['fmt', 'attStmt', 'authData']);
  require(value.get('fmt') === 'apple-appattest');
  const statement = map(value.get('attStmt')!, ['x5c', 'receipt']);
  const chainData = statement.get('x5c'); require(Array.isArray(chainData) && chainData.length === 2);
  const leaf = certificate(binary(chainData[0]));
  await chain(leaf, [certificate(binary(chainData[1]))], certificate(attestationRoot), now);
  const authData = binary(value.get('authData'));
  const auth = await authenticator(authData, policy, true); require(auth.counter === 0);
  require(authData.length >= 55);
  const aaguid = policy.environment === 'production' ? concat(bytes('appattest'), new Uint8Array(7)) : bytes('appattestdevelop');
  require(equal(authData.slice(37, 53), aaguid));
  const credentialLength = new DataView(authData.buffer, authData.byteOffset + 53, 2).getUint16(0);
  require(credentialLength === 32 && equal(authData.slice(55, 87), decode(keyId)));
  const reader = new CborReader(authData.slice(87));
  const cose = map(reader.read(), [1, 3, -1, -2, -3]);
  require(cose.get(1) === 2 && cose.get(3) === -7 && cose.get(-1) === 1);
  const x = binary(cose.get(-2)), y = binary(cose.get(-3)); require(x.length === 32 && y.length === 32);
  const raw = concat(new Uint8Array([4]), x, y);
  require(equal(await sha256(raw), decode(keyId)));
  const key = await leaf.publicKey.export({ name: 'ECDSA', namedCurve: 'P-256' }, ['verify'], crypto);
  require(equal(new Uint8Array(await crypto.subtle.exportKey('raw', key)), raw));
  const nonceExtension = leaf.getExtension('1.2.840.113635.100.8.2'); require(nonceExtension);
  const sequence = children(parsed(new Uint8Array(nonceExtension.value))); require(sequence.length === 1);
  const nonce = children(sequence[0]); require(nonce.length === 1 && equal(octets(nonce[0]), await sha256(concat(authData, clientHash))));
  extensions(reader, auth.flags, policy);
  const receipt = binary(statement.get('receipt')); require(receipt.length <= 12000);
  await verifyReceipt(receipt, leaf, clientHash, policy, now);
  return { publicKey: base64url(new Uint8Array(await crypto.subtle.exportKey('spki', key))), receipt: base64url(receipt) };
}
export async function verifyAssertion(encoded: string, publicKey: string, previousCounter: number, clientHash: Uint8Array<ArrayBuffer>, policy: AttestationPolicy): Promise<number> {
  const value = map(cbor(decode(encoded)), ['signature', 'authenticatorData']);
  const authData = binary(value.get('authenticatorData'));
  const auth = await authenticator(authData, policy, false);
  require(auth.counter > previousCounter);
  extensions(auth.reader, auth.flags, policy);
  const key = await crypto.subtle.importKey('spki', decode(publicKey), { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);
  require(await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, key, signature(binary(value.get('signature'))), await sha256(concat(authData, clientHash))));
  return auth.counter;
}
