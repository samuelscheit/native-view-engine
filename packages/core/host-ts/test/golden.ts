import { Key64 } from '../src/index.ts';
import { HASH_VECTORS } from './hash_vectors.generated.ts';

export const WIRE_SEED = new Key64(0x4952_4532, 0x4e56_4557);

export function seq(n: number): Uint8Array {
  return Uint8Array.from({ length: n }, (_, i) => (i * 31 + 7) & 0xff);
}

/** Golden vectors generated from Rust `nve_hash::xxh64` (`hash_vectors.generated.ts`). */
export const VECTORS: ReadonlyArray<readonly [string, Uint8Array, Key64, Key64]> = HASH_VECTORS.map(
  (v) => [v.label, fromHex(v.hex), v.seed, v.expected] as const,
);

/** Validated by Rust `BinValueView::parse`: {"k": [1, "é", null, true, 1.5, b"\xFF"]}. */
export const GOLDEN_BIN_VALUE = fromHex(
  '07 34000000 01000000 01000000 6b 06 26000000 06000000 02 0100000000000000 04 02000000 c3a9 00 0101 03 ' +
    '000000000000f83f 05 01000000 ff',
);

export function fromHex(hex: string): Uint8Array {
  const digits = hex.replaceAll(' ', '');
  return Uint8Array.from({ length: digits.length / 2 }, (_, i) => parseInt(digits.slice(2 * i, 2 * i + 2), 16));
}
