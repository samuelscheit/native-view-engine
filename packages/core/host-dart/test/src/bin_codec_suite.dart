import 'dart:convert' show utf8;
import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';

import 'check.dart';

/// Validated by Rust `BinValueView::parse`: {"k": [1, "é", null, true, 1.5, b"\xFF"]}.
const String goldenHex = '07 34000000 01000000 01000000 6b 06 26000000 06000000 02 0100000000000000 04 02000000 c3a9 00 '
    '0101 03 000000000000f83f 05 01000000 ff';

Uint8List fromHex(String hex) {
  final digits = hex.replaceAll(' ', '');
  return Uint8List.fromList(
    List<int>.generate(digits.length ~/ 2, (i) => int.parse(digits.substring(2 * i, 2 * i + 2), radix: 16)),
  );
}

bool sameBytes(List<int> a, List<int> b) {
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

void expectKind(Check c, BinErrorKind kind, void Function() body, String what) {
  final e = c.throwsA<BinCodecException>(body, what);
  if (e != null) {
    c.eq(e.kind, kind, '$what kind');
  }
}

Uint8List nested(int depth) {
  final w = BinWriter();
  final marks = <int>[for (var i = 0; i < depth; i++) w.beginArray(1)];
  w.writeNull();
  for (final m in marks.reversed) {
    w.end(m);
  }
  return w.toBytes();
}

void runBinCodec(Check c) {
  final golden = fromHex(goldenHex);

  final w = BinWriter(4); // forces growth
  final map = w.beginMap(1);
  w.writeMapKey('k');
  final arr = w.beginArray(6);
  w
    ..writeI64(1)
    ..writeStr('é')
    ..writeNull()
    ..writeBool(true)
    ..writeF64(1.5)
    ..writeBytes(<int>[0xFF]);
  w
    ..end(arr)
    ..end(map);
  c.that(sameBytes(w.bytes, golden), 'golden encoding');
  BinReader.validate(golden);
  final r = BinReader(golden);
  c.eq(r.beginMap(), 1, 'map count');
  c.eq(r.readMapKey(), 'k', 'map key');
  c.eq(r.beginArray(), 6, 'array count');
  c.eq(r.readI64(), 1, 'i64');
  c.eq(r.readStr(), 'é', 'str');
  c.that(r.readNull(), 'null');
  c.that(!r.readNull(), 'non-null stays');
  c.that(r.readBool(), 'bool');
  c.eq(r.readF64(), 1.5, 'f64');
  c.that(sameBytes(r.readBytes(), <int>[0xFF]), 'bytes');
  c.that(r.isAtEnd, 'at end');

  // Primitives and exact 64-bit values (beyond 2^53) without getInt64 / getUint64.
  final p = BinWriter()
    ..u8(0xAA)
    ..u16(0xBEEF)
    ..u32(0xDEADBEEF)
    ..u64(const Key64(0x89ABCDEF, 0x01234567))
    ..i32(-2)
    ..f32(-0.0)
    ..f64(double.infinity)
    ..writeI64(-7)
    ..writeI64Bits(const Key64(0, 0x80000000)) // i64::MIN
    ..writeI64Bits(const Key64(0xFFFFFFFF, 0x7FFFFFFF)) // i64::MAX
    ..writeF64(double.nan);
  final pr = BinReader(p.bytes);
  c.eq(pr.u8(), 0xAA, 'u8');
  c.eq(pr.u16(), 0xBEEF, 'u16');
  c.eq(pr.u32(), 0xDEADBEEF, 'u32');
  c.eq(pr.u64(), const Key64(0x89ABCDEF, 0x01234567), 'u64');
  c.eq(pr.i32(), -2, 'i32');
  final nz = pr.f32();
  c.that(nz == 0 && nz.isNegative, '-0.0f');
  c.eq(pr.f64(), double.infinity, 'inf');
  c.eq(pr.readF64(), -7.0, 'readF64 widens i64');
  c.eq(pr.readI64Bits(), const Key64(0, 0x80000000), 'i64 min bits');
  c.eq(pr.readF64(), 9223372036854775808.0, 'i64 max widens (rounds) to 2^63');
  c.that(pr.readF64().isNaN, 'nan');
  expectKind(c, BinErrorKind.truncated, pr.u8, 'read past end');
  expectKind(c, BinErrorKind.truncated, () => BinReader(Uint8List(2)).take(-1), 'negative take');
  c.eq(BinWriter().length, 0, 'empty writer');

  // Rejections.
  expectKind(c, BinErrorKind.invalidBool, () => BinReader.validate(Uint8List.fromList([1, 2])), 'bool 2');
  expectKind(c, BinErrorKind.invalidTag, () => BinReader.validate(Uint8List.fromList([8])), 'tag 8');
  expectKind(c, BinErrorKind.invalidUtf8, () => BinReader.validate(Uint8List.fromList([4, 2, 0, 0, 0, 0xC0, 0xAF])), 'overlong');
  expectKind(c, BinErrorKind.truncated, () => BinReader.validate(Uint8List.fromList([4, 0xFF, 0xFF, 0xFF, 0xFF, 0x61])), 'huge len');
  expectKind(c, BinErrorKind.truncated, () => BinReader.validate(Uint8List.fromList([6, 4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF])), 'count bomb');
  expectKind(c, BinErrorKind.truncated, () => BinReader.validate(Uint8List.fromList([7, 8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0])), 'map bomb');
  expectKind(c, BinErrorKind.trailingBytes, () => BinReader.validate(Uint8List.fromList([6, 6, 0, 0, 0, 1, 0, 0, 0, 0, 0])), 'slack');
  expectKind(c, BinErrorKind.trailingBytes, () => BinReader.validate(Uint8List.fromList([0, 0])), 'two values');
  expectKind(c, BinErrorKind.truncated,
      () => BinReader.validate(Uint8List.fromList([6, 5, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0])), 'overrun');
  expectKind(c, BinErrorKind.unexpectedTag, () => BinReader(Uint8List.fromList([0])).readI64(), 'wrong tag');
  BinReader.validate(nested(binValueMaxDepth));
  expectKind(c, BinErrorKind.maxDepth, () => BinReader.validate(nested(binValueMaxDepth + 1)), 'depth 65');

  // Every truncated prefix fails; raw values skip exactly.
  for (var n = 0; n < golden.length; n++) {
    final prefix = Uint8List.sublistView(golden, 0, n);
    c.throwsA<BinCodecException>(() => BinReader.validate(prefix), 'prefix $n');
    c.throwsA<BinCodecException>(() => BinReader(prefix).skipValue(), 'skip prefix $n');
  }
  final two = Uint8List(golden.length + 1)..setRange(0, golden.length, golden);
  final tr = BinReader(two);
  c.eq(tr.readRawValue().length, golden.length, 'raw value');
  c.that(tr.readNull() && tr.isAtEnd, 'trailing null');

  // Deterministic fuzz: 32-bit xorshift (identical on VM and web).
  var s = 0x9E3779B9;
  int next() {
    s ^= (s << 13) & 0xFFFFFFFF;
    s ^= s >>> 17;
    s ^= (s << 5) & 0xFFFFFFFF;
    return s & 0xFFFFFFFF;
  }

  for (var iter = 0; iter < 4000; iter++) {
    final Uint8List buf;
    if (iter.isEven) {
      buf = Uint8List.fromList(golden)..[next() % golden.length] = next() & 0xFF;
    } else {
      buf = Uint8List.fromList(List<int>.generate(next() % 48, (_) => next() % 9));
    }
    var valid = true;
    try {
      BinReader.validate(buf);
    } on BinCodecException {
      valid = false;
    }
    try {
      final fr = BinReader(buf)..readRawValue();
      if (valid) {
        c.that(fr.isAtEnd, 'valid fuzz input skips fully');
      }
    } on BinCodecException {
      if (valid) {
        c.that(false, 'valid fuzz input failed to skip');
      }
    }
  }

  // Hybrid prelude.
  c.that(DynamicValue.nullValue.isNull, 'null dynamic value');
  final dv = DynamicValue(golden);
  c.eq(dv.tag, BinTag.map, 'dynamic tag');
  c.eq(dv, DynamicValue(List<int>.of(golden)), 'dynamic equals');
  c.throwsA<UnsupportedError>(() => dv.bytes[0] = 1, 'dynamic bytes are read-only');
  expectKind(c, BinErrorKind.truncated, () => DynamicValue(golden.sublist(0, 10)), 'dynamic prefix');
  expectKind(c, BinErrorKind.truncated, () => DynamicValue(<int>[]), 'dynamic empty');
  HybridException.checkSchema(const Key64(1, 2), const Key64(1, 2));
  final he = c.throwsA<HybridException>(() => HybridException.checkSchema(const Key64(1, 2), const Key64(1, 3)), 'schema mismatch');
  c.eq(he?.status, HybridStatus.schemaMismatch, 'schema mismatch status');
  c.eq(HostStatus.fromCode(-8), HostStatus.internalError, 'host status -8');
  c.eq(HostStatus.fromCode(99), null, 'unknown host status');
  c.eq(HybridStatus.fromCode(4), HybridStatus.nullHandle, 'hybrid status 4');

  // Container limit: children never read past their container (Rust `&bytes[..end]`).
  expectKind(c, BinErrorKind.truncated,
      () => BinReader.validate(Uint8List.fromList([6, 5, 0, 0, 0, 1, 0, 0, 0, 4, 2, 0, 0, 0, 0xC0, 0xAF])), 'escape container');
  expectKind(c, BinErrorKind.truncated,
      () => BinReader.validate(Uint8List.fromList([6, 10, 0, 0, 0, 1, 0, 0, 0, 6, 9, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0])),
      'nested escape');
  // BOM is text on every path (dart:convert strips one leading BOM by itself).
  for (final s in const ['\uFEFF', '\uFEFFx', '\uFEFF\uFEFFx']) {
    final bw = BinWriter();
    final m = bw.beginMap(1);
    bw
      ..writeMapKey(s)
      ..writeStr(s)
      ..end(m);
    final br = BinReader(bw.toBytes());
    c.eq(br.beginMap(), 1, 'BOM map');
    c.eq(br.readMapKey(), s, 'BOM key ${s.length}');
    c.eq(br.readStr(), s, 'BOM str ${s.length}');
  }
  c.eq(decodeUtf8PreserveBom(Uint8List.fromList([0xEF, 0xBB, 0xBF])), '\uFEFF', 'bare BOM');
  // Lone surrogates are rejected (never U+FFFD) and nothing is written.
  for (final lone in const ['\uD800', 'a\uDC00', '\uD83D', '\uDE80\uD83D']) {
    final lw = BinWriter();
    expectKind(c, BinErrorKind.invalidUtf8, () => lw.writeMapKey(lone), 'key lone surrogate');
    c.eq(lw.length, 0, 'nothing written for a rejected key');
    expectKind(c, BinErrorKind.invalidUtf8, () => lw.writeStr(lone), 'str lone surrogate');
    c.eq(lw.length, 0, 'nothing written for a rejected string (not even the tag)');
    expectKind(c, BinErrorKind.invalidUtf8, () => utf8EncodedLength(lone), 'length lone surrogate');
  }
  c.eq(utf8EncodedLength('h\u00e9 \u{1F680}'), 8, 'encoded length with a pair');
  // Stale marks never patch out of bounds.
  final mw = BinWriter();
  final mark = mw.beginArray(1);
  mw
    ..writeNull()
    ..clear();
  expectKind(c, BinErrorKind.invalidMark, () => mw.end(mark), 'stale mark');
  mw.writeNull();
  expectKind(c, BinErrorKind.invalidMark, () => mw.end(0), 'mark without container');

  // UTF-8.
  c.that(!isValidUtf8(const [-1]), 'negative element');
  c.that(!isValidUtf8(const [0xE2, 0x80, 0x180]), 'continuation above 255');
  c.that(!isValidUtf8(const [0x100]), 'lead above 255');
  c.that(!isValidUtf8(const [0xC3, -0x57]), 'negative continuation');
  c.throwsA<RangeError>(() => isValidUtf8(const [0x41], 0, 2), 'range past end');
  c.throwsA<RangeError>(() => isValidUtf8(const [0x41], 1, 0), 'inverted range');
  c.that(isValidUtf8(utf8.encode('plain é — 🚀 \u{10FFFF}')), 'valid utf8');
  for (final bad in const [
    [0xC0, 0xAF], [0xE0, 0x80, 0xAF], [0xF0, 0x80, 0x80, 0xAF], [0xED, 0xA0, 0x80],
    [0xF4, 0x90, 0x80, 0x80], [0xE2, 0x80], [0xE2, 0x28, 0xA1], [0x80], [0xF5, 0x80, 0x80, 0x80], //
  ]) {
    c.that(!isValidUtf8(bad), 'invalid utf8 $bad');
  }
}
