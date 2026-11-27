import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  FieldKind,
  Key64,
  LengthUnit,
  SceneError,
  SceneReader,
  TableReader,
  decodeLength,
  extractBits,
  extractSigned,
  field,
  rgba,
} from '../src/index.ts';
import type { SceneErrorKind, SceneLayout } from '../src/index.ts';

const SCHEMA = new Key64(0xc0ff_ee11, 0xf00d_face); // high bit set on purpose
const VISUAL = Key64.of('nve.visual');
const OPACITY = field(0, FieldKind.U32, 0, 8);
const Z = field(0, FieldKind.I32, 8, 12);
const VISIBLE = field(0, FieldKind.Bool, 20, 1);
const WIDTH = field(1, FieldKind.Length);
const COLOR = field(2, FieldKind.Color);
const LABEL = field(3, FieldKind.Str);
const SCALE = field(5, FieldKind.F32);
const NODE_STRIDE = 16; // test node: id u32 @0, text_offset u32 @4, text_len u16 @8, child_count u16 @10, child_offset u32 @12

const ROWS = [200 | (0xffd << 8) | (1 << 20), 0x4248_0001, 0x8040_2010, 0, 3, 0x3fc0_0000, 2047 << 8, 2, 0, 3, 0, 0x8000_0000];
const TEXT = [0x68, 0xc3, 0xa9, 0xf0, 0x9f, 0x9a, 0x80]; // "hé" + rocket

function words(w: readonly number[]): DataView {
  const d = new DataView(new ArrayBuffer(w.length * 4));
  w.forEach((v, i) => d.setUint32(i * 4, v >>> 0, true));
  return d;
}

/** wasm32 layout locked by Rust `#[nve_abi(offset = ..)]` (pointers are 4 bytes on wasm32). */
const WASM32: SceneLayout = {
  schemaHash: 0, frameId: 8, nodeCount: 16, dirtyCount: 20, removedCount: 24, textSlabLen: 28,
  childrenSlabLen: 32, tableCount: 36, nodes: 40, textSlab: 44, childrenSlab: 48, dirtyIndices: 52,
  removedIds: 56, tables: 60, nodeStride: NODE_STRIDE,
  tableView: { stride: 36, keyLo: 0, keyHi: 4, slot: 8, words: 12, rowCount: 16, dirtyCount: 20, rows: 24, owners: 28, dirtyRows: 32 },
};

/** One frame in a 4 KiB linear memory; returns [memory, viewPtr]. */
function linearMemory(tableWords = 6): [ArrayBuffer, DataView, number] {
  const memory = new ArrayBuffer(4096);
  const mem = new DataView(memory);
  const bytes = new Uint8Array(memory);
  let cursor = 256;
  const place = (b: ArrayLike<number>): number => {
    const at = cursor;
    bytes.set(b, at);
    cursor = (cursor + b.length + 7) & ~7;
    return at;
  };
  const asBytes = (d: DataView): Uint8Array => new Uint8Array(d.buffer);
  const nodes = new DataView(new ArrayBuffer(3 * NODE_STRIDE));
  [[1, 0, 3, 2, 0], [2, 3, 4, 0, 2], [3, 7, 0, 0, 2]].forEach(([id, to, tl, cc, co], i) => {
    const b = i * NODE_STRIDE;
    nodes.setUint32(b, id!, true);
    nodes.setUint32(b + 4, to!, true);
    nodes.setUint16(b + 8, tl!, true);
    nodes.setUint16(b + 10, cc!, true);
    nodes.setUint32(b + 12, co!, true);
  });
  const nodesPtr = place(asBytes(nodes));
  const textPtr = place(TEXT);
  const childrenPtr = place(asBytes(words([2, 3])));
  const dirtyPtr = place(asBytes(words([0, 2, 99])));
  const removedPtr = place(asBytes(words([42])));
  const rowsPtr = place(asBytes(words(ROWS)));
  const rowDirtyPtr = place(asBytes(words([1])));
  const t = place(new Uint8Array(36));
  const tl = WASM32.tableView;
  for (const [off, v] of [
    [tl.keyLo, VISUAL.lo], [tl.keyHi, VISUAL.hi], [tl.slot, 3], [tl.words, tableWords], [tl.rowCount, 2],
    [tl.dirtyCount, 1], [tl.rows, rowsPtr], [tl.owners, 0], [tl.dirtyRows, rowDirtyPtr],
  ] as const) mem.setUint32(t + off, v, true);
  const view = place(new Uint8Array(64));
  SCHEMA.writeLE(bytes, view + WASM32.schemaHash);
  new Key64(7, 0xffff_0000).writeLE(bytes, view + WASM32.frameId); // above 2^53
  for (const [off, v] of [
    [WASM32.nodeCount, 3], [WASM32.dirtyCount, 3], [WASM32.removedCount, 1], [WASM32.textSlabLen, 7],
    [WASM32.childrenSlabLen, 2], [WASM32.tableCount, 1], [WASM32.nodes, nodesPtr], [WASM32.textSlab, textPtr],
    [WASM32.childrenSlab, childrenPtr], [WASM32.dirtyIndices, dirtyPtr], [WASM32.removedIds, removedPtr], [WASM32.tables, t],
  ] as const) mem.setUint32(view + off, v, true);
  return [memory, mem, view];
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

  const sp = new TableReader({ key: VISUAL, slot: 0, words: 2, rowCount: 2, rows: words([1, 2, 3, 4]), owners: words([0x0001_0007, 0x0002_0003]) });
  assert.ok(sp.isSparse);
  assert.equal(sp.rowOf(0x0002_0003), 1);
  assert.equal(sp.rowOf(1), null);
  assert.equal(sp.owner(0), 0x0001_0007);
  assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 2, rowCount: 3, rows: words([1, 2, 3, 4]) }), RangeError);
  assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 33, rowCount: 0, rows: words([]) }), RangeError);
  assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 0, rowCount: 0, rows: words([]) }), RangeError);
  assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 2, rowCount: 2, rows: words([1, 2, 3, 4]), owners: words([1]) }), RangeError);
  assert.throws(() => new TableReader({ key: VISUAL, slot: 0, words: 2, rowCount: 0, rows: words([]), dirtyCount: 2 }), RangeError);
});

test('scene from wasm32 linear memory', () => {
  const [memory, , viewPtr] = linearMemory();
  const s = SceneReader.fromLinearMemory(memory, viewPtr, WASM32, SCHEMA);
  assert.equal(s.frameId.toBigInt(), 0xffff_0000_0000_0007n, 'frame id above 2^53 exact');
  assert.equal(s.nodeCount, 3);
  assert.equal(s.nodeU32(1, 0), 2);
  assert.equal(s.text(s.nodeU32(0, 4), s.nodeU16(0, 8)), 'hé');
  assert.equal(s.text(3, 4), '🚀');
  assert.equal(s.text(7, 0), '');
  assert.throws(() => s.text(2, 1), RangeError);
  assert.throws(() => s.text(4, 3), RangeError);
  assert.throws(() => s.text(7, 1), RangeError);
  assert.equal(s.childId(s.nodeU32(0, 12) + 1), 3);
  assert.throws(() => s.childId(2), RangeError);
  assert.throws(() => s.nodeU32(3, 0), RangeError);
  assert.throws(() => s.nodeU32(0, 13), RangeError);
  assert.equal(s.nodeU8(0, 0), 1);
  assert.equal(s.nodeI32(0, 0), 1);
  assert.equal(s.nodeF32(0, 0), 1.401298464324817e-45);
  const visited: number[] = [];
  s.forEachDirty((i) => visited.push(i));
  assert.deepEqual(visited, [0, 2]);
  assert.equal(s.removedId(0), 42);
  const t = s.findTable(VISUAL);
  assert.ok(t);
  assert.equal(t.u32(0, OPACITY), 200);
  assert.equal(t.slot, 3);
  assert.equal(t.dirtyRow(0), 1);
  assert.equal(s.findTable(new Key64(1, 0)), null);
});

test('scene rejections', () => {
  const [memory, , viewPtr] = linearMemory();
  expectScene('schemaMismatch', () => SceneReader.fromLinearMemory(memory, viewPtr, WASM32, new Key64(1, 0)), 'schema');
  const [badT, , badTView] = linearMemory(40);
  expectScene('invalidTable', () => SceneReader.fromLinearMemory(badT, badTView, WASM32, SCHEMA), 'table words');
  const [nul, nulMem, nulView] = linearMemory();
  nulMem.setUint32(nulView + WASM32.childrenSlab, 0, true);
  expectScene('missingSlab', () => SceneReader.fromLinearMemory(nul, nulView, WASM32, SCHEMA), 'null children');
  const [oob, oobMem, oobView] = linearMemory();
  oobMem.setUint32(oobView + WASM32.textSlabLen, 0xffff_fff0, true);
  expectScene('missingSlab', () => SceneReader.fromLinearMemory(oob, oobView, WASM32, SCHEMA), 'text past memory');
  expectScene('missingSlab', () => SceneReader.fromLinearMemory(memory, 4094, WASM32, SCHEMA), 'view past memory');
  const [utf, utfMem, utfView] = linearMemory();
  const textPtr = utfMem.getUint32(utfView + WASM32.textSlab, true);
  utfMem.setUint8(textPtr, 0xc0);
  utfMem.setUint8(textPtr + 1, 0xaf);
  expectScene('invalidUtf8', () => SceneReader.fromLinearMemory(utf, utfView, WASM32, SCHEMA), 'bad utf8');
  const empty = new DataView(new ArrayBuffer(0));
  expectScene(
    'missingSlab',
    () =>
      new SceneReader({
        expectedSchema: SCHEMA, schemaHash: SCHEMA, frameId: Key64.ZERO, nodes: new DataView(new ArrayBuffer(8)), nodeCount: 1,
        nodeStride: NODE_STRIDE, textSlab: new Uint8Array(0), childrenSlab: empty, childrenSlabLen: 0,
        dirtyIndices: empty, dirtyCount: 0, removedIds: empty, removedCount: 0,
      }),
    'short nodes slab',
  );
});

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

  const [memory, , viewPtr] = linearMemory();
  const s = SceneReader.fromLinearMemory(memory, viewPtr, WASM32, SCHEMA);
  for (const bad of [NaN, 0.5, 2.5, Infinity, -1]) {
    assert.throws(() => s.textBytes(bad, 1), RangeError, `text offset ${bad}`);
    assert.throws(() => s.textBytes(0, bad), RangeError, `text len ${bad}`);
    assert.throws(() => s.nodeU32(bad, 0), RangeError, `node ${bad}`);
    assert.throws(() => s.nodeU32(0, bad), RangeError, `node offset ${bad}`);
    assert.throws(() => s.childId(bad), RangeError, `child ${bad}`);
    assert.throws(() => s.dirtyIndex(bad), RangeError, `dirty ${bad}`);
    assert.throws(() => s.removedId(bad), RangeError, `removed ${bad}`);
  }
  expectScene('missingSlab', () => SceneReader.fromLinearMemory(memory, viewPtr + 0.5, WASM32, SCHEMA), 'fractional view');
  expectScene('missingSlab', () => SceneReader.fromLinearMemory(memory, NaN, WASM32, SCHEMA), 'NaN view');
  const empty = new DataView(new ArrayBuffer(0));
  for (const bad of [NaN, 0.5, -1, Infinity]) {
    expectScene(
      'missingSlab',
      () =>
        new SceneReader({
          expectedSchema: SCHEMA, schemaHash: SCHEMA, frameId: Key64.ZERO, nodes: empty, nodeCount: bad,
          nodeStride: NODE_STRIDE, textSlab: new Uint8Array(0), childrenSlab: empty, childrenSlabLen: 0,
          dirtyIndices: empty, dirtyCount: 0, removedIds: empty, removedCount: 0,
        }),
      `nodeCount ${bad}`,
    );
  }
  // Layout strides below 1 are rejected before any arithmetic.
  for (const [node, table] of [[0, 36], [-1, 36], [NaN, 36], [16, 0], [16, -36], [16, 0.5]] as const) {
    const bad: SceneLayout = { ...WASM32, nodeStride: node, tableView: { ...WASM32.tableView, stride: table } };
    expectScene('missingSlab', () => SceneReader.fromLinearMemory(memory, viewPtr, bad, SCHEMA), `strides ${node}/${table}`);
  }
  // Hostile wasm32 table words are rejected before any arithmetic.
  const [hostile, hostileMem, hostileView] = linearMemory(0x8000_0000);
  const tablePtr = hostileMem.getUint32(hostileView + WASM32.tables, true);
  hostileMem.setUint32(tablePtr + WASM32.tableView.rowCount, 0x8000_0000, true);
  expectScene('invalidTable', () => SceneReader.fromLinearMemory(hostile, hostileView, WASM32, SCHEMA), 'hostile words');
  // A leading BOM is scene text, never stripped.
  const [bomMem, bomView, bomPtr] = linearMemory();
  const textPtr = bomView.getUint32(bomPtr + WASM32.textSlab, true);
  new Uint8Array(bomMem).set([0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf, 0x78], textPtr);
  const b = SceneReader.fromLinearMemory(bomMem, bomPtr, WASM32, SCHEMA);
  assert.equal(b.text(0, 7), '\ufeff\ufeffx');
  assert.equal(b.text(0, 3), '\ufeff');
});
