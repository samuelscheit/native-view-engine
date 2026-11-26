//! TypeScript (`nve_hash.generated.ts`) XXH64 mirror on `(lo, hi)` `u32` halves: exact with plain
//! `number`s (53-bit safe), zero `BigInt`. Erasable syntax only, so Node runs it directly.

use super::{banner, byte_list, halves, HASH_VECTORS, STREAM_SPLITS};

const BODY: &str = r#"
/** 64-bit key / XXH64 digest as two unsigned 32-bit halves (Rust `nve_hash::Key64`). */
export class NveKey64 {
  static readonly ZERO = new NveKey64(0, 0);
  readonly lo: number;
  readonly hi: number;

  constructor(lo: number, hi: number) {
    this.lo = lo >>> 0;
    this.hi = hi >>> 0;
  }

  /** Reads 8 little-endian bytes at `offset`. */
  static readLE(bytes: Uint8Array, offset = 0): NveKey64 {
    return new NveKey64(read32(bytes, offset), read32(bytes, offset + 4));
  }

  /** Key of a reverse-DNS or plain name (Rust `Key64::of`). */
  static of(name: string): NveKey64 {
    return nveConstId(name);
  }

  /** Writes 8 little-endian bytes at `offset`. */
  writeLE(out: Uint8Array, offset = 0): void {
    for (let i = 0; i < 4; i++) {
      out[offset + i] = (this.lo >>> (8 * i)) & 0xff;
      out[offset + 4 + i] = (this.hi >>> (8 * i)) & 0xff;
    }
  }

  equals(other: NveKey64): boolean {
    return this.lo === other.lo && this.hi === other.hi;
  }

  /** 16 lowercase hex digits, high half first. */
  toHex(): string {
    return this.hi.toString(16).padStart(8, '0') + this.lo.toString(16).padStart(8, '0');
  }
}

const P1 = new NveKey64(0x85ebca87, 0x9e3779b1);
const P2 = new NveKey64(0x27d4eb4f, 0xc2b2ae3d);
const P3 = new NveKey64(0x9e3779f9, 0x165667b1);
const P4 = new NveKey64(0xc2b2ae63, 0x85ebca77);
const P5 = new NveKey64(0x165667c5, 0x27d4eb2f);

function read32(b: Uint8Array, i: number): number {
  return (b[i]! | (b[i + 1]! << 8) | (b[i + 2]! << 16) | (b[i + 3]! << 24)) >>> 0;
}

function add(a: NveKey64, b: NveKey64): NveKey64 {
  const lo = a.lo + b.lo;
  return new NveKey64(lo, a.hi + b.hi + (lo > 0xffffffff ? 1 : 0));
}

function sub(a: NveKey64, b: NveKey64): NveKey64 {
  return add(a, add(new NveKey64(~b.lo, ~b.hi), new NveKey64(1, 0)));
}

function xor(a: NveKey64, b: NveKey64): NveKey64 {
  return new NveKey64(a.lo ^ b.lo, a.hi ^ b.hi);
}

/** `(a * b) mod 2^64` from 16-bit limbs (every partial product stays below 2^53). */
function mul(a: NveKey64, b: NveKey64): NveKey64 {
  const a0 = a.lo & 0xffff, a1 = a.lo >>> 16, b0 = b.lo & 0xffff, b1 = b.lo >>> 16;
  const p00 = a0 * b0, p01 = a0 * b1, p10 = a1 * b0;
  const mid = (p00 >>> 16) + (p01 & 0xffff) + (p10 & 0xffff);
  const lo = (mid & 0xffff) * 0x10000 + (p00 & 0xffff);
  const carry = a1 * b1 + (p01 >>> 16) + (p10 >>> 16) + (mid >>> 16);
  return new NveKey64(lo, carry + Math.imul(a.hi, b.lo) + Math.imul(a.lo, b.hi));
}

/** Rotate left by `0 < r < 32`. */
function rotl(x: NveKey64, r: number): NveKey64 {
  return new NveKey64((x.lo << r) | (x.hi >>> (32 - r)), (x.hi << r) | (x.lo >>> (32 - r)));
}

function shr(x: NveKey64, n: number): NveKey64 {
  return n >= 32
    ? new NveKey64(x.hi >>> (n - 32), 0)
    : new NveKey64((x.lo >>> n) | (x.hi << (32 - n)), x.hi >>> n);
}

function round(acc: NveKey64, lane: NveKey64): NveKey64 {
  return mul(rotl(add(acc, mul(lane, P2)), 31), P1);
}

/** Streaming canonical XXH64; digests equal `nveXxh64` for any chunking. */
export class NveXxh64Hasher {
  private readonly seed: NveKey64;
  private readonly lanes: NveKey64[];
  private readonly buf = new Uint8Array(32);
  private bufLen = 0;
  private total = 0;

  constructor(seed: NveKey64 = NveKey64.ZERO) {
    this.seed = seed;
    this.lanes = [add(add(seed, P1), P2), add(seed, P2), seed, sub(seed, P1)];
  }

  update(bytes: Uint8Array): void {
    const len = bytes.length;
    this.total += len;
    let i = 0;
    if (this.bufLen > 0) {
      while (this.bufLen < 32 && i < len) this.buf[this.bufLen++] = bytes[i++]!;
      if (this.bufLen < 32) return;
      this.stripe(this.buf, 0);
      this.bufLen = 0;
    }
    for (; i + 32 <= len; i += 32) this.stripe(bytes, i);
    while (i < len) this.buf[this.bufLen++] = bytes[i++]!;
  }

  /** Digest of everything fed so far (the hasher stays usable). */
  digest(): NveKey64 {
    let h: NveKey64;
    if (this.total >= 32) {
      const [v1, v2, v3, v4] = this.lanes as [NveKey64, NveKey64, NveKey64, NveKey64];
      h = add(add(rotl(v1, 1), rotl(v2, 7)), add(rotl(v3, 12), rotl(v4, 18)));
      for (const lane of this.lanes) h = add(mul(xor(h, round(NveKey64.ZERO, lane)), P1), P4);
    } else {
      h = add(this.seed, P5);
    }
    h = add(h, new NveKey64(this.total % 0x100000000, Math.floor(this.total / 0x100000000)));
    let i = 0;
    for (; i + 8 <= this.bufLen; i += 8) {
      h = xor(h, round(NveKey64.ZERO, NveKey64.readLE(this.buf, i)));
      h = add(mul(rotl(h, 27), P1), P4);
    }
    if (i + 4 <= this.bufLen) {
      h = xor(h, mul(new NveKey64(read32(this.buf, i), 0), P1));
      h = add(mul(rotl(h, 23), P2), P3);
      i += 4;
    }
    for (; i < this.bufLen; i++) {
      h = xor(h, mul(new NveKey64(this.buf[i]!, 0), P5));
      h = mul(rotl(h, 11), P1);
    }
    h = mul(xor(h, shr(h, 33)), P2);
    h = mul(xor(h, shr(h, 29)), P3);
    return xor(h, shr(h, 32));
  }

  private stripe(b: Uint8Array, i: number): void {
    for (let k = 0; k < 4; k++) this.lanes[k] = round(this.lanes[k]!, NveKey64.readLE(b, i + 8 * k));
  }
}

/** One-shot canonical XXH64 (bit-identical to Rust `nve_hash::xxh64`). */
export function nveXxh64(bytes: Uint8Array, seed: NveKey64 = NveKey64.ZERO): NveKey64 {
  const h = new NveXxh64Hasher(seed);
  h.update(bytes);
  return h.digest();
}

/** Identity of a UTF-8 name: `xxh64(name, 0)` (Rust `nve_hash::const_id`). */
export function nveConstId(name: string): NveKey64 {
  return nveXxh64(new TextEncoder().encode(name));
}
"#;

/// Generates `hosts/web-shell/nve_hash.generated.ts`.
pub(super) fn generate_ts_hash() -> String {
    let mut s = banner();
    s.push_str(BODY);
    s.push_str(
        "\n/** Golden vectors shared with every NVE host. */\n\
         export const NVE_HASH_TEST_VECTORS: ReadonlyArray<{ bytes: readonly number[]; seed: NveKey64; expected: NveKey64 }> = [\n",
    );
    for v in HASH_VECTORS {
        let bytes = byte_list(v.bytes, |b| format!("0x{b:02x}"));
        let ((sl, sh), (el, eh)) = (halves(v.seed), halves(v.expected()));
        writeln!(
            s,
            "  {{ bytes: [{bytes}], seed: new NveKey64(0x{sl:08x}, 0x{sh:08x}), expected: new NveKey64(0x{el:08x}, 0x{eh:08x}) }}, // {}",
            v.label
        );
    }
    let [a, b, c] = STREAM_SPLITS;
    write!(
        s,
        r#"];

/** `true` when one-shot and chunked digests of every golden vector match Rust. */
export function nveHashVerifyTestVectors(): boolean {{
  return NVE_HASH_TEST_VECTORS.every((v) => {{
    const bytes = Uint8Array.from(v.bytes);
    const h = new NveXxh64Hasher(v.seed);
    let at = 0;
    for (const split of [{a}, {b}, {c}]) {{
      const end = Math.min(split, bytes.length);
      if (end > at) {{
        h.update(bytes.subarray(at, end));
        at = end;
      }}
    }}
    h.update(bytes.subarray(at));
    return nveXxh64(bytes, v.seed).equals(v.expected) && h.digest().equals(v.expected);
  }});
}}
"#
    );
    s.into_string()
}
