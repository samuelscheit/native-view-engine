import 'dart:ffi' as ffi;
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/widgets.dart';

import '../nve_flutter.dart';
import 'command_decoder.dart';
import 'flex_layout.dart';
import 'scene_canvas_store.dart';

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

  final Map<int, Offset> _scrollOffsets = <int, Offset>{};
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

  /// Returns the current scroll offset for a `ScrollView` node.
  Offset scrollOffsetOf(int nodeId) => _scrollOffsets[nodeId] ?? Offset.zero;

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
      _scrollOffsets.removeWhere((int id, Offset _) => tree.node(id) == null);
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

  /// Dispatches a pointer `Press` event via `dispatch_events_direct` (with binary `EventRing`
  /// fallback) and immediately renders the updated frame.
  bool dispatchPress(int nodeId, Offset position, {int? timestampUs, bool rerender = true}) {
    _ensureLive();
    final ts = timestampUs ?? DateTime.now().microsecondsSinceEpoch;
    final bool ok;
    if (_instance.supportsDirectEvents) {
      ok = _instance.dispatchEventsDirect(<NveEvent>[
        NveEvent(
          surfaceId: surfaceId,
          nodeId: nodeId,
          timestampUs: ts,
          kind: NveEventKind.press,
          payload: NvePointPayload(x: position.dx, y: position.dy),
        ),
      ]);
    } else {
      final encoded = encodePressEvent(
        surfaceId: surfaceId,
        nodeId: nodeId,
        x: position.dx,
        y: position.dy,
        timestampUs: ts,
      );
      ok = _instance.dispatchEvents(encoded);
    }
    if (rerender) {
      renderNextFrame();
    }
    return ok;
  }

  /// Dispatches a `Scroll` event via `dispatch_events_direct` (with binary `encodeScrollEvent`
  /// `EventRing` fallback).
  bool dispatchScroll(
    int nodeId, {
    required double x,
    required double y,
    required double contentWidth,
    required double contentHeight,
    int? timestampUs,
    bool rerender = false,
  }) {
    _ensureLive();
    final ts = timestampUs ?? DateTime.now().microsecondsSinceEpoch;
    final bool ok;
    if (_instance.supportsDirectEvents) {
      ok = _instance.dispatchEventsDirect(<NveEvent>[
        NveEvent(
          surfaceId: surfaceId,
          nodeId: nodeId,
          timestampUs: ts,
          kind: NveEventKind.scroll,
          payload: NveScrollPayload(
            x: x,
            y: y,
            contentWidth: contentWidth,
            contentHeight: contentHeight,
          ),
        ),
      ]);
    } else {
      final encoded = encodeScrollEvent(
        surfaceId: surfaceId,
        nodeId: nodeId,
        x: x,
        y: y,
        contentWidth: contentWidth,
        contentHeight: contentHeight,
        timestampUs: ts,
      );
      ok = _instance.dispatchEvents(encoded);
    }
    if (rerender) {
      renderNextFrame();
    }
    return ok;
  }

  /// Applies a scroll delta to a `ScrollView` node in [tree], clamping to its content bounds,
  /// and dispatches the resulting scroll event.
  NveScrollUpdate? scrollNodeBy(int nodeId, Offset delta, {int? timestampUs}) {
    _ensureLive();
    final node = tree.node(nodeId);
    if (node == null || node.kind != NveComponentKind.scrollView || !node.hasComputedLayout) {
      return null;
    }
    final contentSize = _scrollContentSize(node);
    final maxX = math.max(0.0, contentSize.width - node.computedRect.width);
    final maxY = math.max(0.0, contentSize.height - node.computedRect.height);
    final current = _scrollOffsets[nodeId] ?? Offset.zero;
    final next = Offset(
      (current.dx + delta.dx).clamp(0.0, maxX).toDouble(),
      (current.dy + delta.dy).clamp(0.0, maxY).toDouble(),
    );
    _scrollOffsets[nodeId] = next;
    dispatchScroll(
      nodeId,
      x: next.dx,
      y: next.dy,
      contentWidth: contentSize.width,
      contentHeight: contentSize.height,
      timestampUs: timestampUs,
    );
    notifyListeners();
    return NveScrollUpdate(nodeId: nodeId, offset: next, contentSize: contentSize);
  }

  Size _scrollContentSize(NveNode scrollNode) {
    var maxW = scrollNode.computedRect.width;
    var maxH = scrollNode.computedRect.height;
    for (final childId in scrollNode.children) {
      final child = tree.node(childId);
      if (child == null || !child.hasComputedLayout) continue;
      final right = (child.computedRect.left - scrollNode.computedRect.left) +
          child.computedRect.width +
          scrollNode.layout.padding.right;
      final bottom = (child.computedRect.top - scrollNode.computedRect.top) +
          child.computedRect.height +
          scrollNode.layout.padding.bottom;
      if (right > maxW) maxW = right;
      if (bottom > maxH) maxH = bottom;
    }
    return Size(maxW, maxH);
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

  /// Dispatches typed events via `dispatch_events_direct` (with binary `EventRing` fallback).
  bool dispatchEventsDirect(List<NveEvent> events, {bool rerender = true}) {
    _ensureLive();
    final ok = _instance.dispatchEventsDirect(events);
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

  /// Pushes structured props via `set_props_direct` (binary `NveBinValue` fallback), binary FFI.
  bool setPropsDirect(Map<String, Object?> props, {bool rerender = true}) {
    _ensureLive();
    final ok = _instance.setPropsDirect(props);
    if (rerender) {
      renderNextFrame();
    }
    return ok;
  }

  /// Applies one binary `NVP1` Tier-1 hot-patch frame via `NveDevApi.apply_hot_patch`.
  bool applyHotPatch(Uint8List patchNvp1, {bool rerender = true}) {
    _ensureLive();
    final ok = _instance.applyHotPatch(patchNvp1);
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

  /// Atomically hot-swaps the loaded NVE dynamic library while preserving app state.
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
    _scrollOffsets.clear();
    tree.reset();
    return renderNextFrame();
  }

  /// Reverse z-order hit-test returning the topmost interactive node at [localPosition],
  /// honoring `ScrollView` viewport clipping and scroll offsets.
  int? hitTestNode(Offset localPosition, {bool interactiveOnly = true}) {
    return _hitTestRecursive(nveRootNodeId, localPosition, interactiveOnly);
  }

  /// Finds the innermost `ScrollView` node containing [localPosition].
  int? hitTestScrollable(Offset localPosition) {
    return _hitTestScrollableRecursive(nveRootNodeId, localPosition);
  }

  int? _hitTestRecursive(int nodeId, Offset pos, bool interactiveOnly) {
    final node = tree.node(nodeId);
    if (node == null || !node.hasComputedLayout) return null;
    if (node.style.opacity <= 0.001) return null;

    final inside = node.computedRect.contains(pos);
    if (!inside && nodeId != nveRootNodeId) return null;

    final childPos =
        node.kind == NveComponentKind.scrollView ? pos + scrollOffsetOf(nodeId) : pos;

    final ordered = childrenInPaintOrder(tree, node.children);
    for (var i = ordered.length - 1; i >= 0; i--) {
      final hit = _hitTestRecursive(ordered[i], childPos, interactiveOnly);
      if (hit != null) return hit;
    }

    if (inside && (!interactiveOnly || node.isInteractive)) {
      return nodeId;
    }
    return null;
  }

  int? _hitTestScrollableRecursive(int nodeId, Offset pos) {
    final node = tree.node(nodeId);
    if (node == null || !node.hasComputedLayout) return null;
    if (node.style.opacity <= 0.001) return null;

    final inside = node.computedRect.contains(pos);
    if (!inside && nodeId != nveRootNodeId) return null;

    final childPos =
        node.kind == NveComponentKind.scrollView ? pos + scrollOffsetOf(nodeId) : pos;

    final ordered = childrenInPaintOrder(tree, node.children);
    for (var i = ordered.length - 1; i >= 0; i--) {
      final hit = _hitTestScrollableRecursive(ordered[i], childPos);
      if (hit != null) return hit;
    }

    if (inside && node.kind == NveComponentKind.scrollView) {
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

/// Stable z-index sort preserving tree order on equal `zIndex`.
List<int> childrenInPaintOrder(NveRetainedTree tree, List<int> children) {
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
