import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  FieldKind,
  Key64,
  LengthUnit,
  NVE_LAYOUT_RECT_LAYOUT,
  NVE_NODE_ROW_LAYOUT,
  NVE_PRIMARY_SLOT_NONE,
  NVE_ROW_INDEX_NO_PAGE,
  NVE_ROW_INDEX_PAGE_LEN,
  NVE_SCENE_LAYOUT_WASM32,
  NVE_SCENE_VIEW_LAYOUT,
  NVE_SCROLL_OFFSET_LAYOUT,
  SceneError,
  SceneReader,
  TableReader,
  decodeLength,
  extractBits,
  extractSigned,
  field,
  rgba,
} from '../src/index.ts';
import type { SceneErrorKind, SceneLayout, SceneSlabs } from '../src/index.ts';

const SCHEMA = new Key64(0xc0ff_ee11, 0xf00d_face); // high bit set on purpose
const VISUAL = Key64.of('nve.visual');
const OPACITY = field(0, FieldKind.U32, 0, 8);
const Z = field(0, FieldKind.I32, 8, 12);
const VISIBLE = field(0, FieldKind.Bool, 20, 1);
const WIDTH = field(1, FieldKind.Length);
const COLOR = field(2, FieldKind.Color);
const LABEL = field(3, FieldKind.Str);
const SCALE = field(5, FieldKind.F32);

const ROWS = [200 | (0xffd << 8) | (1 << 20), 0x4248_0001, 0x8040_2010, 0, 3, 0x3fc0_0000, 2047 << 8, 2, 0, 3, 0, 0x8000_0000];
const TEXT = [0x68, 0xc3, 0xa9, 0xf0, 0x9f, 0x9a, 0x80]; // "hé" + rocket

function words(w: readonly number[]): DataView {
  const d = new DataView(new ArrayBuffer(w.length * 4));
  w.forEach((v, i) => d.setUint32(i * 4, v >>> 0, true));
  return d;
}

/** wasm32 layout generated from the Rust `NveSceneView` / `NveTableView` `#[nve_abi]` structs. */
const WASM32 = NVE_SCENE_LAYOUT_WASM32;
const NODE_ROW_BYTES = NVE_NODE_ROW_LAYOUT.size;
const LAYOUT_RECT_BYTES = NVE_LAYOUT_RECT_LAYOUT.size;
const SCROLL_OFFSET_BYTES = NVE_SCROLL_OFFSET_LAYOUT.size;
const PRIMARY_SLOT_NONE = NVE_PRIMARY_SLOT_NONE;
const CAP = 4; // slot 0 is the null node
const MEMORY_BYTES = 8192;
const NODE_ID = (index: number, generation: number): number => ((generation << 24) | index) >>> 0;
const DENSE_SLOT = 3;
const SPARSE_SLOT = 5;
const NO_PAGE = NVE_ROW_INDEX_NO_PAGE;

interface NodeRow {
  tableSet?: readonly number[];
  parent?: number;
  childOff?: number;
  childLen?: number;
  generation?: number;
  primarySlot?: number;
  effects?: number;
  flags?: number;
}

/** Writes one real 32-byte `NveNodeRow`. */
function nodeRow(r: NodeRow): Uint8Array {
  const d = new DataView(new ArrayBuffer(NODE_ROW_BYTES));
  (r.tableSet ?? [0, 0, 0, 0]).forEach((w, i) => d.setUint32(i * 4, w, true));
  d.setUint32(16, r.parent ?? 0, true);
  d.setUint32(20, r.childOff ?? 0, true);
  d.setUint32(24, r.childLen ?? 0, true);
  d.setUint8(28, r.generation ?? 0);
  d.setUint8(29, r.primarySlot ?? PRIMARY_SLOT_NONE);
  d.setUint8(30, r.effects ?? 0);
  d.setUint8(31, r.flags ?? 0);
  return new Uint8Array(d.buffer);
}

function rect(x: number, y: number, w: number, h: number): Uint8Array {
  const d = new DataView(new ArrayBuffer(LAYOUT_RECT_BYTES));
  [x, y, w, h].forEach((v, i) => d.setFloat32(i * 4, v, true));
  return new Uint8Array(d.buffer);
}

function concat(parts: readonly Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

const asBytes = (d: DataView): Uint8Array => new Uint8Array(d.buffer);

function scrollOffsets(entries: readonly (readonly [number, number, number])[]): Uint8Array {
  const d = new DataView(new ArrayBuffer(entries.length * SCROLL_OFFSET_BYTES));
  entries.forEach(([id, x, y], i) => {
    d.setUint32(i * 12, id, true);
    d.setFloat32(i * 12 + 4, x, true);
    d.setFloat32(i * 12 + 8, y, true);
  });
  return asBytes(d);
}

interface Frame {
  memory: ArrayBuffer;
  mem: DataView;
  view: number;
  /** Byte address of each slab, by name. */
  at: Record<string, number>;
}

/**
 * One frame in linear memory. Nodes: 1 (gen 1, children [2, 3], tables dense+sparse), 2 (gen 2,
 * dense table), 3 (gen 1, sparse table). Dense table slot 3 has one row per node slot; sparse table
 * slot 5 holds node 3 (row 0) and node 1 (row 1) behind a row index of 2 directory entries / 1 page.
 */
function linearMemory(tableWords = 6): Frame {
  const memory = new ArrayBuffer(MEMORY_BYTES);
  const mem = new DataView(memory);
  const bytes = new Uint8Array(memory);
  const at: Record<string, number> = {};
  let cursor = 256;
  const place = (name: string, b: ArrayLike<number>): number => {
    at[name] = cursor;
    bytes.set(b, cursor);
    cursor = (cursor + b.length + 7) & ~7;
    return at[name];
  };
  const live = 1;
  const rowsPtr = place(
    'rows',
    concat([
      nodeRow({}),
      nodeRow({ tableSet: [(1 << DENSE_SLOT) | (1 << SPARSE_SLOT), 0, 0, 0], childOff: 0, childLen: 2, generation: 1, primarySlot: DENSE_SLOT, flags: live }),
      nodeRow({ tableSet: [1 << DENSE_SLOT, 0, 0, 0], parent: NODE_ID(1, 1), generation: 2, effects: 3, flags: live }),
      nodeRow({ tableSet: [1 << SPARSE_SLOT, 0, 0, 0], parent: NODE_ID(1, 1), generation: 1, flags: live }),
    ]),
  );
  place('rects', concat([rect(0, 0, 0, 0), rect(1, 2, 300, 400), rect(-5, 0.5, 10, 20), rect(7, 8, 9, 10)]));
  place('text', TEXT);
  place('children', asBytes(words([NODE_ID(2, 2), NODE_ID(3, 1)])));
  place('dirty', asBytes(words([1, 3])));
  place('moved', asBytes(words([2])));
  place('removed', asBytes(words([42])));
  place('scroll', scrollOffsets([[NODE_ID(1, 1), 1.5, -2], [NODE_ID(2, 2), 3, 4]]));
  place('resources', asBytes(words([10, 20])));
  const denseRows = place('denseRows', asBytes(words([0, 0, 0, 0, 0, 0, ...ROWS, 0, 0, 0, 0, 0, 0])));
  place('denseDirty', asBytes(words([2])));
  const sparseRows = place('sparseRows', asBytes(words([1, 2, 3, 4])));
  place('sparseOwners', asBytes(words([NODE_ID(3, 1), NODE_ID(1, 1)])));
  place('dir', asBytes(words([0, NO_PAGE])));
  const page = new Array<number>(NVE_ROW_INDEX_PAGE_LEN).fill(0);
  page[3] = 1; // node 3 -> row 0
  page[1] = 2; // node 1 -> row 1
  place('pages', asBytes(words(page)));
  const tl = WASM32.tableView;
  const tables = place('tables', new Uint8Array(2 * tl.stride));
  const writeTable = (base: number, fields: readonly (readonly [number, number])[]): void => {
    for (const [off, v] of fields) mem.setUint32(base + off, v >>> 0, true);
  };
  writeTable(tables, [
    [tl.keyLo, VISUAL.lo], [tl.keyHi, VISUAL.hi], [tl.slot, DENSE_SLOT], [tl.words, tableWords], [tl.rowCount, CAP],
    [tl.dirtyCount, 1], [tl.rows, denseRows], [tl.owners, 0], [tl.dirtyRows, at.denseDirty!],
  ]);
  writeTable(tables + tl.stride, [
    [tl.keyLo, 1], [tl.keyHi, 2], [tl.slot, SPARSE_SLOT], [tl.words, 2], [tl.rowCount, 2], [tl.rows, sparseRows],
    [tl.owners, at.sparseOwners!], [tl.rowIndexDirLen, 2], [tl.rowIndexPageCount, 1], [tl.rowIndexDir, at.dir!], [tl.rowIndexPages, at.pages!],
  ]);
  const view = place('view', new Uint8Array(NVE_SCENE_VIEW_LAYOUT.size));
  SCHEMA.writeLE(bytes, view + WASM32.schemaHash);
  new Key64(7, 0xffff_0000).writeLE(bytes, view + WASM32.frameId); // above 2^53
  for (const [off, v] of [
    [WASM32.nodeCapacity, CAP], [WASM32.dirtyCount, 2], [WASM32.removedCount, 1], [WASM32.textSlabLen, TEXT.length],
    [WASM32.childrenSlabLen, 2], [WASM32.tableCount, 2], [WASM32.scrollCount, 2], [WASM32.sceneEpoch, 9],
    [WASM32.dirtyResourceCount, 2], [WASM32.movedCount, 1], [WASM32.rows, rowsPtr], [WASM32.rects, at.rects!],
    [WASM32.textSlab, at.text!], [WASM32.childrenSlab, at.children!], [WASM32.dirtyIndices, at.dirty!],
    [WASM32.removedIds, at.removed!], [WASM32.tables, tables], [WASM32.scrollOffsets, at.scroll!],
    [WASM32.dirtyResources, at.resources!], [WASM32.movedIndices, at.moved!],
  ] as const) mem.setUint32(view + off, v, true);
  return { memory, mem, view, at };
}

function open(f: Frame, layout: SceneLayout = WASM32): SceneReader {
  return SceneReader.fromLinearMemory(f.memory, f.view, layout, SCHEMA);
}

function expectScene(kind: SceneErrorKind, fn: () => unknown, what: string): void {
  assert.throws(fn, (e: unknown) => e instanceof SceneError && e.kind === kind, what);
}

test('field specs, lengths, colors', () => {
  assert.equal(extractBits(field(0, FieldKind.U32, 8, 8), 0xabcd_1234), 0x12);
  assert.equal(extractBits(field(0, FieldKind.U32), 0xabcd_1234), 0xabcd_1234);
  assert.equal(extractSigned(field(0, FieldKind.I32, 0, 3), 7), -1);
  assert.equal(extractSigned(field(0, FieldKind.I32, 0, 3), 4), -4);
  assert.equal(extractSigned(field(0, FieldKind.I32), 0x8000_0000), -0x8000_0000);
  assert.equal(extractSigned(field(0, FieldKind.I32, 31, 1), 0x8000_0000), -1);
  assert.ok(LABEL.fits(5) && !LABEL.fits(4));
  assert.deepEqual(decodeLength(0x3f80_0000), { unit: LengthUnit.Points, value: 1 });
  assert.deepEqual(decodeLength(0x4248_0001), { unit: LengthUnit.Percent, value: 50 });
  assert.deepEqual(decodeLength(2), { unit: LengthUnit.Auto, value: 0 });
  assert.deepEqual(decodeLength(0x3f80_0003), { unit: LengthUnit.Auto, value: 0 });
  assert.deepEqual(rgba(0x8040_2010), { r: 0x10, g: 0x20, b: 0x40, a: 0x80 });
});

test('dense and sparse tables, bounds and invalid shapes', () => {
  const t = new TableReader({ key: VISUAL, slot: 4, words: 6, rowCount: 2, rows: words(ROWS), dirtyRows: words([1]), dirtyCount: 1 });
  assert.ok(!t.isSparse && t.key.equals(VISUAL) && t.slot === 4);
  assert.equal(t.u32(0, OPACITY), 200);
  assert.equal(t.i32(0, Z), -3);
  assert.equal(t.i32(1, Z), 2047);
  assert.ok(t.bool(0, VISIBLE) && !t.bool(1, VISIBLE));
  assert.deepEqual(t.length(0, WIDTH), { unit: LengthUnit.Percent, value: 50 });
  assert.equal(t.length(1, WIDTH).unit, LengthUnit.Auto);
  assert.equal(rgba(t.color(0, COLOR)).a, 0x80);
  assert.deepEqual(t.str(0, LABEL), { offset: 0, len: 3 });
  assert.equal(t.f32(0, SCALE), 1.5);
  assert.ok(Object.is(t.f32(1, SCALE), -0));
  assert.equal(t.u32(1, field(5, FieldKind.U32)), 0x8000_0000, 'u32 stays unsigned');
  assert.equal(t.dirtyRow(0), 1);
  assert.equal(t.rowOf(1), 1);
  assert.equal(t.rowOf(2), null);
  assert.equal(t.owner(1), 1);
  assert.throws(() => t.u32(2, OPACITY), RangeError);
  assert.throws(() => t.word(0, 6), RangeError);
  assert.throws(() => t.str(0, field(5, FieldKind.Str)), RangeError);
  assert.throws(() => t.dirtyRow(1), RangeError);

  const sp = TableReader.sparse({ key: VISUAL, slot: 0, words: 2, rows: words([1, 2, 3, 4]), owners: [0x0100_0007, 0x0200_0003] });
  assert.equal(sp.rowOfNode(3), 1);
  assert.equal(sp.rowOf(0x0100_0003), null, 'stale generation of a live slot');
  assert.equal(sp.owner(0), 0x0100_0007);
  // Rows spread over pages: node 7 (directory slot 0) and node 0x10203 (directory slot 0x102).
  const far = TableReader.sparse({ key: VISUAL, slot: 0, words: 1, rows: words([5, 6]), owners: [7, 0x10203] });
  assert.equal(far.rowOfNode(0x10203), 1);
  assert.equal(far.rowOfNode(0x10204), null, 'empty entry in a present page');
  assert.equal(far.rowOfNode(0x203), null, 'absent page between present ones');
  assert.throws(() => TableReader.sparse({ key: VISUAL, slot: 0, words: 1, rows: words([5, 6]), owners: [7, 0x0100_0007] }), RangeError);
  assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 1, rowCount: 1, rows: words([5]), owners: words([7]) }), RangeError);
  assert.throws(
    () => new TableReader({
      key: VISUAL, slot: 0, words: 1, rowCount: 1, rows: words([5]), owners: words([7]),
      rowIndexDir: words([0]), rowIndexPages: words(new Array<number>(NVE_ROW_INDEX_PAGE_LEN - 1).fill(0)),
    }),
    RangeError,
    'a partial page alone is no row index',
  );
  // Hostile published indices: an entry past rowCount, a page id past the pages, a partial tail page.
  const pageWords = new Array<number>(NVE_ROW_INDEX_PAGE_LEN + 3).fill(0);
  pageWords[1] = 9;
  const hostile = new TableReader({
    key: VISUAL, slot: 0, words: 1, rowCount: 1, rows: words([5]), owners: words([1]),
    rowIndexDir: words([0, 1]), rowIndexPages: words(pageWords),
  });
  assert.equal(hostile.rowOfNode(1), null, 'row + 1 past rowCount');
  assert.equal(hostile.rowOfNode(NVE_ROW_INDEX_PAGE_LEN), null, 'partial tail page is unreachable');
  for (const bad of [NaN, 0.5, -1, 2 ** 32]) assert.equal(hostile.rowOf(bad), null, `sparse rowOf ${bad}`);
  assert.ok(sp.isSparse);
  assert.equal(sp.rowOf(0x0200_0003), 1);
  assert.equal(sp.rowOf(1), null);
  assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 2, rowCount: 3, rows: words([1, 2, 3, 4]) }), RangeError);
  assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 33, rowCount: 0, rows: words([]) }), RangeError);
  assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 0, rowCount: 0, rows: words([]) }), RangeError);
  assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 2, rowCount: 2, rows: words([1, 2, 3, 4]), owners: words([1]) }), RangeError);
  assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 2, rowCount: 0, rows: words([]), dirtyCount: 2 }), RangeError);
});


test('wasm32 scene layout offsets are locked', () => {
  assert.deepEqual(
    [WASM32.schemaHash, WASM32.frameId, WASM32.nodeCapacity, WASM32.rows, WASM32.rects, WASM32.tables, WASM32.movedIndices],
    [0, 8, 16, 56, 60, 80, 92],
  );
  assert.deepEqual([WASM32.tableView.stride, WASM32.tableView.rows, WASM32.tableView.rowIndexPages], [52, 32, 48]);
  assert.equal(NODE_ROW_BYTES, 32);
  assert.equal(LAYOUT_RECT_BYTES, 16);
  assert.equal(NVE_SCROLL_OFFSET_LAYOUT.size, 12);
  assert.equal(NVE_SCENE_VIEW_LAYOUT.size, 96);
});

test('scene from wasm32 linear memory', () => {
  const s = open(linearMemory());
  assert.equal(s.frameId.toBigInt(), 0xffff_0000_0000_0007n, 'frame id above 2^53 exact');
  assert.equal(s.nodeCapacity, CAP);
  assert.equal(s.sceneEpoch, 9);
  assert.deepEqual([s.dirtyCount, s.movedCount, s.removedCount, s.dirtyResourceCount, s.scrollCount], [2, 1, 1, 2, 2]);
  // rows
  assert.ok(!s.isLive(0) && s.isLive(1) && s.isLive(3));
  for (const bad of [CAP, -1, NaN, 0.5]) assert.ok(!s.isLive(bad), `isLive ${bad}`);
  assert.equal(s.generation(2), 2);
  assert.equal(s.primarySlot(1), DENSE_SLOT);
  assert.equal(s.primarySlot(2), PRIMARY_SLOT_NONE);
  assert.equal(s.effects(2), 3);
  assert.equal(s.flags(2), 1);
  assert.equal(s.parentId(2), NODE_ID(1, 1));
  assert.equal(s.parentId(1), 0);
  assert.equal(s.nodeId(2), 0x0200_0002);
  assert.ok(s.hasTable(1, DENSE_SLOT) && s.hasTable(1, SPARSE_SLOT) && !s.hasTable(2, SPARSE_SLOT) && !s.hasTable(3, DENSE_SLOT));
  assert.ok(!s.hasTable(1, 128) && !s.hasTable(1, -1) && !s.hasTable(0, DENSE_SLOT) && !s.hasTable(CAP, DENSE_SLOT));
  // stale / dead ids
  assert.equal(s.indexOf(NODE_ID(2, 2)), 2);
  assert.equal(s.indexOf(NODE_ID(2, 1)), -1, 'stale generation');
  assert.equal(s.indexOf(NODE_ID(0, 0)), -1, 'null node');
  assert.equal(s.indexOf(NODE_ID(CAP, 1)), -1, 'index past capacity');
  // rects
  assert.deepEqual(s.rect(1), [1, 2, 300, 400]);
  assert.deepEqual(s.rect(2), [-5, 0.5, 10, 20]);
  assert.deepEqual(s.rect(3), [7, 8, 9, 10]);
  // scroll
  assert.deepEqual(s.scrollShift(1), [1.5, -2]);
  assert.deepEqual(s.scrollShift(2), [3, 4]);
  assert.deepEqual(s.scrollShift(3), [0, 0]);
  assert.deepEqual(s.scrollShift(0), [0, 0]);
  // children
  assert.equal(s.childCount(1), 2);
  assert.equal(s.childCount(2), 0);
  assert.equal(s.childCount(0), 0);
  assert.equal(s.childId(1, 0), NODE_ID(2, 2));
  assert.equal(s.childId(1, 1), NODE_ID(3, 1));
  assert.throws(() => s.childId(1, 2), RangeError);
  assert.throws(() => s.childId(2, 0), RangeError);
  // change sets
  assert.deepEqual([s.dirtyIndex(0), s.dirtyIndex(1), s.movedIndex(0), s.removedId(0), s.dirtyResource(0), s.dirtyResource(1)], [1, 3, 2, 42, 10, 20]);
  assert.throws(() => s.dirtyIndex(2), RangeError);
  assert.throws(() => s.movedIndex(1), RangeError);
  assert.throws(() => s.removedId(1), RangeError);
  assert.throws(() => s.dirtyResource(2), RangeError);
  // text
  assert.equal(s.text(0, 3), 'hé');
  assert.equal(s.text(3, 4), '🚀');
  assert.equal(s.text(7, 0), '');
  assert.throws(() => s.text(2, 1), RangeError);
  assert.throws(() => s.text(4, 3), RangeError);
  assert.throws(() => s.text(7, 1), RangeError);
  // dense table: row == node slot
  const dense = s.findTable(VISUAL);
  assert.ok(dense && !dense.isSparse && dense.slot === DENSE_SLOT);
  assert.equal(s.tableAtSlot(DENSE_SLOT), dense);
  assert.equal(s.rowOf(dense, 1), 1);
  assert.equal(s.rowOf(dense, 2), 2);
  assert.equal(s.rowOf(dense, 3), -1, 'node 3 has no dense table');
  assert.equal(dense.u32(s.rowOf(dense, 1), OPACITY), 200);
  assert.equal(s.str(dense, 1, LABEL), 'hé');
  assert.equal(dense.dirtyRow(0), 2);
  // sparse table behind rowIndexDir / rowIndexPages
  const sparse = s.findTable(new Key64(1, 2));
  assert.ok(sparse?.isSparse && sparse.slot === SPARSE_SLOT);
  assert.equal(s.tableAtSlot(SPARSE_SLOT), sparse);
  assert.equal(s.rowOf(sparse, 3), 0);
  assert.equal(s.rowOf(sparse, 1), 1);
  assert.equal(s.rowOf(sparse, 2), -1, 'node 2 has no sparse table');
  assert.equal(sparse.word(s.rowOf(sparse, 1), 0), 3);
  assert.equal(sparse.rowOfNode(NVE_ROW_INDEX_PAGE_LEN), null, 'directory entry is NVE_ROW_INDEX_NO_PAGE');
  assert.equal(sparse.rowOfNode(2), null, 'empty entry in a present page');
  assert.equal(sparse.rowOfNode(2 * NVE_ROW_INDEX_PAGE_LEN), null, 'past the directory');
  assert.equal(s.rowOf(null, 1), -1);
  assert.equal(s.findTable(new Key64(3, 0)), null);
  assert.equal(s.tableAtSlot(4), null);
  assert.equal(s.tableAtSlot(128), null);
  assert.equal(s.tableAtSlot(NaN), null);
});

test('a dead row is never validated and never live', () => {
  const f = linearMemory();
  const row3 = f.at.rows! + 3 * NODE_ROW_BYTES;
  f.mem.setUint32(row3 + 24, 99, true); // child_len way outside the slab
  f.mem.setUint8(row3 + 31, 0); // ... but dead
  const s = open(f);
  assert.ok(!s.isLive(3));
  assert.equal(s.childCount(3), 0);
  assert.equal(s.indexOf(NODE_ID(3, 1)), -1, 'dead slot');
  assert.deepEqual(s.scrollShift(3), [0, 0]);
  // Its stale `table_set` bit still names the sparse table: a dead slot resolves no row.
  assert.ok(s.hasTable(3, SPARSE_SLOT));
  assert.equal(s.rowOf(s.tableAtSlot(SPARSE_SLOT), 3), -1, 'dead slot with stale table bits');
});

test('a recycled slot never resolves the previous owner\'s sparse row', () => {
  const f = linearMemory();
  f.mem.setUint8(f.at.rows! + 3 * NODE_ROW_BYTES + 28, 2); // node 3 now generation 2; owner says 1
  const s = open(f);
  assert.ok(s.isLive(3));
  assert.equal(s.rowOf(s.tableAtSlot(SPARSE_SLOT), 3), -1, 'owner generation checked');
  assert.equal(s.rowOf(s.tableAtSlot(SPARSE_SLOT), 1), 1, 'current owner still resolves');
});

test('an empty table with a dangling owners pointer is dense', () => {
  const f = linearMemory();
  const sparseView = f.mem.getUint32(f.view + WASM32.tables, true) + WASM32.tableView.stride;
  f.mem.setUint32(sparseView + WASM32.tableView.rowCount, 0, true); // owners stays non-null
  const t = open(f).tableAtSlot(SPARSE_SLOT)!;
  assert.equal(t.rowCount, 0);
  assert.equal(t.isSparse, false, 'row_count 0 is dense, like C++ is_sparse()');
  assert.equal(t.rowOfNode(1), null);
});

test('scene rejections', () => {
  const f0 = linearMemory();
  expectScene('schemaMismatch', () => SceneReader.fromLinearMemory(f0.memory, f0.view, WASM32, new Key64(1, 0)), 'schema');
  expectScene('invalidTable', () => open(linearMemory(40)), 'table words');
  for (const name of ['rows', 'rects', 'children', 'dirty', 'moved', 'removed', 'scroll', 'resources', 'text'] as const) {
    const f = linearMemory();
    const off = { rows: WASM32.rows, rects: WASM32.rects, children: WASM32.childrenSlab, dirty: WASM32.dirtyIndices, moved: WASM32.movedIndices, removed: WASM32.removedIds, scroll: WASM32.scrollOffsets, resources: WASM32.dirtyResources, text: WASM32.textSlab }[name];
    f.mem.setUint32(f.view + off, 0, true);
    expectScene('missingSlab', () => open(f), `null ${name} slab`);
  }
  const oob = linearMemory();
  oob.mem.setUint32(oob.view + WASM32.textSlabLen, 0xffff_fff0, true);
  expectScene('missingSlab', () => open(oob), 'text past memory');
  const oobRows = linearMemory();
  oobRows.mem.setUint32(oobRows.view + WASM32.nodeCapacity, 0xffff_ffff, true);
  expectScene('missingSlab', () => open(oobRows), 'capacity past memory');
  const tail = linearMemory();
  tail.mem.setUint32(tail.view + WASM32.rows, MEMORY_BYTES - 8, true);
  expectScene('missingSlab', () => open(tail), 'rows slab overruns memory end');
  expectScene('missingSlab', () => SceneReader.fromLinearMemory(f0.memory, MEMORY_BYTES - 2, WASM32, SCHEMA), 'view past memory');
  const hugeTables = linearMemory();
  hugeTables.mem.setUint32(hugeTables.view + WASM32.tables, MEMORY_BYTES - 8, true);
  expectScene('invalidTable', () => open(hugeTables), 'tables past memory');
  const child = linearMemory();
  child.mem.setUint32(child.at.rows! + NODE_ROW_BYTES + 24, 3, true);
  // Child ranges are checked on read, so opening stays O(changed + tables).
  expectScene('missingSlab', () => open(child).childCount(1), 'child range outside slab');
  const childOff = linearMemory();
  childOff.mem.setUint32(childOff.at.rows! + NODE_ROW_BYTES + 20, 0xffff_ffff, true);
  expectScene('missingSlab', () => open(childOff).childId(1, 0), 'child offset outside slab');
  const dirty = linearMemory();
  dirty.mem.setUint32(dirty.at.dirty! + 4, CAP, true);
  expectScene('missingSlab', () => open(dirty), 'dirty index >= capacity');
  const moved = linearMemory();
  moved.mem.setUint32(moved.at.moved!, CAP, true);
  expectScene('missingSlab', () => open(moved), 'moved index >= capacity');
  const unsorted = linearMemory();
  unsorted.mem.setUint32(unsorted.at.scroll!, NODE_ID(2, 2), true);
  expectScene('missingSlab', () => open(unsorted), 'unsorted scroll offsets');
  const dupScroll = linearMemory();
  dupScroll.mem.setUint32(dupScroll.at.scroll! + 12, NODE_ID(1, 1), true);
  expectScene('missingSlab', () => open(dupScroll), 'duplicate scroll offsets');
  const resources = linearMemory();
  resources.mem.setUint32(resources.at.resources!, 30, true);
  expectScene('missingSlab', () => open(resources), 'unsorted dirty resources');
  const dupTable = linearMemory();
  dupTable.mem.setUint32(dupTable.at.tables! + WASM32.tableView.stride + WASM32.tableView.slot, DENSE_SLOT, true);
  expectScene('invalidTable', () => open(dupTable), 'duplicate table slot');
  const slot = linearMemory();
  slot.mem.setUint32(slot.at.tables! + WASM32.tableView.slot, 128, true);
  expectScene('invalidTable', () => open(slot), 'table slot past 127');
  const shortRows = linearMemory();
  shortRows.mem.setUint32(shortRows.at.tables! + WASM32.tableView.rowCount, 0xffff, true);
  expectScene('missingSlab', () => open(shortRows), 'table rows past memory');
  const noIndex = linearMemory();
  noIndex.mem.setUint32(noIndex.at.tables! + WASM32.tableView.stride + WASM32.tableView.rowIndexPageCount, 0, true);
  expectScene('invalidTable', () => open(noIndex), 'sparse table without a row index page');
  const utf = linearMemory();
  utf.mem.setUint8(utf.at.text!, 0xc0);
  utf.mem.setUint8(utf.at.text! + 1, 0xaf);
  expectScene('invalidUtf8', () => open(utf), 'bad utf8');
  const raggedChildren = new DataView(new ArrayBuffer(6));
  expectScene('missingSlab', () => new SceneReader(slabs({ childrenSlab: raggedChildren })), 'children slab not a whole number of u32');
  expectScene('missingSlab', () => new SceneReader(slabs({ nodeCapacity: 2, rows: new DataView(new ArrayBuffer(NODE_ROW_BYTES)) })), 'short rows slab');
  expectScene('missingSlab', () => new SceneReader(slabs({ nodeCapacity: 2, rects: new DataView(new ArrayBuffer(LAYOUT_RECT_BYTES)) })), 'short rects slab');
  expectScene('schemaMismatch', () => new SceneReader(slabs({ schemaHash: new Key64(9, 9) })), 'constructor schema');
});

/** Minimal valid `SceneSlabs` (one node slot) with overrides. */
function slabs(over: Partial<SceneSlabs>): SceneSlabs {
  const empty = new DataView(new ArrayBuffer(0));
  return {
    expectedSchema: SCHEMA, schemaHash: SCHEMA, frameId: Key64.ZERO, nodeCapacity: 1,
    rows: new DataView(new ArrayBuffer(NODE_ROW_BYTES)), rects: new DataView(new ArrayBuffer(LAYOUT_RECT_BYTES)),
    textSlab: new Uint8Array(0), childrenSlab: empty, dirtyIndices: empty, removedIds: empty,
    ...over,
  };
}

test('hardening: specs, NaN / fractional / Infinity arguments, hostile tables, BOM text', () => {
  for (const [w, k, sh, b] of [
    [0, FieldKind.U32, 24, 16], [0, FieldKind.U32, 0, 0], [0, FieldKind.I32, 0, 33], [0, FieldKind.I32, 31, 2],
    [32, FieldKind.U32, 0, 32], [31, FieldKind.Str, 0, 32], [0.5, FieldKind.U32, 0, 8], [0, FieldKind.U32, NaN, 8],
    [0, FieldKind.U32, 0, Infinity], [-1, FieldKind.U32, 0, 8], [0, 10 as FieldKind, 0, 8],
  ] as const) {
    assert.throws(() => field(w, k, sh, b), RangeError, `bad spec ${w}/${sh}/${b}/${k}`);
  }
  const t = new TableReader({ key: VISUAL, slot: 0, words: 6, rowCount: 2, rows: words(ROWS) });
  assert.throws(() => t.u32(0, field(6, FieldKind.U32)), RangeError, 'field past row');
  assert.throws(() => t.i32(0, field(31, FieldKind.I32)), RangeError, 'i32 field past row');
  assert.throws(() => t.str(0, field(3, FieldKind.U32)), RangeError, 'str of a non-string spec');
  assert.throws(() => t.str(0, field(5, FieldKind.Str)), RangeError, 'str past the row');
  assert.deepEqual(t.str(0, LABEL), { offset: 0, len: 3 });
  for (const bad of [NaN, 0.5, Infinity, -1]) {
    assert.throws(() => t.word(bad, 0), RangeError, `row ${bad}`);
    assert.throws(() => t.word(0, bad), RangeError, `word ${bad}`);
    assert.throws(() => t.owner(bad), RangeError, `owner ${bad}`);
    assert.equal(t.rowOf(bad), null, `rowOf ${bad}`);
    assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 6, rowCount: bad, rows: words(ROWS) }), RangeError);
    assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 6, rowCount: 0, rows: words([]), dirtyCount: bad }), RangeError);
    assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: bad, rowCount: 0, rows: words([]) }), RangeError);
  }
  // A plain object literal is not a FieldSpec (nominal class), so it cannot smuggle a bad shift in.
  const forged = { word: 0, shift: 40, bits: 8, kind: FieldKind.U32 } as unknown as ReturnType<typeof field>;
  assert.throws(() => t.u32(0, forged), TypeError);

  const f = linearMemory();
  const s = open(f);
  for (const bad of [NaN, 0.5, 2.5, Infinity, -1]) {
    assert.throws(() => s.textBytes(bad, 1), RangeError, `text offset ${bad}`);
    assert.throws(() => s.textBytes(0, bad), RangeError, `text len ${bad}`);
    assert.throws(() => s.generation(bad), RangeError, `generation ${bad}`);
    assert.throws(() => s.rect(bad), RangeError, `rect ${bad}`);
    assert.throws(() => s.childId(1, bad), RangeError, `child ${bad}`);
    assert.throws(() => s.dirtyIndex(bad), RangeError, `dirty ${bad}`);
    assert.throws(() => s.movedIndex(bad), RangeError, `moved ${bad}`);
    assert.throws(() => s.removedId(bad), RangeError, `removed ${bad}`);
    assert.throws(() => s.dirtyResource(bad), RangeError, `resource ${bad}`);
    assert.ok(!s.isLive(bad), `isLive ${bad}`);
    assert.ok(!s.hasTable(bad, DENSE_SLOT) && !s.hasTable(1, bad), `hasTable ${bad}`);
    assert.deepEqual(s.scrollShift(bad), [0, 0], `scrollShift ${bad}`);
    assert.equal(s.childCount(bad), 0, `childCount ${bad}`);
    assert.equal(s.tableAtSlot(bad), null, `tableAtSlot ${bad}`);
  }
  assert.throws(() => s.generation(CAP), RangeError);
  expectScene('missingSlab', () => SceneReader.fromLinearMemory(f.memory, f.view + 0.5, WASM32, SCHEMA), 'fractional view');
  expectScene('missingSlab', () => SceneReader.fromLinearMemory(f.memory, NaN, WASM32, SCHEMA), 'NaN view');
  for (const bad of [NaN, 0.5, -1, Infinity, 2 ** 32]) {
    expectScene('missingSlab', () => new SceneReader(slabs({ nodeCapacity: bad, rows: new DataView(new ArrayBuffer(0)), rects: new DataView(new ArrayBuffer(0)) })), `nodeCapacity ${bad}`);
  }
  // Layout strides below 1 are rejected before any arithmetic.
  for (const table of [0, -52, NaN, 0.5]) {
    const bad: SceneLayout = { ...WASM32, tableView: { ...WASM32.tableView, stride: table } };
    expectScene('missingSlab', () => open(f, bad), `table stride ${table}`);
  }
  // Hostile wasm32 table words are rejected before any arithmetic.
  const hostile = linearMemory(0x8000_0000);
  hostile.mem.setUint32(hostile.at.tables! + WASM32.tableView.rowCount, 0x8000_0000, true);
  expectScene('invalidTable', () => open(hostile), 'hostile words');
  // A leading BOM is scene text, never stripped.
  const bom = linearMemory();
  new Uint8Array(bom.memory).set([0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf, 0x78], bom.at.text);
  const b = open(bom);
  assert.equal(b.text(0, 7), '\ufeff\ufeffx');
  assert.equal(b.text(0, 3), '\ufeff');
});
