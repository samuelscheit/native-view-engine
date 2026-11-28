/// Flutter [SemanticsNode]s for a published runtime scene: one node per element that carries an
/// `Accessibility` row, visible text, a text input, an image, or a press target, with stable ids
/// across frames, ARIA roles mapped onto Flutter flags, and taps routed back as presses.
library;

import 'package:flutter/rendering.dart';
import 'package:nve_host/nve_host.dart';

import 'scene_tree.dart';

final class NveCanvasSemanticsAssembler {
  final Map<int, SemanticsNode> _nodes = <int, SemanticsNode>{};
  final NveTreeWalk _walk = NveTreeWalk();

  void clear() => _nodes.clear();

  /// Children of the canvas semantics node in paint order; [onTap] receives the raw `NodeId`
  /// and its on-screen center.
  List<SemanticsNode> build(NveSceneTree tree, Rect surface, void Function(int nodeId, Offset center) onTap) {
    final out = <SemanticsNode>[];
    final live = <int>{};
    final root = tree.root;
    if (root >= 0) {
      _Collector(tree, _nodes, live, out, onTap).visit(tree.walk(_walk), root, Offset.zero, surface);
    }
    _nodes.removeWhere((id, _) => !live.contains(id));
    return out;
  }
}

final class _Collector {
  _Collector(this.t, this.cache, this.live, this.out, this.onTap)
      : a11y = t.s.findTable(AccessibilityTable.key),
        text = t.s.findTable(TextTable.key),
        input = t.s.findTable(TextInputTable.key),
        image = t.s.findTable(ImageTable.key);

  final NveSceneTree t;
  final Map<int, SemanticsNode> cache;
  final Set<int> live;
  final List<SemanticsNode> out;
  final void Function(int nodeId, Offset center) onTap;
  final TableReader? a11y, text, input, image;

  SceneReader get s => t.s;

  /// `aria-hidden` prunes the subtree; [invisible] is inherited `visibility: hidden`, which a
  /// descendant's `visibility: visible` overrides (as painted).
  void visit(NveTreeWalk w, int index, Offset shift, Rect clip, [bool invisible = false]) {
    if (!w.enter(index) || t.isDisplayNone(index)) {
      return;
    }
    final vr = s.rowOf(t.visual, index);
    if (vr >= 0 && t.visual!.f32(vr, VisualTable.opacity) <= 0.001) {
      return; // fully transparent subtree: not painted, not exposed
    }
    final hidden = vr < 0
        ? invisible
        : switch (t.visual!.enumValue(vr, VisualTable.visibility)) {
            NveVisibility.visible => false,
            NveVisibility.hidden => true,
            _ => invisible,
          };
    final rect = t.rectOf(index).shift(-shift);
    final ar = s.rowOf(a11y, index);
    if (ar >= 0 && a11y!.boolean(ar, AccessibilityTable.hidden)) {
      return;
    }
    if (!hidden && !rect.intersect(clip).isEmpty) {
      _emit(index, rect, ar);
    }
    final childClip = t.clipsChildren(index) ? clip.intersect(rect) : clip;
    final childShift = shift + t.scrollShift(index);
    for (final kid in t.paintOrder(index)) {
      visit(w, kid, childShift, childClip, hidden);
    }
  }

  void _emit(int index, Rect rect, int ar) {
    final role = ar >= 0 ? a11y!.optEnumValue(ar, AccessibilityTable.role) : null;
    if (role == NveAriaRole.presentation) {
      return;
    }
    final ir = s.rowOf(input, index);
    final tr = s.rowOf(text, index);
    final pressable = t.eventMask(index) & NveInteractionEventSet.press != 0;
    final isImage = role == NveAriaRole.image || s.rowOf(image, index) >= 0;
    String a11yStr(FieldSpec f) => ar >= 0 ? s.str(a11y!, ar, f) : '';
    var label = a11yStr(AccessibilityTable.label);
    var value = a11yStr(AccessibilityTable.valueText);
    final hint = a11yStr(AccessibilityTable.hint);
    if (ir >= 0) {
      final secure = input!.boolean(ir, TextInputTable.secure);
      final v = s.str(input!, ir, TextInputTable.value);
      value = secure ? '•' * v.runes.length : v;
      if (label.isEmpty) {
        label = s.str(input!, ir, TextInputTable.placeholder);
      }
    } else if (label.isEmpty && tr >= 0) {
      label = s.str(text!, tr, TextTable.text);
    }
    if (label.isEmpty && value.isEmpty && hint.isEmpty && !pressable && ar < 0 && !isImage) {
      return;
    }
    final id = s.nodeId(index);
    live.add(id);
    final node = cache[id] ??= SemanticsNode();
    final config = SemanticsConfiguration()
      ..isSemanticBoundary = true
      ..label = label
      ..value = value
      ..hint = hint
      ..textDirection = TextDirection.ltr
      ..isButton = role == NveAriaRole.button || (pressable && ir < 0 && role == null)
      ..isLink = role == NveAriaRole.link
      ..isHeader = role == NveAriaRole.heading
      ..isImage = isImage
      ..isTextField = ir >= 0 || role == NveAriaRole.textBox || role == NveAriaRole.searchBox
      ..isSlider = role == NveAriaRole.slider;
    if (ar >= 0) {
      _states(config, ar, role);
    }
    if (pressable) {
      config.onTap = () => onTap(id, rect.center);
    }
    node
      ..rect = rect
      ..updateWith(config: config);
    out.add(node);
  }

  void _states(SemanticsConfiguration c, int row, int? role) {
    final a = a11y!;
    int? opt(FieldSpec f) => a.optEnumValue(row, f);
    final checked = opt(AccessibilityTable.checked);
    if (checked != null) {
      if (role == NveAriaRole.switchValue) {
        c.isToggled = checked == NveAriaTriState.trueValue;
      } else {
        c
          ..isChecked = checked == NveAriaTriState.trueValue
          ..isCheckStateMixed = checked == NveAriaTriState.mixed;
      }
    }
    final selected = opt(AccessibilityTable.selected);
    if (selected != null) {
      c.isSelected = selected == NveAriaTriState.trueValue;
    }
    final expanded = opt(AccessibilityTable.expanded);
    if (expanded != null) {
      c.isExpanded = expanded == NveAriaTriState.trueValue;
    }
    final pressed = opt(AccessibilityTable.pressed);
    if (pressed != null) {
      c.isToggled = pressed == NveAriaTriState.trueValue;
    }
    if (a.boolean(row, AccessibilityTable.modal)) {
      c.scopesRoute = true;
    }
    final live = opt(AccessibilityTable.live);
    if (live != null && live >= NveAriaLive.polite) {
      c.liveRegion = true;
    }
  }
}
