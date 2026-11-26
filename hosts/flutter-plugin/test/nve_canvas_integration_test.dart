import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_canvas.dart';

String _readStr(NveStrSlice s) =>
    (s.ptr == ffi.nullptr || s.len == 0) ? '' : utf8.decode(s.ptr.asTypedList(s.len));

Object? _readVal(NveValue v) => switch (NveValueTag.fromCode(v.tag)) {
      NveValueTag.nullValue || null => null,
      NveValueTag.boolValue => v.data.boolVal != 0,
      NveValueTag.i64 => v.data.i64Val,
      NveValueTag.f64 => v.data.f64Val,
      NveValueTag.str => _readStr(v.data.strVal),
      NveValueTag.bytes => (v.data.bytesVal.ptr == ffi.nullptr || v.data.bytesVal.len == 0)
          ? Uint8List(0)
          : Uint8List.fromList(v.data.bytesVal.ptr.asTypedList(v.data.bytesVal.len)),
      NveValueTag.array => List<Object?>.generate(
          v.data.arrayVal.len,
          (int i) => _readVal(v.data.arrayVal.ptr[i]),
        ),
      NveValueTag.map => <String, Object?>{
          for (var i = 0; i < v.data.mapVal.len; i++)
            _readStr(v.data.mapVal.ptr[i].key): _readVal(v.data.mapVal.ptr[i].value),
        },
    };

class _TestHeap {
  static final ffi.DynamicLibrary _libc = Platform.isWindows
      ? ffi.DynamicLibrary.open('ucrtbase.dll')
      : ffi.DynamicLibrary.process();

  static final ffi.Pointer<ffi.Void> Function(int, int) _calloc = _libc.lookupFunction<
      ffi.Pointer<ffi.Void> Function(ffi.Size, ffi.Size),
      ffi.Pointer<ffi.Void> Function(int, int)>('calloc');
  static final void Function(ffi.Pointer<ffi.Void>) _free = _libc.lookupFunction<
      ffi.Void Function(ffi.Pointer<ffi.Void>), void Function(ffi.Pointer<ffi.Void>)>('free');

  final List<ffi.Pointer<ffi.Void>> _ptrs = <ffi.Pointer<ffi.Void>>[];

  ffi.Pointer<T> allocate<T extends ffi.NativeType>(int byteCount) {
    final p = _calloc(1, byteCount == 0 ? 1 : byteCount);
    _ptrs.add(p);
    return p.cast<T>();
  }

  ffi.Pointer<ffi.Uint8> copy(Uint8List data) {
    final p = allocate<ffi.Uint8>(data.length);
    if (data.isNotEmpty) {
      p.asTypedList(data.length).setAll(0, data);
    }
    return p;
  }

  void dispose() {
    for (final p in _ptrs) {
      _free(p);
    }
    _ptrs.clear();
  }
}

class _SyntheticNativeApp {
  _SyntheticNativeApp({required this.buildFrame, this.initialState = '0'}) {
    _scratchSlice = _heap.allocate<NveBufferSlice>(ffi.sizeOf<NveBufferSlice>());
    _scratchBatch = _heap.allocate<NveEventBatch>(ffi.sizeOf<NveEventBatch>());
    _createCb = ffi.NativeCallable<NveCreateNative>.isolateLocal(_onCreate);
    _destroyCb = ffi.NativeCallable<NveDestroyNative>.isolateLocal(_onDestroy);
    _renderCb = ffi.NativeCallable<NveRenderNative>.isolateLocal(_onRender);
    _dispatchCb = ffi.NativeCallable<NveDispatchEventsNative>.isolateLocal(
      _onDispatch,
      exceptionalReturn: 0,
    );
    _setPropsCb = ffi.NativeCallable<NveSetPropsNative>.isolateLocal(
      _onSetProps,
      exceptionalReturn: 0,
    );
    _drainCb = ffi.NativeCallable<NveDrainEmittedEventsNative>.isolateLocal(_onDrain);
    _hotPatchCb = ffi.NativeCallable<NveApplyHotPatchNative>.isolateLocal(
      _onHotPatch,
      exceptionalReturn: 0,
    );
    _snapshotCb = ffi.NativeCallable<NveSnapshotStateNative>.isolateLocal(_onSnapshot);
    _restoreCb = ffi.NativeCallable<NveRestoreStateNative>.isolateLocal(_onRestore);

    _surfaceApi = _heap.allocate<NveSurfaceApi>(ffi.sizeOf<NveSurfaceApi>());
    _surfaceApi.ref
      ..create = _createCb.nativeFunction
      ..destroy = _destroyCb.nativeFunction
      ..render = _renderCb.nativeFunction
      ..dispatchEvents = _dispatchCb.nativeFunction;

    _componentApi = _heap.allocate<NveComponentApi>(ffi.sizeOf<NveComponentApi>());
    _componentApi.ref
      ..setProps = _setPropsCb.nativeFunction
      ..drainEmittedEvents = _drainCb.nativeFunction;

    _devApi = _heap.allocate<NveDevApi>(ffi.sizeOf<NveDevApi>());
    _devApi.ref
      ..applyHotPatch = _hotPatchCb.nativeFunction
      ..snapshotState = _snapshotCb.nativeFunction
      ..restoreState = _restoreCb.nativeFunction;

    stateText = initialState;
  }

  final Uint8List Function(int frameId, String state) buildFrame;
  final String initialState;

  final _TestHeap _heap = _TestHeap();
  late final ffi.Pointer<NveBufferSlice> _scratchSlice;
  late final ffi.Pointer<NveEventBatch> _scratchBatch;
  late final ffi.Pointer<NveSurfaceApi> _surfaceApi;
  late final ffi.Pointer<NveComponentApi> _componentApi;
  late final ffi.Pointer<NveDevApi> _devApi;

  late final ffi.NativeCallable<NveCreateNative> _createCb;
  late final ffi.NativeCallable<NveDestroyNative> _destroyCb;
  late final ffi.NativeCallable<NveRenderNative> _renderCb;
  late final ffi.NativeCallable<NveDispatchEventsNative> _dispatchCb;
  late final ffi.NativeCallable<NveSetPropsNative> _setPropsCb;
  late final ffi.NativeCallable<NveDrainEmittedEventsNative> _drainCb;
  late final ffi.NativeCallable<NveApplyHotPatchNative> _hotPatchCb;
  late final ffi.NativeCallable<NveSnapshotStateNative> _snapshotCb;
  late final ffi.NativeCallable<NveRestoreStateNative> _restoreCb;

  String stateText = '';
  final List<NveEvent> receivedEvents = <NveEvent>[];
  final List<Map<String, Object?>> propsLog = <Map<String, Object?>>[];
  final List<Uint8List> hotPatchLog = <Uint8List>[];
  List<NveEvent> pendingOutbound = <NveEvent>[];
  int destroyCount = 0;

  ffi.Pointer<ffi.Void> getInterface(int id) {
    return switch (id) {
      nveIfaceSurface => _surfaceApi.cast<ffi.Void>(),
      nveIfaceComponent => _componentApi.cast<ffi.Void>(),
      nveIfaceDev => _devApi.cast<ffi.Void>(),
      _ => ffi.nullptr,
    };
  }

  ffi.Pointer<ffi.Void> _onCreate(int surfaceId) =>
      ffi.Pointer<ffi.Void>.fromAddress(0x4000 + surfaceId);

  void _onDestroy(ffi.Pointer<ffi.Void> _) {
    destroyCount += 1;
  }

  NveBufferSlice _onRender(ffi.Pointer<ffi.Void> _, int frameId) {
    final bytes = buildFrame(frameId, stateText);
    if (bytes.isEmpty) {
      _scratchSlice.ref
        ..ptr = ffi.nullptr
        ..len = 0;
      return _scratchSlice.ref;
    }
    final ptr = _heap.copy(bytes);
    _scratchSlice.ref
      ..ptr = ptr
      ..len = bytes.length;
    return _scratchSlice.ref;
  }

  int _onDispatch(ffi.Pointer<ffi.Void> _, NveEventBatch batch) {
    if (batch.ptr == ffi.nullptr || batch.len == 0) return 0;
    receivedEvents.addAll(NveDirectEventWriter.decodeBatch(batch));
    return 1;
  }

  int _onSetProps(
    ffi.Pointer<ffi.Void> _,
    ffi.Pointer<NveMapEntry> entries,
    int count,
  ) {
    final map = <String, Object?>{};
    if (entries != ffi.nullptr && count > 0) {
      for (var i = 0; i < count; i++) {
        map[_readStr(entries[i].key)] = _readVal(entries[i].value);
      }
    }
    propsLog.add(map);
    return 1;
  }

  NveEventBatch _onDrain(ffi.Pointer<ffi.Void> _) {
    final out = pendingOutbound;
    pendingOutbound = <NveEvent>[];
    if (out.isEmpty) {
      _scratchBatch.ref
        ..ptr = ffi.nullptr
        ..len = 0
        ..pad = 0;
      return _scratchBatch.ref;
    }
    final ptr = _heap.allocate<NveDirectEvent>(ffi.sizeOf<NveDirectEvent>() * out.length);
    for (var i = 0; i < out.length; i++) {
      final ev = out[i];
      final dst = ptr[i]
        ..surfaceId = ev.surfaceId
        ..nodeId = ev.nodeId
        ..timestampUs = ev.timestampUs
        ..kind = ev.kind.code;
      if (ev.payload case NveBinValuePayload(:final bytes)) {
        dst.payloadTag = NveEventPayloadTag.custom.code;
        if (ev.name != null && ev.name!.isNotEmpty) {
          final nameBytes = Uint8List.fromList(utf8.encode(ev.name!));
          dst.name
            ..ptr = _heap.copy(nameBytes)
            ..len = nameBytes.length;
        }
        if (bytes.isNotEmpty) {
          dst.value.tag = NveValueTag.bytes.code;
          dst.value.data.bytesVal
            ..ptr = _heap.copy(bytes)
            ..len = bytes.length;
        }
      }
    }
    _scratchBatch.ref
      ..ptr = ptr
      ..len = out.length
      ..pad = 0;
    return _scratchBatch.ref;
  }

  int _onHotPatch(ffi.Pointer<ffi.Void> _, NveBufferSlice slice) {
    final bytes = Uint8List.fromList(slice.ptr.asTypedList(slice.len));
    hotPatchLog.add(bytes);
    return 1;
  }

  NveBufferSlice _onSnapshot(ffi.Pointer<ffi.Void> _) {
    final bytes = Uint8List.fromList(utf8.encode(stateText));
    final ptr = _heap.copy(bytes);
    _scratchSlice.ref
      ..ptr = ptr
      ..len = bytes.length;
    return _scratchSlice.ref;
  }

  void _onRestore(ffi.Pointer<ffi.Void> _, NveBufferSlice slice) {
    if (slice.ptr == ffi.nullptr || slice.len == 0) return;
    stateText = utf8.decode(slice.ptr.asTypedList(slice.len));
  }

  void close() {
    _createCb.close();
    _destroyCb.close();
    _renderCb.close();
    _dispatchCb.close();
    _setPropsCb.close();
    _drainCb.close();
    _hotPatchCb.close();
    _snapshotCb.close();
    _restoreCb.close();
    _heap.dispose();
  }
}

String _resolveKitchenSinkLibraryPath() {
  final envPath = Platform.environment['NVE_KITCHEN_SINK_LIB'];
  if (envPath != null && envPath.isNotEmpty && File(envPath).existsSync()) {
    return envPath;
  }
  final libName = Platform.isMacOS
      ? 'libnve_example_kitchen_sink.dylib'
      : Platform.isWindows
          ? 'nve_example_kitchen_sink.dll'
          : 'libnve_example_kitchen_sink.so';
  final candidates = <String>[
    '../../target/debug/$libName',
    'target/debug/$libName',
    '../../target/release/$libName',
    'target/release/$libName',
  ];
  for (final candidate in candidates) {
    final file = File(candidate);
    if (file.existsSync()) {
      return file.absolute.path;
    }
  }
  throw StateError(
    'Could not find $libName. Run `cargo build -p nve-example-kitchen-sink` first '
    '(searched from ${Directory.current.path}).',
  );
}

final Uint8List _nvp1DevStatus = Uint8List.fromList(<int>[
  0x4E, 0x56, 0x50, 0x31, 3, //
  1, 0, 0, 0, 0x78, //
  0, 0, 0, 0, 0, 0, 0, 0,
]);

void main() {
  group('encodePressEvent & NveCanvasView', () {
    test('encodePressEvent matches NveEventRingCodec.decode wire format verbatim', () {
      final bytes = encodePressEvent(
        surfaceId: 9,
        nodeId: 42,
        x: 123.5,
        y: 67.25,
        timestampUs: 987654321,
      );
      expect(bytes.length, 30);

      final decoded = NveEventRingCodec.decode(bytes);
      expect(decoded, hasLength(1));
      final ev = decoded.single;
      expect(ev.surfaceId, 9);
      expect(ev.nodeId, 42);
      expect(ev.timestampUs, 987654321);
      expect(ev.kind, NveEventKind.press);
      final pt = ev.payload as NvePointPayload;
      expect(pt.x, closeTo(123.5, 1e-3));
      expect(pt.y, closeTo(67.25, 1e-3));
    });

    testWidgets('zIndex reverse hit-testing, opacity painting, props, hot-patch, and atomic reload',
        (WidgetTester tester) async {
      Uint8List buildZScene(int frameId, String label) {
        final b = NveCommandBufferBuilder(surfaceId: 1, frameId: frameId)
          ..createNode(2, NveComponentKind.view)
          ..createNode(3, NveComponentKind.pressable)
          ..createNode(4, NveComponentKind.pressable)
          ..createNode(5, NveComponentKind.text)
          ..setChildren(nveRootNodeId, <int>[2])
          ..setChildren(2, <int>[4, 3])
          ..setChildren(4, <int>[5])
          ..setLayout(
            2,
            const NveLayoutProps(
              width: NveDimension.percent(100),
              height: NveDimension.percent(100),
            ),
          )
          ..setLayout(
            3,
            const NveLayoutProps(
              width: NveDimension.points(120),
              height: NveDimension.points(60),
              margin: NveEdgeInsets(top: -60),
            ),
          )
          ..setLayout(
            4,
            const NveLayoutProps(
              width: NveDimension.points(120),
              height: NveDimension.points(60),
              justifyContent: NveFlexJustify.center,
              alignItems: NveFlexAlign.center,
            ),
          )
          ..setStyle(
            3,
            const NveStyleProps(
              background: NveColorRgba.rgb(200, 40, 40),
              zIndex: 1,
            ),
          )
          ..setStyle(
            4,
            const NveStyleProps(
              background: NveColorRgba.rgb(37, 99, 235),
              borderColor: NveColorRgba.white,
              borderWidth: 2,
              borderRadius: 10,
              opacity: 0.85,
              zIndex: 10,
            ),
          )
          ..setStyle(
            5,
            const NveStyleProps(textColor: NveColorRgba.white, fontSize: 16),
          )
          ..setText(5, 'State: $label')
          ..setEventMask(3, NveEventMask.press)
          ..setEventMask(4, NveEventMask.press)
          ..commit();
        return b.toBytes();
      }

      final app1 = _SyntheticNativeApp(buildFrame: buildZScene, initialState: 'v1');
      final app2 = _SyntheticNativeApp(buildFrame: buildZScene, initialState: 'fresh');
      addTearDown(app1.close);
      addTearDown(app2.close);

      final outbound = <NveEvent>[];
      final controller = NveCanvasController.fromInterfaceResolver(
        surfaceId: 1,
        getInterface: app1.getInterface,
      );
      addTearDown(controller.dispose);

      await tester.pumpWidget(
        Directionality(
          textDirection: TextDirection.ltr,
          child: SizedBox(
            width: 300,
            height: 200,
            child: NveCanvasView(
              controller: controller,
              onEvent: outbound.add,
            ),
          ),
        ),
      );

      final renderBox = tester.renderObject<RenderNveCanvas>(find.byType(NveCanvasView));
      expect(renderBox.layoutOf(4), isNotNull);
      expect(renderBox.debugNodes[5]!.text, 'State: v1');

      final hitCenter = renderBox.layoutOf(4)!.center;
      await tester.tapAt(hitCenter);
      await tester.pump();
      expect(app1.receivedEvents, hasLength(1));
      expect(app1.receivedEvents.single.nodeId, 4);

      final dragGesture = await tester.startGesture(hitCenter);
      await dragGesture.moveTo(const Offset(260, 180));
      await dragGesture.up();
      await tester.pump();
      expect(app1.receivedEvents, hasLength(1));

      app1.pendingOutbound = <NveEvent>[
        NveEvent.custom(
          surfaceId: 1,
          nodeId: 4,
          timestampUs: 100,
          name: 'onPing',
          value: <String, Object?>{'ok': true},
        ),
      ];
      expect(controller.setPropsDirect(<String, Object?>{'theme': 'dark'}), isTrue);
      expect(controller.applyHotPatch(_nvp1DevStatus), isTrue);
      expect(app1.propsLog, hasLength(1));
      expect(app1.propsLog.single['theme'], 'dark');
      expect(app1.hotPatchLog, hasLength(1));
      expect(app1.hotPatchLog.single, _nvp1DevStatus);
      expect(outbound.map((e) => e.name), contains('onPing'));

      app1.stateText = 'preserved-42';
      final reloadRes = controller.reloadWithInterfaceResolver(app2.getInterface);
      expect(reloadRes.isOk, isTrue);
      expect(app1.destroyCount, 1);
      expect(app2.stateText, 'preserved-42');
      expect(controller.debugNodes[5]!.text, 'State: preserved-42');
    });
  });

  group('Live Rust nve-example-kitchen-sink cdylib on NveCanvasView', () {
    testWidgets(
      'renders kitchen sink gallery, handles real taps, preserves state across reloadLibrary, resets, and rasterizes PNG',
      (WidgetTester tester) async {
        final libPath = _resolveKitchenSinkLibraryPath();

        String? fontFamily;
        await tester.runAsync(() async {
          const candidates = <String>[
            '/System/Library/Fonts/Supplemental/Arial.ttf',
            '/System/Library/Fonts/SFNS.ttf',
          ];
          for (final path in candidates) {
            final f = File(path);
            if (f.existsSync()) {
              final bytes = f.readAsBytesSync();
              final loader = FontLoader('NveTestSans')
                ..addFont(Future<ByteData>.value(ByteData.sublistView(bytes)));
              await loader.load();
              fontFamily = 'NveTestSans';
              break;
            }
          }
        });

        final controller = NveCanvasController.open(
          surfaceId: 1,
          libraryPath: libPath,
          defaultFontFamily: fontFamily,
        );
        addTearDown(controller.dispose);

        controller.setPropsDirect(<String, Object?>{'initialCount': 0}, rerender: false);

        final boundaryKey = GlobalKey();
        await tester.pumpWidget(
          Directionality(
            textDirection: TextDirection.ltr,
            child: Align(
              alignment: Alignment.topLeft,
              child: RepaintBoundary(
                key: boundaryKey,
                child: SizedBox(
                  width: 960,
                  height: 720,
                  child: NveCanvasView(controller: controller),
                ),
              ),
            ),
          ),
        );

        final renderBox = tester.renderObject<RenderNveCanvas>(find.byType(NveCanvasView));

        NveNode findLeafText(String expected) {
          return renderBox.debugNodes.values.firstWhere(
            (n) => n.isTextLeaf && n.text == expected,
            orElse: () => throw StateError(
              'Expected leaf text "$expected" not found. Present texts: '
              '${renderBox.debugNodes.values.where((n) => n.text != null).map((n) => '"${n.text}"').toList()}',
            ),
          );
        }

        NveNode findPressableForLabel(String label) {
          final leaf = findLeafText(label);
          int? cursor = leaf.parentId;
          while (cursor != null) {
            final node = renderBox.debugNodes[cursor]!;
            if (node.kind == NveComponentKind.pressable) {
              return node;
            }
            cursor = node.parentId;
          }
          throw StateError('No Pressable ancestor found for "$label"');
        }

        final titleNode = findLeafText('NVE Kitchen Sink');
        final countNode = findLeafText('Count: 0');
        final incButton = findPressableForLabel('Increment +');
        final resetButton = findPressableForLabel('Reset');
        final incText = findLeafText('Increment +');
        final resetText = findLeafText('Reset');

        expect(titleNode.style.fontSize, closeTo(16.0, 0.5));
        expect(countNode.style.fontSize, closeTo(28.0, 0.1));

        final countRect = renderBox.layoutOf(countNode.id)!;
        final incBtnRect = renderBox.layoutOf(incButton.id)!;
        final resetBtnRect = renderBox.layoutOf(resetButton.id)!;
        final incTextRect = renderBox.layoutOf(incText.id)!;
        final resetTextRect = renderBox.layoutOf(resetText.id)!;

        expect(countRect.bottom, lessThan(incBtnRect.top));
        expect((incTextRect.center.dx - incBtnRect.center.dx).abs(), lessThanOrEqualTo(4.0));
        expect((incTextRect.center.dy - incBtnRect.center.dy).abs(), lessThanOrEqualTo(4.0));
        expect((resetTextRect.center.dx - resetBtnRect.center.dx).abs(), lessThanOrEqualTo(4.0));
        expect((resetTextRect.center.dy - resetBtnRect.center.dy).abs(), lessThanOrEqualTo(4.0));

        final origin = renderBox.localToGlobal(Offset.zero);
        await tester.tapAt(origin + incBtnRect.center);
        await tester.pump();
        expect(findLeafText('Count: 1').style.fontSize, closeTo(28.0, 0.1));

        await tester.tapAt(origin + incBtnRect.center);
        await tester.pump();
        expect(findLeafText('Count: 2').style.fontSize, closeTo(28.0, 0.1));

        final pngBytes = await tester.runAsync(() async {
          final boundary =
              boundaryKey.currentContext!.findRenderObject()! as RenderRepaintBoundary;
          final image = await boundary.toImage(pixelRatio: 2.0);
          final byteData = await image.toByteData(format: ui.ImageByteFormat.png);
          image.dispose();
          return byteData?.buffer.asUint8List();
        });
        expect(pngBytes, isNotNull);
        expect(pngBytes!.length, greaterThan(1000));
        final outPng = Platform.environment['NVE_SAVE_VERIFY_PNG'];
        if (outPng != null && outPng.isNotEmpty) {
          File(outPng).writeAsBytesSync(pngBytes);
        }

        final reloadResult = controller.reloadLibrary(libPath);
        await tester.pump();
        expect(reloadResult.isOk, isTrue);
        expect(findLeafText('Count: 2').text, 'Count: 2');

        final resetBtnAfterReload = findPressableForLabel('Reset');
        final resetRectAfterReload = renderBox.layoutOf(resetBtnAfterReload.id)!;
        await tester.tapAt(origin + resetRectAfterReload.center);
        await tester.pump();
        expect(findLeafText('Count: 0').text, 'Count: 0');
      },
    );
  });
}
