import { Key64, TWO32, read32LE, write32LE } from './key64.ts';
import { encodeCodePointAt, requireWellFormed, utf16UnitsAt } from './utf8.ts';

/** XXH64 primes 1..5 as `(lo, hi)` halves. */
export type Xxh64Primes = readonly [Key64, Key64, Key64, Key64, Key64];

let primeCache: Xxh64Primes | undefined;

/**
 * XXH64 primes, built on first use: `key64.ts` and this module import each other (`Key64.of`), so
 * neither may construct the other's values while the modules evaluate.
 */
export function xxh64Primes(): Xxh64Primes {
  return (primeCache ??= [
    new Key64(0x85eb_ca87, 0x9e37_79b1),
    new Key64(0x27d4_eb4f, 0xc2b2_ae3d),
    new Key64(0x9e37_79f9, 0x1656_67b1),
    new Key64(0xc2b2_ae63, 0x85eb_ca77),
    new Key64(0x1656_67c5, 0x27d4_eb2f),
  ]);
}

// Wrapping u64 arithmetic on halves: every intermediate stays below 2^53 (no BigInt in the hot loop).
function add(a: Key64, b: Key64): Key64 {
  const lo = a.lo + b.lo;
  return new Key64(lo, a.hi + b.hi + (lo > 0xffff_ffff ? 1 : 0));
}

function sub(a: Key64, b: Key64): Key64 {
  return add(a, add(new Key64(~b.lo, ~b.hi), new Key64(1, 0)));
}

function xor(a: Key64, b: Key64): Key64 {
  return new Key64(a.lo ^ b.lo, a.hi ^ b.hi);
}

/** `(a * b) mod 2^64` from 16-bit limbs. */
function mul(a: Key64, b: Key64): Key64 {
  const a0 = a.lo & 0xffff, a1 = a.lo >>> 16, b0 = b.lo & 0xffff, b1 = b.lo >>> 16;
  const p00 = a0 * b0, p01 = a0 * b1, p10 = a1 * b0;
  const mid = (p00 >>> 16) + (p01 & 0xffff) + (p10 & 0xffff);
  const lo = (mid & 0xffff) * 0x1_0000 + (p00 & 0xffff);
  const carry = a1 * b1 + (p01 >>> 16) + (p10 >>> 16) + (mid >>> 16);
  return new Key64(lo, carry + Math.imul(a.hi, b.lo) + Math.imul(a.lo, b.hi));
}

/** Rotate left by `0 < r < 32`. */
function rotl(x: Key64, r: number): Key64 {
  return new Key64((x.lo << r) | (x.hi >>> (32 - r)), (x.hi << r) | (x.lo >>> (32 - r)));
}

function shr(x: Key64, n: number): Key64 {
  return n >= 32 ? new Key64(x.hi >>> (n - 32), 0) : new Key64((x.lo >>> n) | (x.hi << (32 - n)), x.hi >>> n);
}

function round(acc: Key64, lane: Key64, p: Xxh64Primes): Key64 {
  return mul(rotl(add(acc, mul(lane, p[1])), 31), p[0]);
}

function mergeRound(acc: Key64, lane: Key64, p: Xxh64Primes): Key64 {
  return add(mul(xor(acc, round(Key64.ZERO, lane, p)), p[0]), p[3]);
}

/** Streaming canonical XXH64 (bit-identical to Rust `nve_hash::xxh64` for any chunking). */
export class Xxh64Hasher {
  private readonly p = xxh64Primes();
  private readonly seed: Key64;
  private v1: Key64;
  private v2: Key64;
  private v3: Key64;
  private v4: Key64;
  private readonly buf = new Uint8Array(32);
  private readonly scratch = new Uint8Array(4);
  private bufLen = 0;
  private total = 0;

  constructor(seed: Key64 = Key64.ZERO) {
    const [p1, p2] = this.p;
    this.seed = seed;
    this.v1 = add(add(seed, p1), p2);
    this.v2 = add(seed, p2);
    this.v3 = seed;
    this.v4 = sub(seed, p1);
  }

  /** Feeds `bytes[start .. end)`. */
  update(bytes: Uint8Array, start = 0, end = bytes.length): this {
    if (!(Number.isInteger(start) && Number.isInteger(end) && start >= 0 && start <= end && end <= bytes.length)) {
      throw new RangeError(`range [${start}, ${end}) outside ${bytes.length} bytes`);
    }
    const len = end;
    this.total += end - start;
    let i = start;
    if (this.bufLen > 0) {
      while (this.bufLen < 32 && i < len) this.buf[this.bufLen++] = bytes[i++]!;
      if (this.bufLen < 32) return this;
      this.stripe(this.buf, 0);
      this.bufLen = 0;
    }
    for (; i + 32 <= len; i += 32) this.stripe(bytes, i);
    while (i < len) this.buf[this.bufLen++] = bytes[i++]!;
    return this;
  }

  /** Feeds `v` as 4 little-endian bytes (no allocation). */
  updateU32(v: number): this {
    write32LE(this.scratch, 0, v);
    return this.update(this.scratch);
  }

  /**
   * Feeds the UTF-8 encoding of `text` without allocating; throws `BinCodecError('invalidUtf8')` on a
   * lone surrogate (never hashes U+FFFD).
   */
  updateUtf8(text: string): this {
    requireWellFormed(text);
    for (let i = 0; i < text.length; i += utf16UnitsAt(text, i)) {
      this.update(this.scratch, 0, encodeCodePointAt(text, i, this.scratch, 0));
    }
    return this;
  }

  /** Feeds `v` as 8 little-endian bytes. */
  updateU64(v: Key64): this {
    return this.updateU32(v.lo).updateU32(v.hi);
  }

  /** Digest of everything fed so far (the hasher stays usable). */
  digest(): Key64 {
    const p = this.p;
    const [p1, p2, p3, p4, p5] = p;
    let h: Key64;
    if (this.total >= 32) {
      h = add(add(rotl(this.v1, 1), rotl(this.v2, 7)), add(rotl(this.v3, 12), rotl(this.v4, 18)));
      h = mergeRound(mergeRound(mergeRound(mergeRound(h, this.v1, p), this.v2, p), this.v3, p), this.v4, p);
    } else {
      h = add(this.seed, p5);
    }
    h = add(h, new Key64(this.total % TWO32, Math.floor(this.total / TWO32)));
    let i = 0;
    for (; i + 8 <= this.bufLen; i += 8) {
      h = xor(h, round(Key64.ZERO, Key64.readLE(this.buf, i), p));
      h = add(mul(rotl(h, 27), p1), p4);
    }
    if (i + 4 <= this.bufLen) {
      h = xor(h, mul(new Key64(read32LE(this.buf, i), 0), p1));
      h = add(mul(rotl(h, 23), p2), p3);
      i += 4;
    }
    for (; i < this.bufLen; i++) {
      h = xor(h, mul(new Key64(this.buf[i]!, 0), p5));
      h = mul(rotl(h, 11), p1);
    }
    h = mul(xor(h, shr(h, 33)), p2);
    h = mul(xor(h, shr(h, 29)), p3);
    return xor(h, shr(h, 32));
  }

  private stripe(b: Uint8Array, i: number): void {
    this.v1 = round(this.v1, Key64.readLE(b, i), this.p);
    this.v2 = round(this.v2, Key64.readLE(b, i + 8), this.p);
    this.v3 = round(this.v3, Key64.readLE(b, i + 16), this.p);
    this.v4 = round(this.v4, Key64.readLE(b, i + 24), this.p);
  }
}

/** One-shot canonical XXH64 (bit-identical to Rust `nve_hash::xxh64`). */
export function xxh64(bytes: Uint8Array, seed: Key64 = Key64.ZERO): Key64 {
  return new Xxh64Hasher(seed).update(bytes).digest();
}

/**
 * Identity of a UTF-8 name: `xxh64(name, 0)` (Rust `nve_hash::const_id`); throws
 * `BinCodecError('invalidUtf8')` for a name with a lone surrogate.
 */
export function constId(name: string): Key64 {
  return new Xxh64Hasher().updateUtf8(name).digest();
}
