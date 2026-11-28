import 'dart:ui' show PictureRecorder;

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
        ..bits(visualShape, scroller, VisualTable.overflowY, NveOverflow.scroll)
        ..scrollOffset(scroller, 0, 100)
        ..bits(layoutShape, hiddenNode, LayoutTable.display, NveDisplay.none)
        ..bits(interactionShape, deep, InteractionTable.eventMask, nveEventMaskPress)
        ..bits(interactionShape, sibling, InteractionTable.eventMask, nveEventMaskPress);
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
      b.bits(interactionShape, deep, InteractionTable.disabled, 1);
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
        ..bits(visualShape, a, VisualTable.zIndex, 2)
        ..bits(visualShape, d, VisualTable.zIndex, -1 & 0xFFFFFFFF);
      final t = s.tree();
      expect(t.zIndex(d), -1);
      expect(t.paintOrder(r), [d, c, e, a]);
    });

    test('corrupt cyclic frames terminate: parent walks, hit tests, painting, semantics', () {
      final b = SceneBuilder();
      final root = b.node(const Rect.fromLTWH(0, 0, 100, 100));
      final a = b.node(const Rect.fromLTWH(0, 0, 50, 50), parent: root);
      final c = b.node(const Rect.fromLTWH(0, 0, 20, 20), parent: a);
      b
        ..attach(scrollShape, a)
        ..scrollOffset(a, 0, 5)
        ..extraChild(c, a) // children cycle a -> c -> a
        ..extraChild(c, c) // self loop
        ..reparent(a, c); // parent cycle a <-> c
      final t = b.tree();
      expect(t.visualRect(c).isFinite, isTrue, reason: 'bounded parent walk');
      expect(t.visualRect(a).isFinite, isTrue);
      expect(t.deepestAt(const Offset(10, 10), (_) => true), isNotNull, reason: 'hit test terminates');
      final recorder = PictureRecorder();
      NveScenePainter().paint(Canvas(recorder), t, Offset.zero);
      recorder.endRecording().dispose();
      expect(
          NveCanvasSemanticsAssembler().build(t, const Rect.fromLTWH(0, 0, 100, 100), (_, __) {}), isA<List<Object>>());
    });

    test('empty scene has no root', () {
      final t = NveSceneTree(SceneBuilder().build());
      expect(t.root, -1);
      expect(t.deepestAt(Offset.zero, (_) => true), isNull);
    });
  });

  group('NveTextStyle', () {
    test('inherits unset fields and overrides exactly the set ones', () {
      final b = SceneBuilder();
      final parent = b.node(const Rect.fromLTWH(0, 0, 10, 10));
      final child = b.node(Rect.zero, parent: parent);
      final plain = b.node(Rect.zero, parent: parent);
      b
        ..bits(
            typographyShape,
            parent,
            TypographyTable.set,
            NveTypographySet.color |
                NveTypographySet.fontSize |
                NveTypographySet.fontWeight |
                NveTypographySet.fontFamily |
                NveTypographySet.lineHeight |
                NveTypographySet.textDecoration)
        ..bits(typographyShape, parent, TypographyTable.color, packRgba(255, 0, 0))
        ..f32(typographyShape, parent, TypographyTable.fontSize, 20)
        ..bits(typographyShape, parent, TypographyTable.fontWeight, 700)
        ..str(typographyShape, parent, TypographyTable.fontFamily, 'Inter')
        ..length(typographyShape, parent, TypographyTable.lineHeight, 150, percent: true)
        ..bits(typographyShape, parent, TypographyTable.textDecoration,
            NveTextDecoration.underline | NveTextDecoration.lineThrough)
        ..bits(typographyShape, child, TypographyTable.set,
            NveTypographySet.fontStyle | NveTypographySet.maxLines | NveTypographySet.textAlign)
        ..bits(typographyShape, child, TypographyTable.fontStyle, NveFontStyle.italic)
        ..bits(typographyShape, child, TypographyTable.maxLines, 2)
        ..bits(typographyShape, child, TypographyTable.textAlign, NveTextAlign.center);
      final s = b.build();
      final typo = s.findTable(TypographyTable.key);
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

    test('fields without their set bit never apply, and degenerate sizes fall back', () {
      final b = SceneBuilder();
      final parent = b.node(const Rect.fromLTWH(0, 0, 10, 10));
      final unset = b.node(Rect.zero, parent: parent);
      final zero = b.node(Rect.zero, parent: parent);
      final nan = b.node(Rect.zero, parent: parent);
      b
        // Non-default bits under an empty `set`: an ancestor's style must flow through untouched.
        ..bits(typographyShape, unset, TypographyTable.color, packRgba(0, 255, 0))
        ..f32(typographyShape, unset, TypographyTable.fontSize, 40)
        ..bits(typographyShape, unset, TypographyTable.fontWeight, 900)
        ..bits(typographyShape, unset, TypographyTable.textAlign, NveTextAlign.center)
        ..bits(typographyShape, unset, TypographyTable.set, NveTypographySet.maxLines)
        ..bits(typographyShape, unset, TypographyTable.maxLines, 3)
        ..bits(typographyShape, zero, TypographyTable.set, NveTypographySet.fontSize | NveTypographySet.lineHeight)
        ..f32(typographyShape, zero, TypographyTable.fontSize, 0)
        ..length(typographyShape, zero, TypographyTable.lineHeight, -4)
        ..bits(typographyShape, nan, TypographyTable.set, NveTypographySet.fontSize | NveTypographySet.letterSpacing)
        ..f32(typographyShape, nan, TypographyTable.fontSize, double.nan)
        ..f32(typographyShape, nan, TypographyTable.letterSpacing, double.infinity);
      final s = b.build();
      final typo = s.findTable(TypographyTable.key);
      const inherited = NveTextStyle(color: 7, fontSize: 13, fontWeight: 300);
      final u = inherited.resolve(s, typo, unset);
      expect((u.color, u.fontSize, u.fontWeight, u.textAlign), (7, 13, 300, NveTextAlign.start));
      expect(u.maxLines, 3, reason: 'only the set field applies');
      final z = inherited.resolve(s, typo, zero);
      expect((z.fontSize, z.lineHeight), (13, null), reason: 'zero size / negative line height fall back');
      expect(inherited.resolve(s, typo, nan).fontSize, 13, reason: 'NaN size falls back');
      expect(inherited.resolve(s, typo, nan).letterSpacing, isNull, reason: 'infinite letter spacing is dropped');
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
