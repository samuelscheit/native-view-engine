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
}
