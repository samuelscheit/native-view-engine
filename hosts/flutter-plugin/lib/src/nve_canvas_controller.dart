/// Flutter-side owner of one `libnve_runtime` surface: drives ticks from the frame scheduler,
/// forwards input, and exposes the published scene to painting, hit-testing, and semantics.
library;

import 'dart:ffi' as ffi;
import 'dart:math' as math;
import 'dart:ui' show Offset, Size;

import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';
import 'package:nve_host/nve_host.dart';
import 'package:nve_host/nve_host_ffi.dart';

import 'scene_resources.dart';
import 'scene_tree.dart';

/// [NveSceneResources] over the session's `resource_payload`: valid inside [NveCanvasController.withTree].
final class _SessionResources implements NveSceneResources {
  const _SessionResources(this.session);

  final RuntimeSession session;

  @override
  Uint8List? payload(int resourceId) => session.resourcePayload(resourceId)?.$2;
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
  /// uses the process image) and attaches [app] when given.
  factory NveCanvasController.open({
    String? runtimeLibrary,
    ffi.DynamicLibrary? app,
    String appSymbol = 'nve_get_interface',
    String? defaultFontFamily,
    ValueChanged<RuntimeEvent>? onEvent,
  }) {
    final controller = NveCanvasController(RuntimeSession(RuntimeApi.open(runtimeLibrary), width: 1, height: 1, scaleFactor: 1),
        defaultFontFamily: defaultFontFamily, onEvent: onEvent);
    if (app != null) {
      try {
        controller.attachApp(app, symbol: appSymbol);
      } on Object {
        controller.dispose();
        rethrow;
      }
    }
    return controller;
  }

  final RuntimeSession session;

  /// Font family for text without an explicit `Typography.font_family`.
  final String? defaultFontFamily;

  /// Observes every input event this controller dispatches (after dispatch).
  ValueChanged<RuntimeEvent>? onEvent;

  /// Receives app-emitted events (`drain_emitted_events`, drained after every [tick]).
  ValueChanged<RuntimeEvent>? onAppEvent;

  /// Published resource payloads (`resource_payload`) for painters; valid inside [withTree].
  late final NveSceneResources resources = _SessionResources(session);

  /// Last tick / dispatch failure (also reported through [FlutterError.reportError]).
  Exception? lastError;

  bool _tickScheduled = false;
  Duration? _lastFrame;
  double _lastDelta = 0;

  /// Longest frame step fed to the runtime: the first frame after an idle period (or a long
  /// stall) advances animations by at most this, never by the wall-clock gap.
  static const double maxFrameDelta = 0.1;

  /// `dt` (seconds) of the most recent scheduled tick.
  double get lastFrameDelta => _lastDelta;
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

  /// Forwards embedding-host props to the attached app (`set_props`); `true` (and a scheduled
  /// tick) when the app re-renders, `false` when nothing changed or the app takes no props.
  bool setProps(Map<String, Object?> props) {
    try {
      if (!session.setProps(props)) {
        return false;
      }
    } on RuntimeException catch (e) {
      if (e.status == NveRuntimeStatus.unsupported) {
        return false;
      }
      rethrow;
    }
    scheduleTick();
    return true;
  }

  /// Restores a `RuntimeSession.snapshotState` result into the app and schedules the re-render.
  void restoreState(Uint8List state) {
    session.restoreState(state);
    scheduleTick();
  }

  /// Applies one `NVP1` hot patch; `true` (and a scheduled tick) when it changed the app.
  bool applyHotPatch(Uint8List patch) {
    if (!session.applyHotPatch(patch)) {
      return false;
    }
    scheduleTick();
    return true;
  }

  /// Sends imperative command [name] to the attached app as a `Custom` event on the root node,
  /// with [args] as its structured value (codegen `NveExportSpec.commands`).
  void sendCommand(String name, [Map<String, Object?> args = const <String, Object?>{}]) =>
      dispatch([RuntimeEvent.custom(rootNodeId, name, value: args, timestampUs: _nowUs())]);

  /// Lays the surface out at [size] logical points (no-op when unchanged, empty, or with a
  /// non-finite / non-positive [devicePixelRatio]). The new size is remembered only once the
  /// runtime accepted it, so a refused one is retried on the next layout.
  void setViewport(Size size, double devicePixelRatio) {
    if (size.isEmpty ||
        !size.isFinite ||
        !devicePixelRatio.isFinite ||
        devicePixelRatio <= 0 ||
        (size == _viewport && devicePixelRatio == _scale)) {
      return;
    }
    session.setViewport(size.width, size.height, devicePixelRatio);
    _viewport = size;
    _scale = devicePixelRatio;
    scheduleTick();
  }

  /// Dispatches [events] in order and schedules the follow-up tick.
  void dispatch(List<RuntimeEvent> events) {
    if (events.isEmpty || isDisposed) {
      return;
    }
    try {
      session.dispatchEvents(events);
    } on Exception catch (e, st) {
      // RuntimeException, or a value the ABI cannot carry (BinCodecException); ArgumentError
      // (unsupported value type, lone surrogate) is a caller bug and propagates.
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
    final bool published;
    try {
      published = session.tick(dtSeconds);
    } on RuntimeException catch (e, st) {
      _report(e, st);
      return false;
    }
    // A failed drain must not hide the frame that was just published.
    var emitted = const <RuntimeEvent>[];
    try {
      emitted = session.drainEmittedEvents();
    } on Exception catch (e, st) {
      _report(e, st);
    }
    if (onAppEvent case final listener?) {
      for (final event in emitted) {
        if (isDisposed) {
          break;
        }
        listener(event);
      }
    }
    if (!published || isDisposed) {
      return false;
    }
    _frames++;
    notifyListeners();
    return true;
  }

  void _onFrame(Duration timestamp) {
    _tickScheduled = false;
    final last = _lastFrame;
    _lastDelta = last == null ? 0 : math.min((timestamp - last).inMicroseconds / 1e6, maxFrameDelta);
    // A tick that publishes nothing ends the frame chain: the next one starts from dt = 0.
    _lastFrame = tick(_lastDelta) ? timestamp : null;
  }

  void _report(Exception e, StackTrace st) {
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
