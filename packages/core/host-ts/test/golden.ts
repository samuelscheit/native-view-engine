import { Key64 } from '../src/index.ts';

export const PRIME1 = new Key64(0x85eb_ca87, 0x9e37_79b1);
export const PRIME32 = new Key64(0x9e37_79b1, 0);
export const WIRE_SEED = new Key64(0x4952_4532, 0x4e56_4557);

export function seq(n: number): Uint8Array {
  return Uint8Array.from({ length: n }, (_, i) => (i * 31 + 7) & 0xff);
}

/** Official Cyan4973 xxHash sanity buffer (BigInt is fine in tests). */
export function cyan(n: number): Uint8Array {
  let gen = 0x9e37_79b1n;
  return Uint8Array.from({ length: n }, () => {
    const b = Number(gen >> 56n);
    gen = BigInt.asUintN(64, gen * 0x9e37_79b1_85eb_ca87n);
    return b;
  });
}

const utf8 = (s: string): Uint8Array => new TextEncoder().encode(s);

/** Golden vectors computed by the canonical Rust `nve-hash` crate. */
export const VECTORS: ReadonlyArray<readonly [string, Uint8Array, Key64, Key64]> = [
  ['empty', new Uint8Array(0), Key64.ZERO, new Key64(0x51d8e999, 0xef46db37)],
  ['a', utf8('a'), Key64.ZERO, new Key64(0xa98c6e5b, 0xd24ec4f1)],
  ['abc', utf8('abc'), Key64.ZERO, new Key64(0xad770999, 0x44bc2cf5)],
  ['nve.visual', utf8('nve.visual'), Key64.ZERO, new Key64(0x985a4fd6, 0x33e73115)],
  ['high_bits', Uint8Array.of(0x00, 0x01, 0x7f, 0x80, 0xff), Key64.ZERO, new Key64(0x99a141ea, 0x8babcab4)],
  ['utf8', utf8('héllo wörld — 日本語 🚀'), Key64.ZERO, new Key64(0xf213f592, 0x72ac442f)],
  ['seq101', seq(101), Key64.ZERO, new Key64(0x1a53bc7f, 0xfbfdf3fa)],
  ['seq101_wire', seq(101), WIRE_SEED, new Key64(0x8c3db1bc, 0x277e092a)],
  ['seq37_prime1', seq(37), PRIME1, new Key64(0x85fc58a4, 0x33ddb9a8)],
  ['seq1024', seq(1024), Key64.ZERO, new Key64(0x72cdae00, 0x149aa449)],
  ['cyan1', cyan(1), Key64.ZERO, new Key64(0xdb052768, 0xe934a84a)],
  ['cyan4', cyan(4), Key64.ZERO, new Key64(0xf81e29e4, 0xe244ddbc)],
  ['cyan14', cyan(14), Key64.ZERO, new Key64(0xe0bd0a0a, 0xb89b3598)],
  ['cyan222', cyan(222), Key64.ZERO, new Key64(0x30bcec3a, 0x06cc5bd9)],
  ['cyan1024', cyan(1024), Key64.ZERO, new Key64(0xe063991a, 0x2e307f2f)],
  ['cyan1_p32', cyan(1), PRIME32, new Key64(0x43a9b4c3, 0x50146076)],
  ['cyan222_p32', cyan(222), PRIME32, new Key64(0xdf89af66, 0x0b105469)],
  ['cyan32_p1', cyan(32), PRIME1, new Key64(0xb7e278ed, 0x6d0242b0)],
  ['cyan1024_p1', cyan(1024), PRIME1, new Key64(0x9a39421c, 0xf464c3f6)],
];

/** Validated by Rust `BinValueView::parse`: {"k": [1, "é", null, true, 1.5, b"\xFF"]}. */
export const GOLDEN_BIN_VALUE = fromHex(
  '07 34000000 01000000 01000000 6b 06 26000000 06000000 02 0100000000000000 04 02000000 c3a9 00 0101 03 ' +
    '000000000000f83f 05 01000000 ff',
);

export function fromHex(hex: string): Uint8Array {
  const digits = hex.replaceAll(' ', '');
  return Uint8Array.from({ length: digits.length / 2 }, (_, i) => parseInt(digits.slice(2 * i, 2 * i + 2), 16));
}
