#if canImport(CNveAbi)
import CNveAbi
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

// MARK: - Zero-Copy Direct C-ABI Event Batch Writer

/// Writes typed `NveEvent` batches directly into pointer-free 64-byte C-ABI `NveEvent[]` +
/// shared byte-arena `NveEventBatch` buffers for `NveSurfaceApi.dispatch_events`.
public enum NveDirectEventWriter {
    private static func makeBatch<E>(
        _ rawEvents: UnsafeRawPointer?,
        bytes: UnsafePointer<UInt8>?,
        count: UInt32,
        bytesLen: UInt32,
        ctor: (UnsafePointer<E>?, UnsafePointer<UInt8>?, UInt32, UInt32) -> NveEventBatch
    ) -> NveEventBatch {
        ctor(rawEvents?.assumingMemoryBound(to: E.self), bytes, count, bytesLen)
    }

    public static func withBatch<R>(
        _ events: [NveEvent],
        _ body: (NveEventBatch) throws -> R
    ) throws -> R {
        guard !events.isEmpty else {
            return try body(NveEventBatch(events: nil, bytes: nil, event_count: 0, bytes_len: 0))
        }
        return try NveDirectArena.withArena { arena in
            var rawRecords = Data(count: events.count * 64)
            var byteArena = Data()
            rawRecords.withUnsafeMutableBytes { recBuf in
                for (idx, event) in events.enumerated() {
                    encodeRecord(event, into: recBuf, offset: idx * 64, byteArena: &byteArena)
                }
            }
            let recRaw = arena.allocate(byteCount: rawRecords.count, alignment: 8)
            rawRecords.withUnsafeBytes { src in
                if let base = src.baseAddress {
                    recRaw.copyMemory(from: base, byteCount: rawRecords.count)
                }
            }
            var arenaPtr: UnsafePointer<UInt8>? = nil
            if !byteArena.isEmpty {
                let dst = arena.allocate(byteCount: byteArena.count, alignment: 1)
                byteArena.withUnsafeBytes { src in
                    if let base = src.baseAddress {
                        dst.copyMemory(from: base, byteCount: byteArena.count)
                    }
                }
                arenaPtr = UnsafePointer(dst.assumingMemoryBound(to: UInt8.self))
            }
            let batch = makeBatch(
                UnsafeRawPointer(recRaw),
                bytes: arenaPtr,
                count: UInt32(events.count),
                bytesLen: UInt32(byteArena.count),
                ctor: NveEventBatch.init
            )
            return try body(batch)
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
        let encoded = try value.encode()
        return try withBatch(
            [
                NveEvent(
                    surfaceId: surfaceId,
                    nodeId: nodeId,
                    timestampUs: timestampUs,
                    kind: .custom,
                    name: name,
                    payload: .binValue(encoded)
                ),
            ],
            body
        )
    }

    internal static func tableAndEventId(for kind: NveEventKind) -> (UInt64, UInt32) {
        switch kind {
        case .press: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_PRESS.rawValue))
        case .longPress: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_LONG_PRESS.rawValue))
        case .focus: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_FOCUS.rawValue))
        case .blur: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_BLUR.rawValue))
        case .keyDown: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_KEY_DOWN.rawValue))
        case .keyUp: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_KEY_UP.rawValue))
        case .pointerDown: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_POINTER_DOWN.rawValue))
        case .pointerMove: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_POINTER_MOVE.rawValue))
        case .pointerUp: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_POINTER_UP.rawValue))
        case .pointerEnter: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_POINTER_ENTER.rawValue))
        case .pointerLeave: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_POINTER_LEAVE.rawValue))
        case .wheel: return (UInt64(NVE_TABLE_INTERACTION), UInt32(NVE_INTERACTION_EVENT_WHEEL.rawValue))
        case .textChange: return (UInt64(NVE_TABLE_TEXT_INPUT), UInt32(NVE_TEXT_INPUT_EVENT_CHANGE.rawValue))
        case .submit: return (UInt64(NVE_TABLE_TEXT_INPUT), UInt32(NVE_TEXT_INPUT_EVENT_SUBMIT.rawValue))
        case .compositionStart: return (UInt64(NVE_TABLE_TEXT_INPUT), UInt32(NVE_TEXT_INPUT_EVENT_COMPOSITION_START.rawValue))
        case .compositionUpdate: return (UInt64(NVE_TABLE_TEXT_INPUT), UInt32(NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE.rawValue))
        case .compositionEnd: return (UInt64(NVE_TABLE_TEXT_INPUT), UInt32(NVE_TEXT_INPUT_EVENT_COMPOSITION_END.rawValue))
        case .scroll: return (UInt64(NVE_TABLE_SCROLL), UInt32(NVE_SCROLL_EVENT_SCROLL.rawValue))
        case .layout: return (UInt64(NVE_TABLE_LAYOUT), UInt32(NVE_LAYOUT_EVENT_LAYOUT.rawValue))
        case .custom: return (UInt64(NVE_TABLE_CUSTOM), 0)
        }
    }

    internal static func encodeRecord(
        _ event: NveEvent,
        into buf: UnsafeMutableRawBufferPointer,
        offset base: Int,
        byteArena: inout Data
    ) {
        var (table, eventId) = tableAndEventId(for: event.kind)
        var flags: UInt32 = 0
        var p: (UInt32, UInt32, UInt32, UInt32, UInt32, UInt32) = (0, 0, 0, 0, 0, 0)
        var bytesOff: UInt32 = 0
        var bytesLen: UInt32 = 0

        func appendArena(_ data: Data) {
            if !data.isEmpty {
                bytesOff = UInt32(byteArena.count)
                bytesLen = UInt32(data.count)
                byteArena.append(data)
            }
        }

        switch event.payload {
        case .empty:
            break
        case let .point(x, y):
            p = (x.bitPattern, y.bitPattern, 0, 0, 0, 0)
        case let .scroll(x, y, contentWidth, contentHeight):
            p = (x.bitPattern, y.bitPattern, contentWidth.bitPattern, contentHeight.bitPattern, 0, 0)
        case let .text(text):
            flags |= UInt32(NVE_EVENT_FLAG_UTF8_BYTES)
            appendArena(Data(text.utf8))
        case let .layout(x, y, width, height):
            p = (x.bitPattern, y.bitPattern, width.bitPattern, height.bitPattern, 0, 0)
        case let .binValue(data):
            table = UInt64(NVE_TABLE_CUSTOM)
            eventId = 0
            flags |= UInt32(NVE_EVENT_FLAG_CUSTOM_NAMED)
            let nameBytes = Data((event.name ?? "").utf8)
            p.0 = UInt32(nameBytes.count)
            var combined = Data(capacity: nameBytes.count + data.count)
            combined.append(nameBytes)
            combined.append(data)
            appendArena(combined)
        case let .key(key, code, modifiers, keyCode, isComposing):
            flags |= UInt32(modifiers & 0x1F)
            if isComposing { flags |= UInt32(NVE_EVENT_FLAG_COMPOSING) }
            let keyBytes = Array(key.utf8)
            let codeData = Data(code.utf8)
            if keyBytes.count <= 16 {
                var kb = [UInt8](repeating: 0, count: 16)
                for i in 0 ..< keyBytes.count { kb[i] = keyBytes[i] }
                let w2 = UInt32(kb[0]) | (UInt32(kb[1]) << 8) | (UInt32(kb[2]) << 16) | (UInt32(kb[3]) << 24)
                let w3 = UInt32(kb[4]) | (UInt32(kb[5]) << 8) | (UInt32(kb[6]) << 16) | (UInt32(kb[7]) << 24)
                let w4 = UInt32(kb[8]) | (UInt32(kb[9]) << 8) | (UInt32(kb[10]) << 16) | (UInt32(kb[11]) << 24)
                let w5 = UInt32(kb[12]) | (UInt32(kb[13]) << 8) | (UInt32(kb[14]) << 16) | (UInt32(kb[15]) << 24)
                p = (keyCode, UInt32(keyBytes.count), w2, w3, w4, w5)
                flags |= UInt32(NVE_EVENT_FLAG_INLINE_KEY_UTF8)
                if !codeData.isEmpty { flags |= UInt32(NVE_EVENT_FLAG_UTF8_BYTES) }
                appendArena(codeData)
            } else {
                p = (keyCode, UInt32(keyBytes.count), 0, 0, 0, 0)
                flags |= UInt32(NVE_EVENT_FLAG_KEY_SPLIT_BYTES) | UInt32(NVE_EVENT_FLAG_UTF8_BYTES)
                var combined = Data(capacity: keyBytes.count + codeData.count)
                combined.append(contentsOf: keyBytes)
                combined.append(codeData)
                appendArena(combined)
            }
        case let .pointer(x, y, modifiers, pflags):
            flags |= UInt32(modifiers & 0x1F)
            let button = UInt32(pflags & 0x07)
            flags |= (button << UInt32(NVE_EVENT_FLAG_BUTTON_SHIFT)) & UInt32(NVE_EVENT_FLAG_BUTTON_MASK)
            if (pflags & 0x08) != 0 { flags |= UInt32(NVE_EVENT_FLAG_CAPTURE) }
            p = (x.bitPattern, y.bitPattern, 0, 0, 0, 0)
        case let .wheel(deltaX, deltaY, modifiers):
            flags |= UInt32(modifiers & 0x1F)
            p = (deltaX.bitPattern, deltaY.bitPattern, 0, 0, 0, 0)
        }

        buf.storeBytes(of: event.timestampUs.littleEndian, toByteOffset: base + 0, as: UInt64.self)
        buf.storeBytes(of: table.littleEndian, toByteOffset: base + 8, as: UInt64.self)
        buf.storeBytes(of: event.surfaceId.littleEndian, toByteOffset: base + 16, as: UInt32.self)
        buf.storeBytes(of: event.nodeId.littleEndian, toByteOffset: base + 20, as: UInt32.self)
        buf.storeBytes(of: eventId.littleEndian, toByteOffset: base + 24, as: UInt32.self)
        buf.storeBytes(of: flags.littleEndian, toByteOffset: base + 28, as: UInt32.self)
        buf.storeBytes(of: p.0.littleEndian, toByteOffset: base + 32, as: UInt32.self)
        buf.storeBytes(of: p.1.littleEndian, toByteOffset: base + 36, as: UInt32.self)
        buf.storeBytes(of: p.2.littleEndian, toByteOffset: base + 40, as: UInt32.self)
        buf.storeBytes(of: p.3.littleEndian, toByteOffset: base + 44, as: UInt32.self)
        buf.storeBytes(of: p.4.littleEndian, toByteOffset: base + 48, as: UInt32.self)
        buf.storeBytes(of: p.5.littleEndian, toByteOffset: base + 52, as: UInt32.self)
        buf.storeBytes(of: bytesOff.littleEndian, toByteOffset: base + 56, as: UInt32.self)
        buf.storeBytes(of: bytesLen.littleEndian, toByteOffset: base + 60, as: UInt32.self)
    }
}
