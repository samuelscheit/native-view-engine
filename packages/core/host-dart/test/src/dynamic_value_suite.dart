// Cross-platform (Dart VM + Dart-web JS) tests for `DynamicValue.fromObject`/`toObject`/`read`,
// scoped container readers, `readStrUtf8`/`readMapKeyUtf8`, and `Cursor`-shape binary round-trips.
import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';

import 'bin_codec_suite.dart' show expectKind, sameBytes;
import 'check.dart';

void runDynamicValue(Check c) {
  // 1. DynamicValue.fromObject / toObject / read round-trip across all 8 tags.
  final nestedMeta = DynamicValue.fromObject(<String, Object?>{
    'null': null,
    'flag': true,
    'neg': -42,
    'num': 2.5,
    'emoji': '\uFEFFhéllo 🚀',
    'raw': Uint8List.fromList(<int>[0x00, 0xDE, 0xAD, 0xFF]),
    'emptyList': <Object?>[],
    'emptyMap': <String, Object?>{},
    'nested': <Object?>[
      1,
      <String, Object?>{'inner': 'ok'},
      DynamicValue.fromObject(<Object?>[false, 'sub']),
    ],
  });
  c.eq(nestedMeta.tag, BinTag.map, 'fromObject root tag is map');
  final decodedObj = nestedMeta.toObject() as Map<String, Object?>;
  c.eq(decodedObj['null'], null, 'toObject null');
  c.eq(decodedObj['flag'], true, 'toObject bool');
  c.eq(decodedObj['neg'], -42, 'toObject neg int');
  c.eq(decodedObj['num'], 2.5, 'toObject double');
  c.eq(decodedObj['emoji'], '\uFEFFhéllo 🚀', 'toObject utf8+BOM+emoji');
  c.that(sameBytes(decodedObj['raw'] as Uint8List, <int>[0x00, 0xDE, 0xAD, 0xFF]), 'toObject bytes');
  c.eq((decodedObj['emptyList'] as List<Object?>).length, 0, 'toObject empty list');
  c.eq((decodedObj['emptyMap'] as Map<String, Object?>).length, 0, 'toObject empty map');
  final nestedArr = decodedObj['nested'] as List<Object?>;
  c.eq(nestedArr.length, 3, 'toObject nested array len');
  c.eq((nestedArr[1] as Map<String, Object?>)['inner'], 'ok', 'toObject nested map');
  c.eq((nestedArr[2] as List<Object?>)[1], 'sub', 'toObject embedded DynamicValue');
  c.eq(DynamicValue.fromObject(decodedObj), nestedMeta, 'fromObject(toObject()) byte-identical');

  // 2. Dart-web & VM double/int edge cases: `-0.0`, huge integral doubles (`1e20`, `maxFinite`),
  // `NaN`, `infinity`, and ±(2^53 - 1) safe integer boundaries.
  final negZero = DynamicValue.fromObject(-0.0);
  c.eq(negZero.tag, BinTag.f64, '-0.0 encodes as f64 (not i64 0)');
  final negZeroOut = negZero.toObject() as double;
  c.that(negZeroOut == 0.0 && negZeroOut.isNegative, '-0.0 preserves negative sign');

  for (final (label, val) in <(String, double)>[
    ('1e20', 1e20),
    ('-1e20', -1e20),
    ('maxFinite', double.maxFinite),
    ('-maxFinite', -double.maxFinite),
    ('infinity', double.infinity),
    ('-infinity', double.negativeInfinity),
  ]) {
    final dv = DynamicValue.fromObject(val);
    c.eq(dv.tag, BinTag.f64, '$label encodes as f64');
    c.eq(dv.toObject(), val, '$label round-trips losslessly');
  }
  final nanDv = DynamicValue.fromObject(double.nan);
  c.eq(nanDv.tag, BinTag.f64, 'NaN encodes as f64');
  c.that((nanDv.toObject() as double).isNaN, 'NaN round-trips as NaN');

  for (final safeInt in const <int>[-9007199254740991, 0, 9007199254740991]) {
    final dv = DynamicValue.fromObject(safeInt);
    c.eq(dv.tag, BinTag.i64, 'safe int $safeInt encodes as i64');
    c.eq(dv.toObject(), safeInt, 'safe int $safeInt round-trips');
  }

  // 3. Depth limit: 64 passes, 65 fails (including when triggered through an embedded DynamicValue).
  Object? deepList(int depth) {
    Object? cur;
    for (var i = 0; i < depth; i++) {
      cur = <Object?>[cur];
    }
    return cur;
  }

  final depth64 = DynamicValue.fromObject(deepList(binValueMaxDepth));
  c.eq(depth64.tag, BinTag.array, 'depth 64 allowed');
  expectKind(c, BinErrorKind.maxDepth, () => DynamicValue.fromObject(deepList(binValueMaxDepth + 1)), 'fromObject depth 65');
  expectKind(c, BinErrorKind.maxDepth, () => DynamicValue.fromObject(<Object?>[depth64]), 'nested DynamicValue exceeds depth 64');
  expectKind(c, BinErrorKind.invalidUtf8, () => DynamicValue.fromObject('\uD800'), 'fromObject lone surrogate');
  c.throwsA<ArgumentError>(() => DynamicValue.fromObject(<Object?, Object?>{1: 'bad'}), 'fromObject non-String key');
  c.throwsA<ArgumentError>(() => DynamicValue.fromObject(Object()), 'fromObject unsupported type');

  // 4. `readStrUtf8` and `readMapKeyUtf8` validate UTF-8 in place and return exact zero-copy views.
  final strW = BinWriter();
  final strMap = strW.beginMap(1);
  strW
    ..writeMapKey('\uFEFFk🚀')
    ..writeStr('\uFEFFv-é')
    ..end(strMap);
  final strR = BinReader(strW.toBytes());
  strR.readMapScope<void>((n) {
    c.eq(n, 1, 'utf8 map count');
    c.eq(decodeUtf8PreserveBom(strR.readMapKeyUtf8()), '\uFEFFk🚀', 'readMapKeyUtf8 preserves BOM+emoji');
    c.eq(decodeUtf8PreserveBom(strR.readStrUtf8()), '\uFEFFv-é', 'readStrUtf8 preserves BOM+utf8');
  });
  strR.requireDone();
  expectKind(c, BinErrorKind.invalidUtf8, () => BinReader(Uint8List.fromList(<int>[4, 2, 0, 0, 0, 0xC0, 0xAF])).readStrUtf8(), 'readStrUtf8 rejects overlong UTF-8');
  expectKind(c, BinErrorKind.unexpectedTag, () => BinReader(Uint8List.fromList(<int>[0])).readStrUtf8(), 'readStrUtf8 rejects non-str tag');
  expectKind(c, BinErrorKind.invalidUtf8, () => BinReader(Uint8List.fromList(<int>[2, 0, 0, 0, 0xC0, 0xAF])).readMapKeyUtf8(), 'readMapKeyUtf8 rejects overlong UTF-8');

  // 5. Scoped container reads: reading all elements still rejects 1 slack byte, and inner errors
  //    are preserved while restoring the outer limit.
  final slackReader = BinReader(Uint8List.fromList(<int>[6, 6, 0, 0, 0, 1, 0, 0, 0, 0, 0]));
  expectKind(
    c,
    BinErrorKind.trailingBytes,
    () => slackReader.readArrayScope<void>((n) {
      for (var i = 0; i < n; i++) {
        c.that(slackReader.readNull(), 'read the 1 array element leaving 1 slack byte');
      }
    }),
    'readArrayScope rejects 1 slack byte after reading all elements',
  );
  final escapeReader = BinReader(Uint8List.fromList(<int>[6, 5, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0]));
  expectKind(
    c,
    BinErrorKind.truncated,
    () => escapeReader.readArrayScope<void>((_) => escapeReader.readI64()),
    'readArrayScope clamps inner reads and preserves inner truncated error',
  );
  c.eq(escapeReader.remaining, 18 - escapeReader.position, 'outer limit restored after inner exception');

  // 6. `Cursor`-shape binary codec round-trip (Option, Vec<Vec<int>>, Bytes, nested DynamicValue).
  Uint8List encodeCursor({
    required int mark,
    required Uint8List w,
    required String? r,
    required int count,
    required int? tag,
    required List<List<int>> grid,
    required DynamicValue meta,
  }) {
    final bw = BinWriter();
    final m = bw.beginMap(7);
    bw
      ..writeMapKey('mark')
      ..writeI64(mark)
      ..writeMapKey('w')
      ..writeBytes(w)
      ..writeMapKey('r');
    if (r != null) {
      bw.writeStr(r);
    } else {
      bw.writeNull();
    }
    bw
      ..writeMapKey('count')
      ..writeI64(count)
      ..writeMapKey('tag');
    if (tag != null) {
      bw.writeI64(tag);
    } else {
      bw.writeNull();
    }
    bw.writeMapKey('grid');
    final gm = bw.beginArray(grid.length);
    for (final row in grid) {
      final rm = bw.beginArray(row.length);
      for (final cell in row) {
        bw.writeI64(cell);
      }
      bw.end(rm);
    }
    bw
      ..end(gm)
      ..writeMapKey('meta')
      ..raw(meta.bytes)
      ..end(m);
    return bw.toBytes();
  }

  ({int mark, Uint8List w, String? r, int count, int? tag, List<List<int>> grid, DynamicValue meta}) decodeCursor(
    Uint8List bytes,
  ) {
    final br = BinReader(bytes);
    var mark = 0;
    var w = Uint8List(0);
    String? r;
    var count = 0;
    int? tag;
    var grid = <List<int>>[];
    var meta = DynamicValue.nullValue;
    br.readMapScope<void>((n) {
      for (var i = 0; i < n; i++) {
        switch (br.readMapKey()) {
          case 'mark':
            mark = br.readI64();
          case 'w':
            w = Uint8List.fromList(br.readBytes());
          case 'r':
            r = br.readNull() ? null : br.readStr();
          case 'count':
            count = br.readI64();
          case 'tag':
            tag = br.readNull() ? null : br.readI64();
          case 'grid':
            grid = br.readArrayScope<List<List<int>>>((rows) {
              final out = <List<int>>[];
              for (var ri = 0; ri < rows; ri++) {
                out.add(br.readArrayScope<List<int>>((cols) {
                  final row = <int>[];
                  for (var ci = 0; ci < cols; ci++) {
                    row.add(br.readI64());
                  }
                  return row;
                }));
              }
              return out;
            });
          case 'meta':
            meta = DynamicValue.read(br);
          default:
            br.skipValue();
        }
      }
    });
    br.requireDone();
    return (mark: mark, w: w, r: r, count: count, tag: tag, grid: grid, meta: meta);
  }

  final cursorBytes = encodeCursor(
    mark: 7,
    w: Uint8List.fromList(<int>[1, 2, 3]),
    r: 'hello-é',
    count: 42,
    tag: -9,
    grid: const <List<int>>[<int>[1, 2], <int>[3, 4, 5]],
    meta: nestedMeta,
  );
  final cursor = decodeCursor(cursorBytes);
  c.eq(cursor.mark, 7, 'cursor.mark');
  c.that(sameBytes(cursor.w, <int>[1, 2, 3]), 'cursor.w');
  c.eq(cursor.r, 'hello-é', 'cursor.r');
  c.eq(cursor.count, 42, 'cursor.count');
  c.eq(cursor.tag, -9, 'cursor.tag');
  c.eq(cursor.grid.length, 2, 'cursor.grid rows');
  c.eq(cursor.grid[1][2], 5, 'cursor.grid[1][2]');
  c.eq(cursor.meta, nestedMeta, 'cursor.meta round-trip');

  final nullOptCursor = decodeCursor(
    encodeCursor(
      mark: 0,
      w: Uint8List(0),
      r: null,
      count: 0,
      tag: null,
      grid: const <List<int>>[],
      meta: DynamicValue.nullValue,
    ),
  );
  c.eq(nullOptCursor.r, null, 'cursor.r null');
  c.eq(nullOptCursor.tag, null, 'cursor.tag null');
  c.that(nullOptCursor.meta.isNull, 'cursor.meta null');

  for (var prefixLen = 0; prefixLen < cursorBytes.length; prefixLen++) {
    c.throwsA<BinCodecException>(
      () => decodeCursor(Uint8List.sublistView(cursorBytes, 0, prefixLen)),
      'cursor truncated prefix $prefixLen',
    );
  }
  final withTrailing = Uint8List(cursorBytes.length + 1)..setRange(0, cursorBytes.length, cursorBytes);
  expectKind(c, BinErrorKind.trailingBytes, () => decodeCursor(withTrailing), 'cursor trailing byte');
}
