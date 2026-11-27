/**
 * Zero-copy reader over one published word table (`NveTableView`).
 *
 * Dense tables: row `i` belongs to node index `i` (`owners === null`). Sparse tables: row `r`
 * belongs to raw `NodeId` `owners[r]` (unsorted). Out-of-range reads throw `RangeError`.
 */
import { Key64 } from './key64.ts';

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
} as const;
export type FieldKind = (typeof FieldKind)[keyof typeof FieldKind];

/** One field of a table row: `bits` wide at `shift` inside `word` (`Str` spans two words). */
export interface FieldSpec {
  readonly word: number;
  readonly shift: number;
  readonly bits: number;
  readonly kind: FieldKind;
}

/** `FieldSpec` literal; whole-word by default. */
export function field(word: number, kind: FieldKind, shift = 0, bits = 32): FieldSpec {
  return { word, shift, bits, kind };
}

/** `true` when `f` lies inside a row of `words` words. */
export function fieldFits(f: FieldSpec, words: number): boolean {
  const span = f.kind === FieldKind.Str ? 2 : 1;
  return f.word >= 0 && f.shift >= 0 && f.bits >= 1 && f.bits <= 32 && f.shift + f.bits <= 32 && f.word + span <= words;
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

const F32_SCRATCH = new DataView(new ArrayBuffer(4));

/** Reinterprets u32 bits as an IEEE binary32 value. */
export function f32FromBits(bits: number): number {
  F32_SCRATCH.setUint32(0, bits >>> 0, true);
  return F32_SCRATCH.getFloat32(0, true);
}

/** Every bit pattern decodes; the unused tag `11` normalizes to `auto` with value 0, like Rust. */
export function decodeLength(word: number): LengthValue {
  const tag = word & 3;
  if (tag === 3) return { unit: LengthUnit.Auto, value: 0 };
  return { unit: tag as LengthUnit, value: f32FromBits(word & ~3) };
}

/** Channels of a packed `RGBA8888` word (`r | g<<8 | b<<16 | a<<24`). */
export function rgba(packed: number): { r: number; g: number; b: number; a: number } {
  return { r: packed & 0xff, g: (packed >>> 8) & 0xff, b: (packed >>> 16) & 0xff, a: packed >>> 24 };
}

/** Maximum words per table row (Rust `MAX_TABLE_WORDS`). */
export const MAX_TABLE_WORDS = 32;

export interface TableReaderInit {
  readonly key: Key64;
  readonly slot: number;
  readonly words: number;
  readonly rowCount: number;
  readonly rows: DataView;
  readonly owners?: DataView | null;
  readonly dirtyRows?: DataView;
  readonly dirtyCount?: number;
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

  /** Throws `RangeError` when `words` is outside `1..=32` or a view is shorter than its count. */
  constructor(init: TableReaderInit) {
    const dirtyCount = init.dirtyCount ?? 0;
    if (!Number.isInteger(init.words) || init.words < 1 || init.words > MAX_TABLE_WORDS) {
      throw new RangeError(`words ${init.words} outside 1..=${MAX_TABLE_WORDS}`);
    }
    if (init.rowCount < 0 || dirtyCount < 0) throw new RangeError('negative count');
    if (wordsIn(init.rows) < init.rowCount * init.words) throw new RangeError(`rows hold ${wordsIn(init.rows)} words`);
    if (init.owners && wordsIn(init.owners) < init.rowCount) throw new RangeError(`owners hold ${wordsIn(init.owners)} words`);
    if (wordsIn(init.dirtyRows) < dirtyCount) throw new RangeError(`dirtyRows hold ${wordsIn(init.dirtyRows)} words`);
    this.key = init.key;
    this.slot = init.slot;
    this.words = init.words;
    this.rowCount = init.rowCount;
    this.dirtyCount = dirtyCount;
    this.rows = init.rows;
    this.owners = init.owners ?? null;
    this.dirty = init.dirtyRows ?? null;
  }

  get isSparse(): boolean {
    return this.owners !== null;
  }

  /** Raw word `w` of row `r` (unsigned). */
  word(r: number, w: number): number {
    if (!(r >= 0 && r < this.rowCount && w >= 0 && w < this.words)) {
      throw new RangeError(`row ${r} word ${w} outside ${this.rowCount}x${this.words}`);
    }
    return this.rows.getUint32((r * this.words + w) * 4, true);
  }

  bits(r: number, f: FieldSpec): number {
    return extractBits(f, this.word(r, f.word));
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
    return extractSigned(f, this.word(r, f.word));
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

  /** `{offset, len}` of a `Str` field inside the scene text slab. */
  str(r: number, f: FieldSpec): { offset: number; len: number } {
    return { offset: this.word(r, f.word), len: this.word(r, f.word + 1) };
  }

  /** Raw `NodeId` owning sparse row `r`, or `r` itself for dense tables. */
  owner(r: number): number {
    if (!(r >= 0 && r < this.rowCount)) throw new RangeError(`row ${r} outside ${this.rowCount}`);
    return this.owners ? this.owners.getUint32(r * 4, true) : r;
  }

  /** Row of `ownerId` (raw `NodeId` sparse, node index dense), or `null`; O(rows) sparse. */
  rowOf(ownerId: number): number | null {
    if (!this.owners) return ownerId >= 0 && ownerId < this.rowCount && Number.isInteger(ownerId) ? ownerId : null;
    for (let r = 0; r < this.rowCount; r++) {
      if (this.owners.getUint32(r * 4, true) === ownerId) return r;
    }
    return null;
  }

  dirtyRow(i: number): number {
    if (!(i >= 0 && i < this.dirtyCount) || !this.dirty) throw new RangeError(`dirty ${i} outside ${this.dirtyCount}`);
    return this.dirty.getUint32(i * 4, true);
  }
}
