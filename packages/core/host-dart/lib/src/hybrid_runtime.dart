/// Hybrid FFI prelude shared by every generated Dart hybrid client.
library;

import 'dart:typed_data';

import 'bin_codec.dart';
import 'key64.dart';

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

  const DynamicValue._(this._bytes);

  static final DynamicValue nullValue = DynamicValue._(Uint8List.fromList(const <int>[0]));

  final Uint8List _bytes;

  /// Read-only view of the encoded bytes.
  Uint8List get bytes => Uint8List.sublistView(_bytes).asUnmodifiableView();

  BinTag get tag => BinTag.values[_bytes[0]];
  bool get isNull => tag == BinTag.nullValue;
  BinReader get reader => BinReader(_bytes);

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
}
