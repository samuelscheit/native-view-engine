/// Untyped Dart ⇄ `NveBinValue` bridge for host plumbing (platform-channel payloads, custom
/// event values, props maps). Typed generated code uses [BinWriter] / [BinReader] directly.
library;

import 'dart:typed_data';

import 'bin_codec.dart';
import 'hybrid_runtime.dart';

extension BinWriterDynamic on BinWriter {
  /// Writes `null`, `bool`, `int`, `double`, `String`, `Uint8List`, [DynamicValue], `List`, or a
  /// `Map` with `String` keys; throws [ArgumentError] for anything else and [BinCodecException]
  /// (`maxDepth`) past [binValueMaxDepth] nested containers.
  void writeDynamic(Object? value) => _writeDynamic(this, value, 0);
}

void _writeDynamic(BinWriter w, Object? value, int depth) {
  switch (value) {
    case null:
      w.writeNull();
    case final bool v:
      w.writeBool(v);
    case final num v when encodesAsI64(v):
      w.writeI64(v as int);
    case final num v:
      w.writeF64(v.toDouble());
    case final String v:
      w.writeStr(v);
    case final Uint8List v:
      w.writeBytes(v);
    case final DynamicValue v:
      w.raw(v.bytes);
    case final List<Object?> v:
      checkBinValueDepth(depth);
      final mark = w.beginArray(v.length);
      for (final e in v) {
        _writeDynamic(w, e, depth + 1);
      }
      w.end(mark);
    case final Map<Object?, Object?> v:
      checkBinValueDepth(depth);
      final mark = w.beginMap(v.length);
      for (final MapEntry(:key, value: e) in v.entries) {
        if (key is! String) {
          throw ArgumentError.value(key, 'key', 'NveBinValue map keys must be strings');
        }
        w.writeMapKey(key);
        _writeDynamic(w, e, depth + 1);
      }
      w.end(mark);
    default:
      throw ArgumentError.value(value, 'value', 'not encodable as NveBinValue');
  }
}

extension BinReaderDynamic on BinReader {
  /// Reads the next value as `null` / `bool` / `int` / `double` / `String` / `Uint8List` view /
  /// `List<Object?>` / `Map<String, Object?>` (map entry order preserved).
  Object? readDynamic() => _readDynamic(this, 0);
}

Object? _readDynamic(BinReader r, int depth) {
  switch (r.peekTag()) {
    case BinTag.nullValue:
      r.readNull();
      return null;
    case BinTag.boolValue:
      return r.readBool();
    case BinTag.i64:
      return r.readI64();
    case BinTag.f64:
      return r.readF64();
    case BinTag.str:
      return r.readStr();
    case BinTag.bytes:
      return r.readBytes();
    case BinTag.array:
      checkBinValueDepth(depth);
      final n = r.beginArray();
      return List<Object?>.generate(n, (_) => _readDynamic(r, depth + 1), growable: false);
    case BinTag.map:
      checkBinValueDepth(depth);
      final n = r.beginMap();
      final out = <String, Object?>{};
      for (var i = 0; i < n; i++) {
        final key = r.readMapKey();
        out[key] = _readDynamic(r, depth + 1);
      }
      return out;
  }
}

/// `NveBinValue` bytes of [value] (see [BinWriterDynamic.writeDynamic]).
Uint8List encodeDynamic(Object? value) => (BinWriter()..writeDynamic(value)).toBytes();

/// The single validated value in [bytes] (see [BinReaderDynamic.readDynamic]).
Object? decodeDynamic(Uint8List bytes) {
  BinReader.validate(bytes);
  return BinReader(bytes).readDynamic();
}
