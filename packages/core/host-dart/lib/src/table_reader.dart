/// Zero-copy reader over one published word table (`NveTableView`).
///
/// Dense tables: row `i` belongs to node index `i` (`owners == null`). Sparse tables: row `r`
/// belongs to raw `NodeId` `owners[r]` (unsorted). Out-of-range reads throw [RangeError].
library;

import 'dart:typed_data';

import 'key64.dart';

/// How a field's bits are interpreted (Rust `nve_abi::table::FieldKind`; `index` is the wire value).
enum FieldKind { u32, i32, f32, boolean, enumeration, color, length, str, resource, node }

/// One field of a table row: [bits] wide at [shift] inside [word] (`str` spans two words).
final class FieldSpec {
  const FieldSpec(this.word, this.kind, {this.shift = 0, this.bits = 32});

  final int word;
  final int shift;
  final int bits;
  final FieldKind kind;

  /// `true` when the field lies inside a row of [words] words.
  bool fits(int words) {
    final span = kind == FieldKind.str ? 2 : 1;
    return word >= 0 && shift >= 0 && bits >= 1 && bits <= 32 && shift + bits <= 32 && word + span <= words;
  }

  /// Field bits of [value] (unsigned).
  int extract(int value) => bits >= 32 ? value >>> shift : (value >>> shift) & ((1 << bits) - 1);

  /// Field bits of [value], sign-extended from [bits].
  int extractSigned(int value) {
    final raw = extract(value);
    final sign = bits >= 32 ? 0x80000000 : 1 << (bits - 1);
    final span = bits >= 32 ? 0x100000000 : 1 << bits;
    return raw >= sign ? raw - span : raw;
  }
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

  final LengthUnit unit;
  final double value;

  @override
  bool operator ==(Object other) => other is LengthValue && other.unit == unit && other.value == value;

  @override
  int get hashCode => Object.hash(unit, value);

  @override
  String toString() => 'LengthValue(${unit.name}, $value)';
}

/// Channels of a packed `RGBA8888` word (`r | g<<8 | b<<16 | a<<24`).
extension type const Rgba(int packed) {
  int get r => packed & 0xFF;
  int get g => (packed >>> 8) & 0xFF;
  int get b => (packed >>> 16) & 0xFF;
  int get a => (packed >>> 24) & 0xFF;
}

final ByteData _bits = ByteData(4);

/// Reinterprets u32 bits as an IEEE binary32 value.
double f32FromBits(int bits) {
  _bits.setUint32(0, bits & 0xFFFFFFFF, Endian.little);
  return _bits.getFloat32(0, Endian.little);
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
  })  : _rows = rows,
        _owners = owners,
        _dirty = dirtyRows ?? ByteData(0) {
    if (words < 1 || words > maxTableWords) {
      throw ArgumentError.value(words, 'words', 'must be in 1..=$maxTableWords');
    }
    _require(rows, rowCount * words, 'rows');
    if (owners != null) {
      _require(owners, rowCount, 'owners');
    }
    if (dirtyCount > 0) {
      _require(dirtyRows, dirtyCount, 'dirtyRows');
    }
  }

  static void _require(ByteData? view, int count, String what) {
    if (count < 0 || (count > 0 && (view == null || view.lengthInBytes ~/ 4 < count))) {
      throw ArgumentError('$what holds ${view == null ? 'null' : view.lengthInBytes ~/ 4} words, need $count');
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

  bool get isSparse => _owners != null;

  /// Raw word [w] of row [r].
  int word(int r, int w) {
    RangeError.checkValidIndex(r, this, 'row', rowCount);
    RangeError.checkValidIndex(w, this, 'word', words);
    return _rows.getUint32((r * words + w) * 4, Endian.little);
  }

  int bits(int r, FieldSpec f) => f.extract(word(r, f.word));
  int u32(int r, FieldSpec f) => bits(r, f);
  int enumValue(int r, FieldSpec f) => bits(r, f);
  int resource(int r, FieldSpec f) => bits(r, f);
  int node(int r, FieldSpec f) => bits(r, f);
  int i32(int r, FieldSpec f) => f.extractSigned(word(r, f.word));
  double f32(int r, FieldSpec f) => f32FromBits(bits(r, f));
  bool boolean(int r, FieldSpec f) => (bits(r, f) & 1) != 0;
  Rgba color(int r, FieldSpec f) => Rgba(bits(r, f));
  LengthValue length(int r, FieldSpec f) => LengthValue.decode(bits(r, f));

  /// `(offset, len)` of a `str` field inside the scene text slab.
  (int offset, int len) str(int r, FieldSpec f) => (word(r, f.word), word(r, f.word + 1));

  /// Raw `NodeId` owning sparse row [r], or [r] itself for dense tables.
  int owner(int r) {
    RangeError.checkValidIndex(r, this, 'row', rowCount);
    final owners = _owners;
    return owners == null ? r : owners.getUint32(r * 4, Endian.little);
  }

  /// Row of [ownerId] (raw `NodeId` sparse, node index dense), or `null`; O(rows) sparse.
  int? rowOf(int ownerId) {
    final owners = _owners;
    if (owners == null) {
      return ownerId >= 0 && ownerId < rowCount ? ownerId : null;
    }
    for (var r = 0; r < rowCount; r++) {
      if (owners.getUint32(r * 4, Endian.little) == ownerId) {
        return r;
      }
    }
    return null;
  }

  int dirtyRow(int i) {
    RangeError.checkValidIndex(i, this, 'dirty', dirtyCount);
    return _dirty.getUint32(i * 4, Endian.little);
  }
}
