import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';

import 'check.dart';
import 'table_scene_suite.dart' show expectScene;

const Key64 _schema = Key64(0x54181C0E, 0x9CA6E2D1);

/// Slot of the dense `nve.visual` and sparse `nve.text` tables in the synthetic frame.
const int _visualSlot = 3;
const int _textSlot = 37; // second table_set word: proves the 128-bit bitset indexing

/// Synthetic 4-slot frame: 0 = null, 1 = root (gen 0) with children [2, 3], 2 = text leaf (gen 5),
/// 3 = container (gen 1), with node 3 dead (not live) to prove liveness checks.
final class _Frame {
  _Frame() {
    void row(int i, {required int gen, required int flags, int parent = 0, int off = 0, int len = 0, List<int> slots = const []}) {
      final b = i * nodeRowWords;
      for (final s in slots) {
        rows[b + (s >> 5)] |= 1 << (s & 31);
      }
      rows[b + 4] = parent;
      rows[b + 5] = off;
      rows[b + 6] = len;
      rows[b + 7] = gen | (0xFF << 8) | (flags << 24);
    }

    row(1, gen: 0, flags: 1, off: 0, len: 2, slots: [_visualSlot]);
    row(2, gen: 5, flags: 1, parent: 1, slots: [_visualSlot, _textSlot]);
    row(3, gen: 1, flags: 0, parent: 1);
    rects.setAll(4, [0, 0, 320, 200, 8, 16, 100.5, 20]);
  }

  final rows = Uint32List(4 * nodeRowWords);
  final rects = Float32List(4 * layoutRectFloats);
  final text = Uint8List.fromList([0x68, 0xC3, 0xA9, 0x6C, 0x6C, 0x6F]); // "héllo"
  final children = Uint32List.fromList([2 | (5 << 24), 3 | (1 << 24)]);
  final dirty = Uint32List.fromList([1, 2]);
  final removed = Uint32List.fromList([9 | (2 << 24)]);

  TableReader visual() {
    final words = Uint32List(4 * VisualTable.words);
    words[2 * VisualTable.words + VisualTable.background.word] = 0xFF2010F0;
    return TableReader(
      key: VisualTable.key,
      slot: _visualSlot,
      words: VisualTable.words,
      rowCount: 4,
      rows: ByteData.sublistView(words),
    );
  }

  TableReader textTable() => TableReader.sparse(
        key: TextTable.key,
        slot: _textSlot,
        words: TextTable.words,
        rows: ByteData.sublistView(Uint32List.fromList([0, 6])),
        owners: [2 | (5 << 24)],
      );

  SceneReader open({
    Key64 schema = _schema,
    int? capacity,
    Uint32List? rows,
    Float32List? rects,
    Uint8List? text,
    Uint32List? children,
    Uint32List? dirty,
    List<TableReader>? tables,
    Uint32List? scroll,
  }) =>
      SceneReader(
        expectedSchema: _schema,
        schemaHash: schema,
        frameId: const Key64(7, 0xFFFF0000),
        nodeCapacity: capacity ?? 4,
        rows: rows ?? this.rows,
        rects: rects ?? this.rects,
        textSlab: text ?? this.text,
        childrenSlab: children ?? this.children,
        dirtyIndices: dirty ?? this.dirty,
        removedIds: removed,
        tables: tables ?? [visual(), textTable()],
        scrollOffsets: scroll,
      );
}

/// `NveScrollOffset[]` words for `(nodeId, x, y)` entries.
Uint32List _scrollSlab(List<(int, double, double)> entries) {
  final words = Uint32List(entries.length * scrollOffsetWords);
  final floats = Float32List.sublistView(words);
  for (var i = 0; i < entries.length; i++) {
    final (id, x, y) = entries[i];
    words[i * 3] = id;
    floats[i * 3 + 1] = x;
    floats[i * 3 + 2] = y;
  }
  return words;
}

void _runScrollOffsets(Check c, _Frame f) {
  final base = f.open();
  c.eq(base.scrollCount, 0, 'no scroll slab = nothing scrolled');
  c.eq(base.scrollShift(2), (0.0, 0.0), 'unscrolled shift is zero');
  final id1 = base.nodeId(1), id2 = base.nodeId(2);
  final s = f.open(scroll: _scrollSlab([(id1, 0, 12.5), (id2, 3, 40)]));
  c.eq(s.scrollCount, 2, 'scroll count');
  c.eq(s.scrollShift(1), (0.0, 12.5), 'first entry');
  c.eq(s.scrollShift(2), (3.0, 40.0), 'last entry (binary search)');
  c.eq(s.scrollShift(3), (0.0, 0.0), 'unlisted node');
  c.eq(s.scrollShift(0), (0.0, 0.0), 'null slot');
  final stale = f.open(scroll: _scrollSlab([((id2 & nodeIndexMask) | (((id2 >>> 24) + 1 & 0xFF) << 24), 0, 9)]));
  c.eq(stale.scrollShift(2), (0.0, 0.0), 'stale generation never matches');
  for (final (what, bad) in [
    ('ragged slab', Uint32List(4)),
    ('descending ids', _scrollSlab([(id2, 0, 1), (id1, 0, 1)])),
    ('duplicate ids', _scrollSlab([(id1, 0, 1), (id1, 0, 2)])),
    ('null node id', _scrollSlab([(0, 0, 1)])),
  ]) {
    c.throwsA<SceneException>(() => f.open(scroll: bad), 'rejects $what');
  }
}

/// `ResourceVersions`: stable while bytes are, bumped exactly for dirty ids, reset on a new epoch
/// or a frame gap (including the `u32` carry of the 64-bit frame id), never reused.
void _runResourceVersions(Check c) {
  final v = ResourceVersions();
  Uint32List ids(List<int> l) => Uint32List.fromList(l);
  v.observe(0, const Key64(1, 0), ids([]));
  final a = v.versionOf(7), b = v.versionOf(9);
  c.that(a != b, 'distinct ids, distinct versions');
  v.observe(0, const Key64(1, 0), ids([7]));
  c.eq(v.versionOf(7), a, 'repeat observe of one frame is a no-op');
  v.observe(0, const Key64(2, 0), ids([7]));
  final a2 = v.versionOf(7);
  c.that(a2 != a && v.versionOf(9) == b, 'dirty id re-versioned, clean id kept');
  v.observe(0, const Key64(4, 0), ids([]));
  final b3 = v.versionOf(9);
  c.that(b3 != b && v.versionOf(7) != a2, 'frame gap drops every version');
  v.observe(1, const Key64(5, 0), ids([]));
  c.that(v.versionOf(9) != b3, 'new scene epoch drops every version');
  v.observe(1, const Key64(0xFFFFFFFF, 3), ids([]));
  final carried = v.versionOf(9);
  v.observe(1, const Key64(0, 4), ids([]));
  c.eq(v.versionOf(9), carried, 'consecutive across the lo -> hi carry');
  final other = ResourceVersions()..observe(0, const Key64(1, 0), ids([]));
  c.that(other.versionOf(7) > carried, 'versions are never reused across instances');
}

void runRuntimeScene(Check c) {
  final f = _Frame();
  _runScrollOffsets(c, f);
  _runResourceVersions(c);
  final s = f.open();

  c.eq(s.frameId, const Key64(7, 0xFFFF0000), 'frameId exact above 2^53');
  c.eq(s.nodeCapacity, 4, 'capacity');
  c.that(!s.isLive(0), 'slot 0 (null) never live');
  c.that(s.isLive(1) && s.isLive(2) && !s.isLive(3), 'liveness from flags');
  c.that(!s.isLive(4) && !s.isLive(-1), 'out-of-range slots are not live');
  c.eq(s.generation(2), 5, 'generation');
  c.eq(s.primarySlot(1), primarySlotNone, 'primary slot');
  c.eq(s.parentId(2), 1, 'parent');
  c.eq(s.nodeId(2), 2 | (5 << 24), 'nodeId packs generation');
  c.eq(s.indexOf(2 | (5 << 24)), 2, 'indexOf live id');
  c.eq(s.indexOf(2 | (4 << 24)), -1, 'stale generation rejected');
  c.eq(s.indexOf(3 | (1 << 24)), -1, 'dead slot rejected');
  c.eq(s.indexOf(0), -1, 'null id rejected');
  c.that(s.hasTable(2, _textSlot) && s.hasTable(2, _visualSlot) && !s.hasTable(1, _textSlot), 'table_set bits');
  c.that(!s.hasTable(2, 128) && !s.hasTable(2, -1), 'slot outside 0..127 never attached');
  c.eq([s.x(2), s.y(2), s.width(2), s.height(2)].join(','), [8.0, 16.0, 100.5, 20.0].join(','), 'rect');
  c.eq(s.childCount(1), 2, 'child count');
  c.eq(s.children(1).join(','), [2 | (5 << 24), 3 | (1 << 24)].join(','), 'children are raw ids');
  c.eq(s.children(3).length, 0, 'dead node has no children');
  c.eq(s.dirtyIndices.join(','), '1,2', 'dirty');
  c.eq(s.removedIds.join(','), '${9 | (2 << 24)}', 'removed');

  final visual = s.findTable(VisualTable.key)!;
  c.that(identical(s.tableAtSlot(_visualSlot), visual), 'slot index');
  c.eq(s.tableAtSlot(200), null, 'slot out of range');
  c.eq(s.rowOf(visual, 2), 2, 'dense row = node index');
  c.eq(s.rowOf(visual, 3), -1, 'dense row requires attachment');
  c.eq(s.rowOf(null, 2), -1, 'absent table');
  c.eq(visual.color(2, VisualTable.background).packed, 0xFF2010F0, 'dense color');
  final text = s.tableAtSlot(_textSlot)!;
  c.eq(s.rowOf(text, 2), 0, 'sparse row via the published row index');
  c.eq(s.rowOf(text, 1), -1, 'sparse table not attached');
  c.eq(s.str(text, 0, TextTable.text), 'héllo', 'str field decodes UTF-8');
  c.eq(s.text(0, 0), '', 'empty text');
  // A leading BOM is scene text, not stripped (dart:convert would drop it).
  final bom = f.open(text: Uint8List.fromList([0xEF, 0xBB, 0xBF, 0xEF, 0xBB, 0xBF, 0x78]));
  c.eq(bom.text(0, 7), '\uFEFF\uFEFFx', 'scene double BOM');
  c.eq(bom.text(0, 3), '\uFEFF', 'scene lone BOM');
  c.eq(bom.text(0, 7).length, 3, 'scene BOM length');
  c.throwsA<RangeError>(() => s.textBytes(2, 1), 'text inside a code point');
  c.throwsA<RangeError>(() => s.textBytes(4, 9), 'text past slab');

  // ---- ingress validation ------------------------------------------------------------------
  expectScene(c, SceneErrorKind.schemaMismatch, () => f.open(schema: Key64.zero), 'schema');
  expectScene(c, SceneErrorKind.missingSlab, () => f.open(capacity: 5), 'rows shorter than capacity');
  expectScene(c, SceneErrorKind.missingSlab, () => f.open(rects: Float32List(15)), 'rects shorter than capacity');
  expectScene(c, SceneErrorKind.missingSlab, () => f.open(capacity: -1), 'negative capacity');
  expectScene(c, SceneErrorKind.invalidUtf8, () => f.open(text: Uint8List.fromList([0xC3])), 'truncated UTF-8');
  expectScene(c, SceneErrorKind.missingSlab, () => f.open(children: Uint32List(1)), 'child range past slab');
  expectScene(c, SceneErrorKind.missingSlab, () => f.open(dirty: Uint32List.fromList([4])), 'dirty index >= capacity');
  expectScene(c, SceneErrorKind.invalidTable, () => f.open(tables: [f.visual(), f.visual()]), 'duplicate slot');
  final huge = Uint32List.fromList(f.rows)..[1 * nodeRowWords + 5] = 0xFFFFFFFF;
  expectScene(c, SceneErrorKind.missingSlab, () => f.open(rows: huge), 'hostile child offset cannot wrap');
  final stale = Uint32List.fromList(f.rows)
    ..[1 * nodeRowWords + 6] = 0
    ..[3 * nodeRowWords + 5] = 99; // dead node 3 keeps a stale out-of-slab range
  c.eq(f.open(children: Uint32List(0), rows: stale).childCount(1), 0, 'dead nodes with stale child ranges are not validated');

  // ---- generated core table layouts ---------------------------------------------------------
  c.that(!TextTable.dense && VisualTable.dense, 'storage');
  c.eq(VisualTable.defaults.length, VisualTable.words, 'defaults cover every word');
  c.that(TypographyTable.fontSize.fits(TypographyTable.words), 'typography fontSize fits');
  c.eq(f32FromBits(TypographyTable.defaults[TypographyTable.fontSize.word]), 16.0, 'typography default font size');
  c.eq(NveOverflow.hidden, 1, 'word enum value');
  c.eq(NveAriaTriState.trueValue, 2, 'reserved-word variant renamed');
  c.eq(NveTypographySet.fontStyle, 1 << 7, 'typography set bit');
  c.eq(NveTextDecoration.lineThrough, 4, 'text decoration bit');
}
