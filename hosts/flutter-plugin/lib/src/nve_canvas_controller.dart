/// Flutter-side owner of one `libnve_runtime` surface: drives ticks from the frame scheduler,
/// forwards input, and exposes the published scene to painting, hit-testing, and semantics.
library;

import 'dart:ffi' as ffi;
import 'dart:ui' show Offset, Size;

import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';
import 'package:nve_host/nve_native.dart';

import 'scene_resources.dart';
import 'scene_tree.dart';

/// Event an attached app emitted towards its host (`name` + structured `value`).
final class NveAppEvent {
  const NveAppEvent(this.name, this.value);

  final String name;
  final Object? value;
}

/// Owns a [RuntimeSession] on the UI isolate. Runtime work happens only on scheduled frames:
/// the runtime's wake callback, viewport changes, input, and scrolling schedule one coalesced
/// `tick`, and a tick that publishes a new frame notifies listeners (repaint + semantics).
class NveCanvasController extends ChangeNotifier {
  /// Wraps [session] (owned: disposed with this controller).
  NveCanvasController(this.session, {this.defaultFontFamily, this.onEvent}) {
    session.setWakeListener(scheduleTick);
  }

  /// Creates a surface on `libnve_runtime` ([runtimeLibrary] path, else the platform default; iOS
  /// and statically linked hosts use the process image) and attaches [app] when given.
  factory NveCanvasController.open({
    String? runtimeLibrary,
    ffi.DynamicLibrary? app,
    String appSymbol = 'nve_get_interface',
    String? defaultFontFamily,
    ValueChanged<RuntimeEvent>? onEvent,
  }) {
    final api = runtimeLibrary != null
        ? RuntimeApi.open(runtimeLibrary)
        : (defaultTargetPlatform == TargetPlatform.iOS ? RuntimeApi.process() : RuntimeApi.open());
    final controller = NveCanvasController(RuntimeSession(api, width: 1, height: 1, scaleFactor: 1),
        defaultFontFamily: defaultFontFamily, onEvent: onEvent);
    if (app != null) {
      controller.attachApp(app, symbol: appSymbol);
    }
    return controller;
  }

  final RuntimeSession session;

  /// Font family for text without an explicit `Typography.font_family`.
  final String? defaultFontFamily;

  /// Observes every input event this controller dispatches (after dispatch).
  ValueChanged<RuntimeEvent>? onEvent;

  /// Receives app-emitted events. Hookup point for `NveRuntimeApi.drain_emitted_events`
  /// (Slice 2B-0): drain after each [tick] and forward here.
  ValueChanged<NveAppEvent>? onAppEvent;

  /// Resource payloads for painters. Hookup point for `NveRuntimeApi.resource_payload`
  /// (Slice 2B-0); `null` while the runtime exposes no resource view.
  NveSceneResources? get resources => null;

  /// Last tick / dispatch failure (also reported through [FlutterError.reportError]).
  RuntimeException? lastError;

  bool _tickScheduled = false;
  Duration? _lastFrame;
  int _frames = 0;
  Size _viewport = Size.zero;
  double _scale = 0;

  bool get isDisposed => session.isDisposed;

  /// Published frames observed so far.
  int get frameCount => _frames;

  bool get isPairConnected => session.isPairConnected;

  void attachApp(ffi.DynamicLibrary app, {String symbol = 'nve_get_interface'}) {
    session.attachAppLibrary(app, symbol: symbol);
    scheduleTick();
  }

  void detachApp() {
    session.detachApp();
    scheduleTick();
  }

  /// Mirrors a dev server's surface (`nve://` pair URI); frames arrive through the wake callback.
  void pair(String uri, {String token = ''}) {
    session.pairConnect(uri, token: token);
    scheduleTick();
  }

  void unpair() {
    session.pairDisconnect();
    scheduleTick();
  }

  /// Replays one `NVE2` frame (no app or pairing attached).
  void submitFrame(Uint8List nve2) {
    session.submitNve2(nve2);
    scheduleTick();
  }

  /// Forwards embedding-host props to the attached app. Hookup point for
  /// `NveRuntimeApi.set_props` (Slice 2B-0, `DirectProps.withMap` → `NveMapEntry[]`); returns
  /// `false` while the runtime has no props entry point.
  bool setProps(Map<String, Object?> props) => false;

  /// Sends imperative command [name] to the attached app as a `Custom` event on the root node,
  /// with [args] as its structured value (codegen `NveExportSpec.commands`).
  void sendCommand(String name, [Map<String, Object?> args = const <String, Object?>{}]) =>
      dispatch([RuntimeEvent.custom(rootNodeId, name, value: args, timestampUs: _nowUs())]);

  /// Lays the surface out at [size] logical points (no-op when unchanged or empty).
  void setViewport(Size size, double devicePixelRatio) {
    if (size.isEmpty || !size.isFinite || (size == _viewport && devicePixelRatio == _scale)) {
      return;
    }
    _viewport = size;
    _scale = devicePixelRatio;
    session.setViewport(size.width, size.height, devicePixelRatio);
    scheduleTick();
  }

  /// Dispatches [events] in order and schedules the follow-up tick.
  void dispatch(List<RuntimeEvent> events) {
    if (events.isEmpty || isDisposed) {
      return;
    }
    try {
      session.dispatchEvents(events);
    } on RuntimeException catch (e, st) {
      _report(e, st);
      return;
    }
    final listener = onEvent;
    if (listener != null) {
      events.forEach(listener);
    }
    scheduleTick();
  }

  /// Scrolls container [nodeId] by [delta]; returns the clamped offset, reports it to the app as
  /// a `Scroll` event, and schedules the tick that publishes it.
  Offset? scrollBy(int nodeId, Offset delta) {
    final (double, double) at;
    try {
      at = session.scrollBy(nodeId, delta.dx, delta.dy);
    } on RuntimeException catch (e, st) {
      _report(e, st);
      return null;
    }
    final content = withTree((t) {
      final i = t.s.indexOf(nodeId);
      return i < 0 ? Size.zero : t.contentSize(i);
    });
    dispatch([RuntimeEvent.scroll(nodeId, at.$1, at.$2, content.width, content.height, timestampUs: _nowUs())]);
    return Offset(at.$1, at.$2);
  }

  /// Raw `NodeId` of the runtime's deepest event target under [point], or `null`.
  int? hitTest(Offset point) => isDisposed ? null : session.hitTest(point.dx, point.dy);

  /// Raw `NodeId` of the deepest scroll container under [point], or `null`.
  int? scrollableAt(Offset point) => withTree((t) {
        final i = t.deepestAt(point, t.isScrollContainer);
        return i == null ? null : t.s.nodeId(i);
      });

  /// Runs [read] over the current published frame (not re-entrant; never across a tick).
  T withTree<T>(T Function(NveSceneTree tree) read) => session.withScene((s) => read(NveSceneTree(s)));

  /// Requests one coalesced tick on the next frame (safe from the wake callback).
  void scheduleTick() {
    if (_tickScheduled || isDisposed) {
      return;
    }
    _tickScheduled = true;
    SchedulerBinding.instance
      ..scheduleFrameCallback(_onFrame)
      ..scheduleFrame();
  }

  /// Ticks the runtime now; `true` when a new frame was published.
  bool tick([double dtSeconds = 0]) {
    if (isDisposed) {
      return false;
    }
    try {
      if (!session.tick(dtSeconds)) {
        return false;
      }
    } on RuntimeException catch (e, st) {
      _report(e, st);
      return false;
    }
    _frames++;
    notifyListeners();
    return true;
  }

  void _onFrame(Duration timestamp) {
    _tickScheduled = false;
    final last = _lastFrame;
    _lastFrame = timestamp;
    tick(last == null ? 0 : (timestamp - last).inMicroseconds / 1e6);
  }

  void _report(RuntimeException e, StackTrace st) {
    lastError = e;
    FlutterError.reportError(FlutterErrorDetails(exception: e, stack: st, library: 'nve_flutter'));
  }

  static int _nowUs() => DateTime.now().microsecondsSinceEpoch;

  @override
  void dispose() {
    session.dispose();
    super.dispose();
  }
}
