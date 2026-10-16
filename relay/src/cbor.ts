export type Cbor = number | string | Uint8Array<ArrayBuffer> | Cbor[] | Map<string | number, Cbor>;
export class CborReader {
  private offset = 0;
  private nodes = 0;
  private input: Uint8Array<ArrayBuffer>;
  constructor(input: Uint8Array<ArrayBuffer>) {
    this.input = input;
    if (input.length > 16384) throw new Error('invalid_cbor');
  }
  get position(): number { return this.offset; }
  get remaining(): number { return this.input.length - this.offset; }
  private take(length: number): Uint8Array<ArrayBuffer> {
    if (this.offset + length > this.input.length) throw new Error('invalid_cbor');
    const value = this.input.slice(this.offset, this.offset + length);
    this.offset += length;
    return value;
  }
  read(depth = 0): Cbor {
    if (depth > 8 || ++this.nodes > 128) throw new Error('invalid_cbor');
    const initial = this.take(1)[0];
    const type = initial >> 5;
    const info = initial & 31;
    let length = info;
    if (info >= 24) {
      if (info > 26) throw new Error('invalid_cbor');
      const data = this.take(1 << (info - 24));
      length = data.reduce((n, v) => n * 256 + v, 0);
      if (length < (info === 24 ? 24 : info === 25 ? 256 : 65536)) throw new Error('invalid_cbor');
    }
    if (type === 0) return length;
    if (type === 1) return -1 - length;
    if (type === 2) return this.take(length);
    if (type === 3) return new TextDecoder('utf-8', { fatal: true }).decode(this.take(length));
    if (length > 32) throw new Error('invalid_cbor');
    if (type === 4) return Array.from({ length }, () => this.read(depth + 1));
    if (type === 5) {
      const result = new Map<string | number, Cbor>();
      for (let i = 0; i < length; i++) {
        const key = this.read(depth + 1);
        if ((typeof key !== 'string' && typeof key !== 'number') || result.has(key)) throw new Error('invalid_cbor');
        result.set(key, this.read(depth + 1));
      }
      return result;
    }
    throw new Error('invalid_cbor');
  }
  finish(): void { if (this.offset !== this.input.length) throw new Error('invalid_cbor'); }
}
export function cbor(input: Uint8Array<ArrayBuffer>): Cbor {
  const reader = new CborReader(input);
  const value = reader.read();
  reader.finish();
  return value;
}
export function map(value: Cbor, keys?: (string | number)[]): Map<string | number, Cbor> {
  if (!(value instanceof Map) || (keys && (value.size !== keys.length || keys.some(k => !value.has(k))))) throw new Error('invalid_cbor');
  return value;
}
export function binary(value: Cbor | undefined): Uint8Array<ArrayBuffer> {
  if (!(value instanceof Uint8Array)) throw new Error('invalid_cbor');
  return value;
}
