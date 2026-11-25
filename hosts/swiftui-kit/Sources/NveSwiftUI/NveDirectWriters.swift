#if canImport(CNveAbi)
import CNveAbi
#endif
import Foundation

// MARK: - Zero-Copy Direct C-ABI Props Writer

/// Writes structured Swift props directly into C-ABI `NveMapEntry[]` + `NveValue` buffers
/// backed by `NveDirectArena` for `set_props_direct` with direct binary FFI.
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

    /// Fallback binary encoder used only when a legacy C vtable has `set_props_direct == nil`.
    internal static func encodeFallbackBinData<P: NveDirectEncodable>(from props: P) throws -> Data {
        try withEncodedProps(props) { ptr, count in
            try encodeCMapToBin(ptr: ptr, count: Int(count)).encode()
        }
    }

    private static func encodeCMapToBin(ptr: UnsafePointer<NveMapEntry>?, count: Int) -> NveBinValue {
        guard let ptr, count > 0 else { return .map([]) }
        var entries: [(String, NveBinValue)] = []
        entries.reserveCapacity(count)
        for i in 0 ..< count {
            entries.append((decodeStrSlice(ptr[i].key), decodeCValue(ptr[i].value)))
        }
        return .map(entries)
    }

    private static func decodeStrSlice(_ slice: NveStrSlice) -> String {
        guard let p = slice.ptr, slice.len > 0 else { return "" }
        return String(decoding: UnsafeBufferPointer(start: p, count: slice.len), as: UTF8.self)
    }

    private static func decodeCValue(_ value: NveValue) -> NveBinValue {
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
            return encodeCMapToBin(ptr: m.ptr, count: m.len)
        default:
            return .null
        }
    }
}

// MARK: - Zero-Copy Direct C-ABI Event Batch Writer

/// Writes typed `NveEvent` batches directly into C-ABI `NveDirectEvent[]` + `NveEventBatch`
/// buffers for `NveSurfaceApi.dispatch_events_direct` with direct binary FFI.
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
                buf.advanced(by: idx).initialize(to: try encodeEvent(event, into: arena))
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
            var event = makeDirectEvent(
                surfaceId: surfaceId,
                nodeId: nodeId,
                timestampUs: timestampUs,
                kind: NveEventKind.custom.rawValue,
                payloadTag: UInt8(NVE_EVENT_PAYLOAD_CUSTOM.rawValue),
                name: arena.copyString(name),
                value: try arena.writeValue(value)
            )
            return try withUnsafePointer(to: &event) { ptr in
                try body(NveEventBatch(ptr: ptr, len: 1))
            }
        }
    }

    private static func makeDirectEvent(
        surfaceId: UInt32,
        nodeId: UInt32,
        timestampUs: UInt64,
        kind: UInt8,
        payloadTag: UInt8,
        name: NveStrSlice = NveStrSlice(ptr: nil, len: 0),
        x: Float = 0,
        y: Float = 0,
        width: Float = 0,
        height: Float = 0,
        text: NveStrSlice = NveStrSlice(ptr: nil, len: 0),
        value: NveValue = NveDirectArena.nullValue()
    ) -> NveDirectEvent {
        NveDirectEvent(
            surface_id: surfaceId,
            node_id: nodeId,
            timestamp_us: timestampUs,
            kind: kind,
            payload_tag: payloadTag,
            name: name,
            x: x,
            y: y,
            width: width,
            height: height,
            text: text,
            value: value
        )
    }

    private static func encodeEvent(
        _ event: NveEvent,
        into arena: NveDirectArena
    ) throws -> NveDirectEvent {
        switch event.payload {
        case .empty:
            return makeDirectEvent(
                surfaceId: event.surfaceId,
                nodeId: event.nodeId,
                timestampUs: event.timestampUs,
                kind: event.kind.rawValue,
                payloadTag: UInt8(NVE_EVENT_PAYLOAD_EMPTY.rawValue)
            )
        case let .point(x, y):
            return makeDirectEvent(
                surfaceId: event.surfaceId,
                nodeId: event.nodeId,
                timestampUs: event.timestampUs,
                kind: event.kind.rawValue,
                payloadTag: UInt8(NVE_EVENT_PAYLOAD_POINT.rawValue),
                x: x,
                y: y
            )
        case let .scroll(x, y, contentWidth, contentHeight):
            return makeDirectEvent(
                surfaceId: event.surfaceId,
                nodeId: event.nodeId,
                timestampUs: event.timestampUs,
                kind: event.kind.rawValue,
                payloadTag: UInt8(NVE_EVENT_PAYLOAD_SCROLL.rawValue),
                x: x,
                y: y,
                width: contentWidth,
                height: contentHeight
            )
        case let .text(text):
            return makeDirectEvent(
                surfaceId: event.surfaceId,
                nodeId: event.nodeId,
                timestampUs: event.timestampUs,
                kind: event.kind.rawValue,
                payloadTag: UInt8(NVE_EVENT_PAYLOAD_TEXT.rawValue),
                text: arena.copyString(text)
            )
        case let .layout(x, y, width, height):
            return makeDirectEvent(
                surfaceId: event.surfaceId,
                nodeId: event.nodeId,
                timestampUs: event.timestampUs,
                kind: event.kind.rawValue,
                payloadTag: UInt8(NVE_EVENT_PAYLOAD_LAYOUT.rawValue),
                x: x,
                y: y,
                width: width,
                height: height
            )
        case let .binValue(data):
            let cVal: NveValue
            if data.isEmpty {
                cVal = NveDirectArena.nullValue()
            } else if let parsed = try? NveBinValueView.withParsed(data, { view in
                try arena.writeBinValueView(view)
            }) {
                cVal = parsed
            } else {
                cVal = NveDirectArena.bytesValue(arena.copyBytes(data))
            }
            return makeDirectEvent(
                surfaceId: event.surfaceId,
                nodeId: event.nodeId,
                timestampUs: event.timestampUs,
                kind: NveEventKind.custom.rawValue,
                payloadTag: UInt8(NVE_EVENT_PAYLOAD_CUSTOM.rawValue),
                name: arena.copyString(event.name ?? ""),
                value: cVal
            )
        }
    }
}
