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

/** Integer in `[min, max]` (rejects NaN, ±Infinity, and fractions). */
export function isIntIn(v: number, min: number, max: number): boolean {
  return Number.isInteger(v) && v >= min && v <= max;
}

/**
 * One field of a table row: `bits` wide at `shift` inside `word` (`Str` spans two words).
 *
 * Nominal (the ES private field keeps plain object literals out) and always well-formed: the
 * constructor throws `RangeError` unless `bits` is in `1..=32`, `shift + bits <= 32`, and the field
 * fits a 32-word row, so extraction never mis-shifts.
 */
export class FieldSpec {
  readonly word: number;
  readonly shift: number;
  readonly bits: number;
  readonly kind: FieldKind;
  readonly #validated = true;

  constructor(word: number, kind: FieldKind, shift = 0, bits = 32) {
    const span = kind === FieldKind.Str ? 2 : 1;
    if (!(isIntIn(bits, 1, 32) && isIntIn(shift, 0, 32 - bits) && isIntIn(word, 0, MAX_TABLE_WORDS - span) && isIntIn(kind, 0, 9))) {
      throw new RangeError(`invalid field spec word=${word} shift=${shift} bits=${bits} kind=${kind}`);
    }
    this.word = word;
    this.shift = shift;
    this.bits = bits;
    this.kind = kind;
  }

  /** `true` when the field lies inside a row of `words` words. */
  fits(words: number): boolean {
    return this.#validated && this.word + (this.kind === FieldKind.Str ? 2 : 1) <= words;
  }
}

/** `FieldSpec` shorthand; whole-word by default. */
export function field(word: number, kind: FieldKind, shift = 0, bits = 32): FieldSpec {
  return new FieldSpec(word, kind, shift, bits);
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
    if (!isIntIn(init.words, 1, MAX_TABLE_WORDS)) throw new RangeError(`words ${init.words} outside 1..=${MAX_TABLE_WORDS}`);
    const maxCount = 0xffff_ffff;
    if (!isIntIn(init.rowCount, 0, maxCount) || !isIntIn(dirtyCount, 0, maxCount) || !isIntIn(init.slot, 0, maxCount)) {
      throw new RangeError(`counts must be u32 integers: rowCount=${init.rowCount} dirtyCount=${dirtyCount} slot=${init.slot}`);
    }
    if (Math.floor(wordsIn(init.rows) / init.words) < init.rowCount) throw new RangeError(`rows hold ${wordsIn(init.rows)} words`);
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

  /** `{offset, len}` of a `Str` field inside the scene text slab. */
  /** `{offset, len}` of a `Str` field; throws `RangeError` for a non-`Str` spec or one past the row. */
  str(r: number, f: FieldSpec): { offset: number; len: number } {
    if (f.kind !== FieldKind.Str || !f.fits(this.words)) {
      throw new RangeError(`kind ${f.kind} field at word ${f.word} is not a string in a ${this.words}-word row`);
    }
    return { offset: this.word(r, f.word), len: this.word(r, f.word + 1) };
  }

  /** Raw `NodeId` owning sparse row `r`, or `r` itself for dense tables. */
  owner(r: number): number {
    if (!isIntIn(r, 0, this.rowCount - 1)) throw new RangeError(`row ${r} outside ${this.rowCount}`);
    return this.owners ? this.owners.getUint32(r * 4, true) : r;
  }

  /** Row of `ownerId` (raw `NodeId` sparse, node index dense), or `null`; O(rows) sparse. */
  rowOf(ownerId: number): number | null {
    if (!this.owners) return isIntIn(ownerId, 0, this.rowCount - 1) ? ownerId : null;
    for (let r = 0; r < this.rowCount; r++) {
      if (this.owners.getUint32(r * 4, true) === ownerId) return r;
    }
    return null;
  }

  dirtyRow(i: number): number {
    if (!isIntIn(i, 0, this.dirtyCount - 1) || !this.dirty) throw new RangeError(`dirty ${i} outside ${this.dirtyCount}`);
    return this.dirty.getUint32(i * 4, true);
  }
}
