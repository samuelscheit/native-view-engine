//! Dart (`nve_hash.generated.dart`) XXH64 mirror on `(lo, hi)` `u32` halves: bit-exact on the
//! Dart VM and on the web (53-bit doubles), with zero `BigInt`.

use std::fmt::Write;

use super::{banner, byte_list, halves, HASH_VECTORS, STREAM_SPLITS};

const BODY: &str = r#"import 'dart:convert' show utf8;
import 'dart:typed_data';

const int _m32 = 0xFFFFFFFF;

/// 64-bit key / XXH64 digest as two unsigned 32-bit halves, exact on the Dart VM and on the web
/// and compared without `BigInt` (Rust `nve_hash::Key64`).
final class NveKey64 {
  const NveKey64(this.lo, this.hi);

  /// Reads 8 little-endian bytes at [offset].
  factory NveKey64.readLE(List<int> bytes, [int offset = 0]) =>
      NveKey64(_read32(bytes, offset), _read32(bytes, offset + 4));

  /// Key of a reverse-DNS or plain name (Rust `Key64::of`).
  factory NveKey64.of(String name) => nveConstId(name);

  static const NveKey64 zero = NveKey64(0, 0);

  final int lo;
  final int hi;

  /// Writes 8 little-endian bytes at [offset].
  void writeLE(List<int> out, [int offset = 0]) {
    for (var i = 0; i < 4; i++) {
      out[offset + i] = (lo >>> (8 * i)) & 0xFF;
      out[offset + 4 + i] = (hi >>> (8 * i)) & 0xFF;
    }
  }

  /// 16 lowercase hex digits, high half first.
  String toHex() => hi.toRadixString(16).padLeft(8, '0') + lo.toRadixString(16).padLeft(8, '0');

  @override
  bool operator ==(Object other) => other is NveKey64 && other.lo == lo && other.hi == hi;

  @override
  int get hashCode => Object.hash(lo, hi);

  @override
  String toString() => 'NveKey64(0x${toHex()})';
}

const NveKey64 _p1 = NveKey64(0x85EBCA87, 0x9E3779B1);
const NveKey64 _p2 = NveKey64(0x27D4EB4F, 0xC2B2AE3D);
const NveKey64 _p3 = NveKey64(0x9E3779F9, 0x165667B1);
const NveKey64 _p4 = NveKey64(0xC2B2AE63, 0x85EBCA77);
const NveKey64 _p5 = NveKey64(0x165667C5, 0x27D4EB2F);

int _read32(List<int> b, int i) =>
    (b[i] & 0xFF) + (b[i + 1] & 0xFF) * 0x100 + (b[i + 2] & 0xFF) * 0x10000 + (b[i + 3] & 0xFF) * 0x1000000;

NveKey64 _add(NveKey64 a, NveKey64 b) {
  final lo = a.lo + b.lo;
  return NveKey64(lo & _m32, (a.hi + b.hi + (lo > _m32 ? 1 : 0)) & _m32);
}

NveKey64 _sub(NveKey64 a, NveKey64 b) =>
    _add(a, _add(NveKey64(~b.lo & _m32, ~b.hi & _m32), const NveKey64(1, 0)));

NveKey64 _xor(NveKey64 a, NveKey64 b) => NveKey64((a.lo ^ b.lo) & _m32, (a.hi ^ b.hi) & _m32);

/// `(a * b) mod 2^32` from 16-bit limbs (every partial product stays below 2^53).
int _mulLo32(int a, int b) => ((a & 0xFFFF) * b + (((a >>> 16) * b) & 0xFFFF) * 0x10000) & _m32;

/// `(a * b) mod 2^64`.
NveKey64 _mul(NveKey64 a, NveKey64 b) {
  final a0 = a.lo & 0xFFFF, a1 = a.lo >>> 16, b0 = b.lo & 0xFFFF, b1 = b.lo >>> 16;
  final p00 = a0 * b0, p01 = a0 * b1, p10 = a1 * b0;
  final mid = (p00 >>> 16) + (p01 & 0xFFFF) + (p10 & 0xFFFF);
  final lo = (mid & 0xFFFF) * 0x10000 + (p00 & 0xFFFF);
  final carry = a1 * b1 + (p01 >>> 16) + (p10 >>> 16) + (mid >>> 16);
  return NveKey64(lo, (carry + _mulLo32(a.hi, b.lo) + _mulLo32(a.lo, b.hi)) & _m32);
}

/// Rotate left by `0 < r < 32`.
NveKey64 _rotl(NveKey64 x, int r) => NveKey64(
      ((x.lo << r) | (x.hi >>> (32 - r))) & _m32,
      ((x.hi << r) | (x.lo >>> (32 - r))) & _m32,
    );

NveKey64 _shr(NveKey64 x, int n) => n >= 32
    ? NveKey64(x.hi >>> (n - 32), 0)
    : NveKey64(((x.lo >>> n) | (x.hi << (32 - n))) & _m32, x.hi >>> n);

NveKey64 _round(NveKey64 acc, NveKey64 lane) => _mul(_rotl(_add(acc, _mul(lane, _p2)), 31), _p1);

/// Streaming canonical XXH64; digests equal [nveXxh64] for any chunking.
final class NveXxh64Hasher {
  NveXxh64Hasher([this._seed = NveKey64.zero])
      : _lanes = <NveKey64>[_add(_add(_seed, _p1), _p2), _add(_seed, _p2), _seed, _sub(_seed, _p1)];

  final NveKey64 _seed;
  final List<NveKey64> _lanes;
  final Uint8List _buf = Uint8List(32);
  int _bufLen = 0;
  int _total = 0;

  /// Feeds `bytes[start..end)`.
  void update(List<int> bytes, [int start = 0, int? end]) {
    final stop = end ?? bytes.length;
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

  void _stripe(List<int> b, int i) {
    for (var k = 0; k < 4; k++) {
      _lanes[k] = _round(_lanes[k], NveKey64.readLE(b, i + 8 * k));
    }
  }

  /// Digest of everything fed so far (the hasher stays usable).
  NveKey64 digest() {
    NveKey64 h;
    if (_total >= 32) {
      h = _add(_add(_rotl(_lanes[0], 1), _rotl(_lanes[1], 7)), _add(_rotl(_lanes[2], 12), _rotl(_lanes[3], 18)));
      for (final lane in _lanes) {
        h = _add(_mul(_xor(h, _round(NveKey64.zero, lane)), _p1), _p4);
      }
    } else {
      h = _add(_seed, _p5);
    }
    h = _add(h, NveKey64(_total % 0x100000000, _total ~/ 0x100000000));
    var i = 0;
    for (; i + 8 <= _bufLen; i += 8) {
      h = _xor(h, _round(NveKey64.zero, NveKey64.readLE(_buf, i)));
      h = _add(_mul(_rotl(h, 27), _p1), _p4);
    }
    if (i + 4 <= _bufLen) {
      h = _xor(h, _mul(NveKey64(_read32(_buf, i), 0), _p1));
      h = _add(_mul(_rotl(h, 23), _p2), _p3);
      i += 4;
    }
    for (; i < _bufLen; i++) {
      h = _xor(h, _mul(NveKey64(_buf[i], 0), _p5));
      h = _mul(_rotl(h, 11), _p1);
    }
    h = _mul(_xor(h, _shr(h, 33)), _p2);
    h = _mul(_xor(h, _shr(h, 29)), _p3);
    return _xor(h, _shr(h, 32));
  }
}

/// One-shot canonical XXH64 (bit-identical to Rust `nve_hash::xxh64`).
NveKey64 nveXxh64(List<int> bytes, [NveKey64 seed = NveKey64.zero]) =>
    (NveXxh64Hasher(seed)..update(bytes)).digest();

/// Identity of a UTF-8 name: `xxh64(name, 0)` (Rust `nve_hash::const_id`).
NveKey64 nveConstId(String name) => nveXxh64(utf8.encode(name));
"#;

/// Generates `hosts/flutter-plugin/lib/src/nve_hash.generated.dart`.
pub(super) fn generate_dart_hash() -> String {
    let mut s = banner();
    s.push_str(BODY);
    s.push_str(
        "\n/// Golden vectors shared with every NVE host.\n\
         const List<({List<int> bytes, NveKey64 seed, NveKey64 expected})> nveHashTestVectors = [\n",
    );
    for v in HASH_VECTORS {
        let bytes = byte_list(v.bytes, |b| format!("0x{b:02X}"));
        let ((sl, sh), (el, eh)) = (halves(v.seed), halves(v.expected()));
        writeln!(
            s,
            "  (bytes: <int>[{bytes}], seed: NveKey64(0x{sl:08X}, 0x{sh:08X}), expected: NveKey64(0x{el:08X}, 0x{eh:08X})), // {}",
            v.label
        )
        .unwrap();
    }
    let [a, b, c] = STREAM_SPLITS;
    write!(
        s,
        r#"];

/// `true` when one-shot and chunked digests of every golden vector match Rust.
bool nveHashVerifyTestVectors() {{
  for (final v in nveHashTestVectors) {{
    if (nveXxh64(v.bytes, v.seed) != v.expected) {{
      return false;
    }}
    final h = NveXxh64Hasher(v.seed);
    var at = 0;
    for (final split in const <int>[{a}, {b}, {c}]) {{
      final end = split < v.bytes.length ? split : v.bytes.length;
      if (end > at) {{
        h.update(v.bytes, at, end);
        at = end;
      }}
    }}
    h.update(v.bytes, at);
    if (h.digest() != v.expected) {{
      return false;
    }}
  }}
  return true;
}}
"#
    )
    .unwrap();
    s
}
