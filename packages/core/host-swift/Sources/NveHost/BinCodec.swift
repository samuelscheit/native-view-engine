/// Bounds-checked little-endian `BinReader` / `BinWriter` plus the compact tagged `NveBinValue`
/// format (byte-identical to Rust `nve_abi::encode_bin_value`).
///
/// Wire layout (little-endian): Null `[0]` | Bool `[1, 0|1]` | I64 `[2, i64]` | F64 `[3, f64]` |
/// Str `[4, len u32, utf8]` | Bytes `[5, len u32, raw]` | Array `[6, payload u32, count u32, ...]` |
/// Map `[7, payload u32, count u32, (klen u32, key, value)...]`; `payload` counts every byte after
/// the payload field so containers skip in O(1).

/// `NveBinValue` tag (Rust `nve_abi::NveValueTag`).
public enum BinTag: UInt8, Sendable {
    case null = 0, bool = 1, i64 = 2, f64 = 3, str = 4, bytes = 5, array = 6, map = 7
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
    /// `BinWriter.end` got a mark that is not an open container in this buffer (e.g. after `clear()`).
    case invalidMark
}

/// Zero-copy cursor over `bytes`; slices share the array's storage.
///
/// Every length is compared as `UInt64` against the remaining bytes before it becomes an `Int`, so
/// no wire value can trap on 32-bit hosts (`arm64_32` watchOS).
public struct BinReader: Sendable {
    public let bytes: [UInt8]
    public private(set) var position = 0
    /// Reads stop here: `bytes.count`, or the enclosing container's end while validating.
    private var limit: Int

    public init(_ bytes: [UInt8]) {
        self.bytes = bytes
        limit = bytes.count
    }

    public var remaining: Int { limit - position }
    public var isAtEnd: Bool { position == limit }

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
        let at = try advance(count)
        return bytes[at..<at + count]
    }

    /// Next `count` bytes decoded as strictly validated UTF-8 (a leading U+FEFF is kept).
    public mutating func utf8(_ count: Int) throws(BinError) -> String {
        let at = try checkUtf8(count)
        return String(decoding: bytes[at..<at + count], as: UTF8.self)
    }

    /// Consumes `count` bytes and returns their offset (no slice, no allocation).
    private mutating func advance(_ count: Int) throws(BinError) -> Int {
        guard count >= 0, count <= remaining else { throw .truncated }
        defer { position += count }
        return position
    }

    /// Consumes and validates `count` UTF-8 bytes in place over the raw buffer; returns their offset.
    private mutating func checkUtf8(_ count: Int) throws(BinError) -> Int {
        let at = try advance(count)
        let valid = bytes.withUnsafeBytes { Utf8.isValid(UnsafeRawBufferPointer(rebasing: $0[at..<at + count])) }
        guard valid else { throw .invalidUtf8 }
        return at
    }

    // MARK: tagged NveBinValue

    public func peekTag() throws(BinError) -> BinTag {
        guard position < limit else { throw .truncated }
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
        return try utf8(try length())
    }

    public mutating func readBytes() throws(BinError) -> ArraySlice<UInt8> {
        try expect(.bytes)
        return try take(try length())
    }

    /// Element count of an array; rejects counts its payload cannot hold.
    public mutating func beginArray() throws(BinError) -> Int { try openContainer(.array, minEntryBytes: 1).count }

    /// Opens an array that must hold exactly `count` values (a spec `[T; N]`): fewer throw
    /// `.truncated`, more `.tooLarge`.
    public mutating func beginArray(exactly count: Int) throws(BinError) {
        let actual = try beginArray()
        guard actual == count else { throw actual < count ? BinError.truncated : BinError.tooLarge }
    }

    /// Entry count of a map; read each entry as `readMapKey()` then a value.
    public mutating func beginMap() throws(BinError) -> Int { try openContainer(.map, minEntryBytes: 5).count }

    public mutating func readMapKey() throws(BinError) -> String { try utf8(try length()) }

    /// Skips one value in O(1).
    public mutating func skipValue() throws(BinError) {
        let tag = try peekTag()
        position += 1
        switch tag {
        case .null: return
        case .bool: _ = try take(1)
        case .i64, .f64: _ = try take(8)
        case .str, .bytes, .array, .map: _ = try take(try length())
        }
    }

    /// The next complete encoded value, sharing storage.
    public mutating func readRawValue() throws(BinError) -> ArraySlice<UInt8> {
        let start = position
        try skipValue()
        return bytes[start..<position]
    }

    /// Fully validates and consumes the next value (Rust `BinValueView::parse` rules) without
    /// decoding or copying anything.
    public mutating func checkValue() throws(BinError) { try checkValue(depth: 0) }

    /// Throws unless `bytes` holds exactly one valid `NveBinValue`.
    public static func validate(_ bytes: [UInt8]) throws(BinError) {
        var r = BinReader(bytes)
        try r.checkValue()
        guard r.isAtEnd else { throw .trailingBytes(r.remaining) }
    }

    /// A `u32` length that fits the remaining bytes (compared before any `Int` conversion).
    private mutating func length() throws(BinError) -> Int {
        let len = try u32()
        guard UInt64(len) <= UInt64(remaining) else { throw .truncated }
        return Int(len)
    }

    private mutating func le(_ count: Int) throws(BinError) -> UInt64 {
        let at = try advance(count)
        var v: UInt64 = 0
        for k in 0..<count { v |= UInt64(bytes[at + k]) << (8 * UInt64(k)) }
        return v
    }

    private mutating func expect(_ tag: BinTag) throws(BinError) {
        let actual = try peekTag()
        guard actual == tag else { throw .unexpectedTag(expected: tag, actual: actual) }
        position += 1
    }

    private mutating func openContainer(_ tag: BinTag, minEntryBytes: Int) throws(BinError) -> (count: Int, end: Int) {
        try expect(tag)
        let payload = try length()
        guard payload >= 4 else { throw .truncated }
        let end = position + payload
        let count = try u32()
        guard UInt64(count) <= UInt64((payload - 4) / minEntryBytes) else { throw .truncated }
        return (Int(count), end)
    }

    /// Validates a UTF-8 run of `length()` bytes in place.
    private mutating func checkUtf8Run() throws(BinError) { _ = try checkUtf8(try length()) }

    private mutating func checkValue(depth: Int) throws(BinError) {
        switch try peekTag() {
        case .null, .i64, .f64: try skipValue()
        case .bool: _ = try readBool()
        case .str:
            try expect(.str)
            try checkUtf8Run()
        case .bytes: try skipValue()
        case let tag:
            guard depth < binValueMaxDepth else { throw .maxDepth }
            let (count, end) = try openContainer(tag, minEntryBytes: tag == .map ? 5 : 1)
            // Like Rust's `&bytes[..container_end]`: no child read may escape its container.
            let outer = limit
            limit = end
            defer { limit = outer }
            for _ in 0..<count {
                if tag == .map { try checkUtf8Run() }
                try checkValue(depth: depth + 1)
            }
            guard position == end else { throw .trailingBytes(end - position) }
        }
    }
}

/// Little-endian writer; reuse one instance with `clear()` (keeps capacity).
public struct BinWriter: Sendable {
    /// An open container: its payload-length field and the writer generation that opened it.
    public struct Mark: Sendable {
        fileprivate let lengthPosition: Int
        fileprivate let generation: UInt32
    }

    public private(set) var bytes: [UInt8] = []

    public init(capacity: Int = 64) { bytes.reserveCapacity(capacity) }

    /// Bumped by `clear()`, so marks from before it are rejected even once the buffer refills.
    private var generation: UInt32 = 1

    /// Empties the buffer (keeping capacity) and invalidates every outstanding `Mark`.
    public mutating func clear() {
        bytes.removeAll(keepingCapacity: true)
        generation &+= 1
        if generation == 0 { generation = 1 }  // never 0, like every other runtime
    }

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

    /// Swift strings are always valid Unicode (no lone surrogates), so they encode without checks or copies.
    public mutating func writeStr(_ v: String) throws(BinError) {
        guard UInt32(exactly: v.utf8.count) != nil else { throw .tooLarge }  // before any byte
        u8(BinTag.str.rawValue)
        try writeUtf8(v)
    }

    /// Throws `.tooLarge` (writing nothing) when `v` exceeds `u32` bytes.
    public mutating func writeBytes<Bytes: Collection>(_ v: Bytes) throws(BinError) where Bytes.Element == UInt8 {
        guard UInt32(exactly: v.count) != nil else { throw .tooLarge }  // before the tag
        u8(BinTag.bytes.rawValue)
        try sized(v)
    }

    public mutating func writeMapKey(_ key: String) throws(BinError) { try writeUtf8(key) }

    /// Length-prefixed UTF-8 copied straight from the string's contiguous storage (no temporary).
    private mutating func writeUtf8(_ text: String) throws(BinError) {
        var text = text  // `withUTF8` is mutating; native strings are already contiguous (no copy)
        try text.withUTF8 { (utf8) throws(BinError) in try appendSized(UnsafeRawBufferPointer(utf8)) }
    }

    private mutating func appendSized(_ raw: UnsafeRawBufferPointer) throws(BinError) {
        guard let len = UInt32(exactly: raw.count) else { throw .tooLarge }
        u32(len)
        bytes.append(contentsOf: raw)  // one bulk copy (allocation-free once warm; see the release guard)
    }

    /// Opens an array of `count` values; close it with `end(_:)`.
    public mutating func beginArray(_ count: UInt32) -> Mark { begin(.array, count) }

    /// Opens an array of a Swift collection's `count` values; throws `.tooLarge` (writing nothing)
    /// past `u32`.
    public mutating func beginArray(count: Int) throws(BinError) -> Mark {
        guard let n = UInt32(exactly: count) else { throw .tooLarge }
        return beginArray(n)
    }

    /// Opens an array for a fixed-size spec array (`[T; N]`) whose Swift collection holds `count`
    /// values; throws (writing nothing) unless `count == expected`: fewer `.truncated`, more `.tooLarge`.
    public mutating func beginArray(count: Int, exactly expected: Int) throws(BinError) -> Mark {
        guard count == expected else { throw count < expected ? BinError.truncated : BinError.tooLarge }
        return try beginArray(count: count)
    }

    /// Opens a map of `count` entries (`writeMapKey` + value each); close it with `end(_:)`.
    public mutating func beginMap(_ count: UInt32) -> Mark { begin(.map, count) }

    public mutating func end(_ mark: Mark) throws(BinError) {
        // Stale (pre-`clear()`), out of range, or not right after an array / map tag: reject, write nothing.
        // An open container's length field still holds 0; a closed one holds its payload (>= 4).
        guard mark.generation == generation, mark.lengthPosition >= 1, mark.lengthPosition <= bytes.count - 8,
              bytes[mark.lengthPosition - 1] == BinTag.array.rawValue || bytes[mark.lengthPosition - 1] == BinTag.map.rawValue,
              bytes[mark.lengthPosition ..< mark.lengthPosition + 4].allSatisfy({ $0 == 0 })
        else { throw .invalidMark }
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
        let mark = Mark(lengthPosition: bytes.count, generation: generation)
        u32(0)
        u32(count)
        return mark
    }
}

/// Allocation-free strict UTF-8 validation (Rust `core::str::from_utf8` acceptance). The core is
/// non-generic over raw bytes, so it stays allocation-free even unspecialized (debug, cross-module).
public enum Utf8 {
    public static func isValid(_ p: UnsafeRawBufferPointer) -> Bool {
        var i = 0
        let end = p.count
        while i < end {
            let b0 = p[i]
            if b0 < 0x80 {
                i += 1
                continue
            }
            let need: Int
            var lo: UInt8 = 0x80
            var hi: UInt8 = 0xBF
            if b0 >= 0xC2 && b0 <= 0xDF {
                need = 1
            } else if b0 >= 0xE0 && b0 <= 0xEF {
                need = 2
                if b0 == 0xE0 { lo = 0xA0 }
                if b0 == 0xED { hi = 0x9F }
            } else if b0 >= 0xF0 && b0 <= 0xF4 {
                need = 3
                if b0 == 0xF0 { lo = 0x90 }
                if b0 == 0xF4 { hi = 0x8F }
            } else {
                return false
            }
            guard need < end - i, p[i + 1] >= lo, p[i + 1] <= hi else { return false }
            var k = 2
            while k <= need {
                if p[i + k] & 0xC0 != 0x80 { return false }
                k += 1
            }
            i += need + 1
        }
        return true
    }

    /// Any byte collection (contiguous storage is validated in place).
    public static func isValid<Bytes: Collection>(_ bytes: Bytes) -> Bool where Bytes.Element == UInt8 {
        if let valid = bytes.withContiguousStorageIfAvailable({ isValid(UnsafeRawBufferPointer($0)) }) {
            return valid
        }
        return Array(bytes).withUnsafeBytes { isValid($0) }
    }
}
