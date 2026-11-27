import 'dart:typed_data';

import 'key64.dart';
import 'u64_math.dart';
import 'utf8.dart';

const Key64 _p1 = Key64(0x85EBCA87, 0x9E3779B1);
const Key64 _p2 = Key64(0x27D4EB4F, 0xC2B2AE3D);
const Key64 _p3 = Key64(0x9E3779F9, 0x165667B1);
const Key64 _p4 = Key64(0xC2B2AE63, 0x85EBCA77);
const Key64 _p5 = Key64(0x165667C5, 0x27D4EB2F);

/// XXH64 primes as `(lo, hi)` halves.
abstract final class Xxh64Primes {
  static const Key64 prime1 = _p1;
  static const Key64 prime2 = _p2;
  static const Key64 prime3 = _p3;
  static const Key64 prime4 = _p4;
  static const Key64 prime5 = _p5;
}

Key64 _round(Key64 acc, Key64 lane) => mul64(rotl64(add64(acc, mul64(lane, _p2)), 31), _p1);

Key64 _mergeRound(Key64 acc, Key64 lane) => add64(mul64(xor64(acc, _round(Key64.zero, lane)), _p1), _p4);

/// Streaming canonical XXH64 (bit-identical to Rust `nve_hash::xxh64` for any chunking).
final class Xxh64Hasher {
  Xxh64Hasher([this._seed = Key64.zero])
      : _v1 = add64(add64(_seed, _p1), _p2),
        _v2 = add64(_seed, _p2),
        _v3 = _seed,
        _v4 = sub64(_seed, _p1);

  final Key64 _seed;
  Key64 _v1, _v2, _v3, _v4;
  final Uint8List _buf = Uint8List(32);
  final Uint8List _scratch = Uint8List(4);
  int _bufLen = 0;
  int _total = 0;

  /// Feeds `bytes[start ..< end]`.
  void update(List<int> bytes, [int start = 0, int? end]) {
    final stop = RangeError.checkValidRange(start, end, bytes.length);
    _total += stop - start;
    var i = start;
    if (_bufLen > 0) {
      while (_bufLen < 32 && i < stop) {
        _buf[_bufLen++] = bytes[i++] & 0xFF;
      }
      if (_bufLen < 32) {
        return;
      }
      _stripe(_buf, 0);
      _bufLen = 0;
    }
    for (; i + 32 <= stop; i += 32) {
      _stripe(bytes, i);
    }
    while (i < stop) {
      _buf[_bufLen++] = bytes[i++] & 0xFF;
    }
  }

  /// Feeds `v` as 4 little-endian bytes (no allocation).
  void updateU32(int v) {
    write32LE(_scratch, 0, v);
    update(_scratch);
  }

  /// Feeds the UTF-8 encoding of [text] without allocating; throws [BinCodecException]
  /// (`invalidUtf8`) on a lone surrogate (never hashes U+FFFD).
  void updateUtf8(String text) {
    for (var i = 0; i < text.length; i += utf16UnitsAt(text, i)) {
      update(_scratch, 0, encodeUtf8CodePointAt(text, i, _scratch, 0));
    }
  }

  /// Feeds `v` as 8 little-endian bytes.
  void updateU64(Key64 v) {
    updateU32(v.lo);
    updateU32(v.hi);
  }

  void _stripe(List<int> b, int i) {
    _v1 = _round(_v1, Key64.readLE(b, i));
    _v2 = _round(_v2, Key64.readLE(b, i + 8));
    _v3 = _round(_v3, Key64.readLE(b, i + 16));
    _v4 = _round(_v4, Key64.readLE(b, i + 24));
  }

  /// Digest of everything fed so far (the hasher stays usable).
  Key64 digest() {
    var h = _total >= 32
        ? _mergeRound(
            _mergeRound(
              _mergeRound(
                _mergeRound(add64(add64(rotl64(_v1, 1), rotl64(_v2, 7)), add64(rotl64(_v3, 12), rotl64(_v4, 18))), _v1),
                _v2,
              ),
              _v3,
            ),
            _v4,
          )
        : add64(_seed, _p5);
    h = add64(h, Key64(_total % two32, _total ~/ two32));
    var i = 0;
    for (; i + 8 <= _bufLen; i += 8) {
      h = xor64(h, _round(Key64.zero, Key64.readLE(_buf, i)));
      h = add64(mul64(rotl64(h, 27), _p1), _p4);
    }
    if (i + 4 <= _bufLen) {
      h = xor64(h, mul64(Key64(read32LE(_buf, i), 0), _p1));
      h = add64(mul64(rotl64(h, 23), _p2), _p3);
      i += 4;
    }
    for (; i < _bufLen; i++) {
      h = xor64(h, mul64(Key64(_buf[i], 0), _p5));
      h = mul64(rotl64(h, 11), _p1);
    }
    h = mul64(xor64(h, shr64(h, 33)), _p2);
    h = mul64(xor64(h, shr64(h, 29)), _p3);
    return xor64(h, shr64(h, 32));
  }
}

/// One-shot canonical XXH64 (bit-identical to Rust `nve_hash::xxh64`).
Key64 xxh64(List<int> bytes, [Key64 seed = Key64.zero]) => (Xxh64Hasher(seed)..update(bytes)).digest();

/// Identity of a UTF-8 name: `xxh64(name, 0)` (Rust `nve_hash::const_id`); throws
/// [BinCodecException] (`invalidUtf8`) for a name with a lone surrogate.
Key64 constId(String name) => (Xxh64Hasher()..updateUtf8(name)).digest();
