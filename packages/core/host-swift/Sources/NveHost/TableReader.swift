/// Zero-copy reader over one published word table (`NveTableView`).
///
/// Dense tables: row `i` belongs to node index `i` and `owners` is empty. Sparse tables: row `r`
/// belongs to raw `NodeId` `owners[r]` (unsorted). Reads are bounds-checked and return `nil`
/// instead of touching memory outside the published buffers.

/// Mirrors Rust `nve_abi::table::FieldKind`.
public enum FieldKind: UInt8, Sendable {
    case u32 = 0, i32 = 1, f32 = 2, bool = 3, enumeration = 4, color = 5, length = 6, str = 7, resource = 8, node = 9,
         u64 = 10, i64 = 11, f64 = 12, gridLine = 13
}

public extension FieldKind {
    /// `true` for fields spanning two consecutive 32-bit words (`str`, `u64`, `i64`, `f64`).
    var isTwoWord: Bool {
        switch self {
        case .str, .u64, .i64, .f64: true
        default: false
        }
    }

    var wordSpan: Int { isTwoWord ? 2 : 1 }
}

/// Mirrors Rust `nve_abi::table::OptEncoding`.
public enum OptEncoding: UInt8, Sendable {
    case required = 0
    case zeroNiche = 1
    case bit0Present = 2
    case plusOne = 3
    case tagXor = 4
    case wideBit = 5
}

/// Sentinel `FieldSpec.valWord` / `explicitBit` / `NveTableInfo.eventsWord` meaning "none".
public let noValWord: UInt8 = 0xFF

public struct FieldSpecError: Error, Equatable, Sendable {
    public let word: UInt8
    public let shift: UInt8
    public let bits: UInt8
}

/// One field of a table row: `bits` wide at `shift` inside `word` (`str` / `u64` / `i64` / `f64`
/// span `word` and `word + 1`; `wideBit` optionals store a 1-bit presence flag at `(word, shift)`
/// and their 32-bit payload in `valWord`).
///
/// Always well-formed, so the bit helpers below can never shift out of range.
public struct FieldSpec: Hashable, Sendable {
    public let word: UInt8
    public let shift: UInt8
    public let bits: UInt8
    public let kind: FieldKind
    public let opt: OptEncoding
    /// Value word for `.wideBit` optionals (`noValWord` = `0xFF` for all other fields).
    public let valWord: UInt8
    /// Bit in word 0 (`__explicit`) for `#[nve_table(inherited)]` tables (`0xFF` = none).
    public let explicitBit: UInt8
    /// `true` when an unset field on an inherited table resolves from the parent row.
    public let inherits: Bool

    public init(
        word: UInt8,
        shift: UInt8 = 0,
        bits: UInt8 = 32,
        kind: FieldKind,
        opt: OptEncoding = .required,
        valWord: UInt8 = noValWord,
        explicitBit: UInt8 = noValWord,
        inherits: Bool = false
    ) throws(FieldSpecError) {
        let span = kind.wordSpan
        let rangeOk = span == 2
            ? (shift == 0 && bits == 32 && opt == .required)
            : ((1...32).contains(bits) && Int(shift) + Int(bits) <= 32)
        let wideOk = opt == .wideBit
            ? (bits == 1 && valWord != noValWord && Int(valWord) < maxTableWords && valWord != word)
            : (valWord == noValWord)
        let optShapeOk: Bool = switch opt {
        case .bit0Present: bits >= 2
        case .tagXor: (kind == .length && bits == 32) || (kind == .gridLine && bits == 16)
        default: true
        }
        let inheritOk = (explicitBit == noValWord || explicitBit < 32)
            && (!inherits || explicitBit < 32)
            && (explicitBit == noValWord || (word != 0 && (opt != .wideBit || valWord != 0)))
        guard rangeOk, wideOk, optShapeOk, inheritOk, Int(word) + span <= maxTableWords else {
            throw FieldSpecError(word: word, shift: shift, bits: bits)
        }
        self.word = word
        self.shift = shift
        self.bits = bits
        self.kind = kind
        self.opt = opt
        self.valWord = valWord
        self.explicitBit = explicitBit
        self.inherits = inherits
    }

    public var isOptional: Bool { opt != .required }
    public var isExplicitTracked: Bool { explicitBit < 32 }
    public var explicitMask: UInt32 { explicitBit < 32 ? (UInt32(1) << UInt32(explicitBit)) : 0 }

    /// `true` when the field (and any wide value word / explicit word) lies inside a row of `words` words.
    public func fits(words: Int) -> Bool {
        guard Int(word) + kind.wordSpan <= words else { return false }
        if opt == .wideBit, Int(valWord) >= words { return false }
        if explicitBit < 32, words < 1 { return false }
        return true
    }

    /// Raw field bits of `word` (unsigned, without `OptEncoding` decoding).
    public func extract(_ value: UInt32) -> UInt32 {
        bits >= 32 ? value >> UInt32(shift) : (value >> UInt32(shift)) & ((1 << UInt32(bits)) - 1)
    }

    /// Sign-extends `raw` from `bitWidth` (`1...32`) into `Int32`.
    public static func signExtend(_ raw: UInt32, bits bitWidth: UInt8) -> Int32 {
        guard bitWidth > 0, bitWidth < 32 else { return Int32(bitPattern: raw) }
        let mask: UInt32 = (UInt32(1) << UInt32(bitWidth)) - 1
        let v = raw & mask
        let sign: UInt32 = UInt32(1) << UInt32(bitWidth - 1)
        return v & sign != 0 ? Int32(Int64(v) - (Int64(1) << Int64(bitWidth))) : Int32(v)
    }

    /// Raw field bits of `word`, sign-extended from `bits`.
    public func extractSigned(_ value: UInt32) -> Int32 {
        Self.signExtend(extract(value), bits: bits)
    }

    /// Decodes unsigned / raw payload bits according to `opt`; `nil` when absent.
    public func decodeBits(word w: UInt32, valWord vw: UInt32 = 0) -> UInt32? {
        let raw = extract(w)
        switch opt {
        case .required: return raw
        case .zeroNiche: return raw == 0 ? nil : raw
        case .bit0Present: return (raw & 1) == 0 ? nil : (raw >> 1)
        case .plusOne: return raw == 0 ? nil : (raw - 1)
        case .tagXor:
            if kind == .length { return (raw & 0b11) == 0 ? nil : (raw ^ 0b11) }
            if kind == .gridLine { return (raw & 0xC000) == 0 ? nil : (raw ^ 0xC000) }
            return nil
        case .wideBit: return (raw & 1) == 0 ? nil : vw
        }
    }

    /// Decodes a signed integer field according to `opt`; `nil` when absent.
    public func decodeSigned(word w: UInt32, valWord vw: UInt32 = 0) -> Int32? {
        let raw = extract(w)
        switch opt {
        case .required: return Self.signExtend(raw, bits: bits)
        case .zeroNiche: return raw == 0 ? nil : Self.signExtend(raw, bits: bits)
        case .bit0Present: return (raw & 1) == 0 ? nil : Self.signExtend(raw >> 1, bits: bits - 1)
        case .plusOne: return raw == 0 ? nil : Int32(bitPattern: raw - 1)
        case .wideBit: return (raw & 1) == 0 ? nil : Int32(bitPattern: vw)
        case .tagXor: return nil
        }
    }

    /// Decodes a boolean field according to `opt` (`plusOne` maps `1 -> false`, `2 -> true`, else `nil`).
    public func decodeBool(word w: UInt32, valWord vw: UInt32 = 0) -> Bool? {
        let raw = extract(w)
        switch opt {
        case .required: return (raw & 1) != 0
        case .plusOne: return raw == 1 ? false : (raw == 2 ? true : nil)
        case .bit0Present: return (raw & 1) == 0 ? nil : (((raw >> 1) & 1) != 0)
        case .wideBit: return (raw & 1) == 0 ? nil : ((vw & 1) != 0)
        case .zeroNiche, .tagXor: return nil
        }
    }

    /// Decodes unsigned / raw payload bits from a `defaults` / row word array; `nil` if absent or out of range.
    public func decodeBits(from words: [UInt32]) -> UInt32? {
        guard fits(words: words.count) else { return nil }
        return decodeBits(word: words[Int(word)], valWord: opt == .wideBit ? words[Int(valWord)] : 0)
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

/// Decoded 16-bit CSS Grid line placement (`nve_store::GridLine`).
public enum GridLineValue: Hashable, Sendable {
    case auto
    case line(Int16)
    case span(UInt16)

    /// Minimum representable signed 14-bit grid line (`-8192`).
    public static let minLine: Int16 = -8192
    /// Maximum representable signed 14-bit grid line (`8191`).
    public static let maxLine: Int16 = 8191
    /// Maximum representable unsigned 14-bit grid span (`16383`).
    public static let maxSpan: UInt16 = 16383

    public init(word: UInt32) {
        let payload = word & 0x3FFF
        switch (word >> 14) & 0b11 {
        case 1:
            let signed = Int32(bitPattern: payload << 18) >> 18
            self = .line(Int16(signed))
        case 2:
            self = .span(UInt16(payload))
        default:
            self = .auto
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
    var row_index_dir_len: UInt32 { get }
    var row_index_page_count: UInt32 { get }
    var rows: UnsafePointer<UInt32>! { get }
    var owners: UnsafePointer<UInt32>! { get }
    var dirty_rows: UnsafePointer<UInt32>! { get }
    var row_index_dir: UnsafePointer<UInt32>! { get }
    var row_index_pages: UnsafePointer<UInt32>! { get }
}

/// Maximum words per table row (Rust `MAX_TABLE_WORDS`).
public let maxTableWords = 32
/// Node indices per published row-index page (generated `nveRowIndexPageLen`), as an `Int` count.
private let pageLen = Int(nveRowIndexPageLen)

public struct TableReader {
    public let key: Key64
    public let slot: UInt32
    public let words: Int
    public let rowCount: Int
    public let rows: UnsafeBufferPointer<UInt32>
    public let owners: UnsafeBufferPointer<UInt32>
    public let dirtyRows: UnsafeBufferPointer<UInt32>
    /// Sparse tables: page of every `nveRowIndexPageLen` node indices (`nveRowIndexNoPage` = none).
    public let rowIndexDir: UnsafeBufferPointer<UInt32>
    /// Sparse tables: `row + 1` per node index of each page (0 = no row).
    public let rowIndexPages: UnsafeBufferPointer<UInt32>

    /// `nil` when `rows` cannot hold `rowCount * words` words, `owners` is short, or a sparse table
    /// (owners) with rows has no row index with at least one whole page (its rows could never
    /// resolve). A partial tail page of `rowIndexPages` is dropped (unreachable), as C++
    /// `TableReader::make` does.
    public init?(
        key: Key64, slot: UInt32, words: Int, rowCount: Int,
        rows: UnsafeBufferPointer<UInt32>, owners: UnsafeBufferPointer<UInt32> = .init(start: nil, count: 0),
        dirtyRows: UnsafeBufferPointer<UInt32> = .init(start: nil, count: 0),
        rowIndexDir: UnsafeBufferPointer<UInt32> = .init(start: nil, count: 0),
        rowIndexPages: UnsafeBufferPointer<UInt32> = .init(start: nil, count: 0)
    ) {
        guard (1...maxTableWords).contains(words), rowCount >= 0, rows.count / words >= rowCount,
              owners.isEmpty || owners.count >= rowCount,
              owners.isEmpty || rowCount == 0 || (!rowIndexDir.isEmpty && rowIndexPages.count >= pageLen)
        else { return nil }
        self.key = key
        self.slot = slot
        self.words = words
        self.rowCount = rowCount
        self.rows = UnsafeBufferPointer(rebasing: rows[0..<rowCount * words])
        self.owners = owners.isEmpty ? owners : UnsafeBufferPointer(rebasing: owners[0..<rowCount])
        self.dirtyRows = dirtyRows
        self.rowIndexDir = rowIndexDir
        self.rowIndexPages = UnsafeBufferPointer(rebasing: rowIndexPages[0..<rowIndexPages.count - rowIndexPages.count % pageLen])
    }

    /// Wraps a generated `NveTableView`, rejecting a bad word count, counts the address space cannot
    /// hold (32-bit hosts), and null pointers with non-zero counts — all before any arithmetic can trap.
    public init?(view v: some TableViewABI) {
        let stride = MemoryLayout<UInt32>.stride
        guard v.words >= 1, v.words <= UInt32(maxTableWords),
              let rowWords = Self.fitsAddressSpace(UInt64(v.row_count) * UInt64(v.words), stride: stride),
              let rowCount = Self.fitsAddressSpace(UInt64(v.row_count), stride: stride),
              let dirtyCount = Self.fitsAddressSpace(UInt64(v.dirty_count), stride: stride),
              let dirLen = Self.fitsAddressSpace(UInt64(v.row_index_dir_len), stride: stride),
              let pageWords = Self.fitsAddressSpace(
                  UInt64(v.row_index_page_count) * UInt64(pageLen), stride: stride),
              rowWords == 0 || v.rows != nil, dirtyCount == 0 || v.dirty_rows != nil,
              dirLen == 0 || v.row_index_dir != nil, pageWords == 0 || v.row_index_pages != nil
        else { return nil }
        self.init(
            key: Key64(lo: v.key_lo, hi: v.key_hi), slot: v.slot, words: Int(v.words), rowCount: rowCount,
            rows: UnsafeBufferPointer(start: rowWords > 0 ? v.rows : nil, count: rowWords),
            owners: UnsafeBufferPointer(start: rowCount > 0 ? v.owners : nil, count: v.owners == nil ? 0 : rowCount),
            dirtyRows: UnsafeBufferPointer(start: dirtyCount > 0 ? v.dirty_rows : nil, count: dirtyCount),
            rowIndexDir: UnsafeBufferPointer(start: dirLen > 0 ? v.row_index_dir : nil, count: dirLen),
            rowIndexPages: UnsafeBufferPointer(start: pageWords > 0 ? v.row_index_pages : nil, count: pageWords)
        )
    }

    public var isSparse: Bool { !owners.isEmpty }

    /// `count` as an `Int` when `count` elements of `stride` bytes fit `limit` bytes (default: this
    /// host's address space). Parameterized so 64-bit CI can check 32-bit limits (`Int32.max`).
    public static func fitsAddressSpace(_ count: UInt64, stride: Int, limit: Int = .max) -> Int? {
        guard stride > 0, limit >= 0, count <= UInt64(limit / stride) else { return nil }
        return Int(count)
    }

    /// Dense-table row of node index `ownerId`, compared as `UInt64` (no `Int` trap on 32-bit hosts).
    public static func denseRow(_ ownerId: UInt32, rowCount: Int) -> Int? {
        UInt64(ownerId) < UInt64(max(rowCount, 0)) ? Int(ownerId) : nil
    }

    /// All words of row `r` (empty when out of range).
    public func row(_ r: Int) -> UnsafeBufferPointer<UInt32> {
        guard (0..<rowCount).contains(r) else { return UnsafeBufferPointer(start: nil, count: 0) }
        return UnsafeBufferPointer(rebasing: rows[r * words..<(r + 1) * words])
    }

    public func word(_ r: Int, _ w: Int) -> UInt32? {
        guard (0..<rowCount).contains(r), (0..<words).contains(w) else { return nil }
        return rows[r * words + w]
    }

    /// Raw field bits of row `r` (without `OptEncoding` decoding); `nil` when out of range.
    public func rawBits(_ r: Int, _ f: FieldSpec) -> UInt32? {
        f.fits(words: words) ? word(r, Int(f.word)).map(f.extract) : nil
    }

    /// `true` when `f` fits row `r` and (if optional) is present.
    public func isPresent(_ r: Int, _ f: FieldSpec) -> Bool {
        if f.kind.isTwoWord { return f.fits(words: words) && (0..<rowCount).contains(r) }
        return bits(r, f) != nil
    }

    /// Decoded field payload bits of row `r` (respecting `f.opt`); `nil` when absent or out of range.
    public func bits(_ r: Int, _ f: FieldSpec) -> UInt32? {
        guard f.fits(words: words), let w = word(r, Int(f.word)) else { return nil }
        let vw = f.opt == .wideBit ? (word(r, Int(f.valWord)) ?? 0) : 0
        return f.decodeBits(word: w, valWord: vw)
    }

    public func u32(_ r: Int, _ f: FieldSpec) -> UInt32? { bits(r, f) }
    public func enumValue(_ r: Int, _ f: FieldSpec) -> UInt32? { bits(r, f) }
    public func resource(_ r: Int, _ f: FieldSpec) -> UInt32? { bits(r, f) }
    public func node(_ r: Int, _ f: FieldSpec) -> UInt32? { bits(r, f) }

    public func i32(_ r: Int, _ f: FieldSpec) -> Int32? {
        guard f.fits(words: words), let w = word(r, Int(f.word)) else { return nil }
        let vw = f.opt == .wideBit ? (word(r, Int(f.valWord)) ?? 0) : 0
        return f.decodeSigned(word: w, valWord: vw)
    }

    public func f32(_ r: Int, _ f: FieldSpec) -> Float? { bits(r, f).map(Float.init(bitPattern:)) }

    public func bool(_ r: Int, _ f: FieldSpec) -> Bool? {
        guard f.fits(words: words), let w = word(r, Int(f.word)) else { return nil }
        let vw = f.opt == .wideBit ? (word(r, Int(f.valWord)) ?? 0) : 0
        return f.decodeBool(word: w, valWord: vw)
    }

    public func color(_ r: Int, _ f: FieldSpec) -> Rgba? { bits(r, f).map(Rgba.init(packed:)) }
    public func length(_ r: Int, _ f: FieldSpec) -> LengthValue? { bits(r, f).map(LengthValue.init(word:)) }

    public func gridLine(_ r: Int, _ f: FieldSpec) -> GridLineValue? {
        guard f.kind == .gridLine else { return nil }
        return bits(r, f).map(GridLineValue.init(word:))
    }

    /// Two-word unsigned 64-bit integer (`lo`, `hi`); `nil` for non-64-bit specs or out of range.
    public func u64(_ r: Int, _ f: FieldSpec) -> UInt64? {
        guard f.kind == .u64 || f.kind == .i64 || f.kind == .f64,
              f.fits(words: words),
              let lo = word(r, Int(f.word)),
              let hi = word(r, Int(f.word) + 1)
        else { return nil }
        return UInt64(lo) | (UInt64(hi) << 32)
    }

    public func i64(_ r: Int, _ f: FieldSpec) -> Int64? { u64(r, f).map(Int64.init(bitPattern:)) }
    public func f64(_ r: Int, _ f: FieldSpec) -> Double? { u64(r, f).map(Double.init(bitPattern:)) }

    /// `{offset, len}` of a `Str` field; `nil` for a non-`Str` spec, past the row, or out of range.
    public func str(_ r: Int, _ f: FieldSpec) -> StrSpan? {
        guard f.kind == .str, f.fits(words: words), let offset = word(r, Int(f.word)), let len = word(r, Int(f.word) + 1) else { return nil }
        return StrSpan(offset: offset, len: len)
    }

    /// Per-table `EventSet<E>` listener bitmask from `eventsWord` (`0` when `eventsWord == noValWord` or out of range).
    public func eventSet(_ r: Int, eventsWord: UInt8) -> UInt32 {
        guard eventsWord != noValWord else { return 0 }
        return word(r, Int(eventsWord)) ?? 0
    }

    /// `true` when listener `bit` (e.g. `NveInteractionEventSet.press`) is subscribed on row `r`.
    public func hasEvent(_ r: Int, eventsWord: UInt8, bit: UInt32) -> Bool {
        (eventSet(r, eventsWord: eventsWord) & bit) != 0
    }

    /// Explicit-override bitmask (word 0) of an `#[nve_table(inherited)]` row (`0` when out of range).
    public func explicitMask(_ r: Int) -> UInt32 { word(r, 0) ?? 0 }

    /// `true` when `f` was explicitly set on row `r` (always `true` for non-inherited fields on valid rows).
    public func isExplicit(_ r: Int, _ f: FieldSpec) -> Bool {
        guard (0..<rowCount).contains(r), f.fits(words: words) else { return false }
        guard f.explicitBit < 32 else { return true }
        return (explicitMask(r) & f.explicitMask) != 0
    }

    /// Raw `NodeId` owning sparse row `r`, or `r` itself for dense tables.
    public func owner(_ r: Int) -> UInt32? {
        guard (0..<rowCount).contains(r) else { return nil }
        return owners.isEmpty ? UInt32(r) : owners[r]
    }

    /// Row of node slot `index` in O(1): the slot itself for dense tables, else the published
    /// paged row index (no generation check; see `row(of:)`).
    public func row(ofNode index: Int) -> Int? {
        guard index >= 0 else { return nil }
        if owners.isEmpty { return index < rowCount ? index : nil }
        let dirSlot = index / pageLen
        guard dirSlot < rowIndexDir.count else { return nil }
        let page = UInt64(rowIndexDir[dirSlot])  // `UInt32.max` (no page) fails the bound below
        guard page < UInt64(rowIndexPages.count / pageLen) else { return nil }
        let rowPlusOne = rowIndexPages[Int(page) * pageLen + index % pageLen]
        guard rowPlusOne != 0, UInt64(rowPlusOne) <= UInt64(rowCount) else { return nil }
        return Int(rowPlusOne - 1)
    }

    /// Row of `ownerId` (raw `NodeId` for sparse tables, node index for dense ones) in O(1);
    /// sparse rows must be owned by exactly that id (a recycled slot's stale id resolves to `nil`).
    public func row(of ownerId: UInt32) -> Int? {
        // Compared as `UInt64`: `Int(ownerId)` would trap on 32-bit hosts for ids >= 2^31.
        if owners.isEmpty { return Self.denseRow(ownerId, rowCount: rowCount) }
        guard let r = row(ofNode: NodeId(raw: ownerId).index), owners[r] == ownerId else { return nil }
        return r
    }
}
