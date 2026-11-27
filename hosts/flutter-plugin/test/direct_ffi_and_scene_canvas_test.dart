import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_canvas.dart';

class _NativeSlabAlloc {
  static final ffi.DynamicLibrary _libc = Platform.isWindows
      ? ffi.DynamicLibrary.open('ucrtbase.dll')
      : ffi.DynamicLibrary.process();

  static final ffi.Pointer<ffi.Void> Function(int, int) _calloc = _libc.lookupFunction<
      ffi.Pointer<ffi.Void> Function(ffi.Size, ffi.Size),
      ffi.Pointer<ffi.Void> Function(int, int)>('calloc');
  static final void Function(ffi.Pointer<ffi.Void>) _free = _libc.lookupFunction<
      ffi.Void Function(ffi.Pointer<ffi.Void>),
      void Function(ffi.Pointer<ffi.Void>)>('free');

  final List<ffi.Pointer<ffi.Void>> _ptrs = <ffi.Pointer<ffi.Void>>[];

  ffi.Pointer<T> allocate<T extends ffi.NativeType>(int byteCount) {
    final ptr = _calloc(1, byteCount <= 0 ? 1 : byteCount);
    if (ptr == ffi.nullptr) throw StateError('calloc($byteCount) failed');
    _ptrs.add(ptr);
    return ptr.cast<T>();
  }

  void dispose() {
    for (final p in _ptrs) {
      _free(p);
    }
    _ptrs.clear();
  }
}

int _packRgba(int r, int g, int b, [int a = 255]) =>
    (r & 0xFF) | ((g & 0xFF) << 8) | ((b & 0xFF) << 16) | ((a & 0xFF) << 24);

class _SceneHostWidget extends LeafRenderObjectWidget {
  const _SceneHostWidget({
    required this.onCreated,
    this.onSceneEvent,
    this.backgroundColor,
  });

  final void Function(RenderNveCanvas renderBox) onCreated;
  final NveEventCallback? onSceneEvent;
  final Color? backgroundColor;

  @override
  RenderNveCanvas createRenderObject(BuildContext context) {
    final box = RenderNveCanvas(
      surfaceId: 77,
      backgroundColor: backgroundColor,
      onSceneEvent: onSceneEvent,
    );
    onCreated(box);
    return box;
  }

  @override
  void updateRenderObject(BuildContext context, RenderNveCanvas renderObject) {
    renderObject
      ..backgroundColor = backgroundColor
      ..onSceneEvent = onSceneEvent;
  }
}

class _ScrollTestModule {
  _ScrollTestModule() {
    _scratchSlice = _alloc.allocate<NveBufferSlice>(ffi.sizeOf<NveBufferSlice>());
    _createCb = ffi.NativeCallable<NveCreateNative>.isolateLocal(
      (int sid) => ffi.Pointer<ffi.Void>.fromAddress(0x5000 + sid),
    );
    _destroyCb = ffi.NativeCallable<NveDestroyNative>.isolateLocal((ffi.Pointer<ffi.Void> _) {});
    _renderCb = ffi.NativeCallable<NveRenderNative>.isolateLocal(_onRender);
    _dispatchCb = ffi.NativeCallable<NveDispatchEventsNative>.isolateLocal(
      _onDispatchDirect,
      exceptionalReturn: 0,
    );

    _surfaceApi = _alloc.allocate<NveSurfaceApi>(ffi.sizeOf<NveSurfaceApi>());
    _surfaceApi.ref
      ..create = _createCb.nativeFunction
      ..destroy = _destroyCb.nativeFunction
      ..render = _renderCb.nativeFunction
      ..dispatchEvents = _dispatchCb.nativeFunction;
  }

  final _NativeSlabAlloc _alloc = _NativeSlabAlloc();
  late final ffi.Pointer<NveBufferSlice> _scratchSlice;
  late final ffi.Pointer<NveSurfaceApi> _surfaceApi;
  late final ffi.NativeCallable<NveCreateNative> _createCb;
  late final ffi.NativeCallable<NveDestroyNative> _destroyCb;
  late final ffi.NativeCallable<NveRenderNative> _renderCb;
  late final ffi.NativeCallable<NveDispatchEventsNative> _dispatchCb;

  final List<NveEvent> directEvents = <NveEvent>[];

  ffi.Pointer<ffi.Void> getInterface(int id) =>
      id == nveIfaceSurface ? _surfaceApi.cast<ffi.Void>() : ffi.nullptr;

  NveBufferSlice _onRender(ffi.Pointer<ffi.Void> _, int frameId) {
    final b = NveCommandBufferBuilder(surfaceId: 1, frameId: frameId)
      ..createNode(2, NveComponentKind.scrollView)
      ..createNode(3, NveComponentKind.pressable)
      ..createNode(4, NveComponentKind.pressable)
      ..createNode(5, NveComponentKind.pressable)
      ..setChildren(nveRootNodeId, <int>[2])
      ..setChildren(2, <int>[3, 4, 5])
      ..setLayout(
        2,
        const NveLayoutProps(
          width: NveDimension.points(240),
          height: NveDimension.points(120),
          gap: 10,
        ),
      )
      ..setLayout(3, const NveLayoutProps(width: NveDimension.points(220), height: NveDimension.points(80)))
      ..setLayout(4, const NveLayoutProps(width: NveDimension.points(220), height: NveDimension.points(80)))
      ..setLayout(5, const NveLayoutProps(width: NveDimension.points(220), height: NveDimension.points(80)))
      ..setStyle(
        2,
        const NveStyleProps(
          background: NveColorRgba.rgb(15, 23, 42),
          borderRadius: 12,
        ),
      )
      ..setStyle(3, const NveStyleProps(background: NveColorRgba.rgb(37, 99, 235)))
      ..setStyle(4, const NveStyleProps(background: NveColorRgba.rgb(16, 185, 129)))
      ..setStyle(5, const NveStyleProps(background: NveColorRgba.rgb(245, 158, 11)))
      ..setEventMask(2, NveEventMask.scroll)
      ..setEventMask(3, NveEventMask.press)
      ..setEventMask(4, NveEventMask.press)
      ..setEventMask(5, NveEventMask.press)
      ..commit();
    final bytes = b.toBytes();
    final ptr = _alloc.allocate<ffi.Uint8>(bytes.length);
    ptr.asTypedList(bytes.length).setAll(0, bytes);
    _scratchSlice.ref
      ..ptr = ptr
      ..len = bytes.length;
    return _scratchSlice.ref;
  }

  int _onDispatchDirect(ffi.Pointer<ffi.Void> _, NveEventBatch batch) {
    directEvents.addAll(NveDirectEventWriter.decodeBatch(batch));
    return 1;
  }

  void close() {
    _createCb.close();
    _destroyCb.close();
    _renderCb.close();
    _dispatchCb.close();
    _alloc.dispose();
  }
}

void main() {
  group('Binary-codec and modularity invariants for hosts/flutter-plugin/lib', () {
    test('no jsonEncode/jsonDecode in lib/ and every Dart file < 900 lines', () {
      final libDir = Directory('lib');
      expect(libDir.existsSync(), isTrue);
      final dartFiles = libDir
          .listSync(recursive: true)
          .whereType<File>()
          .where((f) => f.path.endsWith('.dart'))
          .toList();
      expect(dartFiles, isNotEmpty);

      for (final file in dartFiles) {
        final content = file.readAsStringSync();
        final lineCount = '\n'.allMatches(content).length + 1;
        expect(
          lineCount,
          lessThan(900),
          reason: '${file.path} has $lineCount lines (must stay < 900)',
        );
        expect(
          content.contains('jsonEncode'),
          isFalse,
          reason: '${file.path} must not call jsonEncode',
        );
        expect(
          content.contains('jsonDecode'),
          isFalse,
          reason: '${file.path} must not call jsonDecode',
        );
      }
    });
  });

  group('Direct NveSceneView rendering & dirty-index updates in RenderNveCanvas', () {
    testWidgets(
      'paints 64-byte NveRenderNode slabs, updates dirty_indices/removed_ids incrementally, and handles scroll/press',
      (WidgetTester tester) async {
        final alloc = _NativeSlabAlloc();
        addTearDown(alloc.dispose);

        final textUtf8 = Uint8List.fromList(utf8.encode('HeaderCardItemAItemB'));
        final textPtr = alloc.allocate<ffi.Uint8>(textUtf8.length);
        textPtr.asTypedList(textUtf8.length).setAll(0, textUtf8);

        // root(1) -> [2 (header card), 3 (scrollView)]
        // scrollView(3) -> [4 (item A), 5 (item B below fold)]
        final childrenPtr = alloc.allocate<ffi.Uint32>(4 * 4);
        childrenPtr.asTypedList(4).setAll(0, <int>[2, 3, 4, 5]);

        final nodesPtr = alloc.allocate<NveRenderNode>(5 * ffi.sizeOf<NveRenderNode>());
        nodesPtr[0]
          ..nodeId = 1
          ..kind = nveNodeKindRoot
          ..flags = 0
          ..opacityU8 = 255
          ..x = 0
          ..y = 0
          ..width = 300
          ..height = 200
          ..bgRgba = _packRgba(15, 23, 42)
          ..childCount = 2
          ..childOffset = 0;

        nodesPtr[1]
          ..nodeId = 2
          ..kind = nveNodeKindPressable
          ..flags = nveNodeInteractive | nveNodeLeafText
          ..opacityU8 = 230
          ..x = 10
          ..y = 10
          ..width = 280
          ..height = 40
          ..bgRgba = _packRgba(37, 99, 235)
          ..textRgba = _packRgba(255, 255, 255)
          ..borderRgba = _packRgba(147, 197, 253)
          ..borderRadius = 8
          ..borderWidth = 2
          ..fontSize = 16
          ..textOffset = 0
          ..textLen = 10 // "HeaderCard"
          ..zIndex = 2;

        nodesPtr[2]
          ..nodeId = 3
          ..kind = nveNodeKindScrollView
          ..flags = nveNodeClipBounds
          ..opacityU8 = 255
          ..x = 10
          ..y = 60
          ..width = 280
          ..height = 100
          ..bgRgba = _packRgba(30, 41, 59)
          ..borderRadius = 8
          ..childCount = 2
          ..childOffset = 2;

        nodesPtr[3]
          ..nodeId = 4
          ..kind = nveNodeKindPressable
          ..flags = nveNodeInteractive | nveNodeLeafText
          ..opacityU8 = 255
          ..x = 20
          ..y = 70
          ..width = 260
          ..height = 70
          ..bgRgba = _packRgba(16, 185, 129)
          ..textRgba = _packRgba(255, 255, 255)
          ..fontSize = 14
          ..textOffset = 10
          ..textLen = 5; // "ItemA"

        nodesPtr[4]
          ..nodeId = 5
          ..kind = nveNodeKindPressable
          ..flags = nveNodeInteractive | nveNodeLeafText
          ..opacityU8 = 255
          ..x = 20
          ..y = 180
          ..width = 260
          ..height = 70
          ..bgRgba = _packRgba(245, 158, 11)
          ..textRgba = _packRgba(255, 255, 255)
          ..fontSize = 14
          ..textOffset = 15
          ..textLen = 5; // "ItemB"

        final dirtyPtr = alloc.allocate<ffi.Uint32>(5 * 4);
        dirtyPtr.asTypedList(5).setAll(0, <int>[0, 1, 2, 3, 4]);

        final viewPtr = alloc.allocate<NveSceneView>(ffi.sizeOf<NveSceneView>());
        viewPtr.ref
          ..schemaHash = nveSceneSchemaHash.toSignedInt()
          ..frameId = 1
          ..nodeCount = 5
          ..dirtyCount = 5
          ..removedCount = 0
          ..textSlabLen = textUtf8.length
          ..childrenSlabLen = 4
          ..nodes = nodesPtr
          ..textSlab = textPtr
          ..childrenSlab = childrenPtr
          ..dirtyIndices = dirtyPtr
          ..removedIds = ffi.nullptr;

        late RenderNveCanvas renderBox;
        final sceneEvents = <NveEvent>[];
        final boundaryKey = GlobalKey();

        await tester.pumpWidget(
          Directionality(
            textDirection: TextDirection.ltr,
            child: Align(
              alignment: Alignment.topLeft,
              child: RepaintBoundary(
                key: boundaryKey,
                child: SizedBox(
                  width: 300,
                  height: 200,
                  child: _SceneHostWidget(
                    backgroundColor: const Color(0xFF020617),
                    onSceneEvent: sceneEvents.add,
                    onCreated: (RenderNveCanvas box) => renderBox = box,
                  ),
                ),
              ),
            ),
          ),
        );

        // Frame 1: ingest initial 64-byte scene directly via NveSceneReader.
        renderBox.updateFromSceneView(NveSceneReader.fromPointer(viewPtr));
        await tester.pump();
        expect(renderBox.isSceneMode, isTrue);
        expect(renderBox.sceneStore.frameId, 1);
        expect(renderBox.sceneStore.nodeCount, 5);
        expect(renderBox.sceneStore.node(2)?.text, 'HeaderCard');

        // Verify real canvas rasterization succeeds.
        final pngBytes = await tester.runAsync(() async {
          final boundary = boundaryKey.currentContext!.findRenderObject()! as RenderRepaintBoundary;
          final img = await boundary.toImage(pixelRatio: 1.0);
          final data = await img.toByteData(format: ui.ImageByteFormat.png);
          img.dispose();
          return data?.buffer.asUint8List();
        });
        expect(pngBytes, isNotNull);
        expect(pngBytes!.length, greaterThan(200));

        // Tap HeaderCard (node 2) -> press event emitted.
        await tester.tapAt(const Offset(50, 25));
        await tester.pump();
        expect(sceneEvents, hasLength(1));
        expect(sceneEvents.last.kind, NveEventKind.press);
        expect(sceneEvents.last.nodeId, 2);

        // Wheel scroll inside ScrollView (node 3) by +100px -> clamped to contentHeight(190)-100=90.
        final pointer = TestPointer(1, PointerDeviceKind.mouse);
        await tester.sendEventToBinding(pointer.hover(const Offset(100, 100)));
        await tester.sendEventToBinding(pointer.scroll(const Offset(0, 100)));
        await tester.pump();

        expect(renderBox.scrollOffsetOf(3).dy, closeTo(90.0, 0.1));
        expect(sceneEvents.last.kind, NveEventKind.scroll);
        expect(sceneEvents.last.nodeId, 3);
        final scrollPayload = sceneEvents.last.payload as NveScrollPayload;
        expect(scrollPayload.y, closeTo(90.0, 0.1));
        expect(scrollPayload.contentHeight, closeTo(190.0, 0.1));

        // Now tap at (100, 110): with scrollY=90, maps to child y=200 -> hits ItemB (node 5)!
        await tester.tapAt(const Offset(100, 110));
        await tester.pump();
        expect(sceneEvents.last.kind, NveEventKind.press);
        expect(sceneEvents.last.nodeId, 5);

        // Frame 2: incremental dirty update ONLY on index 1 (opacity 255, new bg) + remove node 4.
        nodesPtr[1]
          ..opacityU8 = 255
          ..bgRgba = _packRgba(220, 38, 38);
        dirtyPtr[0] = 1;
        final removedPtr = alloc.allocate<ffi.Uint32>(4);
        removedPtr[0] = 4;
        viewPtr.ref
          ..frameId = 2
          ..dirtyCount = 1
          ..removedCount = 1
          ..removedIds = removedPtr;

        renderBox.updateFromSceneView(NveSceneReader.fromPointer(viewPtr));
        await tester.pump();

        expect(renderBox.sceneStore.frameId, 2);
        expect(renderBox.sceneStore.lastDirtyCount, 1);
        expect(renderBox.sceneStore.lastRemovedCount, 1);
        expect(renderBox.sceneStore.node(2)?.opacityU8, 255);
        expect(renderBox.sceneStore.node(2)?.bgRgba, _packRgba(220, 38, 38));
        expect(renderBox.sceneStore.node(4), isNull);
      },
    );
  });

  group('ScrollView wheel & drag scrolling in RenderNveCanvas (retained tree)', () {
    testWidgets('wheel and drag scroll dispatch via direct NveEventBatch and clamp bounds', (
      WidgetTester tester,
    ) async {
      final module = _ScrollTestModule();
      addTearDown(module.close);

      final controller = NveCanvasController.fromInterfaceResolver(
        surfaceId: 1,
        getInterface: module.getInterface,
      );
      addTearDown(controller.dispose);

      await tester.pumpWidget(
        Directionality(
          textDirection: TextDirection.ltr,
          child: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              width: 240,
              height: 120,
              child: NveCanvasView(controller: controller),
            ),
          ),
        ),
      );
      await tester.pump();

      final renderBox = tester.renderObject<RenderNveCanvas>(find.byType(NveCanvasView));

      // Before scroll: node 3 is at y=0..80; node 5 is at y=180..260 (below 120px viewport).
      expect(controller.hitTestNode(const Offset(50, 40)), 3);

      // 1. Mouse wheel scroll down by 60px -> emits direct scroll event!
      final mouse = TestPointer(1, PointerDeviceKind.mouse);
      await tester.sendEventToBinding(mouse.hover(const Offset(100, 60)));
      await tester.sendEventToBinding(mouse.scroll(const Offset(0, 60)));
      await tester.pump();

      expect(renderBox.scrollOffsetOf(2).dy, closeTo(60.0, 0.1));
      expect(module.directEvents, hasLength(1));
      expect(module.directEvents.single.kind, NveEventKind.scroll);
      expect(module.directEvents.single.nodeId, 2);
      expect((module.directEvents.single.payload as NveScrollPayload).y, closeTo(60.0, 0.1));

      // 2. Touch drag upward by 100px -> scrolls further (clamped to contentHeight(260)-120=140)
      //    and must NOT emit a press event!
      final drag = await tester.startGesture(const Offset(100, 100));
      await drag.moveBy(const Offset(0, -10));
      await drag.moveBy(const Offset(0, -90));
      await drag.up();
      await tester.pump();

      expect(renderBox.scrollOffsetOf(2).dy, closeTo(140.0, 0.1));
      expect(module.directEvents.where((e) => e.kind == NveEventKind.press), isEmpty);
      expect(module.directEvents.last.kind, NveEventKind.scroll);
      expect((module.directEvents.last.payload as NveScrollPayload).y, closeTo(140.0, 0.1));

      // 3. Tap at (50, 80) with scrollY=140 -> maps to child y=220 -> hits node 5!
      await tester.tapAt(const Offset(50, 80));
      await tester.pump();
      expect(module.directEvents.last.kind, NveEventKind.press);
      expect(module.directEvents.last.nodeId, 5);
    });
  });
}
