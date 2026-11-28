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

/// How a field's bits are interpreted (Rust `nve_abi::table::FieldKind`; `index` is the wire value).
enum FieldKind { u32, i32, f32, boolean, enumeration, color, length, str, resource, node }

/// One field of a table row: [bits] wide at [shift] inside [word] (`str` spans two words).
///
/// Always well-formed: construction throws [ArgumentError] unless `bits` is in `1..=32`,
/// `shift + bits <= 32`, and the field fits a 32-word row, so extraction never mis-shifts.
final class FieldSpec {
  factory FieldSpec(int word, FieldKind kind, {int shift = 0, int bits = 32}) {
    final span = kind == FieldKind.str ? 2 : 1;
    // Compare by subtraction after the range checks: `shift + bits` could wrap the 64-bit VM `int`.
    if (bits < 1 || bits > 32 || shift < 0 || shift > 32 - bits || word < 0 || word > maxTableWords - span) {
      throw ArgumentError('invalid field spec word=$word shift=$shift bits=$bits');
    }
    return FieldSpec._(word, kind, shift, bits);
  }

  const FieldSpec._(this.word, this.kind, this.shift, this.bits);

  final int word;
  final int shift;
  final int bits;
  final FieldKind kind;

  /// `true` when the field lies inside a row of [words] words.
  bool fits(int words) => word + (kind == FieldKind.str ? 2 : 1) <= words;

  /// Field bits of [value] (unsigned).
  /// Masks [value] to 32 bits first, so a negative VM `int` extracts exactly as on the web.
  int extract(int value) {
    final word = value & 0xFFFFFFFF;
    return bits >= 32 ? word >>> shift : (word >>> shift) & ((1 << bits) - 1);
  }

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

  /// Field bits of row [r]; throws [RangeError] when the field lies past this table's row.
  int bits(int r, FieldSpec f) => f.extract(_fieldWord(r, f));

  int _fieldWord(int r, FieldSpec f) {
    if (!f.fits(words)) {
      throw RangeError('field word ${f.word} past $words-word row');
    }
    return word(r, f.word);
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
