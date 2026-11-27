/**
 * Validated zero-copy reader over one published `NveSceneView` frame.
 *
 * Node records are read by byte offset: the generated ABI supplies the node stride, field
 * offsets, and (for wasm32 linear memory) the `SceneLayout`, so this static runtime never
 * hard-codes a layout. Construction validates the schema hash, every slab against its count, and
 * the text slab's UTF-8 once; later reads are O(1) bounds checks (out of range throws `RangeError`).
 */
import { NVE_ROW_INDEX_PAGE_LEN } from './generated/nve_abi.generated.ts';
import { Key64 } from './key64.ts';
import { MAX_TABLE_WORDS, TableReader, isIntIn } from './table_reader.ts';
import { UTF8_DECODER, isValidUtf8 } from './utf8.ts';

/** Byte offsets of `NveTableView` fields inside wasm32 linear memory (pointers are u32). */
export interface TableViewLayout {
  readonly stride: number;
  readonly keyLo: number;
  readonly keyHi: number;
  readonly slot: number;
  readonly words: number;
  readonly rowCount: number;
  readonly dirtyCount: number;
  readonly rows: number;
  readonly owners: number;
  readonly dirtyRows: number;
  readonly rowIndexDirLen: number;
  readonly rowIndexPageCount: number;
  readonly rowIndexDir: number;
  readonly rowIndexPages: number;
}

/** Byte offsets of `NveSceneView` fields inside wasm32 linear memory, plus the node stride. */
export interface SceneLayout {
  readonly schemaHash: number;
  readonly frameId: number;
  readonly nodeCount: number;
  readonly dirtyCount: number;
  readonly removedCount: number;
  readonly textSlabLen: number;
  readonly childrenSlabLen: number;
  readonly tableCount: number;
  readonly nodes: number;
  readonly textSlab: number;
  readonly childrenSlab: number;
  readonly dirtyIndices: number;
  readonly removedIds: number;
  readonly tables: number;
  readonly nodeStride: number;
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

export interface SceneSlabs {
  readonly expectedSchema: Key64;
  readonly schemaHash: Key64;
  readonly frameId: Key64;
  readonly nodes: DataView;
  readonly nodeCount: number;
  readonly nodeStride: number;
  readonly textSlab: Uint8Array;
  readonly childrenSlab: DataView;
  readonly childrenSlabLen: number;
  readonly dirtyIndices: DataView;
  readonly dirtyCount: number;
  readonly removedIds: DataView;
  readonly removedCount: number;
  readonly tables?: readonly TableReader[];
}

const EMPTY = new DataView(new ArrayBuffer(0));

export class SceneReader {
  /** Exact 64-bit frame counter. */
  readonly frameId: Key64;
  readonly nodeCount: number;
  readonly nodeStride: number;
  readonly childrenSlabLen: number;
  readonly dirtyCount: number;
  readonly removedCount: number;
  readonly tables: readonly TableReader[];
  readonly textSlab: Uint8Array;
  private readonly nodes: DataView;
  private readonly children: DataView;
  private readonly dirty: DataView;
  private readonly removed: DataView;

  /** Wraps per-slab views; throws `SceneError` when the schema, a slab, or the text UTF-8 is invalid. */
  constructor(s: SceneSlabs) {
    if (!s.schemaHash.equals(s.expectedSchema)) {
      throw new SceneError('schemaMismatch', `expected ${s.expectedSchema}, got ${s.schemaHash}`);
    }
    if (!isIntIn(s.nodeStride, 1, 0xffff_ffff)) throw new SceneError('missingSlab', `nodeStride ${s.nodeStride}`);
    requireRecords(s.nodes, s.nodeCount, s.nodeStride, 'nodes');
    requireRecords(s.childrenSlab, s.childrenSlabLen, 4, 'childrenSlab');
    requireRecords(s.dirtyIndices, s.dirtyCount, 4, 'dirtyIndices');
    requireRecords(s.removedIds, s.removedCount, 4, 'removedIds');
    if (!isValidUtf8(s.textSlab)) throw new SceneError('invalidUtf8', 'textSlab');
    this.frameId = s.frameId;
    this.nodeCount = s.nodeCount;
    this.nodeStride = s.nodeStride;
    this.childrenSlabLen = s.childrenSlabLen;
    this.dirtyCount = s.dirtyCount;
    this.removedCount = s.removedCount;
    this.tables = s.tables ?? [];
    this.textSlab = s.textSlab;
    this.nodes = s.nodes;
    this.children = s.childrenSlab;
    this.dirty = s.dirtyIndices;
    this.removed = s.removedIds;
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
    const slab = (ptr: number, bytes: number, what: string): DataView => {
      if (bytes === 0) return EMPTY;
      if (ptr === 0 || ptr + bytes > mem.byteLength) throw new SceneError('missingSlab', `${what} [${ptr}, +${bytes}) outside memory`);
      return new DataView(memory, ptr, bytes);
    };
    const h = viewPtr;
    const key = (at: number): Key64 => new Key64(u32(h + at), u32(h + at + 4));
    const schema = key(layout.schemaHash);
    if (!schema.equals(expectedSchema)) throw new SceneError('schemaMismatch', `expected ${expectedSchema}, got ${schema}`);
    const nodeCount = u32(h + layout.nodeCount);
    const textLen = u32(h + layout.textSlabLen);
    const childrenLen = u32(h + layout.childrenSlabLen);
    const dirtyCount = u32(h + layout.dirtyCount);
    const removedCount = u32(h + layout.removedCount);
    const tableCount = u32(h + layout.tableCount);
    const tl = layout.tableView;
    if (!isIntIn(layout.nodeStride, 1, 0xffff_ffff) || !isIntIn(tl.stride, 1, 0xffff_ffff)) {
      throw new SceneError('missingSlab', `layout strides must be >= 1 (node ${layout.nodeStride}, table ${tl.stride})`);
    }
    const tablesBase = u32(h + layout.tables);
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
            rows: slab(u32(t + tl.rows), rowCount * words * 4, `table ${i} rows`),
            owners: ownersPtr === 0 ? null : slab(ownersPtr, rowCount * 4, `table ${i} owners`),
            dirtyRows: slab(u32(t + tl.dirtyRows), tableDirty * 4, `table ${i} dirtyRows`),
            dirtyCount: tableDirty,
            rowIndexDir: slab(u32(t + tl.rowIndexDir), u32(t + tl.rowIndexDirLen) * 4, `table ${i} rowIndexDir`),
            rowIndexPages: slab(
              u32(t + tl.rowIndexPages),
              u32(t + tl.rowIndexPageCount) * NVE_ROW_INDEX_PAGE_LEN * 4,
              `table ${i} rowIndexPages`,
            ),
          }),
        );
      } catch (e) {
        throw new SceneError('invalidTable', `table ${i}: ${(e as Error).message}`);
      }
    }
    const text = slab(u32(h + layout.textSlab), textLen, 'textSlab');
    return new SceneReader({
      expectedSchema,
      schemaHash: schema,
      frameId: key(layout.frameId),
      nodes: slab(u32(h + layout.nodes), nodeCount * layout.nodeStride, 'nodes'),
      nodeCount,
      nodeStride: layout.nodeStride,
      textSlab: new Uint8Array(text.buffer, text.byteOffset, text.byteLength),
      childrenSlab: slab(u32(h + layout.childrenSlab), childrenLen * 4, 'childrenSlab'),
      childrenSlabLen: childrenLen,
      dirtyIndices: slab(u32(h + layout.dirtyIndices), dirtyCount * 4, 'dirtyIndices'),
      dirtyCount,
      removedIds: slab(u32(h + layout.removedIds), removedCount * 4, 'removedIds'),
      removedCount,
      tables,
    });
  }

  // ---- node records (offsets from the generated ABI) -------------------------------------

  nodeU8(index: number, offset: number): number {
    return this.nodes.getUint8(this.nodeAt(index, offset, 1));
  }

  nodeU16(index: number, offset: number): number {
    return this.nodes.getUint16(this.nodeAt(index, offset, 2), true);
  }

  nodeU32(index: number, offset: number): number {
    return this.nodes.getUint32(this.nodeAt(index, offset, 4), true);
  }

  nodeI32(index: number, offset: number): number {
    return this.nodes.getInt32(this.nodeAt(index, offset, 4), true);
  }

  nodeF32(index: number, offset: number): number {
    return this.nodes.getFloat32(this.nodeAt(index, offset, 4), true);
  }

  private nodeAt(index: number, offset: number, size: number): number {
    if (!(isIntIn(index, 0, this.nodeCount - 1) && isIntIn(offset, 0, this.nodeStride - size))) {
      throw new RangeError(`node ${index} offset ${offset} (+${size}) outside ${this.nodeCount}x${this.nodeStride}`);
    }
    return index * this.nodeStride + offset;
  }

  // ---- slabs ---------------------------------------------------------------------------------

  /** View of `textSlab[offset .. offset + len)`; throws when out of range or not on UTF-8 boundaries. */
  textBytes(offset: number, len: number): Uint8Array {
    const size = this.textSlab.length;
    if (!(isIntIn(offset, 0, size) && isIntIn(len, 0, size - offset)) || !this.isBoundary(offset) || !this.isBoundary(offset + len)) {
      throw new RangeError(`text [${offset}, +${len}) outside textSlab (${size}) or splits a code point`);
    }
    return this.textSlab.subarray(offset, offset + len);
  }

  text(offset: number, len: number): string {
    return UTF8_DECODER.decode(this.textBytes(offset, len));
  }

  private isBoundary(at: number): boolean {
    return at === this.textSlab.length || (this.textSlab[at]! & 0xc0) !== 0x80;
  }

  childId(slabIndex: number): number {
    return u32At(this.children, slabIndex, this.childrenSlabLen, 'child');
  }

  dirtyIndex(i: number): number {
    return u32At(this.dirty, i, this.dirtyCount, 'dirty');
  }

  removedId(i: number): number {
    return u32At(this.removed, i, this.removedCount, 'removed');
  }

  /** Visits every in-range dirty node index in O(dirty). */
  forEachDirty(visit: (index: number) => void): void {
    for (let i = 0; i < this.dirtyCount; i++) {
      const index = this.dirtyIndex(i);
      if (index < this.nodeCount) visit(index);
    }
  }

  /** Published table with `key`, or `null` (O(tables)). */
  findTable(key: Key64): TableReader | null {
    return this.tables.find((t) => t.key.equals(key)) ?? null;
  }
}

/** `count` integer records of `size` bytes fit `view` (division: no product to overflow or NaN). */
function requireRecords(view: DataView, count: number, size: number, what: string): void {
  if (!(isIntIn(count, 0, 0xffff_ffff) && Math.floor(view.byteLength / size) >= count)) {
    throw new SceneError('missingSlab', `${what} holds ${view.byteLength} bytes, need ${count} x ${size}`);
  }
}

function u32At(view: DataView, i: number, count: number, what: string): number {
  if (!isIntIn(i, 0, count - 1)) throw new RangeError(`${what} ${i} outside ${count}`);
  return view.getUint32(i * 4, true);
}
