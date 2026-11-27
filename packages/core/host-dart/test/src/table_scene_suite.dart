import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';

import 'check.dart';

const Key64 schema = Key64(0xC0FFEE11, 0xF00DFACE); // high bit set on purpose
final Key64 visual = Key64.of('nve.visual');

final opacity = FieldSpec(0, FieldKind.u32, bits: 8);
final z = FieldSpec(0, FieldKind.i32, shift: 8, bits: 12);
final visible = FieldSpec(0, FieldKind.boolean, shift: 20, bits: 1);
final width = FieldSpec(1, FieldKind.length);
final color = FieldSpec(2, FieldKind.color);
final label = FieldSpec(3, FieldKind.str);
final scale = FieldSpec(5, FieldKind.f32);

/// Test node record: id u32 @0, text_offset u32 @4, text_len u16 @8, child_count u16 @10, child_offset u32 @12.
const int nodeStride = 16;

const List<int> rowWords = [
  200 | (0xFFD << 8) | (1 << 20), 0x42480001, 0x80402010, 0, 3, 0x3FC00000, //
  2047 << 8, 2, 0, 3, 0, 0x80000000,
];

const List<int> textBytes = [0x68, 0xC3, 0xA9, 0xF0, 0x9F, 0x9A, 0x80]; // "hé" + rocket

ByteData words(List<int> w) {
  final d = ByteData(w.length * 4);
  for (var i = 0; i < w.length; i++) {
    d.setUint32(i * 4, w[i], Endian.little);
  }
  return d;
}

ByteData nodes() {
  final d = ByteData(3 * nodeStride);
  const n = [
    [1, 0, 3, 2, 0],
    [2, 3, 4, 0, 2],
    [3, 7, 0, 0, 2],
  ];
  for (var i = 0; i < 3; i++) {
    final b = i * nodeStride;
    d
      ..setUint32(b, n[i][0], Endian.little)
      ..setUint32(b + 4, n[i][1], Endian.little)
      ..setUint16(b + 8, n[i][2], Endian.little)
      ..setUint16(b + 10, n[i][3], Endian.little)
      ..setUint32(b + 12, n[i][4], Endian.little);
  }
  return d;
}

/// wasm32 layout locked by Rust `#[nve_abi(offset = ..)]` (pointers are 4 bytes on wasm32).
const SceneLayout wasm32Layout = SceneLayout(
  schemaHash: 0,
  frameId: 8,
  nodeCount: 16,
  dirtyCount: 20,
  removedCount: 24,
  textSlabLen: 28,
  childrenSlabLen: 32,
  tableCount: 36,
  nodes: 40,
  textSlab: 44,
  childrenSlab: 48,
  dirtyIndices: 52,
  removedIds: 56,
  tables: 60,
  nodeStride: nodeStride,
  tableView: TableViewLayout(
    stride: 52,
    keyLo: 0,
    keyHi: 4,
    slot: 8,
    words: 12,
    rowCount: 16,
    dirtyCount: 20,
    rows: 24,
    owners: 28,
    dirtyRows: 32,
    rowIndexDirLen: 36,
    rowIndexPageCount: 40,
    rowIndexDir: 44,
    rowIndexPages: 48,
  ),
);

/// Builds a linear memory holding one frame; returns (memory, viewPtr).
(ByteData, int) linearMemory({int tableWords = 6}) {
  final mem = ByteData(4096);
  var cursor = 256;
  int place(List<int> bytes) {
    final at = cursor;
    for (var i = 0; i < bytes.length; i++) {
      mem.setUint8(at + i, bytes[i]);
    }
    cursor = (cursor + bytes.length + 7) & ~7;
    return at;
  }

  List<int> asBytes(ByteData d) => Uint8List.sublistView(d);
  final nodesPtr = place(asBytes(nodes()));
  final textPtr = place(textBytes);
  final childrenPtr = place(asBytes(words([2, 3])));
  final dirtyPtr = place(asBytes(words([0, 2, 99])));
  final removedPtr = place(asBytes(words([42])));
  final rowsPtr = place(asBytes(words(rowWords)));
  final tableRowDirtyPtr = place(asBytes(words([1])));
  final tablePtr = place(List<int>.filled(36, 0));
  final t = wasm32Layout.tableView;
  mem
    ..setUint32(tablePtr + t.keyLo, visual.lo, Endian.little)
    ..setUint32(tablePtr + t.keyHi, visual.hi, Endian.little)
    ..setUint32(tablePtr + t.slot, 3, Endian.little)
    ..setUint32(tablePtr + t.words, tableWords, Endian.little)
    ..setUint32(tablePtr + t.rowCount, 2, Endian.little)
    ..setUint32(tablePtr + t.dirtyCount, 1, Endian.little)
    ..setUint32(tablePtr + t.rows, rowsPtr, Endian.little)
    ..setUint32(tablePtr + t.owners, 0, Endian.little)
    ..setUint32(tablePtr + t.dirtyRows, tableRowDirtyPtr, Endian.little);
  final view = place(List<int>.filled(64, 0));
  const l = wasm32Layout;
  schema.writeLE(Uint8List.sublistView(mem), view + l.schemaHash);
  // frame id above 2^53: proves exact 64-bit transport.
  const Key64(0x00000007, 0xFFFF0000).writeLE(Uint8List.sublistView(mem), view + l.frameId);
  for (final (off, v) in [
    (l.nodeCount, 3), (l.dirtyCount, 3), (l.removedCount, 1), (l.textSlabLen, 7), (l.childrenSlabLen, 2), //
    (l.tableCount, 1), (l.nodes, nodesPtr), (l.textSlab, textPtr), (l.childrenSlab, childrenPtr),
    (l.dirtyIndices, dirtyPtr), (l.removedIds, removedPtr), (l.tables, tablePtr),
  ]) {
    mem.setUint32(view + off, v, Endian.little);
  }
  return (mem, view);
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
  // Layout strides below 1 are rejected before any arithmetic (no stride-0 / negative walks).
  final (strideMem, strideView) = linearMemory();
  for (final (node, table) in const [(0, 36), (-1, 36), (16, 0), (16, -36)]) {
    final bad = SceneLayout(
      schemaHash: 0, frameId: 8, nodeCount: 16, dirtyCount: 20, removedCount: 24, textSlabLen: 28, //
      childrenSlabLen: 32, tableCount: 36, nodes: 40, textSlab: 44, childrenSlab: 48, dirtyIndices: 52,
      removedIds: 56, tables: 60, nodeStride: node,
      tableView: TableViewLayout(
        stride: table, keyLo: 0, keyHi: 4, slot: 8, words: 12, rowCount: 16, dirtyCount: 20, rows: 24, owners: 28, //
        dirtyRows: 32, rowIndexDirLen: 36, rowIndexPageCount: 40, rowIndexDir: 44, rowIndexPages: 48,
      ),
    );
    expectScene(c, SceneErrorKind.missingSlab, () => SceneReader.fromLinearMemory(strideMem, strideView, bad, schema),
        'strides $node/$table');
  }
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
  // Hostile wasm32 table: words is rejected before any multiplication.
  final (mem, view) = linearMemory(tableWords: 0x80000000);
  final tablePtr = mem.getUint32(view + wasm32Layout.tables, Endian.little);
  mem.setUint32(tablePtr + wasm32Layout.tableView.rowCount, 0x80000000, Endian.little);
  expectScene(c, SceneErrorKind.invalidTable, () => SceneReader.fromLinearMemory(mem, view, wasm32Layout, schema), 'hostile words');
  // A leading BOM is scene text, not stripped (dart:convert would drop it).
  final (bomMem, bomView) = linearMemory();
  final textPtr = bomMem.getUint32(bomView + wasm32Layout.textSlab, Endian.little);
  const bom = [0xEF, 0xBB, 0xBF, 0xEF, 0xBB, 0xBF, 0x78];
  for (var i = 0; i < bom.length; i++) {
    bomMem.setUint8(textPtr + i, bom[i]);
  }
  final s = SceneReader.fromLinearMemory(bomMem, bomView, wasm32Layout, schema);
  c.eq(s.text(0, 7), '\uFEFF\uFEFFx', 'scene double BOM');
  c.eq(s.text(0, 3), '\uFEFF', 'scene lone BOM');
  c.eq(s.text(0, 7).length, 3, 'scene BOM length');
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
  final sp = TableReader(key: visual, slot: 0, words: 2, rowCount: 2, rows: words([1, 2, 3, 4]), owners: words([0x00010007, 0x00020003]));
  c.that(sp.isSparse, 'sparse');
  c.eq(sp.rowOf(0x00020003), 1, 'sparse rowOf');
  c.eq(sp.rowOf(1), null, 'sparse miss');
  c.eq(sp.owner(0), 0x00010007, 'sparse owner');
  c.throwsA<ArgumentError>(() => TableReader(key: visual, slot: 0, words: 2, rowCount: 3, rows: words([1, 2, 3, 4])), 'short rows');
  c.throwsA<ArgumentError>(() => TableReader(key: visual, slot: 0, words: 33, rowCount: 0, rows: ByteData(0)), '33 words');
  c.throwsA<ArgumentError>(() => TableReader(key: visual, slot: 0, words: 0, rowCount: 0, rows: ByteData(0)), '0 words');
  c.throwsA<ArgumentError>(
      () => TableReader(key: visual, slot: 0, words: 2, rowCount: 2, rows: words([1, 2, 3, 4]), owners: words([1])), 'short owners');
  c.throwsA<ArgumentError>(() => TableReader(key: visual, slot: 0, words: 2, rowCount: 0, rows: ByteData(0), dirtyCount: 2), 'null dirty');

  // Scene from wasm32 linear memory.
  final (mem, viewPtr) = linearMemory();
  final s = SceneReader.fromLinearMemory(mem, viewPtr, wasm32Layout, schema);
  c.eq(s.frameId, const Key64(7, 0xFFFF0000), 'frame id above 2^53 exact');
  c.eq(s.nodeCount, 3, 'node count');
  c.eq(s.nodeU32(1, 0), 2, 'node id');
  c.eq(s.text(s.nodeU32(0, 4), s.nodeU16(0, 8)), 'hé', 'node text');
  c.eq(s.text(3, 4), '🚀', 'rocket');
  c.eq(s.text(7, 0), '', 'empty at end');
  c.throwsA<RangeError>(() => s.text(2, 1), 'split char');
  c.throwsA<RangeError>(() => s.text(4, 3), 'mid rocket');
  c.throwsA<RangeError>(() => s.text(7, 1), 'text oob');
  c.eq(s.childId(s.nodeU32(0, 12) + 1), 3, 'child');
  c.throwsA<RangeError>(() => s.childId(2), 'child oob');
  c.throwsA<RangeError>(() => s.nodeU32(3, 0), 'node oob');
  c.throwsA<RangeError>(() => s.nodeU32(0, 13), 'field past stride');
  c.eq(s.nodeU8(0, 0), 1, 'node u8');
  c.eq(s.nodeI32(0, 0), 1, 'node i32');
  final visited = <int>[];
  s.forEachDirty(visited.add);
  c.eq(visited.join(','), '0,2', 'dirty visits skip out-of-range');
  c.eq(s.removedId(0), 42, 'removed');
  final st = s.findTable(visual);
  c.eq(st?.u32(0, opacity), 200, 'table via scene');
  c.eq(st?.slot, 3, 'table slot');
  c.eq(st?.dirtyRow(0), 1, 'table dirty row');
  c.eq(s.findTable(const Key64(1, 0)), null, 'missing table');

  // Rejections.
  expectScene(c, SceneErrorKind.schemaMismatch, () => SceneReader.fromLinearMemory(mem, viewPtr, wasm32Layout, const Key64(1, 0)), 'schema');
  final (badTableMem, badTableView) = linearMemory(tableWords: 40);
  expectScene(c, SceneErrorKind.invalidTable, () => SceneReader.fromLinearMemory(badTableMem, badTableView, wasm32Layout, schema), 'table words');
  final (nullMem, nullView) = linearMemory();
  nullMem.setUint32(nullView + wasm32Layout.childrenSlab, 0, Endian.little);
  expectScene(c, SceneErrorKind.missingSlab, () => SceneReader.fromLinearMemory(nullMem, nullView, wasm32Layout, schema), 'null children');
  final (oobMem, oobView) = linearMemory();
  oobMem.setUint32(oobView + wasm32Layout.textSlabLen, 0xFFFFFFF0, Endian.little);
  expectScene(c, SceneErrorKind.missingSlab, () => SceneReader.fromLinearMemory(oobMem, oobView, wasm32Layout, schema), 'text past memory');
  expectScene(c, SceneErrorKind.missingSlab, () => SceneReader.fromLinearMemory(mem, 4094, wasm32Layout, schema), 'view past memory');
  final (utfMem, utfView) = linearMemory();
  final textPtr = utfMem.getUint32(utfView + wasm32Layout.textSlab, Endian.little);
  utfMem
    ..setUint8(textPtr, 0xC0)
    ..setUint8(textPtr + 1, 0xAF);
  expectScene(c, SceneErrorKind.invalidUtf8, () => SceneReader.fromLinearMemory(utfMem, utfView, wasm32Layout, schema), 'bad utf8');
  expectScene(
    c,
    SceneErrorKind.missingSlab,
    () => SceneReader(
      expectedSchema: schema,
      schemaHash: schema,
      frameId: Key64.zero,
      nodes: ByteData(8),
      nodeCount: 1,
      nodeStride: nodeStride,
      textSlab: Uint8List(0),
      childrenSlab: ByteData(0),
      childrenSlabLen: 0,
      dirtyIndices: ByteData(0),
      dirtyCount: 0,
      removedIds: ByteData(0),
      removedCount: 0,
    ),
    'short nodes slab',
  );
}
