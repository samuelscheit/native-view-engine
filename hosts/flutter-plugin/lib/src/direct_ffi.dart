import 'dart:convert' show utf8;
import 'dart:ffi' as ffi;
import 'dart:io' show Platform;
import 'dart:typed_data';

import 'event_codec.dart';
import 'nve_bin_value.dart';
import 'nve_core_abi.generated.dart' hide NveEvent;
import 'nve_core_abi.generated.dart' as abi show NveEvent;

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

/// Zero-copy chunked bump arena backing synchronous `dart:ffi` calls (`set_props` and
/// `dispatch_events`).
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

/// Strongly typed Dart prop records implement [NveDirectEncodable] to write directly into
/// [NveNativeArena] for `set_props` with zero per-field FFI allocations or `Map` boxing.
abstract interface class NveDirectEncodable {
  int get directPropCount;
  void encodeDirect(NveDirectPropsBuilder builder);
}

/// Arena-backed builder populating contiguous `NveMapEntry` slots for `set_props`.
class NveDirectPropsBuilder {
  NveDirectPropsBuilder._(this.arena, this._entries, this._capacity);

  final NveNativeArena arena;
  final ffi.Pointer<NveMapEntry> _entries;
  final int _capacity;
  int _count = 0;

  int get count => _count;
  ffi.Pointer<NveMapEntry> get entriesPtr => _entries;

  NveMapEntry _next(String key) {
    if (_count >= _capacity) {
      throw StateError('NveDirectPropsBuilder exceeded declared directPropCount ($_capacity)');
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

  void writeBytes(String key, Uint8List? value) {
    if (value == null) {
      writeNull(key);
      return;
    }
    final entry = _next(key);
    entry.value.tag = NveValueTag.bytes.code;
    arena.writeBytesSlice(entry.value.data.bytesVal, value);
  }
}

/// Binary direct writer populating `NveMapEntry[]` + `NveValue` structs for
/// `NveComponentApi.set_props`.
class NveDirectPropsWriter {
  const NveDirectPropsWriter._();

  static T withEncodedProps<T>(
    NveDirectEncodable props,
    T Function(ffi.Pointer<NveMapEntry> entries, int count) body,
  ) {
    final capacity = props.directPropCount;
    if (capacity <= 0) {
      return body(ffi.nullptr, 0);
    }
    final arena = NveNativeArena();
    try {
      final entriesPtr = arena.allocStructArray<NveMapEntry>(ffi.sizeOf<NveMapEntry>(), capacity);
      final builder = NveDirectPropsBuilder._(arena, entriesPtr, capacity);
      props.encodeDirect(builder);
      return body(builder.entriesPtr, builder.count);
    } finally {
      arena.dispose();
    }
  }

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

/// Binary direct writer populating uniform 64-byte `NveEvent[]` + byte-arena `NveEventBatch`
/// structs for `NveSurfaceApi.dispatch_events` and `drain_emitted_events`.
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
          ..events = ffi.nullptr
          ..bytes = ffi.nullptr
          ..eventCount = 0
          ..bytesLen = 0;
        return body(batchPtr.ref);
      }
      final eventsPtr = arena.allocStructArray<abi.NveEvent>(
        ffi.sizeOf<abi.NveEvent>(),
        events.length,
      );
      final byteBuilder = BytesBuilder(copy: false);
      for (var i = 0; i < events.length; i++) {
        _encodeEvent(byteBuilder, eventsPtr[i], events[i]);
      }
      final arenaBytes = byteBuilder.takeBytes();
      final ffi.Pointer<ffi.Uint8> bytesPtr;
      if (arenaBytes.isEmpty) {
        bytesPtr = ffi.nullptr;
      } else {
        bytesPtr = arena.allocBytes(arenaBytes.length, align: 1);
        bytesPtr.asTypedList(arenaBytes.length).setAll(0, arenaBytes);
      }
      batchPtr.ref
        ..events = eventsPtr
        ..bytes = bytesPtr
        ..eventCount = events.length
        ..bytesLen = arenaBytes.length;
      return body(batchPtr.ref);
    } finally {
      arena.dispose();
    }
  }

  static int _f32Bits(double v) {
    final bd = ByteData(4)..setFloat32(0, v, Endian.little);
    return bd.getUint32(0, Endian.little);
  }

  static double _bitsF32(int bits) {
    final bd = ByteData(4)..setUint32(0, bits, Endian.little);
    return bd.getFloat32(0, Endian.little);
  }

  static (int, int) _tableAndEventId(NveEventKind kind) => switch (kind) {
    NveEventKind.press => (nveTableInteraction, NveInteractionEvent.press.code),
    NveEventKind.longPress => (nveTableInteraction, NveInteractionEvent.longPress.code),
    NveEventKind.focus => (nveTableInteraction, NveInteractionEvent.focus.code),
    NveEventKind.blur => (nveTableInteraction, NveInteractionEvent.blur.code),
    NveEventKind.keyDown => (nveTableInteraction, NveInteractionEvent.keyDown.code),
    NveEventKind.keyUp => (nveTableInteraction, NveInteractionEvent.keyUp.code),
    NveEventKind.pointerDown => (nveTableInteraction, NveInteractionEvent.pointerDown.code),
    NveEventKind.pointerMove => (nveTableInteraction, NveInteractionEvent.pointerMove.code),
    NveEventKind.pointerUp => (nveTableInteraction, NveInteractionEvent.pointerUp.code),
    NveEventKind.pointerEnter => (nveTableInteraction, NveInteractionEvent.pointerEnter.code),
    NveEventKind.pointerLeave => (nveTableInteraction, NveInteractionEvent.pointerLeave.code),
    NveEventKind.wheel => (nveTableInteraction, NveInteractionEvent.wheel.code),
    NveEventKind.textChange => (nveTableTextInput, NveTextInputEvent.change.code),
    NveEventKind.submit => (nveTableTextInput, NveTextInputEvent.submit.code),
    NveEventKind.compositionStart => (nveTableTextInput, NveTextInputEvent.compositionStart.code),
    NveEventKind.compositionUpdate => (nveTableTextInput, NveTextInputEvent.compositionUpdate.code),
    NveEventKind.compositionEnd => (nveTableTextInput, NveTextInputEvent.compositionEnd.code),
    NveEventKind.scroll => (nveTableScroll, NveScrollEvent.scroll.code),
    NveEventKind.layout => (nveTableLayout, NveLayoutEvent.layout.code),
    NveEventKind.custom || NveEventKind.tableEvent => (nveTableCustom, 0),
  };

  static void _encodeEvent(BytesBuilder arena, abi.NveEvent dst, NveEvent event) {
    var (table, eventId) = _tableAndEventId(event.kind);
    var flags = 0;
    final p = <int>[0, 0, 0, 0, 0, 0];
    var bytesOff = 0;
    var bytesLen = 0;

    void appendData(List<int> data) {
      if (data.isNotEmpty) {
        bytesOff = arena.length;
        bytesLen = data.length;
        arena.add(data);
      }
    }

    switch (event.payload) {
      case NveEmptyPayload():
        break;
      case NvePointPayload(:final x, :final y):
        p[0] = _f32Bits(x);
        p[1] = _f32Bits(y);
      case NveScrollPayload(:final x, :final y, :final contentWidth, :final contentHeight):
        p[0] = _f32Bits(x);
        p[1] = _f32Bits(y);
        p[2] = _f32Bits(contentWidth);
        p[3] = _f32Bits(contentHeight);
      case NveTextPayload(:final text):
        flags |= nveEventFlagUtf8Bytes;
        appendData(utf8.encode(text));
      case NveLayoutPayload(:final x, :final y, :final width, :final height):
        p[0] = _f32Bits(x);
        p[1] = _f32Bits(y);
        p[2] = _f32Bits(width);
        p[3] = _f32Bits(height);
      case NveBinValuePayload(:final bytes):
        table = nveTableCustom;
        eventId = 0;
        flags |= nveEventFlagCustomNamed;
        final nameBytes = utf8.encode(event.name ?? '');
        p[0] = nameBytes.length;
        final combined = BytesBuilder(copy: false)
          ..add(nameBytes)
          ..add(bytes);
        appendData(combined.takeBytes());
      case NveKeyPayload(:final key, :final code, :final modifiers, :final keyCode, :final isComposing):
        flags |= modifiers & 0x1F;
        if (isComposing) flags |= nveEventFlagComposing;
        final keyBytes = utf8.encode(key);
        final codeBytes = utf8.encode(code);
        p[0] = keyCode;
        p[1] = keyBytes.length;
        if (keyBytes.length <= nveEventInlineKeyCap) {
          final kb = Uint8List(nveEventInlineKeyCap)..setRange(0, keyBytes.length, keyBytes);
          final kbd = ByteData.sublistView(kb);
          for (var w = 0; w < 4; w++) {
            p[2 + w] = kbd.getUint32(w * 4, Endian.little);
          }
          flags |= nveEventFlagInlineKeyUtf8;
          if (codeBytes.isNotEmpty) flags |= nveEventFlagUtf8Bytes;
          appendData(codeBytes);
        } else {
          flags |= nveEventFlagKeySplitBytes | nveEventFlagUtf8Bytes;
          final combined = BytesBuilder(copy: false)
            ..add(keyBytes)
            ..add(codeBytes);
          appendData(combined.takeBytes());
        }
      case NvePointerPayload(:final x, :final y, :final modifiers, flags: final pflags):
        flags = (modifiers & 0x1F) |
            (((pflags & 0x07) << nveEventFlagButtonShift) & nveEventFlagButtonMask) |
            ((pflags & 0x08) != 0 ? nveEventFlagCapture : 0);
        p[0] = _f32Bits(x);
        p[1] = _f32Bits(y);
      case NveWheelPayload(:final deltaX, :final deltaY, :final modifiers):
        flags |= modifiers & 0x1F;
        p[0] = _f32Bits(deltaX);
        p[1] = _f32Bits(deltaY);
      case NveTableEventPayload(:final tableKey, eventId: final tableEvId, :final bytes):
        table = tableKey;
        eventId = tableEvId;
        appendData(bytes);
    }

    dst
      ..timestamp = event.timestampUs
      ..table = table
      ..surface = event.surfaceId
      ..node = event.nodeId
      ..event = eventId
      ..flags = flags;
    for (var i = 0; i < 6; i++) {
      dst.payload[i] = p[i];
    }
    dst.bytes
      ..offset = bytesOff
      ..len = bytesLen;
  }

  static List<NveEvent> decodeBatch(NveEventBatch batch) {
    if (batch.events == ffi.nullptr || batch.eventCount == 0) return const <NveEvent>[];
    final arenaView = (batch.bytes == ffi.nullptr || batch.bytesLen == 0)
        ? Uint8List(0)
        : batch.bytes.asTypedList(batch.bytesLen);
    final out = <NveEvent>[];
    for (var i = 0; i < batch.eventCount; i++) {
      final ev = batch.events[i];
      final off = ev.bytes.offset;
      final len = ev.bytes.len;
      final slice = (len == 0 || off + len > arenaView.length)
          ? Uint8List(0)
          : Uint8List.fromList(arenaView.sublist(off, off + len));
      final p0 = ev.payload[0];
      final p1 = ev.payload[1];
      final p2 = ev.payload[2];
      final p3 = ev.payload[3];
      final p4 = ev.payload[4];
      final p5 = ev.payload[5];
      final flags = ev.flags;

      String? name;
      final NveEventKind kind;
      final NveEventPayload payload;

      if (ev.table == nveTableCustom) {
        kind = NveEventKind.custom;
        final nlen = p0 <= slice.length ? p0 : slice.length;
        name = utf8.decode(slice.sublist(0, nlen), allowMalformed: true);
        payload = NveBinValuePayload(Uint8List.fromList(slice.sublist(nlen)));
      } else if (ev.table == nveTableInteraction) {
        final ie = NveInteractionEvent.fromCode(ev.event);
        switch (ie) {
          case NveInteractionEvent.press:
            kind = NveEventKind.press;
            payload = NvePointPayload(x: _bitsF32(p0), y: _bitsF32(p1));
          case NveInteractionEvent.longPress:
            kind = NveEventKind.longPress;
            payload = NvePointPayload(x: _bitsF32(p0), y: _bitsF32(p1));
          case NveInteractionEvent.focus:
            kind = NveEventKind.focus;
            payload = const NveEmptyPayload();
          case NveInteractionEvent.blur:
            kind = NveEventKind.blur;
            payload = const NveEmptyPayload();
          case NveInteractionEvent.keyDown || NveInteractionEvent.keyUp:
            kind = ie == NveInteractionEvent.keyDown ? NveEventKind.keyDown : NveEventKind.keyUp;
            final String key;
            final String code;
            if ((flags & nveEventFlagInlineKeyUtf8) != 0) {
              final klen = p1 <= nveEventInlineKeyCap ? p1 : nveEventInlineKeyCap;
              final kb = Uint8List(16);
              final kbd = ByteData.sublistView(kb);
              kbd.setUint32(0, p2, Endian.little);
              kbd.setUint32(4, p3, Endian.little);
              kbd.setUint32(8, p4, Endian.little);
              kbd.setUint32(12, p5, Endian.little);
              key = utf8.decode(kb.sublist(0, klen), allowMalformed: true);
              code = utf8.decode(slice, allowMalformed: true);
            } else {
              final klen = p1 <= slice.length ? p1 : slice.length;
              key = utf8.decode(slice.sublist(0, klen), allowMalformed: true);
              code = utf8.decode(slice.sublist(klen), allowMalformed: true);
            }
            payload = NveKeyPayload(
              key: key,
              code: code,
              modifiers: flags & 0x1F,
              keyCode: p0,
              isComposing: (flags & nveEventFlagComposing) != 0,
            );
          case NveInteractionEvent.pointerDown ||
              NveInteractionEvent.pointerMove ||
              NveInteractionEvent.pointerUp ||
              NveInteractionEvent.pointerEnter ||
              NveInteractionEvent.pointerLeave:
            kind = switch (ie) {
              NveInteractionEvent.pointerDown => NveEventKind.pointerDown,
              NveInteractionEvent.pointerMove => NveEventKind.pointerMove,
              NveInteractionEvent.pointerUp => NveEventKind.pointerUp,
              NveInteractionEvent.pointerEnter => NveEventKind.pointerEnter,
              _ => NveEventKind.pointerLeave,
            };
            final btn = (flags & nveEventFlagButtonMask) >> nveEventFlagButtonShift;
            final cap = (flags & nveEventFlagCapture) != 0 ? 0x08 : 0;
            payload = NvePointerPayload(
              x: _bitsF32(p0),
              y: _bitsF32(p1),
              modifiers: flags & 0x1F,
              flags: btn | cap,
            );
          case NveInteractionEvent.wheel:
            kind = NveEventKind.wheel;
            payload = NveWheelPayload(
              deltaX: _bitsF32(p0),
              deltaY: _bitsF32(p1),
              modifiers: flags & 0x1F,
            );
          case null:
            continue;
        }
      } else if (ev.table == nveTableTextInput) {
        kind = switch (NveTextInputEvent.fromCode(ev.event)) {
          NveTextInputEvent.change => NveEventKind.textChange,
          NveTextInputEvent.submit => NveEventKind.submit,
          NveTextInputEvent.compositionStart => NveEventKind.compositionStart,
          NveTextInputEvent.compositionUpdate => NveEventKind.compositionUpdate,
          NveTextInputEvent.compositionEnd => NveEventKind.compositionEnd,
          null => NveEventKind.textChange,
        };
        payload = NveTextPayload(utf8.decode(slice, allowMalformed: true));
      } else if (ev.table == nveTableScroll) {
        kind = NveEventKind.scroll;
        payload = NveScrollPayload(
          x: _bitsF32(p0),
          y: _bitsF32(p1),
          contentWidth: _bitsF32(p2),
          contentHeight: _bitsF32(p3),
        );
      } else if (ev.table == nveTableLayout) {
        kind = NveEventKind.layout;
        payload = NveLayoutPayload(
          x: _bitsF32(p0),
          y: _bitsF32(p1),
          width: _bitsF32(p2),
          height: _bitsF32(p3),
        );
      } else {
        kind = NveEventKind.tableEvent;
        payload = NveTableEventPayload(tableKey: ev.table, eventId: ev.event, bytes: slice);
      }
      out.add(NveEvent(
        surfaceId: ev.surface,
        nodeId: ev.node,
        timestampUs: ev.timestamp,
        kind: kind,
        name: name,
        payload: payload,
      ));
    }
    return out;
  }
}
