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
      final textTable = t.s.findTable(NveTextTable.key);
      for (var i = 1; i < t.s.nodeCapacity; i++) {
        if (!t.s.isLive(i)) {
          continue;
        }
        n++;
        maxRight = t.rectOf(i).right > maxRight ? t.rectOf(i).right : maxRight;
        final row = t.s.rowOf(textTable, i);
        if (text == null && row >= 0 && !t.isDisplayNone(i)) {
          final s = t.s.str(textTable!, row, NveTextTable.text).trim();
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

    final target = c.withTree((t) {
      for (var i = 1; i < t.s.nodeCapacity; i++) {
        if (t.s.isLive(i) && t.eventMask(i) & nveEventMaskPress != 0) {
          final center = t.visualRect(i).center;
          if ((Offset.zero & _surface).contains(center) && t.deepestAt(center, (j) => j == i) == i) {
            return center;
          }
        }
      }
      return null;
    });
    expect(target, isNotNull, reason: 'kitchen sink exposes a visible press target');
    expect(c.hitTest(target!), isNotNull, reason: 'runtime hit-test agrees');
    await tester.tapAt(target);
    await _settle(tester, c);
    expect(seen, containsAllInOrder([NveEventKind.pointerDown, NveEventKind.pointerUp, NveEventKind.press]));
    c.sendCommand('refresh', {'n': 1, 'tags': ['a', null]});
    expect(seen.last, NveEventKind.custom);
    expect(() => c.sendCommand('bad', {'v': Object()}), throwsArgumentError);
    await _settle(tester, c);

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
