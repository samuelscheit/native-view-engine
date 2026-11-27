import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'package:flutter/rendering.dart';

import 'direct_ffi.dart';
import 'event_codec.dart';
import 'package:nve_host/nve_host.dart';
import 'package:nve_host/nve_host_ffi.dart';

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
        _dispatchEvents = surface.ref.dispatchEvents != ffi.nullptr
            ? surface.ref.dispatchEvents
                .asFunction<int Function(ffi.Pointer<ffi.Void>, NveEventBatch)>()
            : null,
        _setProps = component != ffi.nullptr && component.ref.setProps != ffi.nullptr
            ? component.ref.setProps
                .asFunction<int Function(ffi.Pointer<ffi.Void>, ffi.Pointer<NveMapEntry>, int)>()
            : null,
        _drainEmittedEvents =
            component != ffi.nullptr && component.ref.drainEmittedEvents != ffi.nullptr
                ? component.ref.drainEmittedEvents
                    .asFunction<NveEventBatch Function(ffi.Pointer<ffi.Void>)>()
                : null,
        _applyHotPatch = dev != ffi.nullptr && dev.ref.applyHotPatch != ffi.nullptr
            ? dev.ref.applyHotPatch.asFunction()
            : null,
        _snapshotState = dev != ffi.nullptr && dev.ref.snapshotState != ffi.nullptr
            ? dev.ref.snapshotState.asFunction()
            : null,
        _restoreState = dev != ffi.nullptr && dev.ref.restoreState != ffi.nullptr
            ? dev.ref.restoreState.asFunction()
            : null;

  final int surfaceId;
  final ffi.Pointer<NveLayoutApi> _layout;
  ffi.Pointer<ffi.Void> _handle;
  bool _disposed = false;
  int _frameCounter = 0;

  final void Function(ffi.Pointer<ffi.Void>) _destroy;
  final NveBufferSlice Function(ffi.Pointer<ffi.Void>, int) _render;
  final int Function(ffi.Pointer<ffi.Void>, NveEventBatch)? _dispatchEvents;
  final int Function(ffi.Pointer<ffi.Void>, ffi.Pointer<NveMapEntry>, int)? _setProps;
  final NveEventBatch Function(ffi.Pointer<ffi.Void>)? _drainEmittedEvents;
  final int Function(ffi.Pointer<ffi.Void>, NveBufferSlice)? _applyHotPatch;
  final NveBufferSlice Function(ffi.Pointer<ffi.Void>)? _snapshotState;
  final void Function(ffi.Pointer<ffi.Void>, NveBufferSlice)? _restoreState;

  bool get isDisposed => _disposed;
  bool get supportsLayoutAndHitTest => _layout != ffi.nullptr;

  /// Loads `vtableSymbol` (`nve_get_interface`) from `libraryPath` (or the current process)
  /// and creates a surface.
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

  /// Pushes structured props directly via `NveComponentApi.set_props`.
  bool setPropsDirect(Map<String, Object?> props) {
    _ensureLive();
    final directFn = _setProps;
    if (directFn == null) return false;
    return NveDirectPropsWriter.withProps(
      props,
      (ffi.Pointer<NveMapEntry> entries, int count) => directFn(_handle, entries, count) != 0,
    );
  }

  /// Renders the next CommandBuffer frame and returns a copy of its bytes.
  Uint8List renderNextFrame() {
    _ensureLive();
    _frameCounter += 1;
    final slice = _render(_handle, _frameCounter);
    return _copySlice(slice);
  }

  /// Dispatches typed events directly via `NveSurfaceApi.dispatch_events`.
  bool dispatchEventsDirect(List<NveEvent> events) {
    _ensureLive();
    final directFn = _dispatchEvents;
    if (directFn == null) return false;
    return NveDirectEventWriter.withBatch(
      events,
      (NveEventBatch batch) => directFn(_handle, batch) != 0,
    );
  }

  /// Applies one binary `NVP1` Tier-1 hot-patch frame, as broadcast by `nve dev`
  /// (`NveDevApi.apply_hot_patch`).
  bool applyHotPatch(Uint8List patchNvp1) {
    _ensureLive();
    final fn = _applyHotPatch;
    if (fn == null) return false;
    return _withSlice(patchNvp1, (NveBufferSlice slice) => fn(_handle, slice) != 0);
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
    _withSlice(stateBytes, (NveBufferSlice slice) {
      fn(_handle, slice);
      return 0;
    });
  }

  /// Drains outbound events emitted by the Rust component (`NveComponentApi.drain_emitted_events`).
  List<NveEvent> drainEmittedEvents() {
    _ensureLive();
    final fn = _drainEmittedEvents;
    if (fn == null) return const <NveEvent>[];
    return NveDirectEventWriter.decodeBatch(fn(_handle));
  }

  /// Negotiates intrinsic dimensions via zero-cost `NveLayoutApi.measure`.
  Size? measure(BoxConstraints constraints) {
    _ensureLive();
    if (_layout == ffi.nullptr || _layout.ref.measure == ffi.nullptr) return null;
    final measureFn = _layout.ref.measure
        .asFunction<NveIntrinsicSize Function(ffi.Pointer<ffi.Void>, NveSizeConstraints)>();
    final cPtr = NveNativeMem.allocate<NveSizeConstraints>(ffi.sizeOf<NveSizeConstraints>());
    try {
      cPtr.ref
        ..minW = constraints.minWidth
        ..minH = constraints.minHeight
        ..maxW = constraints.maxWidth
        ..maxH = constraints.maxHeight;
      final out = measureFn(_handle, cPtr.ref);
      return Size(out.width, out.height);
    } finally {
      NveNativeMem.free(cPtr);
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
    final dataPtr = NveNativeMem.copyBytes(bytes);
    final slicePtr = NveNativeMem.allocate<NveBufferSlice>(ffi.sizeOf<NveBufferSlice>());
    try {
      final slice = NveNativeMem.sliceFor(dataPtr, bytes.length, slicePtr);
      return body(slice);
    } finally {
      NveNativeMem.free(slicePtr);
      NveNativeMem.free(dataPtr);
    }
  }

  Uint8List _copySlice(NveBufferSlice slice) {
    if (slice.ptr == ffi.nullptr || slice.len == 0) {
      return Uint8List(0);
    }
    return Uint8List.fromList(slice.ptr.asTypedList(slice.len));
  }
}
