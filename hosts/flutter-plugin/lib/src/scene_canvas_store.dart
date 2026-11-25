import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/rendering.dart';

import 'command_types.dart';
import 'nve_scene_abi_gen.dart';

/// Cached immutable snapshot of one 64-byte `NveRenderNode` struct ingested directly via
/// [NveSceneReader] (`dart:ffi`) without per-node Dart command decoding.
class NveSceneNodeSnapshot {
  const NveSceneNodeSnapshot({
    required this.nodeId,
    required this.kind,
    required this.flags,
    required this.opacityU8,
    required this.rect,
    required this.bgRgba,
    required this.textRgba,
    required this.borderRgba,
    required this.borderRadius,
    required this.borderWidth,
    required this.fontSize,
    required this.text,
    required this.children,
    required this.zIndex,
  });

  final int nodeId;
  final int kind;
  final int flags;
  final int opacityU8;
  final Rect rect;
  final int bgRgba;
  final int textRgba;
  final int borderRgba;
  final double borderRadius;
  final double borderWidth;
  final double fontSize;
  final String text;
  final Uint32List children;
  final int zIndex;

  bool get isLeafText => (flags & nveNodeLeafText) != 0;
  bool get isInteractive => (flags & nveNodeInteractive) != 0;
  bool get isScrollView => kind == nveNodeKindScrollView;
  bool get clipsBounds => isScrollView || (flags & nveNodeClipBounds) != 0;
  double get opacity => opacityU8 / 255.0;
}

/// Result of clamping and applying a scroll delta to a `ScrollView` node.
class NveScrollUpdate {
  const NveScrollUpdate({
    required this.nodeId,
    required this.offset,
    required this.contentSize,
  });

  final int nodeId;
  final Offset offset;
  final Size contentSize;
}

/// Incremental `O(dirtyCount + removedCount)` direct `NveSceneView` store & canvas painter
/// backing [RenderNveCanvas.updateFromSceneView].
class NveSceneCanvasStore {
  final Map<int, NveSceneNodeSnapshot> _nodesById = <int, NveSceneNodeSnapshot>{};
  final List<int> _indexToId = <int>[];
  final Map<int, Offset> _scrollOffsets = <int, Offset>{};

  int _frameId = 0;
  int _lastDirtyCount = 0;
  int _lastRemovedCount = 0;

  int get frameId => _frameId;
  int get nodeCount => _nodesById.length;
  int get lastDirtyCount => _lastDirtyCount;
  int get lastRemovedCount => _lastRemovedCount;
  bool get isEmpty => _nodesById.isEmpty;

  Map<int, NveSceneNodeSnapshot> get nodes => Map<int, NveSceneNodeSnapshot>.unmodifiable(_nodesById);

  NveSceneNodeSnapshot? node(int nodeId) => _nodesById[nodeId];

  Offset scrollOffsetOf(int nodeId) => _scrollOffsets[nodeId] ?? Offset.zero;

  /// Applies a direct `dart:ffi` [NveSceneReader] snapshot in `O(dirtyCount + removedCount)`.
  void updateFromSceneView(NveSceneReader reader) {
    final removed = reader.removedIds;
    for (var i = 0; i < removed.length; i++) {
      final deadId = removed[i];
      _nodesById.remove(deadId);
      _scrollOffsets.remove(deadId);
    }

    final needsFullSync = _indexToId.isEmpty ||
        (_indexToId.length != reader.nodeCount && reader.dirtyIndices.isEmpty);

    if (needsFullSync) {
      _nodesById.clear();
      _indexToId
        ..clear()
        ..addAll(List<int>.filled(reader.nodeCount, 0));
      for (var i = 0; i < reader.nodeCount; i++) {
        final snap = _readNodeSnapshot(reader, reader.nodeAt(i));
        _indexToId[i] = snap.nodeId;
        _nodesById[snap.nodeId] = snap;
      }
      _lastDirtyCount = reader.dirtyIndices.isNotEmpty ? reader.dirtyIndices.length : reader.nodeCount;
    } else {
      if (_indexToId.length < reader.nodeCount) {
        _indexToId.addAll(List<int>.filled(reader.nodeCount - _indexToId.length, 0));
      } else if (_indexToId.length > reader.nodeCount) {
        _indexToId.length = reader.nodeCount;
      }

      reader.forEachDirty((int idx, NveRenderNode rawNode) {
        final snap = _readNodeSnapshot(reader, rawNode);
        _indexToId[idx] = snap.nodeId;
        _nodesById[snap.nodeId] = snap;
      });
      _lastDirtyCount = reader.dirtyIndices.length;
    }

    _lastRemovedCount = removed.length;
    _frameId = reader.frameId;
  }

  NveSceneNodeSnapshot _readNodeSnapshot(NveSceneReader reader, NveRenderNode n) {
    final childSlice = reader.childrenFor(n);
    return NveSceneNodeSnapshot(
      nodeId: n.nodeId,
      kind: n.kind,
      flags: n.flags,
      opacityU8: n.opacityU8,
      rect: Rect.fromLTWH(n.x, n.y, n.width, n.height),
      bgRgba: n.bgRgba,
      textRgba: n.textRgba,
      borderRgba: n.borderRgba,
      borderRadius: n.borderRadius,
      borderWidth: n.borderWidth,
      fontSize: n.fontSize,
      text: n.textLen > 0 ? reader.textFor(n) : '',
      children: childSlice.isEmpty ? Uint32List(0) : Uint32List.fromList(childSlice),
      zIndex: n.zIndex,
    );
  }

  /// Applies a scroll delta to `nodeId`, clamping to content bounds computed from its children.
  NveScrollUpdate? scrollBy(int nodeId, Offset delta) {
    final target = _nodesById[nodeId];
    if (target == null || !target.isScrollView) return null;
    final contentSize = _contentSizeFor(target);
    final maxX = math.max(0.0, contentSize.width - target.rect.width);
    final maxY = math.max(0.0, contentSize.height - target.rect.height);
    final current = _scrollOffsets[nodeId] ?? Offset.zero;
    final next = Offset(
      (current.dx + delta.dx).clamp(0.0, maxX).toDouble(),
      (current.dy + delta.dy).clamp(0.0, maxY).toDouble(),
    );
    _scrollOffsets[nodeId] = next;
    return NveScrollUpdate(nodeId: nodeId, offset: next, contentSize: contentSize);
  }

  Size _contentSizeFor(NveSceneNodeSnapshot node) {
    var maxW = node.rect.width;
    var maxH = node.rect.height;
    for (var i = 0; i < node.children.length; i++) {
      final child = _nodesById[node.children[i]];
      if (child == null) continue;
      final right = (child.rect.left - node.rect.left) + child.rect.width;
      final bottom = (child.rect.top - node.rect.top) + child.rect.height;
      if (right > maxW) maxW = right;
      if (bottom > maxH) maxH = bottom;
    }
    return Size(maxW, maxH);
  }

  /// Reverse z-order hit-test over cached 64-byte `NveRenderNode` snapshots.
  int? hitTestNode(Offset localPosition, {bool interactiveOnly = true}) {
    final rootId = _rootId();
    if (rootId == null) return null;
    return _hitTestRecursive(rootId, localPosition, interactiveOnly);
  }

  /// Finds the innermost `ScrollView` node containing [localPosition].
  int? hitTestScrollable(Offset localPosition) {
    final rootId = _rootId();
    if (rootId == null) return null;
    return _hitTestScrollableRecursive(rootId, localPosition);
  }

  /// Computes surface-local visual rect for `nodeId` taking active ancestor scroll offsets into account.
  Rect? layoutOf(int nodeId) => _nodesById[nodeId]?.rect;

  int? _rootId() {
    if (_nodesById.containsKey(nveRootNodeId)) return nveRootNodeId;
    if (_indexToId.isNotEmpty && _nodesById.containsKey(_indexToId.first)) {
      return _indexToId.first;
    }
    return _nodesById.isNotEmpty ? _nodesById.keys.first : null;
  }

  int? _hitTestRecursive(int nodeId, Offset pos, bool interactiveOnly) {
    final n = _nodesById[nodeId];
    if (n == null || n.opacityU8 == 0) return null;
    final inside = n.rect.contains(pos);
    if (!inside && (n.clipsBounds || nodeId != nveRootNodeId)) return null;

    final childPos = n.isScrollView ? pos + scrollOffsetOf(nodeId) : pos;
    final ordered = _childrenInPaintOrder(n.children);
    for (var i = ordered.length - 1; i >= 0; i--) {
      final hit = _hitTestRecursive(ordered[i], childPos, interactiveOnly);
      if (hit != null) return hit;
    }

    if (inside && (!interactiveOnly || n.isInteractive)) {
      return nodeId;
    }
    return null;
  }

  int? _hitTestScrollableRecursive(int nodeId, Offset pos) {
    final n = _nodesById[nodeId];
    if (n == null || n.opacityU8 == 0) return null;
    final inside = n.rect.contains(pos);
    if (!inside && nodeId != nveRootNodeId) return null;

    final childPos = n.isScrollView ? pos + scrollOffsetOf(nodeId) : pos;
    final ordered = _childrenInPaintOrder(n.children);
    for (var i = ordered.length - 1; i >= 0; i--) {
      final hit = _hitTestScrollableRecursive(ordered[i], childPos);
      if (hit != null) return hit;
    }

    if (inside && n.isScrollView) return nodeId;
    return null;
  }

  List<int> _childrenInPaintOrder(Uint32List children) {
    if (children.length <= 1) return children;
    var hasNonZeroZ = false;
    for (var i = 0; i < children.length; i++) {
      final c = _nodesById[children[i]];
      if (c != null && c.zIndex != 0) {
        hasNonZeroZ = true;
        break;
      }
    }
    if (!hasNonZeroZ) return children;

    final sorted = List<int>.of(children);
    for (var i = 1; i < sorted.length; i++) {
      final key = sorted[i];
      final keyZ = _nodesById[key]?.zIndex ?? 0;
      var j = i - 1;
      while (j >= 0 && (_nodesById[sorted[j]]?.zIndex ?? 0) > keyZ) {
        sorted[j + 1] = sorted[j];
        j -= 1;
      }
      sorted[j + 1] = key;
    }
    return sorted;
  }

  /// Paints the direct `NveSceneView` tree onto Flutter's [Canvas].
  void paint(
    Canvas canvas,
    Offset surfaceOffset,
    Rect surfaceBounds, {
    String? defaultFontFamily,
  }) {
    final rootId = _rootId();
    if (rootId == null) return;
    _paintNode(canvas, rootId, surfaceOffset, surfaceBounds, defaultFontFamily);
  }

  void _paintNode(
    Canvas canvas,
    int nodeId,
    Offset surfaceOffset,
    Rect surfaceBounds,
    String? defaultFontFamily,
  ) {
    final n = _nodesById[nodeId];
    if (n == null || n.opacityU8 == 0) return;

    final pushedLayer = n.opacityU8 < 255;
    if (pushedLayer) {
      canvas.saveLayer(
        surfaceBounds,
        Paint()..color = Color.fromARGB(n.opacityU8, 255, 255, 255),
      );
    }

    final rect = n.rect.shift(surfaceOffset);
    final hasArea = rect.width > 0.0 && rect.height > 0.0;
    final radius = math.max(0.0, n.borderRadius);

    // 1. Background fill
    final bg = NveColorRgba.fromWireUint32(n.bgRgba);
    if (hasArea && bg.a > 0) {
      final fillPaint = Paint()
        ..color = bg.toFlutterColor()
        ..style = PaintingStyle.fill
        ..isAntiAlias = true;
      if (radius > 0.0) {
        canvas.drawRRect(RRect.fromRectAndRadius(rect, Radius.circular(radius)), fillPaint);
      } else {
        canvas.drawRect(rect, fillPaint);
      }
    }

    // 2. Border stroke
    final border = NveColorRgba.fromWireUint32(n.borderRgba);
    if (hasArea && n.borderWidth > 0.0 && border.a > 0) {
      final bw = n.borderWidth;
      final strokePaint = Paint()
        ..color = border.toFlutterColor()
        ..style = PaintingStyle.stroke
        ..strokeWidth = bw
        ..isAntiAlias = true;
      final inset = bw * 0.5;
      final strokeRect = rect.width > bw && rect.height > bw ? rect.deflate(inset) : rect;
      if (radius > 0.0) {
        final innerRadius = math.max(0.0, radius - inset);
        canvas.drawRRect(
          RRect.fromRectAndRadius(strokeRect, Radius.circular(innerRadius)),
          strokePaint,
        );
      } else {
        canvas.drawRect(strokeRect, strokePaint);
      }
    }

    // 3. Leaf text
    final textColor = NveColorRgba.fromWireUint32(n.textRgba);
    if (n.isLeafText && n.text.isNotEmpty && textColor.a > 0 && hasArea) {
      final fontSize = (n.fontSize.isFinite && n.fontSize > 0.0) ? n.fontSize : 16.0;
      final painter = TextPainter(
        text: TextSpan(
          text: n.text,
          style: TextStyle(
            color: textColor.toFlutterColor(),
            fontSize: fontSize,
            fontFamily: defaultFontFamily,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout(maxWidth: math.max(0.0, rect.width));
      painter.paint(canvas, rect.topLeft);
      painter.dispose();
    }

    // 4. Children with ScrollView / clipBounds clip rect & scroll translation
    if (n.children.isNotEmpty) {
      final shouldClip = hasArea && n.clipsBounds;
      final scrollOffset = n.isScrollView ? scrollOffsetOf(nodeId) : Offset.zero;
      final needsSave = shouldClip || scrollOffset != Offset.zero;
      if (needsSave) {
        canvas.save();
        if (shouldClip) {
          if (radius > 0.0) {
            canvas.clipRRect(RRect.fromRectAndRadius(rect, Radius.circular(radius)));
          } else {
            canvas.clipRect(rect);
          }
        }
        if (scrollOffset != Offset.zero) {
          canvas.translate(-scrollOffset.dx, -scrollOffset.dy);
        }
      }

      final ordered = _childrenInPaintOrder(n.children);
      for (final childId in ordered) {
        _paintNode(canvas, childId, surfaceOffset, surfaceBounds, defaultFontFamily);
      }

      if (needsSave) {
        canvas.restore();
      }
    }

    if (pushedLayer) {
      canvas.restore();
    }
  }
}
