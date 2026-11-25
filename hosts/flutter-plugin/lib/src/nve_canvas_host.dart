import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

import '../nve_flutter.dart';
import 'canvas_semantics.dart';
import 'command_decoder.dart';
import 'nve_canvas_controller.dart';
import 'scene_canvas_store.dart';

export 'draw_list_painter.dart'
    show
        NveDrawListErrorCode,
        NveDrawListPainter,
        NveDrawListResult,
        NveDrawOpcode,
        nveDrawHeaderSize,
        nveDrawListAttrPrefix,
        nveDrawMagic,
        nveDrawMaxOps,
        nveDrawMaxPolylinePoints,
        nveDrawVersion;
export 'event_codec.dart' show encodePressEvent, encodeScrollEvent;
export 'nve_canvas_controller.dart' show NveCanvasController;
export 'scene_canvas_store.dart'
    show
        NveAccessibilityRole,
        NveImagePlacement,
        NveImageResizeMode,
        NveSceneCanvasStore,
        NveSceneImageCache,
        NveSceneNodeMetadata,
        NveSceneNodeSnapshot,
        NveScrollUpdate;

/// Direct Flutter `RenderBox` + Impeller/Skia [Canvas] host widget for any
/// compiled NVE Rust app/island.
class NveCanvasView extends LeafRenderObjectWidget {
  const NveCanvasView({
    super.key,
    required this.controller,
    this.backgroundColor,
    this.onEvent,
  });

  final NveCanvasController controller;
  final Color? backgroundColor;
  final NveEventCallback? onEvent;

  @override
  RenderNveCanvas createRenderObject(BuildContext context) {
    final dpr = MediaQuery.maybeDevicePixelRatioOf(context) ?? 1.0;
    if (onEvent != null) {
      controller.onEvent = onEvent;
    }
    return RenderNveCanvas(
      controller: controller,
      devicePixelRatio: dpr,
      backgroundColor: backgroundColor,
    );
  }

  @override
  void updateRenderObject(BuildContext context, RenderNveCanvas renderObject) {
    final dpr = MediaQuery.maybeDevicePixelRatioOf(context) ?? 1.0;
    if (onEvent != null) {
      controller.onEvent = onEvent;
    }
    renderObject
      ..controller = controller
      ..devicePixelRatio = dpr
      ..backgroundColor = backgroundColor;
  }
}

class _PointerScrollSession {
  _PointerScrollSession({
    required this.scrollNodeId,
    required this.startPosition,
  });

  final int scrollNodeId;
  final Offset startPosition;
  bool isDragging = false;
}

/// Custom [RenderBox] that paints either a retained [NveRetainedTree] or a direct
/// zero-decode `NveSceneView` slab ([updateFromSceneView]) onto Flutter's [Canvas],
/// handling reverse z-order hit-testing, `ScrollView` clipping, wheel/drag scrolling,
/// `NVE_DRAW1` vector draw lists, cached `Image` rendering, `TextInput`, and live
/// Flutter [SemanticsNode] assembly.
class RenderNveCanvas extends RenderBox {
  RenderNveCanvas({
    NveCanvasController? controller,
    double devicePixelRatio = 1.0,
    Color? backgroundColor,
    int surfaceId = 1,
    String? defaultFontFamily,
    NveEventCallback? onSceneEvent,
    NveSceneImageCache? imageCache,
  })  : _controller = controller,
        _devicePixelRatio = devicePixelRatio,
        _backgroundColor = backgroundColor,
        _surfaceId = surfaceId,
        _defaultFontFamily = defaultFontFamily,
        _onSceneEvent = onSceneEvent,
        _sceneStore = NveSceneCanvasStore(imageCache: imageCache);

  static const double _scrollDragSlop = 4.0;

  NveCanvasController? _controller;
  double _devicePixelRatio;
  Color? _backgroundColor;
  final int _surfaceId;
  final String? _defaultFontFamily;
  NveEventCallback? _onSceneEvent;

  final NveSceneCanvasStore _sceneStore;
  final NveCanvasSemanticsAssembler _semantics = NveCanvasSemanticsAssembler();
  bool _sceneMode = false;

  final Map<int, int> _activePointerTargets = <int, int>{};
  final Map<int, _PointerScrollSession> _activeScrollSessions = <int, _PointerScrollSession>{};

  NveCanvasController? get controller => _controller;

  set controller(NveCanvasController? value) {
    if (identical(_controller, value)) return;
    if (attached) {
      _controller?.removeListener(_onControllerChanged);
    }
    _controller = value;
    _activePointerTargets.clear();
    _activeScrollSessions.clear();
    if (attached) {
      _controller?.addListener(_onControllerChanged);
    }
    markNeedsLayout();
    markNeedsSemanticsUpdate();
  }

  double get devicePixelRatio => _devicePixelRatio;

  set devicePixelRatio(double value) {
    if (_devicePixelRatio == value) return;
    _devicePixelRatio = value;
    markNeedsLayout();
  }

  Color? get backgroundColor => _backgroundColor;

  set backgroundColor(Color? value) {
    if (_backgroundColor == value) return;
    _backgroundColor = value;
    markNeedsPaint();
  }

  NveEventCallback? get onSceneEvent => _onSceneEvent;

  set onSceneEvent(NveEventCallback? value) {
    _onSceneEvent = value;
  }

  /// True once [updateFromSceneView] has ingested a direct `NveSceneView` snapshot.
  bool get isSceneMode => _sceneMode;

  /// Direct `NveSceneView` snapshot store (for diagnostics and tests).
  NveSceneCanvasStore get sceneStore => _sceneStore;

  /// Updates this [RenderBox] directly from a zero-copy `dart:ffi` [NveSceneReader]
  /// (`dirtyIndices` / `removedIds`) with zero per-node Dart command decoding.
  void updateFromSceneView(NveSceneReader reader) {
    _sceneStore.updateFromSceneView(reader);
    _sceneMode = true;
    markNeedsPaint();
    markNeedsSemanticsUpdate();
  }

  /// Exposes computed surface-local node geometry for tests and tooling.
  Rect? layoutOf(int nodeId) =>
      _sceneMode ? _sceneStore.layoutOf(nodeId) : _controller?.layoutOf(nodeId);

  /// Returns current scroll offset for a `ScrollView` node.
  Offset scrollOffsetOf(int nodeId) => _sceneMode
      ? _sceneStore.scrollOffsetOf(nodeId)
      : (_controller?.scrollOffsetOf(nodeId) ?? Offset.zero);

  /// Exposes retained nodes for tests and diagnostics.
  Map<int, NveNode> get debugNodes => _controller?.debugNodes ?? const <int, NveNode>{};

  void _onAsyncImageDecoded() {
    if (attached) {
      markNeedsPaint();
    }
  }

  @override
  void attach(PipelineOwner owner) {
    super.attach(owner);
    _controller?.addListener(_onControllerChanged);
  }

  @override
  void detach() {
    _controller?.removeListener(_onControllerChanged);
    _activePointerTargets.clear();
    _activeScrollSessions.clear();
    super.detach();
  }

  @override
  void dispose() {
    _semantics.clear();
    _sceneStore.dispose();
    super.dispose();
  }

  void _onControllerChanged() {
    if (!attached) return;
    if (hasSize && (!constraints.hasBoundedWidth || !constraints.hasBoundedHeight)) {
      markNeedsLayout();
    } else {
      markNeedsPaint();
    }
    markNeedsSemanticsUpdate();
  }

  @override
  void performLayout() {
    final ctrl = _controller;
    if (!_sceneMode && ctrl != null && !ctrl.isDisposed && ctrl.frameCount == 0) {
      ctrl.renderNextFrame(notify: false);
    }

    if (constraints.hasBoundedWidth && constraints.hasBoundedHeight) {
      size = constraints.biggest;
    } else if (_sceneMode) {
      final rootRect = _sceneStore.layoutOf(nveRootNodeId);
      size = constraints.constrain(rootRect?.size ?? Size.zero);
    } else if (ctrl != null && !ctrl.isDisposed) {
      final intrinsic = ctrl.measure(constraints) ??
          ctrl.layoutSolver.measureRoot(ctrl.tree, constraints);
      size = constraints.constrain(intrinsic);
    } else {
      size = constraints.smallest;
    }

    if (!_sceneMode && ctrl != null && !ctrl.isDisposed) {
      ctrl.updateViewport(size, devicePixelRatio: _devicePixelRatio);
    }
  }

  @override
  bool hitTestSelf(Offset position) => size.contains(position);

  @override
  void handleEvent(PointerEvent event, BoxHitTestEntry entry) {
    final ctrl = _controller;
    if (!_sceneMode && (ctrl == null || ctrl.isDisposed)) return;

    if (event is PointerScrollEvent) {
      _handleScrollDelta(event.localPosition, event.scrollDelta);
      return;
    }

    if (event is PointerDownEvent) {
      final hitId = _sceneMode
          ? _sceneStore.hitTestNode(event.localPosition, interactiveOnly: true)
          : ctrl!.hitTestNode(event.localPosition, interactiveOnly: true);
      if (hitId != null) {
        _activePointerTargets[event.pointer] = hitId;
      }
      final scrollId = _sceneMode
          ? _sceneStore.hitTestScrollable(event.localPosition)
          : ctrl!.hitTestScrollable(event.localPosition);
      if (scrollId != null) {
        _activeScrollSessions[event.pointer] = _PointerScrollSession(
          scrollNodeId: scrollId,
          startPosition: event.localPosition,
        );
      }
    } else if (event is PointerMoveEvent) {
      final session = _activeScrollSessions[event.pointer];
      if (session != null) {
        if (!session.isDragging &&
            (event.localPosition - session.startPosition).distance >= _scrollDragSlop) {
          session.isDragging = true;
          _activePointerTargets.remove(event.pointer);
        }
        if (session.isDragging && event.localDelta != Offset.zero) {
          _applyScroll(session.scrollNodeId, Offset(-event.localDelta.dx, -event.localDelta.dy));
        }
      }
    } else if (event is PointerUpEvent) {
      final session = _activeScrollSessions.remove(event.pointer);
      final targetId = _activePointerTargets.remove(event.pointer);
      if ((session == null || !session.isDragging) && targetId != null) {
        final hitOnUp = _sceneMode
            ? _sceneStore.hitTestNode(event.localPosition, interactiveOnly: true)
            : ctrl!.hitTestNode(event.localPosition, interactiveOnly: true);
        if (hitOnUp == targetId) {
          _emitPress(targetId, event.localPosition);
          markNeedsPaint();
        }
      }
    } else if (event is PointerCancelEvent) {
      _activePointerTargets.remove(event.pointer);
      _activeScrollSessions.remove(event.pointer);
    }
  }

  void _handleScrollDelta(Offset localPosition, Offset delta) {
    final scrollId = _sceneMode
        ? _sceneStore.hitTestScrollable(localPosition)
        : _controller!.hitTestScrollable(localPosition);
    if (scrollId != null) {
      _applyScroll(scrollId, delta);
    }
  }

  void _applyScroll(int scrollNodeId, Offset delta) {
    if (_sceneMode) {
      final updated = _sceneStore.scrollBy(scrollNodeId, delta);
      if (updated == null) return;
      markNeedsPaint();
      markNeedsSemanticsUpdate();
      final ts = DateTime.now().microsecondsSinceEpoch;
      final ctrl = _controller;
      if (ctrl != null && !ctrl.isDisposed) {
        ctrl.dispatchScroll(
          scrollNodeId,
          x: updated.offset.dx,
          y: updated.offset.dy,
          contentWidth: updated.contentSize.width,
          contentHeight: updated.contentSize.height,
          timestampUs: ts,
        );
      }
      _onSceneEvent?.call(
        NveEvent(
          surfaceId: ctrl?.surfaceId ?? _surfaceId,
          nodeId: scrollNodeId,
          timestampUs: ts,
          kind: NveEventKind.scroll,
          payload: NveScrollPayload(
            x: updated.offset.dx,
            y: updated.offset.dy,
            contentWidth: updated.contentSize.width,
            contentHeight: updated.contentSize.height,
          ),
        ),
      );
      return;
    }

    final ctrl = _controller;
    if (ctrl == null || ctrl.isDisposed) return;
    final updated = ctrl.scrollNodeBy(scrollNodeId, delta);
    if (updated != null) {
      markNeedsPaint();
      markNeedsSemanticsUpdate();
    }
  }

  void _emitPress(int nodeId, Offset localPosition) {
    final ts = DateTime.now().microsecondsSinceEpoch;
    final ctrl = _controller;
    if (ctrl != null && !ctrl.isDisposed) {
      ctrl.dispatchPress(nodeId, localPosition, timestampUs: ts, rerender: !_sceneMode);
    }
    if (_sceneMode) {
      _onSceneEvent?.call(
        NveEvent(
          surfaceId: ctrl?.surfaceId ?? _surfaceId,
          nodeId: nodeId,
          timestampUs: ts,
          kind: NveEventKind.press,
          payload: NvePointPayload(x: localPosition.dx, y: localPosition.dy),
        ),
      );
    }
  }

  // MARK: - Live Flutter Semantics Tree (`assembleSemanticsNode`)

  @override
  void describeSemanticsConfiguration(SemanticsConfiguration config) {
    super.describeSemanticsConfiguration(config);
    config
      ..isSemanticBoundary = true
      ..explicitChildNodes = true;
  }

  @override
  void clearSemantics() {
    _semantics.clear();
    super.clearSemantics();
  }

  @override
  void assembleSemanticsNode(
    SemanticsNode node,
    SemanticsConfiguration config,
    Iterable<SemanticsNode> children,
  ) {
    final surfaceRect = Offset.zero & size;
    void onTapNode(int nodeId, Offset center) {
      _emitPress(nodeId, center);
      markNeedsPaint();
    }

    final List<SemanticsNode> built;
    if (_sceneMode) {
      built = _semantics.buildSceneSemantics(
        store: _sceneStore,
        surfaceRect: surfaceRect,
        onTapNode: onTapNode,
      );
    } else {
      final ctrl = _controller;
      built = (ctrl != null && !ctrl.isDisposed)
          ? _semantics.buildRetainedSemantics(
              controller: ctrl,
              surfaceRect: surfaceRect,
              onTapNode: onTapNode,
            )
          : const <SemanticsNode>[];
    }

    node.updateWith(
      config: config,
      childrenInInversePaintOrder: <SemanticsNode>[
        ...children,
        ...built.reversed,
      ],
    );
  }

  // MARK: - Painting

  @override
  void paint(PaintingContext context, Offset offset) {
    final canvas = context.canvas;
    final bounds = offset & size;

    canvas.save();
    canvas.clipRect(bounds);

    if (_backgroundColor != null) {
      canvas.drawRect(bounds, Paint()..color = _backgroundColor!);
    }

    if (_sceneMode) {
      _sceneStore.paint(
        canvas,
        offset,
        bounds,
        defaultFontFamily: _controller?.defaultFontFamily ?? _defaultFontFamily,
        onImageDecoded: _onAsyncImageDecoded,
      );
    } else {
      final ctrl = _controller;
      if (ctrl != null && !ctrl.isDisposed) {
        _paintNode(canvas, ctrl, nveRootNodeId, offset, bounds);
      }
    }

    canvas.restore();
  }

  void _paintNode(
    Canvas canvas,
    NveCanvasController ctrl,
    int nodeId,
    Offset surfaceOffset,
    Rect surfaceBounds,
  ) {
    final tree = ctrl.tree;
    final node = tree.node(nodeId);
    if (node == null || !node.hasComputedLayout) return;

    final opacity = node.style.opacity.clamp(0.0, 1.0);
    if (opacity <= 0.001) return;

    final pushedLayer = opacity < 0.999;
    if (pushedLayer) {
      final alpha = (opacity * 255.0).round().clamp(0, 255);
      canvas.saveLayer(
        surfaceBounds,
        Paint()..color = Color.fromARGB(alpha, 255, 255, 255),
      );
    }

    final rect = node.computedRect.shift(surfaceOffset);
    final hasArea = rect.width > 0.0 && rect.height > 0.0;
    final radius = math.max(0.0, node.style.borderRadius);

    if (hasArea && node.style.background.a > 0) {
      final fillPaint = Paint()
        ..color = node.style.background.toFlutterColor()
        ..style = PaintingStyle.fill
        ..isAntiAlias = true;
      if (radius > 0.0) {
        canvas.drawRRect(RRect.fromRectAndRadius(rect, Radius.circular(radius)), fillPaint);
      } else {
        canvas.drawRect(rect, fillPaint);
      }
    }

    if (hasArea) {
      _paintRetainedContent(canvas, ctrl, node, rect, radius);
    }

    if (hasArea && node.style.borderWidth > 0.0 && node.style.borderColor.a > 0) {
      final bw = node.style.borderWidth;
      final strokePaint = Paint()
        ..color = node.style.borderColor.toFlutterColor()
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

    final isScrollView = node.kind == NveComponentKind.scrollView;
    final scrollOffset = isScrollView ? ctrl.scrollOffsetOf(nodeId) : Offset.zero;
    final clipChildren = hasArea && isScrollView;
    final needsSave = clipChildren || scrollOffset != Offset.zero;
    if (needsSave) {
      canvas.save();
      if (clipChildren) {
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

    for (final childId in childrenInPaintOrder(tree, node.children)) {
      _paintNode(canvas, ctrl, childId, surfaceOffset, surfaceBounds);
    }

    if (needsSave) canvas.restore();
    if (pushedLayer) canvas.restore();
  }

  void _paintRetainedContent(
    Canvas canvas,
    NveCanvasController ctrl,
    NveNode node,
    Rect rect,
    double radius,
  ) {
    if (node.kind == NveComponentKind.skiaCanvas) {
      final rawDraw = node.rawProps['drawList'] ?? node.rawProps['draw_list'];
      Uint8List? drawBytes = rawDraw?.asBytes;
      if (drawBytes == null && rawDraw?.asString != null) {
        drawBytes = NveDrawListPainter.decodeAttrString(rawDraw!.asString!);
      }
      if (drawBytes != null && drawBytes.isNotEmpty) {
        _sceneStore.drawListPainter.paint(canvas, rect, drawBytes, borderRadius: radius);
      }
      return;
    }

    if (node.kind == NveComponentKind.image) {
      final uri = node.rawProps['src']?.asString ??
          node.rawProps['uri']?.asString ??
          node.text ??
          '';
      if (uri.isEmpty) return;
      final img = _sceneStore.imageCache.resolveOrRequest(uri, onDecoded: _onAsyncImageDecoded);
      if (img == null) return;
      final modeProp = node.rawProps['resizeMode'] ?? node.rawProps['resize_mode'];
      final mode = modeProp?.asInt != null
          ? NveImageResizeMode.fromCode(modeProp!.asInt!)
          : NveImageResizeMode.parse(modeProp?.asString);
      final placement = NveImagePlacement.compute(
        rect,
        img.width.toDouble(),
        img.height.toDouble(),
        mode,
      );
      if (placement == null) return;
      canvas.save();
      if (radius > 0.0) {
        canvas.clipRRect(RRect.fromRectAndRadius(rect, Radius.circular(radius)));
      } else {
        canvas.clipRect(rect);
      }
      canvas.drawImageRect(
        img,
        placement.srcRect,
        placement.dstRect,
        Paint()
          ..filterQuality = FilterQuality.medium
          ..isAntiAlias = true,
      );
      canvas.restore();
      return;
    }

    if (node.kind == NveComponentKind.textInput) {
      final rawVal = node.rawProps['value']?.asString ?? node.text ?? '';
      final placeholder = node.rawProps['placeholder']?.asString ?? '';
      final isSecure = node.rawProps['secureTextEntry']?.asBool ?? false;
      final fontSize =
          (node.style.fontSize.isFinite && node.style.fontSize > 0.0) ? node.style.fontSize : 14.0;
      _sceneStore.paintTextInputText(
        canvas,
        rect,
        value: rawVal,
        placeholder: placeholder,
        secureTextEntry: isSecure,
        textRgba: node.style.textColor.toWireUint32(),
        fontSize: fontSize,
        defaultFontFamily: ctrl.defaultFontFamily,
      );
      return;
    }

    if (node.isTextLeaf &&
        node.text != null &&
        node.text!.isNotEmpty &&
        node.style.textColor.a > 0) {
      final fontSize =
          (node.style.fontSize.isFinite && node.style.fontSize > 0.0) ? node.style.fontSize : 16.0;
      final painter = TextPainter(
        text: TextSpan(
          text: node.text!,
          style: TextStyle(
            color: node.style.textColor.toFlutterColor(),
            fontSize: fontSize,
            fontFamily: ctrl.defaultFontFamily,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout(maxWidth: math.max(0.0, rect.width));
      painter.paint(canvas, rect.topLeft);
      painter.dispose();
    }
  }
}
