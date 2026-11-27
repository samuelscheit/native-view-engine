import 'dart:typed_data';
import 'dart:ui' show Tristate;

import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

import 'src/test_support.dart';

const _white = (255, 255, 255, 255);
const _red = (255, 0, 0, 255);
const _blue = (0, 0, 255, 255);
const _green = (0, 255, 0, 255);

void _fill(SceneBuilder b, int node, int color) => b.bits(visualShape, node, NveVisualTable.background, color);

Future<Uint8List> _paint(SceneBuilder b, {NveScenePainter? painter, Offset origin = Offset.zero}) {
  final p = painter ?? NveScenePainter();
  return rasterize(100, 100, (canvas) => p.paint(canvas, b.tree(), origin));
}

void main() {
  group('NveScenePainter', () {
    test('fills, scroll shift, overflow clip, z-order, display none, and surface origin', () async {
      final b = SceneBuilder();
      final root = b.node(const Rect.fromLTWH(0, 0, 100, 100));
      final scroller = b.node(const Rect.fromLTWH(50, 0, 50, 50), parent: root);
      final content = b.node(const Rect.fromLTWH(50, 40, 50, 20), parent: scroller);
      final low = b.node(const Rect.fromLTWH(0, 60, 40, 40), parent: root);
      final high = b.node(const Rect.fromLTWH(0, 60, 40, 40), parent: root);
      final gone = b.node(const Rect.fromLTWH(0, 0, 20, 20), parent: root);
      _fill(b, root, packRgba(255, 255, 255));
      _fill(b, content, packRgba(0, 0, 255));
      _fill(b, low, packRgba(0, 255, 0));
      _fill(b, high, packRgba(255, 0, 0));
      _fill(b, gone, packRgba(255, 0, 0));
      b
        ..bits(visualShape, scroller, NveVisualTable.overflowY, NveOverflow.hidden)
        ..scrollOffset(scroller, 0, 30)
        ..bits(visualShape, low, NveVisualTable.zIndex, 1)
        ..bits(layoutShape, gone, NveLayoutTable.display, NveDisplay.none);
      final px = await _paint(b);
      expect(pixelAt(px, 100, 75, 15), _blue, reason: 'content painted at y - scroll (40 - 30)');
      expect(pixelAt(px, 100, 75, 35), _white, reason: 'content no longer at 30..40');
      expect(pixelAt(px, 100, 75, 55), _white, reason: 'pre-scroll position stays empty');
      expect(pixelAt(px, 100, 20, 80), _green, reason: 'z-index 1 paints over later siblings');
      expect(pixelAt(px, 100, 10, 10), _white, reason: 'display:none is skipped');

      // Without overflow clip and scroll, the content would spill to y = 40..60.
      final shifted = await _paint(b, origin: const Offset(-50, 0));
      expect(pixelAt(shifted, 100, 25, 15), _blue, reason: 'surface origin offsets the whole scene');
    });

    test('opacity, visibility inheritance, border, and transform', () async {
      final b = SceneBuilder();
      final root = b.node(const Rect.fromLTWH(0, 0, 100, 100));
      final faded = b.node(const Rect.fromLTWH(0, 0, 20, 20), parent: root);
      final hiddenParent = b.node(const Rect.fromLTWH(20, 0, 20, 20), parent: root);
      final hiddenChild = b.node(const Rect.fromLTWH(20, 0, 20, 20), parent: hiddenParent);
      final shownChild = b.node(const Rect.fromLTWH(40, 0, 20, 20), parent: hiddenParent);
      final bordered = b.node(const Rect.fromLTWH(0, 40, 40, 40), parent: root);
      final moved = b.node(const Rect.fromLTWH(0, 0, 10, 10), parent: root);
      final invisible = b.node(const Rect.fromLTWH(60, 0, 20, 20), parent: root);
      _fill(b, root, packRgba(255, 255, 255));
      _fill(b, faded, packRgba(0, 0, 0));
      _fill(b, hiddenParent, packRgba(255, 0, 0));
      _fill(b, hiddenChild, packRgba(255, 0, 0));
      _fill(b, shownChild, packRgba(0, 255, 0));
      _fill(b, moved, packRgba(0, 0, 255));
      _fill(b, invisible, packRgba(255, 0, 0));
      b
        ..f32(visualShape, faded, NveVisualTable.opacity, 0.5)
        ..bits(visualShape, hiddenParent, NveVisualTable.visibility, NveVisibility.hidden)
        ..bits(visualShape, shownChild, NveVisualTable.visibility, NveVisibility.visible)
        ..f32(visualShape, bordered, NveVisualTable.borderWidth, 4)
        ..bits(visualShape, bordered, NveVisualTable.borderColor, packRgba(255, 0, 0))
        ..f32(transformShape, moved, NveTransformTable.translateX, 80)
        ..f32(transformShape, moved, NveTransformTable.translateY, 80)
        ..f32(visualShape, invisible, NveVisualTable.opacity, 0);
      final px = await _paint(b);
      final (r, g, bl, _) = pixelAt(px, 100, 10, 10);
      expect([r, g, bl].every((c) => c > 100 && c < 155), isTrue, reason: '50% black over white ($r,$g,$bl)');
      expect(pixelAt(px, 100, 30, 10), _white, reason: 'hidden parent and inheriting child');
      expect(pixelAt(px, 100, 50, 10), _green, reason: 'visible child of a hidden parent');
      expect(pixelAt(px, 100, 1, 60), _red, reason: 'border stroke');
      expect(pixelAt(px, 100, 20, 60), _white, reason: 'border interior');
      expect(pixelAt(px, 100, 85, 85), _blue, reason: 'translated');
      expect(pixelAt(px, 100, 5, 5), isNot(_blue), reason: 'not at its layout origin');
      expect(pixelAt(px, 100, 70, 10), _white, reason: 'opacity 0 skips the subtree');
    });

    testWidgets('text and text-input placeholder use inherited typography', (tester) async {
      final b = SceneBuilder();
      final root = b.node(const Rect.fromLTWH(0, 0, 100, 100));
      final label = b.node(const Rect.fromLTWH(0, 0, 100, 20), parent: root);
      final input = b.node(const Rect.fromLTWH(0, 50, 100, 40), parent: root);
      _fill(b, root, packRgba(255, 255, 255));
      b
        ..bits(typographyShape, root, NveTypographyTable.set, NveTypographyTable.setColor | NveTypographyTable.setFontSize)
        ..bits(typographyShape, root, NveTypographyTable.color, packRgba(255, 0, 0))
        ..f32(typographyShape, root, NveTypographyTable.fontSize, 10)
        ..str(textShape, label, NveTextTable.text, 'MM')
        ..str(textInputShape, input, NveTextInputTable.placeholder, 'Search');
      final px = (await tester.runAsync(() => _paint(b)))!;
      expect(pixelAt(px, 100, 3, 5), _red, reason: 'Ahem glyph box in the inherited color');
      expect(pixelAt(px, 100, 40, 5), _white, reason: 'two 10pt glyphs end at x = 20');
      // Vertically centered: top = (40 - 10 * 1.35) / 2 = 13.25 → glyph rows 63..73.
      expect(pixelAt(px, 100, 3, 58), _white);
      final (r, g, bl, _) = pixelAt(px, 100, 3, 68);
      expect((r, g, bl) != (255, 255, 255) && bl > r, isTrue, reason: 'placeholder slate color ($r,$g,$bl)');
    });

    test('canvas table painter draws the draw-list payload from scene resources', () async {
      final b = SceneBuilder();
      final root = b.node(const Rect.fromLTWH(0, 0, 100, 100));
      final canvasNode = b.node(const Rect.fromLTWH(10, 10, 50, 50), parent: root);
      final canvasShape = TableShape(NveCanvasTablePainter.key, 2, true, const [0, 0]);
      b.bits(canvasShape, canvasNode, NveCanvasTablePainter.drawList, 7);
      final payload = (DrawListEncoder()..fillRect(0, 0, 50, 50, packRgba(255, 0, 0))).finish();
      final px = await rasterize(100, 100, (canvas) {
        NveScenePainter().paint(canvas, b.tree(), Offset.zero, resources: _Resources({7: payload}));
      });
      expect(pixelAt(px, 100, 30, 30), _red);
      expect(pixelAt(px, 100, 70, 70).$4, 0, reason: 'outside the canvas node');
      final none = await rasterize(100, 100, (canvas) => NveScenePainter().paint(canvas, b.tree(), Offset.zero));
      expect(pixelAt(none, 100, 30, 30).$4, 0, reason: 'no resource view: nothing painted');
    });

    test('image source payloads decode to URIs', () {
      Uint8List payload(int tag, List<int> bytes) => Uint8List.fromList([
            tag,
            ...(ByteData(4)..setUint32(0, bytes.length, Endian.little)).buffer.asUint8List(),
            ...bytes,
          ]);
      expect(decodeImageSourceUri(payload(0, 'https://x/a.png'.codeUnits)), 'https://x/a.png');
      expect(decodeImageSourceUri(payload(1, [1, 2, 3])), 'data:application/octet-stream;base64,AQID');
      expect(decodeImageSourceUri(payload(2, [1])), isNull, reason: 'unknown tag');
      expect(decodeImageSourceUri(payload(0, [0xC3])), isNull, reason: 'invalid UTF-8');
      expect(decodeImageSourceUri(Uint8List.fromList([0, 9, 0, 0, 0, 1])), isNull, reason: 'truncated');
      expect(decodeImageSourceUri(Uint8List.fromList([...payload(0, [0x61]), 0])), isNull, reason: 'trailing bytes');
      expect(decodeImageSourceUri(Uint8List(0)), isNull);
    });
  });

  group('NveCanvasSemanticsAssembler', () {
    test('roles, labels, states, clipping, hidden subtrees, and tap routing', () {
      final b = SceneBuilder();
      final root = b.node(const Rect.fromLTWH(0, 0, 200, 200));
      final button = b.node(const Rect.fromLTWH(0, 0, 100, 40), parent: root);
      final heading = b.node(const Rect.fromLTWH(0, 40, 100, 40), parent: root);
      final password = b.node(const Rect.fromLTWH(0, 80, 100, 40), parent: root);
      final hidden = b.node(const Rect.fromLTWH(100, 0, 100, 40), parent: root);
      final hiddenKid = b.node(const Rect.fromLTWH(100, 0, 100, 40), parent: hidden);
      final clipper = b.node(const Rect.fromLTWH(100, 100, 50, 50), parent: root);
      final outside = b.node(const Rect.fromLTWH(100, 160, 50, 20), parent: clipper);
      final toggle = b.node(const Rect.fromLTWH(100, 40, 50, 20), parent: root);
      final decorative = b.node(const Rect.fromLTWH(150, 40, 50, 20), parent: root);
      b
        ..bits(interactionShape, button, NveInteractionTable.eventMask, nveEventMaskPress)
        ..str(textShape, button, NveTextTable.text, 'Save')
        ..bits(a11yShape, heading, NveAccessibilityTable.role, NveAriaRole.heading)
        ..str(textShape, heading, NveTextTable.text, 'Settings')
        ..str(textInputShape, password, NveTextInputTable.value, 'héllo')
        ..bits(textInputShape, password, NveTextInputTable.secure, 1)
        ..str(textInputShape, password, NveTextInputTable.placeholder, 'Password')
        ..bits(a11yShape, hidden, NveAccessibilityTable.hidden, 1)
        ..str(textShape, hiddenKid, NveTextTable.text, 'secret')
        ..bits(visualShape, clipper, NveVisualTable.overflowX, NveOverflow.hidden)
        ..str(textShape, outside, NveTextTable.text, 'clipped')
        ..bits(a11yShape, toggle, NveAccessibilityTable.role, NveAriaRole.switchValue)
        ..bits(a11yShape, toggle, NveAccessibilityTable.checked, NveAriaTriState.trueValue)
        ..str(a11yShape, toggle, NveAccessibilityTable.label, 'Wi-Fi')
        ..bits(a11yShape, decorative, NveAccessibilityTable.role, NveAriaRole.presentation)
        ..str(textShape, decorative, NveTextTable.text, 'ornament');
      final taps = <(int, Offset)>[];
      final asm = NveCanvasSemanticsAssembler();
      final nodes = asm.build(b.tree(), const Rect.fromLTWH(0, 0, 200, 200), (id, c) => taps.add((id, c)));
      final data = {for (final n in nodes) n.getSemanticsData().label: n.getSemanticsData()};
      expect(data.keys, ['Save', 'Settings', 'Password', 'Wi-Fi']);
      expect(data['Save']!.flagsCollection.isButton, isTrue);
      expect(data['Save']!.rect, const Rect.fromLTWH(0, 0, 100, 40));
      expect(data['Settings']!.flagsCollection.isHeader, isTrue);
      expect(data['Settings']!.hasAction(SemanticsAction.tap), isFalse);
      expect(data['Password']!.flagsCollection.isTextField, isTrue);
      expect(data['Password']!.value, '•••••', reason: 'secure value masked per code point');
      expect(data['Wi-Fi']!.flagsCollection.isToggled, Tristate.isTrue);
      final saveNode = nodes.first;
      expect(saveNode.getSemanticsData().hasAction(SemanticsAction.tap), isTrue);
      asm.build(b.tree(), const Rect.fromLTWH(0, 0, 200, 200), (id, c) => taps.add((id, c)));
      final again = asm.build(b.tree(), const Rect.fromLTWH(0, 0, 200, 200), (id, c) => taps.add((id, c)));
      expect(identical(again.first, saveNode), isTrue, reason: 'stable SemanticsNode per NodeId');
      expect(taps, isEmpty);
      expect(asm.build(NveSceneTree(SceneBuilder().build()), Rect.zero, (_, __) {}), isEmpty);
    });
  });
}

final class _Resources implements NveSceneResources {
  _Resources(this.map);

  final Map<int, Uint8List> map;

  @override
  Uint8List? payload(int resourceId) => map[resourceId];
}
