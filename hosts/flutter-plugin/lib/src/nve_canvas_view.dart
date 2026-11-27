/// Zero-PlatformView Flutter host widget: one [RenderBox] that lays the runtime surface out at its
/// size, paints the published scene with [NveScenePainter], turns pointers into runtime input
/// (press, pointer down/move/up, drag and wheel scrolling), and exposes live semantics.
library;

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:nve_host/nve_host.dart';
import 'package:nve_host/nve_host_ffi.dart';

import 'canvas_semantics.dart';
import 'nve_canvas_controller.dart';
import 'scene_painter.dart';

class NveCanvasView extends LeafRenderObjectWidget {
  const NveCanvasView({super.key, required this.controller, this.backgroundColor, this.painter});

  final NveCanvasController controller;
  final Color? backgroundColor;

  /// Custom painter (extra [NveTablePainter]s, shared image cache); one per view by default.
  final NveScenePainter? painter;

  @override
  RenderNveCanvas createRenderObject(BuildContext context) => RenderNveCanvas(
        controller: controller,
        devicePixelRatio: MediaQuery.maybeDevicePixelRatioOf(context) ?? 1,
        backgroundColor: backgroundColor,
        painter: painter,
      );

  @override
  void updateRenderObject(BuildContext context, RenderNveCanvas renderObject) {
    renderObject
      ..controller = controller
      ..devicePixelRatio = MediaQuery.maybeDevicePixelRatioOf(context) ?? 1
      ..backgroundColor = backgroundColor
      ..painter = painter;
  }
}

final class _Gesture {
  _Gesture(this.target, this.scroller, this.start);

  final int? target;
  final int? scroller;
  final Offset start;
  bool dragging = false;
}

class RenderNveCanvas extends RenderBox {
  RenderNveCanvas({
    required NveCanvasController controller,
    double devicePixelRatio = 1,
    Color? backgroundColor,
    NveScenePainter? painter,
  })  : _controller = controller,
        _dpr = devicePixelRatio,
        _background = backgroundColor,
        _painter = painter ?? NveScenePainter(),
        _ownsPainter = painter == null;

  /// Drag distance before a press turns into a scroll (Flutter's touch slop).
  static const double dragSlop = kTouchSlop;

  NveCanvasController _controller;
  double _dpr;
  Color? _background;
  NveScenePainter _painter;
  bool _ownsPainter;
  final NveCanvasSemanticsAssembler _semantics = NveCanvasSemanticsAssembler();
  final Map<int, _Gesture> _gestures = <int, _Gesture>{};

  NveCanvasController get controller => _controller;
  set controller(NveCanvasController value) {
    if (identical(value, _controller)) {
      return;
    }
    if (attached) {
      _controller.removeListener(_onFrame);
      value.addListener(_onFrame);
    }
    _controller = value;
    _gestures.clear();
    markNeedsLayout();
    markNeedsSemanticsUpdate();
  }

  set devicePixelRatio(double value) {
    if (value != _dpr) {
      _dpr = value;
      markNeedsLayout();
    }
  }

  NveScenePainter get painter => _painter;

  /// Swaps in a caller-owned [value], or (with `null`) a painter this render object owns; a
  /// previously owned painter is disposed.
  set painter(NveScenePainter? value) {
    if (value == null ? _ownsPainter : identical(value, _painter)) {
      return;
    }
    if (_ownsPainter) {
      _painter.dispose();
    }
    _painter = value ?? NveScenePainter();
    _ownsPainter = value == null;
    markNeedsPaint();
  }

  set backgroundColor(Color? value) {
    if (value != _background) {
      _background = value;
      markNeedsPaint();
    }
  }

  @override
  void attach(PipelineOwner owner) {
    super.attach(owner);
    _controller.addListener(_onFrame);
  }

  @override
  void detach() {
    _controller.removeListener(_onFrame);
    _gestures.clear();
    super.detach();
  }

  @override
  void dispose() {
    _semantics.clear();
    if (_ownsPainter) {
      _painter.dispose();
    }
    super.dispose();
  }

  void _onFrame() {
    markNeedsPaint();
    markNeedsSemanticsUpdate();
  }

  @override
  bool get sizedByParent => true;

  /// Unbounded axes fall back to the smallest allowed size: the runtime lays out to the box it
  /// is given, so a canvas inside a scrollable must receive an explicit extent.
  @override
  Size computeDryLayout(BoxConstraints constraints) =>
      constraints.constrain(Size(constraints.hasBoundedWidth ? constraints.maxWidth : 0,
          constraints.hasBoundedHeight ? constraints.maxHeight : 0));

  @override
  void performLayout() {
    if (!_controller.isDisposed) {
      _controller.setViewport(size, _dpr);
    }
  }

  @override
  bool hitTestSelf(Offset position) => size.contains(position);

  @override
  void handleEvent(PointerEvent event, BoxHitTestEntry entry) {
    final c = _controller;
    if (c.isDisposed) {
      return;
    }
    final p = event.localPosition;
    final ts = event.timeStamp.inMicroseconds;
    switch (event) {
      case PointerScrollEvent(:final scrollDelta):
        // Through the signal resolver: only the innermost scrollable (this canvas or an
        // enclosing Flutter scroll view) consumes one wheel event.
        final scroller = c.scrollableAt(p);
        if (scroller != null) {
          GestureBinding.instance.pointerSignalResolver
              .register(event, (_) => c.isDisposed ? null : c.scrollBy(scroller, scrollDelta));
        }
      case PointerPanZoomStartEvent(:final pointer):
        // Trackpad two-finger scroll (desktop): pan deltas scroll the container under the start.
        _gestures[pointer] = _Gesture(null, c.scrollableAt(p), p)..dragging = true;
      case PointerPanZoomUpdateEvent(:final pointer, :final localPanDelta):
        if (_gestures[pointer]?.scroller case final scroller?) {
          c.scrollBy(scroller, -localPanDelta);
        }
      case PointerPanZoomEndEvent(:final pointer):
        _gestures.remove(pointer);
      case PointerDownEvent(:final pointer, :final buttons):
        final g = _gestures[pointer] = _Gesture(c.hitTest(p), c.scrollableAt(p), p);
        if (g.target case final target?) {
          c.dispatch([
            RuntimeEvent.pointer(NveEventKind.pointerDown, target, p.dx, p.dy,
                button: _button(buttons), timestampUs: ts)
          ]);
        }
      case PointerMoveEvent(:final pointer, :final delta):
        final g = _gestures[pointer];
        if (g == null) {
          return;
        }
        if (!g.dragging && g.scroller != null && (p - g.start).distance >= dragSlop) {
          g.dragging = true;
        }
        if (g.dragging) {
          c.scrollBy(g.scroller!, -delta);
        } else if (g.target case final target?) {
          c.dispatch([RuntimeEvent.pointer(NveEventKind.pointerMove, target, p.dx, p.dy, timestampUs: ts)]);
        }
      case PointerUpEvent(:final pointer):
        final g = _gestures.remove(pointer);
        final target = g?.target;
        if (g == null || target == null) {
          return;
        }
        c.dispatch([
          RuntimeEvent.pointer(NveEventKind.pointerUp, target, p.dx, p.dy, timestampUs: ts),
          if (!g.dragging && c.hitTest(p) == target) RuntimeEvent.press(target, p.dx, p.dy, timestampUs: ts),
        ]);
      case PointerCancelEvent(:final pointer):
        final g = _gestures.remove(pointer);
        if (g?.target case final target?) {
          c.dispatch([RuntimeEvent.pointer(NveEventKind.pointerUp, target, p.dx, p.dy, timestampUs: ts)]);
        }
      default:
    }
  }

  static int _button(int buttons) => buttons & kSecondaryButton != 0 ? 2 : (buttons & kMiddleMouseButton != 0 ? 1 : 0);

  @override
  void paint(PaintingContext context, Offset offset) {
    final canvas = context.canvas;
    final bounds = offset & size;
    canvas
      ..save()
      ..clipRect(bounds);
    try {
      if (_background case final bg?) {
        canvas.drawRect(bounds, Paint()..color = bg);
      }
      if (!_controller.isDisposed) {
        _controller.withTree((tree) => _painter.paint(canvas, tree, offset,
            resources: _controller.resources,
            defaultFontFamily: _controller.defaultFontFamily,
            onImageLoaded: () => attached ? markNeedsPaint() : null));
      }
    } finally {
      canvas.restore();
    }
  }

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
  void assembleSemanticsNode(SemanticsNode node, SemanticsConfiguration config, Iterable<SemanticsNode> children) {
    final c = _controller;
    final built = c.isDisposed
        ? const <SemanticsNode>[]
        : c.withTree((tree) => _semantics.build(tree, Offset.zero & size, (id, center) {
              c.dispatch([RuntimeEvent.press(id, center.dx, center.dy)]);
            }));
    node.updateWith(config: config, childrenInInversePaintOrder: [...children, ...built.reversed]);
  }
}
