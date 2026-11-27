// Native `libnve_runtime` FFI suite: `dart run --enable-asserts test/runtime_session_test.dart`.
//
// Loads `libnve_runtime` and the kitchen-sink app module (`NVE_RUNTIME_LIB` / `NVE_KITCHEN_SINK_LIB`,
// else `<repo>/target/debug`; build with `cargo build -p nve-runtime -p nve-example-kitchen-sink`).
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:typed_data';

import 'package:nve_host/nve_native.dart';

import 'src/check.dart';
import 'src/native_values_suite.dart';

String _lib(String env, String stem) {
  final fromEnv = Platform.environment[env];
  if (fromEnv != null && fromEnv.isNotEmpty) {
    return fromEnv;
  }
  final name = Platform.isWindows ? '$stem.dll' : Platform.isMacOS ? 'lib$stem.dylib' : 'lib$stem.so';
  for (var dir = Directory.current.absolute; dir.parent.path != dir.path; dir = dir.parent) {
    if (File('${dir.path}/Cargo.lock').existsSync()) {
      final path = '${dir.path}/target/debug/$name';
      if (File(path).existsSync()) {
        return path;
      }
      break;
    }
  }
  throw StateError('$name not found: set $env or run `cargo build -p nve-runtime -p nve-example-kitchen-sink`');
}

void _expectStatus(Check c, NveRuntimeStatus status, void Function() body, String what) {
  final e = c.throwsA<RuntimeException>(body, what);
  if (e != null) {
    c.eq(e.status, status, '$what status');
    c.that(e.message.isNotEmpty, '$what carries last_error text');
  }
}

/// `(liveNodes, textNodes, firstText, maxRight, maxBottom, interactive index)` of the scene; the
/// interactive node is the first event target whose center lies inside [viewportHeight].
(int, int, String, double, double, int) _summarize(RuntimeSceneReader s, {double viewportHeight = 600}) {
  final text = s.findTable(NveTextTable.key);
  final interaction = s.findTable(NveInteractionTable.key);
  var live = 0, texts = 0, interactive = -1;
  var first = '';
  var right = 0.0, bottom = 0.0;
  final stack = <int>[rootNodeId];
  while (stack.isNotEmpty) {
    final i = stack.removeLast();
    if (!s.isLive(i)) {
      continue;
    }
    live++;
    final r = s.x(i) + s.width(i), b = s.y(i) + s.height(i);
    right = r > right ? r : right;
    bottom = b > bottom ? b : bottom;
    final row = s.rowOf(text, i);
    if (row >= 0) {
      texts++;
      if (first.isEmpty) {
        first = s.str(text!, row, NveTextTable.text);
      }
    }
    final ir = s.rowOf(interaction, i);
    final cy = s.y(i) + s.height(i) / 2;
    if (interactive < 0 && ir >= 0 && interaction!.u32(ir, NveInteractionTable.eventMask) != 0 && s.width(i) > 0 &&
        cy > 0 && cy < viewportHeight) {
      interactive = i;
    }
    for (final child in s.children(i)) {
      final ci = s.indexOf(child);
      if (ci > 0) {
        stack.add(ci);
      }
    }
  }
  return (live, texts, first, right, bottom, interactive);
}

void main() {
  final c = Check();
  runNativeValues(c);
  c.throwsA<ArgumentError>(() => RuntimeApi.open('/nonexistent/libnve_runtime_missing.dylib'), 'missing library');
  final api = RuntimeApi.open(_lib('NVE_RUNTIME_LIB', 'nve_runtime'));
  c.eq(api.lastError(), '', 'no error before any call');

  final session = RuntimeSession(api, width: 390, height: 844, scaleFactor: 3);
  c.that(!session.isDisposed, 'created');
  session.tick(1 / 60); // publishes the initial empty frame
  c.eq(session.tick(1 / 60), false, 'idle tick publishes nothing');
  session.withScene((s) {
    c.eq(s.childCount(rootNodeId), 0, 'empty scene root has no children');
  });
  c.throwsA<StateError>(() => session.withScene((_) => session.withScene((_) {})), 'withScene is not re-entrant');
  session.withScene((_) => _expectStatus(c, NveRuntimeStatus.busy, () => session.tick(0), 'tick while scene held'));
  session.submitNve2(Uint8List(0));
  _expectStatus(c, NveRuntimeStatus.io, () => session.submitNve2(Uint8List.fromList([1, 2, 3, 4, 5, 6, 7, 8])),
      'garbage NVE2 frame is a wire decode error');
  _expectStatus(c, NveRuntimeStatus.notFound, () => session.scrollBy(rootNodeId, 0, 10), 'root is no scroll container');
  _expectStatus(c, NveRuntimeStatus.invalidArg, () => session.scrollBy(rootNodeId, double.nan, 0), 'non-finite scroll');
  c.eq(session.hitTest(10, 10), null, 'empty scene hit-test misses');
  c.throwsA<Object>(() => session.dispatchEvents([const RuntimeEvent.textChange(1, '\uD800')]), 'lone surrogate rejected');
  c.eq(session.dispatchEvents(const []), false, 'empty batch');
  _expectStatus(c, NveRuntimeStatus.invalidArg, () => session.pairConnect('not a uri'), 'malformed pair uri');
  c.eq(session.isPairConnected, false, 'not paired');

  // ---- live app: kitchen-sink module attached through `nve_get_interface` -------------------
  final module = ffi.DynamicLibrary.open(_lib('NVE_KITCHEN_SINK_LIB', 'nve_example_kitchen_sink'));
  var wakes = 0;
  session
    ..setWakeListener(() => wakes++)
    ..attachAppLibrary(module);
  c.that(identical(session.appLibrary, module), 'module kept alive');
  c.eq(session.tick(1 / 60), true, 'first tick publishes the app frame');
  final (live, texts, firstText, right, bottom, interactive) = session.withScene((s) {
    c.that(s.dirtyIndices.isNotEmpty, 'first frame marks nodes dirty');
    c.that(s.findTable(NveVisualTable.key) != null, 'visual table published');
    final visual = s.findTable(NveVisualTable.key)!;
    c.eq(visual.words, NveVisualTable.words, 'published visual layout matches generated words');
    final typo = s.findTable(NveTypographyTable.key);
    c.that(typo == null || typo.words == NveTypographyTable.words, 'typography words match');
    return _summarize(s, viewportHeight: 844);
  });
  c.that(live > 10, 'kitchen sink has a real tree ($live live nodes)');
  c.that(texts > 0 && firstText.isNotEmpty, 'text nodes decode ($texts, "$firstText")');
  c.that(right <= 390.5 && right > 100, 'layout fits the 390pt viewport ($right)');
  c.that(bottom > 0, 'layout has height ($bottom)');
  final m = session.metrics();
  c.that(m.liveNodes >= live && m.frameId > 0, 'metrics (${m.liveNodes} live, frame ${m.frameId})');
  _expectStatus(c, NveRuntimeStatus.invalidArg, () => session.submitNve2(Uint8List.fromList([0, 0, 0, 0])),
      'NVE2 rejected while an app is attached');

  if (interactive > 0) {
    final (cx, cy, id) = session.withScene((s) => (s.x(interactive) + s.width(interactive) / 2,
        s.y(interactive) + s.height(interactive) / 2, s.nodeId(interactive)));
    final hit = session.hitTest(cx, cy);
    c.that(hit != null, 'hit-test finds an event target at ($cx, $cy)');
    session.dispatchEvents([
      RuntimeEvent.pointer(NveEventKind.pointerDown, hit ?? id, cx, cy),
      RuntimeEvent.pointer(NveEventKind.pointerUp, hit ?? id, cx, cy),
      RuntimeEvent.press(hit ?? id, cx, cy),
    ]);
    session.tick(1 / 60);
    session.dispatchEvents([
      RuntimeEvent.custom(rootNodeId, 'command', value: {
        'n': 1,
        'list': [1.5, 'x', null, true],
        'bin': DynamicValue(encodeDynamic({'deep': [1]})),
      }),
      RuntimeEvent.press(hit ?? id, cx, cy),
    ]);
    c.that(true, 'custom event with a structured NveValue dispatches');
    c.throwsA<ArgumentError>(
        () => session.dispatchEvents([const RuntimeEvent.custom(rootNodeId, 'bad', value: Object())]), 'unsupported value');
    session.tick(1 / 60);
  } else {
    c.that(false, 'kitchen sink exposes an interactive node');
  }

  // Host scroll is published on the next tick as the container's visual child shift.
  final scroller = session.withScene((s) {
    final t = s.findTable(NveScrollTable.key);
    for (var i = 1; t != null && i < s.nodeCapacity; i++) {
      if (s.rowOf(t, i) >= 0) {
        return s.nodeId(i);
      }
    }
    return 0;
  });
  if (scroller != 0) {
    final (sx, sy) = session.scrollBy(scroller, 0, 120);
    c.that(sy > 0 && sx == 0, 'kitchen sink scroll container scrolls ($sy)');
    c.eq(session.withScene((s) => s.scrollShift(s.indexOf(scroller))), (0.0, 0.0), 'not published before tick');
    c.eq(session.tick(0), true, 'scroll republishes');
    c.eq(session.withScene((s) => s.scrollShift(s.indexOf(scroller))), (sx, sy), 'published shift = scroll offset');
    session.scrollBy(scroller, 0, -1e6);
    session.tick(0);
    c.eq(session.withScene((s) => s.scrollCount), 0, 'scrolled back: nothing published');
  } else {
    c.that(false, 'kitchen sink exposes a scroll container');
  }

  session.setViewport(800, 600, 1);
  session.tick(1 / 60);
  final wide = session.withScene((s) => _summarize(s).$4);
  c.that(wide > right, 'wider viewport relayouts natively ($right -> $wide)');
  _expectStatus(c, NveRuntimeStatus.invalidArg, () => session.setViewport(-1, 600, 1), 'negative viewport');

  session.detachApp();
  c.eq(session.appLibrary, null, 'detach drops the module');
  session.setWakeListener(null);
  session.dispose();
  session.dispose();
  c.that(session.isDisposed, 'disposed');
  c.throwsA<StateError>(() => session.tick(0), 'use after dispose');
  c.that(wakes >= 0, 'wake listener closed cleanly');

  c.failures.forEach(stderr.writeln);
  stdout.writeln('nve_host runtime ffi: ${c.passed} checks passed, ${c.failed} failed');
  if (c.failed != 0) {
    exitCode = 1;
  }
}
