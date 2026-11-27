/// Bounds-checked little-endian `BinReader` / `BinWriter` plus the compact tagged `NveBinValue`
/// format (byte-identical to Rust `nve_abi::encode_bin_value`).
///
/// Wire layout (little-endian): Null `[0]` | Bool `[1, 0|1]` | I64 `[2, i64]` | F64 `[3, f64]` |
/// Str `[4, len u32, utf8]` | Bytes `[5, len u32, raw]` | Array `[6, payload u32, count u32, ...]` |
/// Map `[7, payload u32, count u32, (klen u32, key, value)...]`; `payload` counts every byte after
/// the payload field so containers skip in O(1).
///
/// 64-bit integers are read and written as two u32 halves ([Key64]); `ByteData.getInt64` /
/// `getUint64` are never used because they throw on Dart-web.
library;

import 'dart:typed_data';

import 'key64.dart';
import 'utf8.dart';

/// `NveBinValue` tag (Rust `nve_abi::NveValueTag`); `index` is the wire byte.
enum BinTag { nullValue, boolValue, i64, f64, str, bytes, array, map }

/// Maximum container nesting depth (Rust `NVE_BIN_VALUE_MAX_DEPTH`).
const int binValueMaxDepth = 64;

enum BinErrorKind { truncated, unexpectedTag, invalidTag, invalidBool, invalidUtf8, tooLarge, maxDepth, trailingBytes, invalidMark }

final class BinCodecException implements Exception {
  const BinCodecException(this.kind, this.message);

  final BinErrorKind kind;
  final String message;

  @override
  String toString() => 'BinCodecException(${kind.name}): $message';
}

/// Zero-copy cursor over [bytes]; byte slices are views into the same buffer.
final class BinReader {
  BinReader(this.bytes)
      : _data = ByteData.sublistView(bytes),
        _limit = bytes.length;

  final Uint8List bytes;
  final ByteData _data;
  int _pos = 0;

  /// Reads stop here: `bytes.length`, or the enclosing container's end while validating.
  int _limit;

  int get position => _pos;
  int get remaining => _limit - _pos;
  bool get isAtEnd => _pos == _limit;

  // ---- raw little-endian primitives ---------------------------------------------------------

  int u8() => bytes[_take(1)];
  int u16() => _data.getUint16(_take(2), Endian.little);
  int u32() => _data.getUint32(_take(4), Endian.little);
  int i32() => _data.getInt32(_take(4), Endian.little);
  double f32() => _data.getFloat32(_take(4), Endian.little);
  double f64() => _data.getFloat64(_take(8), Endian.little);

  /// Raw 64 bits as exact `(lo, hi)` halves.
  Key64 u64() {
    final at = _take(8);
    return Key64(_data.getUint32(at, Endian.little), _data.getUint32(at + 4, Endian.little));
  }

  /// View of the next [count] bytes (no copy).
  Uint8List take(int count) {
    final at = _take(count);
    return Uint8List.sublistView(bytes, at, at + count);
  }

  /// Next [count] bytes decoded as strictly validated UTF-8 (a leading U+FEFF is kept).
  String utf8String(int count) => decodeUtf8PreserveBom(_utf8Run(count));

  Uint8List _utf8Run(int count) {
    final at = _take(count);
    if (!isValidUtf8(bytes, at, at + count)) {
      throw BinCodecException(BinErrorKind.invalidUtf8, 'at $at');
    }
    return Uint8List.sublistView(bytes, at, at + count);
  }

  // ---- tagged NveBinValue ------------------------------------------------------------------

  BinTag peekTag() {
    if (_pos >= _limit) {
      throw _truncated(1);
    }
    final raw = bytes[_pos];
    if (raw >= BinTag.values.length) {
      throw BinCodecException(BinErrorKind.invalidTag, 'tag $raw');
    }
    return BinTag.values[raw];
  }

  /// Consumes a null and returns `true`; leaves any other value in place.
  bool readNull() {
    if (peekTag() != BinTag.nullValue) {
      return false;
    }
    _pos++;
    return true;
  }

  bool readBool() {
    _expect(BinTag.boolValue);
    final b = u8();
    if (b > 1) {
      throw BinCodecException(BinErrorKind.invalidBool, 'byte $b');
    }
    return b == 1;
  }

  /// Exact two's-complement bits of an `i64`.
  Key64 readI64Bits() {
    _expect(BinTag.i64);
    return u64();
  }

  /// `i64` as an `int`; throws [StateError] on the web when beyond ±2^53 (use [readI64Bits]).
  int readI64() => readI64Bits().toSignedInt();

  /// `f64`, or an `i64` widened to `double` (Rust `BinValueView::as_number`).
  double readF64() {
    if (peekTag() == BinTag.i64) {
      return _i64ToDouble(readI64Bits());
    }
    _expect(BinTag.f64);
    return f64();
  }

  String readStr() {
    _expect(BinTag.str);
    return utf8String(_length());
  }

  Uint8List readBytes() {
    _expect(BinTag.bytes);
    return take(_length());
  }

  /// Element count of an array; rejects counts its payload cannot hold.
  int beginArray() => _openContainer(BinTag.array, 1).$1;

  /// Entry count of a map; read each entry as [readMapKey] then a value.
  int beginMap() => _openContainer(BinTag.map, 5).$1;

  String readMapKey() => utf8String(_length());

  /// Skips one value in O(1).
  void skipValue() {
    final tag = peekTag();
    _pos++;
    switch (tag) {
      case BinTag.nullValue:
        return;
      case BinTag.boolValue:
        _take(1);
      case BinTag.i64:
      case BinTag.f64:
        _take(8);
      case BinTag.str:
      case BinTag.bytes:
      case BinTag.array:
      case BinTag.map:
        _take(_length());
    }
  }

  /// View of the next complete encoded value.
  Uint8List readRawValue() {
    final start = _pos;
    skipValue();
    return Uint8List.sublistView(bytes, start, _pos);
  }

  /// Fully validates and consumes the next value (Rust `BinValueView::parse` rules).
  void checkValue() => _checkValue(0);

  /// Throws unless [bytes] holds exactly one valid `NveBinValue`.
  static void validate(Uint8List bytes) {
    final r = BinReader(bytes)..checkValue();
    if (!r.isAtEnd) {
      throw BinCodecException(BinErrorKind.trailingBytes, '${r.remaining} bytes');
    }
  }

  int _take(int count) {
    if (count < 0 || count > _limit - _pos) {
      throw _truncated(count);
    }
    final at = _pos;
    _pos += count;
    return at;
  }

  int _length() {
    final len = u32();
    if (len > remaining) {
      throw _truncated(len);
    }
    return len;
  }

  BinCodecException _truncated(int need) =>
      BinCodecException(BinErrorKind.truncated, 'need $need at $_pos, have ${_limit - _pos}');

  void _expect(BinTag tag) {
    final actual = peekTag();
    if (actual != tag) {
      throw BinCodecException(BinErrorKind.unexpectedTag, 'expected ${tag.name}, got ${actual.name}');
    }
    _pos++;
  }

  (int, int) _openContainer(BinTag tag, int minEntryBytes) {
    _expect(tag);
    final payload = _length();
    if (payload < 4) {
      throw _truncated(4);
    }
    final end = _pos + payload;
    final count = u32();
    if (count > (payload - 4) ~/ minEntryBytes) {
      throw _truncated(count);
    }
    return (count, end);
  }

  void _checkValue(int depth) {
    final tag = peekTag();
    switch (tag) {
      case BinTag.nullValue:
      case BinTag.i64:
      case BinTag.f64:
      case BinTag.bytes:
        skipValue();
      case BinTag.boolValue:
        readBool();
      case BinTag.str:
        _expect(BinTag.str);
        _utf8Run(_length()); // validated in place, never decoded
      case BinTag.array:
      case BinTag.map:
        if (depth >= binValueMaxDepth) {
          throw BinCodecException(BinErrorKind.maxDepth, 'depth $depth');
        }
        final isMap = tag == BinTag.map;
        final (count, end) = _openContainer(tag, isMap ? 5 : 1);
        // Like Rust's `&bytes[..container_end]`: no child read may escape its container.
        final outer = _limit;
        _limit = end;
        try {
          for (var i = 0; i < count; i++) {
            if (isMap) {
              _utf8Run(_length());
            }
            _checkValue(depth + 1);
          }
        } finally {
          _limit = outer;
        }
        if (_pos != end) {
          throw BinCodecException(BinErrorKind.trailingBytes, '${end - _pos} bytes');
        }
    }
  }
}

/// `i64` bits widened to the nearest `double` (exact up to 2^53, rounded beyond, like Rust `as f64`).
double _i64ToDouble(Key64 bits) {
  final negative = bits.hi >= 0x80000000;
  final hi = negative ? bits.hi - 0x100000000 : bits.hi;
  return hi * 4294967296.0 + bits.lo;
}

/// Little-endian writer; reuse one instance with [clear] (keeps capacity).
final class BinWriter {
  BinWriter([int capacity = 64]) : _buf = Uint8List(capacity < 16 ? 16 : capacity) {
    _data = ByteData.sublistView(_buf);
  }

  Uint8List _buf;
  late ByteData _data;
  int _len = 0;

  int get length => _len;

  /// View of the written bytes (valid until the next write).
  Uint8List get bytes => Uint8List.sublistView(_buf, 0, _len);

  /// Copy of the written bytes.
  Uint8List toBytes() => Uint8List.fromList(bytes);

  void clear() => _len = 0;

  // ---- raw little-endian primitives ---------------------------------------------------------

  void u8(int v) {
    _ensure(1);
    _buf[_len++] = v & 0xFF;
  }

  void u16(int v) {
    _ensure(2);
    _data.setUint16(_len, v & 0xFFFF, Endian.little);
    _len += 2;
  }

  void u32(int v) {
    _ensure(4);
    _data.setUint32(_len, v & 0xFFFFFFFF, Endian.little);
    _len += 4;
  }

  void i32(int v) => u32(v);

  /// 64 raw bits from exact halves.
  void u64(Key64 v) {
    u32(v.lo);
    u32(v.hi);
  }

  void f32(double v) {
    _ensure(4);
    _data.setFloat32(_len, v, Endian.little);
    _len += 4;
  }

  void f64(double v) {
    _ensure(8);
    _data.setFloat64(_len, v, Endian.little);
    _len += 8;
  }

  void raw(List<int> b) {
    _ensure(b.length);
    _buf.setRange(_len, _len + b.length, b);
    _len += b.length;
  }

  // ---- tagged NveBinValue ------------------------------------------------------------------

  void writeNull() => u8(BinTag.nullValue.index);

  void writeBool(bool v) {
    u8(BinTag.boolValue.index);
    u8(v ? 1 : 0);
  }

  /// `i64` from an `int` (exact for every VM `int` and every integral web double).
  void writeI64(int v) => writeI64Bits(Key64.fromInt(v));

  /// `i64` from exact two's-complement halves.
  void writeI64Bits(Key64 bits) {
    u8(BinTag.i64.index);
    u64(bits);
  }

  void writeF64(double v) {
    u8(BinTag.f64.index);
    f64(v);
  }

  /// Throws [BinCodecException] (`invalidUtf8`) on a lone surrogate, writing nothing.
  void writeStr(String v) {
    final n = utf8EncodedLength(v);
    u8(BinTag.str.index);
    _writeEncoded(v, n);
  }

  void writeBytes(List<int> v) {
    u8(BinTag.bytes.index);
    u32(v.length);
    raw(v);
  }

  /// Length-prefixed UTF-8 encoded in place (no temporary list); throws [BinCodecException]
  /// (`invalidUtf8`) on a lone surrogate instead of writing U+FFFD.
  void writeMapKey(String key) => _writeEncoded(key, utf8EncodedLength(key));

  void _writeEncoded(String s, int n) {
    _ensure(4 + n);
    u32(n);
    for (var i = 0; i < s.length; i += utf16UnitsAt(s, i)) {
      _len = encodeUtf8CodePointAt(s, i, _buf, _len);
    }
  }

  /// Opens an array of [count] values; returns the mark for [end].
  int beginArray(int count) => _begin(BinTag.array, count);

  /// Opens a map of [count] entries ([writeMapKey] + value each); returns the mark for [end].
  int beginMap(int count) => _begin(BinTag.map, count);

  /// Patches the payload length of the container opened at [mark] (stale marks are rejected).
  void end(int mark) {
    if (mark < 0 || mark > _len - 8) {
      throw BinCodecException(BinErrorKind.invalidMark, 'mark $mark of $_len');
    }
    _data.setUint32(mark, _len - mark - 4, Endian.little);
  }

  int _begin(BinTag tag, int count) {
    u8(tag.index);
    final mark = _len;
    u32(0);
    u32(count);
    return mark;
  }

  void _ensure(int extra) {
    if (_len + extra <= _buf.length) {
      return;
    }
    if (_len + extra > 0xFFFFFFFF) {
      throw BinCodecException(BinErrorKind.tooLarge, 'size ${_len + extra}');
    }
    var next = _buf.length * 2;
    while (next < _len + extra) {
      next *= 2;
    }
    _buf = Uint8List(next)..setRange(0, _len, _buf);
    _data = ByteData.sublistView(_buf);
  }
}
