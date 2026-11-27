import CNveAbi
import NveHost

/// The C `NveValue` graph one C ABI call borrows (hybrid `set_props_direct` entries, `NveValue`
/// arguments). Every string, byte run, array, and map entry it returns points into memory this tree
/// owns, so keep the tree alive (`withExtendedLifetime`) until the call returns.
///
/// Storage is a chunked bump arena: one allocation per chunk (the chunks are linked through their
/// first word), never one per node, and nothing moves once handed out. Scalars, and dynamic values
/// that are scalars, allocate nothing.
public final class NveValueTree {
    /// Usable bytes of the first chunk; each further chunk at least doubles (and fits twice the run
    /// that opened it), up to `maxChunkBytes`. Runs past that get a chunk of their own.
    private static let firstChunkBytes = 1024
    private static let maxChunkBytes = 1 << 20
    /// Each chunk starts with the link to the previous one, padded to the arena's alignment.
    private static let header = 16

    private var chunk: UnsafeMutableRawPointer?
    private var cursor = 0
    private var end = 0
    private var nextChunkBytes = NveValueTree.firstChunkBytes
    /// Reused by `encoded`; taken out while in use, so a nested `encoded` gets its own writer.
    private var scratch: BinWriter?

    public init() {}

    deinit {
        var next = chunk
        while let current = next {
            next = current.load(as: UnsafeMutableRawPointer?.self)
            current.deallocate()
        }
    }

    // MARK: scalars

    public func null() -> NveValue { Self.tagged(NVE_VAL_NULL, NveValueData(i64_val: 0)) }

    public func bool(_ v: Bool) -> NveValue { Self.tagged(NVE_VAL_BOOL, NveValueData(bool_val: v ? 1 : 0)) }

    public func int(_ v: Int64) -> NveValue { Self.tagged(NVE_VAL_I64, NveValueData(i64_val: v)) }

    public func float(_ v: Double) -> NveValue { Self.tagged(NVE_VAL_F64, NveValueData(f64_val: v)) }

    // MARK: copied runs

    /// `Str` holding a copy of `v`'s UTF-8.
    public func string(_ v: String) -> NveValue {
        Self.tagged(NVE_VAL_STR, NveValueData(str_val: strSlice(v)))
    }

    /// `Bytes` holding a copy of `v`.
    public func bytes<Bytes: Collection>(_ v: Bytes) -> NveValue where Bytes.Element == UInt8 {
        Self.tagged(NVE_VAL_BYTES, NveValueData(bytes_val: bufferSlice(v)))
    }

    /// `Bytes` holding one `NveBinValue` that `encode` writes (hybrid `Struct` / `Vec` props).
    public func encoded(_ encode: (inout BinWriter) throws -> Void) rethrows -> NveValue {
        var w = scratch.take() ?? BinWriter()
        defer { scratch = w }
        w.clear()
        try encode(&w)
        return bytes(w.bytes)
    }

    /// A copy of `v`'s UTF-8 (`ptr` is null when empty).
    public func strSlice(_ v: String) -> NveStrSlice {
        var v = v  // `withUTF8` is mutating; native strings are already contiguous (no copy)
        return v.withUTF8 { NveStrSlice(ptr: copy(UnsafeRawBufferPointer($0)), len: $0.count) }
    }

    /// A copy of `v` (`ptr` is null when empty).
    public func bufferSlice<Bytes: Collection>(_ v: Bytes) -> NveBufferSlice where Bytes.Element == UInt8 {
        if let slice = v.withContiguousStorageIfAvailable({ NveBufferSlice(ptr: copy(UnsafeRawBufferPointer($0)), len: $0.count) }) {
            return slice
        }
        return Array(v).withUnsafeBytes { NveBufferSlice(ptr: copy($0), len: $0.count) }
    }

    /// A map entry under `key`. Pointer-backed `StaticString`s are borrowed (they live for the whole
    /// program); a single-scalar literal has no pointer and is copied.
    public func entry(_ key: StaticString, _ value: NveValue) -> NveMapEntry {
        let slice = key.hasPointerRepresentation
            ? NveStrSlice(ptr: key.utf8CodeUnitCount > 0 ? key.utf8Start : nil, len: key.utf8CodeUnitCount)
            : key.withUTF8Buffer { NveStrSlice(ptr: copy(UnsafeRawBufferPointer($0)), len: $0.count) }
        return NveMapEntry(key: slice, value: value)
    }

    // MARK: dynamic values

    /// `v` as a lossless `NveValue` tree with its own tags (`Null` ... `Map`), never wrapped as
    /// `Bytes`. Strings, bytes, and keys point into one copy of `v`'s encoding.
    public func value(_ v: DynamicValue) throws(BinError) -> NveValue {
        var r = v.reader
        switch v.tag {
        case .null, .bool, .i64, .f64: return try build(&r, base: nil)  // nothing to point into
        case .str, .bytes, .array, .map: return try build(&r, base: v.bytes.withUnsafeBytes { copy($0) })
        }
    }

    /// `DynamicValue` is validated (at most `binValueMaxDepth` deep), so the recursion is bounded and
    /// the reads below only fail on a broken invariant.
    private func build(_ r: inout BinReader, base: UnsafePointer<UInt8>?) throws(BinError) -> NveValue {
        switch try r.peekTag() {
        case .null:
            _ = try r.readNull()
            return null()
        case .bool: return bool(try r.readBool())
        case .i64: return int(try r.readI64())
        case .f64: return float(try r.readF64())
        case .str:
            _ = try r.u8()
            return Self.tagged(NVE_VAL_STR, NveValueData(str_val: try run(&r, base: base)))
        case .bytes:
            _ = try r.u8()
            let s = try run(&r, base: base)
            return Self.tagged(NVE_VAL_BYTES, NveValueData(bytes_val: NveBufferSlice(ptr: s.ptr, len: s.len)))
        case .array:
            let count = try r.beginArray()
            guard count > 0 else { return Self.tagged(NVE_VAL_ARRAY, NveValueData(array_val: NveValueArray(ptr: nil, len: 0))) }
            let items = try allocate(NveValue.self, count)
            for i in 0..<count { (items + i).initialize(to: try build(&r, base: base)) }
            return Self.tagged(NVE_VAL_ARRAY, NveValueData(array_val: NveValueArray(ptr: items, len: count)))
        case .map:
            let count = try r.beginMap()
            guard count > 0 else { return Self.tagged(NVE_VAL_MAP, NveValueData(map_val: NveValueMap(ptr: nil, len: 0))) }
            let entries = try allocate(NveMapEntry.self, count)
            for i in 0..<count {
                let key = try run(&r, base: base)
                (entries + i).initialize(to: NveMapEntry(key: key, value: try build(&r, base: base)))
            }
            return Self.tagged(NVE_VAL_MAP, NveValueData(map_val: NveValueMap(ptr: entries, len: count)))
        }
    }

    private static func tagged(_ tag: NveValueTag, _ data: NveValueData) -> NveValue {
        NveValue(tag: UInt32(tag.rawValue), data: data)
    }

    /// A `u32`-length-prefixed run of the copied encoding (`ptr` is null when empty).
    private func run(_ r: inout BinReader, base: UnsafePointer<UInt8>?) throws(BinError) -> NveStrSlice {
        let slice = try r.take(Int(try r.u32()))
        guard let base, !slice.isEmpty else { return NveStrSlice(ptr: nil, len: 0) }
        return NveStrSlice(ptr: base + slice.startIndex, len: slice.count)
    }

    // MARK: arena

    private func copy(_ src: UnsafeRawBufferPointer) -> UnsafePointer<UInt8>? {
        guard let from = src.baseAddress, !src.isEmpty else { return nil }
        let to = reserve(src.count, alignment: 1)
        to.copyMemory(from: from, byteCount: src.count)
        return UnsafePointer(to.bindMemory(to: UInt8.self, capacity: src.count))
    }

    /// Uninitialized room for `count` (> 0) values.
    private func allocate<T>(_: T.Type, _ count: Int) throws(BinError) -> UnsafeMutablePointer<T> {
        let (bytes, overflow) = MemoryLayout<T>.stride.multipliedReportingOverflow(by: count)
        guard !overflow, bytes <= Int.max - Self.header else { throw .tooLarge }  // 32-bit `arm64_32`
        return reserve(bytes, alignment: MemoryLayout<T>.alignment).bindMemory(to: T.self, capacity: count)
    }

    /// Chunks are `header`-aligned, so offsets aligned to any power of two up to `header` stay
    /// aligned in memory.
    private func reserve(_ byteCount: Int, alignment: Int) -> UnsafeMutableRawPointer {
        precondition(alignment > 0 && alignment <= Self.header && alignment & (alignment - 1) == 0, "unsupported alignment \(alignment)")
        let start = (cursor + alignment - 1) & ~(alignment - 1)
        if let chunk, byteCount <= end - start {
            cursor = start + byteCount
            return chunk + start
        }
        // No allocation this large can succeed: the same out-of-memory failure, reported up front.
        precondition(byteCount <= Int.max - Self.header, "NveValueTree: \(byteCount) bytes exceed the address space")
        if byteCount > Self.maxChunkBytes, let current = chunk {
            // Oversized: its own chunk, linked behind the current one, which keeps its free space.
            let own = allocateChunk(usable: byteCount, previous: current.load(as: UnsafeMutableRawPointer?.self))
            current.storeBytes(of: own, as: UnsafeMutableRawPointer?.self)
            return own + Self.header
        }
        // Grows geometrically with the runs it holds, so a series of medium runs (bigger than the
        // first chunk) costs O(log n) chunks, not one each.
        let usable = max(byteCount, nextChunkBytes)
        nextChunkBytes = min(usable, Self.maxChunkBytes / 2) * 2  // never overflows
        let fresh = allocateChunk(usable: usable, previous: chunk)
        chunk = fresh
        cursor = Self.header + byteCount
        end = Self.header + usable
        return fresh + Self.header
    }

    private func allocateChunk(usable: Int, previous: UnsafeMutableRawPointer?) -> UnsafeMutableRawPointer {
        let fresh = UnsafeMutableRawPointer.allocate(byteCount: Self.header + usable, alignment: Self.header)
        fresh.storeBytes(of: previous, as: UnsafeMutableRawPointer?.self)
        return fresh
    }
}
