import { base64url, bytes } from '../src/crypto.ts';

export async function signedAssertion(key: CryptoKey, challenge: string, counter: number): Promise<string> {
  const auth = new Uint8Array(37);
  auth.set(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes('TESTTEAM01.dev.rbstp.collie'))));
  new DataView(auth.buffer).setUint32(33, counter);
  const clientHash = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes(challenge)));
  const message = new Uint8Array(69); message.set(auth); message.set(clientHash, 37);
  const signature = new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, key, message));
  const integer = (part: Uint8Array) => {
    let first = 0; while (first < 31 && part[first] === 0) first++;
    const value = [...part.slice(first)]; if (value[0] & 128) value.unshift(0);
    return [2, value.length, ...value];
  };
  const parts = [...integer(signature.slice(0, 32)), ...integer(signature.slice(32))];
  const der = [0x30, parts.length, ...parts];
  return base64url(new Uint8Array([0xa2, 0x69, ...bytes('signature'), 0x58, der.length, ...der,
    0x71, ...bytes('authenticatorData'), 0x58, auth.length, ...auth]));
}
