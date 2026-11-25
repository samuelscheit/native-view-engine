import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_canvas.dart';

// ---------------------------------------------------------------------------
// Minimal native FFI test harness (for isolated controller/widget unit tests)
// ---------------------------------------------------------------------------

class _TestHeap {
  static final ffi.DynamicLibrary _libc = Platform.isWindows
      ? ffi.DynamicLibrary.open('ucrtbase.dll')
      : ffi.DynamicLibrary.process();

  static final ffi.Pointer<ffi.Void> Function(int) _malloc = _libc.lookupFunction<
      ffi.Pointer<ffi.Void> Function(ffi.Size), ffi.Pointer<ffi.Void> Function(int)>('malloc');
  static final void Function(ffi.Pointer<ffi.Void>) _free = _libc.lookupFunction<
      ffi.Void Function(ffi.Pointer<ffi.Void>), void Function(ffi.Pointer<ffi.Void>)>('free');

  final List<ffi.Pointer<ffi.Void>> _ptrs = <ffi.Pointer<ffi.Void>>[];

  ffi.Pointer<T> allocate<T extends ffi.NativeType>(int byteCount) {
    final p = _malloc(byteCount == 0 ? 1 : byteCount);
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
  final List<String> propsLog = <String>[];
  final List<String> hotPatchLog = <String>[];
  Uint8List pendingOutbound = Uint8List(0);
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

  int _onDispatch(ffi.Pointer<ffi.Void> _, NveBufferSlice slice) {
    if (slice.ptr == ffi.nullptr || slice.len == 0) return 0;
    final bytes = Uint8List.fromList(slice.ptr.asTypedList(slice.len));
    receivedEvents.addAll(NveEventRingCodec.decode(bytes));
    return 1;
  }

  int _onSetProps(ffi.Pointer<ffi.Void> _, NveBufferSlice slice) {
    final bytes = Uint8List.fromList(slice.ptr.asTypedList(slice.len));
    propsLog.add(utf8.decode(bytes));
    return 1;
  }

  NveBufferSlice _onDrain(ffi.Pointer<ffi.Void> _) {
    final out = pendingOutbound;
    pendingOutbound = Uint8List(0);
    if (out.isEmpty) {
      _scratchSlice.ref
        ..ptr = ffi.nullptr
        ..len = 0;
      return _scratchSlice.ref;
    }
    final ptr = _heap.copy(out);
    _scratchSlice.ref
      ..ptr = ptr
      ..len = out.length;
    return _scratchSlice.ref;
  }

  int _onHotPatch(ffi.Pointer<ffi.Void> _, NveBufferSlice slice) {
    final bytes = Uint8List.fromList(slice.ptr.asTypedList(slice.len));
    hotPatchLog.add(utf8.decode(bytes));
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

String _resolveCounterLibraryPath() {
  final envPath = Platform.environment['NVE_COUNTER_LIB'];
  if (envPath != null && envPath.isNotEmpty && File(envPath).existsSync()) {
    return envPath;
  }
  final libName = Platform.isMacOS
      ? 'libnve_example_counter.dylib'
      : Platform.isWindows
          ? 'nve_example_counter.dll'
          : 'libnve_example_counter.so';
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
    'Could not find $libName. Run `cargo build -p nve-example-counter` first '
    '(searched from ${Directory.current.path}).',
  );
}

void main() {
  // =========================================================================
  // 1. CommandDecoder & NveRetainedTree unit tests
  // =========================================================================
  group('CommandDecoder & NveRetainedTree', () {
    test('decodes all opcodes, custom nodes, raw props, event masks, commands, and ColorRgba order', () {
      final tree = NveRetainedTree();
      final b = NveCommandBufferBuilder(surfaceId: 7, frameId: 42);
      b.createNode(2, NveComponentKind.view);
      b.createNode(3, NveComponentKind.text);
      b.createCustomNode(4, 'RNSVGPath');
      b.setChildren(nveRootNodeId, <int>[2, 4]);
      b.setChildren(2, <int>[3]);
      b.setText(3, 'Hello Impeller + Skia! 🚀');
      b.setLayout(
        2,
        const NveLayoutProps(
          width: NveDimension.points(320),
          height: NveDimension.percent(100),
          minWidth: NveDimension.points(100),
          maxHeight: NveDimension.points(600),
          flexDirection: NveFlexDirection.row,
          justifyContent: NveFlexJustify.spaceBetween,
          alignItems: NveFlexAlign.center,
          flexGrow: 1.5,
          flexShrink: 0.5,
          gap: 12,
          padding: NveEdgeInsets.symmetric(vertical: 8, horizontal: 16),
          margin: NveEdgeInsets.all(4),
        ),
      );
      b.setStyle(
        2,
        const NveStyleProps(
          background: NveColorRgba(15, 23, 42, 240),
          textColor: NveColorRgba.rgb(0x38, 0xBD, 0xF8),
          borderColor: NveColorRgba.rgb(0x25, 0x63, 0xEB),
          borderWidth: 2.0,
          borderRadius: 12.0,
          opacity: 0.95,
          fontSize: 18.0,
          zIndex: 5,
        ),
      );
      b.setRawProp(4, 'd', const NveRawPropValue.stringValue('M0 0 L10 10'));
      b.setRawProp(4, 'fillOpacity', const NveRawPropValue.f64Value(0.75));
      b.setRawProp(4, 'visible', const NveRawPropValue.boolValue(true));
      b.setRawProp(4, 'seed', const NveRawPropValue.i64Value(-99));
      b.setRawProp(4, 'blob', NveRawPropValue.bytesValue(Uint8List.fromList(<int>[1, 2, 3, 4])));
      b.setRawProp(4, 'empty', const NveRawPropValue.nullValue());
      b.setEventMask(2, NveEventMask.press | NveEventMask.layout);
      b.dispatchCommand(2, 'scrollTo', Uint8List.fromList(<int>[0, 1, 2]));
      b.commit(syncMount: true);

      final result = tree.applyCommandBuffer(b.toBytes());
      expect(result.isOk, isTrue);
      expect(result.header!.surfaceId, 7);
      expect(result.header!.frameId, 42);
      expect(result.committed, isTrue);
      expect(result.syncMount, isTrue);
      expect(result.dispatchedCommands, hasLength(1));
      expect(result.dispatchedCommands.single.nodeId, 2);
      expect(result.dispatchedCommands.single.commandName, 'scrollTo');
      expect(result.dispatchedCommands.single.payload, Uint8List.fromList(<int>[0, 1, 2]));

      final view = tree.node(2)!;
      expect(view.kind, NveComponentKind.view);
      expect(view.children, <int>[3]);
      expect(view.eventMask, NveEventMask.press | NveEventMask.layout);
      expect(view.style.background.toFlutterColor(), const Color.fromARGB(240, 15, 23, 42));
      expect(view.style.textColor.toFlutterColor(), const Color(0xFF38BDF8));
      expect(view.style.borderColor.toFlutterColor(), const Color(0xFF2563EB));
      expect(view.style.borderWidth, closeTo(2.0, 1e-4));
      expect(view.style.borderRadius, closeTo(12.0, 1e-4));
      expect(view.style.opacity, closeTo(0.95, 1e-4));
      expect(view.style.fontSize, closeTo(18.0, 1e-4));
      expect(view.style.zIndex, 5);
      expect(view.layout.flexDirection, NveFlexDirection.row);
      expect(view.layout.justifyContent, NveFlexJustify.spaceBetween);
      expect(view.layout.alignItems, NveFlexAlign.center);
      expect(view.layout.gap, closeTo(12.0, 1e-4));

      final text = tree.node(3)!;
      expect(text.text, 'Hello Impeller + Skia! 🚀');
      expect(text.isTextLeaf, isTrue);

      final custom = tree.node(4)!;
      expect(custom.kind, NveComponentKind.custom);
      expect(custom.customName, 'RNSVGPath');
      expect(custom.rawProps['d']!.asString, 'M0 0 L10 10');
      expect(custom.rawProps['fillOpacity']!.asDouble, closeTo(0.75, 1e-9));
      expect(custom.rawProps['visible']!.asBool, isTrue);
      expect(custom.rawProps['seed']!.asInt, -99);
      expect(custom.rawProps['blob']!.asBytes, Uint8List.fromList(<int>[1, 2, 3, 4]));
      expect(custom.rawProps['empty']!.kind, NveRawPropKind.nullValue);

      // RemoveNode detaches from parent cleanly.
      final rm = NveCommandBufferBuilder(frameId: 43)
        ..removeNode(4)
        ..commit();
      expect(tree.applyCommandBuffer(rm.toBytes()).isOk, isTrue);
      expect(tree.node(4), isNull);
      expect(tree.root.children, <int>[2]);
    });

    test('propagates Text parent->child style inheritance and defaults to opaque black 16pt', () {
      final tree = NveRetainedTree();
      final b = NveCommandBufferBuilder()
        ..createNode(2, NveComponentKind.text) // styled wrapper
        ..createNode(3, NveComponentKind.text) // inherits from 2
        ..createNode(4, NveComponentKind.text) // explicit override
        ..createNode(5, NveComponentKind.text) // top-level unstyled -> default black 16
        ..setStyle(
          2,
          const NveStyleProps(
            textColor: NveColorRgba.rgb(0x38, 0xBD, 0xF8),
            fontSize: 28.0,
          ),
        )
        ..setStyle(
          4,
          const NveStyleProps(
            textColor: NveColorRgba.rgb(0xFF, 0x00, 0xAA),
            fontSize: 12.0,
          ),
        )
        ..setText(3, 'Inherited')
        ..setText(4, 'Explicit')
        ..setText(5, 'Default')
        ..setChildren(nveRootNodeId, <int>[2, 5])
        ..setChildren(2, <int>[3, 4])
        ..commit();

      expect(tree.applyCommandBuffer(b.toBytes()).isOk, isTrue);
      expect(tree.node(2)!.isTextLeaf, isFalse);
      expect(tree.node(3)!.isTextLeaf, isTrue);
      expect(tree.node(3)!.style.textColor, const NveColorRgba.rgb(0x38, 0xBD, 0xF8));
      expect(tree.node(3)!.style.fontSize, closeTo(28.0, 1e-4));
      expect(tree.node(4)!.style.textColor, const NveColorRgba.rgb(0xFF, 0x00, 0xAA));
      expect(tree.node(4)!.style.fontSize, closeTo(12.0, 1e-4));
      expect(tree.node(5)!.style.textColor, NveColorRgba.black);
      expect(tree.node(5)!.style.textColor.toFlutterColor(), const Color(0xFF000000));
      expect(tree.node(5)!.style.fontSize, closeTo(16.0, 1e-4));
    });

    test('rejects corrupt, truncated, bad-magic, bad-version, invalid-UTF8, and trailing buffers safely', () {
      final tree = NveRetainedTree();
      // Seed a valid node so we verify atomic non-mutation on decode errors.
      final seed = NveCommandBufferBuilder()
        ..createNode(10, NveComponentKind.view)
        ..setChildren(nveRootNodeId, <int>[10])
        ..commit();
      expect(tree.applyCommandBuffer(seed.toBytes()).isOk, isTrue);

      // Truncated header
      final rShort = tree.applyCommandBuffer(Uint8List(8));
      expect(rShort.isOk, isFalse);
      expect(rShort.error!.code, NveDecodeErrorCode.truncated);

      // Bad magic
      final badMagic = (NveCommandBufferBuilder()..commit()).toBytes();
      badMagic[0] ^= 0xFF;
      expect(tree.applyCommandBuffer(badMagic).error!.code, NveDecodeErrorCode.invalidMagic);

      // Bad version
      final badVer = (NveCommandBufferBuilder()..commit()).toBytes();
      ByteData.sublistView(badVer).setUint16(4, 99, Endian.little);
      expect(tree.applyCommandBuffer(badVer).error!.code, NveDecodeErrorCode.unsupportedVersion);

      // Unknown opcode
      final badOp = (NveCommandBufferBuilder()..removeNode(10)).toBytes();
      badOp[nveHeaderSize] = 250;
      expect(tree.applyCommandBuffer(badOp).error!.code, NveDecodeErrorCode.unknownOpcode);

      // Truncated mid-operation (sweep every prefix length!)
      final full = (NveCommandBufferBuilder()
            ..createNode(20, NveComponentKind.text)
            ..setText(20, 'Truncation sweep test')
            ..setLayout(20, const NveLayoutProps(width: NveDimension.points(100)))
            ..setStyle(20, const NveStyleProps(fontSize: 20))
            ..commit())
          .toBytes();
      for (var len = 0; len < full.length; len++) {
        final slice = Uint8List.sublistView(full, 0, len);
        final res = tree.applyCommandBuffer(slice);
        expect(res.isOk, isFalse, reason: 'prefix length $len must fail cleanly');
      }

      // Invalid UTF-8 in SetText
      final badUtf8 = (NveCommandBufferBuilder()
            ..createNode(21, NveComponentKind.text)
            ..setText(21, 'AB'))
          .toBytes();
      badUtf8[badUtf8.length - 1] = 0xFF;
      expect(tree.applyCommandBuffer(badUtf8).error!.code, NveDecodeErrorCode.invalidUtf8);

      // Trailing garbage bytes
      final withTrailing = Uint8List(full.length + 3)..setAll(0, full);
      expect(tree.applyCommandBuffer(withTrailing).error!.code, NveDecodeErrorCode.trailingBytes);

      // Inflated op_count
      final hugeOps = (NveCommandBufferBuilder()..commit()).toBytes();
      ByteData.sublistView(hugeOps).setUint32(20, 9999999, Endian.little);
      expect(tree.applyCommandBuffer(hugeOps).error!.code, NveDecodeErrorCode.bufferTooLarge);

      // Seeded tree stayed untouched across every bad buffer!
      expect(tree.nodeCount, 2);
      expect(tree.node(10), isNotNull);
      expect(tree.node(20), isNull);
    });

    test('guards SetChildren against self-parent, root-as-child, duplicates, cycles, and RemoveNode(root)', () {
      final tree = NveRetainedTree();
      final setup = NveCommandBufferBuilder()
        ..createNode(2, NveComponentKind.view)
        ..createNode(3, NveComponentKind.view)
        ..createNode(4, NveComponentKind.view)
        ..setChildren(nveRootNodeId, <int>[2])
        ..setChildren(2, <int>[3])
        ..setChildren(3, <int>[4])
        ..commit();
      expect(tree.applyCommandBuffer(setup.toBytes()).isOk, isTrue);

      // Self-parent
      final selfCycle = NveCommandBufferBuilder()..setChildren(2, <int>[2]);
      expect(
        tree.applyCommandBuffer(selfCycle.toBytes()).error!.code,
        NveDecodeErrorCode.cycleOrDuplicateChild,
      );

      // Root as child
      final rootChild = NveCommandBufferBuilder()..setChildren(2, <int>[nveRootNodeId]);
      expect(
        tree.applyCommandBuffer(rootChild.toBytes()).error!.code,
        NveDecodeErrorCode.cycleOrDuplicateChild,
      );

      // Duplicate child
      final dupChild = NveCommandBufferBuilder()..setChildren(2, <int>[3, 3]);
      expect(
        tree.applyCommandBuffer(dupChild.toBytes()).error!.code,
        NveDecodeErrorCode.cycleOrDuplicateChild,
      );

      // Multi-hop cycle (4 -> 2 where 2 -> 3 -> 4)
      final multiCycle = NveCommandBufferBuilder()..setChildren(4, <int>[2]);
      expect(
        tree.applyCommandBuffer(multiCycle.toBytes()).error!.code,
        NveDecodeErrorCode.cycleOrDuplicateChild,
      );

      // Remove root
      final rmRoot = NveCommandBufferBuilder()..removeNode(nveRootNodeId);
      expect(
        tree.applyCommandBuffer(rmRoot.toBytes()).error!.code,
        NveDecodeErrorCode.cannotRemoveRoot,
      );
    });
  });

  // =========================================================================
  // 2. FlexLayout solver unit tests
  // =========================================================================
  group('NveFlexLayoutSolver', () {
    test('solves column/row, padding/margin/gap, justify center & space-between, align center, flexGrow', () {
      final tree = NveRetainedTree();
      final b = NveCommandBufferBuilder()
        ..createNode(2, NveComponentKind.view) // outer column
        ..createNode(3, NveComponentKind.view) // centered box
        ..createNode(4, NveComponentKind.view) // space-between row
        ..createNode(5, NveComponentKind.view) // row item A
        ..createNode(6, NveComponentKind.view) // row item B
        ..createNode(7, NveComponentKind.view) // flexGrow row
        ..createNode(8, NveComponentKind.view) // grow 1
        ..createNode(9, NveComponentKind.view) // grow 3
        ..setChildren(nveRootNodeId, <int>[2])
        ..setChildren(2, <int>[3, 4, 7])
        ..setChildren(4, <int>[5, 6])
        ..setChildren(7, <int>[8, 9])
        ..setLayout(
          2,
          const NveLayoutProps(
            width: NveDimension.percent(100),
            height: NveDimension.percent(100),
            padding: NveEdgeInsets.all(20),
            gap: 10,
            alignItems: NveFlexAlign.center,
          ),
        )
        ..setLayout(
          3,
          const NveLayoutProps(
            width: NveDimension.points(120),
            height: NveDimension.points(40),
          ),
        )
        ..setLayout(
          4,
          const NveLayoutProps(
            width: NveDimension.percent(100),
            height: NveDimension.points(50),
            flexDirection: NveFlexDirection.row,
            justifyContent: NveFlexJustify.spaceBetween,
            alignItems: NveFlexAlign.center,
          ),
        )
        ..setLayout(
          5,
          const NveLayoutProps(width: NveDimension.points(60), height: NveDimension.points(20)),
        )
        ..setLayout(
          6,
          const NveLayoutProps(width: NveDimension.points(80), height: NveDimension.points(30)),
        )
        ..setLayout(
          7,
          const NveLayoutProps(
            width: NveDimension.percent(100),
            height: NveDimension.points(40),
            flexDirection: NveFlexDirection.row,
            gap: 16,
          ),
        )
        ..setLayout(
          8,
          const NveLayoutProps(flexGrow: 1.0, height: NveDimension.points(40)),
        )
        ..setLayout(
          9,
          const NveLayoutProps(flexGrow: 3.0, height: NveDimension.points(40)),
        )
        ..commit();

      expect(tree.applyCommandBuffer(b.toBytes()).isOk, isTrue);
      final solver = NveFlexLayoutSolver();
      solver.layout(tree, const Size(400, 300));

      // Inner width of node 2 is 400 - 40 = 360, starts at x=20, y=20.
      final r3 = tree.layoutOf(3)!;
      expect(r3.left, closeTo(20 + (360 - 120) / 2, 0.5));
      expect(r3.top, closeTo(20, 0.5));
      expect(r3.width, closeTo(120, 0.5));
      expect(r3.height, closeTo(40, 0.5));

      // Row 4 starts at y = 20 + 40 + 10 = 70, width = 360.
      final r5 = tree.layoutOf(5)!;
      final r6 = tree.layoutOf(6)!;
      expect(r5.left, closeTo(20, 0.5));
      expect(r5.top, closeTo(70 + (50 - 20) / 2, 0.5));
      expect(r6.right, closeTo(380, 0.5));
      expect(r6.top, closeTo(70 + (50 - 30) / 2, 0.5));

      // Row 7 has width 360, gap 16 -> 344 distributed 1:3 => 86 and 258.
      final r8 = tree.layoutOf(8)!;
      final r9 = tree.layoutOf(9)!;
      expect(r8.width, closeTo(86, 0.5));
      expect(r9.width, closeTo(258, 0.5));
      expect(r9.left - r8.right, closeTo(16, 0.5));
    });

    test('re-measures wrapping text height after flexShrink narrows a row child', () {
      // Deterministic custom measurer: 10px per char, 20px line height, wraps cleanly.
      Size deterministicMeasurer(String text, double fontSize, double maxWidth) {
        final singleW = text.length * 10.0;
        if (!maxWidth.isFinite || maxWidth <= 0.0 || singleW <= maxWidth) {
          return Size(singleW, 20.0);
        }
        final lines = (singleW / maxWidth).ceil();
        return Size(maxWidth, lines * 20.0);
      }

      final tree = NveRetainedTree();
      final b = NveCommandBufferBuilder()
        ..createNode(2, NveComponentKind.view) // row width 200
        ..createNode(3, NveComponentKind.view) // fixed 120px sibling
        ..createNode(4, NveComponentKind.text) // 16 chars = 160px intrinsic -> shrinks to 80px -> 2 lines (40px)!
        ..setChildren(nveRootNodeId, <int>[2])
        ..setChildren(2, <int>[3, 4])
        ..setLayout(
          2,
          const NveLayoutProps(
            width: NveDimension.points(200),
            height: NveDimension.points(100),
            flexDirection: NveFlexDirection.row,
            alignItems: NveFlexAlign.flexStart,
          ),
        )
        ..setLayout(
          3,
          const NveLayoutProps(
            width: NveDimension.points(120),
            height: NveDimension.points(30),
            flexShrink: 0.0,
          ),
        )
        ..setLayout(
          4,
          const NveLayoutProps(flexShrink: 1.0),
        )
        ..setText(4, '0123456789abcdef')
        ..commit();

      expect(tree.applyCommandBuffer(b.toBytes()).isOk, isTrue);
      final solver = NveFlexLayoutSolver(textMeasurer: deterministicMeasurer);
      solver.layout(tree, const Size(200, 100));

      final r4 = tree.layoutOf(4)!;
      expect(r4.width, closeTo(80.0, 0.5));
      expect(r4.height, closeTo(40.0, 0.5));
    });

    test('wraps text accurately using real Flutter TextPainter', () {
      final singleLine = flutterTextMeasurer('Hello Flutter Canvas Layout', 16.0, double.infinity);
      final wrapped = flutterTextMeasurer('Hello Flutter Canvas Layout', 16.0, singleLine.width * 0.45);
      expect(singleLine.width, greaterThan(40));
      expect(singleLine.height, greaterThan(10));
      expect(wrapped.width, lessThanOrEqualTo(singleLine.width * 0.45 + 1.0));
      expect(wrapped.height, greaterThan(singleLine.height));
    });
  });

  // =========================================================================
  // 3. Binary Press Event Encoder & Controller/Widget Unit Tests
  // =========================================================================
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
      expect(ev.payloadJson!['x'], closeTo(123.5, 1e-3));
      expect(ev.payloadJson!['y'], closeTo(67.25, 1e-3));
    });

    testWidgets('zIndex reverse hit-testing, opacity painting, props, hot-patch, and atomic reload',
        (WidgetTester tester) async {
      Uint8List buildZScene(int frameId, String label) {
        final b = NveCommandBufferBuilder(surfaceId: 1, frameId: frameId)
          ..createNode(2, NveComponentKind.view)
          ..createNode(3, NveComponentKind.pressable) // lower zIndex
          ..createNode(4, NveComponentKind.pressable) // higher zIndex, overlaps node 3
          ..createNode(5, NveComponentKind.text)
          ..setChildren(nveRootNodeId, <int>[2])
          ..setChildren(2, <int>[4, 3]) // node 4 earlier in tree, but zIndex=10 > zIndex=1
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
              margin: NveEdgeInsets(top: -60), // overlaps node 4 directly
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

      // Tap overlap area -> higher zIndex node (4) wins over later-in-tree node (3).
      final hitCenter = renderBox.layoutOf(4)!.center;
      await tester.tapAt(hitCenter);
      await tester.pump();
      expect(app1.receivedEvents, hasLength(1));
      expect(app1.receivedEvents.single.nodeId, 4);

      // Dragging outside the hit node before PointerUp cancels the press.
      final dragGesture = await tester.startGesture(hitCenter);
      await dragGesture.moveTo(const Offset(260, 180));
      await dragGesture.up();
      await tester.pump();
      expect(app1.receivedEvents, hasLength(1));

      // Props, hot-patch, outbound event draining
      app1.pendingOutbound = NveEventRingCodec.encodeCustomEvents(<NveEvent>[
        const NveEvent(
          surfaceId: 1,
          nodeId: 4,
          timestampUs: 100,
          kind: NveEventKind.custom,
          name: 'onPing',
          payloadJson: <String, dynamic>{'ok': true},
        ),
      ]);
      expect(controller.setPropsJson(<String, dynamic>{'theme': 'dark'}), isTrue);
      expect(controller.applyHotPatch('{"op":"patch"}'), isTrue);
      expect(app1.propsLog, hasLength(1));
      expect(app1.hotPatchLog, hasLength(1));
      expect(outbound.map((e) => e.name), contains('onPing'));

      // State-preserving atomic reload into app2
      app1.stateText = 'preserved-42';
      final reloadRes = controller.reloadWithInterfaceResolver(app2.getInterface);
      expect(reloadRes.isOk, isTrue);
      expect(app1.destroyCount, 1);
      expect(app2.stateText, 'preserved-42');
      expect(controller.debugNodes[5]!.text, 'State: preserved-42');
    });
  });

  // =========================================================================
  // 4. Live real Rust cdylib (`libnve_example_counter.dylib`) integration test
  // =========================================================================
  group('Live Rust nve-example-counter cdylib on NveCanvasView', () {
    testWidgets(
      'renders counter card, handles real taps, preserves state across reloadLibrary, resets, and rasterizes PNG',
      (WidgetTester tester) async {
        final libPath = _resolveCounterLibraryPath();

        // Load a real proportional system font when available so text renders with crisp glyphs.
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

        // Ensure deterministic starting state (`Count: 0`) even across repeated runs.
        controller.setPropsJson(<String, dynamic>{'initialCount': 0}, rerender: false);

        final boundaryKey = GlobalKey();
        await tester.pumpWidget(
          Directionality(
            textDirection: TextDirection.ltr,
            child: Align(
              alignment: Alignment.topLeft,
              child: RepaintBoundary(
                key: boundaryKey,
                child: SizedBox(
                  width: 480,
                  height: 360,
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

        // 1. Assert top dark slate card (#1e293b) geometry: width > 200, height > 60
        final cardNode = renderBox.debugNodes.values.firstWhere(
          (n) => n.style.background == const NveColorRgba.rgb(0x1E, 0x29, 0x3B),
        );
        final cardRect = renderBox.layoutOf(cardNode.id)!;
        expect(cardRect.width, greaterThan(200));
        expect(cardRect.height, greaterThan(60));

        // 2. Assert "NATIVE VIEW ENGINE" and "Count: 0" at 28px inside the card above buttons
        final titleNode = findLeafText('NATIVE VIEW ENGINE');
        final countNode = findLeafText('Count: 0');
        final incButton = findPressableForLabel('Increment +');
        final resetButton = findPressableForLabel('Reset');
        final incText = findLeafText('Increment +');
        final resetText = findLeafText('Reset');

        expect(titleNode.style.fontSize, closeTo(14.0, 0.1));
        expect(titleNode.style.textColor, const NveColorRgba.rgb(0x38, 0xBD, 0xF8));
        expect(countNode.style.fontSize, closeTo(28.0, 0.1));
        expect(countNode.style.textColor, NveColorRgba.white);

        final countRect = renderBox.layoutOf(countNode.id)!;
        final incBtnRect = renderBox.layoutOf(incButton.id)!;
        final resetBtnRect = renderBox.layoutOf(resetButton.id)!;
        final incTextRect = renderBox.layoutOf(incText.id)!;
        final resetTextRect = renderBox.layoutOf(resetText.id)!;

        // "Count: 0" lies inside the dark slate card and above the button row.
        expect(cardRect.contains(countRect.center), isTrue);
        expect(countRect.bottom, lessThan(incBtnRect.top));

        // "Increment +" and "Reset" centered inside their buttons (<= 4px tolerance).
        expect((incTextRect.center.dx - incBtnRect.center.dx).abs(), lessThanOrEqualTo(4.0));
        expect((incTextRect.center.dy - incBtnRect.center.dy).abs(), lessThanOrEqualTo(4.0));
        expect((resetTextRect.center.dx - resetBtnRect.center.dx).abs(), lessThanOrEqualTo(4.0));
        expect((resetTextRect.center.dy - resetBtnRect.center.dy).abs(), lessThanOrEqualTo(4.0));

        // 3. Real tapAt on "Increment +" -> "Count: 1" -> "Count: 2"
        final origin = renderBox.localToGlobal(Offset.zero);
        await tester.tapAt(origin + incBtnRect.center);
        await tester.pump();
        expect(findLeafText('Count: 1').style.fontSize, closeTo(28.0, 0.1));

        await tester.tapAt(origin + incBtnRect.center);
        await tester.pump();
        expect(findLeafText('Count: 2').style.fontSize, closeTo(28.0, 0.1));

        // Optional offscreen PNG capture when NVE_SAVE_VERIFY_PNG is set (also always verifies toImage()).
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

        // 4. State-preserving reloadLibrary(libPath) keeps "Count: 2"!
        final reloadResult = controller.reloadLibrary(libPath);
        await tester.pump();
        expect(reloadResult.isOk, isTrue);
        expect(findLeafText('Count: 2').text, 'Count: 2');

        // 5. Real tapAt on "Reset" resets back to "Count: 0"!
        final resetBtnAfterReload = findPressableForLabel('Reset');
        final resetRectAfterReload = renderBox.layoutOf(resetBtnAfterReload.id)!;
        await tester.tapAt(origin + resetRectAfterReload.center);
        await tester.pump();
        expect(findLeafText('Count: 0').text, 'Count: 0');
      },
    );
  });
}
