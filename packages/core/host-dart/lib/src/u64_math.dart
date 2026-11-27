/// Wrapping unsigned 64-bit arithmetic on `(lo, hi)` u32 halves.
///
/// Every intermediate stays below 2^53 and every bitwise result is masked to 32 bits, so the Dart
/// VM (64-bit ints) and Dart-web (IEEE doubles, 32-bit bitwise ops) produce identical bits.
library;

import 'key64.dart';

const int m32 = 0xFFFFFFFF;

/// 2^32, the weight of the high half.
const int two32 = 0x100000000;

/// `true` when compiled to JavaScript (ints are doubles there).
const bool isWeb = identical(0, 0.0);

Key64 add64(Key64 a, Key64 b) {
  final lo = a.lo + b.lo;
  return Key64(lo & m32, (a.hi + b.hi + (lo > m32 ? 1 : 0)) & m32);
}

Key64 sub64(Key64 a, Key64 b) => add64(a, add64(Key64(~b.lo & m32, ~b.hi & m32), const Key64(1, 0)));

Key64 xor64(Key64 a, Key64 b) => Key64((a.lo ^ b.lo) & m32, (a.hi ^ b.hi) & m32);

/// `(a * b) mod 2^32` from 16-bit limbs (every partial product stays below 2^53).
int mulLo32(int a, int b) => ((a & 0xFFFF) * b + (((a >>> 16) * b) & 0xFFFF) * 0x10000) & m32;

/// `(a * b) mod 2^64`.
Key64 mul64(Key64 a, Key64 b) {
  final a0 = a.lo & 0xFFFF, a1 = a.lo >>> 16, b0 = b.lo & 0xFFFF, b1 = b.lo >>> 16;
  final p00 = a0 * b0, p01 = a0 * b1, p10 = a1 * b0;
  final mid = (p00 >>> 16) + (p01 & 0xFFFF) + (p10 & 0xFFFF);
  final lo = (mid & 0xFFFF) * 0x10000 + (p00 & 0xFFFF);
  final carry = a1 * b1 + (p01 >>> 16) + (p10 >>> 16) + (mid >>> 16);
  return Key64(lo, (carry + mulLo32(a.hi, b.lo) + mulLo32(a.lo, b.hi)) & m32);
}

/// Rotate left by `0 < r < 32`.
Key64 rotl64(Key64 x, int r) => Key64(
      ((x.lo << r) | (x.hi >>> (32 - r))) & m32,
      ((x.hi << r) | (x.lo >>> (32 - r))) & m32,
    );

/// Logical shift right by `0 < n < 64`.
Key64 shr64(Key64 x, int n) => n >= 32
    ? Key64(x.hi >>> (n - 32), 0)
    : Key64(((x.lo >>> n) | (x.hi << (32 - n))) & m32, x.hi >>> n);
