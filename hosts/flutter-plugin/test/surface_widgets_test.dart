import 'dart:convert';

import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

void main() {
  const textureChannel = MethodChannel('dev.nve/flutter_texture');
  const eventChannelName = 'dev.nve/flutter_events';

  group('NveSurfaceController detached behavior', () {
    test('isAttached is false and calls are safe no-ops when detached', () {
      final controller = NveSurfaceController();
      expect(controller.isAttached, isFalse);
      controller.dispatchCommand('reset', <String, dynamic>{'force': true});
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
            props: const <String, dynamic>{'count': 1},
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
      'full texture lifecycle: create, resize, props diff, commands, pointers, events, dispose',
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
          required Map<String, dynamic> props,
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
          buildHost(props: const <String, dynamic>{'count': 1}, controller: controller1),
        );
        await tester.pump();

        // Verifies createTextureSurface + Texture(textureId: 42) + resizeSurface.
        final createCall = calls.firstWhere((c) => c.method == 'createTextureSurface');
        expect(createCall.arguments, <String, dynamic>{
          'surfaceId': 3001,
          'componentName': 'NveCounterCard',
          'vtableSymbol': 'nve_get_interface',
          'libraryPath': null,
          'propsJson': '{"count":1}',
        });

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

        // Rebuilding with identical props & size should not emit setProps or duplicate resizeSurface.
        await tester.pumpWidget(
          buildHost(props: const <String, dynamic>{'count': 1}, controller: controller1),
        );
        expect(calls.where((c) => c.method == 'setProps'), isEmpty);
        expect(calls.where((c) => c.method == 'resizeSurface'), hasLength(1));

        // Updating props & swapping controller detaches old controller, attaches new, sends setProps.
        await tester.pumpWidget(
          buildHost(props: const <String, dynamic>{'count': 2}, controller: controller2),
        );
        expect(controller1.isAttached, isFalse);
        expect(controller2.isAttached, isTrue);

        final setPropsCall = calls.where((c) => c.method == 'setProps').single;
        expect(setPropsCall.arguments, <String, dynamic>{
          'surfaceId': 3001,
          'propsJson': jsonEncode(<String, dynamic>{'count': 2}),
        });

        // Imperative controller commands and render request.
        controller2.dispatchCommand('reset', <String, dynamic>{'animated': false});
        controller2.requestRender();
        expect(controller2.snapshotState(), isNull);

        final cmdCall = calls.where((c) => c.method == 'dispatchCommand').single;
        expect(cmdCall.arguments, <String, dynamic>{
          'surfaceId': 3001,
          'command': 'reset',
          'argsJson': '{"animated":false}',
        });
        expect(
          calls.where((c) => c.method == 'requestRender').single.arguments,
          <String, dynamic>{'surfaceId': 3001},
        );

        // Pointer forwarding (down, move, up).
        final gesture = await tester.startGesture(const Offset(25, 40));
        await gesture.moveBy(const Offset(10, 5));
        await gesture.up();
        await tester.pump();

        final pointerPhases = calls
            .where((c) => c.method == 'pointerEvent')
            .map((c) => (c.arguments as Map<Object?, Object?>)['phase'])
            .toList();
        expect(pointerPhases, containsAllInOrder(<int>[1, 2, 3]));

        // Inbound native event via EventChannel.
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
        expect(receivedEvents.single.payloadJson, <String, dynamic>{'count': 3});

        // Disposal sends destroyTextureSurface and detaches controller.
        await tester.pumpWidget(const SizedBox.shrink());
        expect(controller2.isAttached, isFalse);
        final destroyCall = calls.where((c) => c.method == 'destroyTextureSurface').single;
        expect(destroyCall.arguments, <String, dynamic>{
          'surfaceId': 3001,
          'textureId': 42,
        });
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
      'renders UiKitView on iOS and bridges view channel commands, props, and events',
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

        Widget buildIos(Map<String, dynamic> props) {
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

        await tester.pumpWidget(buildIos(const <String, dynamic>{'theme': 'dark'}));

        final uiKitFinder = find.byType(UiKitView);
        expect(uiKitFinder, findsOneWidget);
        final uiKit = tester.widget<UiKitView>(uiKitFinder);
        expect(uiKit.viewType, 'dev.nve/fabric_view');
        expect(uiKit.creationParams, <String, dynamic>{
          'surfaceId': 4002,
          'componentName': 'NveIosCard',
          'vtableSymbol': 'nve_get_interface',
          'libraryPath': null,
          'propsJson': '{"theme":"dark"}',
        });

        // Simulate platform view creation callback and exercise view-scoped MethodChannel.
        uiKit.onPlatformViewCreated!(7);
        expect(controller.isAttached, isTrue);

        controller.dispatchCommand('scrollToTop', <String, dynamic>{'animated': true});
        controller.requestRender();
        expect(controller.snapshotState(), isNull);

        await tester.pumpWidget(buildIos(const <String, dynamic>{'theme': 'light'}));

        expect(viewCalls.map((c) => c.method), <String>[
          'dispatchCommand',
          'requestRender',
          'setProps',
        ]);
        expect(viewCalls[0].arguments, <String, dynamic>{
          'command': 'scrollToTop',
          'argsJson': '{"animated":true}',
        });
        expect(viewCalls[2].arguments, <String, dynamic>{
          'propsJson': '{"theme":"light"}',
        });

        // Simulate native-to-Dart onEvent callback over the view channel.
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
        expect(received.single.payloadJson, <String, dynamic>{'x': 10});

        await tester.pumpWidget(const SizedBox.shrink());
        expect(controller.isAttached, isFalse);
      },
    );

    testWidgets(
      'renders AndroidView with creation params on Android',
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
              props: <String, dynamic>{'step': 10},
            ),
          ),
        );

        final androidFinder = find.byType(AndroidView);
        expect(androidFinder, findsOneWidget);
        final androidView = tester.widget<AndroidView>(androidFinder);
        expect(androidView.viewType, 'dev.nve/fabric_view');
        expect(androidView.hitTestBehavior, PlatformViewHitTestBehavior.opaque);
        expect(androidView.creationParams, <String, dynamic>{
          'surfaceId': 4003,
          'componentName': 'NveAndroidCard',
          'vtableSymbol': 'nve_get_interface',
          'libraryPath': null,
          'propsJson': '{"step":10}',
        });
      },
    );
  });
}
