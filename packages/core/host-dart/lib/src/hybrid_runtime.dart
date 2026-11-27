/// Hybrid FFI prelude shared by every generated Dart hybrid client.
library;

import 'dart:typed_data';

import 'bin_codec.dart';
import 'key64.dart';
import 'u64_math.dart';

/// Status of a fallible hybrid FFI trampoline (Rust `NveHybridStatus`, `#[repr(i32)]`).
enum HybridStatus {
  ok(0),
  err(1),
  invalidArg(2),
  schemaMismatch(3),
  nullHandle(4);

  const HybridStatus(this.code);
  final int code;

  /// Status for [code], or `null` for an unknown code.
  static HybridStatus? fromCode(int code) {
    for (final s in values) {
      if (s.code == code) {
        return s;
      }
    }
    return null;
  }
}

/// Status of `NveHostCallbacks::invoke` / `channel_publish` (Rust `NveHostStatus`, `#[repr(i32)]`).
enum HostStatus {
  ok(0),
  pending(1),
  bufferTooSmall(-1),
  unsupported(-2),
  permissionDenied(-3),
  invalidArg(-4),
  notFound(-5),
  cancelled(-6),
  ioError(-7),
  internalError(-8);

  const HostStatus(this.code);
  final int code;

  /// Status for [code], or `null` for an unknown code.
  static HostStatus? fromCode(int code) {
    for (final s in values) {
      if (s.code == code) {
        return s;
      }
    }
    return null;
  }
}

/// A hybrid call failed: bad status, schema mismatch, or a remote error message.
final class HybridException implements Exception {
  const HybridException(this.status, this.message);

  final HybridStatus status;
  final String message;

  /// Throws [HybridStatus.schemaMismatch] unless a peer's schema hash equals the generated one.
  static void checkSchema(Key64 expected, Key64 actual) {
    if (expected != actual) {
      throw HybridException(HybridStatus.schemaMismatch, 'expected $expected, got $actual');
    }
  }

  @override
  String toString() => 'HybridException(${status.name}): $message';
}

/// Owned dynamic value in the compact `NveBinValue` encoding; always exactly one valid value.
final class DynamicValue {
  /// Validated copy of [bytes]; throws [BinCodecException] when invalid.
  factory DynamicValue(List<int> bytes) {
    final copy = Uint8List.fromList(bytes);
    BinReader.validate(copy);
    return DynamicValue._(copy);
  }

  /// Validates and consumes the next `NveBinValue` from [reader], returning an owned copy.
  factory DynamicValue.read(BinReader reader) {
    final raw = reader.readCheckedValue();
    return DynamicValue._(Uint8List.fromList(raw));
  }

  /// Encodes a Dart value graph (`null`, `bool`, `int`, `double`, `String`, `Uint8List`,
  /// [DynamicValue], `List<Object?>`, `Map<String, Object?>`) into a validated [DynamicValue].
  ///
  /// **Dart-web note:** JS represents all numbers as IEEE-754 `double`, so finite integral doubles
  /// within ±(2^53 - 1) (e.g. `1.0`) are indistinguishable from `int` at runtime and encode as
  /// `i64`. Negative zero (`-0.0`), non-integral doubles, infinities, `NaN`, and magnitudes beyond
  /// ±9007199254740991 always encode as `f64` so no double is silently corrupted.
  factory DynamicValue.fromObject(Object? value) {
    if (value == null) {
      return nullValue;
    }
    if (value is DynamicValue) {
      return value;
    }
    final writer = BinWriter();
    _writeObject(writer, value, 0);
    return DynamicValue._(writer.toBytes());
  }

  const DynamicValue._(this._bytes);

  static final DynamicValue nullValue = DynamicValue._(Uint8List.fromList(const <int>[0]));

  final Uint8List _bytes;

  /// Read-only view of the encoded bytes.
  Uint8List get bytes => Uint8List.sublistView(_bytes).asUnmodifiableView();

  BinTag get tag => BinTag.values[_bytes[0]];
  bool get isNull => tag == BinTag.nullValue;
  BinReader get reader => BinReader(_bytes);

  /// Decodes this value into plain Dart objects (`null`, `bool`, `int`, `double`, `String`,
  /// `Uint8List`, `List<Object?>`, `Map<String, Object?>`).
  Object? toObject() => _readObject(reader);

  @override
  bool operator ==(Object other) {
    if (other is! DynamicValue || other._bytes.length != _bytes.length) {
      return false;
    }
    for (var i = 0; i < _bytes.length; i++) {
      if (other._bytes[i] != _bytes[i]) {
        return false;
      }
    }
    return true;
  }

  @override
  int get hashCode => Object.hashAll(_bytes);

  static void _writeObject(BinWriter writer, Object? value, int depth) {
    if (value == null) {
      writer.writeNull();
      return;
    }
    if (value is bool) {
      writer.writeBool(value);
      return;
    }
    if (value is int) {
      // On Dart-web, `-0.0` and huge integral doubles (`1e20`, `double.maxFinite`) satisfy
      // `value is int`. Use `1.0 / value < 0` to detect `-0.0` reliably under dart2js static
      // `int` promotion, and route out-of-safe-integer magnitudes to `writeF64`.
      if (isWeb && ((value == 0 && 1.0 / value < 0) || value < -9007199254740991 || value > 9007199254740991)) {
        writer.writeF64(value.toDouble());
      } else {
        writer.writeI64(value);
      }
      return;
    }
    if (value is double) {
      writer.writeF64(value);
      return;
    }
    if (value is String) {
      writer.writeStr(value);
      return;
    }
    if (value is Uint8List) {
      writer.writeBytes(value);
      return;
    }
    if (value is DynamicValue) {
      if (depth > 0) {
        _checkNestedDepth(value.reader, depth);
      }
      writer.raw(value._bytes);
      return;
    }
    if (value is List<Object?>) {
      if (depth >= binValueMaxDepth) {
        throw BinCodecException(BinErrorKind.maxDepth, 'depth $depth');
      }
      final mark = writer.beginArray(value.length);
      for (final elem in value) {
        _writeObject(writer, elem, depth + 1);
      }
      writer.end(mark);
      return;
    }
    if (value is Map<Object?, Object?>) {
      if (depth >= binValueMaxDepth) {
        throw BinCodecException(BinErrorKind.maxDepth, 'depth $depth');
      }
      final mark = writer.beginMap(value.length);
      for (final entry in value.entries) {
        final key = entry.key;
        if (key is! String) {
          throw ArgumentError('DynamicValue map key must be a String, got ${key.runtimeType}');
        }
        writer.writeMapKey(key);
        _writeObject(writer, entry.value, depth + 1);
      }
      writer.end(mark);
      return;
    }
    throw ArgumentError('Unsupported DynamicValue type: ${value.runtimeType}');
  }

  static void _checkNestedDepth(BinReader r, int depth) {
    switch (r.peekTag()) {
      case BinTag.nullValue:
      case BinTag.boolValue:
      case BinTag.i64:
      case BinTag.f64:
      case BinTag.str:
      case BinTag.bytes:
        r.skipValue();
      case BinTag.array:
        if (depth >= binValueMaxDepth) {
          throw BinCodecException(BinErrorKind.maxDepth, 'depth $depth');
        }
        r.readArrayScope((count) {
          for (var i = 0; i < count; i++) {
            _checkNestedDepth(r, depth + 1);
          }
        });
      case BinTag.map:
        if (depth >= binValueMaxDepth) {
          throw BinCodecException(BinErrorKind.maxDepth, 'depth $depth');
        }
        r.readMapScope((count) {
          for (var i = 0; i < count; i++) {
            r.take(r.u32());
            _checkNestedDepth(r, depth + 1);
          }
        });
    }
  }

  static Object? _readObject(BinReader r) {
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
        return Uint8List.fromList(r.readBytes());
      case BinTag.array:
        return r.readArrayScope((count) {
          final out = <Object?>[];
          for (var i = 0; i < count; i++) {
            out.add(_readObject(r));
          }
          return out;
        });
      case BinTag.map:
        return r.readMapScope((count) {
          final out = <String, Object?>{};
          for (var i = 0; i < count; i++) {
            final key = r.readMapKey();
            out[key] = _readObject(r);
          }
          return out;
        });
    }
  }
}
