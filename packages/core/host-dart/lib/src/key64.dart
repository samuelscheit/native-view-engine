import 'u64_math.dart';
import 'xxh64.dart';

/// 64-bit identity key or unsigned 64-bit value as two unsigned 32-bit halves
/// (Rust `nve_hash::Key64`). Exact on the Dart VM and on the web, compared without `BigInt`.
final class Key64 implements Comparable<Key64> {
  /// `lo` and `hi` must each be in `0 ..= 0xFFFFFFFF` (checked in debug builds).
  const Key64(this.lo, this.hi)
      : assert(lo >= 0 && lo <= 0xFFFFFFFF, 'lo must be an unsigned 32-bit value'),
        assert(hi >= 0 && hi <= 0xFFFFFFFF, 'hi must be an unsigned 32-bit value');

  /// Splits a non-negative or negative `int` into its two's-complement 64-bit halves.
  ///
  /// Exact for every VM `int`; on the web for every integral double (callers must not pass values
  /// that already lost precision).
  factory Key64.fromInt(int value) {
    final lo = value % two32;
    final hi = ((value - lo) ~/ two32) % two32;
    return Key64(lo, hi);
  }

  /// Reads 8 little-endian bytes at [offset].
  factory Key64.readLE(List<int> bytes, [int offset = 0]) =>
      Key64(read32LE(bytes, offset), read32LE(bytes, offset + 4));

  /// Key of a reverse-DNS or plain name (Rust `Key64::of`).
  factory Key64.of(String name) => constId(name);

  static const Key64 zero = Key64(0, 0);

  final int lo;
  final int hi;

  /// The unsigned value as an `int` when it is exact on this platform (always below 2^63 on the
  /// VM, below 2^53 on the web); otherwise throws [StateError].
  int toUnsignedInt() {
    final limit = isWeb ? 0x200000 : 0x80000000;
    if (hi >= limit) {
      throw StateError('Key64(0x${toHex()}) is not exactly representable as an int here');
    }
    return hi * two32 + lo;
  }

  /// The two's-complement signed value (Rust `i64`) when exact on this platform; else throws.
  int toSignedInt() {
    if (hi < 0x80000000) {
      return toUnsignedInt();
    }
    final negHi = hi - two32; // -2^31 ..= -1
    if (isWeb && negHi < -0x200000) {
      throw StateError('Key64(0x${toHex()}) as i64 is not exactly representable as an int here');
    }
    return negHi * two32 + lo;
  }

  /// Writes 8 little-endian bytes at [offset]; throws [RangeError] before writing anything when
  /// they do not fit.
  void writeLE(List<int> out, [int offset = 0]) {
    RangeError.checkValueInInterval(offset, 0, out.length - 8, 'offset');
    write32LE(out, offset, lo);
    write32LE(out, offset + 4, hi);
  }

  /// 16 lowercase hex digits, high half first.
  String toHex() => hi.toRadixString(16).padLeft(8, '0') + lo.toRadixString(16).padLeft(8, '0');

  @override
  int compareTo(Key64 other) => hi != other.hi ? hi.compareTo(other.hi) : lo.compareTo(other.lo);

  @override
  bool operator ==(Object other) => other is Key64 && other.lo == lo && other.hi == hi;

  @override
  int get hashCode => Object.hash(lo, hi);

  @override
  String toString() => 'Key64(0x${toHex()})';
}

/// Little-endian `u32` at `b[i ..< i + 4]` (no 32-bit overflow on the web).
int read32LE(List<int> b, int i) =>
    (b[i] & 0xFF) + (b[i + 1] & 0xFF) * 0x100 + (b[i + 2] & 0xFF) * 0x10000 + (b[i + 3] & 0xFF) * 0x1000000;

/// Writes 4 little-endian bytes; throws [RangeError] before writing anything when they do not fit.
void write32LE(List<int> out, int offset, int v) {
  RangeError.checkValueInInterval(offset, 0, out.length - 4, 'offset');
  out[offset] = v & 0xFF;
  out[offset + 1] = (v >>> 8) & 0xFF;
  out[offset + 2] = (v >>> 16) & 0xFF;
  out[offset + 3] = (v >>> 24) & 0xFF;
}
