import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';

import 'check.dart';

final Key64 visual = Key64.of('nve.visual');

final opacity = FieldSpec(0, FieldKind.u32, bits: 8);
final z = FieldSpec(0, FieldKind.i32, shift: 8, bits: 12);
final visible = FieldSpec(0, FieldKind.boolean, shift: 20, bits: 1);
final width = FieldSpec(1, FieldKind.length);
final color = FieldSpec(2, FieldKind.color);
final label = FieldSpec(3, FieldKind.str);
final scale = FieldSpec(5, FieldKind.f32);

const List<int> rowWords = [
  200 | (0xFFD << 8) | (1 << 20), 0x42480001, 0x80402010, 0, 3, 0x3FC00000, //
  2047 << 8, 2, 0, 3, 0, 0x80000000,
];

ByteData words(List<int> w) {
  final d = ByteData(w.length * 4);
  for (var i = 0; i < w.length; i++) {
    d.setUint32(i * 4, w[i], Endian.little);
  }
  return d;
}

void expectScene(Check c, SceneErrorKind kind, void Function() body, String what) {
  final e = c.throwsA<SceneException>(body, what);
  if (e != null) {
    c.eq(e.kind, kind, '$what kind');
  }
}

void runHardening(Check c) {
  // Malformed specs cannot be built.
  for (final (word, shift, bits, kind) in const [
    (0, 24, 16, FieldKind.u32), (0, 0, 0, FieldKind.u32), (0, 0, 33, FieldKind.i32), (0, 31, 2, FieldKind.i32),
    (32, 0, 32, FieldKind.u32), (31, 0, 32, FieldKind.str), (-1, 0, 8, FieldKind.u32), (0, -1, 8, FieldKind.u32), //
    (0x1FFFFFFFFFFFFF, 0, 32, FieldKind.u32), (0, 0x1FFFFFFFFFFFFF, 1, FieldKind.u32), // overflow-free comparisons
  ]) {
    c.throwsA<ArgumentError>(() => FieldSpec(word, kind, shift: shift, bits: bits), 'bad spec $word/$shift/$bits');
  }
  final t = TableReader(key: visual, slot: 0, words: 6, rowCount: 2, rows: words(rowWords));
  c.throwsA<RangeError>(() => t.u32(0, FieldSpec(6, FieldKind.u32)), 'field past row');
  c.throwsA<RangeError>(() => t.i32(0, FieldSpec(31, FieldKind.i32)), 'i32 field past row');
  c.throwsA<RangeError>(() => t.str(0, FieldSpec(3, FieldKind.u32)), 'str of a non-string spec');
  c.throwsA<RangeError>(() => t.str(0, FieldSpec(5, FieldKind.str)), 'str past the row');
  c.eq(t.str(0, label), (0, 3), 'valid string spec still reads');
  // A negative VM int extracts its low 32 bits, exactly like the web.
  c.eq(FieldSpec(0, FieldKind.u32).extract(-1), 0xFFFFFFFF, 'extract(-1) whole word');
  c.eq(FieldSpec(0, FieldKind.u32, shift: 24, bits: 8).extract(-1), 0xFF, 'extract(-1) top byte');
  c.eq(FieldSpec(0, FieldKind.i32, shift: 28, bits: 4).extractSigned(-1), -1, 'extractSigned(-1)');
  // Negative and overflowing counts.
  c.throwsA<ArgumentError>(
      () => TableReader(key: visual, slot: 0, words: 2, rowCount: 0, rows: ByteData(0), dirtyCount: -1), 'dirtyCount -1');
  c.throwsA<ArgumentError>(() => TableReader(key: visual, slot: 0, words: 2, rowCount: -1, rows: ByteData(0)), 'rowCount -1');
  const isWeb = identical(0, 0.0);
  if (!isWeb) {
    // `shift + bits` with a VM max int wraps negative: the old check let this spec through.
    final maxInt = (1 << 62) + ((1 << 62) - 1); // computed: no >2^53 literal reaches dart2js
    c.throwsA<ArgumentError>(() => FieldSpec(0, FieldKind.u32, shift: maxInt, bits: 1), 'VM shift overflow');
    c.throwsA<ArgumentError>(() => FieldSpec(maxInt, FieldKind.u32), 'VM word overflow');
    // `rowCount * words` would wrap the 64-bit VM int to a tiny positive number.
    final wrap = (1 << 62) + 1;
    c.throwsA<ArgumentError>(() => TableReader(key: visual, slot: 0, words: 4, rowCount: wrap, rows: words(rowWords)), 'VM wrap');
  }
}

void runTableScene(Check c) {
  runHardening(c);
  // Field specs.
  c.eq(FieldSpec(0, FieldKind.u32, shift: 8, bits: 8).extract(0xABCD1234), 0x12, 'extract');
  c.eq(FieldSpec(0, FieldKind.u32).extract(0xABCD1234), 0xABCD1234, 'extract whole');
  c.eq(FieldSpec(0, FieldKind.i32, bits: 3).extractSigned(7), -1, 'sign 3 bits');
  c.eq(FieldSpec(0, FieldKind.i32, bits: 3).extractSigned(4), -4, 'sign min');
  c.eq(FieldSpec(0, FieldKind.i32).extractSigned(0x80000000), -0x80000000, 'sign 32');
  c.eq(FieldSpec(0, FieldKind.i32, shift: 31, bits: 1).extractSigned(0x80000000), -1, 'sign top bit');
  c.that(label.fits(5) && !label.fits(4), 'str fits');
  c.eq(LengthValue.decode(0x3F800000), const LengthValue(LengthUnit.points, 1), 'points');
  c.eq(LengthValue.decode(0x42480001), const LengthValue(LengthUnit.percent, 50), 'percent');
  c.eq(LengthValue.decode(2), const LengthValue(LengthUnit.auto, 0), 'auto');
  c.eq(LengthValue.decode(0x3F800003), const LengthValue(LengthUnit.auto, 0), 'tag 11');
  const rgba = Rgba(0x80402010);
  c.that(rgba.r == 0x10 && rgba.g == 0x20 && rgba.b == 0x40 && rgba.a == 0x80, 'rgba');

  // Dense table.
  final t = TableReader(key: visual, slot: 4, words: 6, rowCount: 2, rows: words(rowWords), dirtyRows: words([1]), dirtyCount: 1);
  c.that(!t.isSparse && t.key == visual && t.slot == 4, 'dense header');
  c.eq(t.u32(0, opacity), 200, 'opacity');
  c.eq(t.i32(0, z), -3, 'z -3');
  c.eq(t.i32(1, z), 2047, 'z max');
  c.that(t.boolean(0, visible) && !t.boolean(1, visible), 'visible');
  c.eq(t.length(0, width), const LengthValue(LengthUnit.percent, 50), 'width');
  c.eq(t.length(1, width).unit, LengthUnit.auto, 'width auto');
  c.eq(t.color(0, color).a, 0x80, 'color');
  c.eq(t.str(0, label), (0, 3), 'label');
  c.eq(t.f32(0, scale), 1.5, 'scale');
  c.that(t.f32(1, scale) == 0 && t.f32(1, scale).isNegative, '-0.0 scale');
  c.eq(t.dirtyRow(0), 1, 'dirty row');
  c.eq(t.rowOf(1), 1, 'dense rowOf');
  c.eq(t.rowOf(2), null, 'dense rowOf miss');
  c.eq(t.owner(1), 1, 'dense owner');
  c.throwsA<RangeError>(() => t.u32(2, opacity), 'row oob');
  c.throwsA<RangeError>(() => t.word(0, 6), 'word oob');
  c.throwsA<RangeError>(() => t.str(0, FieldSpec(5, FieldKind.str)), 'str second word oob');
  c.throwsA<RangeError>(() => t.dirtyRow(1), 'dirty oob');

  // Sparse and invalid tables.
  final sp = TableReader.sparse(key: visual, slot: 0, words: 2, rows: words([1, 2, 3, 4]), owners: [0x01000007, 0x02000003]);
  c.that(sp.isSparse, 'sparse');
  c.eq(sp.rowOf(0x02000003), 1, 'sparse rowOf');
  c.eq(sp.rowOfNode(3), 1, 'sparse rowOfNode');
  c.eq(sp.rowOf(0x01000003), null, 'stale generation of a live slot misses');
  c.eq(sp.rowOf(1), null, 'sparse miss');
  c.eq(sp.rowOfNode(-1), null, 'negative node index');
  c.eq(sp.owner(0), 0x01000007, 'sparse owner');
  // Rows spread over pages: node 7 (page of slot 0) and node 0x10203 (directory slot 0x102).
  final far = TableReader.sparse(key: visual, slot: 0, words: 1, rows: words([5, 6]), owners: [7, 0x10203]);
  c.eq(far.rowOfNode(0x10203), 1, 'far page row');
  c.eq(far.rowOfNode(0x10204), null, 'empty entry in a present page');
  c.eq(far.rowOfNode(0x203), null, 'absent page between present ones');
  c.throwsA<ArgumentError>(
      () => TableReader.sparse(key: visual, slot: 0, words: 1, rows: words([5, 6]), owners: [7, 0x01000007]), 'shared node index');
  c.throwsA<ArgumentError>(
      () => TableReader(key: visual, slot: 0, words: 1, rowCount: 1, rows: words([5]), owners: words([7])), 'sparse rows without a row index');
  c.throwsA<ArgumentError>(
      () => TableReader(
          key: visual, slot: 0, words: 1, rowCount: 1, rows: words([5]), owners: words([7]), //
          rowIndexDir: words([0]), rowIndexPages: ByteData(4 * (nveRowIndexPageLen - 1))),
      'a partial page alone is no row index');
  c.eq(TableReader(key: visual, slot: 0, words: 1, rowCount: 0, rows: ByteData(0), owners: ByteData(0)).rowOf(1), null,
      'empty sparse table needs no index');
  // Hostile published indices: an entry past rowCount, a page id past the pages, a partial tail page.
  final pageWords = Uint32List(nveRowIndexPageLen + 3)..[1] = 9;
  final hostile = TableReader(
      key: visual, slot: 0, words: 1, rowCount: 1, rows: words([5]), owners: words([1]), //
      rowIndexDir: words([0, 1]), rowIndexPages: ByteData.sublistView(pageWords));
  c.eq(hostile.rowOfNode(1), null, 'row + 1 past rowCount misses');
  c.eq(hostile.rowOfNode(nveRowIndexPageLen), null, 'partial tail page is unreachable');
  c.throwsA<ArgumentError>(() => TableReader(key: visual, slot: 0, words: 2, rowCount: 3, rows: words([1, 2, 3, 4])), 'short rows');
  c.throwsA<ArgumentError>(() => TableReader(key: visual, slot: 0, words: 33, rowCount: 0, rows: ByteData(0)), '33 words');
  c.throwsA<ArgumentError>(() => TableReader(key: visual, slot: 0, words: 0, rowCount: 0, rows: ByteData(0)), '0 words');
  c.throwsA<ArgumentError>(
      () => TableReader(key: visual, slot: 0, words: 2, rowCount: 2, rows: words([1, 2, 3, 4]), owners: words([1])), 'short owners');
  c.throwsA<ArgumentError>(() => TableReader(key: visual, slot: 0, words: 2, rowCount: 0, rows: ByteData(0), dirtyCount: 2), 'null dirty');

  _runOptWideAndGrid(c);
}

void _runOptWideAndGrid(Check c) {
  // Invalid optional / wide / inherited FieldSpec rejections.
  c.throwsA<ArgumentError>(() => FieldSpec(0, FieldKind.f32, bits: 32, opt: OptEncoding.wideBit, valWord: 1), 'wideBit requires bits=1');
  c.throwsA<ArgumentError>(() => FieldSpec(0, FieldKind.f32, bits: 1, opt: OptEncoding.wideBit), 'wideBit requires valid valWord');
  c.throwsA<ArgumentError>(() => FieldSpec(1, FieldKind.f32, bits: 1, opt: OptEncoding.wideBit, valWord: 1), 'wideBit valWord != word');
  c.throwsA<ArgumentError>(() => FieldSpec(0, FieldKind.u32, valWord: 1), 'non-wideBit rejects valWord');
  c.throwsA<ArgumentError>(() => FieldSpec(0, FieldKind.u32, bits: 1, opt: OptEncoding.bit0Present), 'bit0Present requires bits>=2');
  c.throwsA<ArgumentError>(() => FieldSpec(0, FieldKind.u32, opt: OptEncoding.tagXor), 'tagXor rejects u32');
  c.throwsA<ArgumentError>(() => FieldSpec(0, FieldKind.length, bits: 16, opt: OptEncoding.tagXor), 'tagXor length requires 32b');
  c.throwsA<ArgumentError>(() => FieldSpec(0, FieldKind.gridLine, bits: 32, opt: OptEncoding.tagXor), 'tagXor gridLine requires 16b');
  c.throwsA<ArgumentError>(() => FieldSpec(0, FieldKind.u32, inherits: true), 'inherits requires explicitBit');
  c.throwsA<ArgumentError>(() => FieldSpec(1, FieldKind.u32, explicitBit: 32), 'explicitBit >= 32');
  c.throwsA<ArgumentError>(() => FieldSpec(0, FieldKind.u32, explicitBit: 0), 'explicitBit rejects word == 0');
  c.throwsA<ArgumentError>(
      () => FieldSpec(1, FieldKind.f32, bits: 1, opt: OptEncoding.wideBit, valWord: 0, explicitBit: 0),
      'explicitBit rejects valWord == 0');
  c.throwsA<ArgumentError>(() => FieldSpec(31, FieldKind.u64), 'u64 at word 31 overflows');
  c.throwsA<ArgumentError>(() => FieldSpec(0, FieldKind.f64, shift: 1), 'f64 requires shift=0');

  // GridLine decode + TagXor optional decode (matches Rust `GridLine`).
  c.eq(GridLineValue.decode(0), const GridLineValue.auto(), 'gridLine auto');
  c.eq(GridLineValue.decode(0xC000), const GridLineValue.auto(), 'gridLine tag 11 -> auto');
  c.eq(GridLineValue.decode((1 << 14) | 5), const GridLineValue.line(5), 'gridLine line +5');
  c.eq(GridLineValue.decode((1 << 14) | 0x3FFF), const GridLineValue.line(-1), 'gridLine line -1');
  c.eq(GridLineValue.decode((1 << 14) | 0x2000), const GridLineValue.line(-8192), 'gridLine min line');
  c.eq(GridLineValue.decode((1 << 14) | 0x1FFF), const GridLineValue.line(8191), 'gridLine max line');
  c.eq(GridLineValue.decode((2 << 14) | 3), const GridLineValue.span(3), 'gridLine span 3');
  c.eq(GridLineValue.decode((2 << 14) | 0x3FFF), const GridLineValue.span(16383), 'gridLine max span');
  c.eq(GridLineValue.decodeOpt(0), null, 'opt gridLine 0 -> null');
  c.eq(GridLineValue.decodeOpt(0x0042), null, 'opt gridLine niche 00 -> null');
  c.eq(GridLineValue.decodeOpt(0 ^ 0xC000), const GridLineValue.auto(), 'opt gridLine auto');
  c.eq(GridLineValue.decodeOpt(((2 << 14) | 4) ^ 0xC000), const GridLineValue.span(4), 'opt gridLine span 4');

  // LengthValue.decodeOpt (`TagXor`).
  c.eq(LengthValue.decodeOpt(0), null, 'opt length 0 -> null');
  c.eq(LengthValue.decodeOpt(0x12345600), null, 'opt length tag 00 -> null');
  c.eq(LengthValue.decodeOpt(0x3F800000 ^ 3), const LengthValue(LengthUnit.points, 1), 'opt length points');
  c.eq(LengthValue.decodeOpt(0x42480001 ^ 3), const LengthValue(LengthUnit.percent, 50), 'opt length percent');
  c.eq(LengthValue.decodeOpt(2 ^ 3), const LengthValue(LengthUnit.auto, 0), 'opt length auto');

  // Table with every OptEncoding, 64-bit fields (u64, i64, f64), GridLine, and explicit bit 31.
  final fZeroNiche = FieldSpec(1, FieldKind.u32, shift: 0, bits: 16, opt: OptEncoding.zeroNiche, explicitBit: 0, inherits: true);
  final fBit0Signed = FieldSpec(2, FieldKind.i32, shift: 0, bits: 17, opt: OptEncoding.bit0Present);
  final fPlusOneEnum = FieldSpec(2, FieldKind.enumeration, shift: 17, bits: 4, opt: OptEncoding.plusOne);
  final fPlusOneBool = FieldSpec(2, FieldKind.enumeration, shift: 21, bits: 2, opt: OptEncoding.plusOne);
  final fWideF32 = FieldSpec(2, FieldKind.f32, shift: 23, bits: 1, opt: OptEncoding.wideBit, valWord: 3, explicitBit: 31, inherits: true);
  final fOptLen = FieldSpec(4, FieldKind.length, opt: OptEncoding.tagXor);
  final fOptGrid = FieldSpec(5, FieldKind.gridLine, shift: 0, bits: 16, opt: OptEncoding.tagXor);
  final fReqGrid = FieldSpec(5, FieldKind.gridLine, shift: 16, bits: 16);
  final fU64 = FieldSpec(6, FieldKind.u64, bits: 64);
  final fI64 = FieldSpec(8, FieldKind.i64, bits: 64);
  final fF64 = FieldSpec(10, FieldKind.f64, bits: 64);

  // Row 0: all optionals absent, 64-bit values populated (including >2^53 u64 for dart2js safety).
  // Row 1: all optionals present, negative i16 (-7 -> 0xFFF9), explicit bit 0 and bit 31 set.
  final f64Bits = ByteData(8)..setFloat64(0, -12.5, Endian.little);
  final row0 = [
    0x00000001, // word 0: explicit bit 0 only
    0, // word 1: zeroNiche absent
    0, // word 2: bit0Present=0, plusOne=0, optBool=0, wideBit=0
    0x40A00000, // word 3: wide valWord ignored when bit is 0
    0, // word 4: optLength absent
    ((1 << 14) | 2) << 16, // word 5: optGrid absent, reqGrid = Line(2)
    0xDEADBEEF, 0xCAFEBABE, // word 6..7: u64 > 2^53
    0xFFFFFFFE, 0xFFFFFFFF, // word 8..9: i64 = -2
    f64Bits.getUint32(0, Endian.little), f64Bits.getUint32(4, Endian.little), // word 10..11: f64 = -12.5
  ];
  final row1 = [
    0x80000000, // word 0: explicit bit 31 (tests JS unsigned shift!)
    42, // word 1: zeroNiche = 42
    ((0xFFF9 << 1) | 1) | (5 << 17) | (2 << 21) | (1 << 23), // word 2: -7, enum 4, bool true, wideBit=1
    0x40200000, // word 3: 2.5f
    0x42480001 ^ 3, // word 4: 50%
    (((2 << 14) | 3) ^ 0xC000) | (((1 << 14) | 0x3FFE) << 16), // word 5: Some(Span(3)), Line(-2)
    1, 0, // word 6..7: u64 = 1
    5, 0, // word 8..9: i64 = 5
    0, 0x80000000, // word 10..11: f64 = -0.0
  ];
  final tab = TableReader(key: visual, slot: 1, words: 12, rowCount: 2, rows: words([...row0, ...row1]));

  c.that(tab.isExplicit(0, fZeroNiche) && !tab.isExplicit(0, fWideF32), 'row 0 explicit bits');
  c.that(!tab.isExplicit(1, fZeroNiche) && tab.isExplicit(1, fWideF32), 'row 1 explicit bit 31');
  c.that(tab.isExplicit(0, fBit0Signed), 'non-inherited field isExplicit returns true on valid row');
  c.throwsA<RangeError>(() => tab.isExplicit(99, fBit0Signed), 'isExplicit validates row bounds even without explicitBit');
  c.throwsA<RangeError>(() => tab.isExplicit(0, FieldSpec(12, FieldKind.u32)), 'isExplicit validates field fit');
  c.eq(tab.eventMask(0, noFieldSlot), 0, 'no eventsWord -> 0');
  c.eq(tab.eventMask(0, 0), 1, 'eventsWord reads word 0');

  c.eq(tab.optU32(0, fZeroNiche), null, 'zeroNiche absent');
  c.eq(tab.optU32(1, fZeroNiche), 42, 'zeroNiche present');
  c.eq(tab.optI32(0, fBit0Signed), null, 'bit0Present absent');
  c.eq(tab.optI32(1, fBit0Signed), -7, 'bit0Present signed -7');
  c.eq(tab.optEnumValue(0, fPlusOneEnum), null, 'plusOne enum absent');
  c.eq(tab.optEnumValue(1, fPlusOneEnum), 4, 'plusOne enum present');
  c.eq(tab.optBoolean(0, fPlusOneBool), null, 'plusOne bool absent');
  c.eq(tab.optBoolean(1, fPlusOneBool), true, 'plusOne bool true');
  c.eq(tab.optF32(0, fWideF32), null, 'wideBit f32 absent');
  c.eq(tab.optF32(1, fWideF32), 2.5, 'wideBit f32 present');
  c.eq(tab.optLength(0, fOptLen), null, 'tagXor length absent');
  c.eq(tab.optLength(1, fOptLen), const LengthValue(LengthUnit.percent, 50), 'tagXor length present');
  c.eq(tab.optGridLine(0, fOptGrid), null, 'tagXor gridLine absent');
  c.eq(tab.optGridLine(1, fOptGrid), const GridLineValue.span(3), 'tagXor gridLine present');
  c.eq(tab.gridLine(0, fReqGrid), const GridLineValue.line(2), 'req gridLine +2');
  c.eq(tab.gridLine(1, fReqGrid), const GridLineValue.line(-2), 'req gridLine -2');

  c.eq(tab.u64(0, fU64), const Key64(0xDEADBEEF, 0xCAFEBABE), 'u64 exact Key64 above 2^53');
  c.eq(tab.i64Bits(0, fI64), const Key64(0xFFFFFFFE, 0xFFFFFFFF), 'i64Bits -2');
  c.eq(tab.i64(0, fI64), -2, 'i64 -2');
  c.eq(tab.i64(1, fI64), 5, 'i64 +5');
  c.eq(tab.f64(0, fF64), -12.5, 'f64 -12.5');
  c.that(tab.f64(1, fF64) == 0 && tab.f64(1, fF64).isNegative, 'f64 -0.0');
  c.throwsA<RangeError>(() => tab.u64(0, FieldSpec(11, FieldKind.u64)), 'u64 past row');
  c.throwsA<RangeError>(() => tab.optF32(0, FieldSpec(0, FieldKind.f32, bits: 1, opt: OptEncoding.wideBit, valWord: 12)), 'wideBit valWord past row');
  // Corrupt PLUS_ONE bool raw 0b11 is absent for both isPresent and optBoolean.
  final fBoolPlus = FieldSpec(0, FieldKind.boolean, bits: 2, opt: OptEncoding.plusOne);
  final bt = TableReader(key: visual, slot: 0, words: 1, rowCount: 1, rows: words([3]));
  c.that(!bt.isPresent(0, fBoolPlus) && bt.optBoolean(0, fBoolPlus) == null, 'plusOne bool raw 0b11 is absent');
}
