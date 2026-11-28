// Native `libnve_runtime` FFI suite: `dart run --enable-asserts test/runtime_session_test.dart`.
//
// Loads `libnve_runtime`, the kitchen-sink app module, and the echo fixture app
// (`NVE_RUNTIME_LIB` / `NVE_KITCHEN_SINK_LIB` / `NVE_HOST_FIXTURE_APP_LIB`, else `<repo>/target/debug`;
// build with `cargo build -p nve-runtime -p nve-example-kitchen-sink -p nve-host-fixtures`).
import 'dart:convert' show jsonEncode;
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';
import 'package:nve_host/nve_host_ffi.dart';

import 'src/check.dart';
import 'src/runtime_ingress_suite.dart';

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
  throw StateError(
      '$name not found: set $env or run `cargo build -p nve-runtime -p nve-example-kitchen-sink -p nve-host-fixtures`');
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
(int, int, String, double, double, int) _summarize(SceneReader s, {double viewportHeight = 600}) {
  final text = s.findTable(TextTable.key);
  final interaction = s.findTable(InteractionTable.key);
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
        first = s.str(text!, row, TextTable.text);
      }
    }
    final ir = s.rowOf(interaction, i);
    final cy = s.y(i) + s.height(i) / 2;
    if (interactive < 0 && ir >= 0 && interaction!.u32(ir, InteractionTable.eventMask) != 0 && s.width(i) > 0 &&
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

/// Every text of the scene in tree order: a press that re-renders visibly changes it.
String _textDigest(SceneReader s) {
  final text = s.findTable(TextTable.key);
  final out = StringBuffer();
  void walk(int i) {
    final row = s.rowOf(text, i);
    if (row >= 0) {
      out
        ..write(s.str(text!, row, TextTable.text))
        ..write('\u0000');
    }
    for (final child in s.children(i)) {
      final ci = s.indexOf(child);
      if (ci > 0) {
        walk(ci);
      }
    }
  }

  if (s.isLive(rootNodeId)) {
    walk(rootNodeId);
  }
  return out.toString();
}

/// Visible press targets `(raw NodeId, center x, center y)`.
List<(int, double, double)> _pressTargets(SceneReader s, double viewportHeight) {
  final interaction = s.findTable(InteractionTable.key);
  return [
    for (var i = 1; i < s.nodeCapacity; i++)
      if (s.rowOf(interaction, i) case final r when r >= 0)
        if (interaction!.u32(r, InteractionTable.eventMask) & 1 != 0 &&
            s.width(i) > 0 &&
            s.y(i) + s.height(i) / 2 > 0 &&
            s.y(i) + s.height(i) / 2 < viewportHeight)
          (s.nodeId(i), s.x(i) + s.width(i) / 2, s.y(i) + s.height(i) / 2),
  ];
}

bool _sameBytes(Uint8List a, Uint8List b) {
  if (a.length != b.length) {
    return false;
  }
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) {
      return false;
    }
  }
  return true;
}

/// Resource kind of each core table's resource field(s), by table name.
const _resourceKinds = {
  'nve.grid': 'nve.res.grid_tracks',
  'nve.visual': 'nve.res.box_shadows|nve.res.background_fill',
  'nve.motion': 'nve.res.transitions',
  'nve.responsive': 'nve.res.responsive_rules',
  'nve.interaction': 'nve.res.state_rules',
  'nve.accessibility': 'nve.res.aria_relations',
  'nve.image': 'nve.res.image_src',
};

/// Slice 2B-0 embedding-host hooks against the kitchen sink.
void _runtimeHooks(Check c, RuntimeSession session) {
  // ---- resource_payload: every published resource word resolves inside the scene read -----
  c.throwsA<StateError>(() => session.resourcePayload(1), 'resource payload outside withScene');
  final resolved = session.withScene((s) {
    c.eq(session.resourcePayload(0), null, 'resource id 0 is absent');
    c.eq(session.resourcePayload(0xFFFFFF), null, 'unknown resource id is NOT_FOUND');
    var n = 0;
    for (final info in nveTables) {
      final table = s.findTable(info.key);
      if (table == null) {
        continue;
      }
      final kinds = (_resourceKinds[info.name] ?? '').split('|').map(Key64.of).toSet();
      for (final field in info.fields.where((f) => f.kind == FieldKind.resource)) {
        for (var row = 0; row < table.rowCount; row++) {
          final id = table.resource(row, field);
          if (id == 0) {
            continue;
          }
          final got = session.resourcePayload(id);
          final again = session.resourcePayload(id);
          if (got == null || again == null) {
            c.that(false, '${info.name} resource $id resolves');
            continue;
          }
          c.that(kinds.contains(got.$1), '${info.name} resource $id kind ${got.$1}');
          c.that(got.$2.isNotEmpty, '${info.name} resource $id payload is non-empty');
          c.that(_sameBytes(again.$2, got.$2), '${info.name} resource $id payload is stable within one read');
          n++;
        }
      }
    }
    return n;
  });
  c.that(resolved > 0, 'kitchen sink publishes resources ($resolved resolved)');

  // ---- set_props / drain_emitted_events / snapshot / hot patch ------------------------------
  c.eq(session.setProps(const {}), false, 'empty props change nothing');
  c.eq(session.setProps({'unknown': 1, 'nested': {'a': [1, 'b']}}), false, 'unknown props change nothing');
  c.eq(session.drainEmittedEvents(), const <RuntimeEvent>[], 'kitchen sink emits no events');
  final snapshot = session.snapshotState();
  c.that(snapshot.isNotEmpty, 'snapshot carries app state (${snapshot.length} bytes)');
  session.restoreState(snapshot);
  session.tick(0);
  c.that(_sameBytes(session.snapshotState(), snapshot), 'restore -> snapshot round-trips');
  _expectStatus(c, NveRuntimeStatus.invalidArg, () => session.applyHotPatch(Uint8List(4)), 'malformed NVP1 patch');

  // ---- state files (crash-atomic StateFile round trip) --------------------------------------
  final dir = Directory.systemTemp.createTempSync('nve_state_');
  try {
    final path = '${dir.path}/app.nve';
    c.eq(session.restoreStateFile(path, 1), false, 'missing state file restores nothing');
    c.eq(session.saveStateFile(path, 1), true, 'kitchen sink state saved');
    c.that(File(path).existsSync(), 'state file written');
    c.eq(session.restoreStateFile(path, 1), true, 'state file restores');
    session.tick(0);
    c.that(_sameBytes(session.snapshotState(), snapshot), 'file restore -> snapshot round-trips');
    session.clearStateFile(path);
    session.clearStateFile(path);
    c.that(!File(path).existsSync(), 'clear deletes the file (idempotent)');
  } finally {
    dir.deleteSync(recursive: true);
  }
}

/// Real app-emitted events across the runtime C ABI: `libnve_host_fixture_app` echoes props as a
/// structured `Custom` event and, on `malformed`, first emits one whose name is invalid UTF-8.
void _fixtureAppEvents(Check c, RuntimeApi api) {
  final session = RuntimeSession(api, surfaceId: 7);
  try {
    session.attachAppLibrary(ffi.DynamicLibrary.open(_lib('NVE_HOST_FIXTURE_APP_LIB', 'nve_host_fixture_app')));
    session.tick(0);
    c.eq(session.drainEmittedEvents(), const <RuntimeEvent>[], 'nothing emitted before props');
    c.eq(session.setProps({'count': 3, 'label': 'hi'}), true, 'new props re-render the fixture app');
    final events = session.drainEmittedEvents();
    c.eq(events.length, 1, 'one emitted event');
    if (events case [final e]) {
      c.eq(e.kind, NveEventKind.custom, 'custom kind');
      c.eq(e.payload, NveEventPayloadTag.custom, 'custom payload');
      c.eq((e.name, e.nodeId), ('props', 1), 'name + node');
      c.eq(jsonEncode(e.value), '{"count":2,"keys":["count","label"],"ok":true,"ratio":0.5}', 'structured value');
    }
    c.eq(session.drainEmittedEvents(), const <RuntimeEvent>[], 'draining is destructive');
    c.eq(session.setProps({'count': 3, 'label': 'hi'}), false, 'identical props change nothing');
    c.eq(session.setProps({'malformed': true}), true, 'malformed request re-renders');
    final mixed = session.drainEmittedEvents();
    c.eq(mixed.map((e) => e.name).join('|'), 'bad\uFFFDname|props', 'malformed name decodes lossy, batch intact');
    if (mixed.length == 2) {
      c.eq(jsonEncode(mixed[1].value), '{"count":1,"keys":["malformed"],"ok":true,"ratio":0.5}', 'second event intact');
    }
    // Wake listener swaps: re-registering and clearing never leaves a dangling callable.
    session.setWakeListener(() {});
    session.setWakeListener(() {});
    session.setWakeListener(null);
  } finally {
    session.dispose();
  }
}

void main() {
  final c = Check();
  runRuntimeIngress(c);
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
  c.throwsA<StateError>(() => session.withScene((_) => session.dispose()), 'dispose inside withScene');
  c.that(!session.isDisposed, 'rejected dispose leaves the session live');
  c.eq(session.withScene((s) => s.nodeCapacity) >= 0, true, 'scene still readable after the rejected dispose');
  final first = session.withScene((s) => s);
  c.that(identical(session.withScene((s) => s), first), 'unchanged publication reuses the validated reader');
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
    c.that(s.findTable(VisualTable.key) != null, 'visual table published');
    final visual = s.findTable(VisualTable.key)!;
    c.eq(visual.words, VisualTable.words, 'published visual layout matches generated words');
    final typo = s.findTable(TypographyTable.key);
    c.that(typo == null || typo.words == TypographyTable.words, 'typography words match');
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
    c.eq(hit, id, 'runtime hit-test resolves the press target');
    // A press reaches the app: some visible press target re-renders and changes the scene's text.
    final before = session.withScene(_textDigest);
    final targets = session.withScene((s) => _pressTargets(s, 844));
    var rerendered = false, changed = false;
    for (final (target, x, y) in targets.take(40)) {
      rerendered = session.dispatchEvents([
            RuntimeEvent.pointer(NveEventKind.pointerDown, target, x, y),
            RuntimeEvent.pointer(NveEventKind.pointerUp, target, x, y),
            RuntimeEvent.press(target, x, y),
          ]) ||
          rerendered;
      session.tick(1 / 60);
      if (session.withScene(_textDigest) != before) {
        changed = true;
        break;
      }
    }
    c.that(rerendered, 'a press requests a re-render (${targets.length} targets)');
    c.that(changed, 'a press changes the published scene text');
    // Structured values cross the ABI (status ok); their app-side round-trip is asserted once
    // `drain_emitted_events` (Slice 2B-0) exposes app-emitted events.
    session.dispatchEvents([
      RuntimeEvent.custom(rootNodeId, 'command', value: {
        'n': 1,
        'list': [1.5, 'x', null, true],
        'bin': DynamicValue.fromObject({'deep': [1]}),
      }),
    ]);
    c.throwsA<ArgumentError>(
        () => session.dispatchEvents([const RuntimeEvent.custom(rootNodeId, 'bad', value: Object())]), 'unsupported value');
    session.tick(1 / 60);
  } else {
    c.that(false, 'kitchen sink exposes an interactive node');
  }

  // Host scroll is published on the next tick as the container's visual child shift.
  final scroller = session.withScene((s) {
    final t = s.findTable(ScrollTable.key);
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
    c.eq(session.withScene((s) => s.scrollShift(s.indexOf(scroller))), (0.0, 0.0), 'scrolled back: zero shift');
  } else {
    c.that(false, 'kitchen sink exposes a scroll container');
  }

  _runtimeHooks(c, session);
  _fixtureAppEvents(c, api);

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
  // The kitchen sink has no async work and nothing is paired here, so the runtime never wakes this
  // session: `wakes` stays 0. Wake delivery from pair frames is asserted natively
  // (`packages/core/runtime/tests/pair_integration.rs`).
  c.eq(wakes, 0, 'no spurious wakes from a synchronous app');

  c.failures.forEach(stderr.writeln);
  stdout.writeln('nve_host runtime ffi: ${c.passed} checks passed, ${c.failed} failed');
  if (c.failed != 0) {
    exitCode = 1;
  }
}
