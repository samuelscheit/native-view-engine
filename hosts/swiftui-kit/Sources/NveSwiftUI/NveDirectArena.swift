#if canImport(CNveAbi)
import CNveAbi
#endif
import Foundation

/// Errors thrown while encoding values or events into zero-copy direct C-ABI structs.
public enum NveDirectFfiError: Error, Equatable, Sendable {
    case unsupportedValueType(String)
    case maxDepthExceeded
}

/// Temporary chunked bump arena backed by an inline 4 KiB stack slab with heap overflow chunks
/// allocated only when a payload exceeds 4 KiB. Existing pointers never move for the arena's
/// lifetime.
public final class NveDirectArena {
    public static let stackSlabByteCount = 4096

    private var currentBase: UnsafeMutableRawPointer
    private var currentCapacity: Int
    private var currentOffset: Int = 0
    private var heapChunks: [UnsafeMutableRawPointer] = []

    /// Number of heap overflow chunks allocated beyond the initial 4 KiB stack slab.
    public var heapChunkCount: Int { heapChunks.count }

    public init(stackSlab: UnsafeMutableRawBufferPointer) {
        self.currentBase = stackSlab.baseAddress!
        self.currentCapacity = stackSlab.count
    }

    deinit {
        for chunk in heapChunks {
            chunk.deallocate()
        }
    }

    public func allocate(byteCount: Int, alignment: Int) -> UnsafeMutableRawPointer {
        let size = max(byteCount, 1)
        let align = max(alignment, 1)
        let alignedOffset = (currentOffset + (align - 1)) & ~(align - 1)
        if alignedOffset + size <= currentCapacity {
            let ptr = currentBase.advanced(by: alignedOffset)
            currentOffset = alignedOffset + size
            return ptr
        }
        let chunkSize = max(size, max(currentCapacity * 2, Self.stackSlabByteCount))
        let chunk = UnsafeMutableRawPointer.allocate(byteCount: chunkSize, alignment: max(align, 8))
        heapChunks.append(chunk)
        currentBase = chunk
        currentCapacity = chunkSize
        currentOffset = size
        return chunk
    }

    public func allocateTyped<T>(_ type: T.Type, count: Int) -> UnsafeMutablePointer<T>? {
        guard count > 0 else { return nil }
        let raw = allocate(
            byteCount: MemoryLayout<T>.stride * count,
            alignment: MemoryLayout<T>.alignment
        )
        return raw.bindMemory(to: T.self, capacity: count)
    }

    /// Borrows a compile-time `StaticString` directly with zero copy and zero arena bytes when
    /// pointer-backed (falling back to copying only for single-scalar `StaticString`s).
    @inline(__always)
    public func staticStringSlice(_ key: StaticString) -> NveStrSlice {
        if key.hasPointerRepresentation {
            guard key.utf8CodeUnitCount > 0 else {
                return NveStrSlice(ptr: nil, len: 0)
            }
            return NveStrSlice(ptr: key.utf8Start, len: key.utf8CodeUnitCount)
        }
        return copyString(String(key.unicodeScalar))
    }

    public func copyString(_ string: String) -> NveStrSlice {
        let utf8 = string.utf8
        let len = utf8.count
        guard len > 0 else {
            return NveStrSlice(ptr: nil, len: 0)
        }
        let raw = allocate(byteCount: len, alignment: 1)
        let copied = utf8.withContiguousStorageIfAvailable { buf -> Bool in
            if let base = buf.baseAddress {
                raw.copyMemory(from: base, byteCount: len)
                return true
            }
            return false
        } ?? false
        let bytesPtr = raw.assumingMemoryBound(to: UInt8.self)
        if !copied {
            var i = 0
            for byte in utf8 {
                bytesPtr[i] = byte
                i += 1
            }
        }
        return NveStrSlice(ptr: UnsafePointer(bytesPtr), len: len)
    }

    public func copyBytes(_ data: Data) -> NveBufferSlice {
        let len = data.count
        guard len > 0 else {
            return NveBufferSlice(ptr: nil, len: 0)
        }
        let raw = allocate(byteCount: len, alignment: 1).assumingMemoryBound(to: UInt8.self)
        data.copyBytes(to: raw, count: len)
        return NveBufferSlice(ptr: UnsafePointer(raw), len: len)
    }

    public func copyBytes(_ bytes: [UInt8]) -> NveBufferSlice {
        let len = bytes.count
        guard len > 0 else {
            return NveBufferSlice(ptr: nil, len: 0)
        }
        let raw = allocate(byteCount: len, alignment: 1)
        bytes.withUnsafeBufferPointer { buf in
            if let base = buf.baseAddress {
                raw.copyMemory(from: base, byteCount: len)
            }
        }
        return NveBufferSlice(ptr: raw.assumingMemoryBound(to: UInt8.self), len: len)
    }

    public func copyRawBuffer(_ buffer: UnsafeRawBufferPointer) -> NveBufferSlice {
        let len = buffer.count
        guard len > 0, let base = buffer.baseAddress else {
            return NveBufferSlice(ptr: nil, len: 0)
        }
        let raw = allocate(byteCount: len, alignment: 1)
        raw.copyMemory(from: base, byteCount: len)
        return NveBufferSlice(ptr: raw.assumingMemoryBound(to: UInt8.self), len: len)
    }

    @inline(__always)
    public static func nullValue() -> NveValue {
        NveValue(tag: UInt32(NVE_VAL_NULL.rawValue), data: NveValueData(i64_val: 0))
    }

    @inline(__always)
    public static func boolValue(_ value: Bool) -> NveValue {
        NveValue(tag: UInt32(NVE_VAL_BOOL.rawValue), data: NveValueData(bool_val: value ? 1 : 0))
    }

    @inline(__always)
    public static func int64Value(_ value: Int64) -> NveValue {
        NveValue(tag: UInt32(NVE_VAL_I64.rawValue), data: NveValueData(i64_val: value))
    }

    @inline(__always)
    public static func float64Value(_ value: Double) -> NveValue {
        NveValue(tag: UInt32(NVE_VAL_F64.rawValue), data: NveValueData(f64_val: value))
    }

    @inline(__always)
    public static func stringValue(_ slice: NveStrSlice) -> NveValue {
        NveValue(tag: UInt32(NVE_VAL_STR.rawValue), data: NveValueData(str_val: slice))
    }

    @inline(__always)
    public static func bytesValue(_ slice: NveBufferSlice) -> NveValue {
        NveValue(tag: UInt32(NVE_VAL_BYTES.rawValue), data: NveValueData(bytes_val: slice))
    }

    @inline(__always)
    public static func arrayValue(ptr: UnsafePointer<NveValue>?, len: Int) -> NveValue {
        NveValue(
            tag: UInt32(NVE_VAL_ARRAY.rawValue),
            data: NveValueData(array_val: NveValueArray(ptr: ptr, len: len))
        )
    }

    @inline(__always)
    public static func mapValue(ptr: UnsafePointer<NveMapEntry>?, len: Int) -> NveValue {
        NveValue(
            tag: UInt32(NVE_VAL_MAP.rawValue),
            data: NveValueData(map_val: NveValueMap(ptr: ptr, len: len))
        )
    }

    public static func withArena<R>(_ body: (NveDirectArena) throws -> R) rethrows -> R {
        try withUnsafeTemporaryAllocation(byteCount: stackSlabByteCount, alignment: 8) { stackSlab in
            let arena = NveDirectArena(stackSlab: stackSlab)
            return try body(arena)
        }
    }
}
