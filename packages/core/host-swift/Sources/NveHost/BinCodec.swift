/// Bounds-checked little-endian `BinReader` / `BinWriter` plus the compact tagged `NveBinValue`
/// format (byte-identical to Rust `nve_abi::encode_bin_value`).
///
/// Wire layout (little-endian): Null `[0]` | Bool `[1, 0|1]` | I64 `[2, i64]` | F64 `[3, f64]` |
/// Str `[4, len u32, utf8]` | Bytes `[5, len u32, raw]` | Array `[6, payload u32, count u32, ...]` |
/// Map `[7, payload u32, count u32, (klen u32, key, value)...]`; `payload` counts every byte after
/// the payload field so containers skip in O(1).

/// `NveBinValue` tag (Rust `nve_abi::NveValueTag`).
public enum BinTag: UInt8, Sendable {
    case null = 0, bool, i64, f64, str, bytes, array, map
}

/// Maximum container nesting depth (Rust `NVE_BIN_VALUE_MAX_DEPTH`).
public let binValueMaxDepth = 64

public enum BinError: Error, Equatable, Sendable {
    case truncated
    case unexpectedTag(expected: BinTag, actual: BinTag)
    case invalidTag(UInt8)
    case invalidBool(UInt8)
    case invalidUtf8
    case tooLarge
    case maxDepth
    case trailingBytes(Int)
}

/// Zero-copy cursor over `bytes`; slices share the array's storage.
public struct BinReader: Sendable {
    public let bytes: [UInt8]
    public private(set) var position = 0

    public init(_ bytes: [UInt8]) { self.bytes = bytes }

    public var remaining: Int { bytes.count - position }
    public var isAtEnd: Bool { position == bytes.count }

    // MARK: raw little-endian primitives

    public mutating func u8() throws(BinError) -> UInt8 { try take(1).first ?? 0 }
    public mutating func u16() throws(BinError) -> UInt16 { UInt16(truncatingIfNeeded: try le(2)) }
    public mutating func u32() throws(BinError) -> UInt32 { UInt32(truncatingIfNeeded: try le(4)) }
    public mutating func u64() throws(BinError) -> UInt64 { try le(8) }
    public mutating func i32() throws(BinError) -> Int32 { Int32(bitPattern: try u32()) }
    public mutating func i64() throws(BinError) -> Int64 { Int64(bitPattern: try u64()) }
    public mutating func f32() throws(BinError) -> Float { Float(bitPattern: try u32()) }
    public mutating func f64() throws(BinError) -> Double { Double(bitPattern: try u64()) }

    /// Next `count` bytes, sharing storage with `bytes`.
    public mutating func take(_ count: Int) throws(BinError) -> ArraySlice<UInt8> {
        guard count >= 0, count <= remaining else { throw .truncated }
        defer { position += count }
        return bytes[position..<position + count]
    }

    /// Next `count` bytes decoded as strictly validated UTF-8.
    public mutating func utf8(_ count: Int) throws(BinError) -> String {
        let slice = try take(count)
        guard Utf8.isValid(slice) else { throw .invalidUtf8 }
        return String(decoding: slice, as: UTF8.self)
    }

    // MARK: tagged NveBinValue

    public func peekTag() throws(BinError) -> BinTag {
        guard position < bytes.count else { throw .truncated }
        let raw = bytes[position]
        guard let tag = BinTag(rawValue: raw) else { throw .invalidTag(raw) }
        return tag
    }

    /// Consumes a `null` and returns `true`; leaves any other value in place.
    public mutating func readNull() throws(BinError) -> Bool {
        guard try peekTag() == .null else { return false }
        position += 1
        return true
    }

    public mutating func readBool() throws(BinError) -> Bool {
        try expect(.bool)
        let b = try u8()
        guard b <= 1 else { throw .invalidBool(b) }
        return b == 1
    }

    public mutating func readI64() throws(BinError) -> Int64 {
        try expect(.i64)
        return try i64()
    }

    /// `f64`, or an `i64` widened to `Double` (Rust `BinValueView::as_number`).
    public mutating func readF64() throws(BinError) -> Double {
        if try peekTag() == .i64 { return Double(try readI64()) }
        try expect(.f64)
        return try f64()
    }

    public mutating func readStr() throws(BinError) -> String {
        try expect(.str)
        return try utf8(Int(try u32()))
    }

    public mutating func readBytes() throws(BinError) -> ArraySlice<UInt8> {
        try expect(.bytes)
        return try take(Int(try u32()))
    }

    /// Element count of an array; rejects counts its payload cannot hold.
    public mutating func beginArray() throws(BinError) -> Int { try openContainer(.array, minEntryBytes: 1).count }

    /// Entry count of a map; read each entry as `readMapKey()` then a value.
    public mutating func beginMap() throws(BinError) -> Int { try openContainer(.map, minEntryBytes: 5).count }

    public mutating func readMapKey() throws(BinError) -> String { try utf8(Int(try u32())) }

    /// Skips one value in O(1).
    public mutating func skipValue() throws(BinError) {
        let tag = try peekTag()
        position += 1
        switch tag {
        case .null: return
        case .bool: _ = try take(1)
        case .i64, .f64: _ = try take(8)
        case .str, .bytes, .array, .map: _ = try take(Int(try u32()))
        }
    }

    /// The next complete encoded value, sharing storage.
    public mutating func readRawValue() throws(BinError) -> ArraySlice<UInt8> {
        let start = position
        try skipValue()
        return bytes[start..<position]
    }

    /// Fully validates and consumes the next value (Rust `BinValueView::parse` rules).
    public mutating func checkValue() throws(BinError) { try checkValue(depth: 0) }

    /// Throws unless `bytes` holds exactly one valid `NveBinValue`.
    public static func validate(_ bytes: [UInt8]) throws(BinError) {
        var r = BinReader(bytes)
        try r.checkValue()
        guard r.isAtEnd else { throw .trailingBytes(r.remaining) }
    }

    private mutating func le(_ count: Int) throws(BinError) -> UInt64 {
        let slice = try take(count)
        var v: UInt64 = 0
        for (k, b) in slice.enumerated() { v |= UInt64(b) << (8 * UInt64(k)) }
        return v
    }

    private mutating func expect(_ tag: BinTag) throws(BinError) {
        let actual = try peekTag()
        guard actual == tag else { throw .unexpectedTag(expected: tag, actual: actual) }
        position += 1
    }

    private mutating func openContainer(_ tag: BinTag, minEntryBytes: Int) throws(BinError) -> (count: Int, end: Int) {
        try expect(tag)
        let payload = Int(try u32())
        guard payload >= 4, payload <= remaining else { throw .truncated }
        let end = position + payload
        let count = Int(try u32())
        guard count <= (payload - 4) / minEntryBytes else { throw .truncated }
        return (count, end)
    }

    private mutating func checkValue(depth: Int) throws(BinError) {
        switch try peekTag() {
        case .null, .i64, .f64: try skipValue()
        case .bool: _ = try readBool()
        case .str: _ = try readStr()
        case .bytes: _ = try readBytes()
        case let tag:
            guard depth < binValueMaxDepth else { throw .maxDepth }
            let (count, end) = try openContainer(tag, minEntryBytes: tag == .map ? 5 : 1)
            for _ in 0..<count {
                if tag == .map { _ = try readMapKey() }
                try checkValue(depth: depth + 1)
            }
            guard position <= end else { throw .truncated }
            guard position == end else { throw .trailingBytes(end - position) }
        }
    }
}

/// Little-endian writer; reuse one instance with `clear()` (keeps capacity).
public struct BinWriter: Sendable {
    /// Position of an open container's payload-length field.
    public struct Mark: Sendable { fileprivate let lengthPosition: Int }

    public private(set) var bytes: [UInt8] = []

    public init(capacity: Int = 64) { bytes.reserveCapacity(capacity) }

    public mutating func clear() { bytes.removeAll(keepingCapacity: true) }

    // MARK: raw little-endian primitives

    public mutating func u8(_ v: UInt8) { bytes.append(v) }
    public mutating func u16(_ v: UInt16) { le(UInt64(v), 2) }
    public mutating func u32(_ v: UInt32) { le(UInt64(v), 4) }
    public mutating func u64(_ v: UInt64) { le(v, 8) }
    public mutating func i32(_ v: Int32) { u32(UInt32(bitPattern: v)) }
    public mutating func i64(_ v: Int64) { u64(UInt64(bitPattern: v)) }
    public mutating func f32(_ v: Float) { u32(v.bitPattern) }
    public mutating func f64(_ v: Double) { u64(v.bitPattern) }
    public mutating func raw<Bytes: Sequence>(_ b: Bytes) where Bytes.Element == UInt8 { bytes.append(contentsOf: b) }

    // MARK: tagged NveBinValue

    public mutating func writeNull() { u8(BinTag.null.rawValue) }

    public mutating func writeBool(_ v: Bool) {
        u8(BinTag.bool.rawValue)
        u8(v ? 1 : 0)
    }

    public mutating func writeI64(_ v: Int64) {
        u8(BinTag.i64.rawValue)
        i64(v)
    }

    public mutating func writeF64(_ v: Double) {
        u8(BinTag.f64.rawValue)
        f64(v)
    }

    public mutating func writeStr(_ v: String) throws(BinError) {
        u8(BinTag.str.rawValue)
        try sized(Array(v.utf8))
    }

    public mutating func writeBytes<Bytes: Collection>(_ v: Bytes) throws(BinError) where Bytes.Element == UInt8 {
        u8(BinTag.bytes.rawValue)
        try sized(v)
    }

    public mutating func writeMapKey(_ key: String) throws(BinError) { try sized(Array(key.utf8)) }

    /// Opens an array of `count` values; close it with `end(_:)`.
    public mutating func beginArray(_ count: UInt32) -> Mark { begin(.array, count) }

    /// Opens a map of `count` entries (`writeMapKey` + value each); close it with `end(_:)`.
    public mutating func beginMap(_ count: UInt32) -> Mark { begin(.map, count) }

    public mutating func end(_ mark: Mark) throws(BinError) {
        let payload = bytes.count - mark.lengthPosition - 4
        guard let len = UInt32(exactly: payload) else { throw .tooLarge }
        for k in 0..<4 { bytes[mark.lengthPosition + k] = UInt8(truncatingIfNeeded: len >> (8 * UInt32(k))) }
    }

    private mutating func le(_ v: UInt64, _ count: Int) {
        for k in 0..<count { bytes.append(UInt8(truncatingIfNeeded: v >> (8 * UInt64(k)))) }
    }

    private mutating func sized<Bytes: Collection>(_ b: Bytes) throws(BinError) where Bytes.Element == UInt8 {
        guard let len = UInt32(exactly: b.count) else { throw .tooLarge }
        u32(len)
        bytes.append(contentsOf: b)
    }

    private mutating func begin(_ tag: BinTag, _ count: UInt32) -> Mark {
        u8(tag.rawValue)
        let mark = Mark(lengthPosition: bytes.count)
        u32(0)
        u32(count)
        return mark
    }
}

/// Allocation-free strict UTF-8 validation (Rust `core::str::from_utf8` acceptance).
public enum Utf8 {
    public static func isValid<Bytes: Collection>(_ bytes: Bytes) -> Bool where Bytes.Element == UInt8, Bytes.Index == Int {
        var i = bytes.startIndex
        let end = bytes.endIndex
        while i < end {
            let b0 = bytes[i]
            if b0 < 0x80 {
                i += 1
                continue
            }
            var need = 0
            var lo: UInt8 = 0x80
            var hi: UInt8 = 0xBF
            switch b0 {
            case 0xC2...0xDF: need = 1
            case 0xE0...0xEF:
                need = 2
                if b0 == 0xE0 { lo = 0xA0 }
                if b0 == 0xED { hi = 0x9F }
            case 0xF0...0xF4:
                need = 3
                if b0 == 0xF0 { lo = 0x90 }
                if b0 == 0xF4 { hi = 0x8F }
            default: return false
            }
            guard need < end - i, (lo...hi).contains(bytes[i + 1]) else { return false }
            for k in stride(from: 2, through: need, by: 1) where bytes[i + k] & 0xC0 != 0x80 {
                return false
            }
            i += need + 1
        }
        return true
    }
}
