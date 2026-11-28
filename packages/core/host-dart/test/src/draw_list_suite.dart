import 'dart:typed_data';

import 'package:nve_host/nve_draw_list.generated.dart';
import 'package:nve_host/nve_host.dart';

import 'check.dart';

Uint8List _words(List<int> words) {
  final Uint8List out = Uint8List(words.length * 4);
  final ByteData view = ByteData.sublistView(out);
  for (int i = 0; i < words.length; i++) {
    view.setUint32(i * 4, words[i], Endian.little);
  }
  return out;
}

int _f32(double v) => (ByteData(4)..setFloat32(0, v, Endian.little)).getUint32(0, Endian.little);

int _rgba(int r, int g, int b, [int a = 255]) =>
    (r & 0xFF) | ((g & 0xFF) << 8) | ((b & 0xFF) << 16) | ((a & 0xFF) << 24);

class _LogSink implements NveDrawListSink {
  final List<String> ops = <String>[];

  @override
  void onClear(int color) {
    ops.add('clear:$color');
  }

  @override
  void onFillRect(double x, double y, double w, double h, int color) {
    ops.add('rect:$x,$y,$w,$h,$color');
  }

  @override
  void onFillRoundedRect(double x, double y, double w, double h, double radius, int color) {
    ops.add('rrect:$x,$y,$w,$h,$radius,$color');
  }

  @override
  void onFillCircle(double cx, double cy, double radius, int color) {
    ops.add('circle:$cx,$cy,$radius,$color');
  }

  @override
  void onStrokeLine(double x0, double y0, double x1, double y1, double width, int color) {
    ops.add('line:$x0,$y0,$x1,$y1,$width,$color');
  }

  @override
  void onStrokePolylineStart(int count, double width, int color) {
    ops.add('poly:$count,$width,$color');
  }

  @override
  void onStrokePolylinePoint(int index, double x, double y) {
    ops.add('pt$index:$x,$y');
  }

  @override
  void onStrokePolylineEnd() {
    ops.add('polyEnd');
  }

  @override
  void onClipRect(double x, double y, double w, double h) {
    ops.add('clip:$x,$y,$w,$h');
  }

  @override
  void onRestoreClip() {
    ops.add('restore');
  }
}

final class _ThrowingClipSink extends _LogSink {
  @override
  void onClipRect(double x, double y, double w, double h) {
    super.onClipRect(x, y, w, h);
    if (ops.length == 2) {
      throw StateError('paint boom');
    }
  }
}

Uint8List _all8Ops() {
  final int c1 = _rgba(1, 2, 3, 4);
  final int white = _rgba(255, 255, 255, 255);
  return _words(<int>[
    8,
    NveDrawList.opClear, c1,
    NveDrawList.opFillRect, _f32(1.0), _f32(2.0), _f32(3.0), _f32(4.0), white,
    NveDrawList.opFillRoundedRect, _f32(0.0), _f32(0.0), _f32(10.0), _f32(10.0), _f32(2.5), white,
    NveDrawList.opFillCircle, _f32(5.0), _f32(6.0), _f32(3.0), white,
    NveDrawList.opStrokeLine, _f32(0.0), _f32(0.0), _f32(3.0), _f32(4.0), _f32(2.0), white,
    NveDrawList.opStrokePolyline, _f32(1.5), white, 3,
    _f32(0.0), _f32(0.0), _f32(5.0), _f32(7.5), _f32(-2.0), _f32(12.0),
    NveDrawList.opClipRect, _f32(1.0), _f32(2.0), _f32(30.0), _f32(40.0),
    NveDrawList.opRestoreClip,
  ]);
}

/// Golden vector parity and `MAX_CLIP_DEPTH = 64` balancing for generated `NveDrawList` (VM + JS).
void runDrawList(Check c) {
  c.that(NveDrawList.kind == Key64.of('nve.res.draw_list'), 'draw list kind key');
  c.eq(NveDrawList.headerSize, 4, 'header size');
  c.eq(NveDrawList.maxDrawOps, 65536, 'maxDrawOps');
  c.eq(NveDrawList.maxPolylinePoints, 16384, 'maxPolylinePoints');
  c.eq(NveDrawList.maxClipDepth, 64, 'maxClipDepth');

  final Uint8List all8 = _all8Ops();
  c.eq(all8.lengthInBytes, 44 * 4, 'exact 44-word FlatResource byte length');

  // Subview with non-zero byte offset decodes cleanly.
  final Uint8List padded = Uint8List(all8.length + 7);
  padded.setRange(3, 3 + all8.length, all8);
  final Uint8List sliced = Uint8List.sublistView(padded, 3, 3 + all8.length);
  final _LogSink log = _LogSink();
  c.that(NveDrawList.decode(sliced, log) == null, 'all 8 ops decode from offset subview');
  c.eq(NveDrawList.opCount(sliced), 8, 'opCount');
  c.eq(log.ops.length, 12, '8 ops + 3 polyline points + polyEnd');
  c.that(log.ops[5] == 'poly:3,1.5,${_rgba(255, 255, 255, 255)}', 'polyline header');
  c.that(log.ops[7] == 'pt1:5.0,7.5' || log.ops[7] == 'pt1:5,7.5', 'polyline vertex 1');
  c.that(log.ops[11] == 'restore', 'trailing restore');

  // Empty stream, 0-pt polyline, and -0.0 non-negative float are valid.
  c.that(NveDrawList.validate(_words(<int>[0])) == null, 'empty stream valid');
  c.that(
    NveDrawList.validate(_words(<int>[1, NveDrawList.opStrokePolyline, _f32(-0.0), 0, 0])) == null,
    '0-pt polyline with -0.0 width valid',
  );
  c.that(
    NveDrawList.validate(_words(<int>[1, NveDrawList.opFillRect, _f32(-1.0), _f32(-2.0), _f32(-3.0), _f32(-4.0), 0])) == null,
    'negative rect coords valid',
  );

  // Truncation, trailing, non-4B lengths, limits, unknown opcodes, non-finite & negative floats.
  for (int cut = 0; cut < all8.length; cut++) {
    c.eq(NveDrawList.validate(Uint8List.sublistView(all8, 0, cut)), NveDrawListError.truncated, 'cut $cut');
  }
  for (int extra = 1; extra <= 4; extra++) {
    final Uint8List withExtra = Uint8List(all8.length + extra)..setRange(0, all8.length, all8);
    c.eq(NveDrawList.validate(withExtra), NveDrawListError.trailingBytes, 'trailing $extra');
  }
  c.eq(NveDrawList.validate(_words(<int>[NveDrawList.maxDrawOps + 1])), NveDrawListError.opCountExceedsLimit, 'op limit');
  c.eq(NveDrawList.validate(_words(<int>[0xFFFFFFFF])), NveDrawListError.opCountExceedsLimit, 'u32 max op limit');
  c.eq(NveDrawList.validate(_words(<int>[1, 0])), NveDrawListError.unknownOpcode, 'opcode 0');
  c.eq(NveDrawList.validate(_words(<int>[1, 9])), NveDrawListError.unknownOpcode, 'opcode 9');
  c.eq(NveDrawList.validate(_words(<int>[1, 0x00010001])), NveDrawListError.unknownOpcode, 'high-bit opcode');
  c.eq(
    NveDrawList.validate(_words(<int>[1, NveDrawList.opStrokePolyline, _f32(1.0), 0, NveDrawList.maxPolylinePoints + 1])),
    NveDrawListError.sliceLenExceedsLimit,
    'polyline limit',
  );
  c.eq(
    NveDrawList.validate(_words(<int>[1, NveDrawList.opFillRect, _f32(double.nan), 0, 0, 0, 0])),
    NveDrawListError.nonFiniteFloat,
    'NaN rect',
  );
  c.eq(
    NveDrawList.validate(_words(<int>[1, NveDrawList.opFillCircle, 0, 0, _f32(double.negativeInfinity), 0])),
    NveDrawListError.nonFiniteFloat,
    '-Inf radius is nonFiniteFloat',
  );
  c.eq(
    NveDrawList.validate(_words(<int>[1, NveDrawList.opFillRoundedRect, 0, 0, _f32(10.0), _f32(10.0), _f32(-0.5), 0])),
    NveDrawListError.negativeFloat,
    'negative rounded rect radius',
  );
  c.eq(
    NveDrawList.validate(_words(<int>[1, NveDrawList.opFillCircle, 0, 0, _f32(-1.0), 0])),
    NveDrawListError.negativeFloat,
    'negative circle radius',
  );
  c.eq(
    NveDrawList.validate(_words(<int>[1, NveDrawList.opStrokeLine, 0, 0, _f32(1.0), _f32(1.0), _f32(-0.25), 0])),
    NveDrawListError.negativeFloat,
    'negative line width',
  );
  c.eq(
    NveDrawList.validate(_words(<int>[1, NveDrawList.opStrokePolyline, _f32(-2.0), 0, 0])),
    NveDrawListError.negativeFloat,
    'negative polyline width',
  );

  // Malformed second op triggers zero callbacks.
  final _LogSink zero = _LogSink();
  c.eq(
    NveDrawList.decode(_words(<int>[2, NveDrawList.opClear, 1, NveDrawList.opFillCircle, 0, 0, _f32(-1.0), 0]), zero),
    NveDrawListError.negativeFloat,
    'bad second op rejected',
  );
  c.eq(zero.ops.length, 0, 'zero callbacks on malformed stream');

  // 66 nested clips + 2 restores + clear: 2 restores absorb overflow, 64 unwind after clear.
  final List<int> deepWords = <int>[NveDrawList.maxClipDepth + 2 + 2 + 1];
  for (int i = 0; i < NveDrawList.maxClipDepth + 2; i++) {
    deepWords.addAll(<int>[NveDrawList.opClipRect, _f32(0.0), _f32(0.0), _f32(10.0), _f32(10.0)]);
  }
  deepWords.addAll(<int>[NveDrawList.opRestoreClip, NveDrawList.opRestoreClip, NveDrawList.opClear, 7]);
  final _LogSink deepLog = _LogSink();
  c.that(NveDrawList.decode(_words(deepWords), deepLog) == null, '66 clips + 2 restores valid');
  c.eq(deepLog.ops.length, NveDrawList.maxClipDepth * 2 + 1, '64 clips + 1 clear + 64 unwind restores');
  c.eq(deepLog.ops[NveDrawList.maxClipDepth], 'clear:7', '2 overflow restores do not pop clip #64');

  // Throwing sink still unwinds all already-opened clips via finally.
  final Uint8List twoClips = _words(<int>[
    3,
    NveDrawList.opRestoreClip,
    NveDrawList.opClipRect, _f32(1.0), _f32(2.0), _f32(3.0), _f32(4.0),
    NveDrawList.opClipRect, _f32(1.0), _f32(2.0), _f32(3.0), _f32(4.0),
  ]);
  final _ThrowingClipSink throwing = _ThrowingClipSink();
  bool caught = false;
  try {
    NveDrawList.decode(twoClips, throwing);
  } on StateError {
    caught = true;
  }
  c.that(caught, 'sink exception propagates');
  c.eq(throwing.ops.length, 4, '2 clips + 2 unwind restores');
  c.eq(throwing.ops[2], 'restore', 'first unwind restore');
  c.eq(throwing.ops[3], 'restore', 'second unwind restore');
}
