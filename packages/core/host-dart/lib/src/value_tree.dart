/// Native `NveValue` FFI tree builder and direct hybrid view props encoder backed by [HybridArena].
///
/// Matches `nve::host::NveValueTree` in `packages/core/host-cxx`: materializes `DynamicValue` and
/// Dart object graphs into true C-ABI `NveValue` trees (`NULL`/`BOOL`/`I64`/`F64`/`STR`/`BYTES`/
/// `ARRAY`/`MAP`) inside a single scoped [HybridArena] so `HybridType::Value` props and parameters
/// cross FFI losslessly without ever double-wrapping in `NVE_VAL_BYTES`.
library;

import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'bin_codec.dart';
import 'generated/nve_abi_const.generated.dart';
import 'generated/nve_abi_ffi.generated.dart';
import 'hybrid_ffi.dart';
import 'hybrid_runtime.dart';
import 'utf8.dart';

/// Native `NveValue` tree allocated inside a caller-owned [HybridArena].
///
/// Strings, bytes, map keys, and nested container arrays (`NveValue[]` / `NveMapEntry[]`) all
/// borrow the arena's slabs and are freed deterministically when `HybridArena.using` finishes.
final class NveValueTree {
  NveValueTree._(this.rootPtr);

  /// Materializes a validated [DynamicValue] into [arena] with its true `NveValue` tags.
  factory NveValueTree.fromDynamic(HybridArena arena, DynamicValue value) {
    final slot = arena.allocStruct<NveValue>(ffi.sizeOf<NveValue>());
    writeDynamic(arena, slot.ref, value);
    return NveValueTree._(slot);
  }

  /// Validates and materializes the next `NveBinValue` from [reader] into [arena].
  factory NveValueTree.fromReader(HybridArena arena, BinReader reader) {
    final slot = arena.allocStruct<NveValue>(ffi.sizeOf<NveValue>());
    writeFromReader(arena, slot.ref, reader);
    return NveValueTree._(slot);
  }

  /// Materializes a Dart value graph (`null`, `bool`, `int`, `double`, `String`, `Uint8List`,
  /// [DynamicValue], `List<Object?>`, `Map<String, Object?>`) directly into [arena].
  factory NveValueTree.fromObject(HybridArena arena, Object? value) {
    final slot = arena.allocStruct<NveValue>(ffi.sizeOf<NveValue>());
    writeObject(arena, slot.ref, value);
    return NveValueTree._(slot);
  }

  /// Pointer to the root `NveValue` inside the backing [HybridArena].
  final ffi.Pointer<NveValue> rootPtr;

  /// Root `NveValue` struct by value (for C ABI signatures taking `NveValue`).
  NveValue get root => rootPtr.ref;

  /// Writes a validated [DynamicValue] directly into [target] using [arena] for nested storage.
  static void writeDynamic(HybridArena arena, NveValue target, DynamicValue value, {int depth = 0}) {
    _writeValidated(arena, target, value.reader, depth);
  }

  /// Validates the next `NveBinValue` from [reader] and writes it directly into [target].
  static void writeFromReader(HybridArena arena, NveValue target, BinReader reader, {int depth = 0}) {
    final raw = reader.readCheckedValue();
    _writeValidated(arena, target, BinReader(raw), depth);
  }

  /// Materializes a Dart value graph directly into [target] using [arena] for nested storage.
  static void writeObject(HybridArena arena, NveValue target, Object? value, {int depth = 0}) {
    if (value == null) {
      target.tag = NveValueTag.nullValue.code;
      target.data.i64Val = 0;
      return;
    }
    if (value is bool) {
      target.tag = NveValueTag.boolValue.code;
      target.data.boolVal = value ? 1 : 0;
      return;
    }
    if (value is int) {
      target.tag = NveValueTag.i64.code;
      target.data.i64Val = value;
      return;
    }
    if (value is double) {
      target.tag = NveValueTag.f64.code;
      target.data.f64Val = value;
      return;
    }
    if (value is String) {
      target.tag = NveValueTag.str.code;
      arena.writeStrSlice(target.data.strVal, value);
      return;
    }
    if (value is Uint8List) {
      target.tag = NveValueTag.bytes.code;
      arena.writeBytesSlice(target.data.bytesVal, value);
      return;
    }
    if (value is DynamicValue) {
      _writeValidated(arena, target, value.reader, depth);
      return;
    }
    if (value is List<Object?>) {
      if (depth >= binValueMaxDepth) {
        throw BinCodecException(BinErrorKind.maxDepth, 'depth $depth');
      }
      target.tag = NveValueTag.array.code;
      if (value.isEmpty) {
        target.data.arrayVal
          ..ptr = ffi.nullptr
          ..len = 0;
        return;
      }
      final itemsPtr = arena.allocStructArray<NveValue>(ffi.sizeOf<NveValue>(), value.length);
      for (var i = 0; i < value.length; i++) {
        writeObject(arena, itemsPtr[i], value[i], depth: depth + 1);
      }
      target.data.arrayVal
        ..ptr = itemsPtr
        ..len = value.length;
      return;
    }
    if (value is Map<Object?, Object?>) {
      if (depth >= binValueMaxDepth) {
        throw BinCodecException(BinErrorKind.maxDepth, 'depth $depth');
      }
      target.tag = NveValueTag.map.code;
      if (value.isEmpty) {
        target.data.mapVal
          ..ptr = ffi.nullptr
          ..len = 0;
        return;
      }
      final entriesPtr = arena.allocStructArray<NveMapEntry>(ffi.sizeOf<NveMapEntry>(), value.length);
      var idx = 0;
      for (final entry in value.entries) {
        final key = entry.key;
        if (key is! String) {
          throw ArgumentError('NveMapEntry key must be a String, got ${key.runtimeType}');
        }
        arena.writeStrSlice(entriesPtr[idx].key, key);
        writeObject(arena, entriesPtr[idx].value, entry.value, depth: depth + 1);
        idx++;
      }
      target.data.mapVal
        ..ptr = entriesPtr
        ..len = idx;
      return;
    }
    throw ArgumentError('Unsupported NveValue type: ${value.runtimeType}');
  }

  /// Encodes a native [NveValue] tree back into a validated [DynamicValue] (`NveBinValue` bytes).
  static DynamicValue toDynamicValue(NveValue value) {
    final writer = BinWriter();
    _encodeNativeValue(writer, value, 0);
    return DynamicValue(writer.bytes);
  }

  static void _writeValidated(HybridArena arena, NveValue target, BinReader r, int depth) {
    switch (r.peekTag()) {
      case BinTag.nullValue:
        r.readNull();
        target.tag = NveValueTag.nullValue.code;
        target.data.i64Val = 0;
      case BinTag.boolValue:
        target.tag = NveValueTag.boolValue.code;
        target.data.boolVal = r.readBool() ? 1 : 0;
      case BinTag.i64:
        target.tag = NveValueTag.i64.code;
        target.data.i64Val = r.readI64();
      case BinTag.f64:
        target.tag = NveValueTag.f64.code;
        target.data.f64Val = r.readF64();
      case BinTag.str:
        // Copy pre-validated UTF-8 bytes directly into the arena (no UTF-16 round-trip).
        r.u8();
        final len = r.u32();
        target.tag = NveValueTag.str.code;
        arena.writeUtf8BytesSlice(target.data.strVal, r.take(len));
      case BinTag.bytes:
        target.tag = NveValueTag.bytes.code;
        arena.writeBytesSlice(target.data.bytesVal, r.readBytes());
      case BinTag.array:
        if (depth >= binValueMaxDepth) {
          throw BinCodecException(BinErrorKind.maxDepth, 'depth $depth');
        }
        target.tag = NveValueTag.array.code;
        r.readArrayScope((count) {
          if (count == 0) {
            target.data.arrayVal
              ..ptr = ffi.nullptr
              ..len = 0;
            return;
          }
          final itemsPtr = arena.allocStructArray<NveValue>(ffi.sizeOf<NveValue>(), count);
          for (var i = 0; i < count; i++) {
            _writeValidated(arena, itemsPtr[i], r, depth + 1);
          }
          target.data.arrayVal
            ..ptr = itemsPtr
            ..len = count;
        });
      case BinTag.map:
        if (depth >= binValueMaxDepth) {
          throw BinCodecException(BinErrorKind.maxDepth, 'depth $depth');
        }
        target.tag = NveValueTag.map.code;
        r.readMapScope((count) {
          if (count == 0) {
            target.data.mapVal
              ..ptr = ffi.nullptr
              ..len = 0;
            return;
          }
          final entriesPtr = arena.allocStructArray<NveMapEntry>(ffi.sizeOf<NveMapEntry>(), count);
          for (var i = 0; i < count; i++) {
            final klen = r.u32();
            arena.writeUtf8BytesSlice(entriesPtr[i].key, r.take(klen));
            _writeValidated(arena, entriesPtr[i].value, r, depth + 1);
          }
          target.data.mapVal
            ..ptr = entriesPtr
            ..len = count;
        });
    }
  }

  static void _encodeNativeValue(BinWriter writer, NveValue v, int depth) {
    final tag = NveValueTag.fromCode(v.tag);
    if (tag == null) {
      throw BinCodecException(BinErrorKind.invalidTag, 'tag ${v.tag}');
    }
    switch (tag) {
      case NveValueTag.nullValue:
        writer.writeNull();
      case NveValueTag.boolValue:
        if (v.data.boolVal > 1) {
          throw BinCodecException(BinErrorKind.invalidBool, 'byte ${v.data.boolVal}');
        }
        writer.writeBool(v.data.boolVal == 1);
      case NveValueTag.i64:
        writer.writeI64(v.data.i64Val);
      case NveValueTag.f64:
        writer.writeF64(v.data.f64Val);
      case NveValueTag.str:
        final s = v.data.strVal;
        final utf8Bytes = _sliceBytes(s.ptr, s.len);
        if (!isValidUtf8(utf8Bytes)) {
          throw const BinCodecException(BinErrorKind.invalidUtf8, 'invalid UTF-8 in NveStrSlice');
        }
        writer
          ..u8(BinTag.str.index)
          ..u32(utf8Bytes.length)
          ..raw(utf8Bytes);
      case NveValueTag.bytes:
        final b = v.data.bytesVal;
        writer.writeBytes(_sliceBytes(b.ptr, b.len));
      case NveValueTag.array:
        if (depth >= binValueMaxDepth) {
          throw BinCodecException(BinErrorKind.maxDepth, 'depth $depth');
        }
        final arr = v.data.arrayVal;
        if (arr.len > 0 && arr.ptr == ffi.nullptr) {
          throw const BinCodecException(BinErrorKind.truncated, 'null NveValueArray pointer');
        }
        final mark = writer.beginArray(arr.len);
        for (var i = 0; i < arr.len; i++) {
          _encodeNativeValue(writer, arr.ptr[i], depth + 1);
        }
        writer.end(mark);
      case NveValueTag.map:
        if (depth >= binValueMaxDepth) {
          throw BinCodecException(BinErrorKind.maxDepth, 'depth $depth');
        }
        final map = v.data.mapVal;
        if (map.len > 0 && map.ptr == ffi.nullptr) {
          throw const BinCodecException(BinErrorKind.truncated, 'null NveValueMap pointer');
        }
        final mark = writer.beginMap(map.len);
        for (var i = 0; i < map.len; i++) {
          final entry = map.ptr[i];
          final keyUtf8 = _sliceBytes(entry.key.ptr, entry.key.len);
          if (!isValidUtf8(keyUtf8)) {
            throw const BinCodecException(BinErrorKind.invalidUtf8, 'invalid UTF-8 in NveMapEntry key');
          }
          writer
            ..u32(keyUtf8.length)
            ..raw(keyUtf8);
          _encodeNativeValue(writer, entry.value, depth + 1);
        }
        writer.end(mark);
    }
  }

  static Uint8List _sliceBytes(ffi.Pointer<ffi.Uint8> ptr, int len) {
    if (len == 0) {
      return Uint8List(0);
    }
    if (ptr == ffi.nullptr) {
      throw const BinCodecException(BinErrorKind.truncated, 'null slice pointer with non-zero len');
    }
    return ptr.asTypedList(len);
  }
}

/// Strongly typed Dart hybrid view props implement [HybridPropsEncodable] to write directly into
/// a [HybridArena] for `set_props_direct` with zero `Map` boxing and zero double-wrapping.
abstract interface class HybridPropsEncodable {
  int get directPropCount;
  void encodeDirect(HybridPropsBuilder builder);
}

/// Arena-backed builder populating contiguous `NveMapEntry` slots for `set_props_direct`.
final class HybridPropsBuilder {
  HybridPropsBuilder._(this.arena, this._entries, this._capacity);

  final HybridArena arena;
  final ffi.Pointer<NveMapEntry> _entries;
  final int _capacity;
  int _count = 0;

  int get count => _count;
  ffi.Pointer<NveMapEntry> get entriesPtr => _entries;

  NveMapEntry _next(String key) {
    if (_count >= _capacity) {
      throw StateError('HybridPropsBuilder exceeded declared directPropCount ($_capacity)');
    }
    final entry = _entries[_count++];
    arena.writeStrSlice(entry.key, key);
    return entry;
  }

  void writeNull(String key) {
    final entry = _next(key);
    entry.value.tag = NveValueTag.nullValue.code;
    entry.value.data.i64Val = 0;
  }

  void writeBool(String key, bool? value) {
    if (value == null) {
      writeNull(key);
      return;
    }
    final entry = _next(key);
    entry.value.tag = NveValueTag.boolValue.code;
    entry.value.data.boolVal = value ? 1 : 0;
  }

  void writeInt64(String key, int? value) {
    if (value == null) {
      writeNull(key);
      return;
    }
    final entry = _next(key);
    entry.value.tag = NveValueTag.i64.code;
    entry.value.data.i64Val = value;
  }

  void writeDouble(String key, double? value) {
    if (value == null) {
      writeNull(key);
      return;
    }
    final entry = _next(key);
    entry.value.tag = NveValueTag.f64.code;
    entry.value.data.f64Val = value;
  }

  void writeString(String key, String? value) {
    if (value == null) {
      writeNull(key);
      return;
    }
    final entry = _next(key);
    entry.value.tag = NveValueTag.str.code;
    arena.writeStrSlice(entry.value.data.strVal, value);
  }

  void writeBytes(String key, List<int>? value) {
    if (value == null) {
      writeNull(key);
      return;
    }
    final entry = _next(key);
    entry.value.tag = NveValueTag.bytes.code;
    arena.writeBytesSlice(entry.value.data.bytesVal, value);
  }

  /// Materializes a [DynamicValue] losslessly into `entry.value` with its true `NveValue` tags
  /// (never double-wrapped in `NVE_VAL_BYTES`).
  void writeDynamicValue(String key, DynamicValue? value) {
    if (value == null) {
      writeNull(key);
      return;
    }
    final entry = _next(key);
    NveValueTree.writeDynamic(arena, entry.value, value);
  }
}

/// Scoped runner allocating `NveMapEntry[]` and nested `NveValue` trees in a single [HybridArena].
abstract final class HybridPropsWriter {
  static T withEncodedProps<T>(
    HybridPropsEncodable props,
    T Function(ffi.Pointer<NveMapEntry> entries, int count) body,
  ) {
    final capacity = props.directPropCount;
    if (capacity <= 0) {
      return body(ffi.nullptr, 0);
    }
    return HybridArena.using((arena) {
      final entriesPtr = arena.allocStructArray<NveMapEntry>(ffi.sizeOf<NveMapEntry>(), capacity);
      final builder = HybridPropsBuilder._(arena, entriesPtr, capacity);
      props.encodeDirect(builder);
      return body(builder.entriesPtr, builder.count);
    });
  }
}
