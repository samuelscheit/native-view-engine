/**
 * Bounds-checked little-endian `BinReader` / `BinWriter` plus the compact tagged `NveBinValue`
 * format (byte-identical to Rust `nve_abi::encode_bin_value`).
 *
 * Wire layout (little-endian): Null `[0]` | Bool `[1, 0|1]` | I64 `[2, i64]` | F64 `[3, f64]` |
 * Str `[4, len u32, utf8]` | Bytes `[5, len u32, raw]` | Array `[6, payload u32, count u32, ...]` |
 * Map `[7, payload u32, count u32, (klen u32, key, value)...]`; `payload` counts every byte after
 * the payload field so containers skip in O(1).
 *
 * 64-bit integers stay exact: `i64` reads as `bigint` (`readI64`) or `(lo, hi)` halves
 * (`readI64Bits`); a 64-bit value is never squeezed through `Number(...)`.
 */
import { Key64 } from './key64.ts';

/** `NveBinValue` tag (Rust `nve_abi::NveValueTag`). */
export const BinTag = { Null: 0, Bool: 1, I64: 2, F64: 3, Str: 4, Bytes: 5, Array: 6, Map: 7 } as const;
export type BinTag = (typeof BinTag)[keyof typeof BinTag];

/** Maximum container nesting depth (Rust `NVE_BIN_VALUE_MAX_DEPTH`). */
export const BIN_VALUE_MAX_DEPTH = 64;

export type BinErrorKind =
  | 'truncated'
  | 'unexpectedTag'
  | 'invalidTag'
  | 'invalidBool'
  | 'invalidUtf8'
  | 'tooLarge'
  | 'maxDepth'
  | 'trailingBytes';

export class BinCodecError extends Error {
  readonly kind: BinErrorKind;

  constructor(kind: BinErrorKind, detail: string) {
    super(`${kind}: ${detail}`);
    this.name = 'BinCodecError';
    this.kind = kind;
  }
}

/** Strict like Rust `str::from_utf8`; `ignoreBOM` keeps a leading U+FEFF instead of stripping it. */
export const UTF8_DECODER = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
const UTF8_ENCODER = new TextEncoder();

/** Zero-copy cursor over `bytes`; byte slices are `subarray` views. */
export class BinReader {
  readonly bytes: Uint8Array;
  private readonly view: DataView;
  private pos = 0;

  constructor(bytes: Uint8Array) {
    this.bytes = bytes;
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  get position(): number {
    return this.pos;
  }

  get remaining(): number {
    return this.bytes.length - this.pos;
  }

  get atEnd(): boolean {
    return this.pos === this.bytes.length;
  }

  // ---- raw little-endian primitives ---------------------------------------------------------

  u8(): number {
    return this.view.getUint8(this.take(1));
  }

  u16(): number {
    return this.view.getUint16(this.take(2), true);
  }

  u32(): number {
    return this.view.getUint32(this.take(4), true);
  }

  i32(): number {
    return this.view.getInt32(this.take(4), true);
  }

  f32(): number {
    return this.view.getFloat32(this.take(4), true);
  }

  f64(): number {
    return this.view.getFloat64(this.take(8), true);
  }

  /** Raw 64 bits as exact halves. */
  u64(): Key64 {
    const at = this.take(8);
    return new Key64(this.view.getUint32(at, true), this.view.getUint32(at + 4, true));
  }

  /** View of the next `count` bytes (no copy). */
  bytesView(count: number): Uint8Array {
    const at = this.take(count);
    return this.bytes.subarray(at, at + count);
  }

  /** Next `count` bytes decoded as strictly validated UTF-8. */
  utf8(count: number): string {
    const at = this.take(count);
    try {
      return UTF8_DECODER.decode(this.bytes.subarray(at, at + count));
    } catch {
      throw new BinCodecError('invalidUtf8', `at ${at}`);
    }
  }

  // ---- tagged NveBinValue ------------------------------------------------------------------

  peekTag(): BinTag {
    if (this.pos >= this.bytes.length) throw this.truncated(1);
    const raw = this.bytes[this.pos]!;
    if (raw > BinTag.Map) throw new BinCodecError('invalidTag', `tag ${raw}`);
    return raw as BinTag;
  }

  /** Consumes a null and returns `true`; leaves any other value in place. */
  readNull(): boolean {
    if (this.peekTag() !== BinTag.Null) return false;
    this.pos += 1;
    return true;
  }

  readBool(): boolean {
    this.expect(BinTag.Bool);
    const b = this.u8();
    if (b > 1) throw new BinCodecError('invalidBool', `byte ${b}`);
    return b === 1;
  }

  /** Exact two's-complement halves of an `i64`. */
  readI64Bits(): Key64 {
    this.expect(BinTag.I64);
    return this.u64();
  }

  /** Exact `i64`. */
  readI64(): bigint {
    return this.readI64Bits().toSignedBigInt();
  }

  /** `f64`, or an `i64` widened to the nearest double (Rust `BinValueView::as_number`). */
  readF64(): number {
    if (this.peekTag() === BinTag.I64) return this.readI64Bits().toSignedNumber();
    this.expect(BinTag.F64);
    return this.f64();
  }

  readStr(): string {
    this.expect(BinTag.Str);
    return this.utf8(this.length());
  }

  readBytes(): Uint8Array {
    this.expect(BinTag.Bytes);
    return this.bytesView(this.length());
  }

  /** Element count of an array; rejects counts its payload cannot hold. */
  beginArray(): number {
    return this.openContainer(BinTag.Array, 1)[0];
  }

  /** Entry count of a map; read each entry as `readMapKey()` then a value. */
  beginMap(): number {
    return this.openContainer(BinTag.Map, 5)[0];
  }

  readMapKey(): string {
    return this.utf8(this.length());
  }

  /** Skips one value in O(1). */
  skipValue(): void {
    const tag = this.peekTag();
    this.pos += 1;
    switch (tag) {
      case BinTag.Null:
        return;
      case BinTag.Bool:
        this.take(1);
        return;
      case BinTag.I64:
      case BinTag.F64:
        this.take(8);
        return;
      default:
        this.take(this.length());
    }
  }

  /** View of the next complete encoded value. */
  readRawValue(): Uint8Array {
    const start = this.pos;
    this.skipValue();
    return this.bytes.subarray(start, this.pos);
  }

  /** Fully validates and consumes the next value (Rust `BinValueView::parse` rules). */
  checkValue(): void {
    this.checkAt(0);
  }

  /** Throws unless `bytes` holds exactly one valid `NveBinValue`. */
  static validate(bytes: Uint8Array): void {
    const r = new BinReader(bytes);
    r.checkValue();
    if (!r.atEnd) throw new BinCodecError('trailingBytes', `${r.remaining} bytes`);
  }

  private take(count: number): number {
    if (count < 0 || count > this.bytes.length - this.pos) throw this.truncated(count);
    const at = this.pos;
    this.pos += count;
    return at;
  }

  private length(): number {
    const len = this.u32();
    if (len > this.remaining) throw this.truncated(len);
    return len;
  }

  private truncated(need: number): BinCodecError {
    return new BinCodecError('truncated', `need ${need} at ${this.pos}, have ${this.bytes.length - this.pos}`);
  }

  private expect(tag: BinTag): void {
    const actual = this.peekTag();
    if (actual !== tag) throw new BinCodecError('unexpectedTag', `expected ${tag}, got ${actual}`);
    this.pos += 1;
  }

  /** `[count, end]` of a container. */
  private openContainer(tag: BinTag, minEntryBytes: number): [number, number] {
    this.expect(tag);
    const payload = this.length();
    if (payload < 4) throw this.truncated(4);
    const end = this.pos + payload;
    const count = this.u32();
    if (count > Math.floor((payload - 4) / minEntryBytes)) throw this.truncated(count);
    return [count, end];
  }

  private checkAt(depth: number): void {
    const tag = this.peekTag();
    switch (tag) {
      case BinTag.Null:
      case BinTag.I64:
      case BinTag.F64:
        this.skipValue();
        return;
      case BinTag.Bool:
        this.readBool();
        return;
      case BinTag.Str:
        this.readStr();
        return;
      case BinTag.Bytes:
        this.readBytes();
        return;
      default: {
        if (depth >= BIN_VALUE_MAX_DEPTH) throw new BinCodecError('maxDepth', `depth ${depth}`);
        const isMap = tag === BinTag.Map;
        const [count, end] = this.openContainer(tag, isMap ? 5 : 1);
        for (let i = 0; i < count; i++) {
          if (isMap) this.readMapKey();
          this.checkAt(depth + 1);
        }
        if (this.pos > end) throw this.truncated(this.pos - end);
        if (this.pos < end) throw new BinCodecError('trailingBytes', `${end - this.pos} bytes`);
      }
    }
  }
}

/** Little-endian writer; reuse one instance with `clear()` (keeps capacity). */
export class BinWriter {
  private buf: Uint8Array;
  private view: DataView;
  private len = 0;

  constructor(capacity = 64) {
    this.buf = new Uint8Array(Math.max(16, capacity));
    this.view = new DataView(this.buf.buffer);
  }

  get length(): number {
    return this.len;
  }

  /** View of the written bytes (valid until the next write). */
  get bytes(): Uint8Array {
    return this.buf.subarray(0, this.len);
  }

  /** Copy of the written bytes. */
  toBytes(): Uint8Array {
    return this.buf.slice(0, this.len);
  }

  clear(): void {
    this.len = 0;
  }

  // ---- raw little-endian primitives ---------------------------------------------------------

  u8(v: number): void {
    this.ensure(1);
    this.buf[this.len++] = v & 0xff;
  }

  u16(v: number): void {
    this.ensure(2);
    this.view.setUint16(this.len, v & 0xffff, true);
    this.len += 2;
  }

  u32(v: number): void {
    this.ensure(4);
    this.view.setUint32(this.len, v >>> 0, true);
    this.len += 4;
  }

  i32(v: number): void {
    this.u32(v | 0);
  }

  /** 64 raw bits from exact halves. */
  u64(v: Key64): void {
    this.u32(v.lo);
    this.u32(v.hi);
  }

  f32(v: number): void {
    this.ensure(4);
    this.view.setFloat32(this.len, v, true);
    this.len += 4;
  }

  f64(v: number): void {
    this.ensure(8);
    this.view.setFloat64(this.len, v, true);
    this.len += 8;
  }

  raw(b: Uint8Array): void {
    this.ensure(b.length);
    this.buf.set(b, this.len);
    this.len += b.length;
  }

  // ---- tagged NveBinValue ------------------------------------------------------------------

  writeNull(): void {
    this.u8(BinTag.Null);
  }

  writeBool(v: boolean): void {
    this.u8(BinTag.Bool);
    this.u8(v ? 1 : 0);
  }

  /** Exact `i64` (wraps modulo 2^64 like Rust `as i64`). */
  writeI64(v: bigint): void {
    this.writeI64Bits(Key64.fromBigInt(v));
  }

  writeI64Bits(bits: Key64): void {
    this.u8(BinTag.I64);
    this.u64(bits);
  }

  writeF64(v: number): void {
    this.u8(BinTag.F64);
    this.f64(v);
  }

  writeStr(v: string): void {
    this.u8(BinTag.Str);
    this.writeMapKey(v);
  }

  writeBytes(v: Uint8Array): void {
    this.u8(BinTag.Bytes);
    this.u32(v.length);
    this.raw(v);
  }

  writeMapKey(key: string): void {
    const encoded = UTF8_ENCODER.encode(key);
    this.u32(encoded.length);
    this.raw(encoded);
  }

  /** Opens an array of `count` values; returns the mark for `end()`. */
  beginArray(count: number): number {
    return this.begin(BinTag.Array, count);
  }

  /** Opens a map of `count` entries (`writeMapKey` + value each); returns the mark for `end()`. */
  beginMap(count: number): number {
    return this.begin(BinTag.Map, count);
  }

  /** Patches the payload length of the container opened at `mark`. */
  end(mark: number): void {
    if (!Number.isInteger(mark) || mark < 0 || mark + 4 > this.len) throw new RangeError(`mark ${mark}`);
    this.view.setUint32(mark, this.len - mark - 4, true);
  }

  private begin(tag: BinTag, count: number): number {
    this.u8(tag);
    const mark = this.len;
    this.u32(0);
    this.u32(count);
    return mark;
  }

  private ensure(extra: number): void {
    if (this.len + extra <= this.buf.length) return;
    if (this.len + extra > 0xffff_ffff) throw new BinCodecError('tooLarge', `size ${this.len + extra}`);
    let next = this.buf.length * 2;
    while (next < this.len + extra) next *= 2;
    const grown = new Uint8Array(next);
    grown.set(this.buf.subarray(0, this.len));
    this.buf = grown;
    this.view = new DataView(grown.buffer);
  }
}

/** Strict UTF-8 validation of `b[start .. end)` without decoding (Rust `core::str::from_utf8`). */
export function isValidUtf8(b: Uint8Array, start = 0, end = b.length): boolean {
  let i = start;
  while (i < end) {
    const b0 = b[i]!;
    if (b0 < 0x80) {
      i++;
      continue;
    }
    let need: number;
    let lo = 0x80;
    let hi = 0xbf;
    if (b0 >= 0xc2 && b0 <= 0xdf) {
      need = 1;
    } else if (b0 >= 0xe0 && b0 <= 0xef) {
      need = 2;
      if (b0 === 0xe0) lo = 0xa0;
      if (b0 === 0xed) hi = 0x9f;
    } else if (b0 >= 0xf0 && b0 <= 0xf4) {
      need = 3;
      if (b0 === 0xf0) lo = 0x90;
      if (b0 === 0xf4) hi = 0x8f;
    } else {
      return false;
    }
    if (need >= end - i) return false;
    const b1 = b[i + 1]!;
    if (b1 < lo || b1 > hi) return false;
    for (let k = 2; k <= need; k++) {
      if ((b[i + k]! & 0xc0) !== 0x80) return false;
    }
    i += need + 1;
  }
  return true;
}
