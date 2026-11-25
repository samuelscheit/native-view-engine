#if canImport(CNveAbi)
import CNveAbi
#endif
#if canImport(CoreFoundation)
import CoreFoundation
#endif
import Foundation

// MARK: - Zero-Copy Typed Direct Props Protocol & Builder

/// Strongly typed Swift prop structs conform to `NveDirectEncodable` to write directly into
/// `NveDirectArena` using compile-time `StaticString` keys with zero `[String: Any]` boxing,
/// zero `NveBinValue` heap-tree construction, and zero key sorting.
public protocol NveDirectEncodable {
    /// Exact number of top-level prop entries written by ``encodeDirect(into:)``.
    var directPropCount: Int { get }

    /// Writes each prop field directly into `builder` using `StaticString` keys.
    func encodeDirect(into builder: inout NveDirectPropsBuilder) throws
}

/// Arena-backed builder populating contiguous `NveMapEntry` slots for `set_props_direct`.
public struct NveDirectPropsBuilder {
    public let arena: NveDirectArena
    private let buffer: UnsafeMutablePointer<NveMapEntry>?
    private let capacity: Int
    public private(set) var count: Int = 0

    internal init(arena: NveDirectArena, capacity: Int) {
        self.arena = arena
        self.capacity = capacity
        self.buffer = capacity > 0 ? arena.allocateTyped(NveMapEntry.self, count: capacity) : nil
    }

    internal var baseAddress: UnsafePointer<NveMapEntry>? {
        buffer.map { UnsafePointer($0) }
    }

    @inline(__always)
    public mutating func write(_ key: StaticString, value: NveValue) {
        precondition(count < capacity, "NveDirectPropsBuilder exceeded declared directPropCount")
        let keySlice = arena.staticStringSlice(key)
        buffer!.advanced(by: count).initialize(to: NveMapEntry(key: keySlice, value: value))
        count += 1
    }

    @inline(__always)
    public mutating func writeNull(_ key: StaticString) {
        write(key, value: NveDirectArena.nullValue())
    }

    @inline(__always)
    public mutating func write(_ key: StaticString, _ value: Bool) {
        write(key, value: NveDirectArena.boolValue(value))
    }

    @inline(__always)
    public mutating func write(_ key: StaticString, _ value: Bool?) {
        if let v = value { write(key, v) } else { writeNull(key) }
    }

    @inline(__always)
    public mutating func write<I: SignedInteger>(_ key: StaticString, _ value: I) {
        write(key, value: NveDirectArena.int64Value(Int64(value)))
    }

    @inline(__always)
    public mutating func write<I: SignedInteger>(_ key: StaticString, _ value: I?) {
        if let v = value { write(key, v) } else { writeNull(key) }
    }

    @inline(__always)
    public mutating func write<U: UnsignedInteger>(_ key: StaticString, _ value: U) {
        write(key, value: NveDirectArena.int64Value(Int64(bitPattern: UInt64(value))))
    }

    @inline(__always)
    public mutating func write<U: UnsignedInteger>(_ key: StaticString, _ value: U?) {
        if let v = value { write(key, v) } else { writeNull(key) }
    }

    @inline(__always)
    public mutating func write<F: BinaryFloatingPoint>(_ key: StaticString, _ value: F) {
        write(key, value: NveDirectArena.float64Value(Double(value)))
    }

    @inline(__always)
    public mutating func write<F: BinaryFloatingPoint>(_ key: StaticString, _ value: F?) {
        if let v = value { write(key, v) } else { writeNull(key) }
    }

    @inline(__always)
    public mutating func write(_ key: StaticString, _ value: String) {
        write(key, value: NveDirectArena.stringValue(arena.copyString(value)))
    }

    @inline(__always)
    public mutating func write(_ key: StaticString, _ value: String?) {
        if let v = value { write(key, v) } else { writeNull(key) }
    }

    @inline(__always)
    public mutating func write(_ key: StaticString, _ value: [UInt8]) {
        write(key, value: NveDirectArena.bytesValue(arena.copyBytes(value)))
    }

    @inline(__always)
    public mutating func write(_ key: StaticString, _ value: [UInt8]?) {
        if let v = value { write(key, v) } else { writeNull(key) }
    }

    @inline(__always)
    public mutating func write(_ key: StaticString, _ value: Data) {
        write(key, value: NveDirectArena.bytesValue(arena.copyBytes(value)))
    }

    @inline(__always)
    public mutating func write(_ key: StaticString, _ value: Data?) {
        if let v = value { write(key, v) } else { writeNull(key) }
    }

    @inline(__always)
    public mutating func write<E: RawRepresentable>(_ key: StaticString, enum value: E)
    where E.RawValue == UInt32 {
        write(key, value.rawValue)
    }

    @inline(__always)
    public mutating func write<E: RawRepresentable>(_ key: StaticString, enum value: E?)
    where E.RawValue == UInt32 {
        if let v = value { write(key, v.rawValue) } else { writeNull(key) }
    }

    public mutating func write<P: NveDirectEncodable>(_ key: StaticString, nested value: P) throws {
        var nestedBuilder = NveDirectPropsBuilder(arena: arena, capacity: value.directPropCount)
        try value.encodeDirect(into: &nestedBuilder)
        write(key, value: NveDirectArena.mapValue(ptr: nestedBuilder.baseAddress, len: nestedBuilder.count))
    }
}

// MARK: - Dynamic `Any` -> `NveBinValue` Conversion (Strictly for Untyped Plugins)

extension NveBinValue {
    /// Converts a dynamic Swift value into a typed `NveBinValue` with binary FFI overhead.
    public static func from(any value: Any, depth: Int = 0) throws -> NveBinValue {
        if depth > nveBinValueMaxDepth { throw NveDirectFfiError.maxDepthExceeded }
        switch value {
        case let existing as NveBinValue: return existing
        case is NSNull: return .null
        case let boolVal as Bool where type(of: value) == Bool.self: return .bool(boolVal)
        #if canImport(CoreFoundation)
        case let num as NSNumber:
            if CFGetTypeID(num) == CFBooleanGetTypeID() { return .bool(num.boolValue) }
            return CFNumberIsFloatType(num) ? .float64(num.doubleValue) : .int64(num.int64Value)
        #endif
        case let i as any SignedInteger: return .int64(Int64(i))
        case let u as any UnsignedInteger: return .int64(Int64(bitPattern: UInt64(u)))
        case let f as any BinaryFloatingPoint: return .float64(Double(f))
        case let s as String: return .string(s)
        case let data as Data: return .bytes(data)
        case let array as [NveBinValue]:
            guard depth < nveBinValueMaxDepth else { throw NveDirectFfiError.maxDepthExceeded }
            return .array(array)
        case let array as [Any]:
            guard depth < nveBinValueMaxDepth else { throw NveDirectFfiError.maxDepthExceeded }
            return .array(try array.map { try from(any: $0, depth: depth + 1) })
        case let map as [String: NveBinValue]:
            guard depth < nveBinValueMaxDepth else { throw NveDirectFfiError.maxDepthExceeded }
            return .map(map.keys.sorted().map { ($0, map[$0]!) })
        case let map as [String: Any]:
            guard depth < nveBinValueMaxDepth else { throw NveDirectFfiError.maxDepthExceeded }
            return .map(try map.keys.sorted().map { ($0, try from(any: map[$0]!, depth: depth + 1)) })
        default:
            throw NveDirectFfiError.unsupportedValueType(String(describing: type(of: value)))
        }
    }

    /// Converts a `[String: Any]` prop map into deterministically key-sorted `[(String, NveBinValue)]`.
    public static func canonicalEntries(from props: [String: Any]) throws -> [(String, NveBinValue)] {
        try props.keys.sorted().map { ($0, try from(any: props[$0]!, depth: 1)) }
    }
}

// MARK: - Arena Value & Entry Encoding Helpers

extension NveDirectArena {
    public func writeValue(_ value: NveBinValue, depth: Int = 0) throws -> NveValue {
        if depth > nveBinValueMaxDepth { throw NveDirectFfiError.maxDepthExceeded }
        switch value {
        case .null: return Self.nullValue()
        case let .bool(v): return Self.boolValue(v)
        case let .int64(v): return Self.int64Value(v)
        case let .float64(v): return Self.float64Value(v)
        case let .string(s): return Self.stringValue(copyString(s))
        case let .bytes(d): return Self.bytesValue(copyBytes(d))
        case let .array(items):
            guard depth < nveBinValueMaxDepth else { throw NveDirectFfiError.maxDepthExceeded }
            guard !items.isEmpty, let buf = allocateTyped(NveValue.self, count: items.count) else {
                return Self.arrayValue(ptr: nil, len: 0)
            }
            for (idx, item) in items.enumerated() {
                buf.advanced(by: idx).initialize(to: try writeValue(item, depth: depth + 1))
            }
            return Self.arrayValue(ptr: UnsafePointer(buf), len: items.count)
        case let .map(entries):
            guard depth < nveBinValueMaxDepth else { throw NveDirectFfiError.maxDepthExceeded }
            let (ptr, count) = try writeEntries(entries, depth: depth + 1)
            return Self.mapValue(ptr: ptr, len: Int(count))
        }
    }

    /// Writes a validated zero-copy `NveBinValueView` directly into `NveValue` structs without
    /// building an intermediate `NveBinValue` Swift enum tree.
    public func writeBinValueView(_ view: NveBinValueView, depth: Int = 0) throws -> NveValue {
        if depth > nveBinValueMaxDepth { throw NveDirectFfiError.maxDepthExceeded }
        switch view.tag {
        case .null: return Self.nullValue()
        case .bool: return Self.boolValue(view.boolValue ?? false)
        case .int64: return Self.int64Value(view.int64Value ?? 0)
        case .float64: return Self.float64Value(view.doubleValue ?? 0)
        case .string:
            let slice = view.utf8Bytes.map { copyRawBuffer($0) } ?? NveBufferSlice(ptr: nil, len: 0)
            return Self.stringValue(NveStrSlice(ptr: slice.ptr, len: slice.len))
        case .bytes:
            return Self.bytesValue(view.bytesValue.map { copyRawBuffer($0) } ?? NveBufferSlice(ptr: nil, len: 0))
        case .array:
            guard depth < nveBinValueMaxDepth else { throw NveDirectFfiError.maxDepthExceeded }
            let count = view.arrayCount ?? 0
            guard count > 0, let buf = allocateTyped(NveValue.self, count: count) else {
                return Self.arrayValue(ptr: nil, len: 0)
            }
            for i in 0 ..< count {
                let elem = try view.arrayElement(at: i).map {
                    try writeBinValueView($0, depth: depth + 1)
                } ?? Self.nullValue()
                buf.advanced(by: i).initialize(to: elem)
            }
            return Self.arrayValue(ptr: UnsafePointer(buf), len: count)
        case .map:
            guard depth < nveBinValueMaxDepth else { throw NveDirectFfiError.maxDepthExceeded }
            let count = view.mapCount ?? 0
            guard count > 0, let buf = allocateTyped(NveMapEntry.self, count: count) else {
                return Self.mapValue(ptr: nil, len: 0)
            }
            var idx = 0
            try view.forEachMapEntry { key, val in
                let entry = NveMapEntry(key: copyString(key), value: try writeBinValueView(val, depth: depth + 1))
                buf.advanced(by: idx).initialize(to: entry)
                idx += 1
            }
            return Self.mapValue(ptr: UnsafePointer(buf), len: idx)
        }
    }

    public func writeEntries(
        _ entries: [(String, NveBinValue)],
        depth: Int = 0
    ) throws -> (UnsafePointer<NveMapEntry>?, UInt32) {
        guard !entries.isEmpty, let buf = allocateTyped(NveMapEntry.self, count: entries.count) else {
            return (nil, 0)
        }
        for (idx, (key, val)) in entries.enumerated() {
            let entry = NveMapEntry(key: copyString(key), value: try writeValue(val, depth: depth))
            buf.advanced(by: idx).initialize(to: entry)
        }
        return (UnsafePointer(buf), UInt32(entries.count))
    }
}
