import 'dart:math' as math;

import 'package:flutter/rendering.dart';

import 'command_decoder.dart';

/// Pluggable text measurement function used by [NveFlexLayoutSolver].
///
/// `maxWidth <= 0` or non-finite means unconstrained single-line width.
typedef NveTextMeasurer = Size Function(String text, double fontSize, double maxWidth);

/// Default text measurer backed by Flutter's real [TextPainter] so layout
/// dimensions match painted glyphs 1:1.
Size flutterTextMeasurer(
  String text,
  double fontSize,
  double maxWidth, {
  String? fontFamily,
}) {
  if (text.isEmpty) return Size.zero;
  final effectiveFontSize = (fontSize.isFinite && fontSize > 0.0) ? fontSize : 16.0;
  final effectiveMaxWidth =
      (maxWidth.isFinite && maxWidth > 0.0) ? maxWidth : double.infinity;

  final painter = TextPainter(
    text: TextSpan(
      text: text,
      style: TextStyle(fontSize: effectiveFontSize, fontFamily: fontFamily),
    ),
    textDirection: TextDirection.ltr,
  )..layout(maxWidth: effectiveMaxWidth);

  final measuredW = painter.width.ceilToDouble();
  final clampedW = effectiveMaxWidth.isFinite
      ? math.min(measuredW, effectiveMaxWidth)
      : measuredW;
  final measuredH = painter.height.ceilToDouble();
  painter.dispose();
  return Size(clampedW, measuredH);
}

class _MeasureCacheEntry {
  _MeasureCacheEntry({
    required this.availW,
    required this.availH,
    required this.parentWDef,
    required this.parentHDef,
    required this.size,
  });

  final double availW;
  final double availH;
  final bool parentWDef;
  final bool parentHDef;
  final Size size;
}

/// Pure-Dart CSS Flexbox layout solver tuned for NVE's `LayoutProps` and
/// backed by Flutter's [TextPainter] for exact glyph measurement.
class NveFlexLayoutSolver {
  NveFlexLayoutSolver({NveTextMeasurer? textMeasurer, String? defaultFontFamily})
      : _textMeasurer = textMeasurer ??
            ((String text, double fontSize, double maxWidth) =>
                flutterTextMeasurer(text, fontSize, maxWidth, fontFamily: defaultFontFamily));

  static const double _unconstrained = -1.0;
  static const int _maxDepth = 512;

  NveTextMeasurer _textMeasurer;

  NveTextMeasurer get textMeasurer => _textMeasurer;

  set textMeasurer(NveTextMeasurer value) {
    _textMeasurer = value;
  }

  /// Measures the intrinsic size of the surface root within [constraints].
  Size measureRoot(NveRetainedTree tree, BoxConstraints constraints) {
    tree.propagateTextStyles();
    final cache = <int, _MeasureCacheEntry>{};
    final availW = constraints.hasBoundedWidth ? constraints.maxWidth : _unconstrained;
    final availH = constraints.hasBoundedHeight ? constraints.maxHeight : _unconstrained;
    final measured = _measureIntrinsic(
      tree,
      nveRootNodeId,
      availW,
      availH,
      constraints.hasBoundedWidth,
      constraints.hasBoundedHeight,
      cache,
      0,
    );
    return constraints.constrain(measured);
  }

  /// Computes surface-absolute [Rect]s for every reachable node in [tree]
  /// within [viewportSize].
  void layout(NveRetainedTree tree, Size viewportSize) {
    final rootNode = tree.node(nveRootNodeId);
    if (rootNode == null) return;

    tree.propagateTextStyles();

    final vw = viewportSize.width.isFinite ? math.max(0.0, viewportSize.width) : 0.0;
    final vh = viewportSize.height.isFinite ? math.max(0.0, viewportSize.height) : 0.0;
    final cache = <int, _MeasureCacheEntry>{};

    final defW = _resolveDefinite(rootNode.layout.width, vw);
    final defH = _resolveDefinite(rootNode.layout.height, vh);
    final rootW = _clampAxis(
      defW >= 0.0 ? defW : vw,
      rootNode.layout.minWidth,
      rootNode.layout.maxWidth,
      vw,
    );
    final rootH = _clampAxis(
      defH >= 0.0 ? defH : vh,
      rootNode.layout.minHeight,
      rootNode.layout.maxHeight,
      vh,
    );

    _layoutNode(tree, nveRootNodeId, 0.0, 0.0, rootW, rootH, cache, 0);
  }

  static double _resolveDefinite(NveDimension d, double parentSize) {
    return switch (d.unit) {
      NveDimensionUnit.points => math.max(0.0, d.value),
      NveDimensionUnit.percent =>
        parentSize >= 0.0 ? math.max(0.0, parentSize * (d.value * 0.01)) : _unconstrained,
      NveDimensionUnit.auto => _unconstrained,
    };
  }

  static double _clampAxis(
    double value,
    NveDimension minDim,
    NveDimension maxDim,
    double parentSize,
  ) {
    var result = math.max(0.0, value);
    final maxV = _resolveDefinite(maxDim, parentSize);
    if (maxV >= 0.0) {
      result = math.min(result, maxV);
    }
    final minV = _resolveDefinite(minDim, parentSize);
    if (minV >= 0.0) {
      result = math.max(result, minV);
    }
    return math.max(0.0, result);
  }

  Size _measureText(NveNode node, double maxW) {
    final text = node.text;
    if (text == null || text.isEmpty) return Size.zero;
    final fontSize =
        (node.style.fontSize.isFinite && node.style.fontSize > 0.0) ? node.style.fontSize : 16.0;
    final measured = _textMeasurer(text, fontSize, maxW);
    var outW = (measured.width.isFinite && measured.width >= 0.0) ? measured.width : 0.0;
    final outH = (measured.height.isFinite && measured.height >= 0.0) ? measured.height : 0.0;
    if (maxW > 0.0) {
      outW = math.min(outW, maxW);
    }
    return Size(outW, outH);
  }

  Size _measureIntrinsic(
    NveRetainedTree tree,
    int nodeId,
    double availW,
    double availH,
    bool parentWDef,
    bool parentHDef,
    Map<int, _MeasureCacheEntry> cache,
    int depth,
  ) {
    if (depth > _maxDepth) return Size.zero;
    final node = tree.node(nodeId);
    if (node == null) return Size.zero;

    final cached = cache[nodeId];
    if (cached != null &&
        cached.parentWDef == parentWDef &&
        cached.parentHDef == parentHDef &&
        (cached.availW - availW).abs() <= 1e-3 &&
        (cached.availH - availH).abs() <= 1e-3) {
      return cached.size;
    }

    final l = node.layout;
    final pctBaseW = parentWDef ? availW : _unconstrained;
    final pctBaseH = parentHDef ? availH : _unconstrained;

    var knownW = _resolveDefinite(l.width, pctBaseW);
    var knownH = _resolveDefinite(l.height, pctBaseH);
    if (knownW >= 0.0) {
      knownW = _clampAxis(knownW, l.minWidth, l.maxWidth, pctBaseW);
    }
    if (knownH >= 0.0) {
      knownH = _clampAxis(knownH, l.minHeight, l.maxHeight, pctBaseH);
    }

    Size result;
    if (knownW >= 0.0 && knownH >= 0.0) {
      result = Size(knownW, knownH);
    } else if (node.isTextLeaf) {
      var textMaxW = 0.0;
      if (knownW >= 0.0) {
        textMaxW = knownW;
      } else {
        var boundW = availW;
        final maxWDim = _resolveDefinite(l.maxWidth, pctBaseW);
        if (maxWDim >= 0.0) {
          boundW = boundW >= 0.0 ? math.min(boundW, maxWDim) : maxWDim;
        }
        if (boundW > 0.0) {
          textMaxW = boundW;
        }
      }
      final measured = _measureText(node, textMaxW);
      final w = knownW >= 0.0
          ? knownW
          : _clampAxis(measured.width, l.minWidth, l.maxWidth, pctBaseW);
      final h = knownH >= 0.0
          ? knownH
          : _clampAxis(measured.height, l.minHeight, l.maxHeight, pctBaseH);
      result = Size(w, h);
    } else {
      final padW = math.max(0.0, l.padding.left) + math.max(0.0, l.padding.right);
      final padH = math.max(0.0, l.padding.top) + math.max(0.0, l.padding.bottom);

      if (node.children.isEmpty) {
        final w = knownW >= 0.0
            ? knownW
            : _clampAxis(padW, l.minWidth, l.maxWidth, pctBaseW);
        final h = knownH >= 0.0
            ? knownH
            : _clampAxis(padH, l.minHeight, l.maxHeight, pctBaseH);
        result = Size(w, h);
      } else {
        final isRow = l.flexDirection.isRow;
        final gap = math.max(0.0, l.gap);

        var boundW = knownW;
        if (boundW < 0.0) {
          boundW = availW;
          final maxWDim = _resolveDefinite(l.maxWidth, pctBaseW);
          if (maxWDim >= 0.0) {
            boundW = boundW >= 0.0 ? math.min(boundW, maxWDim) : maxWDim;
          }
        }
        final innerAvailW = boundW >= 0.0 ? math.max(0.0, boundW - padW) : _unconstrained;
        final innerAvailH = knownH >= 0.0 ? math.max(0.0, knownH - padH) : _unconstrained;

        var sumMain = 0.0;
        var maxCross = 0.0;
        var validChildren = 0;

        for (final cid in node.children) {
          final child = tree.node(cid);
          if (child == null) continue;
          final cl = child.layout;
          final cMw = cl.margin.left + cl.margin.right;
          final cMh = cl.margin.top + cl.margin.bottom;
          final cAvailW = innerAvailW >= 0.0 ? math.max(0.0, innerAvailW - cMw) : _unconstrained;
          final cAvailH = innerAvailH >= 0.0 ? math.max(0.0, innerAvailH - cMh) : _unconstrained;

          final cs = _measureIntrinsic(
            tree,
            cid,
            cAvailW,
            cAvailH,
            knownW >= 0.0,
            knownH >= 0.0,
            cache,
            depth + 1,
          );
          final outerW = math.max(0.0, cs.width + cMw);
          final outerH = math.max(0.0, cs.height + cMh);
          if (isRow) {
            sumMain += outerW;
            maxCross = math.max(maxCross, outerH);
          } else {
            sumMain += outerH;
            maxCross = math.max(maxCross, outerW);
          }
          validChildren += 1;
        }
        if (validChildren > 1) {
          sumMain += gap * (validChildren - 1);
        }

        final contentW = padW + (isRow ? sumMain : maxCross);
        final contentH = padH + (isRow ? maxCross : sumMain);
        final w = knownW >= 0.0
            ? knownW
            : _clampAxis(contentW, l.minWidth, l.maxWidth, pctBaseW);
        final h = knownH >= 0.0
            ? knownH
            : _clampAxis(contentH, l.minHeight, l.maxHeight, pctBaseH);
        result = Size(w, h);
      }
    }

    cache[nodeId] = _MeasureCacheEntry(
      availW: availW,
      availH: availH,
      parentWDef: parentWDef,
      parentHDef: parentHDef,
      size: result,
    );
    return result;
  }

  void _layoutNode(
    NveRetainedTree tree,
    int nodeId,
    double absX,
    double absY,
    double width,
    double height,
    Map<int, _MeasureCacheEntry> cache,
    int depth,
  ) {
    if (depth > _maxDepth) return;
    final node = tree.node(nodeId);
    if (node == null) return;

    node.computedRect = Rect.fromLTWH(absX, absY, width, height);
    node.hasComputedLayout = true;

    if (node.children.isEmpty) return;

    final liveChildren = <int>[];
    for (final cid in node.children) {
      if (tree.node(cid) != null) {
        liveChildren.add(cid);
      }
    }
    if (liveChildren.isEmpty) return;

    final nl = node.layout;
    final isRow = nl.flexDirection.isRow;
    final isReverse = nl.flexDirection.isReverse;
    final padL = math.max(0.0, nl.padding.left);
    final padR = math.max(0.0, nl.padding.right);
    final padT = math.max(0.0, nl.padding.top);
    final padB = math.max(0.0, nl.padding.bottom);
    final innerX = absX + padL;
    final innerY = absY + padT;
    final innerW = math.max(0.0, width - padL - padR);
    final innerH = math.max(0.0, height - padT - padB);
    final mainAvail = isRow ? innerW : innerH;
    final crossAvail = isRow ? innerH : innerW;
    final gap = math.max(0.0, nl.gap);
    final count = liveChildren.length;
    final totalGap = count > 1 ? gap * (count - 1) : 0.0;

    var fixedMain = 0.0;
    var totalGrow = 0.0;
    final childMain = List<double>.filled(count, 0.0);
    final childCross = List<double>.filled(count, 0.0);
    final childInitialMain = List<double>.filled(count, 0.0);
    final childAutoCross = List<bool>.filled(count, false);

    for (var i = 0; i < count; i++) {
      final child = tree.node(liveChildren[i])!;
      final cl = child.layout;
      final mMargin = isRow ? cl.margin.horizontal : cl.margin.vertical;
      final cMargin = isRow ? cl.margin.vertical : cl.margin.horizontal;
      final mDim = isRow ? cl.width : cl.height;
      final cDim = isRow ? cl.height : cl.width;
      final mMin = isRow ? cl.minWidth : cl.minHeight;
      final mMax = isRow ? cl.maxWidth : cl.maxHeight;
      final cMin = isRow ? cl.minHeight : cl.minWidth;
      final cMax = isRow ? cl.maxHeight : cl.maxWidth;

      final defMain = _resolveDefinite(mDim, mainAvail);
      final defCross = _resolveDefinite(cDim, crossAvail);
      final crossStretched = defCross < 0.0 && nl.alignItems == NveFlexAlign.stretch;
      childAutoCross[i] = defCross < 0.0 && !crossStretched;

      var resolvedMain = defMain >= 0.0
          ? _clampAxis(defMain, mMin, mMax, mainAvail)
          : _unconstrained;
      var resolvedCross = defCross >= 0.0
          ? _clampAxis(defCross, cMin, cMax, crossAvail)
          : (crossStretched
              ? _clampAxis(math.max(0.0, crossAvail - cMargin), cMin, cMax, crossAvail)
              : _unconstrained);

      if (resolvedMain < 0.0 || resolvedCross < 0.0) {
        final cMw = cl.margin.horizontal;
        final cMh = cl.margin.vertical;
        final cAvailW = isRow
            ? (resolvedMain >= 0.0 ? resolvedMain : math.max(0.0, innerW - cMw))
            : (resolvedCross >= 0.0 ? resolvedCross : math.max(0.0, innerW - cMw));
        final cAvailH = isRow
            ? (resolvedCross >= 0.0 ? resolvedCross : math.max(0.0, innerH - cMh))
            : (resolvedMain >= 0.0 ? resolvedMain : _unconstrained);
        final cWDef = isRow ? (resolvedMain >= 0.0) : (resolvedCross >= 0.0);
        final cHDef = isRow ? (resolvedCross >= 0.0) : (resolvedMain >= 0.0);

        final measured = _measureIntrinsic(
          tree,
          liveChildren[i],
          cAvailW,
          cAvailH,
          cWDef,
          cHDef,
          cache,
          depth + 1,
        );
        if (resolvedMain < 0.0) {
          resolvedMain = _clampAxis(
            isRow ? measured.width : measured.height,
            mMin,
            mMax,
            mainAvail,
          );
        }
        if (resolvedCross < 0.0) {
          resolvedCross = _clampAxis(
            isRow ? measured.height : measured.width,
            cMin,
            cMax,
            crossAvail,
          );
        }
      }

      childMain[i] = resolvedMain;
      childInitialMain[i] = resolvedMain;
      childCross[i] = resolvedCross;
      fixedMain += resolvedMain + mMargin;
      totalGrow += math.max(0.0, cl.flexGrow);
    }

    // Flex grow / shrink distribution along the main axis.
    final freeMain = mainAvail - fixedMain - totalGap;
    if (freeMain > 0.0 && totalGrow > 0.0) {
      for (var i = 0; i < count; i++) {
        final cl = tree.node(liveChildren[i])!.layout;
        if (cl.flexGrow > 0.0) {
          final mMin = isRow ? cl.minWidth : cl.minHeight;
          final mMax = isRow ? cl.maxWidth : cl.maxHeight;
          childMain[i] = _clampAxis(
            childMain[i] + freeMain * (cl.flexGrow / totalGrow),
            mMin,
            mMax,
            mainAvail,
          );
        }
      }
    } else if (freeMain < 0.0) {
      var totalScaledShrink = 0.0;
      for (var i = 0; i < count; i++) {
        final cl = tree.node(liveChildren[i])!.layout;
        totalScaledShrink += math.max(0.0, cl.flexShrink) * childMain[i];
      }
      if (totalScaledShrink > 0.0) {
        final overflow = -freeMain;
        for (var i = 0; i < count; i++) {
          final cl = tree.node(liveChildren[i])!.layout;
          final weight = math.max(0.0, cl.flexShrink) * childMain[i];
          if (weight > 0.0) {
            final mMin = isRow ? cl.minWidth : cl.minHeight;
            final mMax = isRow ? cl.maxWidth : cl.maxHeight;
            final shrunk = math.max(0.0, childMain[i] - overflow * (weight / totalScaledShrink));
            childMain[i] = _clampAxis(shrunk, mMin, mMax, mainAvail);
          }
        }
      }
    }

    // Post-flex wrapping re-measurement: if a row child's width changed after
    // flex grow/shrink and its height is Auto (un-stretched), re-measure height
    // at its final width so wrapped text/containers expand vertically.
    if (isRow) {
      for (var i = 0; i < count; i++) {
        if (childAutoCross[i] && (childMain[i] - childInitialMain[i]).abs() > 1e-3) {
          final cid = liveChildren[i];
          final cl = tree.node(cid)!.layout;
          final cAvailH = math.max(0.0, innerH - cl.margin.vertical);
          final remeasured = _measureIntrinsic(
            tree,
            cid,
            childMain[i],
            cAvailH,
            true,
            false,
            cache,
            depth + 1,
          );
          childCross[i] = _clampAxis(
            remeasured.height,
            cl.minHeight,
            cl.maxHeight,
            crossAvail,
          );
        }
      }
    }

    var usedMain = totalGap;
    for (var i = 0; i < count; i++) {
      final cl = tree.node(liveChildren[i])!.layout;
      usedMain += childMain[i] + (isRow ? cl.margin.horizontal : cl.margin.vertical);
    }

    final remMain = mainAvail - usedMain;
    var leadingMain = 0.0;
    var betweenExtra = 0.0;
    switch (nl.justifyContent) {
      case NveFlexJustify.flexStart:
        break;
      case NveFlexJustify.center:
        leadingMain = remMain * 0.5;
      case NveFlexJustify.flexEnd:
        leadingMain = remMain;
      case NveFlexJustify.spaceBetween:
        if (count > 1 && remMain > 0.0) {
          betweenExtra = remMain / (count - 1);
        }
      case NveFlexJustify.spaceAround:
        if (remMain > 0.0) {
          betweenExtra = remMain / count;
          leadingMain = betweenExtra * 0.5;
        } else {
          leadingMain = remMain * 0.5;
        }
      case NveFlexJustify.spaceEvenly:
        if (remMain > 0.0) {
          betweenExtra = remMain / (count + 1);
          leadingMain = betweenExtra;
        } else {
          leadingMain = remMain * 0.5;
        }
    }

    // Optional baseline reference for row containers with alignItems == baseline.
    double maxBaseline = 0.0;
    List<double>? childBaselines;
    if (isRow && nl.alignItems == NveFlexAlign.baseline) {
      childBaselines = List<double>.filled(count, 0.0);
      for (var i = 0; i < count; i++) {
        final child = tree.node(liveChildren[i])!;
        final b = _estimateBaseline(child, childCross[i]) + child.layout.margin.top;
        childBaselines[i] = b;
        if (b > maxBaseline) maxBaseline = b;
      }
    }

    var cursor = isReverse
        ? ((isRow ? (innerX + innerW) : (innerY + innerH)) - leadingMain)
        : ((isRow ? innerX : innerY) + leadingMain);

    for (var i = 0; i < count; i++) {
      final cid = liveChildren[i];
      final cl = tree.node(cid)!.layout;
      final mStart = isRow ? cl.margin.left : cl.margin.top;
      final mEnd = isRow ? cl.margin.right : cl.margin.bottom;
      final cStart = isRow ? cl.margin.top : cl.margin.left;
      final cEnd = isRow ? cl.margin.bottom : cl.margin.right;

      final remCross = crossAvail - (childCross[i] + cStart + cEnd);
      double crossOffset;
      switch (nl.alignItems) {
        case NveFlexAlign.center:
          crossOffset = cStart + remCross * 0.5;
        case NveFlexAlign.flexEnd:
          crossOffset = cStart + remCross;
        case NveFlexAlign.baseline:
          if (childBaselines != null) {
            crossOffset = cStart + (maxBaseline - childBaselines[i]);
          } else {
            crossOffset = cStart;
          }
        case NveFlexAlign.stretch:
        case NveFlexAlign.flexStart:
          crossOffset = cStart;
      }

      double mainPos;
      if (isReverse) {
        cursor -= mEnd + childMain[i];
        mainPos = cursor;
        cursor -= mStart + gap + betweenExtra;
      } else {
        cursor += mStart;
        mainPos = cursor;
        cursor += childMain[i] + mEnd + gap + betweenExtra;
      }

      final childX = isRow ? mainPos : (innerX + crossOffset);
      final childY = isRow ? (innerY + crossOffset) : mainPos;
      final childW = isRow ? childMain[i] : childCross[i];
      final childH = isRow ? childCross[i] : childMain[i];

      _layoutNode(tree, cid, childX, childY, childW, childH, cache, depth + 1);
    }
  }

  static double _estimateBaseline(NveNode node, double height) {
    if (node.kind == NveComponentKind.text) {
      final fontSize =
          (node.style.fontSize.isFinite && node.style.fontSize > 0.0) ? node.style.fontSize : 16.0;
      return math.min(height, fontSize * 0.8);
    }
    return height;
  }
}
