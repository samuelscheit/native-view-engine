/// Zero-copy binary `NveBinValue` wire decoder and encoder matching
/// `crates/nve-abi/src/value.rs` (tags 0..7, little-endian, max depth 64).
import 'dart:convert';
import 'dart:typed_data';

/// Maximum nesting depth permitted by `NveBinValue` (`NVE_BIN_VALUE_MAX_DEPTH`).
const int nveBinValueMaxDepth = 64;

/// Binary `NveBinValue` wire tags `0..7`.
enum NveBinValueTag {
  nullValue(0),
  boolValue(1),
  i64(2),
  f64(3),
  str(4),
  bytes(5),
  array(6),
  map(7);

  const NveBinValueTag(this.wireCode);
  final int wireCode;

  static NveBinValueTag? fromWireCode(int code) {
    if (code < 0 || code >= values.length) {
      return null;
    }
    return values[code];
  }
}

/// Error kind thrown when decoding or encoding a malformed `NveBinValue` buffer.
enum NveBinValueErrorKind {
  truncated,
  invalidTag,
  invalidBool,
  invalidUtf8,
  maxDepthExceeded,
  trailingBytes,
}

/// Exception raised on invalid `NveBinValue` binary payloads.
class NveBinValueException implements Exception {
  const NveBinValueException(this.kind, this.message);

  final NveBinValueErrorKind kind;
  final String message;

  @override
  String toString() => 'NveBinValueException($kind): $message';
}

/// Zero-copy borrowed view over a validated binary `NveBinValue` buffer.
///
/// Sub-elements and sub-buffers are sliced via `Uint8List.sublistView` (zero heap copy), and
/// container skipping during `arrayAt` / `get` runs in `O(1)` using each container's
/// `payload_byte_len` header.
class NveBinValueView {
  NveBinValueView._(this._bytes, this.tag) : _data = ByteData.sublistView(_bytes);

  final Uint8List _bytes;
  final ByteData _data;
  final NveBinValueTag tag;

  /// Validates `bytes` once (tags 0..7, bool 0|1, strict UTF-8, bounds, depth <= 64,
  /// zero trailing bytes) and returns a zero-copy view.
  factory NveBinValueView.parse(Uint8List bytes) {
    final ByteData data = ByteData.sublistView(bytes);
    final _ValidatedSpan span = _validateValueAt(bytes, data, 0, bytes.length, 0);
    if (span.end != bytes.length) {
      throw NveBinValueException(
        NveBinValueErrorKind.trailingBytes,
        'Unexpected ${bytes.length - span.end} trailing byte(s)',
      );
    }
    return NveBinValueView._(bytes, span.tag);
  }

  Uint8List get rawBytes => _bytes;

  bool get isNull => tag == NveBinValueTag.nullValue;

  bool? asBool() => tag == NveBinValueTag.boolValue ? _bytes[1] != 0 : null;

  int? asInt64() => tag == NveBinValueTag.i64 ? _data.getInt64(1, Endian.little) : null;

  double? asFloat64() => tag == NveBinValueTag.f64 ? _data.getFloat64(1, Endian.little) : null;

  double? asNumber() {
    if (tag == NveBinValueTag.f64) {
      return _data.getFloat64(1, Endian.little);
    }
    if (tag == NveBinValueTag.i64) {
      return _data.getInt64(1, Endian.little).toDouble();
    }
    return null;
  }

  Uint8List? asUtf8Bytes() {
    if (tag != NveBinValueTag.str) {
      return null;
    }
    final int len = _data.getUint32(1, Endian.little);
    return Uint8List.sublistView(_bytes, 5, 5 + len);
  }

  String? asString() {
    final Uint8List? slice = asUtf8Bytes();
    if (slice == null) {
      return null;
    }
    return utf8.decode(slice, allowMalformed: false);
  }

  Uint8List? asBytes() {
    if (tag != NveBinValueTag.bytes) {
      return null;
    }
    final int len = _data.getUint32(1, Endian.little);
    return Uint8List.sublistView(_bytes, 5, 5 + len);
  }

  int? get arrayLength =>
      tag == NveBinValueTag.array ? _data.getUint32(5, Endian.little) : null;

  /// Returns element `index` as a zero-copy view, skipping nested containers in `O(1)`.
  NveBinValueView? arrayAt(int index) {
    final int? count = arrayLength;
    if (count == null || index < 0 || index >= count) {
      return null;
    }
    int pos = 9;
    for (int i = 0; i <= index; i++) {
      final _ValidatedSpan? span = _skipValidatedValue(_bytes, _data, pos);
      if (span == null) {
        return null;
      }
      if (i == index) {
        return NveBinValueView._(Uint8List.sublistView(_bytes, pos, span.end), span.tag);
      }
      pos = span.end;
    }
    return null;
  }

  int? get mapLength => tag == NveBinValueTag.map ? _data.getUint32(5, Endian.little) : null;

  /// Looks up `key` inside a map without decoding non-matching entries (`O(1)` subtree skip).
  NveBinValueView? get(String key) {
    final int? count = mapLength;
    if (count == null) {
      return null;
    }
    final Uint8List keyUtf8 = Uint8List.fromList(utf8.encode(key));
    int pos = 9;
    for (int i = 0; i < count; i++) {
      final int klen = _data.getUint32(pos, Endian.little);
      pos += 4;
      final int kend = pos + klen;
      bool matches = klen == keyUtf8.length;
      if (matches) {
        for (int b = 0; b < klen; b++) {
          if (_bytes[pos + b] != keyUtf8[b]) {
            matches = false;
            break;
          }
        }
      }
      pos = kend;
      final _ValidatedSpan? valSpan = _skipValidatedValue(_bytes, _data, pos);
      if (valSpan == null) {
        return null;
      }
      if (matches) {
        return NveBinValueView._(
          Uint8List.sublistView(_bytes, pos, valSpan.end),
          valSpan.tag,
        );
      }
      pos = valSpan.end;
    }
    return null;
  }

  void forEachMapEntry(void Function(String key, NveBinValueView value) visit) {
    final int? count = mapLength;
    if (count == null) {
      return;
    }
    int pos = 9;
    for (int i = 0; i < count; i++) {
      final int klen = _data.getUint32(pos, Endian.little);
      pos += 4;
      final int kend = pos + klen;
      final String key = utf8.decode(
        Uint8List.sublistView(_bytes, pos, kend),
        allowMalformed: false,
      );
      pos = kend;
      final _ValidatedSpan? valSpan = _skipValidatedValue(_bytes, _data, pos);
      if (valSpan == null) {
        return;
      }
      visit(
        key,
        NveBinValueView._(
          Uint8List.sublistView(_bytes, pos, valSpan.end),
          valSpan.tag,
        ),
      );
      pos = valSpan.end;
    }
  }

  static _ValidatedSpan _validateValueAt(
    Uint8List bytes,
    ByteData data,
    int start,
    int limit,
    int depth,
  ) {
    if (start >= limit) {
      throw const NveBinValueException(
        NveBinValueErrorKind.truncated,
        'Unexpected end of buffer while reading value tag',
      );
    }
    final int rawTag = bytes[start];
    final NveBinValueTag? tag = NveBinValueTag.fromWireCode(rawTag);
    if (tag == null) {
      throw NveBinValueException(
        NveBinValueErrorKind.invalidTag,
        'Unknown NveBinValue tag: $rawTag',
      );
    }
    int pos = start + 1;

    switch (tag) {
      case NveBinValueTag.nullValue:
        return _ValidatedSpan(tag, pos);

      case NveBinValueTag.boolValue:
        if (pos >= limit) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Truncated bool payload',
          );
        }
        final int b = bytes[pos];
        if (b > 1) {
          throw NveBinValueException(
            NveBinValueErrorKind.invalidBool,
            'Invalid bool byte: $b',
          );
        }
        return _ValidatedSpan(tag, pos + 1);

      case NveBinValueTag.i64:
      case NveBinValueTag.f64:
        if (pos + 8 > limit) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Truncated 64-bit numeric payload',
          );
        }
        return _ValidatedSpan(tag, pos + 8);

      case NveBinValueTag.str:
        if (pos + 4 > limit) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Truncated string length',
          );
        }
        final int len = data.getUint32(pos, Endian.little);
        pos += 4;
        if (len > limit - pos) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Truncated string bytes',
          );
        }
        try {
          utf8.decode(Uint8List.sublistView(bytes, pos, pos + len), allowMalformed: false);
        } on FormatException catch (e) {
          throw NveBinValueException(
            NveBinValueErrorKind.invalidUtf8,
            'Invalid UTF-8 string: ${e.message}',
          );
        }
        return _ValidatedSpan(tag, pos + len);

      case NveBinValueTag.bytes:
        if (pos + 4 > limit) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Truncated bytes length',
          );
        }
        final int len = data.getUint32(pos, Endian.little);
        pos += 4;
        if (len > limit - pos) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Truncated raw bytes payload',
          );
        }
        return _ValidatedSpan(tag, pos + len);

      case NveBinValueTag.array:
        if (depth >= nveBinValueMaxDepth) {
          throw const NveBinValueException(
            NveBinValueErrorKind.maxDepthExceeded,
            'Exceeded maximum nesting depth of $nveBinValueMaxDepth',
          );
        }
        if (pos + 4 > limit) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Truncated array payload length',
          );
        }
        final int payloadLen = data.getUint32(pos, Endian.little);
        pos += 4;
        if (payloadLen < 4 || payloadLen > limit - pos) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Array payload length exceeds buffer bounds',
          );
        }
        final int containerEnd = pos + payloadLen;
        final int count = data.getUint32(pos, Endian.little);
        pos += 4;
        if (count > containerEnd - pos) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Array element count exceeds payload bytes',
          );
        }
        for (int i = 0; i < count; i++) {
          final _ValidatedSpan elem = _validateValueAt(bytes, data, pos, containerEnd, depth + 1);
          pos = elem.end;
        }
        if (pos != containerEnd) {
          throw NveBinValueException(
            NveBinValueErrorKind.trailingBytes,
            'Array has ${containerEnd - pos} trailing byte(s)',
          );
        }
        return _ValidatedSpan(tag, containerEnd);

      case NveBinValueTag.map:
        if (depth >= nveBinValueMaxDepth) {
          throw const NveBinValueException(
            NveBinValueErrorKind.maxDepthExceeded,
            'Exceeded maximum nesting depth of $nveBinValueMaxDepth',
          );
        }
        if (pos + 4 > limit) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Truncated map payload length',
          );
        }
        final int payloadLen = data.getUint32(pos, Endian.little);
        pos += 4;
        if (payloadLen < 4 || payloadLen > limit - pos) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Map payload length exceeds buffer bounds',
          );
        }
        final int containerEnd = pos + payloadLen;
        final int count = data.getUint32(pos, Endian.little);
        pos += 4;
        if (count > (containerEnd - pos) ~/ 5) {
          throw const NveBinValueException(
            NveBinValueErrorKind.truncated,
            'Map entry count exceeds payload bytes',
          );
        }
        for (int i = 0; i < count; i++) {
          if (pos + 4 > containerEnd) {
            throw const NveBinValueException(
              NveBinValueErrorKind.truncated,
              'Truncated map key length',
            );
          }
          final int klen = data.getUint32(pos, Endian.little);
          pos += 4;
          if (klen > containerEnd - pos) {
            throw const NveBinValueException(
              NveBinValueErrorKind.truncated,
              'Truncated map key bytes',
            );
          }
          try {
            utf8.decode(Uint8List.sublistView(bytes, pos, pos + klen), allowMalformed: false);
          } on FormatException catch (e) {
            throw NveBinValueException(
              NveBinValueErrorKind.invalidUtf8,
              'Invalid UTF-8 map key: ${e.message}',
            );
          }
          pos += klen;
          final _ValidatedSpan val = _validateValueAt(bytes, data, pos, containerEnd, depth + 1);
          pos = val.end;
        }
        if (pos != containerEnd) {
          throw NveBinValueException(
            NveBinValueErrorKind.trailingBytes,
            'Map has ${containerEnd - pos} trailing byte(s)',
          );
        }
        return _ValidatedSpan(tag, containerEnd);
    }
  }

  static _ValidatedSpan? _skipValidatedValue(Uint8List bytes, ByteData data, int start) {
    if (start >= bytes.length) {
      return null;
    }
    final NveBinValueTag? tag = NveBinValueTag.fromWireCode(bytes[start]);
    if (tag == null) {
      return null;
    }
    final int pos = start + 1;
    final int end;
    switch (tag) {
      case NveBinValueTag.nullValue:
        end = pos;
      case NveBinValueTag.boolValue:
        end = pos + 1;
      case NveBinValueTag.i64:
      case NveBinValueTag.f64:
        end = pos + 8;
      case NveBinValueTag.str:
      case NveBinValueTag.bytes:
      case NveBinValueTag.array:
      case NveBinValueTag.map:
        final int len = data.getUint32(pos, Endian.little);
        end = pos + 4 + len;
    }
    return end <= bytes.length ? _ValidatedSpan(tag, end) : null;
  }
}

class _ValidatedSpan {
  const _ValidatedSpan(this.tag, this.end);
  final NveBinValueTag tag;
  final int end;
}

/// Compact binary encoder producing `NveBinValue` wire payloads from Dart values.
class NveBinValueEncoder {
  static Uint8List encode(Object? value) {
    final BytesBuilder builder = BytesBuilder(copy: false);
    _encodeDepth(value, builder, 0);
    return builder.takeBytes();
  }

  static void _encodeDepth(Object? value, BytesBuilder out, int depth) {
    if (value == null) {
      out.addByte(NveBinValueTag.nullValue.wireCode);
      return;
    }
    if (value is bool) {
      out.addByte(NveBinValueTag.boolValue.wireCode);
      out.addByte(value ? 1 : 0);
      return;
    }
    if (value is int) {
      out.addByte(NveBinValueTag.i64.wireCode);
      final ByteData bd = ByteData(8)..setInt64(0, value, Endian.little);
      out.add(bd.buffer.asUint8List());
      return;
    }
    if (value is double) {
      out.addByte(NveBinValueTag.f64.wireCode);
      final ByteData bd = ByteData(8)..setFloat64(0, value, Endian.little);
      out.add(bd.buffer.asUint8List());
      return;
    }
    if (value is String) {
      final Uint8List utf8Bytes = Uint8List.fromList(utf8.encode(value));
      out.addByte(NveBinValueTag.str.wireCode);
      _writeU32LE(out, utf8Bytes.length);
      out.add(utf8Bytes);
      return;
    }
    if (value is Uint8List) {
      out.addByte(NveBinValueTag.bytes.wireCode);
      _writeU32LE(out, value.length);
      out.add(value);
      return;
    }
    if (value is List<Object?>) {
      if (depth >= nveBinValueMaxDepth) {
        throw const NveBinValueException(
          NveBinValueErrorKind.maxDepthExceeded,
          'Exceeded maximum nesting depth of $nveBinValueMaxDepth',
        );
      }
      final BytesBuilder payload = BytesBuilder(copy: false);
      _writeU32LE(payload, value.length);
      for (final Object? elem in value) {
        _encodeDepth(elem, payload, depth + 1);
      }
      final Uint8List payloadBytes = payload.takeBytes();
      out.addByte(NveBinValueTag.array.wireCode);
      _writeU32LE(out, payloadBytes.length);
      out.add(payloadBytes);
      return;
    }
    if (value is Map<String, Object?>) {
      if (depth >= nveBinValueMaxDepth) {
        throw const NveBinValueException(
          NveBinValueErrorKind.maxDepthExceeded,
          'Exceeded maximum nesting depth of $nveBinValueMaxDepth',
        );
      }
      final BytesBuilder payload = BytesBuilder(copy: false);
      _writeU32LE(payload, value.length);
      for (final MapEntry<String, Object?> entry in value.entries) {
        final Uint8List kb = Uint8List.fromList(utf8.encode(entry.key));
        _writeU32LE(payload, kb.length);
        payload.add(kb);
        _encodeDepth(entry.value, payload, depth + 1);
      }
      final Uint8List payloadBytes = payload.takeBytes();
      out.addByte(NveBinValueTag.map.wireCode);
      _writeU32LE(out, payloadBytes.length);
      out.add(payloadBytes);
      return;
    }
    throw ArgumentError('Unsupported NveBinValue type: ${value.runtimeType}');
  }

  static void _writeU32LE(BytesBuilder out, int value) {
    final ByteData bd = ByteData(4)..setUint32(0, value, Endian.little);
    out.add(bd.buffer.asUint8List());
  }
}
