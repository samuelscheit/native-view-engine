import 'dart:typed_data';
import 'dart:ui' show Color, Size;

import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_canvas.dart';

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
}
