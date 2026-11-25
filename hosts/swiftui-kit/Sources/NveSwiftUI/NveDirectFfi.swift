import CNveAbi
import CoreFoundation
import Foundation

/// Errors thrown while encoding values or events into zero-copy direct C-ABI structs.
public enum NveDirectFfiError: Error, Equatable, Sendable {
    case unsupportedValueType(String)
    case maxDepthExceeded
}

// MARK: - Swift `Any` -> `NveBinValue` Conversion (Zero JSON)

extension NveBinValue {
    /// Converts a dynamic Swift value into a typed `NveBinValue` with zero JSON overhead.
    public static func from(any value: Any, depth: Int = 0) throws -> NveBinValue {
        if depth > nveBinValueMaxDepth {
            throw NveDirectFfiError.maxDepthExceeded
        }
        switch value {
        case let existing as NveBinValue:
            return existing
        case is NSNull:
            return .null
        case let boolVal as Bool where type(of: value) == Bool.self:
            return .bool(boolVal)
        case let num as NSNumber:
            if CFGetTypeID(num) == CFBooleanGetTypeID() {
                return .bool(num.boolValue)
            }
            if CFNumberIsFloatType(num) {
                return .float64(num.doubleValue)
            }
            return .int64(num.int64Value)
        case let i as Int:
            return .int64(Int64(i))
        case let i as Int64:
            return .int64(i)
        case let i as Int32:
            return .int64(Int64(i))
        case let i as Int16:
            return .int64(Int64(i))
        case let i as Int8:
            return .int64(Int64(i))
        case let u as UInt:
            return .int64(Int64(bitPattern: UInt64(u)))
        case let u as UInt64:
            return .int64(Int64(bitPattern: u))
        case let u as UInt32:
            return .int64(Int64(u))
        case let u as UInt16:
            return .int64(Int64(u))
        case let u as UInt8:
            return .int64(Int64(u))
        case let d as Double:
            return .float64(d)
        case let f as Float:
            return .float64(Double(f))
        case let s as String:
            return .string(s)
        case let data as Data:
            return .bytes(data)
        case let array as [NveBinValue]:
            if depth >= nveBinValueMaxDepth {
                throw NveDirectFfiError.maxDepthExceeded
            }
            return .array(array)
        case let array as [Any]:
            if depth >= nveBinValueMaxDepth {
                throw NveDirectFfiError.maxDepthExceeded
            }
            var items: [NveBinValue] = []
            items.reserveCapacity(array.count)
            for elem in array {
                items.append(try from(any: elem, depth: depth + 1))
            }
            return .array(items)
        case let map as [String: NveBinValue]:
            if depth >= nveBinValueMaxDepth {
                throw NveDirectFfiError.maxDepthExceeded
            }
            let sortedKeys = map.keys.sorted()
            return .map(sortedKeys.map { ($0, map[$0]!) })
        case let map as [String: Any]:
            if depth >= nveBinValueMaxDepth {
                throw NveDirectFfiError.maxDepthExceeded
            }
            let sortedKeys = map.keys.sorted()
            var entries: [(String, NveBinValue)] = []
            entries.reserveCapacity(sortedKeys.count)
            for key in sortedKeys {
                let converted = try from(any: map[key]!, depth: depth + 1)
                entries.append((key, converted))
            }
            return .map(entries)
        default:
            throw NveDirectFfiError.unsupportedValueType(String(describing: type(of: value)))
        }
    }

    /// Converts a `[String: Any]` prop map into deterministically key-sorted `[(String, NveBinValue)]`.
    public static func canonicalEntries(from props: [String: Any]) throws -> [(String, NveBinValue)] {
        let sortedKeys = props.keys.sorted()
        var entries: [(String, NveBinValue)] = []
        entries.reserveCapacity(sortedKeys.count)
        for key in sortedKeys {
            entries.append((key, try from(any: props[key]!, depth: 1)))
        }
        return entries
    }
}

// MARK: - Zero-Allocation Stack-First Bump Arena

/// Temporary chunked bump arena backed by an inline stack slab (4 KiB) with heap overflow chunks
/// only when a payload exceeds 4 KiB. Existing pointers never move for the arena's lifetime.
private final class NveDirectArena {
    private var currentBase: UnsafeMutableRawPointer
    private var currentCapacity: Int
    private var currentOffset: Int = 0
    private var heapChunks: [UnsafeMutableRawPointer] = []

    init(stackSlab: UnsafeMutableRawBufferPointer) {
        self.currentBase = stackSlab.baseAddress!
        self.currentCapacity = stackSlab.count
    }

    deinit {
        for chunk in heapChunks {
            chunk.deallocate()
        }
    }

    func allocate(byteCount: Int, alignment: Int) -> UnsafeMutableRawPointer {
        let size = max(byteCount, 1)
        let align = max(alignment, 1)
        let alignedOffset = (currentOffset + (align - 1)) & ~(align - 1)
        if alignedOffset + size <= currentCapacity {
            let ptr = currentBase.advanced(by: alignedOffset)
            currentOffset = alignedOffset + size
            return ptr
        }
        let chunkSize = max(size, max(currentCapacity * 2, 4096))
        let chunk = UnsafeMutableRawPointer.allocate(byteCount: chunkSize, alignment: max(align, 8))
        heapChunks.append(chunk)
        currentBase = chunk
        currentCapacity = chunkSize
        currentOffset = size
        return chunk
    }

    func allocateTyped<T>(_ type: T.Type, count: Int) -> UnsafeMutablePointer<T>? {
        guard count > 0 else { return nil }
        let raw = allocate(
            byteCount: MemoryLayout<T>.stride * count,
            alignment: MemoryLayout<T>.alignment
        )
        return raw.bindMemory(to: T.self, capacity: count)
    }

    func copyString(_ string: String) -> NveStrSlice {
        let utf8 = string.utf8
        let len = utf8.count
        guard len > 0 else {
            return NveStrSlice(ptr: nil, len: 0)
        }
        let raw = allocate(byteCount: len, alignment: 1).assumingMemoryBound(to: UInt8.self)
        var i = 0
        for byte in utf8 {
            raw[i] = byte
            i += 1
        }
        return NveStrSlice(ptr: UnsafePointer(raw), len: len)
    }

    func copyBytes(_ data: Data) -> NveBufferSlice {
        let len = data.count
        guard len > 0 else {
            return NveBufferSlice(ptr: nil, len: 0)
        }
        let raw = allocate(byteCount: len, alignment: 1).assumingMemoryBound(to: UInt8.self)
        data.copyBytes(to: raw, count: len)
        return NveBufferSlice(ptr: UnsafePointer(raw), len: len)
    }

    func copyRawBuffer(_ buffer: UnsafeRawBufferPointer) -> NveBufferSlice {
        let len = buffer.count
        guard len > 0, let base = buffer.baseAddress else {
            return NveBufferSlice(ptr: nil, len: 0)
        }
        let raw = allocate(byteCount: len, alignment: 1)
        raw.copyMemory(from: base, byteCount: len)
        return NveBufferSlice(ptr: raw.assumingMemoryBound(to: UInt8.self), len: len)
    }

    func writeValue(_ value: NveBinValue, depth: Int = 0) throws -> NveValue {
        if depth > nveBinValueMaxDepth {
            throw NveDirectFfiError.maxDepthExceeded
        }
        switch value {
        case .null:
            return NveValue(tag: UInt32(NVE_VAL_NULL.rawValue), data: NveValueData(i64_val: 0))
        case let .bool(v):
            return NveValue(
                tag: UInt32(NVE_VAL_BOOL.rawValue),
                data: NveValueData(bool_val: v ? 1 : 0)
            )
        case let .int64(v):
            return NveValue(tag: UInt32(NVE_VAL_I64.rawValue), data: NveValueData(i64_val: v))
        case let .float64(v):
            return NveValue(tag: UInt32(NVE_VAL_F64.rawValue), data: NveValueData(f64_val: v))
        case let .string(s):
            return NveValue(
                tag: UInt32(NVE_VAL_STR.rawValue),
                data: NveValueData(str_val: copyString(s))
            )
        case let .bytes(d):
            return NveValue(
                tag: UInt32(NVE_VAL_BYTES.rawValue),
                data: NveValueData(bytes_val: copyBytes(d))
            )
        case let .array(items):
            if depth >= nveBinValueMaxDepth {
                throw NveDirectFfiError.maxDepthExceeded
            }
            guard !items.isEmpty, let buf = allocateTyped(NveValue.self, count: items.count) else {
                return NveValue(
                    tag: UInt32(NVE_VAL_ARRAY.rawValue),
                    data: NveValueData(array_val: NveValueArray(ptr: nil, len: 0))
                )
            }
            for (idx, item) in items.enumerated() {
                buf.advanced(by: idx).initialize(to: try writeValue(item, depth: depth + 1))
            }
            return NveValue(
                tag: UInt32(NVE_VAL_ARRAY.rawValue),
                data: NveValueData(array_val: NveValueArray(ptr: UnsafePointer(buf), len: items.count))
            )
        case let .map(entries):
            if depth >= nveBinValueMaxDepth {
                throw NveDirectFfiError.maxDepthExceeded
            }
            let (ptr, count) = try writeEntries(entries, depth: depth + 1)
            return NveValue(
                tag: UInt32(NVE_VAL_MAP.rawValue),
                data: NveValueData(map_val: NveValueMap(ptr: ptr, len: Int(count)))
            )
        }
    }

    /// Writes a validated zero-copy `NveBinValueView` directly into `NveValue` structs without
    /// building an intermediate `NveBinValue` Swift enum tree.
    func writeBinValueView(_ view: NveBinValueView, depth: Int = 0) throws -> NveValue {
        if depth > nveBinValueMaxDepth {
            throw NveDirectFfiError.maxDepthExceeded
        }
        switch view.tag {
        case .null:
            return NveValue(tag: UInt32(NVE_VAL_NULL.rawValue), data: NveValueData(i64_val: 0))
        case .bool:
            return NveValue(
                tag: UInt32(NVE_VAL_BOOL.rawValue),
                data: NveValueData(bool_val: (view.boolValue ?? false) ? 1 : 0)
            )
        case .int64:
            return NveValue(
                tag: UInt32(NVE_VAL_I64.rawValue),
                data: NveValueData(i64_val: view.int64Value ?? 0)
            )
        case .float64:
            return NveValue(
                tag: UInt32(NVE_VAL_F64.rawValue),
                data: NveValueData(f64_val: view.doubleValue ?? 0)
            )
        case .string:
            let slice = view.utf8Bytes.map { copyRawBuffer($0) } ?? NveBufferSlice(ptr: nil, len: 0)
            return NveValue(
                tag: UInt32(NVE_VAL_STR.rawValue),
                data: NveValueData(str_val: NveStrSlice(ptr: slice.ptr, len: slice.len))
            )
        case .bytes:
            let slice = view.bytesValue.map { copyRawBuffer($0) } ?? NveBufferSlice(ptr: nil, len: 0)
            return NveValue(
                tag: UInt32(NVE_VAL_BYTES.rawValue),
                data: NveValueData(bytes_val: slice)
            )
        case .array:
            if depth >= nveBinValueMaxDepth {
                throw NveDirectFfiError.maxDepthExceeded
            }
            let count = view.arrayCount ?? 0
            guard count > 0, let buf = allocateTyped(NveValue.self, count: count) else {
                return NveValue(
                    tag: UInt32(NVE_VAL_ARRAY.rawValue),
                    data: NveValueData(array_val: NveValueArray(ptr: nil, len: 0))
                )
            }
            for i in 0 ..< count {
                if let elem = view.arrayElement(at: i) {
                    buf.advanced(by: i).initialize(to: try writeBinValueView(elem, depth: depth + 1))
                } else {
                    buf.advanced(by: i).initialize(
                        to: NveValue(tag: UInt32(NVE_VAL_NULL.rawValue), data: NveValueData(i64_val: 0))
                    )
                }
            }
            return NveValue(
                tag: UInt32(NVE_VAL_ARRAY.rawValue),
                data: NveValueData(array_val: NveValueArray(ptr: UnsafePointer(buf), len: count))
            )
        case .map:
            if depth >= nveBinValueMaxDepth {
                throw NveDirectFfiError.maxDepthExceeded
            }
            let count = view.mapCount ?? 0
            guard count > 0, let buf = allocateTyped(NveMapEntry.self, count: count) else {
                return NveValue(
                    tag: UInt32(NVE_VAL_MAP.rawValue),
                    data: NveValueData(map_val: NveValueMap(ptr: nil, len: 0))
                )
            }
            var idx = 0
            try view.forEachMapEntry { key, val in
                let keySlice = copyString(key)
                let valStruct = try writeBinValueView(val, depth: depth + 1)
                buf.advanced(by: idx).initialize(to: NveMapEntry(key: keySlice, value: valStruct))
                idx += 1
            }
            return NveValue(
                tag: UInt32(NVE_VAL_MAP.rawValue),
                data: NveValueData(map_val: NveValueMap(ptr: UnsafePointer(buf), len: idx))
            )
        }
    }

    func writeEntries(
        _ entries: [(String, NveBinValue)],
        depth: Int = 0
    ) throws -> (UnsafePointer<NveMapEntry>?, UInt32) {
        guard !entries.isEmpty, let buf = allocateTyped(NveMapEntry.self, count: entries.count) else {
            return (nil, 0)
        }
        for (idx, (key, val)) in entries.enumerated() {
            let keySlice = copyString(key)
            let valStruct = try writeValue(val, depth: depth)
            buf.advanced(by: idx).initialize(to: NveMapEntry(key: keySlice, value: valStruct))
        }
        return (UnsafePointer(buf), UInt32(entries.count))
    }

    static func withArena<R>(_ body: (NveDirectArena) throws -> R) rethrows -> R {
        try withUnsafeTemporaryAllocation(byteCount: 4096, alignment: 8) { stackSlab in
            let arena = NveDirectArena(stackSlab: stackSlab)
            return try body(arena)
        }
    }
}

// MARK: - Zero-Copy Direct C-ABI Props Writer

/// Writes structured Swift props directly into C-ABI `NveMapEntry[]` + `NveValue` buffers
/// for `NveComponentApi.set_props_direct` with zero JSON serialization.
public enum NveDirectPropsWriter {
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
}

// MARK: - Zero-Copy Direct C-ABI Event Batch Writer

/// Writes typed `NveEvent` batches directly into C-ABI `NveDirectEvent[]` + `NveEventBatch`
/// buffers for `NveSurfaceApi.dispatch_events_direct` with zero JSON serialization.
public enum NveDirectEventWriter {
    public static func withBatch<R>(
        _ events: [NveEvent],
        _ body: (NveEventBatch) throws -> R
    ) throws -> R {
        guard !events.isEmpty else {
            return try body(NveEventBatch(ptr: nil, len: 0))
        }
        return try NveDirectArena.withArena { arena in
            guard let buf = arena.allocateTyped(NveDirectEvent.self, count: events.count) else {
                return try body(NveEventBatch(ptr: nil, len: 0))
            }
            for (idx, event) in events.enumerated() {
                let direct = try encodeEvent(event, into: arena)
                buf.advanced(by: idx).initialize(to: direct)
            }
            return try body(NveEventBatch(ptr: UnsafePointer(buf), len: UInt32(events.count)))
        }
    }

    public static func withCustomEvent<R>(
        surfaceId: UInt32,
        nodeId: UInt32 = 1,
        timestampUs: UInt64,
        name: String,
        value: NveBinValue,
        _ body: (NveEventBatch) throws -> R
    ) throws -> R {
        try NveDirectArena.withArena { arena in
            let nameSlice = arena.copyString(name)
            let cVal = try arena.writeValue(value)
            var event = NveDirectEvent(
                surface_id: surfaceId,
                node_id: nodeId,
                timestamp_us: timestampUs,
                kind: NveEventKind.custom.rawValue,
                payload_tag: UInt8(NVE_EVENT_PAYLOAD_CUSTOM.rawValue),
                name: nameSlice,
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                text: NveStrSlice(ptr: nil, len: 0),
                value: cVal
            )
            return try withUnsafePointer(to: &event) { ptr in
                try body(NveEventBatch(ptr: ptr, len: 1))
            }
        }
    }

    private static func encodeEvent(
        _ event: NveEvent,
        into arena: NveDirectArena
    ) throws -> NveDirectEvent {
        let emptyStr = NveStrSlice(ptr: nil, len: 0)
        let nullVal = NveValue(tag: UInt32(NVE_VAL_NULL.rawValue), data: NveValueData(i64_val: 0))

        switch event.payload {
        case .empty:
            return NveDirectEvent(
                surface_id: event.surfaceId,
                node_id: event.nodeId,
                timestamp_us: event.timestampUs,
                kind: event.kind.rawValue,
                payload_tag: UInt8(NVE_EVENT_PAYLOAD_EMPTY.rawValue),
                name: emptyStr,
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                text: emptyStr,
                value: nullVal
            )
        case let .point(x, y):
            return NveDirectEvent(
                surface_id: event.surfaceId,
                node_id: event.nodeId,
                timestamp_us: event.timestampUs,
                kind: event.kind.rawValue,
                payload_tag: UInt8(NVE_EVENT_PAYLOAD_POINT.rawValue),
                name: emptyStr,
                x: x,
                y: y,
                width: 0,
                height: 0,
                text: emptyStr,
                value: nullVal
            )
        case let .scroll(x, y, contentWidth, contentHeight):
            return NveDirectEvent(
                surface_id: event.surfaceId,
                node_id: event.nodeId,
                timestamp_us: event.timestampUs,
                kind: event.kind.rawValue,
                payload_tag: UInt8(NVE_EVENT_PAYLOAD_SCROLL.rawValue),
                name: emptyStr,
                x: x,
                y: y,
                width: contentWidth,
                height: contentHeight,
                text: emptyStr,
                value: nullVal
            )
        case let .text(text):
            return NveDirectEvent(
                surface_id: event.surfaceId,
                node_id: event.nodeId,
                timestamp_us: event.timestampUs,
                kind: event.kind.rawValue,
                payload_tag: UInt8(NVE_EVENT_PAYLOAD_TEXT.rawValue),
                name: emptyStr,
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                text: arena.copyString(text),
                value: nullVal
            )
        case let .layout(x, y, width, height):
            return NveDirectEvent(
                surface_id: event.surfaceId,
                node_id: event.nodeId,
                timestamp_us: event.timestampUs,
                kind: event.kind.rawValue,
                payload_tag: UInt8(NVE_EVENT_PAYLOAD_LAYOUT.rawValue),
                name: emptyStr,
                x: x,
                y: y,
                width: width,
                height: height,
                text: emptyStr,
                value: nullVal
            )
        case let .binValue(data):
            let nameSlice = arena.copyString(event.name ?? "")
            let cVal: NveValue
            if data.isEmpty {
                cVal = nullVal
            } else if let parsed = try? NveBinValueView.withParsed(data, { view in
                try arena.writeBinValueView(view)
            }) {
                cVal = parsed
            } else {
                cVal = NveValue(
                    tag: UInt32(NVE_VAL_BYTES.rawValue),
                    data: NveValueData(bytes_val: arena.copyBytes(data))
                )
            }
            return NveDirectEvent(
                surface_id: event.surfaceId,
                node_id: event.nodeId,
                timestamp_us: event.timestampUs,
                kind: NveEventKind.custom.rawValue,
                payload_tag: UInt8(NVE_EVENT_PAYLOAD_CUSTOM.rawValue),
                name: nameSlice,
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                text: emptyStr,
                value: cVal
            )
        }
    }
}
