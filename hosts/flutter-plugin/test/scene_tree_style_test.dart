import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

import 'src/test_support.dart';

void main() {
  group('NveSceneTree', () {
    late SceneBuilder b;
    late int root, scroller, inner, deep, sibling, hiddenNode;

    setUp(() {
      b = SceneBuilder();
      root = b.node(const Rect.fromLTWH(0, 0, 200, 200));
      scroller = b.node(const Rect.fromLTWH(0, 0, 100, 100), parent: root);
      inner = b.node(const Rect.fromLTWH(0, 120, 100, 50), parent: scroller);
      deep = b.node(const Rect.fromLTWH(10, 130, 20, 20), parent: inner);
      sibling = b.node(const Rect.fromLTWH(100, 0, 100, 100), parent: root);
      hiddenNode = b.node(const Rect.fromLTWH(100, 0, 100, 100), parent: root);
      b
        ..attach(scrollShape, scroller)
        ..bits(visualShape, scroller, NveVisualTable.overflowY, NveOverflow.scroll)
        ..scrollOffset(scroller, 0, 100)
        ..bits(layoutShape, hiddenNode, NveLayoutTable.display, NveDisplay.none)
        ..bits(interactionShape, deep, NveInteractionTable.eventMask, nveEventMaskPress)
        ..bits(interactionShape, sibling, NveInteractionTable.eventMask, nveEventMaskPress);
    });

    test('visual rects subtract every ancestor scroll shift; published rects stay pre-scroll', () {
      final t = b.tree();
      expect(t.root, root);
      expect(t.rectOf(deep), const Rect.fromLTWH(10, 130, 20, 20));
      expect(t.scrollShift(scroller), const Offset(0, 100));
      expect(t.visualRect(inner), const Rect.fromLTWH(0, 20, 100, 50));
      expect(t.visualRect(deep), const Rect.fromLTWH(10, 30, 20, 20));
      expect(t.visualRect(scroller), const Rect.fromLTWH(0, 0, 100, 100));
    });

    test('display none, clipping, scroll containers, and content size', () {
      final t = b.tree();
      expect(t.isDisplayNone(hiddenNode), isTrue);
      expect(t.isDisplayNone(sibling), isFalse);
      expect(t.clipsChildren(scroller), isTrue);
      expect(t.clipsChildren(root), isFalse);
      expect(t.isScrollContainer(scroller), isTrue);
      expect(t.isScrollContainer(inner), isFalse);
      expect(t.contentSize(scroller), const Size(100, 170));
      expect(t.contentSize(deep), const Size(20, 20));
    });

    test('deepestAt honors scroll shift, clip, display none, and accept', () {
      final t = b.tree();
      bool pressable(int i) => t.eventMask(i) != 0;
      expect(t.deepestAt(const Offset(15, 35), pressable), deep, reason: 'scrolled into view');
      expect(t.deepestAt(const Offset(15, 135), pressable), isNull, reason: 'pre-scroll position is clipped away');
      expect(t.deepestAt(const Offset(150, 50), pressable), sibling, reason: 'display:none sibling on top is skipped');
      expect(t.deepestAt(const Offset(50, 50), t.isScrollContainer), scroller);
      expect(t.deepestAt(const Offset(150, 50), (_) => true), sibling);
      expect(t.deepestAt(const Offset(500, 500), (_) => true), isNull);
    });

    test('disabled interaction yields an empty event mask', () {
      b.bits(interactionShape, deep, NveInteractionTable.disabled, 1);
      final t = b.tree();
      expect(t.eventMask(deep), 0);
      expect(t.eventMask(sibling), nveEventMaskPress);
      expect(t.eventMask(root), 0);
    });

    test('paint order sorts by z-index stably and keeps document order otherwise', () {
      final s = SceneBuilder();
      final r = s.node(const Rect.fromLTWH(0, 0, 10, 10));
      final a = s.node(Rect.zero, parent: r);
      final c = s.node(Rect.zero, parent: r);
      final d = s.node(Rect.zero, parent: r);
      final e = s.node(Rect.zero, parent: r);
      expect(s.tree().paintOrder(r), [a, c, d, e]);
      s
        ..bits(visualShape, a, NveVisualTable.zIndex, 2)
        ..bits(visualShape, d, NveVisualTable.zIndex, -1 & 0xFFFFFFFF);
      final t = s.tree();
      expect(t.zIndex(d), -1);
      expect(t.paintOrder(r), [d, c, e, a]);
    });

    test('empty scene has no root', () {
      final t = NveSceneTree(SceneBuilder().build());
      expect(t.root, -1);
      expect(t.deepestAt(Offset.zero, (_) => true), isNull);
    });
  });

  group('NveTextStyle', () {
    test('inherits unset fields and overrides set or non-default ones', () {
      final b = SceneBuilder();
      final parent = b.node(const Rect.fromLTWH(0, 0, 10, 10));
      final child = b.node(Rect.zero, parent: parent);
      final plain = b.node(Rect.zero, parent: parent);
      b
        ..bits(typographyShape, parent, NveTypographyTable.set,
            NveTypographyTable.setFontFamily | NveTypographyTable.setLineHeight | NveTypographyTable.setTextDecoration)
        ..bits(typographyShape, parent, NveTypographyTable.color, packRgba(255, 0, 0))
        ..f32(typographyShape, parent, NveTypographyTable.fontSize, 20)
        ..bits(typographyShape, parent, NveTypographyTable.fontWeight, 700)
        ..str(typographyShape, parent, NveTypographyTable.fontFamily, 'Inter')
        ..length(typographyShape, parent, NveTypographyTable.lineHeight, 150, percent: true)
        ..bits(typographyShape, parent, NveTypographyTable.textDecoration,
            NveTypographyTable.decorationUnderline | NveTypographyTable.decorationLineThrough)
        ..bits(typographyShape, child, NveTypographyTable.set, NveTypographyTable.setFontStyle | NveTypographyTable.setMaxLines)
        ..bits(typographyShape, child, NveTypographyTable.fontStyle, NveFontStyle.italic)
        ..bits(typographyShape, child, NveTypographyTable.maxLines, 2)
        ..bits(typographyShape, child, NveTypographyTable.textAlign, NveTextAlign.center);
      final s = b.build();
      final typo = s.findTable(NveTypographyTable.key);
      final p = const NveTextStyle().resolve(s, typo, parent);
      expect(p.color, packRgba(255, 0, 0));
      expect(p.fontSize, 20);
      expect(p.fontWeight, 700);
      expect(p.fontFamily, 'Inter');
      expect(p.lineHeight, 30, reason: '150% of 20pt');
      final c = p.resolve(s, typo, child);
      expect(c.color, p.color);
      expect(c.fontSize, 20);
      expect(c.fontFamily, 'Inter');
      expect(c.italic, isTrue);
      expect(c.maxLines, 2);
      expect(c.flutterAlign, TextAlign.center);
      expect(p.resolve(s, typo, plain), same(p), reason: 'no Typography row');
      final ts = c.toTextStyle();
      expect(ts.color, const Color(0xFFFF0000));
      expect(ts.fontWeight, FontWeight.w700);
      expect(ts.fontStyle, FontStyle.italic);
      expect(ts.height, 1.5);
      expect(ts.decoration, TextDecoration.combine([TextDecoration.underline, TextDecoration.lineThrough]));
      expect(c.toTextStyle(colorOverride: packRgba(0, 0, 255, 128)).color, const Color(0x800000FF));
      expect(c == p.resolve(s, typo, child), isTrue);
      expect(c.hashCode, p.resolve(s, typo, child).hashCode);
      expect(c == p, isFalse);
    });

    test('weight maps onto the nearest Flutter weight and colors unpack RGBA', () {
      expect(const NveTextStyle(fontWeight: 0).toTextStyle().fontWeight, FontWeight.w100);
      expect(const NveTextStyle(fontWeight: 449).toTextStyle().fontWeight, FontWeight.w400);
      expect(const NveTextStyle(fontWeight: 450).toTextStyle().fontWeight, FontWeight.w500);
      expect(const NveTextStyle(fontWeight: 1000).toTextStyle().fontWeight, FontWeight.w900);
      expect(nveColor(packRgba(1, 2, 3, 4)), const Color.fromARGB(4, 1, 2, 3));
    });
  });
}
