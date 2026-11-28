import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  FieldKind,
  FieldSpec,
  GridItemTable,
  GridLineKind,
  InteractionTable,
  Key64,
  LayoutTable,
  LengthUnit,
  NO_VAL_WORD,
  NVE_INTERACTION_EVENT,
  NVE_LAYOUT_EVENT,
  NVE_LAYOUT_RECT_LAYOUT,
  NVE_NODE_ROW_FLAG_LIVE,
  NVE_NODE_ROW_LAYOUT,
  NVE_PRIMARY_SLOT_NONE,
  NVE_SCROLL_EVENT,
  OptEncoding,
  SceneReader,
  ScrollTable,
  TableReader,
  TypographyTable,
  decodeGridLine,
  f64FromBits,
  field,
} from '../src/index.ts';

function words(w: readonly number[]): DataView {
  const d = new DataView(new ArrayBuffer(w.length * 4));
  w.forEach((v, i) => d.setUint32(i * 4, v >>> 0, true));
  return d;
}

function f32Bits(v: number): number {
  const d = new DataView(new ArrayBuffer(4));
  d.setFloat32(0, v, true);
  return d.getUint32(0, true);
}

function f64Halves(v: number): [number, number] {
  const d = new DataView(new ArrayBuffer(8));
  d.setFloat64(0, v, true);
  return [d.getUint32(0, true), d.getUint32(4, true)];
}

test('GridLine decoding and TagXor Option<GridLine> parity with Rust', () => {
  assert.deepEqual(decodeGridLine(0), { kind: GridLineKind.Auto, value: 0 });
  assert.deepEqual(decodeGridLine(0xc000), { kind: GridLineKind.Auto, value: 0 }, 'unused tag 11 -> Auto');
  assert.deepEqual(decodeGridLine(0x4000 | 0), { kind: GridLineKind.Line, value: 0 });
  assert.deepEqual(decodeGridLine(0x4000 | 1), { kind: GridLineKind.Line, value: 1 });
  assert.deepEqual(decodeGridLine(0x4000 | 0x3fff), { kind: GridLineKind.Line, value: -1 });
  assert.deepEqual(decodeGridLine(0x4000 | 0x2000), { kind: GridLineKind.Line, value: -8192 });
  assert.deepEqual(decodeGridLine(0x4000 | 0x1fff), { kind: GridLineKind.Line, value: 8191 });
  assert.deepEqual(decodeGridLine(0x8000 | 0), { kind: GridLineKind.Span, value: 0 });
  assert.deepEqual(decodeGridLine(0x8000 | 3), { kind: GridLineKind.Span, value: 3 });
  assert.deepEqual(decodeGridLine(0x8000 | 16383), { kind: GridLineKind.Span, value: 16383 });

  const colStart = GridItemTable.fields.columnStart;
  const colEnd = GridItemTable.fields.columnEnd;
  const rowStart = GridItemTable.fields.rowStart;
  const rowEnd = GridItemTable.fields.rowEnd;
  const t = new TableReader({
    key: GridItemTable.key,
    slot: 0,
    words: GridItemTable.words,
    rowCount: 1,
    rows: words([
      (0x4000 | (0x3ffe & 0x3fff)) | ((0x8000 | 4) << 16), // Line(-2), Span(4)
      (0x4000 | 7) | (0 << 16), // Line(7), Auto
    ]),
  });
  assert.deepEqual(t.gridLine(0, colStart), { kind: GridLineKind.Line, value: -2 });
  assert.deepEqual(t.gridLine(0, colEnd), { kind: GridLineKind.Span, value: 4 });
  assert.deepEqual(t.gridLine(0, rowStart), { kind: GridLineKind.Line, value: 7 });
  assert.deepEqual(t.gridLine(0, rowEnd), { kind: GridLineKind.Auto, value: 0 });

  // TagXor optional GridLine (`0 = None`, valid tags XORed with `0xC000`).
  const optGl = field(0, FieldKind.GridLine, 0, 16, OptEncoding.TagXor);
  const optT = new TableReader({
    key: Key64.of('test.opt_gl'),
    slot: 0,
    words: 1,
    rowCount: 5,
    rows: words([
      0, // None
      0x0042, // tag bits 00 with non-zero payload -> None
      0x0000 ^ 0xc000, // Some(Auto)
      (0x4000 | 0x3fff) ^ 0xc000, // Some(Line(-1))
      (0x8000 | 5) ^ 0xc000, // Some(Span(5))
    ]),
  });
  assert.equal(optT.isPresent(0, optGl), false);
  assert.equal(optT.optGridLine(0, optGl), null);
  assert.equal(optT.optGridLine(1, optGl), null);
  assert.deepEqual(optT.optGridLine(2, optGl), { kind: GridLineKind.Auto, value: 0 });
  assert.deepEqual(optT.optGridLine(3, optGl), { kind: GridLineKind.Line, value: -1 });
  assert.deepEqual(optT.optGridLine(4, optGl), { kind: GridLineKind.Span, value: 5 });
});

test('all 6 OptEncoding strategies decode exactly like Rust', () => {
  const fReq = field(0, FieldKind.U32, 0, 8, OptEncoding.Required);
  const fZero = field(0, FieldKind.U32, 8, 8, OptEncoding.ZeroNiche);
  const fBit0U = field(1, FieldKind.U32, 0, 9, OptEncoding.Bit0Present);
  const fBit0I = field(1, FieldKind.I32, 9, 17, OptEncoding.Bit0Present); // Option<i16>
  const fPlusEnum = field(2, FieldKind.Enum, 0, 4, OptEncoding.PlusOne);
  const fPlusBool = field(2, FieldKind.Enum, 4, 2, OptEncoding.PlusOne);
  const fTagLen = field(3, FieldKind.Length, 0, 32, OptEncoding.TagXor);
  const fWideF = field(2, FieldKind.F32, 6, 1, OptEncoding.WideBit, 4);
  const fWideI = field(2, FieldKind.I32, 7, 1, OptEncoding.WideBit, 5);
  const fWideC = field(2, FieldKind.Color, 8, 1, OptEncoding.WideBit, 6);

  const rowNone = [0x0000_002a, 0x01fe | (0x1_fffe << 9), 0x0000_0030, 0x1234_5600, f32Bits(99), -1, 0xdead_beef];
  const rowSome = [
    0x0000_072a, // req=0x2a, zeroNiche=7
    ((255 << 1) | 1) | ((((0xffff << 1) | 1) & 0x1ffff) << 9), // Some(255u8), Some(-1i16)
    (3) | (2 << 4) | (1 << 6) | (1 << 7) | (1 << 8), // plusEnum=Some(2), plusBool=Some(true), wide bits set
    0x4248_0001 ^ 3, // Some(Length::percent(50))
    f32Bits(1.75),
    -42,
    0x8040_2010,
  ];
  const rowEdge = [
    0,
    ((0 << 1) | 1) | ((((0x8000 << 1) | 1) & 0x1ffff) << 9), // Some(0u8), Some(-32768i16)
    (1) | (1 << 4), // plusEnum=Some(0), plusBool=Some(false)
    0x0000_0002 ^ 3, // Some(Length::AUTO)
    0,
    0,
    0,
  ];
  const t = new TableReader({
    key: Key64.of('test.opt'),
    slot: 0,
    words: 7,
    rowCount: 3,
    rows: words([...rowNone, ...rowSome, ...rowEdge]),
  });

  // Row 0: all optionals absent (including plusBool raw=3 -> null, TagXor low bits 00 -> null)
  assert.equal(t.optU32(0, fReq), 0x2a);
  assert.equal(t.isPresent(0, fReq), true);
  assert.equal(t.optU32(0, fZero), null);
  assert.equal(t.optU32(0, fBit0U), null);
  assert.equal(t.optI32(0, fBit0I), null);
  assert.equal(t.optEnum(0, fPlusEnum), null);
  assert.equal(t.optBool(0, fPlusBool), null, 'Option<bool> raw 3 normalizes to null');
  assert.equal(t.optLength(0, fTagLen), null);
  assert.equal(t.optF32(0, fWideF), null);
  assert.equal(t.optI32(0, fWideI), null);
  assert.equal(t.optColor(0, fWideC), null);

  // Row 1: all optionals present
  assert.equal(t.optU32(1, fZero), 7);
  assert.equal(t.optU32(1, fBit0U), 255);
  assert.equal(t.optI32(1, fBit0I), -1);
  assert.equal(t.optEnum(1, fPlusEnum), 2);
  assert.equal(t.optBool(1, fPlusBool), true);
  assert.deepEqual(t.optLength(1, fTagLen), { unit: LengthUnit.Percent, value: 50 });
  assert.equal(t.optF32(1, fWideF), 1.75);
  assert.equal(t.optI32(1, fWideI), -42);
  assert.equal(t.optColor(1, fWideC), 0x8040_2010);

  // Row 2: edge values (Some(0), Some(i16::MIN), Some(false), Some(Length::AUTO))
  assert.equal(t.optU32(2, fBit0U), 0);
  assert.equal(t.optI32(2, fBit0I), -32768);
  assert.equal(t.optEnum(2, fPlusEnum), 0);
  assert.equal(t.optBool(2, fPlusBool), false);
  assert.deepEqual(t.optLength(2, fTagLen), { unit: LengthUnit.Auto, value: 0 });
});

test('64-bit U64 / I64 / F64 fields stay 53-bit-safe', () => {
  const fU64 = field(0, FieldKind.U64);
  const fI64 = field(2, FieldKind.I64);
  const fF64 = field(4, FieldKind.F64);
  const pi = f64Halves(Math.PI);
  const negZero = f64Halves(-0);
  const t = new TableReader({
    key: Key64.of('test.wide64'),
    slot: 0,
    words: 6,
    rowCount: 2,
    rows: words([
      0x0000_0001, 0x0020_0000, // 2^53 + 1
      0x0000_0000, 0x8000_0000, // i64::MIN (-2^63)
      pi[0], pi[1],
      0xffff_ffff, 0xffff_ffff, // u64::MAX
      0xffff_ffff, 0x7fff_ffff, // i64::MAX (2^63 - 1)
      negZero[0], negZero[1],
    ]),
  });

  assert.equal(t.u64(0, fU64).toBigInt(), 2n ** 53n + 1n);
  assert.equal(t.i64(0, fI64), -(2n ** 63n));
  assert.equal(t.i64Bits(0, fI64).toSignedSafeInteger(), null);
  assert.equal(t.f64(0, fF64), Math.PI);
  assert.equal(t.u64(1, fU64).toBigInt(), 2n ** 64n - 1n);
  assert.equal(t.i64(1, fI64), 2n ** 63n - 1n);
  assert.ok(Object.is(t.f64(1, fF64), -0));
  assert.equal(f64FromBits(pi[0], pi[1]), Math.PI);

  // Kind mismatch throws RangeError
  assert.throws(() => t.u64(0, fI64), RangeError);
  assert.throws(() => t.i64(0, fU64), RangeError);
  assert.throws(() => t.f64(0, fU64), RangeError);
  assert.throws(() => t.u64(0, field(5, FieldKind.U64)), RangeError);
});

test('inherited Typography resolution and per-table EventSet on SceneReader', () => {
  const SCHEMA = new Key64(0x1111_2222, 0x3333_4444);
  const TYPO_SLOT = 2;
  const INTER_SLOT = 4;
  const CAP = 5; // 0 = null, 1 = root, 2 = child, 3 = grandchild, 4 = cyclic test
  const nodeId = (idx: number, gen = 1): number => ((gen << 24) | idx) >>> 0;

  const rows = new DataView(new ArrayBuffer(CAP * NVE_NODE_ROW_LAYOUT.size));
  const setNode = (idx: number, mask: number, parent: number, gen = 1): void => {
    const b = idx * NVE_NODE_ROW_LAYOUT.size;
    rows.setUint32(b, mask, true);
    rows.setUint32(b + NVE_NODE_ROW_LAYOUT.parent, parent, true);
    rows.setUint8(b + NVE_NODE_ROW_LAYOUT.generation, gen);
    rows.setUint8(b + NVE_NODE_ROW_LAYOUT.primarySlot, NVE_PRIMARY_SLOT_NONE);
    rows.setUint8(b + NVE_NODE_ROW_LAYOUT.flags, NVE_NODE_ROW_FLAG_LIVE);
  };
  setNode(1, (1 << TYPO_SLOT) | (1 << INTER_SLOT), 0);
  setNode(2, 1 << TYPO_SLOT, nodeId(1));
  setNode(3, 0, nodeId(2));
  setNode(4, 0, nodeId(4)); // hostile self-cycle

  // Typography row 0 (node 1): explicit color (bit 0) = 0xFF112233
  const r0 = [...TypographyTable.defaults];
  r0[0] = 1 << TypographyTable.fields.color.explicitBit;
  r0[TypographyTable.fields.color.word] = 0xff11_2233;
  // Typography row 1 (node 2): explicit fontSize (bit 1) = 24
  const r1 = [...TypographyTable.defaults];
  r1[0] = 1 << TypographyTable.fields.fontSize.explicitBit;
  r1[TypographyTable.fields.fontSize.word] = f32Bits(24);
  const typoTable = TableReader.sparse({
    key: TypographyTable.key,
    slot: TYPO_SLOT,
    words: TypographyTable.words,
    rows: words([...r0, ...r1]),
    owners: [nodeId(1), nodeId(2)],
  });

  // Interaction row 0 (node 1): subscribed to PRESS | KEY_DOWN
  const interRow = [...InteractionTable.defaults];
  interRow[InteractionTable.eventsWord] = (1 << NVE_INTERACTION_EVENT.PRESS) | (1 << NVE_INTERACTION_EVENT.KEY_DOWN);
  const interTable = TableReader.sparse({
    key: InteractionTable.key,
    slot: INTER_SLOT,
    words: InteractionTable.words,
    rows: words(interRow),
    owners: [nodeId(1)],
  });

  const empty = new DataView(new ArrayBuffer(0));
  const scene = new SceneReader({
    expectedSchema: SCHEMA,
    schemaHash: SCHEMA,
    frameId: new Key64(1, 0),
    nodeCapacity: CAP,
    rows,
    rects: new DataView(new ArrayBuffer(CAP * NVE_LAYOUT_RECT_LAYOUT.size)),
    textSlab: new Uint8Array(0),
    childrenSlab: empty,
    dirtyIndices: empty,
    removedIds: empty,
    tables: [typoTable, interTable],
  });

  // Grandchild (node 3) inherits fontSize from node 2 (row 1), color from node 1 (row 0), letterSpacing -> -1 (default)
  const fsRow = scene.inheritedRow(typoTable, 3, TypographyTable.fields.fontSize);
  assert.equal(fsRow, 1);
  assert.equal(typoTable.f32(fsRow, TypographyTable.fields.fontSize), 24);

  const colorRow = scene.inheritedRow(typoTable, 3, TypographyTable.fields.color);
  assert.equal(colorRow, 0);
  assert.equal(typoTable.color(colorRow, TypographyTable.fields.color), 0xff11_2233);

  assert.equal(scene.inheritedRow(typoTable, 3, TypographyTable.fields.letterSpacing), -1);
  // Non-inheriting field on an inherited table (`explicitBit` set, `inherits = false`) never walks to parent.
  const noInherit = new FieldSpec(1, FieldKind.Color, 0, 32, OptEncoding.Required, NO_VAL_WORD, 0, false);
  assert.equal(scene.inheritedRow(typoTable, 1, noInherit), 0);
  assert.equal(scene.inheritedRow(typoTable, 2, noInherit), -1);
  assert.equal(scene.inheritedRow(typoTable, 3, noInherit), -1);
  // Hostile self-cycle terminates cleanly.
  assert.equal(scene.inheritedRow(typoTable, 4, TypographyTable.fields.color), -1);
  assert.equal(scene.inheritedRow(typoTable, 0, TypographyTable.fields.color), -1);

  // Per-table EventSet checks
  assert.equal(scene.hasEvent(interTable, 1, InteractionTable.eventsWord, NVE_INTERACTION_EVENT.PRESS), true);
  assert.equal(scene.hasEvent(interTable, 1, InteractionTable.eventsWord, NVE_INTERACTION_EVENT.KEY_DOWN), true);
  assert.equal(scene.hasEvent(interTable, 1, InteractionTable.eventsWord, NVE_INTERACTION_EVENT.WHEEL), false);
  assert.equal(scene.hasEvent(interTable, 1, InteractionTable.eventsWord, 32), false);
  assert.equal(scene.hasEvent(interTable, 2, InteractionTable.eventsWord, NVE_INTERACTION_EVENT.PRESS), false);
  assert.equal(scene.eventMask(typoTable, 1, TypographyTable.eventsWord), 0);
  assert.equal(LayoutTable.eventsWord, 0);
  assert.equal(ScrollTable.eventsWord, 0);
  assert.equal(NVE_LAYOUT_EVENT.LAYOUT, 0);
  assert.equal(NVE_SCROLL_EVENT.SCROLL, 0);
});

test('FieldSpec 8-arg validation hardening', () => {
  assert.throws(() => new FieldSpec(0, FieldKind.F32, 0, 1, OptEncoding.WideBit, NO_VAL_WORD), RangeError, 'WideBit needs valWord');
  assert.throws(() => new FieldSpec(0, FieldKind.F32, 0, 2, OptEncoding.WideBit, 1), RangeError, 'WideBit needs bits=1');
  assert.throws(() => new FieldSpec(2, FieldKind.F32, 0, 1, OptEncoding.WideBit, 2), RangeError, 'WideBit valWord != word');
  assert.throws(() => new FieldSpec(0, FieldKind.F32, 0, 32, OptEncoding.Required, 1), RangeError, 'Required forbids valWord');
  assert.throws(() => new FieldSpec(0, FieldKind.U32, 0, 1, OptEncoding.Bit0Present), RangeError, 'Bit0Present needs bits>=2');
  assert.throws(() => new FieldSpec(0, FieldKind.U32, 0, 32, OptEncoding.TagXor), RangeError, 'TagXor only Length/GridLine');
  assert.throws(() => new FieldSpec(0, FieldKind.GridLine, 0, 32, OptEncoding.TagXor), RangeError, 'TagXor GridLine must be 16b');
  assert.throws(() => new FieldSpec(1, FieldKind.U32, 0, 32, OptEncoding.Required, NO_VAL_WORD, NO_VAL_WORD, true), RangeError, 'inherits needs explicitBit');
  assert.throws(() => new FieldSpec(0, FieldKind.U32, 0, 32, OptEncoding.Required, NO_VAL_WORD, 0, true), RangeError, 'inherited field cannot sit in word 0');
  assert.throws(() => new FieldSpec(1, FieldKind.F32, 0, 1, OptEncoding.WideBit, 0, 0, true), RangeError, 'inherited WideBit valWord cannot sit in word 0');
  assert.throws(() => new FieldSpec(0, FieldKind.U64, 1, 32), RangeError, 'two-word shift must be 0');
  assert.throws(() => new FieldSpec(0, FieldKind.I64, 0, 16), RangeError, 'two-word bits must be 32');
  assert.throws(() => new FieldSpec(31, FieldKind.F64, 0, 32), RangeError, 'two-word at 31 overflows 32');
  assert.throws(() => new FieldSpec(0, FieldKind.U32, 0, 32, 6 as OptEncoding), RangeError, 'unknown OptEncoding');
  assert.ok(LayoutTable.fields.aspectRatio.fits(26) && !LayoutTable.fields.aspectRatio.fits(25), 'WideBit fits checks valWord');
});
