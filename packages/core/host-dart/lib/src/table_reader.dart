/// Zero-copy reader over one published word table (`NveTableView`).
///
/// Dense tables: row `i` belongs to node index `i` (`owners == null`). Sparse tables: row `r`
/// belongs to raw `NodeId` `owners[r]` (unsorted), found in O(1) through the published paged
/// node-index → row index. Out-of-range reads throw [RangeError].
library;

import 'dart:typed_data';

import 'generated/nve_abi_const.generated.dart' show nveRowIndexNoPage, nveRowIndexPageBits, nveRowIndexPageLen;
import 'key64.dart';

/// Low bits of a raw `NodeId` holding its slot index (`index: 24 | generation: 8`).
const int nodeIndexMask = 0xFFFFFF;

/// Sentinel for `FieldSpec.valWord`, `FieldSpec.explicitBit`, and `NveTableInfo.eventsWord` ("none").
const int noFieldSlot = 0xFF;

/// How a field's bits are interpreted (Rust `nve_abi::table::FieldKind`; `index` is the wire value).
enum FieldKind { u32, i32, f32, boolean, enumeration, color, length, str, resource, node, u64, i64, f64, gridLine }

/// Optionality and wire-bit encoding strategy (Rust `nve_abi::table::OptEncoding`; `index` is the wire value).
enum OptEncoding { required, zeroNiche, bit0Present, plusOne, tagXor, wideBit }

/// One field of a table row: [bits] wide at [shift] inside [word] (`str`, `u64`, `i64`, `f64` span
/// two words; `wideBit` optionals store a 1-bit presence flag at `[word, shift]` and the full
/// 32-bit value at [valWord]).
///
/// Always well-formed: construction throws [ArgumentError] on out-of-range word/shift/bits or
/// inconsistent optionality / inheritance metadata, so extraction never mis-shifts.
final class FieldSpec {
  factory FieldSpec(
    int word,
    FieldKind kind, {
    int shift = 0,
    int bits = 32,
    OptEncoding opt = OptEncoding.required,
    int valWord = noFieldSlot,
    int explicitBit = noFieldSlot,
    bool inherits = false,
  }) {
    final twoWord = _isTwoWordKind(kind);
    final span = twoWord ? 2 : 1;
    if (word < 0 || word > maxTableWords - span) {
      throw ArgumentError('invalid field spec word=$word kind=${kind.name}');
    }
    if (twoWord) {
      if (opt != OptEncoding.required || shift != 0 || (bits != 32 && bits != 64)) {
        throw ArgumentError('invalid 2-word field spec kind=${kind.name} shift=$shift bits=$bits opt=${opt.name}');
      }
    } else {
      // Compare by subtraction after the range checks: `shift + bits` could wrap the 64-bit VM `int`.
      if (bits < 1 || bits > 32 || shift < 0 || shift > 32 - bits) {
        throw ArgumentError('invalid field spec word=$word shift=$shift bits=$bits');
      }
    }
    if (opt == OptEncoding.wideBit) {
      if (bits != 1 || valWord < 0 || valWord >= maxTableWords) {
        throw ArgumentError('invalid wideBit field spec bits=$bits valWord=$valWord');
      }
    } else if (valWord != noFieldSlot) {
      throw ArgumentError('valWord=$valWord is only valid for OptEncoding.wideBit');
    }
    if (opt == OptEncoding.bit0Present && bits < 2) {
      throw ArgumentError('bit0Present field requires bits >= 2, got $bits');
    }
    if (explicitBit != noFieldSlot && (explicitBit < 0 || explicitBit >= 32)) {
      throw ArgumentError('invalid explicitBit=$explicitBit');
    }
    if (inherits && explicitBit == noFieldSlot) {
      throw ArgumentError('inheriting field requires explicitBit in 0..31');
    }
    return FieldSpec._(word, kind, shift, bits, opt, valWord, explicitBit, inherits);
  }

  const FieldSpec._(this.word, this.kind, this.shift, this.bits, this.opt, this.valWord, this.explicitBit, this.inherits);

  static bool _isTwoWordKind(FieldKind k) =>
      k == FieldKind.str || k == FieldKind.u64 || k == FieldKind.i64 || k == FieldKind.f64;

  final int word;
  final int shift;
  final int bits;
  final FieldKind kind;
  final OptEncoding opt;
  final int valWord;
  final int explicitBit;
  final bool inherits;

  bool get isTwoWord => _isTwoWordKind(kind);
  bool get isOptional => opt != OptEncoding.required;
  bool get hasExplicitBit => explicitBit != noFieldSlot;

  /// `true` when [explicitMask] (word 0 of an inherited table) marks this field explicitly set, or
  /// when this field has no explicit bit.
  bool isExplicitIn(int explicitMask) => !hasExplicitBit || (((explicitMask & 0xFFFFFFFF) >>> explicitBit) & 1) != 0;

  /// `true` when the field (and its [valWord] for `wideBit`) lies inside a row of [words] words.
  bool fits(int words) {
    if (word + (isTwoWord ? 2 : 1) > words) {
      return false;
    }
    return opt != OptEncoding.wideBit || valWord < words;
  }

  /// Field bits of [value] (unsigned).
  /// Masks [value] to 32 bits first, so a negative VM `int` extracts exactly as on the web.
  int extract(int value) {
    final w = value & 0xFFFFFFFF;
    return bits >= 32 ? w >>> shift : (w >>> shift) & ((1 << bits) - 1);
  }

  /// Field bits of [value], sign-extended from [bits].
  int extractSigned(int value) => _signExtend(extract(value), bits);
}

int _signExtend(int raw, int width) {
  final sign = width >= 32 ? 0x80000000 : 1 << (width - 1);
  final span = width >= 32 ? 0x100000000 : 1 << width;
  return raw >= sign ? raw - span : raw;
}

/// Decoded `nve_store::Length` (f32 bits with a 2-bit unit tag in the low mantissa bits).
enum LengthUnit { points, percent, auto }

final class LengthValue {
  const LengthValue(this.unit, this.value);

  /// Every bit pattern decodes; the unused tag `11` normalizes to `auto` with value 0, like Rust.
  factory LengthValue.decode(int word) {
    final tag = word & 3;
    if (tag == 3) {
      return const LengthValue(LengthUnit.auto, 0);
    }
    return LengthValue(LengthUnit.values[tag], f32FromBits(word & 0xFFFFFFFC));
  }

  /// Decodes `Option<Length>` (`OptEncoding.tagXor`: low tag bits `00` = `null`, else XOR `0b11`).
  static LengthValue? decodeOpt(int word) {
    final w = word & 0xFFFFFFFF;
    return (w & 3) == 0 ? null : LengthValue.decode(w ^ 3);
  }

  final LengthUnit unit;
  final double value;

  @override
  bool operator ==(Object other) => other is LengthValue && other.unit == unit && other.value == value;

  @override
  int get hashCode => Object.hash(unit, value);

  @override
  String toString() => 'LengthValue(${unit.name}, $value)';
}

/// Discriminant of a packed 16-bit CSS Grid line placement (`nve_store::GridLine`).
enum GridLineKind { auto, line, span }

/// Decoded `nve_store::GridLine`: 2-bit tag in bits 14..15 (`00` auto, `01` signed 14-bit line,
/// `10` unsigned 14-bit span; `11` unused niche normalizing to `auto`).
final class GridLineValue {
  const GridLineValue(this.kind, [this.value = 0]);
  const GridLineValue.auto() : this(GridLineKind.auto, 0);
  const GridLineValue.line(int line) : this(GridLineKind.line, line);
  const GridLineValue.span(int span) : this(GridLineKind.span, span);

  factory GridLineValue.decode(int bits) {
    final b = bits & 0xFFFF;
    final v = b & 0x3FFF;
    return switch ((b >>> 14) & 3) {
      1 => GridLineValue(GridLineKind.line, v >= 0x2000 ? v - 0x4000 : v),
      2 => GridLineValue(GridLineKind.span, v),
      _ => const GridLineValue.auto(),
    };
  }

  /// Decodes `Option<GridLine>` (`OptEncoding.tagXor`: bits 14..15 `00` = `null`, else XOR `0xC000`).
  static GridLineValue? decodeOpt(int bits) {
    final b = bits & 0xFFFF;
    return (b & 0xC000) == 0 ? null : GridLineValue.decode(b ^ 0xC000);
  }

  final GridLineKind kind;
  final int value;

  @override
  bool operator ==(Object other) => other is GridLineValue && other.kind == kind && other.value == value;

  @override
  int get hashCode => Object.hash(kind, value);

  @override
  String toString() => switch (kind) {
        GridLineKind.auto => 'GridLineValue.auto()',
        GridLineKind.line => 'GridLineValue.line($value)',
        GridLineKind.span => 'GridLineValue.span($value)',
      };
}

/// Channels of a packed `RGBA8888` word (`r | g<<8 | b<<16 | a<<24`).
extension type const Rgba(int packed) {
  int get r => packed & 0xFF;
  int get g => (packed >>> 8) & 0xFF;
  int get b => (packed >>> 16) & 0xFF;
  int get a => (packed >>> 24) & 0xFF;
}

final ByteData _bits = ByteData(8);

/// Reinterprets u32 bits as an IEEE binary32 value.
double f32FromBits(int bits) {
  _bits.setUint32(0, bits & 0xFFFFFFFF, Endian.little);
  return _bits.getFloat32(0, Endian.little);
}

/// Reinterprets an IEEE binary32 value as unsigned u32 bits (web-safe).
int f32ToBits(double value) {
  _bits.setFloat32(0, value, Endian.little);
  return _bits.getUint32(0, Endian.little);
}

/// Reinterprets little-endian `(lo, hi)` u32 halves as an IEEE binary64 value (web-safe).
double f64FromBits(int lo, int hi) {
  _bits
    ..setUint32(0, lo & 0xFFFFFFFF, Endian.little)
    ..setUint32(4, hi & 0xFFFFFFFF, Endian.little);
  return _bits.getFloat64(0, Endian.little);
}

/// Maximum words per table row (Rust `MAX_TABLE_WORDS`).
const int maxTableWords = 32;

final class TableReader {
  /// Throws [ArgumentError] when [words] is outside `1..=32` or a view is shorter than its count.
  TableReader({
    required this.key,
    required this.slot,
    required this.words,
    required this.rowCount,
    required ByteData rows,
    ByteData? owners,
    ByteData? dirtyRows,
    this.dirtyCount = 0,
    ByteData? rowIndexDir,
    ByteData? rowIndexPages,
  })  : _rows = rows,
        // An empty table is dense whatever its owners pointer (Rust publishes
        // `NonNull::dangling()` for an empty `Vec`), like C++ `is_sparse()`.
        _owners = rowCount > 0 ? owners : null,
        _dirty = dirtyRows ?? ByteData(0),
        _dir = rowIndexDir ?? ByteData(0),
        // Whole pages only (like C++ `TableReader::make`): a partial tail page is unreachable.
        _pageCount = (rowIndexPages?.lengthInBytes ?? 0) ~/ (4 * nveRowIndexPageLen),
        _pages = rowIndexPages ?? ByteData(0) {
    if (words < 1 || words > maxTableWords) {
      throw ArgumentError.value(words, 'words', 'must be in 1..=$maxTableWords');
    }
    if (rowCount < 0 || dirtyCount < 0) {
      throw ArgumentError('negative count: rowCount=$rowCount dirtyCount=$dirtyCount');
    }
    // Division, never `rowCount * words`: a hostile count cannot wrap the 64-bit VM `int`.
    _require(rows, rowCount, words, 'rows');
    if (owners != null) {
      _require(owners, rowCount, 1, 'owners');
      // Sparse rows without a row index (one whole page at least) could never be resolved.
      if (rowCount > 0 && (_dir.lengthInBytes < 4 || _pageCount == 0)) {
        throw ArgumentError('sparse table with $rowCount rows publishes no row index');
      }
    }
    _require(dirtyRows, dirtyCount, 1, 'dirtyRows');
  }

  /// Throws unless [view] holds `count` groups of `wordsPer` u32 words.
  static void _require(ByteData? view, int count, int wordsPer, String what) {
    if (count == 0) {
      return;
    }
    final have = view == null ? 0 : view.lengthInBytes ~/ (4 * wordsPer);
    if (have < count) {
      throw ArgumentError('$what holds $have rows of $wordsPer words, need $count');
    }
  }

  final Key64 key;
  final int slot;
  final int words;
  final int rowCount;
  final int dirtyCount;
  final ByteData _rows;
  final ByteData? _owners;
  final ByteData _dirty;
  final ByteData _dir;
  final ByteData _pages;
  final int _pageCount;

  /// A sparse table a host assembles itself (recorded or synthetic scenes): row `r` is owned by
  /// raw `NodeId` `owners[r]`, and the row index the runtime publishes is built here. Throws
  /// [ArgumentError] when two rows share a node index.
  factory TableReader.sparse({
    required Key64 key,
    required int slot,
    required int words,
    required ByteData rows,
    required List<int> owners,
    ByteData? dirtyRows,
    int dirtyCount = 0,
  }) {
    final dir = <int>[];
    final pages = <Uint32List>[];
    for (var r = 0; r < owners.length; r++) {
      final index = owners[r] & nodeIndexMask;
      final dirSlot = index >> nveRowIndexPageBits;
      while (dir.length <= dirSlot) {
        dir.add(nveRowIndexNoPage);
      }
      if (dir[dirSlot] == nveRowIndexNoPage) {
        dir[dirSlot] = pages.length;
        pages.add(Uint32List(nveRowIndexPageLen));
      }
      final page = pages[dir[dirSlot]];
      final entry = index & (nveRowIndexPageLen - 1);
      if (page[entry] != 0) {
        throw ArgumentError('rows ${page[entry] - 1} and $r share node index $index');
      }
      page[entry] = r + 1;
    }
    final flat = Uint32List(pages.length * nveRowIndexPageLen);
    for (var p = 0; p < pages.length; p++) {
      flat.setAll(p * nveRowIndexPageLen, pages[p]);
    }
    return TableReader(
      key: key,
      slot: slot,
      words: words,
      rowCount: owners.length,
      rows: rows,
      owners: ByteData.sublistView(Uint32List.fromList(owners)),
      dirtyRows: dirtyRows,
      dirtyCount: dirtyCount,
      rowIndexDir: ByteData.sublistView(Uint32List.fromList(dir)),
      rowIndexPages: ByteData.sublistView(flat),
    );
  }

  bool get isSparse => _owners != null;

  /// Raw word [w] of row [r].
  int word(int r, int w) {
    RangeError.checkValidIndex(r, this, 'row', rowCount);
    RangeError.checkValidIndex(w, this, 'word', words);
    return _rows.getUint32((r * words + w) * 4, Endian.little);
  }

  /// Raw `__explicit` override bitmask at word 0 of an inherited table row [r].
  int explicitMask(int r) => word(r, 0);

  /// `true` when [f]'s explicit-override bit is set in word 0 of row [r] (always `true` for
  /// fields without an explicit bit).
  bool isExplicit(int r, FieldSpec f) => !f.hasExplicitBit || f.isExplicitIn(word(r, 0));

  /// Listener bitmask (`EventSet<E>`) at [eventsWord] of row [r] (`0` when [eventsWord] is `0xFF`).
  int eventMask(int r, int eventsWord) {
    RangeError.checkValidIndex(r, this, 'row', rowCount);
    return eventsWord == noFieldSlot ? 0 : word(r, eventsWord);
  }

  int _fieldWord(int r, FieldSpec f) {
    if (!f.fits(words)) {
      throw RangeError('field word ${f.word} past $words-word row');
    }
    return word(r, f.word);
  }

  /// Raw extracted field bits of row [r] at [f.word]; throws [RangeError] if [f] lies past the row.
  int bits(int r, FieldSpec f) => f.extract(_fieldWord(r, f));

  /// `true` when optional field [f] is present in row [r] (always `true` for `required` fields).
  bool isPresent(int r, FieldSpec f) => _presentRaw(r, f) != null;

  int? _presentRaw(int r, FieldSpec f) {
    final raw = bits(r, f);
    return switch (f.opt) {
      OptEncoding.required => raw,
      OptEncoding.zeroNiche => raw == 0 ? null : raw,
      OptEncoding.bit0Present => (raw & 1) == 0 ? null : raw >>> 1,
      OptEncoding.plusOne => raw == 0 ? null : raw - 1,
      OptEncoding.tagXor => f.kind == FieldKind.gridLine
          ? ((raw & 0xC000) == 0 ? null : (raw & 0xFFFF) ^ 0xC000)
          : ((raw & 3) == 0 ? null : raw ^ 3),
      OptEncoding.wideBit => (raw & 1) == 0 ? null : word(r, f.valWord),
    };
  }

  int u32(int r, FieldSpec f) => bits(r, f);
  int enumValue(int r, FieldSpec f) => bits(r, f);
  int resource(int r, FieldSpec f) => bits(r, f);
  int node(int r, FieldSpec f) => bits(r, f);
  int i32(int r, FieldSpec f) => f.extractSigned(_fieldWord(r, f));
  double f32(int r, FieldSpec f) => f32FromBits(bits(r, f));
  bool boolean(int r, FieldSpec f) => (bits(r, f) & 1) != 0;
  Rgba color(int r, FieldSpec f) => Rgba(bits(r, f));
  LengthValue length(int r, FieldSpec f) => LengthValue.decode(bits(r, f));
  GridLineValue gridLine(int r, FieldSpec f) => GridLineValue.decode(bits(r, f));

  /// Two-word unsigned 64-bit value as exact `(lo, hi)` [Key64] (53-bit-safe on Dart-web).
  Key64 u64(int r, FieldSpec f) {
    if (f.kind != FieldKind.u64 || !f.fits(words)) {
      throw RangeError('${f.kind.name} field at word ${f.word} is not a u64 in a $words-word row');
    }
    return Key64(word(r, f.word), word(r, f.word + 1));
  }

  /// Two-word signed 64-bit value as exact two's-complement `(lo, hi)` [Key64] (53-bit-safe on Dart-web).
  Key64 i64Bits(int r, FieldSpec f) {
    if (f.kind != FieldKind.i64 || !f.fits(words)) {
      throw RangeError('${f.kind.name} field at word ${f.word} is not an i64 in a $words-word row');
    }
    return Key64(word(r, f.word), word(r, f.word + 1));
  }

  /// Two-word signed 64-bit integer (`int` when exact on this platform; throws [StateError] on
  /// Dart-web beyond ±2^53).
  int i64(int r, FieldSpec f) => i64Bits(r, f).toSignedInt();

  /// Two-word IEEE binary64 float (web-safe).
  double f64(int r, FieldSpec f) {
    if (f.kind != FieldKind.f64 || !f.fits(words)) {
      throw RangeError('${f.kind.name} field at word ${f.word} is not an f64 in a $words-word row');
    }
    return f64FromBits(word(r, f.word), word(r, f.word + 1));
  }

  /// Optional unsigned integer (`zeroNiche`, `bit0Present`, `plusOne`, `wideBit`, or `required`).
  int? optU32(int r, FieldSpec f) => _presentRaw(r, f);

  /// Optional enum discriminant (`plusOne`, `wideBit`, `zeroNiche`, or `required`).
  int? optEnumValue(int r, FieldSpec f) => _presentRaw(r, f);

  /// Optional `NodeId` (`zeroNiche`, `wideBit`, or `required`).
  int? optNode(int r, FieldSpec f) => _presentRaw(r, f);

  /// Optional signed integer (`bit0Present`, `wideBit`, `zeroNiche`, `plusOne`, or `required`).
  int? optI32(int r, FieldSpec f) {
    final raw = _presentRaw(r, f);
    if (raw == null) {
      return null;
    }
    final width = switch (f.opt) {
      OptEncoding.bit0Present => f.bits - 1,
      OptEncoding.wideBit => 32,
      _ => f.bits,
    };
    return _signExtend(raw, width);
  }

  /// Optional IEEE binary32 float (`wideBit` or `required`).
  double? optF32(int r, FieldSpec f) {
    final raw = _presentRaw(r, f);
    return raw == null ? null : f32FromBits(raw);
  }

  /// Optional boolean (`plusOne`: `0` = null, `1` = false, `2` = true, `3` = null; `wideBit`; or `required`).
  bool? optBoolean(int r, FieldSpec f) {
    if (f.opt == OptEncoding.plusOne) {
      return switch (bits(r, f)) {
        1 => false,
        2 => true,
        _ => null,
      };
    }
    final raw = _presentRaw(r, f);
    return raw == null ? null : (raw & 1) != 0;
  }

  /// Optional packed RGBA color (`wideBit` or `required`).
  Rgba? optColor(int r, FieldSpec f) {
    final raw = _presentRaw(r, f);
    return raw == null ? null : Rgba(raw);
  }

  /// Optional CSS length (`tagXor`, `wideBit`, or `required`).
  LengthValue? optLength(int r, FieldSpec f) {
    final raw = _presentRaw(r, f);
    return raw == null ? null : LengthValue.decode(raw);
  }

  /// Optional CSS Grid line (`tagXor`, `wideBit`, or `required`).
  GridLineValue? optGridLine(int r, FieldSpec f) {
    final raw = _presentRaw(r, f);
    return raw == null ? null : GridLineValue.decode(raw);
  }

  /// `(offset, len)` of a `str` field inside the scene text slab.
  /// Throws [RangeError] for a non-`str` spec or one past this table's row.
  (int offset, int len) str(int r, FieldSpec f) {
    if (f.kind != FieldKind.str || !f.fits(words)) {
      throw RangeError('${f.kind.name} field at word ${f.word} is not a string in a $words-word row');
    }
    return (word(r, f.word), word(r, f.word + 1));
  }

  /// Raw `NodeId` owning sparse row [r], or [r] itself for dense tables.
  int owner(int r) {
    RangeError.checkValidIndex(r, this, 'row', rowCount);
    final owners = _owners;
    return owners == null ? r : owners.getUint32(r * 4, Endian.little);
  }

  /// Row of node slot [index], or `null`; O(1). Sparse rows resolve through the row index
  /// (`row_index_dir` page → `row + 1` entry), without checking the owner's generation.
  int? rowOfNode(int index) {
    // Node slots are 24-bit; a wider value would wrap dart2js's 32-bit shifts onto a real page.
    if (index < 0 || index > nodeIndexMask) {
      return null;
    }
    if (_owners == null) {
      return index < rowCount ? index : null;
    }
    final dirSlot = index >> nveRowIndexPageBits;
    if (dirSlot >= _dir.lengthInBytes ~/ 4) {
      return null;
    }
    final page = _dir.getUint32(dirSlot * 4, Endian.little);
    if (page == nveRowIndexNoPage || page >= _pageCount) {
      return null;
    }
    final rowPlusOne = _pages.getUint32((page * nveRowIndexPageLen + (index & (nveRowIndexPageLen - 1))) * 4, Endian.little);
    return rowPlusOne == 0 || rowPlusOne > rowCount ? null : rowPlusOne - 1;
  }

  /// Row of [ownerId] (raw `NodeId` sparse, node index dense), or `null`; O(1). A sparse row must
  /// be owned by exactly [ownerId]: a recycled slot's stale id resolves to `null`.
  int? rowOf(int ownerId) {
    final owners = _owners;
    if (owners == null) {
      return rowOfNode(ownerId);
    }
    final r = rowOfNode(ownerId & nodeIndexMask);
    return r != null && owners.getUint32(r * 4, Endian.little) == ownerId ? r : null;
  }

  int dirtyRow(int i) {
    RangeError.checkValidIndex(i, this, 'dirty', dirtyCount);
    return _dirty.getUint32(i * 4, Endian.little);
  }
}
