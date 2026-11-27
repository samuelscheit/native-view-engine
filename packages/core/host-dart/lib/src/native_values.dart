/// Native `NveValue` / `NveMapEntry` marshalling for synchronous `dart:ffi` calls (`set_props`,
/// generated hybrid clients): a bump arena plus the typed direct-props builder.
library;

import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'bin_codec.dart';
import 'hybrid_runtime.dart';
import 'nve_core_abi.generated.dart';
import 'runtime_api.dart' show NativeHeap;
import 'utf8.dart';

/// Owned heap byte buffer returned across hybrid C-ABI vtables and released via `free_bytes`.
final class NveHybridOwnedBytes extends ffi.Struct {
  external ffi.Pointer<ffi.Uint8> ptr;
  @ffi.Size()
  external int len;
  @ffi.Size()
  external int cap;
}

/// Chunked bump arena for the borrowed arguments of one synchronous FFI call. Pointers never
/// move until [dispose]; one zeroed 4 KiB chunk covers typical calls.
final class NativeArena {
  NativeArena({int chunkBytes = 4096}) : _chunkBytes = chunkBytes < 256 ? 256 : chunkBytes;

  final int _chunkBytes;
  final List<ffi.Pointer<ffi.Uint8>> _chunks = <ffi.Pointer<ffi.Uint8>>[];
  int _base = 0;
  int _capacity = 0;
  int _offset = 0;

  /// [byteCount] zeroed bytes aligned to [align] (a power of two).
  ffi.Pointer<T> alloc<T extends ffi.NativeType>(int byteCount, {int align = 8}) {
    final size = byteCount < 1 ? 1 : byteCount;
    final at = (_offset + align - 1) & ~(align - 1);
    if (_chunks.isNotEmpty && at + size <= _capacity) {
      _offset = at + size;
      return ffi.Pointer<T>.fromAddress(_base + at);
    }
    final cap = size > _chunkBytes ? size : _chunkBytes;
    final chunk = NativeHeap.zeroed<ffi.Uint8>(cap);
    _chunks.add(chunk);
    _base = chunk.address;
    _capacity = cap;
    _offset = size;
    return chunk.cast<T>();
  }

  ffi.Pointer<T> allocArray<T extends ffi.Struct>(int elementSize, int count) =>
      count <= 0 ? ffi.nullptr : alloc<T>(elementSize * count);

  /// UTF-8 of [value] into [target]; throws [BinCodecException] (`invalidUtf8`) on a lone
  /// surrogate, which a Rust `&str` can never hold.
  void writeStr(NveStrSlice target, String value) {
    final n = utf8EncodedLength(value);
    final buf = n == 0 ? ffi.nullptr.cast<ffi.Uint8>() : alloc<ffi.Uint8>(n, align: 1);
    if (n > 0) {
      final out = buf.asTypedList(n);
      for (var i = 0, o = 0; i < value.length; i += utf16UnitsAt(value, i)) {
        o = encodeUtf8CodePointAt(value, i, out, o);
      }
    }
    target
      ..ptr = buf
      ..len = n;
  }

  /// Arena copy of [bytes] (`nullptr` when empty).
  ffi.Pointer<ffi.Uint8> copyBytes(List<int> bytes) {
    if (bytes.isEmpty) {
      return ffi.nullptr;
    }
    final buf = alloc<ffi.Uint8>(bytes.length, align: 1);
    buf.asTypedList(bytes.length).setAll(0, bytes);
    return buf;
  }

  /// Writes `null` / `bool` / `int` / `double` / `String` / `Uint8List` / [DynamicValue] /
  /// `List` / `Map<String, _>` as a native `NveValue` tree.
  void writeValue(NveValue target, Object? value, [int depth = 0]) {
    switch (value) {
      case null:
        target.tag = NveValueTag.nullValue.code;
      case final bool v:
        target.tag = NveValueTag.boolValue.code;
        target.data.boolVal = v ? 1 : 0;
      case final int v:
        target.tag = NveValueTag.i64.code;
        target.data.i64Val = v;
      case final double v:
        target.tag = NveValueTag.f64.code;
        target.data.f64Val = v;
      case final String v:
        target.tag = NveValueTag.str.code;
        writeStr(target.data.strVal, v);
      case final Uint8List v:
        target.tag = NveValueTag.bytes.code;
        target.data.bytesVal
          ..ptr = copyBytes(v)
          ..len = v.length;
      case final DynamicValue v:
        writeBinValue(target, v.reader, depth);
      case final List<Object?> v:
        checkBinValueDepth(depth);
        target.tag = NveValueTag.array.code;
        final items = allocArray<NveValue>(ffi.sizeOf<NveValue>(), v.length);
        for (var i = 0; i < v.length; i++) {
          writeValue(items[i], v[i], depth + 1);
        }
        target.data.arrayVal
          ..ptr = items
          ..len = v.length;
      case final Map<Object?, Object?> v:
        checkBinValueDepth(depth);
        target.tag = NveValueTag.map.code;
        target.data.mapVal
          ..ptr = writeEntries(v, depth + 1)
          ..len = v.length;
      default:
        throw ArgumentError.value(value, 'value', 'not representable as NveValue');
    }
  }

  /// Native `NveMapEntry[map.length]` of [map] (`String` keys).
  ffi.Pointer<NveMapEntry> writeEntries(Map<Object?, Object?> map, [int depth = 0]) {
    final entries = allocArray<NveMapEntry>(ffi.sizeOf<NveMapEntry>(), map.length);
    var i = 0;
    for (final MapEntry(:key, :value) in map.entries) {
      if (key is! String) {
        throw ArgumentError.value(key, 'key', 'NveMapEntry keys must be strings');
      }
      writeStr(entries[i].key, key);
      writeValue(entries[i].value, value, depth);
      i++;
    }
    return entries;
  }

  /// Converts the next value of [r] into [target] without an intermediate Dart object tree.
  void writeBinValue(NveValue target, BinReader r, [int depth = 0]) {
    switch (r.peekTag()) {
      case BinTag.nullValue:
        r.readNull();
        target.tag = NveValueTag.nullValue.code;
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
        target.tag = NveValueTag.str.code;
        writeStr(target.data.strVal, r.readStr());
      case BinTag.bytes:
        final b = r.readBytes();
        target.tag = NveValueTag.bytes.code;
        target.data.bytesVal
          ..ptr = copyBytes(b)
          ..len = b.length;
      case BinTag.array:
        checkBinValueDepth(depth);
        final n = r.beginArray();
        final items = allocArray<NveValue>(ffi.sizeOf<NveValue>(), n);
        for (var i = 0; i < n; i++) {
          writeBinValue(items[i], r, depth + 1);
        }
        target.tag = NveValueTag.array.code;
        target.data.arrayVal
          ..ptr = items
          ..len = n;
      case BinTag.map:
        checkBinValueDepth(depth);
        final n = r.beginMap();
        final entries = allocArray<NveMapEntry>(ffi.sizeOf<NveMapEntry>(), n);
        for (var i = 0; i < n; i++) {
          writeStr(entries[i].key, r.readMapKey());
          writeBinValue(entries[i].value, r, depth + 1);
        }
        target.tag = NveValueTag.map.code;
        target.data.mapVal
          ..ptr = entries
          ..len = n;
    }
  }

  /// Frees every chunk; earlier pointers dangle afterwards.
  void dispose() {
    for (final c in _chunks) {
      NativeHeap.free(c);
    }
    _chunks.clear();
    _base = 0;
    _capacity = 0;
    _offset = 0;
  }
}

/// Typed prop records (generated) write straight into native `NveMapEntry` slots.
abstract interface class DirectEncodable {
  /// Upper bound of entries [encodeDirect] writes.
  int get directPropCount;
  void encodeDirect(DirectPropsBuilder builder);
}

/// Fills a fixed-capacity `NveMapEntry` array; `null` values write `NveValue` null.
final class DirectPropsBuilder {
  DirectPropsBuilder._(this.arena, this.entries, this._capacity);

  final NativeArena arena;
  final ffi.Pointer<NveMapEntry> entries;
  final int _capacity;
  int _count = 0;

  int get count => _count;

  NveValue _slot(String key) {
    if (_count >= _capacity) {
      throw StateError('DirectPropsBuilder exceeded directPropCount ($_capacity)');
    }
    final entry = entries[_count++];
    arena.writeStr(entry.key, key);
    return entry.value;
  }

  void writeNull(String key) => _slot(key).tag = NveValueTag.nullValue.code;

  void writeBool(String key, bool? value) => arena.writeValue(_slot(key), value);
  void writeI64(String key, int? value) => arena.writeValue(_slot(key), value);
  void writeF64(String key, double? value) => arena.writeValue(_slot(key), value);
  void writeStr(String key, String? value) => arena.writeValue(_slot(key), value);
  void writeBytes(String key, Uint8List? value) => arena.writeValue(_slot(key), value);
}

/// Scoped native `NveMapEntry[]` for one synchronous `set_props`-style call.
abstract final class DirectProps {
  static T withEncoded<T>(DirectEncodable props, T Function(ffi.Pointer<NveMapEntry> entries, int count) body) {
    final capacity = props.directPropCount;
    if (capacity <= 0) {
      return body(ffi.nullptr, 0);
    }
    final arena = NativeArena();
    try {
      final builder = DirectPropsBuilder._(
          arena, arena.allocArray<NveMapEntry>(ffi.sizeOf<NveMapEntry>(), capacity), capacity);
      props.encodeDirect(builder);
      return body(builder.entries, builder.count);
    } finally {
      arena.dispose();
    }
  }

  static T withMap<T>(Map<String, Object?> props, T Function(ffi.Pointer<NveMapEntry> entries, int count) body) {
    if (props.isEmpty) {
      return body(ffi.nullptr, 0);
    }
    final arena = NativeArena();
    try {
      return body(arena.writeEntries(props), props.length);
    } finally {
      arena.dispose();
    }
  }
}
