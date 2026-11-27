/// Zero-copy reader over one published word table (`NveTableView`).
///
/// Dense tables: row `i` belongs to node index `i` and `owners` is empty. Sparse tables: row `r`
/// belongs to raw `NodeId` `owners[r]` (unsorted). Reads are bounds-checked and return `nil`
/// instead of touching memory outside the published buffers.

/// Mirrors Rust `nve_abi::table::FieldKind`.
public enum FieldKind: UInt8, Sendable {
    case u32 = 0, i32 = 1, f32 = 2, bool = 3, enumeration = 4, color = 5, length = 6, str = 7, resource = 8, node = 9
}

public struct FieldSpecError: Error, Equatable, Sendable {
    public let word: UInt8
    public let shift: UInt8
    public let bits: UInt8
}

/// One field of a table row: `bits` wide at `shift` inside `word` (`Str` spans `word` and `word + 1`).
///
/// Always well-formed: `bits` in `1...32`, `shift + bits <= 32`, and inside a 32-word row, so the
/// bit helpers below can never shift out of range.
public struct FieldSpec: Hashable, Sendable {
    public let word: UInt8
    public let shift: UInt8
    public let bits: UInt8
    public let kind: FieldKind

    public init(word: UInt8, shift: UInt8 = 0, bits: UInt8 = 32, kind: FieldKind) throws(FieldSpecError) {
        let span = kind == .str ? 2 : 1
        guard (1...32).contains(bits), Int(shift) + Int(bits) <= 32, Int(word) + span <= maxTableWords else {
            throw FieldSpecError(word: word, shift: shift, bits: bits)
        }
        self.word = word
        self.shift = shift
        self.bits = bits
        self.kind = kind
    }

    /// `true` when the field lies inside a row of `words` words.
    public func fits(words: Int) -> Bool { Int(word) + (kind == .str ? 2 : 1) <= words }

    /// Field bits of `word` (unsigned).
    public func extract(_ value: UInt32) -> UInt32 {
        bits >= 32 ? value >> UInt32(shift) : (value >> UInt32(shift)) & ((1 << UInt32(bits)) - 1)
    }

    /// Field bits of `word`, sign-extended from `bits`.
    public func extractSigned(_ value: UInt32) -> Int32 {
        let raw = extract(value)
        guard bits < 32 else { return Int32(bitPattern: raw) }
        let sign: UInt32 = 1 << UInt32(bits - 1)
        return raw & sign != 0 ? Int32(Int64(raw) - (Int64(1) << Int64(bits))) : Int32(raw)
    }
}

/// Decoded `nve_store::Length` (f32 bits with a 2-bit unit tag in the low mantissa bits).
public struct LengthValue: Equatable, Sendable {
    public enum Unit: UInt8, Sendable { case points = 0, percent = 1, auto = 2 }

    public let unit: Unit
    public let value: Float

    public init(unit: Unit, value: Float) {
        self.unit = unit
        self.value = value
    }

    /// Every bit pattern decodes; the unused tag `11` normalizes to `auto` like Rust.
    public init(word: UInt32) {
        switch word & 3 {
        case 0: self.init(unit: .points, value: Float(bitPattern: word & ~3))
        case 1: self.init(unit: .percent, value: Float(bitPattern: word & ~3))
        case 2: self.init(unit: .auto, value: Float(bitPattern: word & ~3))
        default: self.init(unit: .auto, value: 0)
        }
    }
}

/// Packed `RGBA8888` colour (`r | g<<8 | b<<16 | a<<24`).
public struct Rgba: Hashable, Sendable {
    public let packed: UInt32

    public init(packed: UInt32) { self.packed = packed }

    public var r: UInt8 { UInt8(truncatingIfNeeded: packed) }
    public var g: UInt8 { UInt8(truncatingIfNeeded: packed >> 8) }
    public var b: UInt8 { UInt8(truncatingIfNeeded: packed >> 16) }
    public var a: UInt8 { UInt8(truncatingIfNeeded: packed >> 24) }
}

/// Two-word string reference `{offset, len}` into the scene text slab.
public struct StrSpan: Hashable, Sendable {
    public let offset: UInt32
    public let len: UInt32

    public init(offset: UInt32, len: UInt32) {
        self.offset = offset
        self.len = len
    }
}

/// Shape of the generated C `NveTableView`; conform it with `extension NveTableView: TableViewABI {}`.
/// Pointers are implicitly unwrapped because that is how Swift imports C pointer fields.
public protocol TableViewABI {
    var key_lo: UInt32 { get }
    var key_hi: UInt32 { get }
    var slot: UInt32 { get }
    var words: UInt32 { get }
    var row_count: UInt32 { get }
    var dirty_count: UInt32 { get }
    var rows: UnsafePointer<UInt32>! { get }
    var owners: UnsafePointer<UInt32>! { get }
    var dirty_rows: UnsafePointer<UInt32>! { get }
}

/// Maximum words per table row (Rust `MAX_TABLE_WORDS`).
public let maxTableWords = 32

public struct TableReader {
    public let key: Key64
    public let slot: UInt32
    public let words: Int
    public let rowCount: Int
    public let rows: UnsafeBufferPointer<UInt32>
    public let owners: UnsafeBufferPointer<UInt32>
    public let dirtyRows: UnsafeBufferPointer<UInt32>

    /// `nil` when `rows` cannot hold `rowCount * words` words or `owners` is short.
    public init?(
        key: Key64, slot: UInt32, words: Int, rowCount: Int,
        rows: UnsafeBufferPointer<UInt32>, owners: UnsafeBufferPointer<UInt32> = .init(start: nil, count: 0),
        dirtyRows: UnsafeBufferPointer<UInt32> = .init(start: nil, count: 0)
    ) {
        guard (1...maxTableWords).contains(words), rowCount >= 0, rows.count / words >= rowCount,
              owners.isEmpty || owners.count >= rowCount
        else { return nil }
        self.key = key
        self.slot = slot
        self.words = words
        self.rowCount = rowCount
        self.rows = UnsafeBufferPointer(rebasing: rows[0..<rowCount * words])
        self.owners = owners.isEmpty ? owners : UnsafeBufferPointer(rebasing: owners[0..<rowCount])
        self.dirtyRows = dirtyRows
    }

    /// Wraps a generated `NveTableView`, rejecting a bad word count, counts the address space cannot
    /// hold (32-bit hosts), and null pointers with non-zero counts — all before any arithmetic can trap.
    public init?(view v: some TableViewABI) {
        guard v.words >= 1, v.words <= UInt32(maxTableWords),
              let rowCount = Int(exactly: v.row_count), let dirtyCount = Int(exactly: v.dirty_count),
              let rowWords = Int(exactly: UInt64(v.row_count) * UInt64(v.words)),
              rowWords <= Int.max / MemoryLayout<UInt32>.stride,
              rowWords == 0 || v.rows != nil, dirtyCount == 0 || v.dirty_rows != nil
        else { return nil }
        self.init(
            key: Key64(lo: v.key_lo, hi: v.key_hi), slot: v.slot, words: Int(v.words), rowCount: rowCount,
            rows: UnsafeBufferPointer(start: rowWords > 0 ? v.rows : nil, count: rowWords),
            owners: UnsafeBufferPointer(start: rowCount > 0 ? v.owners : nil, count: v.owners == nil ? 0 : rowCount),
            dirtyRows: UnsafeBufferPointer(start: dirtyCount > 0 ? v.dirty_rows : nil, count: dirtyCount)
        )
    }

    public var isSparse: Bool { !owners.isEmpty }

    /// All words of row `r` (empty when out of range).
    public func row(_ r: Int) -> UnsafeBufferPointer<UInt32> {
        guard (0..<rowCount).contains(r) else { return UnsafeBufferPointer(start: nil, count: 0) }
        return UnsafeBufferPointer(rebasing: rows[r * words..<(r + 1) * words])
    }

    public func word(_ r: Int, _ w: Int) -> UInt32? {
        guard (0..<rowCount).contains(r), (0..<words).contains(w) else { return nil }
        return rows[r * words + w]
    }

    /// Field bits of row `r`; `nil` for an out-of-range row or a field past this table's row.
    public func bits(_ r: Int, _ f: FieldSpec) -> UInt32? { f.fits(words: words) ? word(r, Int(f.word)).map(f.extract) : nil }
    public func u32(_ r: Int, _ f: FieldSpec) -> UInt32? { bits(r, f) }
    public func enumValue(_ r: Int, _ f: FieldSpec) -> UInt32? { bits(r, f) }
    public func resource(_ r: Int, _ f: FieldSpec) -> UInt32? { bits(r, f) }
    public func node(_ r: Int, _ f: FieldSpec) -> UInt32? { bits(r, f) }
    public func i32(_ r: Int, _ f: FieldSpec) -> Int32? {
        f.fits(words: words) ? word(r, Int(f.word)).map(f.extractSigned) : nil
    }
    public func f32(_ r: Int, _ f: FieldSpec) -> Float? { bits(r, f).map(Float.init(bitPattern:)) }
    public func bool(_ r: Int, _ f: FieldSpec) -> Bool? { bits(r, f).map { $0 & 1 != 0 } }
    public func color(_ r: Int, _ f: FieldSpec) -> Rgba? { bits(r, f).map(Rgba.init(packed:)) }
    public func length(_ r: Int, _ f: FieldSpec) -> LengthValue? { bits(r, f).map(LengthValue.init(word:)) }

    public func str(_ r: Int, _ f: FieldSpec) -> StrSpan? {
        guard let offset = word(r, Int(f.word)), let len = word(r, Int(f.word) + 1) else { return nil }
        return StrSpan(offset: offset, len: len)
    }

    /// Raw `NodeId` owning sparse row `r`, or `r` itself for dense tables.
    public func owner(_ r: Int) -> UInt32? {
        guard (0..<rowCount).contains(r) else { return nil }
        return owners.isEmpty ? UInt32(r) : owners[r]
    }

    /// Row of `ownerId` (raw `NodeId` for sparse tables, node index for dense ones); O(rows) sparse.
    public func row(of ownerId: UInt32) -> Int? {
        if owners.isEmpty { return Int(ownerId) < rowCount ? Int(ownerId) : nil }
        return owners.firstIndex(of: ownerId)
    }
}
