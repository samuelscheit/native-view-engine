#if canImport(CNveAbi)
import CNveAbi
import NveHost
#endif
import Foundation

// MARK: - Zero-Copy Direct C-ABI Props Writer

/// Writes structured Swift props directly into C-ABI `NveMapEntry[]` + `NveValue` buffers
/// backed by `NveDirectArena` for `NveComponentApi.set_props`.
public enum NveDirectPropsWriter {
    /// Zero-boxing typed fast path: encodes `props` directly into a 4 KiB stack-slab bump arena
    /// using `StaticString` keys with zero `[String: Any]` boxing, zero `NveBinValue` tree
    /// allocation, and zero key sorting.
    public static func withEncodedProps<P: NveDirectEncodable, R>(
        _ props: P,
        _ body: (UnsafePointer<NveMapEntry>?, UInt32) throws -> R
    ) throws -> R {
        let capacity = props.directPropCount
        guard capacity > 0 else {
            return try body(nil, 0)
        }
        return try NveDirectArena.withArena { arena in
            var builder = NveDirectPropsBuilder(arena: arena, capacity: capacity)
            try props.encodeDirect(into: &builder)
            return try body(builder.baseAddress, UInt32(builder.count))
        }
    }

    public static func withEntries<R>(
        _ entries: [(String, NveBinValue)],
        _ body: (UnsafePointer<NveMapEntry>?, UInt32) throws -> R
    ) throws -> R {
        try NveDirectArena.withArena { arena in
            let (ptr, count) = try arena.writeEntries(entries)
            return try body(ptr, count)
        }
    }

    public static func withProps<R>(
        _ props: [String: Any],
        _ body: (UnsafePointer<NveMapEntry>?, UInt32) throws -> R
    ) throws -> R {
        let entries = try NveBinValue.canonicalEntries(from: props)
        return try withEntries(entries, body)
    }

    internal static func decodeStrSlice(_ slice: NveStrSlice) -> String {
        guard let p = slice.ptr, slice.len > 0 else { return "" }
        return String(decoding: UnsafeBufferPointer(start: p, count: slice.len), as: UTF8.self)
    }

    internal static func decodeCValue(_ value: NveValue) -> NveBinValue {
        switch value.tag {
        case UInt32(NVE_VAL_BOOL.rawValue): return .bool(value.data.bool_val != 0)
        case UInt32(NVE_VAL_I64.rawValue): return .int64(value.data.i64_val)
        case UInt32(NVE_VAL_F64.rawValue): return .float64(value.data.f64_val)
        case UInt32(NVE_VAL_STR.rawValue): return .string(decodeStrSlice(value.data.str_val))
        case UInt32(NVE_VAL_BYTES.rawValue):
            let b = value.data.bytes_val
            return .bytes(b.ptr.map { Data(bytes: $0, count: b.len) } ?? Data())
        case UInt32(NVE_VAL_ARRAY.rawValue):
            let a = value.data.array_val
            guard let p = a.ptr, a.len > 0 else { return .array([]) }
            return .array((0 ..< a.len).map { decodeCValue(p[$0]) })
        case UInt32(NVE_VAL_MAP.rawValue):
            let m = value.data.map_val
            guard let p = m.ptr, m.len > 0 else { return .map([]) }
            return .map((0 ..< m.len).map { (decodeStrSlice(p[$0].key), decodeCValue(p[$0].value)) })
        default:
            return .null
        }
    }
}
