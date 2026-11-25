import 'dart:convert' show utf8;
import 'dart:ffi' as ffi;
import 'dart:io' show Platform;
import 'dart:typed_data';

import 'event_codec.dart';
import 'nve_bin_value.dart';
import 'nve_core_abi_gen.dart';

/// Self-contained C heap allocator so `nve_flutter` requires zero external pub packages.
class NveNativeMem {
  const NveNativeMem._();

  static final ffi.DynamicLibrary _libc = Platform.isWindows
      ? ffi.DynamicLibrary.open('ucrtbase.dll')
      : ffi.DynamicLibrary.process();

  static final ffi.Pointer<ffi.Void> Function(int, int) _calloc = _libc.lookupFunction<
      ffi.Pointer<ffi.Void> Function(ffi.Size, ffi.Size),
      ffi.Pointer<ffi.Void> Function(int, int)>('calloc');
  static final void Function(ffi.Pointer<ffi.Void>) _free = _libc.lookupFunction<
      ffi.Void Function(ffi.Pointer<ffi.Void>),
      void Function(ffi.Pointer<ffi.Void>)>('free');

  static ffi.Pointer<T> allocate<T extends ffi.NativeType>(int byteCount) {
    final ptr = _calloc(1, byteCount <= 0 ? 1 : byteCount);
    if (ptr == ffi.nullptr) {
      throw StateError('Failed to allocate $byteCount native bytes');
    }
    return ptr.cast<T>();
  }

  static void free(ffi.Pointer<ffi.NativeType> ptr) {
    if (ptr != ffi.nullptr) {
      _free(ptr.cast<ffi.Void>());
    }
  }

  static ffi.Pointer<ffi.Uint8> copyBytes(Uint8List bytes) {
    final ptr = allocate<ffi.Uint8>(bytes.length);
    if (bytes.isNotEmpty) {
      ptr.asTypedList(bytes.length).setAll(0, bytes);
    }
    return ptr;
  }

  static NveBufferSlice sliceFor(
    ffi.Pointer<ffi.Uint8> ptr,
    int len,
    ffi.Pointer<NveBufferSlice> slot,
  ) {
    slot.ref.ptr = ptr;
    slot.ref.len = len;
    return slot.ref;
  }
}

/// Zero-copy chunked bump arena backing synchronous `dart:ffi` calls (`set_props_direct` and
/// `dispatch_events_direct`).
///
/// Allocates one zero-initialized 4 KiB slab for typical calls and chains larger chunks only
/// when needed; existing pointers never move until [dispose].
class NveNativeArena {
  NveNativeArena({int initialChunkBytes = 4096})
      : _chunkSize = initialChunkBytes < 256 ? 256 : initialChunkBytes;

  final int _chunkSize;
  final List<ffi.Pointer<ffi.Uint8>> _chunks = <ffi.Pointer<ffi.Uint8>>[];
  ffi.Pointer<ffi.Uint8> _current = ffi.nullptr;
  int _capacity = 0;
  int _offset = 0;

  ffi.Pointer<ffi.Uint8> allocBytes(int byteCount, {int align = 8}) {
    final size = byteCount <= 0 ? 1 : byteCount;
    final aligned = (_offset + (align - 1)) & ~(align - 1);
    if (_current != ffi.nullptr && aligned + size <= _capacity) {
      final ptr = ffi.Pointer<ffi.Uint8>.fromAddress(_current.address + aligned);
      _offset = aligned + size;
      return ptr;
    }
    final nextCap = size > _chunkSize ? size : (_capacity > 0 ? _capacity * 2 : _chunkSize);
    final chunk = NveNativeMem.allocate<ffi.Uint8>(nextCap);
    _chunks.add(chunk);
    _current = chunk;
    _capacity = nextCap;
    _offset = size;
    return chunk;
  }

  ffi.Pointer<T> allocStructArray<T extends ffi.Struct>(int elementSize, int count) {
    if (count <= 0) return ffi.nullptr.cast<T>();
    return allocBytes(elementSize * count, align: 8).cast<T>();
  }

  void writeStrSlice(NveStrSlice target, String value) {
    if (value.isEmpty) {
      target
        ..ptr = ffi.nullptr
        ..len = 0;
      return;
    }
    final encoded = utf8.encode(value);
    final buf = allocBytes(encoded.length, align: 1);
    buf.asTypedList(encoded.length).setAll(0, encoded);
    target
      ..ptr = buf
      ..len = encoded.length;
  }

  void writeBytesSlice(NveBufferSlice target, Uint8List bytes) {
    if (bytes.isEmpty) {
      target
        ..ptr = ffi.nullptr
        ..len = 0;
      return;
    }
    final buf = allocBytes(bytes.length, align: 1);
    buf.asTypedList(bytes.length).setAll(0, bytes);
    target
      ..ptr = buf
      ..len = bytes.length;
  }

  void writeUtf8Slice(NveStrSlice target, Uint8List utf8Bytes) {
    if (utf8Bytes.isEmpty) {
      target
        ..ptr = ffi.nullptr
        ..len = 0;
      return;
    }
    final buf = allocBytes(utf8Bytes.length, align: 1);
    buf.asTypedList(utf8Bytes.length).setAll(0, utf8Bytes);
    target
      ..ptr = buf
      ..len = utf8Bytes.length;
  }

  void writeValue(NveValue target, Object? value, {int depth = 0}) {
    if (depth > nveBinValueMaxDepth) {
      throw const NveBinValueException(
        NveBinValueErrorKind.maxDepthExceeded,
        'Exceeded maximum nesting depth of $nveBinValueMaxDepth',
      );
    }
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
      writeStrSlice(target.data.strVal, value);
      return;
    }
    if (value is Uint8List) {
      target.tag = NveValueTag.bytes.code;
      writeBytesSlice(target.data.bytesVal, value);
      return;
    }
    if (value is NveBinValueView) {
      writeBinValueView(target, value, depth: depth);
      return;
    }
    if (value is List) {
      if (depth >= nveBinValueMaxDepth) {
        throw const NveBinValueException(
          NveBinValueErrorKind.maxDepthExceeded,
          'Exceeded maximum nesting depth of $nveBinValueMaxDepth',
        );
      }
      target.tag = NveValueTag.array.code;
      if (value.isEmpty) {
        target.data.arrayVal
          ..ptr = ffi.nullptr
          ..len = 0;
        return;
      }
      final arrPtr = allocStructArray<NveValue>(ffi.sizeOf<NveValue>(), value.length);
      for (var i = 0; i < value.length; i++) {
        writeValue(arrPtr[i], value[i], depth: depth + 1);
      }
      target.data.arrayVal
        ..ptr = arrPtr
        ..len = value.length;
      return;
    }
    if (value is Map) {
      if (depth >= nveBinValueMaxDepth) {
        throw const NveBinValueException(
          NveBinValueErrorKind.maxDepthExceeded,
          'Exceeded maximum nesting depth of $nveBinValueMaxDepth',
        );
      }
      target.tag = NveValueTag.map.code;
      if (value.isEmpty) {
        target.data.mapVal
          ..ptr = ffi.nullptr
          ..len = 0;
        return;
      }
      final entriesPtr = writeMapEntries(value, depth: depth + 1);
      target.data.mapVal
        ..ptr = entriesPtr
        ..len = value.length;
      return;
    }
    throw ArgumentError('Unsupported NveValue type: ${value.runtimeType}');
  }

  void writeBinValueView(NveValue target, NveBinValueView view, {int depth = 0}) {
    if (depth > nveBinValueMaxDepth) {
      throw const NveBinValueException(
        NveBinValueErrorKind.maxDepthExceeded,
        'Exceeded maximum nesting depth of $nveBinValueMaxDepth',
      );
    }
    switch (view.tag) {
      case NveBinValueTag.nullValue:
        target.tag = NveValueTag.nullValue.code;
        target.data.i64Val = 0;
      case NveBinValueTag.boolValue:
        target.tag = NveValueTag.boolValue.code;
        target.data.boolVal = (view.asBool() ?? false) ? 1 : 0;
      case NveBinValueTag.i64:
        target.tag = NveValueTag.i64.code;
        target.data.i64Val = view.asInt64() ?? 0;
      case NveBinValueTag.f64:
        target.tag = NveValueTag.f64.code;
        target.data.f64Val = view.asFloat64() ?? 0.0;
      case NveBinValueTag.str:
        target.tag = NveValueTag.str.code;
        writeUtf8Slice(target.data.strVal, view.asUtf8Bytes() ?? Uint8List(0));
      case NveBinValueTag.bytes:
        target.tag = NveValueTag.bytes.code;
        writeBytesSlice(target.data.bytesVal, view.asBytes() ?? Uint8List(0));
      case NveBinValueTag.array:
        if (depth >= nveBinValueMaxDepth) {
          throw const NveBinValueException(
            NveBinValueErrorKind.maxDepthExceeded,
            'Exceeded maximum nesting depth of $nveBinValueMaxDepth',
          );
        }
        target.tag = NveValueTag.array.code;
        final count = view.arrayLength ?? 0;
        if (count == 0) {
          target.data.arrayVal
            ..ptr = ffi.nullptr
            ..len = 0;
          return;
        }
        final arrPtr = allocStructArray<NveValue>(ffi.sizeOf<NveValue>(), count);
        for (var i = 0; i < count; i++) {
          final elem = view.arrayAt(i);
          if (elem != null) {
            writeBinValueView(arrPtr[i], elem, depth: depth + 1);
          } else {
            arrPtr[i].tag = NveValueTag.nullValue.code;
          }
        }
        target.data.arrayVal
          ..ptr = arrPtr
          ..len = count;
      case NveBinValueTag.map:
        if (depth >= nveBinValueMaxDepth) {
          throw const NveBinValueException(
            NveBinValueErrorKind.maxDepthExceeded,
            'Exceeded maximum nesting depth of $nveBinValueMaxDepth',
          );
        }
        target.tag = NveValueTag.map.code;
        final count = view.mapLength ?? 0;
        if (count == 0) {
          target.data.mapVal
            ..ptr = ffi.nullptr
            ..len = 0;
          return;
        }
        final entriesPtr = allocStructArray<NveMapEntry>(ffi.sizeOf<NveMapEntry>(), count);
        var idx = 0;
        view.forEachMapEntry((String k, NveBinValueView v) {
          writeStrSlice(entriesPtr[idx].key, k);
          writeBinValueView(entriesPtr[idx].value, v, depth: depth + 1);
          idx += 1;
        });
        target.data.mapVal
          ..ptr = entriesPtr
          ..len = idx;
    }
  }

  ffi.Pointer<NveMapEntry> writeMapEntries(Map<Object?, Object?> map, {int depth = 0}) {
    if (map.isEmpty) return ffi.nullptr;
    final entriesPtr = allocStructArray<NveMapEntry>(ffi.sizeOf<NveMapEntry>(), map.length);
    var idx = 0;
    for (final entry in map.entries) {
      final key = entry.key;
      if (key is! String) {
        throw ArgumentError('NveMapEntry key must be a String, got ${key.runtimeType}');
      }
      writeStrSlice(entriesPtr[idx].key, key);
      writeValue(entriesPtr[idx].value, entry.value, depth: depth);
      idx += 1;
    }
    return entriesPtr;
  }

  void dispose() {
    for (final chunk in _chunks) {
      NveNativeMem.free(chunk);
    }
    _chunks.clear();
    _current = ffi.nullptr;
    _capacity = 0;
    _offset = 0;
  }
}

/// Zero-JSON direct writer populating `NveMapEntry[]` + `NveValue` structs for
/// `NveComponentApi.set_props_direct`.
class NveDirectPropsWriter {
  const NveDirectPropsWriter._();

  static T withProps<T>(
    Map<String, Object?> props,
    T Function(ffi.Pointer<NveMapEntry> entries, int count) body,
  ) {
    if (props.isEmpty) {
      return body(ffi.nullptr, 0);
    }
    final arena = NveNativeArena();
    try {
      final entriesPtr = arena.writeMapEntries(props, depth: 1);
      return body(entriesPtr, props.length);
    } finally {
      arena.dispose();
    }
  }
}

/// Zero-JSON direct writer populating `NveDirectEvent[]` + `NveEventBatch` structs for
/// `NveSurfaceApi.dispatch_events_direct`.
class NveDirectEventWriter {
  const NveDirectEventWriter._();

  static T withBatch<T>(
    List<NveEvent> events,
    T Function(NveEventBatch batch) body,
  ) {
    final arena = NveNativeArena();
    try {
      final batchPtr = arena.allocStructArray<NveEventBatch>(ffi.sizeOf<NveEventBatch>(), 1);
      if (events.isEmpty) {
        batchPtr.ref
          ..ptr = ffi.nullptr
          ..len = 0;
        return body(batchPtr.ref);
      }
      final eventsPtr = arena.allocStructArray<NveDirectEvent>(
        ffi.sizeOf<NveDirectEvent>(),
        events.length,
      );
      for (var i = 0; i < events.length; i++) {
        _encodeEvent(arena, eventsPtr[i], events[i]);
      }
      batchPtr.ref
        ..ptr = eventsPtr
        ..len = events.length;
      return body(batchPtr.ref);
    } finally {
      arena.dispose();
    }
  }

  static void _encodeEvent(NveNativeArena arena, NveDirectEvent dst, NveEvent event) {
    dst
      ..surfaceId = event.surfaceId
      ..nodeId = event.nodeId
      ..timestampUs = event.timestampUs
      ..kind = event.kind.code;

    switch (event.payload) {
      case NveEmptyPayload():
        dst.payloadTag = NveEventPayloadTag.empty.code;
        dst.value.tag = NveValueTag.nullValue.code;
      case NvePointPayload(:final x, :final y):
        dst
          ..payloadTag = NveEventPayloadTag.point.code
          ..x = x
          ..y = y;
        dst.value.tag = NveValueTag.nullValue.code;
      case NveScrollPayload(:final x, :final y, :final contentWidth, :final contentHeight):
        dst
          ..payloadTag = NveEventPayloadTag.scroll.code
          ..x = x
          ..y = y
          ..width = contentWidth
          ..height = contentHeight;
        dst.value.tag = NveValueTag.nullValue.code;
      case NveTextPayload(:final text):
        dst.payloadTag = NveEventPayloadTag.text.code;
        arena.writeStrSlice(dst.text, text);
        dst.value.tag = NveValueTag.nullValue.code;
      case NveLayoutPayload(:final x, :final y, :final width, :final height):
        dst
          ..payloadTag = NveEventPayloadTag.layout.code
          ..x = x
          ..y = y
          ..width = width
          ..height = height;
        dst.value.tag = NveValueTag.nullValue.code;
      case NveBinValuePayload(:final bytes):
        dst
          ..kind = NveEventKind.custom.code
          ..payloadTag = NveEventPayloadTag.custom.code;
        arena.writeStrSlice(dst.name, event.name ?? '');
        if (bytes.isEmpty) {
          dst.value.tag = NveValueTag.nullValue.code;
        } else {
          try {
            final view = NveBinValueView.parse(bytes);
            arena.writeBinValueView(dst.value, view);
          } on NveBinValueException {
            dst.value.tag = NveValueTag.bytes.code;
            arena.writeBytesSlice(dst.value.data.bytesVal, bytes);
          }
        }
    }
  }
}
