import { constId } from './xxh64.ts';

/** 2^32, the weight of the high half. */
export const TWO32 = 0x1_0000_0000;

/**
 * 64-bit identity key or unsigned 64-bit value as two unsigned 32-bit halves (Rust
 * `nve_hash::Key64`). Exact with plain `number`s (every half < 2^32); converts to `bigint` only on
 * request.
 */
export class Key64 {
  static readonly ZERO = new Key64(0, 0);

  readonly lo: number;
  readonly hi: number;

  /**
   * Halves are normalized with `>>> 0` (wrapping), so any int32 / uint32 bit pattern is accepted;
   * NaN, ±Infinity, fractions, and unsafe integers throw `RangeError` instead of becoming 0.
   */
  constructor(lo: number, hi: number) {
    if (!Number.isSafeInteger(lo) || !Number.isSafeInteger(hi)) throw new RangeError(`Key64 halves ${lo}, ${hi}`);
    this.lo = lo >>> 0;
    this.hi = hi >>> 0;
  }

  /** Key of a reverse-DNS or plain name (Rust `Key64::of`). */
  static of(name: string): Key64 {
    return constId(name);
  }

  /** Two's-complement halves of a signed or unsigned 64-bit `bigint` (wraps modulo 2^64). */
  static fromBigInt(value: bigint): Key64 {
    const v = BigInt.asUintN(64, value);
    return new Key64(Number(v & 0xffff_ffffn), Number(v >> 32n));
  }

  /** Halves of a safe integer (`|value| <= 2^53 - 1`); throws `RangeError` otherwise. */
  static fromSafeInteger(value: number): Key64 {
    if (!Number.isSafeInteger(value)) throw new RangeError(`${value} is not a safe integer`);
    const lo = ((value % TWO32) + TWO32) % TWO32;
    return new Key64(lo, (value - lo) / TWO32);
  }

  /** Reads 8 little-endian bytes at `offset`. */
  static readLE(bytes: Uint8Array, offset = 0): Key64 {
    return new Key64(read32LE(bytes, offset), read32LE(bytes, offset + 4));
  }

  /** Writes 8 little-endian bytes at `offset`. */
  writeLE(out: Uint8Array, offset = 0): void {
    // Checked up front so a half-fitting key never leaves 4 written bytes behind.
    if (!(Number.isInteger(offset) && offset >= 0 && offset <= out.length - 8)) {
      throw new RangeError(`writeLE at ${offset} outside ${out.length} bytes`);
    }
    write32LE(out, offset, this.lo);
    write32LE(out, offset + 4, this.hi);
  }

  toBigInt(): bigint {
    return (BigInt(this.hi) << 32n) | BigInt(this.lo);
  }

  /** Two's-complement signed value (Rust `i64`). */
  toSignedBigInt(): bigint {
    return BigInt.asIntN(64, this.toBigInt());
  }

  /** Unsigned value as a `number` when exact (< 2^53); throws `RangeError` otherwise. */
  toSafeInteger(): number {
    if (this.hi >= 0x20_0000) throw new RangeError(`Key64(0x${this.toHex()}) exceeds 2^53`);
    return this.hi * TWO32 + this.lo;
  }

  /**
   * Signed (`i64`) value as a JS `number` when within safe integer bounds (`|v| <= 2^53 - 1`),
   * or `null` when outside (zero `BigInt` allocation on the safe-integer fast path).
   */
  toSignedSafeInteger(): number | null {
    if (this.hi <= 0x001f_ffff) return this.hi * TWO32 + this.lo;
    if (this.hi >= 0xffe0_0000 && (this.hi !== 0xffe0_0000 || this.lo !== 0)) {
      return (this.hi - TWO32) * TWO32 + this.lo;
    }
    return null;
  }

  /** Signed (`i64`) value widened to the nearest double, like Rust `as f64`. */
  toSignedNumber(): number {
    const hi = this.hi >= 0x8000_0000 ? this.hi - TWO32 : this.hi;
    return hi * TWO32 + this.lo;
  }

  equals(other: Key64): boolean {
    return this.lo === other.lo && this.hi === other.hi;
  }

  /** 16 lowercase hex digits, high half first. */
  toHex(): string {
    return this.hi.toString(16).padStart(8, '0') + this.lo.toString(16).padStart(8, '0');
  }

  toString(): string {
    return `Key64(0x${this.toHex()})`;
  }
}

/** Little-endian `u32` at `b[i .. i + 4)`. */
export function read32LE(b: Uint8Array, i: number): number {
  if (!(Number.isInteger(i) && i >= 0 && i <= b.length - 4)) throw new RangeError(`read32 at ${i} outside ${b.length} bytes`);
  return (b[i]! | (b[i + 1]! << 8) | (b[i + 2]! << 16) | (b[i + 3]! << 24)) >>> 0;
}

export function write32LE(out: Uint8Array, offset: number, v: number): void {
  if (!(Number.isInteger(offset) && offset >= 0 && offset <= out.length - 4)) {
    throw new RangeError(`write32 at ${offset} outside ${out.length} bytes`);
  }
  out[offset] = v & 0xff;
  out[offset + 1] = (v >>> 8) & 0xff;
  out[offset + 2] = (v >>> 16) & 0xff;
  out[offset + 3] = (v >>> 24) & 0xff;
}
