/// Tree queries shared by painting, hit-testing, scrolling, and semantics over one borrowed
/// [RuntimeSceneReader] frame: paint order (`z_index`), clipping, `display: none`, visual rects
/// (published rects are pre-scroll; ancestors' scroll shifts are subtracted here), and point
/// queries. Valid only inside the `withScene` scope that produced the reader.
library;

import 'dart:ui' show Offset, Rect, Size;

import 'package:nve_host/nve_host.dart';

final class NveSceneTree {
  NveSceneTree(this.s)
      : visual = s.findTable(NveVisualTable.key),
        layout = s.findTable(NveLayoutTable.key),
        scroll = s.findTable(NveScrollTable.key),
        interaction = s.findTable(NveInteractionTable.key);

  final RuntimeSceneReader s;
  final TableReader? visual;
  final TableReader? layout;
  final TableReader? scroll;
  final TableReader? interaction;

  /// Root slot index, or `-1` before the first published frame.
  int get root => s.indexOf(rootNodeId);

  /// Published (pre-scroll) rect of [index].
  Rect rectOf(int index) => Rect.fromLTWH(s.x(index), s.y(index), s.width(index), s.height(index));

  /// Scroll shift [index] applies to its children.
  Offset scrollShift(int index) {
    final (x, y) = s.scrollShift(index);
    return Offset(x, y);
  }

  /// On-screen rect of [index]: its published rect minus every ancestor's scroll shift.
  Rect visualRect(int index) {
    var shift = Offset.zero;
    for (var p = s.indexOf(s.parentId(index)); p > 0; p = s.indexOf(s.parentId(p))) {
      shift += scrollShift(p);
    }
    return rectOf(index).shift(-shift);
  }

  bool isDisplayNone(int index) {
    final row = s.rowOf(layout, index);
    return row >= 0 && layout!.enumValue(row, NveLayoutTable.display) == NveDisplay.none;
  }

  /// `Visual.overflow_x / overflow_y` clips children.
  bool clipsChildren(int index) {
    final row = s.rowOf(visual, index);
    return row >= 0 &&
        (visual!.enumValue(row, NveVisualTable.overflowX) != NveOverflow.visible ||
            visual!.enumValue(row, NveVisualTable.overflowY) != NveOverflow.visible);
  }

  bool isScrollContainer(int index) => s.rowOf(scroll, index) >= 0;

  /// `Interaction.event_mask` of [index] (`0` when absent or disabled).
  int eventMask(int index) {
    final row = s.rowOf(interaction, index);
    if (row < 0 || interaction!.boolean(row, NveInteractionTable.disabled)) {
      return 0;
    }
    return interaction!.u32(row, NveInteractionTable.eventMask);
  }

  /// Live child slot indices of [index] in paint order (stable `z_index` sort when any is set).
  List<int> paintOrder(int index) {
    final ids = s.children(index);
    final out = <int>[];
    var anyZ = false;
    for (final id in ids) {
      final i = s.indexOf(id);
      if (i > 0) {
        out.add(i);
        anyZ = anyZ || zIndex(i) != 0;
      }
    }
    if (anyZ && out.length > 1) {
      final order = List<int>.generate(out.length, (i) => i);
      order.sort((a, b) {
        final z = zIndex(out[a]).compareTo(zIndex(out[b]));
        return z != 0 ? z : a.compareTo(b);
      });
      return [for (final i in order) out[i]];
    }
    return out;
  }

  int zIndex(int index) {
    final row = s.rowOf(visual, index);
    return row < 0 ? 0 : visual!.i32(row, NveVisualTable.zIndex);
  }

  /// Deepest node under [point] (surface coordinates) for which [accept] holds, honoring
  /// `display: none`, child clipping, scroll shifts, and reverse paint order. Transforms are not
  /// applied (they never move layout or hit-test boxes in the runtime either).
  int? deepestAt(Offset point, bool Function(int index) accept) {
    final r = root;
    return r < 0 ? null : _deepestAt(r, point, Offset.zero, accept);
  }

  int? _deepestAt(int index, Offset point, Offset shift, bool Function(int index) accept) {
    if (isDisplayNone(index)) {
      return null;
    }
    final rect = rectOf(index).shift(-shift);
    final inside = rect.contains(point);
    if (!inside && clipsChildren(index)) {
      return null;
    }
    final kids = paintOrder(index);
    final childShift = shift + scrollShift(index);
    for (var i = kids.length - 1; i >= 0; i--) {
      final hit = _deepestAt(kids[i], point, childShift, accept);
      if (hit != null) {
        return hit;
      }
    }
    return inside && accept(index) ? index : null;
  }

  /// Extent of [index]'s children measured from its own origin (scroll content size).
  Size contentSize(int index) {
    final box = rectOf(index);
    var w = box.width, h = box.height;
    for (final child in paintOrder(index)) {
      final r = rectOf(child);
      w = r.right - box.left > w ? r.right - box.left : w;
      h = r.bottom - box.top > h ? r.bottom - box.top : h;
    }
    return Size(w, h);
  }
}
