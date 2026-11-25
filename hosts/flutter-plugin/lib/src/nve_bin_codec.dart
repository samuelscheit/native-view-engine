/// Shared streaming binary `NveBinValue` encoder & reader matching
/// `crates/nve-abi/src/value.rs` (`encode_bin_value`, tags 0..7, little-endian, max depth 64).
///
/// Pure Dart SDK (zero `package:flutter` imports) so both `nve_flutter` runtime code and
/// generated `nve-codegen` hybrid Dart bindings share one canonical implementation.
import 'dart:convert' show utf8;
import 'dart:typed_data';

import 'nve_core_abi_gen.dart';

/// Owned dynamic value encoded in NVE's compact binary `NveBinValue` format (zero JSON).
class NveHybridDynamicValue {
  NveHybridDynamicValue([Uint8List? binBytes])
      : binBytes = binBytes ?? Uint8List.fromList(<int>[0]);

  final Uint8List binBytes;
}

/// Canonical streaming binary `NveBinValue` encoder matching `nve_abi::encode_bin_value`
/// byte-for-byte.
class NveBinWriter {
  final BytesBuilder _builder = BytesBuilder(copy: false);

  void writeNull() => _builder.addByte(0);

  void writeBool(bool v) {
    _builder.addByte(1);
    _builder.addByte(v ? 1 : 0);
  }

  void writeInt64(int v) {
    _builder.addByte(2);
    final bd = ByteData(8)..setInt64(0, v, Endian.little);
    _builder.add(bd.buffer.asUint8List());
  }

  void writeDouble(double v) {
    _builder.addByte(3);
    final bd = ByteData(8)..setFloat64(0, v, Endian.little);
    _builder.add(bd.buffer.asUint8List());
  }

  void writeString(String v) {
    final bytes = utf8.encode(v);
    _builder.addByte(4);
    _writeU32LE(bytes.length);
    _builder.add(bytes);
  }

  void writeBytes(Uint8List bytes) {
    _builder.addByte(5);
    _writeU32LE(bytes.length);
    _builder.add(bytes);
  }

  void writeRaw(Uint8List raw) => _builder.add(raw);

  void writeArray(int count, void Function(NveBinWriter writer) body) {
    final inner = NveBinWriter();
    inner._writeU32LE(count);
    body(inner);
    final payload = inner.takeBytes();
    _builder.addByte(6);
    _writeU32LE(payload.length);
    _builder.add(payload);
  }

  void writeMap(int count, void Function(NveBinWriter writer) body) {
    final inner = NveBinWriter();
    inner._writeU32LE(count);
    body(inner);
    final payload = inner.takeBytes();
    _builder.addByte(7);
    _writeU32LE(payload.length);
    _builder.add(payload);
  }

  void writeMapKey(String key) {
    final bytes = utf8.encode(key);
    _writeU32LE(bytes.length);
    _builder.add(bytes);
  }

  /// Encodes a dynamic Dart value graph (`null`, `bool`, `int`, `double`, `String`, `Uint8List`,
  /// `List<Object?>`, `Map<String, Object?>`) with a strict nesting depth guard.
  void writeDynamic(
    Object? value, {
    int depth = 0,
    Never Function()? onMaxDepthExceeded,
  }) {
    if (value == null) {
      writeNull();
      return;
    }
    if (value is bool) {
      writeBool(value);
      return;
    }
    if (value is int) {
      writeInt64(value);
      return;
    }
    if (value is double) {
      writeDouble(value);
      return;
    }
    if (value is String) {
      writeString(value);
      return;
    }
    if (value is Uint8List) {
      writeBytes(value);
      return;
    }
    if (value is List<Object?>) {
      if (depth >= nveBinValueMaxDepth) {
        if (onMaxDepthExceeded != null) onMaxDepthExceeded();
        throw StateError('Exceeded maximum nesting depth of $nveBinValueMaxDepth');
      }
      writeArray(value.length, (aw) {
        for (final elem in value) {
          aw.writeDynamic(elem, depth: depth + 1, onMaxDepthExceeded: onMaxDepthExceeded);
        }
      });
      return;
    }
    if (value is Map<String, Object?>) {
      if (depth >= nveBinValueMaxDepth) {
        if (onMaxDepthExceeded != null) onMaxDepthExceeded();
        throw StateError('Exceeded maximum nesting depth of $nveBinValueMaxDepth');
      }
      writeMap(value.length, (mw) {
        for (final entry in value.entries) {
          mw.writeMapKey(entry.key);
          mw.writeDynamic(entry.value, depth: depth + 1, onMaxDepthExceeded: onMaxDepthExceeded);
        }
      });
      return;
    }
    throw ArgumentError('Unsupported NveBinValue type: ${value.runtimeType}');
  }

  Uint8List takeBytes() => _builder.takeBytes();

  void _writeU32LE(int value) {
    final bd = ByteData(4)..setUint32(0, value, Endian.little);
    _builder.add(bd.buffer.asUint8List());
  }
}

/// Bounds-checked streaming reader for compact binary `NveBinValue` payloads.
class NveBinReader {
  NveBinReader(this._bytes) : _view = ByteData.sublistView(_bytes);

  final Uint8List _bytes;
  final ByteData _view;
  int _pos = 0;

  int peekTag() {
    _require(1);
    return _bytes[_pos];
  }

  bool readNullIfPresent() {
    if (peekTag() == 0) {
      _pos += 1;
      return true;
    }
    return false;
  }

  bool readBool() {
    _expectTag(1);
    _require(1);
    return _bytes[_pos++] != 0;
  }

  int readInt64() {
    _expectTag(2);
    _require(8);
    final v = _view.getInt64(_pos, Endian.little);
    _pos += 8;
    return v;
  }

  double readDouble() {
    final tag = peekTag();
    if (tag == 3) {
      _pos += 1;
      _require(8);
      final v = _view.getFloat64(_pos, Endian.little);
      _pos += 8;
      return v;
    }
    if (tag == 2) return readInt64().toDouble();
    throw StateError('expected NveBinValue number');
  }

  String readString() {
    _expectTag(4);
    final len = _readU32();
    _require(len);
    final out = utf8.decode(Uint8List.sublistView(_bytes, _pos, _pos + len));
    _pos += len;
    return out;
  }

  Uint8List readBytes() {
    _expectTag(5);
    final len = _readU32();
    _require(len);
    final out = Uint8List.fromList(Uint8List.sublistView(_bytes, _pos, _pos + len));
    _pos += len;
    return out;
  }

  int beginArray() {
    _expectTag(6);
    final byteLen = _readU32();
    _require(byteLen);
    return _readU32();
  }

  int beginMap() {
    _expectTag(7);
    final byteLen = _readU32();
    _require(byteLen);
    return _readU32();
  }

  String readMapKey() {
    final len = _readU32();
    _require(len);
    final out = utf8.decode(Uint8List.sublistView(_bytes, _pos, _pos + len));
    _pos += len;
    return out;
  }

  Uint8List readRawValue() {
    final start = _pos;
    skipValue();
    return Uint8List.fromList(Uint8List.sublistView(_bytes, start, _pos));
  }

  void skipValue() {
    _require(1);
    final tag = _bytes[_pos++];
    switch (tag) {
      case 0:
        return;
      case 1:
        _require(1);
        _pos += 1;
      case 2:
      case 3:
        _require(8);
        _pos += 8;
      case 4:
      case 5:
      case 6:
      case 7:
        final len = _readU32();
        _require(len);
        _pos += len;
      default:
        throw StateError('invalid NveBinValue tag: $tag');
    }
  }

  void _require(int n) {
    if (n < 0 || _pos + n > _bytes.length) {
      throw StateError('truncated NveBinValue');
    }
  }

  void _expectTag(int expected) {
    _require(1);
    final actual = _bytes[_pos++];
    if (actual != expected) {
      throw StateError('unexpected NveBinValue tag: expected $expected, got $actual');
    }
  }

  int _readU32() {
    _require(4);
    final v = _view.getUint32(_pos, Endian.little);
    _pos += 4;
    return v;
  }
}
