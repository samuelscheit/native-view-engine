// Live `libnve_runtime` + kitchen-sink app through NveCanvasController / NveCanvasView: layout at
// the widget size, painting, semantics, taps, wheel and drag scrolling, and teardown.
// Libraries: `NVE_RUNTIME_LIB` / `NVE_KITCHEN_SINK_LIB`, else `<repo>/target/debug`
// (`cargo build -p nve-runtime -p nve-example-kitchen-sink`; `scripts/host-runtimes.sh flutter`).
import 'dart:ffi' as ffi;
import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

String _lib(String env, String stem) {
  final fromEnv = Platform.environment[env];
  if (fromEnv != null && fromEnv.isNotEmpty) {
    return fromEnv;
  }
  final name = Platform.isWindows ? '$stem.dll' : Platform.isMacOS ? 'lib$stem.dylib' : 'lib$stem.so';
  final path = '${Directory.current.path}/../../target/debug/$name';
  if (!File(path).existsSync()) {
    throw StateError('$name not found: set $env or run `cargo build -p nve-runtime -p nve-example-kitchen-sink`');
  }
  return path;
}

const Size _surface = Size(390, 600);

Widget _host(NveCanvasController c) => Directionality(
      textDirection: TextDirection.ltr,
      child: Align(
        alignment: Alignment.topLeft,
        child: SizedBox.fromSize(size: _surface, child: NveCanvasView(controller: c)),
      ),
    );

Future<void> _settle(WidgetTester tester, NveCanvasController c) async {
  for (var i = 0; i < 6; i++) {
    await tester.pump(const Duration(milliseconds: 16));
  }
  expect(c.lastError, isNull);
}

/// Every text of the published scene in slot order.
String _texts(NveSceneTree t) {
  final table = t.s.findTable(TextTable.key);
  return [
    for (var i = 1; i < t.s.nodeCapacity; i++)
      if (t.s.rowOf(table, i) case final r when r >= 0) t.s.str(table!, r, TextTable.text),
  ].join('\u0000');
}

void main() {
  late NveCanvasController c;

  setUp(() {
    c = NveCanvasController.open(
      runtimeLibrary: _lib('NVE_RUNTIME_LIB', 'nve_runtime'),
      app: ffi.DynamicLibrary.open(_lib('NVE_KITCHEN_SINK_LIB', 'nve_example_kitchen_sink')),
    );
  });

  tearDown(() {
    if (!c.isDisposed) {
      c.dispose();
    }
  });

  testWidgets('lays out at the widget size, paints, and exposes semantics', (tester) async {
    final semantics = tester.ensureSemantics();
    await tester.pumpWidget(_host(c));
    await _settle(tester, c);
    expect(c.frameCount, greaterThan(0));
    final (live, right, firstText) = c.withTree((t) {
      var n = 0, maxRight = 0.0;
      String? text;
      final textTable = t.s.findTable(TextTable.key);
      for (var i = 1; i < t.s.nodeCapacity; i++) {
        if (!t.s.isLive(i)) {
          continue;
        }
        n++;
        maxRight = t.rectOf(i).right > maxRight ? t.rectOf(i).right : maxRight;
        final row = t.s.rowOf(textTable, i);
        if (text == null && row >= 0 && !t.isDisplayNone(i)) {
          final s = t.s.str(textTable!, row, TextTable.text).trim();
          text = s.isEmpty ? null : s;
        }
      }
      return (n, maxRight, text);
    });
    expect(live, greaterThan(10));
    expect(right, lessThanOrEqualTo(_surface.width + 0.5), reason: 'runtime laid out at the widget width');
    expect(firstText, isNotNull);

    final labels = <String>[];
    void walk(SemanticsNode n) {
      labels.add(n.label);
      n.visitChildren((k) {
        walk(k);
        return true;
      });
    }

    walk(tester.getSemantics(find.byType(NveCanvasView)));
    expect(labels.where((l) => l.isNotEmpty).length, greaterThan(3));
    expect(labels.any((l) => l.contains(firstText!)), isTrue, reason: '"$firstText" in $labels');
    semantics.dispose();
  });

  testWidgets('taps dispatch press events and wheel / drag scroll the container', (tester) async {
    final seen = <NveEventKind>[];
    c.onEvent = (e) => seen.add(e.kind);
    await tester.pumpWidget(_host(c));
    await _settle(tester, c);

    final targets = c.withTree((t) => [
          for (var i = 1; i < t.s.nodeCapacity; i++)
            if (t.s.isLive(i) && t.eventMask(i) & nveEventMaskPress != 0)
              if (t.visualRect(i).center case final center
                  when (Offset.zero & _surface).contains(center) && t.deepestAt(center, (j) => j == i) == i)
                center,
        ]);
    expect(targets, isNotEmpty, reason: 'kitchen sink exposes visible press targets');
    expect(c.hitTest(targets.first), isNotNull, reason: 'runtime hit-test agrees');
    // Idle for 2 s: the first tick after it must not advance animations by the wall-clock gap.
    await tester.pump(const Duration(seconds: 2));
    final before = c.withTree(_texts);
    final framesBefore = c.frameCount;
    var changed = false;
    for (final target in targets.take(30)) {
      await tester.tapAt(target);
      await tester.pump(const Duration(milliseconds: 16));
      expect(c.lastFrameDelta, lessThanOrEqualTo(NveCanvasController.maxFrameDelta));
      await _settle(tester, c);
      if (c.withTree(_texts) != before) {
        changed = true;
        break;
      }
    }
    expect(seen, containsAllInOrder([NveEventKind.pointerDown, NveEventKind.pointerUp, NveEventKind.press]));
    expect(changed, isTrue, reason: 'a tap reaches the app and changes the painted text');
    expect(c.frameCount, greaterThan(framesBefore));
    expect(() => c.sendCommand('bad', {'v': Object()}), throwsArgumentError);

    final (scroller, at) = c.withTree((t) {
      for (var i = 1; i < t.s.nodeCapacity; i++) {
        if (t.s.isLive(i) && t.isScrollContainer(i) && !t.isDisplayNone(i)) {
          final r = t.visualRect(i).intersect(Offset.zero & _surface);
          if (!r.isEmpty && t.contentSize(i).height > t.rectOf(i).height + 50) {
            return (t.s.nodeId(i), r.center);
          }
        }
      }
      return (0, Offset.zero);
    });
    expect(scroller, isNot(0), reason: 'kitchen sink has a visible overflowing scroll container');
    expect(c.scrollableAt(at), isNotNull);
    double shiftY() => c.withTree((t) => t.scrollShift(t.s.indexOf(scroller)).dy);

    seen.clear();
    final pointer = TestPointer(1, PointerDeviceKind.mouse);
    await tester.sendEventToBinding(pointer.hover(at));
    await tester.sendEventToBinding(pointer.scroll(const Offset(0, 80)));
    await _settle(tester, c);
    final afterWheel = shiftY();
    expect(afterWheel, greaterThan(0), reason: 'wheel scroll published as a visual shift');
    expect(seen, contains(NveEventKind.scroll));

    seen.clear();
    final drag = await tester.startGesture(at);
    await drag.moveBy(const Offset(0, -30));
    await drag.moveBy(const Offset(0, -30));
    await drag.up();
    await _settle(tester, c);
    expect(shiftY(), greaterThan(afterWheel), reason: 'drag past slop scrolls content up');
    expect(seen, isNot(contains(NveEventKind.press)), reason: 'a drag is no tap');

    await tester.sendEventToBinding(pointer.scroll(const Offset(0, -1e6)));
    await _settle(tester, c);
    expect(shiftY(), 0, reason: 'clamped back to the top');

    final trackpad = TestPointer(2, PointerDeviceKind.trackpad);
    await tester.sendEventToBinding(trackpad.panZoomStart(at));
    await tester.sendEventToBinding(trackpad.panZoomUpdate(at, pan: const Offset(0, -40)));
    await tester.sendEventToBinding(trackpad.panZoomUpdate(at, pan: const Offset(0, -90)));
    await tester.sendEventToBinding(trackpad.panZoomEnd());
    await _settle(tester, c);
    expect(shiftY(), closeTo(90, 0.5), reason: 'two-finger trackpad pan scrolls by the pan distance');
  });

  testWidgets('controller swap and disposal tear down cleanly', (tester) async {
    await tester.pumpWidget(_host(c));
    await _settle(tester, c);
    final other = NveCanvasController.open(runtimeLibrary: _lib('NVE_RUNTIME_LIB', 'nve_runtime'));
    await tester.pumpWidget(_host(other));
    await _settle(tester, other);
    expect(other.withTree((t) => t.root), -1, reason: 'no app attached: empty frame');
    c.dispose();
    await tester.pump();
    await tester.pumpWidget(const SizedBox());
    other.dispose();
    expect(other.isDisposed, isTrue);
    expect(other.hitTest(Offset.zero), isNull);
    other.scheduleTick();
    expect(other.tick(), isFalse);
  });
}
