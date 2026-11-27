/**
 * Strict UTF-8 shared by the codec, hashing, and scene text (Rust `core::str::from_utf8`
 * acceptance): validation without decoding, a BOM-preserving strict decoder, and in-place
 * encoding that rejects lone surrogates (a Rust `&str` can never hold one).
 */
import { BinCodecError } from './bin_codec.ts';

/** Strict like Rust `str::from_utf8`; `ignoreBOM` keeps a leading U+FEFF instead of stripping it. */
export const UTF8_DECODER = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

/** Strict validation of `b[start .. end)` without decoding. */
export function isValidUtf8(b: Uint8Array, start = 0, end = b.length): boolean {
  if (!(Number.isInteger(start) && Number.isInteger(end) && start >= 0 && start <= end && end <= b.length)) {
    throw new RangeError(`range [${start}, ${end}) outside ${b.length} bytes`);
  }
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

/** Throws `BinCodecError('invalidUtf8')` when `s` holds a lone surrogate. */
export function requireWellFormed(s: string): void {
  if (!s.isWellFormed()) throw new BinCodecError('invalidUtf8', 'string contains a lone surrogate');
}

/**
 * Encodes the code point at `s[i]` (a surrogate pair consumes `s[i + 1]` too) into `out[o ..]` and
 * returns the end offset. Callers must have checked `requireWellFormed(s)`.
 */
export function encodeCodePointAt(s: string, i: number, out: Uint8Array, o: number): number {
  const c = s.codePointAt(i)!;
  if (c < 0x80) {
    out[o] = c;
    return o + 1;
  }
  if (c < 0x800) {
    out[o] = 0xc0 | (c >>> 6);
    out[o + 1] = 0x80 | (c & 0x3f);
    return o + 2;
  }
  if (c < 0x1_0000) {
    out[o] = 0xe0 | (c >>> 12);
    out[o + 1] = 0x80 | ((c >>> 6) & 0x3f);
    out[o + 2] = 0x80 | (c & 0x3f);
    return o + 3;
  }
  out[o] = 0xf0 | (c >>> 18);
  out[o + 1] = 0x80 | ((c >>> 12) & 0x3f);
  out[o + 2] = 0x80 | ((c >>> 6) & 0x3f);
  out[o + 3] = 0x80 | (c & 0x3f);
  return o + 4;
}

/** UTF-16 code units of the code point at `s[i]` (2 for a surrogate pair). */
export function utf16UnitsAt(s: string, i: number): number {
  return s.codePointAt(i)! > 0xffff ? 2 : 1;
}
