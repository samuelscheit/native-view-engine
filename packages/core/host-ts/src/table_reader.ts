/**
 * Zero-copy reader over one published word table (`NveTableView`).
 *
 * Dense tables: row `i` belongs to node index `i` (`owners === null`). Sparse tables: row `r`
 * belongs to raw `NodeId` `owners[r]` (unsorted), found in O(1) through the published paged
 * node-index → row index. Out-of-range reads throw `RangeError`.
 */
import { NVE_ROW_INDEX_NO_PAGE, NVE_ROW_INDEX_PAGE_BITS, NVE_ROW_INDEX_PAGE_LEN } from './generated/nve_abi.generated.ts';
import { Key64 } from './key64.ts';

/** Low bits of a raw `NodeId` holding its slot index (`index: 24 | generation: 8`). */
export const NODE_INDEX_MASK = 0xff_ffff;

/** Sentinel `valWord` / `explicitBit` / `eventsWord` meaning "not applicable" (Rust `NO_VAL_WORD`). */
export const NO_VAL_WORD = 0xff;

/** Maximum words per table row (Rust `MAX_TABLE_WORDS`). */
export const MAX_TABLE_WORDS = 32;

/** How a field's bits are interpreted (Rust `nve_abi::table::FieldKind`). */
export const FieldKind = {
  U32: 0,
  I32: 1,
  F32: 2,
  Bool: 3,
  Enum: 4,
  Color: 5,
  Length: 6,
  Str: 7,
  Resource: 8,
  Node: 9,
  U64: 10,
  I64: 11,
  F64: 12,
  GridLine: 13,
} as const;
export type FieldKind = (typeof FieldKind)[keyof typeof FieldKind];

/** Optionality and wire-bit encoding strategy (Rust `nve_abi::table::OptEncoding`). */
export const OptEncoding = {
  Required: 0,
  ZeroNiche: 1,
  Bit0Present: 2,
  PlusOne: 3,
  TagXor: 4,
  WideBit: 5,
} as const;
export type OptEncoding = (typeof OptEncoding)[keyof typeof OptEncoding];

/** Integer in `[min, max]` (rejects NaN, ±Infinity, and fractions). */
export function isIntIn(v: number, min: number, max: number): boolean {
  return Number.isInteger(v) && v >= min && v <= max;
}

/** `true` when `kind` spans two consecutive 32-bit words (`Str`, `U64`, `I64`, `F64`). */
export function isTwoWordKind(kind: FieldKind): boolean {
  return kind === FieldKind.Str || kind === FieldKind.U64 || kind === FieldKind.I64 || kind === FieldKind.F64;
}

/**
 * One field of a table row: `bits` wide at `shift` inside `word` (`Str` / 64-bit kinds span two
 * words; `OptEncoding.WideBit` puts a 1-bit presence flag at `(word, shift)` and the 32-bit value
 * in `valWord`). On `#[nve_table(inherited)]` tables, `explicitBit` names the field's bit in word 0
 * and `inherits` marks fields that fall back to the parent row when unset.
 *
 * Nominal (the ES private field keeps plain object literals out) and always well-formed: the
 * constructor throws `RangeError` on any invalid shape so extraction never mis-shifts or overruns.
 */
export class FieldSpec {
  readonly word: number;
  readonly shift: number;
  readonly bits: number;
  readonly kind: FieldKind;
  readonly opt: OptEncoding;
  readonly valWord: number;
  readonly explicitBit: number;
  readonly inherits: boolean;
  readonly #validated = true;

  constructor(
    word: number,
    kind: FieldKind,
    shift = 0,
    bits = 32,
    opt: OptEncoding = OptEncoding.Required,
    valWord = NO_VAL_WORD,
    explicitBit = NO_VAL_WORD,
    inherits = false,
  ) {
    if (!isIntIn(kind, 0, 13) || !isIntIn(opt, 0, 5) || typeof inherits !== 'boolean') {
      throw new RangeError(`invalid field spec word=${word} shift=${shift} bits=${bits} kind=${kind} opt=${opt}`);
    }
    const twoWord = isTwoWordKind(kind);
    const span = twoWord ? 2 : 1;
    const shapeOk = twoWord
      ? shift === 0 && bits === 32 && opt === OptEncoding.Required
      : isIntIn(bits, 1, 32) && isIntIn(shift, 0, 32 - bits);
    const wide = opt === OptEncoding.WideBit;
    const wideOk = wide
      ? bits === 1 && isIntIn(valWord, 0, MAX_TABLE_WORDS - 1) && valWord !== word
      : valWord === NO_VAL_WORD;
    const bit0Ok = opt !== OptEncoding.Bit0Present || bits >= 2;
    const tagXorOk =
      opt !== OptEncoding.TagXor ||
      (kind === FieldKind.Length && bits === 32) ||
      (kind === FieldKind.GridLine && bits === 16);
    const explicitOk = explicitBit === NO_VAL_WORD || isIntIn(explicitBit, 0, 31);
    const inheritOk =
      (!inherits || explicitBit !== NO_VAL_WORD) &&
      (explicitBit === NO_VAL_WORD || (word > 0 && (!wide || valWord > 0)));
    if (!(shapeOk && isIntIn(word, 0, MAX_TABLE_WORDS - span) && wideOk && bit0Ok && tagXorOk && explicitOk && inheritOk)) {
      throw new RangeError(
        `invalid field spec word=${word} shift=${shift} bits=${bits} kind=${kind} opt=${opt} valWord=${valWord} explicitBit=${explicitBit}`,
      );
    }
    this.word = word;
    this.shift = shift;
    this.bits = bits;
    this.kind = kind;
    this.opt = opt;
    this.valWord = valWord;
    this.explicitBit = explicitBit;
    this.inherits = inherits;
  }

  /** `true` when the field (and its wide value word, if any) lies inside a row of `words` words. */
  fits(words: number): boolean {
    const span = isTwoWordKind(this.kind) ? 2 : 1;
    return (
      this.#validated &&
      isIntIn(words, 1, MAX_TABLE_WORDS) &&
      this.word + span <= words &&
      (this.opt !== OptEncoding.WideBit || this.valWord < words)
    );
  }
}

/** `FieldSpec` shorthand; whole-word required by default. */
export function field(
  word: number,
  kind: FieldKind,
  shift = 0,
  bits = 32,
  opt: OptEncoding = OptEncoding.Required,
  valWord = NO_VAL_WORD,
  explicitBit = NO_VAL_WORD,
  inherits = false,
): FieldSpec {
  return new FieldSpec(word, kind, shift, bits, opt, valWord, explicitBit, inherits);
}

/** Field bits of `value` (unsigned). */
export function extractBits(f: FieldSpec, value: number): number {
  return f.bits >= 32 ? value >>> f.shift : (value >>> f.shift) & ((1 << f.bits) - 1);
}

/** Field bits of `value`, sign-extended from `bits`. */
export function extractSigned(f: FieldSpec, value: number): number {
  return f.bits >= 32 ? value | 0 : (extractBits(f, value) << (32 - f.bits)) >> (32 - f.bits);
}

export const LengthUnit = { Points: 0, Percent: 1, Auto: 2 } as const;
export type LengthUnit = (typeof LengthUnit)[keyof typeof LengthUnit];

/** Decoded `nve_store::Length` (f32 bits with a 2-bit unit tag in the low mantissa bits). */
export interface LengthValue {
  readonly unit: LengthUnit;
  readonly value: number;
}

export const GridLineKind = { Auto: 0, Line: 1, Span: 2 } as const;
export type GridLineKind = (typeof GridLineKind)[keyof typeof GridLineKind];

/** Decoded `nve_store::GridLine` (16-bit tagged placement: `Auto`, signed 14-bit `Line`, unsigned 14-bit `Span`). */
export interface GridLineValue {
  readonly kind: GridLineKind;
  readonly value: number;
}

const FLOAT_SCRATCH = new DataView(new ArrayBuffer(8));

/** Reinterprets u32 bits as an IEEE binary32 value. */
export function f32FromBits(bits: number): number {
  FLOAT_SCRATCH.setUint32(0, bits >>> 0, true);
  return FLOAT_SCRATCH.getFloat32(0, true);
}

/** Reinterprets little-endian `(lo, hi)` u32 halves as an IEEE binary64 value. */
export function f64FromBits(lo: number, hi: number): number {
  FLOAT_SCRATCH.setUint32(0, lo >>> 0, true);
  FLOAT_SCRATCH.setUint32(4, hi >>> 0, true);
  return FLOAT_SCRATCH.getFloat64(0, true);
}

/** Every bit pattern decodes; the unused tag `11` normalizes to `auto` with value 0, like Rust. */
export function decodeLength(word: number): LengthValue {
  const tag = word & 3;
  if (tag === 3) return { unit: LengthUnit.Auto, value: 0 };
  return { unit: tag as LengthUnit, value: f32FromBits(word & ~3) };
}

/** Every 16-bit pattern decodes; the unused tag `11` normalizes to `Auto` with value 0, like Rust. */
export function decodeGridLine(bits: number): GridLineValue {
  const v = bits & 0x3fff;
  const tag = (bits >>> 14) & 3;
  if (tag === 1) return { kind: GridLineKind.Line, value: (v << 18) >> 18 };
  if (tag === 2) return { kind: GridLineKind.Span, value: v };
  return { kind: GridLineKind.Auto, value: 0 };
}

/** Channels of a packed `RGBA8888` word (`r | g<<8 | b<<16 | a<<24`). */
export function rgba(packed: number): { r: number; g: number; b: number; a: number } {
  return { r: packed & 0xff, g: (packed >>> 8) & 0xff, b: (packed >>> 16) & 0xff, a: packed >>> 24 };
}

export interface TableReaderInit {
  readonly key: Key64;
  readonly slot: number;
  readonly words: number;
  readonly rowCount: number;
  readonly rows: DataView;
  readonly owners?: DataView | null;
  readonly dirtyRows?: DataView;
  readonly dirtyCount?: number;
  /** Sparse tables: page of every `NVE_ROW_INDEX_PAGE_LEN` node indices (`NVE_ROW_INDEX_NO_PAGE` = none). */
  readonly rowIndexDir?: DataView;
  /** Sparse tables: `row + 1` per node index of each page (`0` = no row); a partial tail page is dropped. */
  readonly rowIndexPages?: DataView;
}

function wordsIn(view: DataView | null | undefined): number {
  return view ? Math.floor(view.byteLength / 4) : 0;
}

export class TableReader {
  readonly key: Key64;
  readonly slot: number;
  readonly words: number;
  readonly rowCount: number;
  readonly dirtyCount: number;
  private readonly rows: DataView;
  private readonly owners: DataView | null;
  private readonly dirty: DataView | null;
  private readonly dir: DataView | null;
  private readonly pages: DataView | null;
  private readonly pageCount: number;

  /**
   * A sparse table a host assembles itself (recorded or synthetic scenes): row `r` is owned by raw
   * `NodeId` `owners[r]`, and the row index the runtime publishes is built here. Throws
   * `RangeError` when two rows share a node index.
   */
  static sparse(init: Omit<TableReaderInit, 'rowCount' | 'owners' | 'rowIndexDir' | 'rowIndexPages'> & { readonly owners: readonly number[] }): TableReader {
    const dir: number[] = [];
    const pages: Uint32Array[] = [];
    init.owners.forEach((owner, r) => {
      const index = owner & NODE_INDEX_MASK;
      const dirSlot = index >>> NVE_ROW_INDEX_PAGE_BITS;
      while (dir.length <= dirSlot) dir.push(NVE_ROW_INDEX_NO_PAGE);
      if (dir[dirSlot] === NVE_ROW_INDEX_NO_PAGE) {
        dir[dirSlot] = pages.length;
        pages.push(new Uint32Array(NVE_ROW_INDEX_PAGE_LEN));
      }
      const page = pages[dir[dirSlot]!]!;
      const entry = index & (NVE_ROW_INDEX_PAGE_LEN - 1);
      if (page[entry] !== 0) throw new RangeError(`rows ${page[entry]! - 1} and ${r} share node index ${index}`);
      page[entry] = r + 1;
    });
    const flat = new Uint32Array(pages.length * NVE_ROW_INDEX_PAGE_LEN);
    pages.forEach((p, i) => flat.set(p, i * NVE_ROW_INDEX_PAGE_LEN));
    const view = (a: Uint32Array) => new DataView(a.buffer, a.byteOffset, a.byteLength);
    return new TableReader({
      ...init,
      rowCount: init.owners.length,
      owners: view(Uint32Array.from(init.owners)),
      rowIndexDir: view(Uint32Array.from(dir)),
      rowIndexPages: view(flat),
    });
  }

  /** Throws `RangeError` when `words` is outside `1..=32` or a view is shorter than its count. */
  constructor(init: TableReaderInit) {
    const dirtyCount = init.dirtyCount ?? 0;
    if (!isIntIn(init.words, 1, MAX_TABLE_WORDS)) throw new RangeError(`words ${init.words} outside 1..=${MAX_TABLE_WORDS}`);
    const maxCount = 0xffff_ffff;
    if (!isIntIn(init.rowCount, 0, maxCount) || !isIntIn(dirtyCount, 0, maxCount) || !isIntIn(init.slot, 0, maxCount)) {
      throw new RangeError(`counts must be u32 integers: rowCount=${init.rowCount} dirtyCount=${dirtyCount} slot=${init.slot}`);
    }
    if (Math.floor(wordsIn(init.rows) / init.words) < init.rowCount) throw new RangeError(`rows hold ${wordsIn(init.rows)} words`);
    if (init.owners && wordsIn(init.owners) < init.rowCount) throw new RangeError(`owners hold ${wordsIn(init.owners)} words`);
    if (wordsIn(init.dirtyRows) < dirtyCount) throw new RangeError(`dirtyRows hold ${wordsIn(init.dirtyRows)} words`);
    // Sparse rows without a row index (one whole page at least) could never be resolved.
    if (init.owners && init.rowCount > 0 && (wordsIn(init.rowIndexDir) === 0 || wordsIn(init.rowIndexPages) < NVE_ROW_INDEX_PAGE_LEN)) {
      throw new RangeError(`sparse table with ${init.rowCount} rows publishes no row index`);
    }
    this.key = init.key;
    this.slot = init.slot;
    this.words = init.words;
    this.rowCount = init.rowCount;
    this.dirtyCount = dirtyCount;
    this.rows = init.rows;
    // An empty table is dense whatever its owners pointer (Rust publishes `NonNull::dangling()`
    // for an empty `Vec`), like C++ `is_sparse()`.
    this.owners = init.rowCount > 0 ? (init.owners ?? null) : null;
    this.dirty = init.dirtyRows ?? null;
    this.dir = init.rowIndexDir ?? null;
    this.pages = init.rowIndexPages ?? null;
    // Whole pages only (like C++ `TableReader::make`): a partial tail page is unreachable.
    this.pageCount = Math.floor(wordsIn(init.rowIndexPages) / NVE_ROW_INDEX_PAGE_LEN);
  }

  get isSparse(): boolean {
    return this.owners !== null;
  }

  /** Raw word `w` of row `r` (unsigned). */
  word(r: number, w: number): number {
    if (!(isIntIn(r, 0, this.rowCount - 1) && isIntIn(w, 0, this.words - 1))) {
      throw new RangeError(`row ${r} word ${w} outside ${this.rowCount}x${this.words}`);
    }
    return this.rows.getUint32((r * this.words + w) * 4, true);
  }

  /** Field bits of row `r`; throws `RangeError` when the field lies past this table's row. */
  bits(r: number, f: FieldSpec): number {
    return extractBits(f, this.fieldWord(r, f));
  }

  private fieldWord(r: number, f: FieldSpec): number {
    if (!f.fits(this.words)) throw new RangeError(`field word ${f.word} past ${this.words}-word row`);
    return this.word(r, f.word);
  }

  /** `true` when `f` was explicitly written on row `r` (always `true` for non-inherited tables). */
  isExplicit(r: number, f: FieldSpec): boolean {
    if (!f.fits(this.words)) throw new RangeError(`field word ${f.word} past ${this.words}-word row`);
    const mask = this.word(r, 0);
    return f.explicitBit === NO_VAL_WORD || ((mask >>> f.explicitBit) & 1) !== 0;
  }

  /** `true` when an optional (or required) field holds a `Some` value in row `r`. */
  isPresent(r: number, f: FieldSpec): boolean {
    return this.optRawUnsigned(r, f) !== null;
  }

  /** Raw `EventSet<E>` listener bitmask from `eventsWord` (`0` when `eventsWord === NO_VAL_WORD`). */
  eventMask(r: number, eventsWord: number): number {
    if (eventsWord === NO_VAL_WORD) {
      if (!isIntIn(r, 0, this.rowCount - 1)) throw new RangeError(`row ${r} outside ${this.rowCount}`);
      return 0;
    }
    return this.word(r, eventsWord);
  }

  /** `true` when row `r` subscribes to `eventId` (`0..=31`) in `eventsWord`. */
  hasEvent(r: number, eventsWord: number, eventId: number): boolean {
    const mask = this.eventMask(r, eventsWord);
    return isIntIn(eventId, 0, 31) && ((mask >>> eventId) & 1) !== 0;
  }

  u32(r: number, f: FieldSpec): number {
    return this.bits(r, f);
  }

  enumValue(r: number, f: FieldSpec): number {
    return this.bits(r, f);
  }

  resource(r: number, f: FieldSpec): number {
    return this.bits(r, f);
  }

  node(r: number, f: FieldSpec): number {
    return this.bits(r, f);
  }

  i32(r: number, f: FieldSpec): number {
    return extractSigned(f, this.fieldWord(r, f));
  }

  f32(r: number, f: FieldSpec): number {
    return f32FromBits(this.bits(r, f));
  }

  bool(r: number, f: FieldSpec): boolean {
    return (this.bits(r, f) & 1) !== 0;
  }

  color(r: number, f: FieldSpec): number {
    return this.bits(r, f);
  }

  length(r: number, f: FieldSpec): LengthValue {
    return decodeLength(this.bits(r, f));
  }

  gridLine(r: number, f: FieldSpec): GridLineValue {
    return decodeGridLine(this.bits(r, f));
  }

  /** Exact unsigned 64-bit field as `(lo, hi)` `u32` halves (53-bit-safe, zero `BigInt`). */
  u64(r: number, f: FieldSpec): Key64 {
    if (f.kind !== FieldKind.U64 || !f.fits(this.words)) {
      throw new RangeError(`kind ${f.kind} field at word ${f.word} is not a u64 in a ${this.words}-word row`);
    }
    return new Key64(this.word(r, f.word), this.word(r, f.word + 1));
  }

  /** Exact two's-complement halves of a signed 64-bit field (zero `BigInt`). */
  i64Bits(r: number, f: FieldSpec): Key64 {
    if (f.kind !== FieldKind.I64 || !f.fits(this.words)) {
      throw new RangeError(`kind ${f.kind} field at word ${f.word} is not an i64 in a ${this.words}-word row`);
    }
    return new Key64(this.word(r, f.word), this.word(r, f.word + 1));
  }

  /** Exact signed 64-bit field as a `bigint` (53-bit-safe). */
  i64(r: number, f: FieldSpec): bigint {
    return this.i64Bits(r, f).toSignedBigInt();
  }

  /** IEEE binary64 field from two consecutive little-endian words. */
  f64(r: number, f: FieldSpec): number {
    if (f.kind !== FieldKind.F64 || !f.fits(this.words)) {
      throw new RangeError(`kind ${f.kind} field at word ${f.word} is not an f64 in a ${this.words}-word row`);
    }
    return f64FromBits(this.word(r, f.word), this.word(r, f.word + 1));
  }

  /** `{offset, len}` of a `Str` field in the scene text slab; throws `RangeError` for a non-`Str` spec or one past the row. */
  str(r: number, f: FieldSpec): { offset: number; len: number } {
    if (f.kind !== FieldKind.Str || !f.fits(this.words)) {
      throw new RangeError(`kind ${f.kind} field at word ${f.word} is not a string in a ${this.words}-word row`);
    }
    return { offset: this.word(r, f.word), len: this.word(r, f.word + 1) };
  }

  // ---- optional-aware readers (handle `Required` and every `OptEncoding`) --------------------

  optU32(r: number, f: FieldSpec): number | null {
    return this.optRawUnsigned(r, f);
  }

  optEnum(r: number, f: FieldSpec): number | null {
    return this.optRawUnsigned(r, f);
  }

  optNode(r: number, f: FieldSpec): number | null {
    return this.optRawUnsigned(r, f);
  }

  optColor(r: number, f: FieldSpec): number | null {
    return this.optRawUnsigned(r, f);
  }

  optF32(r: number, f: FieldSpec): number | null {
    const raw = this.optRawUnsigned(r, f);
    return raw === null ? null : f32FromBits(raw);
  }

  optBool(r: number, f: FieldSpec): boolean | null {
    const raw = this.optRawUnsigned(r, f);
    if (raw === null) return null;
    if (f.opt === OptEncoding.PlusOne && raw > 1) return null;
    return (raw & 1) !== 0;
  }

  optI32(r: number, f: FieldSpec): number | null {
    const w = this.fieldWord(r, f);
    switch (f.opt) {
      case OptEncoding.Required:
        return extractSigned(f, w);
      case OptEncoding.Bit0Present: {
        const raw = extractBits(f, w);
        if ((raw & 1) === 0) return null;
        const shift = 33 - f.bits;
        return ((raw >>> 1) << shift) >> shift;
      }
      case OptEncoding.WideBit:
        return ((w >>> f.shift) & 1) !== 0 ? this.word(r, f.valWord) | 0 : null;
      case OptEncoding.ZeroNiche: {
        const raw = extractBits(f, w);
        return raw === 0 ? null : extractSigned(f, w);
      }
      case OptEncoding.PlusOne: {
        const raw = extractBits(f, w);
        return raw === 0 ? null : (raw - 1) | 0;
      }
      case OptEncoding.TagXor:
        return null;
    }
  }

  optLength(r: number, f: FieldSpec): LengthValue | null {
    const raw = this.optRawUnsigned(r, f);
    return raw === null ? null : decodeLength(raw);
  }

  optGridLine(r: number, f: FieldSpec): GridLineValue | null {
    const raw = this.optRawUnsigned(r, f);
    return raw === null ? null : decodeGridLine(raw);
  }

  private optRawUnsigned(r: number, f: FieldSpec): number | null {
    const w = this.fieldWord(r, f);
    switch (f.opt) {
      case OptEncoding.Required:
        return extractBits(f, w);
      case OptEncoding.ZeroNiche: {
        const raw = extractBits(f, w);
        return raw === 0 ? null : raw;
      }
      case OptEncoding.Bit0Present: {
        const raw = extractBits(f, w);
        return (raw & 1) === 0 ? null : raw >>> 1;
      }
      case OptEncoding.PlusOne: {
        const raw = extractBits(f, w);
        return raw === 0 ? null : raw - 1;
      }
      case OptEncoding.TagXor: {
        const raw = extractBits(f, w);
        const niche = f.kind === FieldKind.GridLine ? 0xc000 : 3;
        return (raw & niche) === 0 ? null : (raw ^ niche) >>> 0;
      }
      case OptEncoding.WideBit:
        return ((w >>> f.shift) & 1) !== 0 ? this.word(r, f.valWord) : null;
    }
  }

  /** Raw `NodeId` owning sparse row `r`, or `r` itself for dense tables. */
  owner(r: number): number {
    if (!isIntIn(r, 0, this.rowCount - 1)) throw new RangeError(`row ${r} outside ${this.rowCount}`);
    return this.owners ? this.owners.getUint32(r * 4, true) : r;
  }

  /**
   * Row of node slot `index`, or `null`; O(1). Sparse rows resolve through the row index
   * (`row_index_dir` page → `row + 1` entry), without checking the owner's generation.
   */
  rowOfNode(index: number): number | null {
    if (!this.owners) return isIntIn(index, 0, this.rowCount - 1) ? index : null;
    if (!isIntIn(index, 0, NODE_INDEX_MASK) || !this.dir || !this.pages) return null;
    const dirSlot = index >>> NVE_ROW_INDEX_PAGE_BITS;
    if (dirSlot >= wordsIn(this.dir)) return null;
    const page = this.dir.getUint32(dirSlot * 4, true);
    if (page === NVE_ROW_INDEX_NO_PAGE || page >= this.pageCount) return null;
    const rowPlusOne = this.pages.getUint32((page * NVE_ROW_INDEX_PAGE_LEN + (index & (NVE_ROW_INDEX_PAGE_LEN - 1))) * 4, true);
    return rowPlusOne === 0 || rowPlusOne > this.rowCount ? null : rowPlusOne - 1;
  }

  /**
   * Row of `ownerId` (raw `NodeId` sparse, node index dense), or `null`; O(1). A sparse row must be
   * owned by exactly `ownerId`: a recycled slot's stale id resolves to `null`.
   */
  rowOf(ownerId: number): number | null {
    if (!this.owners) return this.rowOfNode(ownerId);
    if (!isIntIn(ownerId, 0, 0xffff_ffff)) return null;
    const r = this.rowOfNode(ownerId & NODE_INDEX_MASK);
    return r !== null && this.owners.getUint32(r * 4, true) === ownerId ? r : null;
  }

  dirtyRow(i: number): number {
    if (!isIntIn(i, 0, this.dirtyCount - 1) || !this.dirty) throw new RangeError(`dirty ${i} outside ${this.dirtyCount}`);
    return this.dirty.getUint32(i * 4, true);
  }
}
