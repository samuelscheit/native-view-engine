import 'dart:ffi' as ffi;
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

import '../nve_flutter.dart';
import 'command_decoder.dart';
import 'flex_layout.dart';

/// Encodes a single pointer `Press` event matching `crates/nve-abi/src/events.rs`
/// (`NativeEventKind::Press = 1`, `EventPayload::Point { x, y } = tag 1`).
Uint8List encodePressEvent({
  required int surfaceId,
  required int nodeId,
  required double x,
  required double y,
  int? timestampUs,
}) {
  final ts = timestampUs ?? DateTime.now().microsecondsSinceEpoch;
  // 4 (count) + 4 (surfaceId) + 4 (nodeId) + 8 (timestampUs) + 1 (kind) + 1 (payloadTag) + 8 (x, y)
  final out = Uint8List(30);
  final bd = ByteData.sublistView(out)
    ..setUint32(0, 1, Endian.little)
    ..setUint32(4, surfaceId, Endian.little)
    ..setUint32(8, nodeId, Endian.little)
    ..setUint64(12, ts, Endian.little)
    ..setUint8(20, NveEventKind.press.code)
    ..setUint8(21, 1) // EventPayload::Point
    ..setFloat32(22, x, Endian.little)
    ..setFloat32(26, y, Endian.little);
  assert(bd.lengthInBytes == 30);
  return out;
}

/// Controller driving a direct `dart:ffi` NVE app instance rendered straight onto
/// Flutter's Impeller/Skia [Canvas] via [NveCanvasView].
class NveCanvasController extends ChangeNotifier {
  NveCanvasController.fromInstance(
    NveAppInstance instance, {
    String? libraryPath,
    NveTextMeasurer? textMeasurer,
    this.defaultFontFamily,
    this.onEvent,
  })  : _instance = instance,
        _libraryPath = libraryPath,
        tree = NveRetainedTree(),
        layoutSolver = NveFlexLayoutSolver(
          textMeasurer: textMeasurer,
          defaultFontFamily: defaultFontFamily,
        );

  /// Loads an NVE dynamic library (`lib*.dylib`, `*.so`, `*.dll`) via `nve_get_interface`.
  factory NveCanvasController.open({
    int surfaceId = 1,
    String? libraryPath,
    String vtableSymbol = 'nve_get_interface',
    NveTextMeasurer? textMeasurer,
    String? defaultFontFamily,
    NveEventCallback? onEvent,
  }) {
    final instance = NveAppInstance.open(
      surfaceId: surfaceId,
      libraryPath: libraryPath,
      vtableSymbol: vtableSymbol,
    );
    return NveCanvasController.fromInstance(
      instance,
      libraryPath: libraryPath,
      textMeasurer: textMeasurer,
      defaultFontFamily: defaultFontFamily,
      onEvent: onEvent,
    );
  }

  /// Creates a controller backed by an explicit `nve_get_interface` resolver (ideal for tests).
  factory NveCanvasController.fromInterfaceResolver({
    required ffi.Pointer<ffi.Void> Function(int) getInterface,
    int surfaceId = 1,
    NveTextMeasurer? textMeasurer,
    String? defaultFontFamily,
    NveEventCallback? onEvent,
  }) {
    final instance = NveAppInstance.fromInterfaceResolver(
      surfaceId: surfaceId,
      getInterface: getInterface,
    );
    return NveCanvasController.fromInstance(
      instance,
      textMeasurer: textMeasurer,
      defaultFontFamily: defaultFontFamily,
      onEvent: onEvent,
    );
  }

  NveAppInstance _instance;
  String? _libraryPath;

  /// Optional default font family shared by [layoutSolver] and [RenderNveCanvas].
  final String? defaultFontFamily;

  /// Retained node tree decoded from the latest binary `CommandBuffer` frames.
  final NveRetainedTree tree;

  /// Flexbox layout solver computing node rects with Flutter's [TextPainter].
  final NveFlexLayoutSolver layoutSolver;

  /// Optional callback invoked for every outbound event drained from the Rust app.
  NveEventCallback? onEvent;

  int _frameCount = 0;
  NveFrameApplyResult? _lastFrameResult;
  NveDecodeError? _lastError;
  Size _viewportSize = Size.zero;
  double _devicePixelRatio = 1.0;
  bool _disposed = false;

  int get surfaceId => _instance.surfaceId;
  String? get libraryPath => _libraryPath;
  int get frameCount => _frameCount;
  int get nodeCount => tree.nodeCount;
  NveFrameApplyResult? get lastFrameResult => _lastFrameResult;
  NveDecodeError? get lastError => _lastError;
  Size get viewportSize => _viewportSize;
  double get devicePixelRatio => _devicePixelRatio;
  bool get isDisposed => _disposed;

  /// Snapshot of all retained nodes for diagnostics and geometry tests.
  Map<int, NveNode> get debugNodes => tree.nodes;

  /// Returns the computed surface-local bounding rect of `nodeId`, or `null`.
  Rect? layoutOf(int nodeId) => tree.layoutOf(nodeId);

  /// Updates viewport dimensions and recomputes Flexbox layout if needed.
  void updateViewport(Size size, {double devicePixelRatio = 1.0}) {
    _ensureLive();
    _viewportSize = size;
    _devicePixelRatio = devicePixelRatio > 0.0 ? devicePixelRatio : 1.0;
    layoutSolver.layout(tree, _viewportSize);
  }

  /// Queries optional `NveLayoutApi.measure` if exported by the module.
  Size? measure(BoxConstraints constraints) {
    _ensureLive();
    return _instance.measure(constraints);
  }

  /// Pulls the next binary `CommandBuffer` frame from Rust, decodes it into [tree],
  /// recomputes Flexbox layout, drains outbound events, and notifies listeners.
  NveFrameApplyResult renderNextFrame({bool notify = true}) {
    _ensureLive();
    final bytes = _instance.renderNextFrame();
    if (bytes.isEmpty) {
      const emptyResult = NveFrameApplyResult.ok(
        header: NveCommandBufferHeader(
          magic: nveAbiMagic,
          version: nveAbiVersion,
          flags: 0,
          surfaceId: 0,
          frameId: 0,
          opCount: 0,
        ),
        mutationCount: 0,
        committed: false,
        syncMount: false,
        dispatchedCommands: <NveDispatchedCommand>[],
      );
      _lastFrameResult = emptyResult;
      return emptyResult;
    }

    final result = tree.applyCommandBuffer(bytes);
    _lastFrameResult = result;
    if (result.isOk) {
      _lastError = null;
      _frameCount += 1;
      if (!_viewportSize.isEmpty) {
        layoutSolver.layout(tree, _viewportSize);
      }
      drainEmittedEvents();
      if (notify) {
        notifyListeners();
      }
    } else {
      _lastError = result.error;
    }
    return result;
  }

  /// Dispatches a binary pointer `Press` event to `nodeId` at surface-local [position],
  /// then immediately renders the updated frame.
  bool dispatchPress(int nodeId, Offset position, {int? timestampUs, bool rerender = true}) {
    _ensureLive();
    final encoded = encodePressEvent(
      surfaceId: surfaceId,
      nodeId: nodeId,
      x: position.dx,
      y: position.dy,
      timestampUs: timestampUs,
    );
    return dispatchEvents(encoded, rerender: rerender);
  }

  /// Dispatches raw binary `EventRing` bytes into the Rust app.
  bool dispatchEvents(Uint8List eventRingBytes, {bool rerender = true}) {
    _ensureLive();
    final ok = _instance.dispatchEvents(eventRingBytes);
    if (rerender) {
      renderNextFrame();
    }
    return ok;
  }

  /// Drains outbound events emitted by the Rust component and forwards them to [onEvent].
  List<NveEvent> drainEmittedEvents() {
    _ensureLive();
    final events = _instance.drainEmittedEvents();
    final cb = onEvent;
    if (cb != null) {
      for (final ev in events) {
        cb(ev);
      }
    }
    return events;
  }

  /// Pushes JSON props to `NveComponentApi.set_props` and optionally re-renders.
  bool setPropsJson(Map<String, dynamic> props, {bool rerender = true}) {
    _ensureLive();
    final ok = _instance.setPropsJson(props);
    if (rerender) {
      renderNextFrame();
    }
    return ok;
  }

  /// Applies a Tier-1 RSX hot-reload patch via `NveDevApi.apply_hot_patch`.
  bool applyHotPatch(String patchJson, {bool rerender = true}) {
    _ensureLive();
    final ok = _instance.applyHotPatch(patchJson);
    if (rerender) {
      renderNextFrame();
    }
    return ok;
  }

  /// Captures serialized app state via `NveDevApi.snapshot_state`.
  Uint8List snapshotState() {
    _ensureLive();
    return _instance.snapshotState();
  }

  /// Restores serialized app state via `NveDevApi.restore_state`.
  void restoreState(Uint8List stateBytes) {
    _ensureLive();
    _instance.restoreState(stateBytes);
  }

  /// Atomically hot-swaps the loaded NVE dynamic library while preserving app state:
  /// `snapshotState` -> open new library -> `restoreState` -> reset tree -> `renderNextFrame`.
  ///
  /// If opening `newLibraryPath` fails, the current live instance remains intact.
  NveFrameApplyResult reloadLibrary(
    String newLibraryPath, {
    String vtableSymbol = 'nve_get_interface',
  }) {
    _ensureLive();
    final savedState = _instance.snapshotState();
    final nextInstance = NveAppInstance.open(
      surfaceId: surfaceId,
      libraryPath: newLibraryPath,
      vtableSymbol: vtableSymbol,
    );
    return _swapInstance(nextInstance, savedState, libraryPath: newLibraryPath);
  }

  /// Atomically hot-swaps the backing module using an explicit `nve_get_interface` resolver.
  NveFrameApplyResult reloadWithInterfaceResolver(
    ffi.Pointer<ffi.Void> Function(int) getInterface,
  ) {
    _ensureLive();
    final savedState = _instance.snapshotState();
    final nextInstance = NveAppInstance.fromInterfaceResolver(
      surfaceId: surfaceId,
      getInterface: getInterface,
    );
    return _swapInstance(nextInstance, savedState, libraryPath: _libraryPath);
  }

  NveFrameApplyResult _swapInstance(
    NveAppInstance nextInstance,
    Uint8List savedState, {
    required String? libraryPath,
  }) {
    if (savedState.isNotEmpty) {
      nextInstance.restoreState(savedState);
    }
    final previous = _instance;
    _instance = nextInstance;
    _libraryPath = libraryPath;
    previous.dispose();
    tree.reset();
    return renderNextFrame();
  }

  /// Reverse z-order hit-test returning the topmost interactive node at [localPosition].
  int? hitTestNode(Offset localPosition, {bool interactiveOnly = true}) {
    return _hitTestRecursive(nveRootNodeId, localPosition, interactiveOnly);
  }

  int? _hitTestRecursive(int nodeId, Offset pos, bool interactiveOnly) {
    final node = tree.node(nodeId);
    if (node == null || !node.hasComputedLayout) return null;
    if (node.style.opacity <= 0.001) return null;

    final inside = node.computedRect.contains(pos);
    if (!inside && nodeId != nveRootNodeId) return null;

    final ordered = _childrenInPaintOrder(tree, node.children);
    for (var i = ordered.length - 1; i >= 0; i--) {
      final hit = _hitTestRecursive(ordered[i], pos, interactiveOnly);
      if (hit != null) return hit;
    }

    if (inside && (!interactiveOnly || node.isInteractive)) {
      return nodeId;
    }
    return null;
  }

  void _ensureLive() {
    if (_disposed) {
      throw StateError('NveCanvasController has been disposed');
    }
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _instance.dispose();
    super.dispose();
  }
}

List<int> _childrenInPaintOrder(NveRetainedTree tree, List<int> children) {
  if (children.length <= 1) return children;
  var hasNonZeroZ = false;
  for (final cid in children) {
    final c = tree.node(cid);
    if (c != null && c.style.zIndex != 0) {
      hasNonZeroZ = true;
      break;
    }
  }
  if (!hasNonZeroZ) return children;

  final sorted = List<int>.of(children);
  // Dart's List.sort is not guaranteed stable; use insertion sort to preserve tree order on ties.
  for (var i = 1; i < sorted.length; i++) {
    final key = sorted[i];
    final keyZ = tree.node(key)?.style.zIndex ?? 0;
    var j = i - 1;
    while (j >= 0 && (tree.node(sorted[j])?.style.zIndex ?? 0) > keyZ) {
      sorted[j + 1] = sorted[j];
      j -= 1;
    }
    sorted[j + 1] = key;
  }
  return sorted;
}

/// Direct Flutter `RenderBox` + Impeller/Skia [Canvas] host widget for any
/// compiled NVE Rust app/island.
///
/// Requires zero PlatformViews and zero native platform plugin boilerplate —
/// renders the binary `CommandBuffer` directly onto Flutter's own [Canvas].
class NveCanvasView extends LeafRenderObjectWidget {
  const NveCanvasView({
    super.key,
    required this.controller,
    this.backgroundColor,
    this.onEvent,
  });

  final NveCanvasController controller;

  /// Optional canvas backdrop color painted before the root node.
  final Color? backgroundColor;

  /// Optional convenience listener synced onto [controller.onEvent].
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

/// Custom [RenderBox] that lays out and paints an NVE retained tree directly
/// onto Flutter's [Canvas] and handles reverse z-order pointer hit-testing.
class RenderNveCanvas extends RenderBox {
  RenderNveCanvas({
    required NveCanvasController controller,
    double devicePixelRatio = 1.0,
    Color? backgroundColor,
  })  : _controller = controller,
        _devicePixelRatio = devicePixelRatio,
        _backgroundColor = backgroundColor;

  NveCanvasController _controller;
  double _devicePixelRatio;
  Color? _backgroundColor;
  final Map<int, int> _activePointerTargets = <int, int>{};

  NveCanvasController get controller => _controller;

  set controller(NveCanvasController value) {
    if (identical(_controller, value)) return;
    if (attached) {
      _controller.removeListener(_onControllerChanged);
    }
    _controller = value;
    _activePointerTargets.clear();
    if (attached) {
      _controller.addListener(_onControllerChanged);
    }
    markNeedsLayout();
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

  /// Exposes computed surface-local node geometry for tests and tooling.
  Rect? layoutOf(int nodeId) => _controller.layoutOf(nodeId);

  /// Exposes retained nodes for tests and diagnostics.
  Map<int, NveNode> get debugNodes => _controller.debugNodes;

  @override
  void attach(PipelineOwner owner) {
    super.attach(owner);
    _controller.addListener(_onControllerChanged);
  }

  @override
  void detach() {
    _controller.removeListener(_onControllerChanged);
    _activePointerTargets.clear();
    super.detach();
  }

  void _onControllerChanged() {
    if (!attached) return;
    if (hasSize && (!constraints.hasBoundedWidth || !constraints.hasBoundedHeight)) {
      markNeedsLayout();
    } else {
      markNeedsPaint();
    }
  }

  @override
  void performLayout() {
    if (!_controller.isDisposed && _controller.frameCount == 0) {
      _controller.renderNextFrame(notify: false);
    }

    if (constraints.hasBoundedWidth && constraints.hasBoundedHeight) {
      size = constraints.biggest;
    } else {
      final intrinsic = !_controller.isDisposed
          ? (_controller.measure(constraints) ??
              _controller.layoutSolver.measureRoot(_controller.tree, constraints))
          : Size.zero;
      size = constraints.constrain(intrinsic);
    }

    if (!_controller.isDisposed) {
      _controller.updateViewport(size, devicePixelRatio: _devicePixelRatio);
    }
  }

  @override
  bool hitTestSelf(Offset position) => size.contains(position);

  @override
  void handleEvent(PointerEvent event, BoxHitTestEntry entry) {
    if (_controller.isDisposed) return;

    if (event is PointerDownEvent) {
      final hitId = _controller.hitTestNode(event.localPosition, interactiveOnly: true);
      if (hitId != null) {
        _activePointerTargets[event.pointer] = hitId;
      }
    } else if (event is PointerUpEvent) {
      final targetId = _activePointerTargets.remove(event.pointer);
      if (targetId != null) {
        final targetRect = _controller.layoutOf(targetId);
        if (targetRect != null && targetRect.contains(event.localPosition)) {
          _controller.dispatchPress(targetId, event.localPosition);
          markNeedsPaint();
        }
      }
    } else if (event is PointerCancelEvent) {
      _activePointerTargets.remove(event.pointer);
    }
  }

  @override
  void paint(PaintingContext context, Offset offset) {
    final canvas = context.canvas;
    final bounds = offset & size;

    canvas.save();
    canvas.clipRect(bounds);

    if (_backgroundColor != null) {
      canvas.drawRect(bounds, Paint()..color = _backgroundColor!);
    }

    if (!_controller.isDisposed) {
      _paintNode(canvas, _controller.tree, nveRootNodeId, offset, bounds);
    }

    canvas.restore();
  }

  void _paintNode(
    Canvas canvas,
    NveRetainedTree tree,
    int nodeId,
    Offset surfaceOffset,
    Rect surfaceBounds,
  ) {
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

    // 1. Rounded rectangle background fill.
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

    // 2. Border stroke.
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

    // 3. Leaf Text node glyph painting (skips wrapper Text nodes with children).
    if (node.isTextLeaf &&
        node.text != null &&
        node.text!.isNotEmpty &&
        node.style.textColor.a > 0 &&
        hasArea) {
      final fontSize =
          (node.style.fontSize.isFinite && node.style.fontSize > 0.0) ? node.style.fontSize : 16.0;
      final painter = TextPainter(
        text: TextSpan(
          text: node.text!,
          style: TextStyle(
            color: node.style.textColor.toFlutterColor(),
            fontSize: fontSize,
            fontFamily: _controller.defaultFontFamily,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout(maxWidth: math.max(0.0, rect.width));
      painter.paint(canvas, rect.topLeft);
      painter.dispose();
    }

    // 4. Children in ascending zIndex order.
    final orderedChildren = _childrenInPaintOrder(tree, node.children);
    for (final childId in orderedChildren) {
      _paintNode(canvas, tree, childId, surfaceOffset, surfaceBounds);
    }

    if (pushedLayer) {
      canvas.restore();
    }
  }
}
