import 'dart:convert' show utf8;
import 'dart:ffi' as ffi;
import 'dart:io' show Platform;

import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

class _DirectFfiTextureMock {
  _DirectFfiTextureMock() {
    _calloc = _libc.lookupFunction<
        ffi.Pointer<ffi.Void> Function(ffi.Size, ffi.Size),
        ffi.Pointer<ffi.Void> Function(int, int)>('calloc');
    _free = _libc.lookupFunction<
        ffi.Void Function(ffi.Pointer<ffi.Void>),
        void Function(ffi.Pointer<ffi.Void>)>('free');

    _emptySlice = _alloc<NveBufferSlice>(ffi.sizeOf<NveBufferSlice>());
    _outboundSlice = _alloc<NveBufferSlice>(ffi.sizeOf<NveBufferSlice>());

    _createCb = ffi.NativeCallable<NveCreateNative>.isolateLocal(
      (int sid) => ffi.Pointer<ffi.Void>.fromAddress(0x7000 + sid),
    );
    _destroyCb = ffi.NativeCallable<NveDestroyNative>.isolateLocal((ffi.Pointer<ffi.Void> _) {});
    _renderCb = ffi.NativeCallable<NveRenderNative>.isolateLocal(_onRender);
    _dispatchRingCb = ffi.NativeCallable<NveDispatchEventsNative>.isolateLocal(
      (ffi.Pointer<ffi.Void> _, NveBufferSlice __) => 0,
      exceptionalReturn: 0,
    );
    _dispatchDirectCb = ffi.NativeCallable<NveDispatchEventsDirectNative>.isolateLocal(
      _onDispatchDirect,
      exceptionalReturn: 0,
    );
    _setPropsDirectCb = ffi.NativeCallable<NveSetPropsDirectNative>.isolateLocal(
      _onSetPropsDirect,
      exceptionalReturn: 0,
    );
    _drainEventsCb = ffi.NativeCallable<NveDrainEmittedEventsNative>.isolateLocal(_onDrainEvents);

    _surfaceApi = _alloc<NveSurfaceApi>(ffi.sizeOf<NveSurfaceApi>());
    _surfaceApi.ref
      ..create = _createCb.nativeFunction
      ..destroy = _destroyCb.nativeFunction
      ..render = _renderCb.nativeFunction
      ..dispatchEvents = _dispatchRingCb.nativeFunction
      ..dispatchEventsDirect = _dispatchDirectCb.nativeFunction;

    _componentApi = _alloc<NveComponentApi>(ffi.sizeOf<NveComponentApi>());
    _componentApi.ref
      ..setProps = ffi.nullptr
      ..drainEmittedEvents = _drainEventsCb.nativeFunction
      ..setPropsDirect = _setPropsDirectCb.nativeFunction;
  }

  static final ffi.DynamicLibrary _libc = Platform.isWindows
      ? ffi.DynamicLibrary.open('ucrtbase.dll')
      : ffi.DynamicLibrary.process();

  late final ffi.Pointer<ffi.Void> Function(int, int) _calloc;
  late final void Function(ffi.Pointer<ffi.Void>) _free;
  final List<ffi.Pointer<ffi.Void>> _allocs = <ffi.Pointer<ffi.Void>>[];

  late final ffi.Pointer<NveBufferSlice> _emptySlice;
  late final ffi.Pointer<NveBufferSlice> _outboundSlice;
  late final ffi.Pointer<NveSurfaceApi> _surfaceApi;
  late final ffi.Pointer<NveComponentApi> _componentApi;

  late final ffi.NativeCallable<NveCreateNative> _createCb;
  late final ffi.NativeCallable<NveDestroyNative> _destroyCb;
  late final ffi.NativeCallable<NveRenderNative> _renderCb;
  late final ffi.NativeCallable<NveDispatchEventsNative> _dispatchRingCb;
  late final ffi.NativeCallable<NveDispatchEventsDirectNative> _dispatchDirectCb;
  late final ffi.NativeCallable<NveSetPropsDirectNative> _setPropsDirectCb;
  late final ffi.NativeCallable<NveDrainEmittedEventsNative> _drainEventsCb;

  int renderCount = 0;
  int directPropsCount = 0;
  int lastDirectCountProp = -1;
  final List<String> directCommands = <String>[];
  final List<NveEvent> _pendingOutbound = <NveEvent>[];

  ffi.Pointer<T> _alloc<T extends ffi.NativeType>(int bytes) {
    final p = _calloc(1, bytes <= 0 ? 1 : bytes);
    _allocs.add(p);
    return p.cast<T>();
  }

  ffi.Pointer<ffi.Void> getInterface(int ifaceId) {
    if (ifaceId == nveIfaceSurface) return _surfaceApi.cast<ffi.Void>();
    if (ifaceId == nveIfaceComponent) return _componentApi.cast<ffi.Void>();
    return ffi.nullptr;
  }

  NveBufferSlice _onRender(ffi.Pointer<ffi.Void> _, int frameId) {
    renderCount += 1;
    return _emptySlice.ref;
  }

  int _onSetPropsDirect(
    ffi.Pointer<ffi.Void> _,
    ffi.Pointer<NveMapEntry> entries,
    int count,
  ) {
    directPropsCount += 1;
    for (var i = 0; i < count; i++) {
      final entry = entries[i];
      final key = entry.key.len == 0
          ? ''
          : utf8.decode(entry.key.ptr.cast<ffi.Uint8>().asTypedList(entry.key.len));
      if (key == 'count' && entry.value.tag == NveValueTag.i64.code) {
        lastDirectCountProp = entry.value.data.i64Val;
      }
    }
    _pendingOutbound.add(
      NveEvent.custom(
        surfaceId: 3100,
        nodeId: 1,
        name: 'onDirectProps',
        value: <String, Object?>{'count': lastDirectCountProp},
      ),
    );
    return 1;
  }

  int _onDispatchDirect(ffi.Pointer<ffi.Void> _, NveEventBatch batch) {
    for (var i = 0; i < batch.len; i++) {
      final ev = batch.ptr[i];
      if (ev.name.len > 0) {
        final name = utf8.decode(ev.name.ptr.cast<ffi.Uint8>().asTypedList(ev.name.len));
        directCommands.add(name);
      }
    }
    return 1;
  }

  NveBufferSlice _onDrainEvents(ffi.Pointer<ffi.Void> _) {
    if (_pendingOutbound.isEmpty) return _emptySlice.ref;
    final bytes = NveEventRingCodec.encodeEvents(_pendingOutbound);
    _pendingOutbound.clear();
    final ptr = _alloc<ffi.Uint8>(bytes.length);
    ptr.asTypedList(bytes.length).setAll(0, bytes);
    _outboundSlice.ref
      ..ptr = ptr
      ..len = bytes.length;
    return _outboundSlice.ref;
  }

  void dispose() {
    _createCb.close();
    _destroyCb.close();
    _renderCb.close();
    _dispatchRingCb.close();
    _dispatchDirectCb.close();
    _setPropsDirectCb.close();
    _drainEventsCb.close();
    for (final p in _allocs) {
      _free(p);
    }
    _allocs.clear();
  }
}

void main() {
  const textureChannel = MethodChannel('dev.nve/flutter_texture');
  const eventChannelName = 'dev.nve/flutter_events';

  group('NveSurfaceController detached behavior', () {
    test('isAttached is false and calls are safe no-ops when detached', () {
      final controller = NveSurfaceController();
      expect(controller.isAttached, isFalse);
      controller.dispatchCommand('reset', <String, Object?>{'force': true});
      controller.requestRender();
      expect(controller.snapshotState(), isNull);
    });
  });

  group('NveTextureHostWidget', () {
    testWidgets('mounts cleanly without native plugin handlers (headless fallback)', (
      WidgetTester tester,
    ) async {
      final controller = NveSurfaceController();

      await tester.pumpWidget(
        Directionality(
          textDirection: TextDirection.ltr,
          child: NveTextureHostWidget(
            componentName: 'NveCounterCard',
            surfaceId: 2001,
            width: 240,
            height: 120,
            controller: controller,
            props: const <String, Object?>{'count': 1},
          ),
        ),
      );
      await tester.pump();

      expect(controller.isAttached, isTrue);
      expect(find.byType(Texture), findsNothing);
      expect(find.byType(NveTextureHostWidget), findsOneWidget);

      await tester.pumpWidget(const SizedBox.shrink());
      expect(controller.isAttached, isFalse);
    });

    testWidgets(
      'full texture lifecycle: create, resize, binary props diff, commands, pointers, events, dispose',
      (WidgetTester tester) async {
        final messenger = TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
        final calls = <MethodCall>[];

        messenger.setMockMethodCallHandler(textureChannel, (MethodCall call) async {
          calls.add(call);
          if (call.method == 'createTextureSurface') {
            return 42;
          }
          return null;
        });
        messenger.setMockMethodCallHandler(
          const MethodChannel(eventChannelName),
          (MethodCall call) async => null,
        );
        addTearDown(() {
          messenger.setMockMethodCallHandler(textureChannel, null);
          messenger.setMockMethodCallHandler(const MethodChannel(eventChannelName), null);
        });

        final controller1 = NveSurfaceController();
        final controller2 = NveSurfaceController();
        final receivedEvents = <NveEvent>[];

        Widget buildHost({
          required Map<String, Object?> props,
          required NveSurfaceController controller,
          double width = 300,
          double height = 160,
        }) {
          return MediaQuery(
            data: const MediaQueryData(devicePixelRatio: 2.0),
            child: Directionality(
              textDirection: TextDirection.ltr,
              child: Align(
                alignment: Alignment.topLeft,
                child: NveTextureHostWidget(
                  componentName: 'NveCounterCard',
                  surfaceId: 3001,
                  vtableSymbol: 'nve_get_interface',
                  width: width,
                  height: height,
                  props: props,
                  controller: controller,
                  onEvent: receivedEvents.add,
                ),
              ),
            ),
          );
        }

        await tester.pumpWidget(
          buildHost(props: const <String, Object?>{'count': 1}, controller: controller1),
        );
        await tester.pump();

        final createCall = calls.firstWhere((c) => c.method == 'createTextureSurface');
        final createArgs = createCall.arguments as Map<Object?, Object?>;
        expect(createArgs['surfaceId'], 3001);
        expect(createArgs['componentName'], 'NveCounterCard');
        expect(createArgs['vtableSymbol'], 'nve_get_interface');
        expect(createArgs['libraryPath'], isNull);
        expect(
          NveBinValueView.parse(createArgs['propsBin']! as Uint8List).get('count')?.asInt64(),
          1,
        );

        final textureFinder = find.byType(Texture);
        expect(textureFinder, findsOneWidget);
        expect(tester.widget<Texture>(textureFinder).textureId, 42);

        final resizeCalls = calls.where((c) => c.method == 'resizeSurface').toList();
        expect(resizeCalls, hasLength(1));
        expect(resizeCalls.single.arguments, <String, dynamic>{
          'surfaceId': 3001,
          'width': 300.0,
          'height': 160.0,
          'scaleFactor': 2.0,
        });

        await tester.pumpWidget(
          buildHost(props: const <String, Object?>{'count': 1}, controller: controller1),
        );
        expect(calls.where((c) => c.method == 'setProps'), isEmpty);
        expect(calls.where((c) => c.method == 'resizeSurface'), hasLength(1));

        await tester.pumpWidget(
          buildHost(props: const <String, Object?>{'count': 2}, controller: controller2),
        );
        expect(controller1.isAttached, isFalse);
        expect(controller2.isAttached, isTrue);

        final setPropsCall = calls.where((c) => c.method == 'setProps').single;
        final setPropsArgs = setPropsCall.arguments as Map<Object?, Object?>;
        expect(setPropsArgs['surfaceId'], 3001);
        expect(
          NveBinValueView.parse(setPropsArgs['propsBin']! as Uint8List).get('count')?.asInt64(),
          2,
        );

        controller2.dispatchCommand('reset', <String, Object?>{'animated': false});
        controller2.requestRender();
        expect(controller2.snapshotState(), isNull);

        final cmdCall = calls.where((c) => c.method == 'dispatchCommand').single;
        final cmdArgs = cmdCall.arguments as Map<Object?, Object?>;
        expect(cmdArgs['surfaceId'], 3001);
        expect(cmdArgs['command'], 'reset');
        expect(
          NveBinValueView.parse(cmdArgs['argsBin']! as Uint8List).get('animated')?.asBool(),
          isFalse,
        );
        expect(
          calls.where((c) => c.method == 'requestRender').single.arguments,
          <String, dynamic>{'surfaceId': 3001},
        );

        final gesture = await tester.startGesture(const Offset(25, 40));
        await gesture.moveBy(const Offset(10, 5));
        await gesture.up();
        await tester.pump();

        final pointerPhases = calls
            .where((c) => c.method == 'pointerEvent')
            .map((c) => (c.arguments as Map<Object?, Object?>)['phase'])
            .toList();
        expect(pointerPhases, containsAllInOrder(<int>[1, 2, 3]));

        const codec = StandardMethodCodec();
        await messenger.handlePlatformMessage(
          eventChannelName,
          codec.encodeSuccessEnvelope(<String, dynamic>{
            'surfaceId': 3001,
            'nodeId': 5,
            'timestampUs': 12345,
            'kind': NveEventKind.custom.code,
            'name': 'onIncrement',
            'payload': <String, dynamic>{'count': 3},
          }),
          (_) {},
        );
        await tester.pump();

        expect(receivedEvents, hasLength(1));
        expect(receivedEvents.single.name, 'onIncrement');
        expect(receivedEvents.single.parseBinValue()?.get('count')?.asInt64(), 3);

        await tester.pumpWidget(const SizedBox.shrink());
        expect(controller2.isAttached, isFalse);
        final destroyCall = calls.where((c) => c.method == 'destroyTextureSurface').single;
        expect(destroyCall.arguments, <String, dynamic>{
          'surfaceId': 3001,
          'textureId': 42,
        });
      },
    );

    testWidgets(
      'direct FFI handles setProps, dispatchCommand, and requestRender with zero redundant MethodChannel calls',
      (WidgetTester tester) async {
        final messenger = TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
        final calls = <MethodCall>[];
        final mock = _DirectFfiTextureMock();

        messenger.setMockMethodCallHandler(textureChannel, (MethodCall call) async {
          calls.add(call);
          if (call.method == 'createTextureSurface') {
            return 88;
          }
          return null;
        });
        messenger.setMockMethodCallHandler(
          const MethodChannel(eventChannelName),
          (MethodCall call) async => null,
        );
        addTearDown(() {
          messenger.setMockMethodCallHandler(textureChannel, null);
          messenger.setMockMethodCallHandler(const MethodChannel(eventChannelName), null);
          mock.dispose();
        });

        final controller = NveSurfaceController();
        final receivedEvents = <NveEvent>[];

        Widget buildDirectHost(Map<String, Object?> props) {
          return Directionality(
            textDirection: TextDirection.ltr,
            child: NveTextureHostWidget(
              componentName: 'NveDirectCard',
              surfaceId: 3100,
              interfaceResolver: mock.getInterface,
              width: 240,
              height: 120,
              props: props,
              controller: controller,
              onEvent: receivedEvents.add,
            ),
          );
        }

        await tester.pumpWidget(buildDirectHost(const <String, Object?>{'count': 1}));
        await tester.pump();

        expect(mock.directPropsCount, 1);
        expect(mock.lastDirectCountProp, 1);
        expect(mock.renderCount, 1);
        expect(receivedEvents, hasLength(1));
        expect(receivedEvents.single.name, 'onDirectProps');

        // Push updated props, dispatch a command, and request an explicit render.
        await tester.pumpWidget(buildDirectHost(const <String, Object?>{'count': 7}));
        controller.dispatchCommand('refresh', const <String, Object?>{'force': true});
        controller.requestRender();
        await tester.pump();

        expect(mock.directPropsCount, 2);
        expect(mock.lastDirectCountProp, 7);
        expect(mock.directCommands, <String>['refresh']);
        // 1 initial + 1 props update + 1 command + 1 explicit requestRender = 4
        expect(mock.renderCount, 4);
        expect(receivedEvents, hasLength(2));
        expect(receivedEvents.last.parseBinValue()?.get('count')?.asInt64(), 7);

        // Zero redundant MethodChannel invocations when direct FFI handled the operations.
        expect(calls.where((c) => c.method == 'setProps'), isEmpty);
        expect(calls.where((c) => c.method == 'dispatchCommand'), isEmpty);
        expect(calls.where((c) => c.method == 'requestRender'), isEmpty);

        await tester.pumpWidget(const SizedBox.shrink());
      },
    );
  });

  group('NvePlatformViewWidget', () {
    testWidgets(
      'falls back to NveTextureHostWidget on desktop platforms',
      variant: TargetPlatformVariant.only(TargetPlatform.macOS),
      (WidgetTester tester) async {
        await tester.pumpWidget(
          const Directionality(
            textDirection: TextDirection.ltr,
            child: NvePlatformViewWidget(
              componentName: 'NveDesktopCard',
              surfaceId: 4001,
              width: 280,
              height: 140,
            ),
          ),
        );
        await tester.pump();

        expect(find.byType(NveTextureHostWidget), findsOneWidget);
        expect(find.byType(UiKitView), findsNothing);
        expect(find.byType(AndroidView), findsNothing);
      },
    );

    testWidgets(
      'renders UiKitView on iOS and bridges view channel commands, binary props, and events',
      variant: TargetPlatformVariant.only(TargetPlatform.iOS),
      (WidgetTester tester) async {
        final messenger = TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
        const viewChannel = MethodChannel('dev.nve/fabric_view_7');
        final viewCalls = <MethodCall>[];
        messenger.setMockMethodCallHandler(viewChannel, (MethodCall call) async {
          viewCalls.add(call);
          return null;
        });
        addTearDown(() => messenger.setMockMethodCallHandler(viewChannel, null));

        final controller = NveSurfaceController();
        final received = <NveEvent>[];

        Widget buildIos(Map<String, Object?> props) {
          return Directionality(
            textDirection: TextDirection.ltr,
            child: NvePlatformViewWidget(
              componentName: 'NveIosCard',
              surfaceId: 4002,
              width: 320,
              height: 200,
              props: props,
              controller: controller,
              onEvent: received.add,
            ),
          );
        }

        await tester.pumpWidget(buildIos(const <String, Object?>{'theme': 'dark'}));

        final uiKitFinder = find.byType(UiKitView);
        expect(uiKitFinder, findsOneWidget);
        final uiKit = tester.widget<UiKitView>(uiKitFinder);
        expect(uiKit.viewType, 'dev.nve/fabric_view');
        final params = uiKit.creationParams as Map<Object?, Object?>;
        expect(params['surfaceId'], 4002);
        expect(
          NveBinValueView.parse(params['propsBin']! as Uint8List).get('theme')?.asString(),
          'dark',
        );

        uiKit.onPlatformViewCreated!(7);
        expect(controller.isAttached, isTrue);

        controller.dispatchCommand('scrollToTop', <String, Object?>{'animated': true});
        controller.requestRender();
        expect(controller.snapshotState(), isNull);

        await tester.pumpWidget(buildIos(const <String, Object?>{'theme': 'light'}));

        expect(viewCalls.map((c) => c.method), <String>[
          'dispatchCommand',
          'requestRender',
          'setProps',
        ]);
        final cmdArgs = viewCalls[0].arguments as Map<Object?, Object?>;
        expect(cmdArgs['command'], 'scrollToTop');
        expect(
          NveBinValueView.parse(cmdArgs['argsBin']! as Uint8List).get('animated')?.asBool(),
          isTrue,
        );
        final setPropsArgs = viewCalls[2].arguments as Map<Object?, Object?>;
        expect(
          NveBinValueView.parse(setPropsArgs['propsBin']! as Uint8List).get('theme')?.asString(),
          'light',
        );

        const codec = StandardMethodCodec();
        await messenger.handlePlatformMessage(
          viewChannel.name,
          codec.encodeMethodCall(
            MethodCall('onEvent', <String, dynamic>{
              'surfaceId': 4002,
              'nodeId': 9,
              'timestampUs': 4567,
              'kind': NveEventKind.custom.code,
              'name': 'onTap',
              'payload': <String, dynamic>{'x': 10},
            }),
          ),
          (_) {},
        );
        expect(received, hasLength(1));
        expect(received.single.name, 'onTap');
        expect(received.single.parseBinValue()?.get('x')?.asInt64(), 10);

        await tester.pumpWidget(const SizedBox.shrink());
        expect(controller.isAttached, isFalse);
      },
    );

    testWidgets(
      'renders AndroidView with binary creation params on Android',
      variant: TargetPlatformVariant.only(TargetPlatform.android),
      (WidgetTester tester) async {
        await tester.pumpWidget(
          const Directionality(
            textDirection: TextDirection.ltr,
            child: NvePlatformViewWidget(
              componentName: 'NveAndroidCard',
              surfaceId: 4003,
              width: 360,
              height: 180,
              props: <String, Object?>{'step': 10},
            ),
          ),
        );

        final androidFinder = find.byType(AndroidView);
        expect(androidFinder, findsOneWidget);
        final androidView = tester.widget<AndroidView>(androidFinder);
        expect(androidView.viewType, 'dev.nve/fabric_view');
        expect(androidView.hitTestBehavior, PlatformViewHitTestBehavior.opaque);
        final params = androidView.creationParams as Map<Object?, Object?>;
        expect(params['surfaceId'], 4003);
        expect(
          NveBinValueView.parse(params['propsBin']! as Uint8List).get('step')?.asInt64(),
          10,
        );
      },
    );
  });
}
