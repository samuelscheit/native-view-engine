import Foundation

/// Maximum nesting depth permitted by the binary `NveBinValue` wire format
/// (matches `NVE_BIN_VALUE_MAX_DEPTH` in `crates/nve-abi/src/value.rs`).
public let nveBinValueMaxDepth = 64

/// Binary `NveBinValue` wire tags `0..7` matching `crates/nve-abi/src/value.rs`.
public enum NveBinValueTag: UInt8, Equatable, Sendable {
    case null = 0
    case bool = 1
    case int64 = 2
    case float64 = 3
    case string = 4
    case bytes = 5
    case array = 6
    case map = 7
}

/// Strict validation errors thrown while parsing a binary `NveBinValue` buffer.
public enum NveBinValueError: Error, Equatable, Sendable {
    case truncated
    case invalidTag(UInt8)
    case invalidBool(UInt8)
    case invalidUtf8
    case maxDepthExceeded
    case trailingBytes(Int)
}

/// Zero-copy borrowed reader over a validated compact binary `NveBinValue` byte buffer.
///
/// Supports O(1) subtree skipping via length-prefixed `Array` (tag 6) and `Map` (tag 7)
/// containers without intermediate JSON parsing or heap allocation.
public struct NveBinValueView {
    public let bytes: UnsafeRawBufferPointer
    public let tag: NveBinValueTag

    private init(validatedBytes: UnsafeRawBufferPointer, tag: NveBinValueTag) {
        self.bytes = validatedBytes
        self.tag = tag
    }

    /// Validates `buffer` once (bounds, UTF-8, bool range, depth <= 64, no trailing bytes) and
    /// returns a zero-copy view over `buffer`.
    public init(validating buffer: UnsafeRawBufferPointer) throws {
        let (tag, end) = try Self.validateValue(in: buffer, start: 0, depth: 0)
        if end != buffer.count {
            throw NveBinValueError.trailingBytes(buffer.count - end)
        }
        self.bytes = buffer
        self.tag = tag
    }

    /// Convenience helper scoping a zero-copy ``NveBinValueView`` over `Data` with no buffer copy.
    public static func withParsed<R>(
        _ data: Data,
        _ body: (NveBinValueView) throws -> R
    ) throws -> R {
        try data.withUnsafeBytes { raw in
            let view = try NveBinValueView(validating: raw)
            return try body(view)
        }
    }

    @inlinable
    public var isNull: Bool { tag == .null }

    @inlinable
    public var boolValue: Bool? {
        guard tag == .bool else { return nil }
        return bytes[1] != 0
    }

    @inlinable
    public var int64Value: Int64? {
        guard tag == .int64 else { return nil }
        return Int64(littleEndian: bytes.loadUnaligned(fromByteOffset: 1, as: Int64.self))
    }

    @inlinable
    public var doubleValue: Double? {
        guard tag == .float64 else { return nil }
        let bits = UInt64(littleEndian: bytes.loadUnaligned(fromByteOffset: 1, as: UInt64.self))
        return Double(bitPattern: bits)
    }

    @inlinable
    public var numberValue: Double? {
        switch tag {
        case .float64: return doubleValue
        case .int64: return int64Value.map(Double.init)
        default: return nil
        }
    }

    /// Zero-copy raw UTF-8 slice for `.string` values.
    public var utf8Bytes: UnsafeRawBufferPointer? {
        guard tag == .string else { return nil }
        let len = Int(Self.readU32LEUnchecked(bytes, at: 1))
        return UnsafeRawBufferPointer(rebasing: bytes[5 ..< (5 + len)])
    }

    public var stringValue: String? {
        guard let slice = utf8Bytes else { return nil }
        return String(bytes: slice, encoding: .utf8)
    }

    /// Zero-copy raw byte slice for `.bytes` values.
    public var bytesValue: UnsafeRawBufferPointer? {
        guard tag == .bytes else { return nil }
        let len = Int(Self.readU32LEUnchecked(bytes, at: 1))
        return UnsafeRawBufferPointer(rebasing: bytes[5 ..< (5 + len)])
    }

    public var arrayCount: Int? {
        guard tag == .array else { return nil }
        return Int(Self.readU32LEUnchecked(bytes, at: 5))
    }

    /// O(index) single-pass skip to element `index` (nested containers skipped in O(1)).
    public func arrayElement(at index: Int) -> NveBinValueView? {
        guard let count = arrayCount, index >= 0, index < count else { return nil }
        var pos = 9
        for i in 0 ... index {
            guard let (elemTag, end) = Self.skipValidatedValue(in: bytes, at: pos) else {
                return nil
            }
            if i == index {
                return NveBinValueView(
                    validatedBytes: UnsafeRawBufferPointer(rebasing: bytes[pos ..< end]),
                    tag: elemTag
                )
            }
            pos = end
        }
        return nil
    }

    public var mapCount: Int? {
        guard tag == .map else { return nil }
        return Int(Self.readU32LEUnchecked(bytes, at: 5))
    }

    /// Zero-allocation map lookup by UTF-8 key, skipping non-matching values in O(1).
    public func mapValue(forKey key: String) -> NveBinValueView? {
        guard let count = mapCount else { return nil }
        let keyUtf8 = Array(key.utf8)
        var pos = 9
        for _ in 0 ..< count {
            let klen = Int(Self.readU32LEUnchecked(bytes, at: pos))
            pos += 4
            let kend = pos + klen
            let matches: Bool
            if klen == keyUtf8.count {
                var eq = true
                for i in 0 ..< klen where bytes[pos + i] != keyUtf8[i] {
                    eq = false
                    break
                }
                matches = eq
            } else {
                matches = false
            }
            pos = kend
            guard let (valTag, valEnd) = Self.skipValidatedValue(in: bytes, at: pos) else {
                return nil
            }
            if matches {
                return NveBinValueView(
                    validatedBytes: UnsafeRawBufferPointer(rebasing: bytes[pos ..< valEnd]),
                    tag: valTag
                )
            }
            pos = valEnd
        }
        return nil
    }

    public func forEachMapEntry(_ body: (String, NveBinValueView) throws -> Void) rethrows {
        guard let count = mapCount else { return }
        var pos = 9
        for _ in 0 ..< count {
            let klen = Int(Self.readU32LEUnchecked(bytes, at: pos))
            pos += 4
            let kend = pos + klen
            let keySlice = UnsafeRawBufferPointer(rebasing: bytes[pos ..< kend])
            let key = String(bytes: keySlice, encoding: .utf8) ?? ""
            pos = kend
            guard let (valTag, valEnd) = Self.skipValidatedValue(in: bytes, at: pos) else {
                return
            }
            let valView = NveBinValueView(
                validatedBytes: UnsafeRawBufferPointer(rebasing: bytes[pos ..< valEnd]),
                tag: valTag
            )
            try body(key, valView)
            pos = valEnd
        }
    }

    // MARK: - Private validation & O(1) skipping

    @inline(__always)
    private static func readU32LE(_ buffer: UnsafeRawBufferPointer, at pos: Int) throws -> UInt32 {
        let (end, overflow) = pos.addingReportingOverflow(4)
        guard !overflow, pos >= 0, end <= buffer.count else {
            throw NveBinValueError.truncated
        }
        return UInt32(littleEndian: buffer.loadUnaligned(fromByteOffset: pos, as: UInt32.self))
    }

    @inline(__always)
    private static func readU32LEUnchecked(_ buffer: UnsafeRawBufferPointer, at pos: Int) -> UInt32 {
        UInt32(littleEndian: buffer.loadUnaligned(fromByteOffset: pos, as: UInt32.self))
    }

    private static func validateUtf8(_ buffer: UnsafeRawBufferPointer, start: Int, end: Int) throws {
        if start == end {
            return
        }
        let slice = UnsafeRawBufferPointer(rebasing: buffer[start ..< end])
        var iter = slice.makeIterator()
        var codec = UTF8()
        while true {
            switch codec.decode(&iter) {
            case .scalarValue:
                continue
            case .emptyInput:
                return
            case .error:
                throw NveBinValueError.invalidUtf8
            }
        }
    }

    private static func validateValue(
        in buffer: UnsafeRawBufferPointer,
        start: Int,
        depth: Int
    ) throws -> (NveBinValueTag, Int) {
        guard start < buffer.count else {
            throw NveBinValueError.truncated
        }
        let rawTag = buffer[start]
        guard let tag = NveBinValueTag(rawValue: rawTag) else {
            throw NveBinValueError.invalidTag(rawTag)
        }
        var pos = start + 1

        switch tag {
        case .null:
            return (tag, pos)

        case .bool:
            guard pos < buffer.count else { throw NveBinValueError.truncated }
            let b = buffer[pos]
            guard b <= 1 else { throw NveBinValueError.invalidBool(b) }
            return (tag, pos + 1)

        case .int64, .float64:
            let (end, overflow) = pos.addingReportingOverflow(8)
            guard !overflow, end <= buffer.count else { throw NveBinValueError.truncated }
            return (tag, end)

        case .string:
            let len = Int(try readU32LE(buffer, at: pos))
            pos += 4
            let (end, overflow) = pos.addingReportingOverflow(len)
            guard !overflow, end <= buffer.count else { throw NveBinValueError.truncated }
            try validateUtf8(buffer, start: pos, end: end)
            return (tag, end)

        case .bytes:
            let len = Int(try readU32LE(buffer, at: pos))
            pos += 4
            let (end, overflow) = pos.addingReportingOverflow(len)
            guard !overflow, end <= buffer.count else { throw NveBinValueError.truncated }
            return (tag, end)

        case .array:
            if depth >= nveBinValueMaxDepth {
                throw NveBinValueError.maxDepthExceeded
            }
            let payloadLen = Int(try readU32LE(buffer, at: pos))
            pos += 4
            let (containerEnd, overflow) = pos.addingReportingOverflow(payloadLen)
            guard !overflow, containerEnd <= buffer.count, payloadLen >= 4 else {
                throw NveBinValueError.truncated
            }
            let count = Int(try readU32LE(buffer, at: pos))
            pos += 4
            if count > containerEnd - pos {
                throw NveBinValueError.truncated
            }
            let bounded = UnsafeRawBufferPointer(rebasing: buffer[0 ..< containerEnd])
            for _ in 0 ..< count {
                let (_, nextPos) = try validateValue(in: bounded, start: pos, depth: depth + 1)
                pos = nextPos
            }
            if pos != containerEnd {
                throw NveBinValueError.trailingBytes(containerEnd - pos)
            }
            return (tag, containerEnd)

        case .map:
            if depth >= nveBinValueMaxDepth {
                throw NveBinValueError.maxDepthExceeded
            }
            let payloadLen = Int(try readU32LE(buffer, at: pos))
            pos += 4
            let (containerEnd, overflow) = pos.addingReportingOverflow(payloadLen)
            guard !overflow, containerEnd <= buffer.count, payloadLen >= 4 else {
                throw NveBinValueError.truncated
            }
            let count = Int(try readU32LE(buffer, at: pos))
            pos += 4
            if count > (containerEnd - pos) / 5 {
                throw NveBinValueError.truncated
            }
            let bounded = UnsafeRawBufferPointer(rebasing: buffer[0 ..< containerEnd])
            for _ in 0 ..< count {
                let klen = Int(try readU32LE(bounded, at: pos))
                pos += 4
                let (kend, kOverflow) = pos.addingReportingOverflow(klen)
                guard !kOverflow, kend <= containerEnd else { throw NveBinValueError.truncated }
                try validateUtf8(bounded, start: pos, end: kend)
                pos = kend
                let (_, nextPos) = try validateValue(in: bounded, start: pos, depth: depth + 1)
                pos = nextPos
            }
            if pos != containerEnd {
                throw NveBinValueError.trailingBytes(containerEnd - pos)
            }
            return (tag, containerEnd)
        }
    }

    private static func skipValidatedValue(
        in buffer: UnsafeRawBufferPointer,
        at start: Int
    ) -> (NveBinValueTag, Int)? {
        guard start < buffer.count, let tag = NveBinValueTag(rawValue: buffer[start]) else {
            return nil
        }
        let pos = start + 1
        let end: Int
        switch tag {
        case .null:
            end = pos
        case .bool:
            end = pos + 1
        case .int64, .float64:
            end = pos + 8
        case .string, .bytes, .array, .map:
            let len = Int(readU32LEUnchecked(buffer, at: pos))
            end = pos + 4 + len
        }
        return end <= buffer.count ? (tag, end) : nil
    }
}

/// Owned Swift value enum with compact binary `NveBinValue` encoding support.
public enum NveBinValue: Equatable, Sendable {
    case null
    case bool(Bool)
    case int64(Int64)
    case float64(Double)
    case string(String)
    case bytes(Data)
    case array([NveBinValue])
    case map([(String, NveBinValue)])

    public static func == (lhs: NveBinValue, rhs: NveBinValue) -> Bool {
        switch (lhs, rhs) {
        case (.null, .null): return true
        case let (.bool(a), .bool(b)): return a == b
        case let (.int64(a), .int64(b)): return a == b
        case let (.float64(a), .float64(b)): return a.bitPattern == b.bitPattern
        case let (.string(a), .string(b)): return a == b
        case let (.bytes(a), .bytes(b)): return a == b
        case let (.array(a), .array(b)): return a == b
        case let (.map(a), .map(b)):
            guard a.count == b.count else { return false }
            return zip(a, b).allSatisfy { $0.0 == $1.0 && $0.1 == $1.1 }
        default:
            return false
        }
    }

    /// Encodes this value into the compact binary `NveBinValue` wire format.
    public func encode() throws -> Data {
        var out = Data()
        try encode(into: &out, depth: 0)
        return out
    }

    private func encode(into out: inout Data, depth: Int) throws {
        switch self {
        case .null:
            out.append(NveBinValueTag.null.rawValue)
        case let .bool(v):
            out.append(NveBinValueTag.bool.rawValue)
            out.append(v ? 1 : 0)
        case let .int64(v):
            out.append(NveBinValueTag.int64.rawValue)
            var le = v.littleEndian
            withUnsafeBytes(of: &le) { out.append(contentsOf: $0) }
        case let .float64(v):
            out.append(NveBinValueTag.float64.rawValue)
            var le = v.bitPattern.littleEndian
            withUnsafeBytes(of: &le) { out.append(contentsOf: $0) }
        case let .string(s):
            let utf8 = Array(s.utf8)
            out.append(NveBinValueTag.string.rawValue)
            Self.appendU32LE(UInt32(utf8.count), to: &out)
            out.append(contentsOf: utf8)
        case let .bytes(d):
            out.append(NveBinValueTag.bytes.rawValue)
            Self.appendU32LE(UInt32(d.count), to: &out)
            out.append(d)
        case let .array(items):
            if depth >= nveBinValueMaxDepth {
                throw NveBinValueError.maxDepthExceeded
            }
            out.append(NveBinValueTag.array.rawValue)
            let lenPos = out.count
            Self.appendU32LE(0, to: &out)
            let payloadStart = out.count
            Self.appendU32LE(UInt32(items.count), to: &out)
            for item in items {
                try item.encode(into: &out, depth: depth + 1)
            }
            Self.patchU32LE(UInt32(out.count - payloadStart), at: lenPos, in: &out)
        case let .map(entries):
            if depth >= nveBinValueMaxDepth {
                throw NveBinValueError.maxDepthExceeded
            }
            out.append(NveBinValueTag.map.rawValue)
            let lenPos = out.count
            Self.appendU32LE(0, to: &out)
            let payloadStart = out.count
            Self.appendU32LE(UInt32(entries.count), to: &out)
            for (k, v) in entries {
                let kb = Array(k.utf8)
                Self.appendU32LE(UInt32(kb.count), to: &out)
                out.append(contentsOf: kb)
                try v.encode(into: &out, depth: depth + 1)
            }
            Self.patchU32LE(UInt32(out.count - payloadStart), at: lenPos, in: &out)
        }
    }

    private static func appendU32LE(_ v: UInt32, to out: inout Data) {
        var le = v.littleEndian
        withUnsafeBytes(of: &le) { out.append(contentsOf: $0) }
    }

    private static func patchU32LE(_ v: UInt32, at offset: Int, in out: inout Data) {
        var le = v.littleEndian
        withUnsafeBytes(of: &le) { raw in
            out.replaceSubrange(offset ..< (offset + 4), with: raw)
        }
    }
}
