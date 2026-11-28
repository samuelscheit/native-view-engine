/// Tree queries shared by painting, hit-testing, scrolling, and semantics over one borrowed
/// [SceneReader] frame: paint order (`z_index`), clipping, `display: none`, visual rects
/// (published rects are pre-scroll; ancestors' scroll shifts are subtracted here), and point
/// queries. Valid only inside the `withScene` scope that produced the reader.
library;

import 'dart:typed_data' show Uint32List;
import 'dart:ui' show Offset, Rect, Size;

import 'package:nve_host/nve_host.dart';

/// Visit marks of one recursive tree walk: [enter] admits each live slot once, so a corrupt frame
/// whose children slab forms a cycle (or lists one child twice) ends after at most
/// `nodeCapacity` visits instead of hanging the UI isolate.
final class NveTreeWalk {
  /// Reusable marks: keep one per walker (painter, semantics, hit testing) and pass it to
  /// [NveSceneTree.walk]; each walk bumps an epoch instead of allocating or clearing.
  NveTreeWalk();

  Uint32List _seen = Uint32List(0);
  int _capacity = 0;
  int _epoch = 0;

  void _begin(int capacity) {
    if (_seen.length < capacity) {
      _seen = Uint32List(capacity);
      _epoch = 0;
    }
    _capacity = capacity;
    if (++_epoch > 0xFFFFFFFF) {
      _seen.fillRange(0, _seen.length, 0);
      _epoch = 1;
    }
  }

  /// `true` the first time [index] (a valid slot) is entered in this walk.
  bool enter(int index) {
    if (index <= 0 || index >= _capacity || _seen[index] == _epoch) {
      return false;
    }
    _seen[index] = _epoch;
    return true;
  }
}

final class NveSceneTree {
  NveSceneTree(this.s)
      : visual = s.findTable(VisualTable.key),
        layout = s.findTable(LayoutTable.key),
        scroll = s.findTable(ScrollTable.key),
        interaction = s.findTable(InteractionTable.key);

  final SceneReader s;
  final TableReader? visual;
  final TableReader? layout;
  final TableReader? scroll;
  final TableReader? interaction;

  /// Root slot index, or `-1` before the first published frame.
  int get root => s.indexOf(rootNodeId);

  /// Visit marks for one recursive walk over this frame: [reuse] (re-armed in O(1), no
  /// allocation once sized) or a fresh one.
  NveTreeWalk walk([NveTreeWalk? reuse]) => (reuse ?? NveTreeWalk()).._begin(s.nodeCapacity);

  /// Published (pre-scroll) rect of [index].
  Rect rectOf(int index) => Rect.fromLTWH(s.x(index), s.y(index), s.width(index), s.height(index));

  /// Scroll shift [index] applies to its children.
  Offset scrollShift(int index) {
    final (x, y) = s.scrollShift(index);
    return Offset(x, y);
  }

  /// On-screen rect of [index]: its published rect minus every ancestor's scroll shift. The
  /// parent walk is bounded by `nodeCapacity` (a corrupt cyclic parent chain cannot hang it).
  Rect visualRect(int index) {
    var shift = Offset.zero;
    var prev = index;
    for (var p = s.indexOf(s.parentId(index)), depth = 0;
        p > 0 && p != prev && depth < s.nodeCapacity;
        prev = p, p = s.indexOf(s.parentId(p)), depth++) {
      shift += scrollShift(p);
    }
    return rectOf(index).shift(-shift);
  }

  bool isDisplayNone(int index) {
    final row = s.rowOf(layout, index);
    return row >= 0 && layout!.enumValue(row, LayoutTable.display) == NveDisplay.none;
  }

  /// `Visual.overflow_x / overflow_y` clips children.
  bool clipsChildren(int index) {
    final row = s.rowOf(visual, index);
    return row >= 0 &&
        (visual!.enumValue(row, VisualTable.overflowX) != NveOverflow.visible ||
            visual!.enumValue(row, VisualTable.overflowY) != NveOverflow.visible);
  }

  bool isScrollContainer(int index) => s.rowOf(scroll, index) >= 0;

  /// `Interaction` listener bitmask (`NveInteractionEventSet`) of [index] (`0` when absent or disabled).
  int eventMask(int index) {
    final row = s.rowOf(interaction, index);
    if (row < 0 || interaction!.boolean(row, InteractionTable.disabled)) {
      return 0;
    }
    return interaction!.eventMask(row, InteractionTable.eventsWord);
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
    return row < 0 ? 0 : visual!.i32(row, VisualTable.zIndex);
  }

  /// Deepest node under [point] (surface coordinates) for which [accept] holds, honoring
  /// `display: none`, child clipping, scroll shifts, and reverse paint order. Transforms are not
  /// applied (they never move layout or hit-test boxes in the runtime either).
  int? deepestAt(Offset point, bool Function(int index) accept, {NveTreeWalk? walk}) {
    final r = root;
    return r < 0 ? null : _deepestAt(this.walk(walk), r, point, Offset.zero, accept);
  }

  int? _deepestAt(NveTreeWalk w, int index, Offset point, Offset shift, bool Function(int index) accept) {
    if (!w.enter(index) || isDisplayNone(index)) {
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
      final hit = _deepestAt(w, kids[i], point, childShift, accept);
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
