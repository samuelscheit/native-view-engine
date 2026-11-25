import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io' show Platform;
import 'dart:typed_data';

import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

String _readStrSlice(NveStrSlice slice) {
  if (slice.ptr == ffi.nullptr || slice.len == 0) return '';
  return utf8.decode(slice.ptr.asTypedList(slice.len));
}

Object? _readNveValue(NveValue v) {
  return switch (NveValueTag.fromCode(v.tag)) {
    NveValueTag.nullValue => null,
    NveValueTag.boolValue => v.data.boolVal != 0,
    NveValueTag.i64 => v.data.i64Val,
    NveValueTag.f64 => v.data.f64Val,
    NveValueTag.str => _readStrSlice(v.data.strVal),
    NveValueTag.bytes => v.data.bytesVal.ptr == ffi.nullptr || v.data.bytesVal.len == 0
        ? Uint8List(0)
        : Uint8List.fromList(v.data.bytesVal.ptr.asTypedList(v.data.bytesVal.len)),
    NveValueTag.array => List<Object?>.generate(
        v.data.arrayVal.len,
        (int i) => _readNveValue(v.data.arrayVal.ptr[i]),
      ),
    NveValueTag.map => <String, Object?>{
        for (var i = 0; i < v.data.mapVal.len; i++)
          _readStrSlice(v.data.mapVal.ptr[i].key): _readNveValue(v.data.mapVal.ptr[i].value),
      },
    null => null,
  };
}

class _RecordedDirectEvent {
  const _RecordedDirectEvent({
    required this.surfaceId,
    required this.nodeId,
    required this.timestampUs,
    required this.kind,
    required this.payloadTag,
    required this.name,
    required this.x,
    required this.y,
    required this.width,
    required this.height,
    required this.text,
    required this.value,
  });

  final int surfaceId;
  final int nodeId;
  final int timestampUs;
  final int kind;
  final int payloadTag;
  final String name;
  final double x;
  final double y;
  final double width;
  final double height;
  final String text;
  final Object? value;
}

class _TestAlloc {
  static final ffi.DynamicLibrary _libc = Platform.isWindows
      ? ffi.DynamicLibrary.open('ucrtbase.dll')
      : ffi.DynamicLibrary.process();

  static final ffi.Pointer<ffi.Void> Function(int) _malloc = _libc.lookupFunction<
      ffi.Pointer<ffi.Void> Function(ffi.Size), ffi.Pointer<ffi.Void> Function(int)>('malloc');
  static final void Function(ffi.Pointer<ffi.Void>) _free = _libc.lookupFunction<
      ffi.Void Function(ffi.Pointer<ffi.Void>), void Function(ffi.Pointer<ffi.Void>)>('free');

  final List<ffi.Pointer<ffi.Void>> _allocations = <ffi.Pointer<ffi.Void>>[];

  ffi.Pointer<T> allocate<T extends ffi.NativeType>(int byteCount) {
    final ptr = _malloc(byteCount == 0 ? 1 : byteCount);
    if (ptr == ffi.nullptr) {
      throw StateError('malloc($byteCount) failed');
    }
    _allocations.add(ptr);
    return ptr.cast<T>();
  }

  ffi.Pointer<ffi.Uint8> copyBytes(Uint8List bytes) {
    final ptr = allocate<ffi.Uint8>(bytes.length);
    if (bytes.isNotEmpty) {
      ptr.asTypedList(bytes.length).setAll(0, bytes);
    }
    return ptr;
  }

  void dispose() {
    for (final ptr in _allocations) {
      _free(ptr);
    }
    _allocations.clear();
  }
}

class _MockNativeModule {
  _MockNativeModule({
    this.includeLayout = true,
    this.includeComponent = true,
    this.includeDev = true,
    this.includeSurface = true,
    this.includeDirectApis = true,
    this.createReturnsNull = false,
  }) {
    _scratchSlice = _alloc.allocate<NveBufferSlice>(ffi.sizeOf<NveBufferSlice>());
    _scratchSize = _alloc.allocate<NveIntrinsicSize>(ffi.sizeOf<NveIntrinsicSize>());

    _createCb = ffi.NativeCallable<NveCreateNative>.isolateLocal(_onCreate);
    _destroyCb = ffi.NativeCallable<NveDestroyNative>.isolateLocal(_onDestroy);
    _renderCb = ffi.NativeCallable<NveRenderNative>.isolateLocal(_onRender);
    _dispatchEventsCb = ffi.NativeCallable<NveDispatchEventsNative>.isolateLocal(
      _onDispatchEvents,
      exceptionalReturn: 0,
    );
    _dispatchDirectCb = ffi.NativeCallable<NveDispatchEventsDirectNative>.isolateLocal(
      _onDispatchEventsDirect,
      exceptionalReturn: 0,
    );
    _measureCb = ffi.NativeCallable<NveMeasureNative>.isolateLocal(_onMeasure);
    _hitTestCb = ffi.NativeCallable<NveHitTestNative>.isolateLocal(
      _onHitTest,
      exceptionalReturn: 0,
    );
    _setPropsCb = ffi.NativeCallable<NveSetPropsNative>.isolateLocal(
      _onSetProps,
      exceptionalReturn: 0,
    );
    _setPropsDirectCb = ffi.NativeCallable<NveSetPropsDirectNative>.isolateLocal(
      _onSetPropsDirect,
      exceptionalReturn: 0,
    );
    _drainCb = ffi.NativeCallable<NveDrainEmittedEventsNative>.isolateLocal(_onDrain);
    _hotPatchCb = ffi.NativeCallable<NveApplyHotPatchNative>.isolateLocal(
      _onHotPatch,
      exceptionalReturn: 0,
    );
    _snapshotCb = ffi.NativeCallable<NveSnapshotStateNative>.isolateLocal(_onSnapshot);
    _restoreCb = ffi.NativeCallable<NveRestoreStateNative>.isolateLocal(_onRestore);

    _surfaceApi = _alloc.allocate<NveSurfaceApi>(ffi.sizeOf<NveSurfaceApi>());
    _surfaceApi.ref
      ..create = _createCb.nativeFunction
      ..destroy = _destroyCb.nativeFunction
      ..render = _renderCb.nativeFunction
      ..dispatchEvents = _dispatchEventsCb.nativeFunction
      ..dispatchEventsDirect = includeDirectApis ? _dispatchDirectCb.nativeFunction : ffi.nullptr;

    _layoutApi = _alloc.allocate<NveLayoutApi>(ffi.sizeOf<NveLayoutApi>());
    _layoutApi.ref
      ..measure = _measureCb.nativeFunction
      ..hitTest = _hitTestCb.nativeFunction;

    _componentApi = _alloc.allocate<NveComponentApi>(ffi.sizeOf<NveComponentApi>());
    _componentApi.ref
      ..setProps = _setPropsCb.nativeFunction
      ..drainEmittedEvents = _drainCb.nativeFunction
      ..setPropsDirect = includeDirectApis ? _setPropsDirectCb.nativeFunction : ffi.nullptr;

    _devApi = _alloc.allocate<NveDevApi>(ffi.sizeOf<NveDevApi>());
    _devApi.ref
      ..applyHotPatch = _hotPatchCb.nativeFunction
      ..snapshotState = _snapshotCb.nativeFunction
      ..restoreState = _restoreCb.nativeFunction;
  }

  final bool includeSurface;
  final bool includeLayout;
  final bool includeComponent;
  final bool includeDev;
  final bool includeDirectApis;
  final bool createReturnsNull;

  final _TestAlloc _alloc = _TestAlloc();
  late final ffi.Pointer<NveBufferSlice> _scratchSlice;
  late final ffi.Pointer<NveIntrinsicSize> _scratchSize;
  late final ffi.Pointer<NveSurfaceApi> _surfaceApi;
  late final ffi.Pointer<NveLayoutApi> _layoutApi;
  late final ffi.Pointer<NveComponentApi> _componentApi;
  late final ffi.Pointer<NveDevApi> _devApi;

  late final ffi.NativeCallable<NveCreateNative> _createCb;
  late final ffi.NativeCallable<NveDestroyNative> _destroyCb;
  late final ffi.NativeCallable<NveRenderNative> _renderCb;
  late final ffi.NativeCallable<NveDispatchEventsNative> _dispatchEventsCb;
  late final ffi.NativeCallable<NveDispatchEventsDirectNative> _dispatchDirectCb;
  late final ffi.NativeCallable<NveMeasureNative> _measureCb;
  late final ffi.NativeCallable<NveHitTestNative> _hitTestCb;
  late final ffi.NativeCallable<NveSetPropsNative> _setPropsCb;
  late final ffi.NativeCallable<NveSetPropsDirectNative> _setPropsDirectCb;
  late final ffi.NativeCallable<NveDrainEmittedEventsNative> _drainCb;
  late final ffi.NativeCallable<NveApplyHotPatchNative> _hotPatchCb;
  late final ffi.NativeCallable<NveSnapshotStateNative> _snapshotCb;
  late final ffi.NativeCallable<NveRestoreStateNative> _restoreCb;

  int? createdSurfaceId;
  int destroyCount = 0;
  final List<int> renderedFrameIds = <int>[];
  Uint8List nextRenderBytes = Uint8List.fromList(<int>[0x4E, 0x56, 0x45, 0x31]);
  final List<Uint8List> dispatchedEvents = <Uint8List>[];
  final List<List<_RecordedDirectEvent>> dispatchedDirectBatches = <List<_RecordedDirectEvent>>[];
  int dispatchReturn = 1;
  final List<Uint8List> propsBinHistory = <Uint8List>[];
  final List<Map<String, Object?>> propsDirectHistory = <Map<String, Object?>>[];
  int setPropsReturn = 1;
  Uint8List outboundEvents = Uint8List(0);
  final List<Uint8List> hotPatches = <Uint8List>[];
  int hotPatchReturn = 1;
  Uint8List savedState = Uint8List.fromList(utf8.encode('state-v1'));
  Uint8List? restoredState;

  double? lastMinW;
  double? lastMinH;
  double? lastMaxW;
  double? lastMaxH;
  Size nextMeasuredSize = const Size(240, 120);

  double? lastHitX;
  double? lastHitY;
  int? lastHitPhase;
  int nextHitDisposition = NveGestureDisposition.claim.code;

  ffi.Pointer<ffi.Void> getInterface(int interfaceId) {
    return switch (interfaceId) {
      nveIfaceSurface when includeSurface => _surfaceApi.cast<ffi.Void>(),
      nveIfaceLayout when includeLayout => _layoutApi.cast<ffi.Void>(),
      nveIfaceComponent when includeComponent => _componentApi.cast<ffi.Void>(),
      nveIfaceDev when includeDev => _devApi.cast<ffi.Void>(),
      _ => ffi.nullptr,
    };
  }

  ffi.Pointer<ffi.Void> _onCreate(int surfaceId) {
    createdSurfaceId = surfaceId;
    if (createReturnsNull) return ffi.nullptr;
    return ffi.Pointer<ffi.Void>.fromAddress(0x2000 + surfaceId);
  }

  void _onDestroy(ffi.Pointer<ffi.Void> handle) {
    destroyCount += 1;
  }

  NveBufferSlice _onRender(ffi.Pointer<ffi.Void> handle, int frameId) {
    renderedFrameIds.add(frameId);
    if (nextRenderBytes.isEmpty) {
      _scratchSlice.ref
        ..ptr = ffi.nullptr
        ..len = 0;
      return _scratchSlice.ref;
    }
    final buf = _alloc.copyBytes(nextRenderBytes);
    _scratchSlice.ref
      ..ptr = buf
      ..len = nextRenderBytes.length;
    return _scratchSlice.ref;
  }

  int _onDispatchEvents(ffi.Pointer<ffi.Void> handle, NveBufferSlice slice) {
    final copied = slice.ptr == ffi.nullptr || slice.len == 0
        ? Uint8List(0)
        : Uint8List.fromList(slice.ptr.asTypedList(slice.len));
    dispatchedEvents.add(copied);
    return dispatchReturn;
  }

  int _onDispatchEventsDirect(ffi.Pointer<ffi.Void> handle, NveEventBatch batch) {
    final captured = <_RecordedDirectEvent>[];
    if (batch.ptr != ffi.nullptr && batch.len > 0) {
      for (var i = 0; i < batch.len; i++) {
        final ev = batch.ptr[i];
        captured.add(
          _RecordedDirectEvent(
            surfaceId: ev.surfaceId,
            nodeId: ev.nodeId,
            timestampUs: ev.timestampUs,
            kind: ev.kind,
            payloadTag: ev.payloadTag,
            name: _readStrSlice(ev.name),
            x: ev.x,
            y: ev.y,
            width: ev.width,
            height: ev.height,
            text: _readStrSlice(ev.text),
            value: _readNveValue(ev.value),
          ),
        );
      }
    }
    dispatchedDirectBatches.add(captured);
    return dispatchReturn;
  }

  NveIntrinsicSize _onMeasure(ffi.Pointer<ffi.Void> handle, NveSizeConstraints c) {
    lastMinW = c.minW;
    lastMinH = c.minH;
    lastMaxW = c.maxW;
    lastMaxH = c.maxH;
    _scratchSize.ref
      ..width = nextMeasuredSize.width
      ..height = nextMeasuredSize.height;
    return _scratchSize.ref;
  }

  int _onHitTest(ffi.Pointer<ffi.Void> handle, double x, double y, int phase) {
    lastHitX = x;
    lastHitY = y;
    lastHitPhase = phase;
    return nextHitDisposition;
  }

  int _onSetProps(ffi.Pointer<ffi.Void> handle, NveBufferSlice slice) {
    final bytes = slice.ptr == ffi.nullptr || slice.len == 0
        ? Uint8List(0)
        : Uint8List.fromList(slice.ptr.asTypedList(slice.len));
    propsBinHistory.add(bytes);
    return setPropsReturn;
  }

  int _onSetPropsDirect(
    ffi.Pointer<ffi.Void> handle,
    ffi.Pointer<NveMapEntry> entries,
    int count,
  ) {
    final captured = <String, Object?>{};
    if (entries != ffi.nullptr && count > 0) {
      for (var i = 0; i < count; i++) {
        captured[_readStrSlice(entries[i].key)] = _readNveValue(entries[i].value);
      }
    }
    propsDirectHistory.add(captured);
    return setPropsReturn;
  }

  NveBufferSlice _onDrain(ffi.Pointer<ffi.Void> handle) {
    final bytes = outboundEvents;
    outboundEvents = Uint8List(0);
    if (bytes.isEmpty) {
      _scratchSlice.ref
        ..ptr = ffi.nullptr
        ..len = 0;
      return _scratchSlice.ref;
    }
    final buf = _alloc.copyBytes(bytes);
    _scratchSlice.ref
      ..ptr = buf
      ..len = bytes.length;
    return _scratchSlice.ref;
  }

  int _onHotPatch(ffi.Pointer<ffi.Void> handle, NveBufferSlice slice) {
    final bytes = slice.ptr == ffi.nullptr || slice.len == 0
        ? Uint8List(0)
        : Uint8List.fromList(slice.ptr.asTypedList(slice.len));
    hotPatches.add(bytes);
    return hotPatchReturn;
  }

  NveBufferSlice _onSnapshot(ffi.Pointer<ffi.Void> handle) {
    if (savedState.isEmpty) {
      _scratchSlice.ref
        ..ptr = ffi.nullptr
        ..len = 0;
      return _scratchSlice.ref;
    }
    final buf = _alloc.copyBytes(savedState);
    _scratchSlice.ref
      ..ptr = buf
      ..len = savedState.length;
    return _scratchSlice.ref;
  }

  void _onRestore(ffi.Pointer<ffi.Void> handle, NveBufferSlice slice) {
    restoredState = slice.ptr == ffi.nullptr || slice.len == 0
        ? Uint8List(0)
        : Uint8List.fromList(slice.ptr.asTypedList(slice.len));
  }

  void close() {
    _createCb.close();
    _destroyCb.close();
    _renderCb.close();
    _dispatchEventsCb.close();
    _dispatchDirectCb.close();
    _measureCb.close();
    _hitTestCb.close();
    _setPropsCb.close();
    _setPropsDirectCb.close();
    _drainCb.close();
    _hotPatchCb.close();
    _snapshotCb.close();
    _restoreCb.close();
    _alloc.dispose();
  }
}

/// A complete binary `NVP1` `DevStatus` frame ("x", 0 ms), as `nve dev` broadcasts it.
final Uint8List _nvp1DevStatus = Uint8List.fromList(<int>[
  0x4E, 0x56, 0x50, 0x31, 3, //
  1, 0, 0, 0, 0x78, //
  0, 0, 0, 0, 0, 0, 0, 0,
]);

void main() {
  group('NveAppInstance FFI contracts', () {
    test('full vtable lifecycle, direct props & events, rendering, dev, layout, hitTest', () {
      final mock = _MockNativeModule();
      addTearDown(mock.close);

      final instance = NveAppInstance.fromInterfaceResolver(
        surfaceId: 42,
        getInterface: mock.getInterface,
      );
      expect(instance.surfaceId, 42);
      expect(instance.isDisposed, isFalse);
      expect(instance.supportsLayoutAndHitTest, isTrue);
      expect(instance.supportsDirectProps, isTrue);
      expect(instance.supportsDirectEvents, isTrue);
      expect(mock.createdSurfaceId, 42);

      // setPropsDirect with all types including nested arrays/maps & bytes
      mock.setPropsReturn = 1;
      final blob = Uint8List.fromList(<int>[0xDE, 0xAD, 0xBE, 0xEF]);
      expect(
        instance.setPropsDirect(<String, Object?>{
          'nullVal': null,
          'flag': true,
          'count': 5,
          'ratio': 2.5,
          'title': 'Hi 🚀',
          'blob': blob,
          'nested': <Object?>[
            1,
            <String, Object?>{'inner': 'ok'},
          ],
        }),
        isTrue,
      );
      expect(mock.propsBinHistory, isEmpty);
      expect(mock.propsDirectHistory, hasLength(1));
      expect(mock.propsDirectHistory.single['nullVal'], isNull);
      expect(mock.propsDirectHistory.single['flag'], isTrue);
      expect(mock.propsDirectHistory.single['count'], 5);
      expect(mock.propsDirectHistory.single['ratio'], closeTo(2.5, 1e-9));
      expect(mock.propsDirectHistory.single['title'], 'Hi 🚀');
      expect(mock.propsDirectHistory.single['blob'], blob);
      expect(mock.propsDirectHistory.single['nested'], <Object?>[
        1,
        <String, Object?>{'inner': 'ok'},
      ]);

      mock.setPropsReturn = 0;
      expect(instance.setPropsDirect(<String, Object?>{'count': 5}), isFalse);

      // renderNextFrame increments frame ID and handles empty slice
      expect(instance.renderNextFrame(), Uint8List.fromList(<int>[0x4E, 0x56, 0x45, 0x31]));
      mock.nextRenderBytes = Uint8List(0);
      expect(instance.renderNextFrame(), isEmpty);
      expect(mock.renderedFrameIds, <int>[1, 2]);

      // dispatchEventsDirect roundtrips all event kinds
      mock.dispatchReturn = 1;
      expect(
        instance.dispatchEventsDirect(<NveEvent>[
          const NveEvent(
            surfaceId: 42,
            nodeId: 2,
            timestampUs: 100,
            kind: NveEventKind.press,
            payload: NvePointPayload(x: 10.5, y: 20.25),
          ),
          const NveEvent(
            surfaceId: 42,
            nodeId: 3,
            timestampUs: 200,
            kind: NveEventKind.scroll,
            payload: NveScrollPayload(x: 0.0, y: 80.0, contentWidth: 320.0, contentHeight: 900.0),
          ),
          NveEvent.custom(
            surfaceId: 42,
            nodeId: 1,
            timestampUs: 500,
            name: 'reset',
            value: <String, Object?>{'hard': true},
          ),
        ]),
        isTrue,
      );
      expect(mock.dispatchedEvents, isEmpty);
      expect(mock.dispatchedDirectBatches, hasLength(1));
      final batch = mock.dispatchedDirectBatches.single;
      expect(batch, hasLength(3));
      expect(batch[0].kind, NveEventKind.press.code);
      expect(batch[0].x, closeTo(10.5, 1e-3));
      expect(batch[0].y, closeTo(20.25, 1e-3));
      expect(batch[1].kind, NveEventKind.scroll.code);
      expect(batch[1].y, closeTo(80.0, 1e-3));
      expect(batch[1].height, closeTo(900.0, 1e-3));
      expect(batch[2].name, 'reset');
      expect(batch[2].value, <String, Object?>{'hard': true});

      // drainEmittedEvents
      mock.outboundEvents = NveEventRingCodec.encodeEvents(<NveEvent>[
        NveEvent.custom(
          surfaceId: 42,
          nodeId: 3,
          timestampUs: 999,
          name: 'onIncrement',
          value: <String, Object?>{'count': 6},
        ),
      ]);
      final drained = instance.drainEmittedEvents();
      expect(drained, hasLength(1));
      expect(drained.single.name, 'onIncrement');
      expect(drained.single.parseBinValue()?.get('count')?.asInt64(), 6);
      expect(instance.drainEmittedEvents(), isEmpty);

      // applyHotPatch, snapshotState, restoreState
      mock.hotPatchReturn = 1;
      expect(instance.applyHotPatch(_nvp1DevStatus), isTrue);
      expect(mock.hotPatches, hasLength(1));
      expect(mock.hotPatches.single, _nvp1DevStatus, reason: 'NVP1 bytes forwarded unchanged');

      final snap = instance.snapshotState();
      expect(utf8.decode(snap), 'state-v1');
      final nextState = Uint8List.fromList(utf8.encode('state-v2'));
      instance.restoreState(nextState);
      expect(mock.restoredState, nextState);

      // measure
      mock.nextMeasuredSize = const Size(192.5, 84.0);
      final measured = instance.measure(
        const BoxConstraints(minWidth: 20, minHeight: 10, maxWidth: 400, maxHeight: double.infinity),
      );
      expect(measured, isNotNull);
      expect(measured!.width, closeTo(192.5, 0.001));
      expect(measured.height, closeTo(84.0, 0.001));
      expect(mock.lastMinW, closeTo(20, 0.001));
      expect(mock.lastMinH, closeTo(10, 0.001));
      expect(mock.lastMaxW, closeTo(400, 0.001));
      expect(mock.lastMaxH, double.infinity);

      // hitTest
      mock.nextHitDisposition = NveGestureDisposition.deferToParent.code;
      expect(
        instance.hitTest(const Offset(12.5, 34.25), pointerPhase: 2),
        NveGestureDisposition.deferToParent,
      );
      expect(mock.lastHitX, closeTo(12.5, 0.001));
      expect(mock.lastHitY, closeTo(34.25, 0.001));
      expect(mock.lastHitPhase, 2);

      // dispose is idempotent
      instance.dispose();
      instance.dispose();
      expect(instance.isDisposed, isTrue);
      expect(mock.destroyCount, 1);

      // Post-disposal calls throw StateError
      expect(() => instance.setPropsDirect(<String, Object?>{}), throwsStateError);
      expect(instance.renderNextFrame, throwsStateError);
      expect(() => instance.dispatchEvents(Uint8List(0)), throwsStateError);
      expect(() => instance.dispatchEventsDirect(const <NveEvent>[]), throwsStateError);
      expect(() => instance.applyHotPatch(_nvp1DevStatus), throwsStateError);
      expect(instance.snapshotState, throwsStateError);
      expect(() => instance.restoreState(Uint8List(0)), throwsStateError);
      expect(instance.drainEmittedEvents, throwsStateError);
      expect(() => instance.measure(const BoxConstraints()), throwsStateError);
      expect(() => instance.hitTest(Offset.zero), throwsStateError);
    });

    test('falls back to binary NveBinValue set_props and EventRing dispatch_events when direct pointers are null', () {
      final mock = _MockNativeModule(includeDirectApis: false);
      addTearDown(mock.close);

      final instance = NveAppInstance.fromInterfaceResolver(
        surfaceId: 7,
        getInterface: mock.getInterface,
      );
      addTearDown(instance.dispose);

      expect(instance.supportsDirectProps, isFalse);
      expect(instance.supportsDirectEvents, isFalse);

      expect(instance.setPropsDirect(<String, Object?>{'count': 9, 'mode': 'bin'}), isTrue);
      expect(mock.propsDirectHistory, isEmpty);
      expect(mock.propsBinHistory, hasLength(1));
      final view = NveBinValueView.parse(mock.propsBinHistory.single);
      expect(view.get('count')?.asInt64(), 9);
      expect(view.get('mode')?.asString(), 'bin');

      expect(
        instance.dispatchEventsDirect(<NveEvent>[
          NveEvent.custom(
            surfaceId: 7,
            nodeId: 2,
            timestampUs: 123,
            name: 'ping',
            value: <String, Object?>{'ok': true},
          ),
        ]),
        isTrue,
      );
      expect(mock.dispatchedDirectBatches, isEmpty);
      expect(mock.dispatchedEvents, hasLength(1));
      final decoded = NveEventRingCodec.decode(mock.dispatchedEvents.single);
      expect(decoded, hasLength(1));
      expect(decoded.single.name, 'ping');
      expect(decoded.single.parseBinValue()?.get('ok')?.asBool(), isTrue);
    });

    test('optional interfaces (layout, component, dev) degrade cleanly when NULL', () {
      final mock = _MockNativeModule(
        includeLayout: false,
        includeComponent: false,
        includeDev: false,
      );
      addTearDown(mock.close);

      final instance = NveAppInstance.fromInterfaceResolver(
        surfaceId: 9,
        getInterface: mock.getInterface,
      );
      addTearDown(instance.dispose);

      expect(instance.supportsLayoutAndHitTest, isFalse);
      expect(instance.setPropsDirect(<String, Object?>{'a': 1}), isFalse);
      expect(instance.drainEmittedEvents(), isEmpty);
      expect(instance.applyHotPatch(_nvp1DevStatus), isFalse);
      expect(instance.snapshotState(), isEmpty);
      instance.restoreState(Uint8List.fromList(<int>[1, 2]));
      expect(instance.measure(const BoxConstraints(maxWidth: 300, maxHeight: 200)), isNull);
      expect(instance.hitTest(const Offset(5, 5)), NveGestureDisposition.claim);
    });

    test('throws StateError when NVE_IFACE_SURFACE is missing or create returns NULL', () {
      final missingSurface = _MockNativeModule(includeSurface: false);
      addTearDown(missingSurface.close);
      expect(
        () => NveAppInstance.fromInterfaceResolver(
          surfaceId: 1,
          getInterface: missingSurface.getInterface,
        ),
        throwsStateError,
      );

      final nullCreate = _MockNativeModule(createReturnsNull: true);
      addTearDown(nullCreate.close);
      expect(
        () => NveAppInstance.fromInterfaceResolver(
          surfaceId: 2,
          getInterface: nullCreate.getInterface,
        ),
        throwsStateError,
      );
    });

    test('NveAppInstance.open rejects missing dynamic library or symbol', () {
      expect(
        () => NveAppInstance.open(
          surfaceId: 1,
          libraryPath: '/nonexistent/libnve_missing.dylib',
        ),
        throwsArgumentError,
      );
      expect(
        () => NveAppInstance.open(
          surfaceId: 1,
          vtableSymbol: 'nve_nonexistent_symbol_xyz',
        ),
        throwsArgumentError,
      );
    });
  });
}
