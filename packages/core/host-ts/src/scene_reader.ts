/**
 * Validated zero-copy reader over one published `NveSceneView` — the one scene ABI every host
 * reads, published by `libnve_runtime` straight from `NodeStore` slabs (32-byte `NveNodeRow`s,
 * 16-byte `NveLayoutRect`s, text / children slabs, change sets, `NveTableView`s, scroll offsets).
 *
 * Mirrors the C++ `nve::host::SceneReader` and the Dart `SceneReader`: construction validates the
 * schema hash, every slab against its count, the text slab's UTF-8, every live node's child range,
 * every dirty / moved index, every table slot, and the scroll-offset order once; later reads are
 * O(1) bounds checks (out of range throws `RangeError`). `fromLinearMemory` reads a view out of
 * wasm32 linear memory through the generated `NVE_SCENE_LAYOUT_WASM32` offsets, so this static
 * runtime never hard-codes a layout.
 */
import {
  NVE_LAYOUT_RECT_LAYOUT,
  NVE_NODE_ROW_FLAG_LIVE,
  NVE_NODE_ROW_LAYOUT,
  NVE_ROW_INDEX_PAGE_LEN,
  NVE_SCROLL_OFFSET_LAYOUT,
} from './generated/nve_abi.generated.ts';
import { Key64 } from './key64.ts';
import { MAX_TABLE_WORDS, NODE_INDEX_MASK, TableReader, isIntIn } from './table_reader.ts';
import type { FieldSpec } from './table_reader.ts';
import { UTF8_DECODER, isValidUtf8 } from './utf8.ts';

/** Highest table slot a 128-bit `table_set` can address. */
export const MAX_TABLE_SLOTS = 128;

// Record layouts come from the generated ABI (`nve codegen abi`), never hard-coded here.
const ROW = NVE_NODE_ROW_LAYOUT;
const RECT = NVE_LAYOUT_RECT_LAYOUT;
const SCROLL = NVE_SCROLL_OFFSET_LAYOUT;

/** Byte offsets of `NveTableView` fields inside wasm32 linear memory (pointers are u32). */
export interface TableViewLayout {
  readonly stride: number;
  readonly keyLo: number;
  readonly keyHi: number;
  readonly slot: number;
  readonly words: number;
  readonly rowCount: number;
  readonly dirtyCount: number;
  readonly rowIndexDirLen: number;
  readonly rowIndexPageCount: number;
  readonly rows: number;
  readonly owners: number;
  readonly dirtyRows: number;
  readonly rowIndexDir: number;
  readonly rowIndexPages: number;
}

/** Byte offsets of `NveSceneView` fields inside wasm32 linear memory. */
export interface SceneLayout {
  readonly schemaHash: number;
  readonly frameId: number;
  readonly nodeCapacity: number;
  readonly dirtyCount: number;
  readonly removedCount: number;
  readonly textSlabLen: number;
  readonly childrenSlabLen: number;
  readonly tableCount: number;
  readonly scrollCount: number;
  readonly sceneEpoch: number;
  readonly dirtyResourceCount: number;
  readonly movedCount: number;
  readonly rows: number;
  readonly rects: number;
  readonly textSlab: number;
  readonly childrenSlab: number;
  readonly dirtyIndices: number;
  readonly removedIds: number;
  readonly tables: number;
  readonly scrollOffsets: number;
  readonly dirtyResources: number;
  readonly movedIndices: number;
  readonly tableView: TableViewLayout;
}

export type SceneErrorKind = 'schemaMismatch' | 'missingSlab' | 'invalidUtf8' | 'invalidTable';

export class SceneError extends Error {
  readonly kind: SceneErrorKind;

  constructor(kind: SceneErrorKind, detail: string) {
    super(`${kind}: ${detail}`);
    this.name = 'SceneError';
    this.kind = kind;
  }
}

/** Borrowed slabs of one frame; every `DataView` is little-endian and sized to at least its count. */
export interface SceneSlabs {
  readonly expectedSchema: Key64;
  readonly schemaHash: Key64;
  readonly frameId: Key64;
  readonly nodeCapacity: number;
  readonly rows: DataView;
  readonly rects: DataView;
  readonly textSlab: Uint8Array;
  readonly childrenSlab: DataView;
  readonly dirtyIndices: DataView;
  readonly removedIds: DataView;
  readonly tables?: readonly TableReader[];
  readonly scrollOffsets?: DataView;
  readonly sceneEpoch?: number;
  readonly dirtyResources?: DataView;
  readonly movedIndices?: DataView;
}

const EMPTY = new DataView(new ArrayBuffer(0));

export class SceneReader {
  /** Exact 64-bit frame counter. */
  readonly frameId: Key64;
  /** Slot capacity of rows / rects (slot 0 is the reserved null node). */
  readonly nodeCapacity: number;
  /** Bumped on every runtime scene reset: ids and caches of an earlier epoch are stale. */
  readonly sceneEpoch: number;
  readonly dirtyCount: number;
  readonly movedCount: number;
  readonly removedCount: number;
  readonly dirtyResourceCount: number;
  readonly scrollCount: number;
  readonly tables: readonly TableReader[];
  readonly textSlab: Uint8Array;
  private readonly rows: DataView;
  private readonly rects: DataView;
  private readonly children: DataView;
  private readonly childLen: number;
  private readonly dirty: DataView;
  private readonly moved: DataView;
  private readonly removed: DataView;
  private readonly resources: DataView;
  private readonly scroll: DataView;
  private readonly bySlot: (TableReader | undefined)[];

  /** Wraps per-slab views; throws `SceneError` when the schema, a slab, a range, or a table is invalid. */
  constructor(s: SceneSlabs) {
    if (!s.schemaHash.equals(s.expectedSchema)) {
      throw new SceneError('schemaMismatch', `expected ${s.expectedSchema}, got ${s.schemaHash}`);
    }
    const cap = s.nodeCapacity;
    requireRecords(s.rows, cap, ROW.size, 'rows');
    requireRecords(s.rects, cap, RECT.size, 'rects');
    if (!isValidUtf8(s.textSlab)) throw new SceneError('invalidUtf8', 'textSlab');
    this.frameId = s.frameId;
    this.nodeCapacity = cap;
    this.sceneEpoch = s.sceneEpoch ?? 0;
    this.rows = s.rows;
    this.rects = s.rects;
    this.textSlab = s.textSlab;
    this.children = s.childrenSlab;
    this.dirty = s.dirtyIndices;
    this.moved = s.movedIndices ?? EMPTY;
    this.removed = s.removedIds;
    this.resources = s.dirtyResources ?? EMPTY;
    this.scroll = s.scrollOffsets ?? EMPTY;
    this.dirtyCount = recordCount(this.dirty, 4, 'dirtyIndices');
    this.movedCount = recordCount(this.moved, 4, 'movedIndices');
    this.removedCount = recordCount(this.removed, 4, 'removedIds');
    this.dirtyResourceCount = recordCount(this.resources, 4, 'dirtyResources');
    this.scrollCount = recordCount(this.scroll, SCROLL.size, 'scrollOffsets');
    // Child ranges are checked when read (`childCount`), never by an O(capacity) walk here.
    this.childLen = recordCount(this.children, 4, 'childrenSlab');
    for (const [what, view, count] of [
      ['dirty', this.dirty, this.dirtyCount],
      ['moved', this.moved, this.movedCount],
    ] as const) {
      for (let i = 0; i < count; i++) {
        const index = view.getUint32(i * 4, true);
        if (index >= cap) throw new SceneError('missingSlab', `${what} index ${index} >= capacity ${cap}`);
      }
    }
    this.tables = s.tables ?? [];
    this.bySlot = new Array<TableReader | undefined>(MAX_TABLE_SLOTS);
    for (const t of this.tables) {
      if (!isIntIn(t.slot, 0, MAX_TABLE_SLOTS - 1) || this.bySlot[t.slot] !== undefined) {
        throw new SceneError('invalidTable', `table ${t.key} slot ${t.slot} invalid or duplicated`);
      }
      this.bySlot[t.slot] = t;
    }
    ascending(this.scroll, this.scrollCount, SCROLL.size, 'scroll offset node ids');
    ascending(this.resources, this.dirtyResourceCount, 4, 'dirty resource ids');
  }

  /** Reads the `NveSceneView` at `viewPtr` inside wasm32 linear memory (zero copy). */
  static fromLinearMemory(memory: ArrayBuffer, viewPtr: number, layout: SceneLayout, expectedSchema: Key64): SceneReader {
    const mem = new DataView(memory);
    const u32 = (at: number): number => {
      if (!isIntIn(at, 0, mem.byteLength - 4)) {
        throw new SceneError('missingSlab', `read at ${at} outside memory (${mem.byteLength})`);
      }
      return mem.getUint32(at, true);
    };
    // `count` is a u32 and `size <= 32`: the product is an exact double, and any value past the
    // memory end fails the bounds check.
    const slab = (ptr: number, count: number, size: number, what: string): DataView => {
      const bytes = count * size;
      if (bytes === 0) return EMPTY;
      if (ptr === 0 || ptr + bytes > mem.byteLength) throw new SceneError('missingSlab', `${what} [${ptr}, +${bytes}) outside memory`);
      return new DataView(memory, ptr, bytes);
    };
    const h = viewPtr;
    const key = (at: number): Key64 => new Key64(u32(h + at), u32(h + at + 4));
    const schema = key(layout.schemaHash);
    if (!schema.equals(expectedSchema)) throw new SceneError('schemaMismatch', `expected ${expectedSchema}, got ${schema}`);
    const field = (at: number): number => u32(h + at);
    const tl = layout.tableView;
    if (!isIntIn(tl.stride, 1, 0xffff_ffff)) throw new SceneError('missingSlab', `table view stride ${tl.stride}`);
    const tableCount = field(layout.tableCount);
    const tablesBase = field(layout.tables);
    if (tableCount > 0 && (tablesBase === 0 || tablesBase + tableCount * tl.stride > mem.byteLength)) {
      throw new SceneError('invalidTable', `tables [${tablesBase}, x${tableCount}) outside memory`);
    }
    const tables: TableReader[] = [];
    for (let i = 0; i < tableCount; i++) {
      const t = tablesBase + i * tl.stride;
      const words = u32(t + tl.words);
      if (!isIntIn(words, 1, MAX_TABLE_WORDS)) throw new SceneError('invalidTable', `table ${i}: words ${words}`);
      try {
        const rowCount = u32(t + tl.rowCount);
        const tableDirty = u32(t + tl.dirtyCount);
        const ownersPtr = u32(t + tl.owners);
        tables.push(
          new TableReader({
            key: new Key64(u32(t + tl.keyLo), u32(t + tl.keyHi)),
            slot: u32(t + tl.slot),
            words,
            rowCount,
            rows: slab(u32(t + tl.rows), rowCount * words, 4, `table ${i} rows`),
            owners: ownersPtr === 0 ? null : slab(ownersPtr, rowCount, 4, `table ${i} owners`),
            dirtyRows: slab(u32(t + tl.dirtyRows), tableDirty, 4, `table ${i} dirtyRows`),
            dirtyCount: tableDirty,
            rowIndexDir: slab(u32(t + tl.rowIndexDir), u32(t + tl.rowIndexDirLen), 4, `table ${i} rowIndexDir`),
            rowIndexPages: slab(u32(t + tl.rowIndexPages), u32(t + tl.rowIndexPageCount) * NVE_ROW_INDEX_PAGE_LEN, 4, `table ${i} rowIndexPages`),
          }),
        );
      } catch (e) {
        if (e instanceof SceneError) throw e;
        throw new SceneError('invalidTable', `table ${i}: ${(e as Error).message}`);
      }
    }
    const cap = field(layout.nodeCapacity);
    const text = slab(field(layout.textSlab), field(layout.textSlabLen), 1, 'textSlab');
    return new SceneReader({
      expectedSchema,
      schemaHash: schema,
      frameId: key(layout.frameId),
      nodeCapacity: cap,
      rows: slab(field(layout.rows), cap, ROW.size, 'rows'),
      rects: slab(field(layout.rects), cap, RECT.size, 'rects'),
      textSlab: new Uint8Array(text.buffer, text.byteOffset, text.byteLength),
      childrenSlab: slab(field(layout.childrenSlab), field(layout.childrenSlabLen), 4, 'childrenSlab'),
      dirtyIndices: slab(field(layout.dirtyIndices), field(layout.dirtyCount), 4, 'dirtyIndices'),
      removedIds: slab(field(layout.removedIds), field(layout.removedCount), 4, 'removedIds'),
      movedIndices: slab(field(layout.movedIndices), field(layout.movedCount), 4, 'movedIndices'),
      dirtyResources: slab(field(layout.dirtyResources), field(layout.dirtyResourceCount), 4, 'dirtyResources'),
      scrollOffsets: slab(field(layout.scrollOffsets), field(layout.scrollCount), SCROLL.size, 'scrollOffsets'),
      sceneEpoch: field(layout.sceneEpoch),
      tables,
    });
  }

  // ---- node rows -----------------------------------------------------------------------------

  isLive(index: number): boolean {
    return isIntIn(index, 1, this.nodeCapacity - 1) && (this.rows.getUint8(index * ROW.size + ROW.flags) & NVE_NODE_ROW_FLAG_LIVE) !== 0;
  }

  generation(index: number): number {
    return this.rows.getUint8(this.rowAt(index) + ROW.generation);
  }

  primarySlot(index: number): number {
    return this.rows.getUint8(this.rowAt(index) + ROW.primarySlot);
  }

  effects(index: number): number {
    return this.rows.getUint8(this.rowAt(index) + ROW.effects);
  }

  flags(index: number): number {
    return this.rows.getUint8(this.rowAt(index) + ROW.flags);
  }

  /** Raw parent `NodeId` (`0` = root / detached). */
  parentId(index: number): number {
    return this.rows.getUint32(this.rowAt(index) + ROW.parent, true);
  }

  /** `true` when table `slot` is attached to node `index`. */
  hasTable(index: number, slot: number): boolean {
    if (!isIntIn(slot, 0, MAX_TABLE_SLOTS - 1) || !isIntIn(index, 1, this.nodeCapacity - 1)) return false;
    return ((this.rows.getUint32(index * ROW.size + (slot >>> 5) * 4, true) >>> (slot & 31)) & 1) !== 0;
  }

  /** Raw `NodeId` of live slot `index`. */
  nodeId(index: number): number {
    return ((this.generation(index) << 24) | index) >>> 0;
  }

  /** Slot index of raw `nodeId`, or `-1` when it is null, stale, or dead. */
  indexOf(nodeId: number): number {
    const index = nodeId & NODE_INDEX_MASK;
    return this.isLive(index) && this.generation(index) === (nodeId >>> 24) ? index : -1;
  }

  private rowAt(index: number): number {
    if (!isIntIn(index, 0, this.nodeCapacity - 1)) throw new RangeError(`node ${index} outside capacity ${this.nodeCapacity}`);
    return index * ROW.size;
  }

  // ---- layout rects --------------------------------------------------------------------------

  /** Absolute pre-scroll rect `[x, y, width, height]` of node `index`. */
  rect(index: number): [number, number, number, number] {
    const at = (this.rowAt(index) / ROW.size) * RECT.size;
    const r = this.rects;
    return [r.getFloat32(at + RECT.x, true), r.getFloat32(at + RECT.y, true), r.getFloat32(at + RECT.width, true), r.getFloat32(at + RECT.height, true)];
  }

  // ---- scroll offsets ------------------------------------------------------------------------

  /** Visual child shift `[x, y]` of live node `index`; `[0, 0]` when not scrolled. O(log scrollCount). */
  scrollShift(index: number): [number, number] {
    if (!this.isLive(index)) return [0, 0];
    const id = this.nodeId(index);
    let lo = 0;
    let hi = this.scrollCount;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      const at = mid * SCROLL.size;
      const node = this.scroll.getUint32(at + SCROLL.nodeId, true);
      if (node === id) return [this.scroll.getFloat32(at + SCROLL.x, true), this.scroll.getFloat32(at + SCROLL.y, true)];
      if (node < id) lo = mid + 1;
      else hi = mid;
    }
    return [0, 0];
  }

  // ---- children ------------------------------------------------------------------------------

  /** Child count of node `index` (`0` when dead); a child range outside the slab throws `SceneError`. */
  childCount(index: number): number {
    if (!this.isLive(index)) return 0;
    const off = this.rows.getUint32(index * ROW.size + ROW.childOff, true);
    const len = this.rows.getUint32(index * ROW.size + ROW.childLen, true);
    if (off > this.childLen || len > this.childLen - off) {
      throw new SceneError('missingSlab', `node ${index} children [${off}, +${len}) outside slab (${this.childLen})`);
    }
    return len;
  }

  /** Raw child `NodeId` `i` of node `index` in paint order. */
  childId(index: number, i: number): number {
    const count = this.childCount(index);
    if (!isIntIn(i, 0, count - 1)) throw new RangeError(`child ${i} outside ${count}`);
    return this.children.getUint32((this.rows.getUint32(index * ROW.size + ROW.childOff, true) + i) * 4, true);
  }

  // ---- change sets ---------------------------------------------------------------------------

  dirtyIndex(i: number): number {
    return u32At(this.dirty, i, this.dirtyCount, 'dirty');
  }

  movedIndex(i: number): number {
    return u32At(this.moved, i, this.movedCount, 'moved');
  }

  removedId(i: number): number {
    return u32At(this.removed, i, this.removedCount, 'removed');
  }

  dirtyResource(i: number): number {
    return u32At(this.resources, i, this.dirtyResourceCount, 'dirty resource');
  }

  // ---- text ----------------------------------------------------------------------------------

  /** View of `textSlab[offset .. offset + len)`; throws when out of range or not on UTF-8 boundaries. */
  textBytes(offset: number, len: number): Uint8Array {
    const size = this.textSlab.length;
    if (!(isIntIn(offset, 0, size) && isIntIn(len, 0, size - offset)) || !this.isBoundary(offset) || !this.isBoundary(offset + len)) {
      throw new RangeError(`text [${offset}, +${len}) outside textSlab (${size}) or splits a code point`);
    }
    return this.textSlab.subarray(offset, offset + len);
  }

  /** Text at `[offset, offset + len)`; a leading U+FEFF is kept (never stripped). */
  text(offset: number, len: number): string {
    return UTF8_DECODER.decode(this.textBytes(offset, len));
  }

  private isBoundary(at: number): boolean {
    return at === this.textSlab.length || (this.textSlab[at]! & 0xc0) !== 0x80;
  }

  // ---- tables --------------------------------------------------------------------------------

  /** Published table with `key`, or `null` (O(tables)). */
  findTable(key: Key64): TableReader | null {
    return this.tables.find((t) => t.key.equals(key)) ?? null;
  }

  /** Published table bound to `slot`, or `null`. */
  tableAtSlot(slot: number): TableReader | null {
    return isIntIn(slot, 0, MAX_TABLE_SLOTS - 1) ? (this.bySlot[slot] ?? null) : null;
  }

  /**
   * Row of live node `index` in `table`, or `-1` when the table is absent or not attached to it;
   * O(1). A sparse row must be owned by the node's current `NodeId` (generation checked), so a
   * freed slot with stale `table_set` bits never resolves.
   */
  rowOf(table: TableReader | null, index: number): number {
    if (table === null || !this.isLive(index) || !this.hasTable(index, table.slot)) return -1;
    return (table.isSparse ? table.rowOf(this.nodeId(index)) : table.rowOfNode(index)) ?? -1;
  }

  /** Text of `str` field `f` in `row` of `table`. */
  str(table: TableReader, row: number, f: FieldSpec): string {
    const { offset, len } = table.str(row, f);
    return this.text(offset, len);
  }
}

/** `count` records of `size` bytes fit `view` (division: no product to overflow or NaN). */
function requireRecords(view: DataView, count: number, size: number, what: string): void {
  if (!(isIntIn(count, 0, 0xffff_ffff) && Math.floor(view.byteLength / size) >= count)) {
    throw new SceneError('missingSlab', `${what} holds ${view.byteLength} bytes, need ${count} x ${size}`);
  }
}

/** Whole records of `size` bytes in `view`; a ragged tail is a malformed slab. */
function recordCount(view: DataView, size: number, what: string): number {
  if (view.byteLength % size !== 0) throw new SceneError('missingSlab', `${what} holds ${view.byteLength} bytes, not a multiple of ${size}`);
  return view.byteLength / size;
}

/** The leading `u32` of each of `count` `size`-byte records is strictly ascending. */
function ascending(view: DataView, count: number, size: number, what: string): void {
  let previous = -1;
  for (let i = 0; i < count; i++) {
    const v = view.getUint32(i * size, true);
    if (v <= previous) throw new SceneError('missingSlab', `${what} not strictly ascending at ${v}`);
    previous = v;
  }
}

function u32At(view: DataView, i: number, count: number, what: string): number {
  if (!isIntIn(i, 0, count - 1)) throw new RangeError(`${what} ${i} outside ${count}`);
  return view.getUint32(i * 4, true);
}
