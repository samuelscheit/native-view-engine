/// Native View Engine (NVE) Flutter host bindings and widgets.
///
/// Provides:
/// - Direct `dart:ffi` bindings to NVE's modular `nve_get_interface(uint32_t)` C ABI
///   (`NveSurfaceApi`, `NveLayoutApi`, `NveComponentApi`, `NveDevApi`).
/// - Binary `EventRing` codec matching `crates/nve-abi/src/events.rs`.
/// - [NveTextureHostWidget]: zero-copy 120fps Skia compositing via `FlutterTextureRegistry`
///   (`Texture(textureId: ...)`) across iOS, Android, macOS, Windows, and Linux.
/// - [NvePlatformViewWidget]: native Fabric view embedding via `UiKitView` / `AndroidView`.
/// - [NveSurfaceController]: imperative command dispatch, hot-patch application, and snapshots.
/// - [NveChannelClient]: typed Dart `Stream` bridge to `NveChannelBus`.
library nve_flutter;

import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io' show Platform;
import 'dart:typed_data';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

// ============================================================================
// 1. Direct dart:ffi bindings to modular `nve_get_interface` C ABI (`nve_abi.h`)
// ============================================================================

/// Modular interface IDs passed to `nve_get_interface(uint32_t)`.
const int nveIfaceSurface = 1;
const int nveIfaceLayout = 2;
const int nveIfaceComponent = 3;
const int nveIfaceDev = 4;

/// C ABI byte slice (`NveBufferSlice` in `nve_abi.h`).
final class NveBufferSlice extends ffi.Struct {
  external ffi.Pointer<ffi.Uint8> ptr;

  @ffi.Size()
  external int len;
}

/// C ABI surface creation config (`NveSurfaceConfig` in `nve_abi.h`).
final class NveSurfaceConfig extends ffi.Struct {
  @ffi.Uint32()
  external int surfaceId;

  @ffi.Float()
  external double width;

  @ffi.Float()
  external double height;

  @ffi.Float()
  external double scaleFactor;
}

/// C ABI frame performance counters (`NveFrameStats` in `nve_abi.h`).
final class NveFrameStats extends ffi.Struct {
  @ffi.Uint32()
  external int opCount;

  @ffi.Uint32()
  external int mutationCount;

  @ffi.Uint64()
  external int decodeUs;

  @ffi.Uint64()
  external int layoutUs;

  @ffi.Uint64()
  external int diffUs;

  @ffi.Uint64()
  external int mountUs;
}

/// Planned ABI v2 C-compatible box constraints (`SizeConstraints` in `crates/nve-interop`).
final class NveSizeConstraints extends ffi.Struct {
  @ffi.Float()
  external double minW;

  @ffi.Float()
  external double minH;

  @ffi.Float()
  external double maxW;

  @ffi.Float()
  external double maxH;
}

/// Planned ABI v2 C-compatible intrinsic dimensions (`IntrinsicSize` in `crates/nve-interop`).
final class NveIntrinsicSize extends ffi.Struct {
  @ffi.Float()
  external double width;

  @ffi.Float()
  external double height;
}

// Native function typedefs matching modular `nve_get_interface` tables in `crates/nve-abi/src/ffi.rs`.
typedef NveCreateNative = ffi.Pointer<ffi.Void> Function(ffi.Uint32 surfaceId);
typedef NveDestroyNative = ffi.Void Function(ffi.Pointer<ffi.Void> handle);
typedef NveRenderNative = NveBufferSlice Function(
  ffi.Pointer<ffi.Void> handle,
  ffi.Uint64 frameId,
);
typedef NveDispatchEventsNative = ffi.Uint32 Function(
  ffi.Pointer<ffi.Void> handle,
  NveBufferSlice events,
);
typedef NveApplyHotPatchNative = ffi.Uint32 Function(
  ffi.Pointer<ffi.Void> handle,
  NveBufferSlice patchJson,
);
typedef NveSnapshotStateNative = NveBufferSlice Function(ffi.Pointer<ffi.Void> handle);
typedef NveRestoreStateNative = ffi.Void Function(
  ffi.Pointer<ffi.Void> handle,
  NveBufferSlice state,
);
typedef NveSetPropsNative = ffi.Uint32 Function(
  ffi.Pointer<ffi.Void> handle,
  NveBufferSlice props,
);
typedef NveDrainEmittedEventsNative = NveBufferSlice Function(ffi.Pointer<ffi.Void> handle);

// Planned ABI v2 vtable extensions (negotiated when `abiVersion >= 2`):
//   NveIntrinsicSize (*measure)(NveAppHandle handle, NveSizeConstraints constraints);
//   uint8_t (*hit_test)(NveAppHandle handle, float x, float y, uint8_t pointer_phase);
typedef NveMeasureNative = NveIntrinsicSize Function(
  ffi.Pointer<ffi.Void> handle,
  NveSizeConstraints constraints,
);
typedef NveHitTestNative = ffi.Uint8 Function(
  ffi.Pointer<ffi.Void> handle,
  ffi.Float x,
  ffi.Float y,
  ffi.Uint8 pointerPhase,
);

/// Single exported entry point (`nve_get_interface`).
typedef NveGetInterfaceNative = ffi.Pointer<ffi.Void> Function(ffi.Uint32 interfaceId);

/// Required core surface interface (`NVE_IFACE_SURFACE = 1`).
final class NveSurfaceApi extends ffi.Struct {
  external ffi.Pointer<ffi.NativeFunction<NveCreateNative>> create;
  external ffi.Pointer<ffi.NativeFunction<NveDestroyNative>> destroy;
  external ffi.Pointer<ffi.NativeFunction<NveRenderNative>> render;
  external ffi.Pointer<ffi.NativeFunction<NveDispatchEventsNative>> dispatchEvents;
  external ffi.Pointer<ffi.Void> dispatchEventsDirect;
}

/// Optional layout & gesture interface (`NVE_IFACE_LAYOUT = 2`).
final class NveLayoutApi extends ffi.Struct {
  external ffi.Pointer<ffi.NativeFunction<NveMeasureNative>> measure;
  external ffi.Pointer<ffi.NativeFunction<NveHitTestNative>> hitTest;
}

/// Optional component props & outbound event interface (`NVE_IFACE_COMPONENT = 3`).
final class NveComponentApi extends ffi.Struct {
  external ffi.Pointer<ffi.NativeFunction<NveSetPropsNative>> setProps;
  external ffi.Pointer<ffi.NativeFunction<NveDrainEmittedEventsNative>> drainEmittedEvents;
  external ffi.Pointer<ffi.Void> setPropsDirect;
}

/// Optional dev / 2-tier hot-reload interface (`NVE_IFACE_DEV = 4`, may be NULL in release builds).
final class NveDevApi extends ffi.Struct {
  external ffi.Pointer<ffi.NativeFunction<NveApplyHotPatchNative>> applyHotPatch;
  external ffi.Pointer<ffi.NativeFunction<NveSnapshotStateNative>> snapshotState;
  external ffi.Pointer<ffi.NativeFunction<NveRestoreStateNative>> restoreState;
}

/// Self-contained C heap allocator so `nve_flutter` requires zero external pub packages.
class _NativeMem {
  static final ffi.DynamicLibrary _libc = Platform.isWindows
      ? ffi.DynamicLibrary.open('ucrtbase.dll')
      : ffi.DynamicLibrary.process();

  static final ffi.Pointer<ffi.Void> Function(int) _malloc = _libc.lookupFunction<
      ffi.Pointer<ffi.Void> Function(ffi.Size), ffi.Pointer<ffi.Void> Function(int)>('malloc');
  static final void Function(ffi.Pointer<ffi.Void>) _free = _libc.lookupFunction<
      ffi.Void Function(ffi.Pointer<ffi.Void>), void Function(ffi.Pointer<ffi.Void>)>('free');

  static ffi.Pointer<T> allocate<T extends ffi.NativeType>(int byteCount) {
    final ptr = _malloc(byteCount == 0 ? 1 : byteCount);
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

// ============================================================================
// 2. EventRing Binary Codec & Event Types (mirrors `crates/nve-abi/src/events.rs`)
// ============================================================================

/// Gesture disposition returned by planned ABI v2 `hit_test` and `GestureArbitrator`.
enum NveGestureDisposition {
  claim(1),
  passThrough(2),
  deferToParent(3);

  const NveGestureDisposition(this.code);
  final int code;

  static NveGestureDisposition fromCode(int code) {
    return switch (code) {
      1 => NveGestureDisposition.claim,
      3 => NveGestureDisposition.deferToParent,
      _ => NveGestureDisposition.passThrough,
    };
  }
}

/// Native event discriminator matching `NativeEventKind` in `crates/nve-abi/src/events.rs`.
enum NveEventKind {
  press(1),
  longPress(2),
  scroll(3),
  textChange(4),
  focus(5),
  blur(6),
  layout(7),
  custom(8);

  const NveEventKind(this.code);
  final int code;

  static NveEventKind fromCode(int code) {
    return switch (code) {
      1 => NveEventKind.press,
      2 => NveEventKind.longPress,
      3 => NveEventKind.scroll,
      4 => NveEventKind.textChange,
      5 => NveEventKind.focus,
      6 => NveEventKind.blur,
      7 => NveEventKind.layout,
      _ => NveEventKind.custom,
    };
  }
}

/// Decoded outbound or inbound NVE event.
class NveEvent {
  const NveEvent({
    required this.surfaceId,
    required this.nodeId,
    required this.timestampUs,
    required this.kind,
    this.name,
    this.payloadJson,
    Uint8List? rawPayload,
  }) : _rawPayload = rawPayload;

  static final Uint8List _emptyBytes = Uint8List(0);

  final int surfaceId;
  final int nodeId;
  final int timestampUs;
  final NveEventKind kind;

  /// Custom event name (e.g. `"onIncrement"`) when [kind] is [NveEventKind.custom].
  final String? name;

  /// Decoded JSON map if the payload contains UTF-8 JSON object data.
  final Map<String, dynamic>? payloadJson;

  final Uint8List? _rawPayload;

  /// Raw binary payload bytes.
  Uint8List get rawPayload => _rawPayload ?? _emptyBytes;

  factory NveEvent.fromPlatformMap(Map<Object?, Object?> map) {
    final kindRaw = map['kind'];
    final kind = kindRaw is int
        ? NveEventKind.fromCode(kindRaw)
        : NveEventKind.custom;
    final rawPayloadObj = map['rawPayload'];
    final rawBytes = rawPayloadObj is Uint8List ? rawPayloadObj : _emptyBytes;
    Map<String, dynamic>? jsonPayload;
    final payloadObj = map['payload'];
    if (payloadObj is Map) {
      jsonPayload = Map<String, dynamic>.from(payloadObj);
    } else if (rawBytes.isNotEmpty) {
      jsonPayload = _tryDecodeJsonMap(rawBytes);
    }
    return NveEvent(
      surfaceId: (map['surfaceId'] as int?) ?? 0,
      nodeId: (map['nodeId'] as int?) ?? 0,
      timestampUs: (map['timestampUs'] as int?) ?? 0,
      kind: kind,
      name: map['name'] as String?,
      payloadJson: jsonPayload,
      rawPayload: rawBytes,
    );
  }
}

typedef NveEventCallback = void Function(NveEvent event);

Map<String, dynamic>? _tryDecodeJsonMap(Uint8List bytes) {
  if (bytes.isEmpty) return null;
  try {
    final decoded = jsonDecode(utf8.decode(bytes));
    if (decoded is Map<String, dynamic>) {
      return decoded;
    }
  } catch (_) {
    // Payload is binary rather than a JSON object.
  }
  return null;
}

/// Binary encoder/decoder for `EventRing` buffers matching `crates/nve-abi/src/events.rs`.
class NveEventRingCodec {
  const NveEventRingCodec._();

  static List<NveEvent> decode(Uint8List bytes) {
    if (bytes.length < 4) return const <NveEvent>[];
    final bd = ByteData.sublistView(bytes);
    var pos = 0;
    final count = bd.getUint32(pos, Endian.little);
    pos += 4;
    final events = <NveEvent>[];

    for (var i = 0; i < count; i++) {
      if (pos + 18 > bytes.length) break;
      final surfaceId = bd.getUint32(pos, Endian.little);
      pos += 4;
      final nodeId = bd.getUint32(pos, Endian.little);
      pos += 4;
      final timestampUs = bd.getUint64(pos, Endian.little);
      pos += 8;
      final kind = NveEventKind.fromCode(bytes[pos++]);
      final payloadTag = bytes[pos++];

      String? name;
      Map<String, dynamic>? payloadJson;
      Uint8List rawPayload = Uint8List(0);

      var truncated = false;
      switch (payloadTag) {
        case 0: // Empty
          break;
        case 1: // Point { x, y }
          if (pos + 8 > bytes.length) {
            truncated = true;
            break;
          }
          final x = bd.getFloat32(pos, Endian.little);
          final y = bd.getFloat32(pos + 4, Endian.little);
          pos += 8;
          payloadJson = <String, dynamic>{'x': x, 'y': y};
        case 2: // Scroll { x, y, content_width, content_height }
          if (pos + 16 > bytes.length) {
            truncated = true;
            break;
          }
          final x = bd.getFloat32(pos, Endian.little);
          final y = bd.getFloat32(pos + 4, Endian.little);
          final cw = bd.getFloat32(pos + 8, Endian.little);
          final ch = bd.getFloat32(pos + 12, Endian.little);
          pos += 16;
          payloadJson = <String, dynamic>{
            'x': x,
            'y': y,
            'contentWidth': cw,
            'contentHeight': ch,
          };
        case 3: // Text(String)
          if (pos + 4 > bytes.length) {
            truncated = true;
            break;
          }
          final len = bd.getUint32(pos, Endian.little);
          pos += 4;
          if (len > bytes.length - pos) {
            truncated = true;
            break;
          }
          rawPayload = Uint8List.fromList(Uint8List.sublistView(bytes, pos, pos + len));
          pos += len;
          payloadJson = <String, dynamic>{'text': utf8.decode(rawPayload, allowMalformed: true)};
        case 4: // Layout { x, y, width, height }
          if (pos + 16 > bytes.length) {
            truncated = true;
            break;
          }
          final x = bd.getFloat32(pos, Endian.little);
          final y = bd.getFloat32(pos + 4, Endian.little);
          final w = bd.getFloat32(pos + 8, Endian.little);
          final h = bd.getFloat32(pos + 12, Endian.little);
          pos += 16;
          payloadJson = <String, dynamic>{
            'x': x,
            'y': y,
            'width': w,
            'height': h,
          };
        case 5: // Custom { name, data }
          if (pos + 2 > bytes.length) {
            truncated = true;
            break;
          }
          final nameLen = bd.getUint16(pos, Endian.little);
          pos += 2;
          if (nameLen > bytes.length - pos) {
            truncated = true;
            break;
          }
          name = utf8.decode(Uint8List.sublistView(bytes, pos, pos + nameLen), allowMalformed: true);
          pos += nameLen;
          if (pos + 4 > bytes.length) {
            truncated = true;
            break;
          }
          final dataLen = bd.getUint32(pos, Endian.little);
          pos += 4;
          if (dataLen > bytes.length - pos) {
            truncated = true;
            break;
          }
          rawPayload = Uint8List.fromList(Uint8List.sublistView(bytes, pos, pos + dataLen));
          pos += dataLen;
          payloadJson = _tryDecodeJsonMap(rawPayload);
        default:
          truncated = true;
      }

      if (truncated) break;

      events.add(
        NveEvent(
          surfaceId: surfaceId,
          nodeId: nodeId,
          timestampUs: timestampUs,
          kind: kind,
          name: name,
          payloadJson: payloadJson,
          rawPayload: rawPayload,
        ),
      );
    }

    return events;
  }

  static Uint8List encodeCustomEvents(List<NveEvent> events) {
    final builder = BytesBuilder(copy: false);
    final header = ByteData(4)..setUint32(0, events.length, Endian.little);
    builder.add(header.buffer.asUint8List());

    for (final ev in events) {
      final meta = ByteData(18)
        ..setUint32(0, ev.surfaceId, Endian.little)
        ..setUint32(4, ev.nodeId, Endian.little)
        ..setUint64(8, ev.timestampUs, Endian.little)
        ..setUint8(16, ev.kind.code)
        ..setUint8(17, 5); // EventPayload::Custom
      builder.add(meta.buffer.asUint8List());

      final nameBytes = utf8.encode(ev.name ?? '');
      final dataBytes = ev.rawPayload.isNotEmpty
          ? ev.rawPayload
          : (ev.payloadJson != null
              ? Uint8List.fromList(utf8.encode(jsonEncode(ev.payloadJson)))
              : Uint8List(0));

      final nameLen = ByteData(2)..setUint16(0, nameBytes.length, Endian.little);
      builder.add(nameLen.buffer.asUint8List());
      builder.add(nameBytes);

      final dataLen = ByteData(4)..setUint32(0, dataBytes.length, Endian.little);
      builder.add(dataLen.buffer.asUint8List());
      builder.add(dataBytes);
    }

    return builder.toBytes();
  }
}

// ============================================================================
// 3. Direct FFI Runtime Wrapper (`NveAppInstance`)
// ============================================================================

/// Managed Dart wrapper around a native NVE module resolved via `nve_get_interface`.
class NveAppInstance {
  NveAppInstance._({
    required this.surfaceId,
    required ffi.Pointer<NveSurfaceApi> surface,
    required ffi.Pointer<NveLayoutApi> layout,
    required ffi.Pointer<NveComponentApi> component,
    required ffi.Pointer<NveDevApi> dev,
    required ffi.Pointer<ffi.Void> handle,
  })  : _layout = layout,
        _handle = handle,
        _destroy = surface.ref.destroy.asFunction(),
        _render = surface.ref.render.asFunction(),
        _dispatchEvents = surface.ref.dispatchEvents.asFunction(),
        _setProps = component != ffi.nullptr ? component.ref.setProps.asFunction() : null,
        _drainEmittedEvents =
            component != ffi.nullptr ? component.ref.drainEmittedEvents.asFunction() : null,
        _applyHotPatch = dev != ffi.nullptr ? dev.ref.applyHotPatch.asFunction() : null,
        _snapshotState = dev != ffi.nullptr ? dev.ref.snapshotState.asFunction() : null,
        _restoreState = dev != ffi.nullptr ? dev.ref.restoreState.asFunction() : null;

  final int surfaceId;
  final ffi.Pointer<NveLayoutApi> _layout;
  ffi.Pointer<ffi.Void> _handle;
  bool _disposed = false;
  int _frameCounter = 0;

  final void Function(ffi.Pointer<ffi.Void>) _destroy;
  final NveBufferSlice Function(ffi.Pointer<ffi.Void>, int) _render;
  final int Function(ffi.Pointer<ffi.Void>, NveBufferSlice) _dispatchEvents;
  final int Function(ffi.Pointer<ffi.Void>, NveBufferSlice)? _setProps;
  final NveBufferSlice Function(ffi.Pointer<ffi.Void>)? _drainEmittedEvents;
  final int Function(ffi.Pointer<ffi.Void>, NveBufferSlice)? _applyHotPatch;
  final NveBufferSlice Function(ffi.Pointer<ffi.Void>)? _snapshotState;
  final void Function(ffi.Pointer<ffi.Void>, NveBufferSlice)? _restoreState;

  bool get isDisposed => _disposed;
  bool get supportsLayoutAndHitTest => _layout != ffi.nullptr;

  /// Loads `vtableSymbol` (`nve_get_interface`) from `libraryPath` (or the current process) and creates a surface.
  factory NveAppInstance.open({
    required int surfaceId,
    String vtableSymbol = 'nve_get_interface',
    String? libraryPath,
  }) {
    final lib = libraryPath != null && libraryPath.isNotEmpty
        ? ffi.DynamicLibrary.open(libraryPath)
        : ffi.DynamicLibrary.process();
    final getIface = lib.lookupFunction<NveGetInterfaceNative, ffi.Pointer<ffi.Void> Function(int)>(
      vtableSymbol,
    );
    return NveAppInstance.fromInterfaceResolver(surfaceId: surfaceId, getInterface: getIface);
  }

  /// Creates an instance from a resolved `nve_get_interface` function.
  factory NveAppInstance.fromInterfaceResolver({
    required int surfaceId,
    required ffi.Pointer<ffi.Void> Function(int) getInterface,
  }) {
    final surface = getInterface(nveIfaceSurface).cast<NveSurfaceApi>();
    if (surface == ffi.nullptr) {
      throw StateError('NVE module missing required NVE_IFACE_SURFACE interface');
    }
    final layout = getInterface(nveIfaceLayout).cast<NveLayoutApi>();
    final component = getInterface(nveIfaceComponent).cast<NveComponentApi>();
    final dev = getInterface(nveIfaceDev).cast<NveDevApi>();

    final createFn = surface.ref.create.asFunction<ffi.Pointer<ffi.Void> Function(int)>();
    final handle = createFn(surfaceId);
    if (handle == ffi.nullptr) {
      throw StateError('NveSurfaceApi.create($surfaceId) returned NULL');
    }
    return NveAppInstance._(
      surfaceId: surfaceId,
      surface: surface,
      layout: layout,
      component: component,
      dev: dev,
      handle: handle,
    );
  }

  /// Pushes JSON-encoded props into the Rust NVE component (`NveComponentApi.set_props`).
  bool setPropsJson(Map<String, dynamic> props) {
    _ensureLive();
    final setProps = _setProps;
    if (setProps == null) return false;
    final bytes = Uint8List.fromList(utf8.encode(jsonEncode(props)));
    return _withSlice(bytes, (slice) => setProps(_handle, slice) != 0);
  }

  /// Renders the next CommandBuffer frame and returns a copy of its bytes.
  Uint8List renderNextFrame() {
    _ensureLive();
    _frameCounter += 1;
    final slice = _render(_handle, _frameCounter);
    return _copySlice(slice);
  }

  /// Dispatches an encoded binary `EventRing` batch into the Rust app.
  bool dispatchEvents(Uint8List eventRingBytes) {
    _ensureLive();
    return _withSlice(eventRingBytes, (slice) => _dispatchEvents(_handle, slice) != 0);
  }

  /// Applies a Tier-1 JSON hot-reload patch (`NveDevApi.apply_hot_patch`).
  bool applyHotPatch(String patchJson) {
    _ensureLive();
    final fn = _applyHotPatch;
    if (fn == null) return false;
    final bytes = Uint8List.fromList(utf8.encode(patchJson));
    return _withSlice(bytes, (slice) => fn(_handle, slice) != 0);
  }

  /// Snapshots app state for Tier-2 hot-reload preservation (`NveDevApi.snapshot_state`).
  Uint8List snapshotState() {
    _ensureLive();
    final fn = _snapshotState;
    if (fn == null) return Uint8List(0);
    return _copySlice(fn(_handle));
  }

  /// Restores previously snapshotted state (`NveDevApi.restore_state`).
  void restoreState(Uint8List stateBytes) {
    _ensureLive();
    final fn = _restoreState;
    if (fn == null) return;
    _withSlice(stateBytes, (slice) {
      fn(_handle, slice);
      return 0;
    });
  }

  /// Drains outbound events emitted by the Rust component (`NveComponentApi.drain_emitted_events`).
  List<NveEvent> drainEmittedEvents() {
    _ensureLive();
    final fn = _drainEmittedEvents;
    if (fn == null) return const [];
    final slice = fn(_handle);
    final raw = _copySlice(slice);
    return NveEventRingCodec.decode(raw);
  }

  /// Negotiates intrinsic dimensions via zero-cost `NveLayoutApi.measure`.
  Size? measure(BoxConstraints constraints) {
    _ensureLive();
    if (_layout == ffi.nullptr || _layout.ref.measure == ffi.nullptr) return null;
    final measureFn = _layout.ref.measure
        .asFunction<NveIntrinsicSize Function(ffi.Pointer<ffi.Void>, NveSizeConstraints)>();
    final cPtr = _NativeMem.allocate<NveSizeConstraints>(ffi.sizeOf<NveSizeConstraints>());
    try {
      cPtr.ref
        ..minW = constraints.minWidth
        ..minH = constraints.minHeight
        ..maxW = constraints.maxWidth
        ..maxH = constraints.maxHeight;
      final out = measureFn(_handle, cPtr.ref);
      return Size(out.width, out.height);
    } finally {
      _NativeMem.free(cPtr);
    }
  }

  /// Queries gesture ownership via zero-cost `NveLayoutApi.hit_test`.
  NveGestureDisposition hitTest(Offset position, {int pointerPhase = 1}) {
    _ensureLive();
    if (_layout == ffi.nullptr || _layout.ref.hitTest == ffi.nullptr) {
      return NveGestureDisposition.claim;
    }
    final fn = _layout.ref.hitTest
        .asFunction<int Function(ffi.Pointer<ffi.Void>, double, double, int)>();
    return NveGestureDisposition.fromCode(fn(_handle, position.dx, position.dy, pointerPhase));
  }

  void dispose() {
    if (_disposed) return;
    _disposed = true;
    if (_handle != ffi.nullptr) {
      _destroy(_handle);
      _handle = ffi.nullptr;
    }
  }

  void _ensureLive() {
    if (_disposed || _handle == ffi.nullptr) {
      throw StateError('NveAppInstance(surfaceId=$surfaceId) has been disposed');
    }
  }

  T _withSlice<T>(Uint8List bytes, T Function(NveBufferSlice slice) body) {
    final dataPtr = _NativeMem.copyBytes(bytes);
    final slicePtr = _NativeMem.allocate<NveBufferSlice>(ffi.sizeOf<NveBufferSlice>());
    try {
      final slice = _NativeMem.sliceFor(dataPtr, bytes.length, slicePtr);
      return body(slice);
    } finally {
      _NativeMem.free(slicePtr);
      _NativeMem.free(dataPtr);
    }
  }

  Uint8List _copySlice(NveBufferSlice slice) {
    if (slice.ptr == ffi.nullptr || slice.len == 0) {
      return Uint8List(0);
    }
    return Uint8List.fromList(slice.ptr.asTypedList(slice.len));
  }
}

// ============================================================================
// 4. Imperative Surface Controller & Shared Channel Bus Client
// ============================================================================

/// Imperative controller attached to [NveTextureHostWidget] or [NvePlatformViewWidget].
class NveSurfaceController {
  void Function(String name, Map<String, dynamic> args)? _commandHandler;
  VoidCallback? _renderHandler;
  Uint8List? Function()? _snapshotHandler;

  bool get isAttached => _commandHandler != null;

  void _attach({
    required void Function(String name, Map<String, dynamic> args) onCommand,
    required VoidCallback onRender,
    required Uint8List? Function() onSnapshot,
  }) {
    _commandHandler = onCommand;
    _renderHandler = onRender;
    _snapshotHandler = onSnapshot;
  }

  void _detach() {
    _commandHandler = null;
    _renderHandler = null;
    _snapshotHandler = null;
  }

  /// Dispatches an imperative component command (matching `codegenNativeCommands` / `NveExportSpec.commands`).
  void dispatchCommand(String name, [Map<String, dynamic> args = const <String, dynamic>{}]) {
    _commandHandler?.call(name, args);
  }

  /// Requests an immediate frame render from the underlying NVE surface.
  void requestRender() {
    _renderHandler?.call();
  }

  /// Captures a state snapshot if backed by a direct FFI instance.
  Uint8List? snapshotState() => _snapshotHandler?.call();
}

/// Message delivered across [NveChannelClient] and Rust's `NveChannelBus`.
class NveChannelMessage {
  const NveChannelMessage({
    required this.channelId,
    required this.topic,
    required this.seq,
    required this.payload,
    this.senderSurfaceId,
    this.isJson = true,
  });

  final int channelId;
  final String topic;
  final int seq;
  final Uint8List payload;
  final int? senderSurfaceId;
  final bool isJson;

  Map<String, dynamic>? decodeJson() => _tryDecodeJsonMap(payload);
}

/// Bridges Dart `Stream`s to `NveChannelBus` (`theme`, `locale`, `auth`, `navigation`, custom topics).
class NveChannelClient {
  NveChannelClient({
    MethodChannel? methodChannel,
    EventChannel? eventChannel,
  })  : _methodChannel = methodChannel ?? const MethodChannel('dev.nve/channel_bus'),
        _eventChannel = eventChannel ?? const EventChannel('dev.nve/channel_bus/events');

  static const String topicTheme = 'theme';
  static const String topicLocale = 'locale';
  static const String topicAuth = 'auth';
  static const String topicNavigation = 'navigation';

  final MethodChannel _methodChannel;
  final EventChannel _eventChannel;
  final Map<String, StreamController<NveChannelMessage>> _topicControllers =
      <String, StreamController<NveChannelMessage>>{};
  final Map<String, NveChannelMessage> _retained = <String, NveChannelMessage>{};
  final Map<String, int> _topicIds = <String, int>{
    topicTheme: 1,
    topicLocale: 2,
    topicAuth: 3,
    topicNavigation: 4,
  };
  int _nextCustomId = 100;
  int _seq = 0;
  StreamSubscription<dynamic>? _platformSub;

  /// Returns a broadcast stream of messages for `topic`, immediately replaying the latest
  /// retained shared-state value when `replayRetained` is `true`.
  Stream<NveChannelMessage> subscribe(String topic, {bool replayRetained = true}) {
    _ensurePlatformListener();
    final ctrl = _topicControllers.putIfAbsent(
      topic,
      () => StreamController<NveChannelMessage>.broadcast(),
    );
    _methodChannel.invokeMethod<void>('subscribe', <String, dynamic>{'topic': topic}).catchError(
      (_) {},
    );

    if (replayRetained && _retained.containsKey(topic)) {
      final latest = _retained[topic]!;
      return _prependStream(latest, ctrl.stream);
    }
    return ctrl.stream;
  }

  /// Publishes raw bytes to `topic` across both local Dart listeners and the native `NveChannelBus`.
  Future<void> publish(
    String topic,
    Uint8List payload, {
    bool retain = false,
    bool isJson = false,
    int? senderSurfaceId,
  }) async {
    final id = _topicIds.putIfAbsent(topic, () => _nextCustomId++);
    _seq += 1;
    final msg = NveChannelMessage(
      channelId: id,
      topic: topic,
      seq: _seq,
      payload: payload,
      senderSurfaceId: senderSurfaceId,
      isJson: isJson,
    );
    if (retain) {
      _retained[topic] = msg;
    }
    _topicControllers[topic]?.add(msg);
    try {
      await _methodChannel.invokeMethod<void>('publish', <String, dynamic>{
        'channelId': id,
        'topic': topic,
        'payload': payload,
        'retain': retain,
        'isJson': isJson,
        if (senderSurfaceId != null) 'senderSurfaceId': senderSurfaceId,
      });
    } on MissingPluginException {
      // Allow pure-Dart / widget-test usage when native plugin host is not registered.
    }
  }

  /// Publishes a JSON object payload to `topic`.
  Future<void> publishJson(
    String topic,
    Map<String, dynamic> data, {
    bool retain = false,
    int? senderSurfaceId,
  }) {
    final bytes = Uint8List.fromList(utf8.encode(jsonEncode(data)));
    return publish(
      topic,
      bytes,
      retain: retain,
      isJson: true,
      senderSurfaceId: senderSurfaceId,
    );
  }

  /// Returns the last retained message for `topic`, if any.
  NveChannelMessage? retained(String topic) => _retained[topic];

  Future<void> unsubscribe(String topic) async {
    final ctrl = _topicControllers.remove(topic);
    await ctrl?.close();
    try {
      await _methodChannel.invokeMethod<void>('unsubscribe', <String, dynamic>{'topic': topic});
    } on MissingPluginException {
      // Ignored in tests/headless mode.
    }
  }

  Future<void> dispose() async {
    await _platformSub?.cancel();
    _platformSub = null;
    for (final ctrl in _topicControllers.values) {
      await ctrl.close();
    }
    _topicControllers.clear();
  }

  void _ensurePlatformListener() {
    if (_platformSub != null) return;
    try {
      _platformSub = _eventChannel.receiveBroadcastStream().listen(
        (dynamic raw) {
          if (raw is! Map) return;
          final topic = raw['topic'] as String?;
          if (topic == null) return;
          final bytes = raw['payload'] is Uint8List
              ? raw['payload'] as Uint8List
              : Uint8List(0);
          final msg = NveChannelMessage(
            channelId: (raw['channelId'] as int?) ??
                _topicIds.putIfAbsent(topic, () => _nextCustomId++),
            topic: topic,
            seq: (raw['seq'] as int?) ?? (++_seq),
            payload: bytes,
            senderSurfaceId: raw['senderSurfaceId'] as int?,
            isJson: (raw['isJson'] as bool?) ?? true,
          );
          if (raw['retained'] == true) {
            _retained[topic] = msg;
          }
          _topicControllers[topic]?.add(msg);
        },
        onError: (_) {},
      );
    } on MissingPluginException {
      // Pure-Dart local bus fallback.
    }
  }

  static Stream<NveChannelMessage> _prependStream(
    NveChannelMessage first,
    Stream<NveChannelMessage> rest,
  ) async* {
    yield first;
    yield* rest;
  }
}

// ============================================================================
// 5. Flutter Host Widgets: `NveTextureHostWidget` & `NvePlatformViewWidget`
// ============================================================================

/// Rendering strategy for embedding an NVE component inside Flutter.
enum NveRenderMode {
  /// Zero-copy 120fps Skia GPU texture via `FlutterTextureRegistry` (`Texture` widget).
  texture,

  /// Native React Native Fabric view via `UiKitView` / `AndroidView`.
  platformView,
}

int _nextSurfaceId = 1000;

/// Embeds an NVE Rust component into Flutter using zero-copy `Texture(textureId: ...)`
/// compositing at up to 120fps via `FlutterTextureRegistry` (Skia / Metal / Vulkan / Direct3D).
///
/// Works identically across iOS, Android, macOS, Windows, and Linux without platform-view
/// thread-merging penalties.
class NveTextureHostWidget extends StatefulWidget {
  const NveTextureHostWidget({
    super.key,
    required this.componentName,
    this.vtableSymbol = 'nve_get_interface',
    this.libraryPath,
    this.props = const <String, dynamic>{},
    this.controller,
    this.onEvent,
    this.surfaceId,
    this.channelClient,
    this.width,
    this.height,
  });

  final String componentName;
  final String vtableSymbol;
  final String? libraryPath;
  final Map<String, dynamic> props;
  final NveSurfaceController? controller;
  final NveEventCallback? onEvent;
  final int? surfaceId;
  final NveChannelClient? channelClient;
  final double? width;
  final double? height;

  @override
  State<NveTextureHostWidget> createState() => _NveTextureHostWidgetState();
}

class _NveTextureHostWidgetState extends State<NveTextureHostWidget> {
  static const MethodChannel _textureChannel = MethodChannel('dev.nve/flutter_texture');
  static const EventChannel _eventChannel = EventChannel('dev.nve/flutter_events');

  late final int _surfaceId = widget.surfaceId ?? (_nextSurfaceId++);
  int? _textureId;
  NveAppInstance? _ffiInstance;
  StreamSubscription<dynamic>? _eventSub;
  Size _lastReportedSize = Size.zero;

  @override
  void initState() {
    super.initState();
    _attachController();
    _initSurface();
  }

  @override
  void didUpdateWidget(covariant NveTextureHostWidget oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller != widget.controller) {
      oldWidget.controller?._detach();
      _attachController();
    }
    if (!mapEquals(oldWidget.props, widget.props)) {
      _pushProps(widget.props);
    }
  }

  @override
  void dispose() {
    widget.controller?._detach();
    _eventSub?.cancel();
    _ffiInstance?.dispose();
    if (_textureId != null) {
      _textureChannel.invokeMethod<void>('destroyTextureSurface', <String, dynamic>{
        'surfaceId': _surfaceId,
        'textureId': _textureId,
      }).catchError((_) {});
    }
    super.dispose();
  }

  void _attachController() {
    widget.controller?._attach(
      onCommand: _dispatchCommand,
      onRender: _requestRender,
      onSnapshot: () => _ffiInstance?.snapshotState(),
    );
  }

  Future<void> _initSurface() async {
    // Attempt direct FFI binding when a dynamic library path is explicitly provided.
    if (widget.libraryPath != null && widget.libraryPath!.isNotEmpty) {
      try {
        final instance = NveAppInstance.open(
          surfaceId: _surfaceId,
          vtableSymbol: widget.vtableSymbol,
          libraryPath: widget.libraryPath,
        );
        instance.setPropsJson(widget.props);
        instance.renderNextFrame();
        _ffiInstance = instance;
        _drainFfiEvents();
      } catch (_) {
        // Fallback to MethodChannel-hosted texture registry.
      }
    }

    try {
      final textureId = await _textureChannel.invokeMethod<int>(
        'createTextureSurface',
        <String, dynamic>{
          'surfaceId': _surfaceId,
          'componentName': widget.componentName,
          'vtableSymbol': widget.vtableSymbol,
          'libraryPath': widget.libraryPath,
          'propsJson': jsonEncode(widget.props),
        },
      );
      if (mounted && textureId != null) {
        setState(() => _textureId = textureId);
      }
      _eventSub = _eventChannel.receiveBroadcastStream(_surfaceId).listen(
        (dynamic payload) {
          if (payload is Map && widget.onEvent != null) {
            widget.onEvent!(NveEvent.fromPlatformMap(payload));
          }
        },
        onError: (_) {},
      );
    } on MissingPluginException {
      // Running in headless widget test or pure-Dart environment.
    }
  }

  void _pushProps(Map<String, dynamic> props) {
    final ffi = _ffiInstance;
    if (ffi != null && !ffi.isDisposed) {
      if (ffi.setPropsJson(props)) {
        ffi.renderNextFrame();
      }
      _drainFfiEvents();
    }
    _textureChannel.invokeMethod<void>('setProps', <String, dynamic>{
      'surfaceId': _surfaceId,
      'propsJson': jsonEncode(props),
    }).catchError((_) {});
  }

  void _dispatchCommand(String name, Map<String, dynamic> args) {
    final ffi = _ffiInstance;
    if (ffi != null && !ffi.isDisposed) {
      final ev = NveEvent(
        surfaceId: _surfaceId,
        nodeId: 1,
        timestampUs: DateTime.now().microsecondsSinceEpoch,
        kind: NveEventKind.custom,
        name: name,
        payloadJson: args,
      );
      if (ffi.dispatchEvents(NveEventRingCodec.encodeCustomEvents(<NveEvent>[ev]))) {
        ffi.renderNextFrame();
      }
      _drainFfiEvents();
    }
    _textureChannel.invokeMethod<void>('dispatchCommand', <String, dynamic>{
      'surfaceId': _surfaceId,
      'command': name,
      'argsJson': jsonEncode(args),
    }).catchError((_) {});
  }

  void _requestRender() {
    _ffiInstance?.renderNextFrame();
    _textureChannel.invokeMethod<void>('requestRender', <String, dynamic>{
      'surfaceId': _surfaceId,
    }).catchError((_) {});
  }

  void _drainFfiEvents() {
    final cb = widget.onEvent;
    final ffi = _ffiInstance;
    if (cb == null || ffi == null || ffi.isDisposed) return;
    for (final ev in ffi.drainEmittedEvents()) {
      cb(ev);
    }
  }

  void _notifyResize(Size size, double dpr) {
    if (size == _lastReportedSize || size.isEmpty) return;
    _lastReportedSize = size;
    _textureChannel.invokeMethod<void>('resizeSurface', <String, dynamic>{
      'surfaceId': _surfaceId,
      'width': size.width,
      'height': size.height,
      'scaleFactor': dpr,
    }).catchError((_) {});
  }

  void _forwardPointer(PointerEvent event, int phase) {
    _textureChannel.invokeMethod<void>('pointerEvent', <String, dynamic>{
      'surfaceId': _surfaceId,
      'pointerId': event.pointer,
      'phase': phase,
      'x': event.localPosition.dx,
      'y': event.localPosition.dy,
      'deltaX': event.delta.dx,
      'deltaY': event.delta.dy,
      'timestampUs': event.timeStamp.inMicroseconds,
    }).catchError((_) {});
  }

  @override
  Widget build(BuildContext context) {
    final dpr = MediaQuery.maybeDevicePixelRatioOf(context) ?? 1.0;

    return SizedBox(
      width: widget.width,
      height: widget.height,
      child: LayoutBuilder(
        builder: (BuildContext context, BoxConstraints constraints) {
          final intrinsic = _ffiInstance?.measure(constraints);
          final resolved = Size(
            widget.width ??
                intrinsic?.width ??
                (constraints.hasBoundedWidth ? constraints.maxWidth : 0.0),
            widget.height ??
                intrinsic?.height ??
                (constraints.hasBoundedHeight ? constraints.maxHeight : 0.0),
          );
          _notifyResize(resolved, dpr);

          final Widget surface = _textureId != null
              ? Texture(textureId: _textureId!, filterQuality: FilterQuality.low)
              : const SizedBox.expand();

          return Listener(
            behavior: HitTestBehavior.opaque,
            onPointerDown: (PointerDownEvent e) => _forwardPointer(e, 1),
            onPointerMove: (PointerMoveEvent e) => _forwardPointer(e, 2),
            onPointerUp: (PointerUpEvent e) => _forwardPointer(e, 3),
            onPointerCancel: (PointerCancelEvent e) => _forwardPointer(e, 4),
            child: SizedBox(
              width: resolved.width > 0 ? resolved.width : null,
              height: resolved.height > 0 ? resolved.height : null,
              child: surface,
            ),
          );
        },
      ),
    );
  }
}

/// Embeds an NVE component rendered via React Native Fabric native views (`UiKitView` on iOS,
/// `AndroidView` on Android), automatically falling back to [NveTextureHostWidget] on desktop.
class NvePlatformViewWidget extends StatefulWidget {
  const NvePlatformViewWidget({
    super.key,
    required this.componentName,
    this.vtableSymbol = 'nve_get_interface',
    this.libraryPath,
    this.props = const <String, dynamic>{},
    this.controller,
    this.onEvent,
    this.surfaceId,
    this.channelClient,
    this.width,
    this.height,
  });

  final String componentName;
  final String vtableSymbol;
  final String? libraryPath;
  final Map<String, dynamic> props;
  final NveSurfaceController? controller;
  final NveEventCallback? onEvent;
  final int? surfaceId;
  final NveChannelClient? channelClient;
  final double? width;
  final double? height;

  @override
  State<NvePlatformViewWidget> createState() => _NvePlatformViewWidgetState();
}

class _NvePlatformViewWidgetState extends State<NvePlatformViewWidget> {
  static const String _viewType = 'dev.nve/fabric_view';

  late final int _surfaceId = widget.surfaceId ?? (_nextSurfaceId++);
  MethodChannel? _viewChannel;

  @override
  void didUpdateWidget(covariant NvePlatformViewWidget oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller != widget.controller) {
      oldWidget.controller?._detach();
      _attachController();
    }
    if (!mapEquals(oldWidget.props, widget.props)) {
      _viewChannel?.invokeMethod<void>('setProps', <String, dynamic>{
        'propsJson': jsonEncode(widget.props),
      }).catchError((_) {});
    }
  }

  @override
  void dispose() {
    widget.controller?._detach();
    _viewChannel?.setMethodCallHandler(null);
    super.dispose();
  }

  void _attachController() {
    widget.controller?._attach(
      onCommand: (String name, Map<String, dynamic> args) {
        _viewChannel?.invokeMethod<void>('dispatchCommand', <String, dynamic>{
          'command': name,
          'argsJson': jsonEncode(args),
        }).catchError((_) {});
      },
      onRender: () {
        _viewChannel?.invokeMethod<void>('requestRender').catchError((_) {});
      },
      onSnapshot: () => null,
    );
  }

  void _onPlatformViewCreated(int viewId) {
    final channel = MethodChannel('dev.nve/fabric_view_$viewId');
    channel.setMethodCallHandler((MethodCall call) async {
      if (call.method == 'onEvent' && call.arguments is Map && widget.onEvent != null) {
        widget.onEvent!(NveEvent.fromPlatformMap(call.arguments as Map<Object?, Object?>));
      }
      return null;
    });
    _viewChannel = channel;
    _attachController();
  }

  @override
  Widget build(BuildContext context) {
    final creationParams = <String, dynamic>{
      'surfaceId': _surfaceId,
      'componentName': widget.componentName,
      'vtableSymbol': widget.vtableSymbol,
      'libraryPath': widget.libraryPath,
      'propsJson': jsonEncode(widget.props),
    };

    final Widget child;
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      child = UiKitView(
        viewType: _viewType,
        creationParams: creationParams,
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: _onPlatformViewCreated,
        gestureRecognizers: const <Factory<OneSequenceGestureRecognizer>>{},
      );
    } else if (defaultTargetPlatform == TargetPlatform.android) {
      child = AndroidView(
        viewType: _viewType,
        creationParams: creationParams,
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: _onPlatformViewCreated,
        hitTestBehavior: PlatformViewHitTestBehavior.opaque,
        gestureRecognizers: const <Factory<OneSequenceGestureRecognizer>>{},
      );
    } else {
      // Desktop (macOS / Windows / Linux) uses zero-copy Skia texture compositing.
      return NveTextureHostWidget(
        componentName: widget.componentName,
        vtableSymbol: widget.vtableSymbol,
        libraryPath: widget.libraryPath,
        props: widget.props,
        controller: widget.controller,
        onEvent: widget.onEvent,
        surfaceId: _surfaceId,
        channelClient: widget.channelClient,
        width: widget.width,
        height: widget.height,
      );
    }

    return SizedBox(
      width: widget.width,
      height: widget.height,
      child: child,
    );
  }
}
