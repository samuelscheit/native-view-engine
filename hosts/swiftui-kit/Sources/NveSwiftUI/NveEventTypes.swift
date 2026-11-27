#if canImport(CNveAbi)
import CNveAbi
#endif
import Foundation

// MARK: - Core Event & Backend Types

/// Rendering backend used for an embedded NVE surface inside SwiftUI.
public enum NveRenderBackend: String, Sendable, Hashable {
    case fabric
    case skia
}

/// Native event kind bridged over the NVE C ABI and `NVEV` v2 wire format.
public enum NveEventKind: UInt8, Sendable, Hashable {
    case press = 1
    case longPress = 2
    case scroll = 3
    case textChange = 4
    case focus = 5
    case blur = 6
    case layout = 7
    case custom = 8
    case compositionStart = 9
    case compositionUpdate = 10
    case compositionEnd = 11
    case keyDown = 12
    case keyUp = 13
    case submit = 14
    case pointerDown = 15
    case pointerMove = 16
    case pointerUp = 17
    case pointerEnter = 18
    case pointerLeave = 19
    case wheel = 20
}

/// Gesture arbitration outcome mirroring `GestureDisposition` in `packages/core/interop`.
public enum NveGestureDisposition: UInt8, Sendable, Hashable {
    case claim = 1
    case passThrough = 2
    case deferToParent = 3
}

/// Zero-allocation typed event payload replacing legacy JSON-encoded event dictionaries.
public enum NveEventPayload: Sendable, Equatable {
    case empty
    case point(x: Float, y: Float)
    case scroll(x: Float, y: Float, contentWidth: Float = 0, contentHeight: Float = 0)
    case text(String)
    case layout(x: Float = 0, y: Float = 0, width: Float, height: Float)
    case binValue(Data)
    case key(key: String, code: String = "", modifiers: UInt8 = 0, keyCode: UInt32 = 0, isComposing: Bool = false)
    case pointer(x: Float, y: Float, modifiers: UInt8 = 0, flags: UInt8 = 0)
    case wheel(deltaX: Float, deltaY: Float, modifiers: UInt8 = 0)

    /// Zero-copy view over `.binValue(Data)` payloads encoded in `NveBinValue` format.
    public func withBinValueView<R>(_ body: (NveBinValueView) throws -> R) throws -> R? {
        guard case let .binValue(data) = self, !data.isEmpty else {
            return nil
        }
        return try NveBinValueView.withParsed(data, body)
    }
}

/// Decoded outbound or inbound NVE event with strongly typed payload (binary).
public struct NveEvent: Sendable, Equatable {
    public let surfaceId: UInt32
    public let nodeId: UInt32
    public let timestampUs: UInt64
    public let kind: NveEventKind
    public let name: String?
    public let payload: NveEventPayload

    public init(
        surfaceId: UInt32,
        nodeId: UInt32,
        timestampUs: UInt64,
        kind: NveEventKind,
        name: String? = nil,
        payload: NveEventPayload = .empty
    ) {
        self.surfaceId = surfaceId
        self.nodeId = nodeId
        self.timestampUs = timestampUs
        self.kind = kind
        self.name = name
        self.payload = payload
    }

    /// Borrows a zero-copy ``NveBinValueView`` if `payload` holds a valid `NveBinValue` buffer.
    public func withBinValueView<R>(_ body: (NveBinValueView) throws -> R) throws -> R? {
        try payload.withBinValueView(body)
    }
}

/// Strict validation errors thrown by ``NveEventRingCodec/decodeStrict(_:)``.
public enum NveEventRingError: Error, Equatable, Sendable {
    case truncated
    case invalidMagic
    case unsupportedVersion(UInt16)
    case invalidEventTable(UInt64, UInt32)
    case invalidBounds
    case invalidUtf8
    case trailingBytes(Int)
}

// MARK: - Binary `NVEV` v2 Event Batch Codec (matches `packages/core/abi/src/event/wire.rs`)

public enum NveEventRingCodec {
    public static let wireHeaderBytes: Int = 14
    public static let eventRecordBytes: Int = 64
    public static let wireVersion: UInt16 = 2

    /// Strictly decodes a binary `NVEV` v2 event batch buffer, throwing on bad magic/version,
    /// truncation, out-of-bounds arena ranges, unknown event tables/IDs, invalid UTF-8, or
    /// trailing bytes.
    public static func decodeStrict(_ data: Data) throws -> [NveEvent] {
        guard data.count >= wireHeaderBytes else {
            throw NveEventRingError.truncated
        }
        return try data.withUnsafeBytes { raw in
            guard raw[0] == 0x4E, raw[1] == 0x56, raw[2] == 0x45, raw[3] == 0x56 else {
                throw NveEventRingError.invalidMagic
            }
            let version = UInt16(littleEndian: raw.loadUnaligned(fromByteOffset: 4, as: UInt16.self))
            guard version == wireVersion else {
                throw NveEventRingError.unsupportedVersion(version)
            }
            let eventCount = Int(UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: 6, as: UInt32.self)))
            let bytesLen = Int(UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: 10, as: UInt32.self)))
            let remaining = raw.count - wireHeaderBytes
            guard eventCount <= remaining / eventRecordBytes else {
                throw NveEventRingError.truncated
            }
            let recordsBytes = eventCount * eventRecordBytes
            let arenaStart = wireHeaderBytes + recordsBytes
            let availableArena = raw.count - arenaStart
            if availableArena < bytesLen {
                throw NveEventRingError.truncated
            }
            if availableArena > bytesLen {
                throw NveEventRingError.trailingBytes(availableArena - bytesLen)
            }

            func sliceArena(off: Int, len: Int) throws -> UnsafeRawBufferPointer {
                guard off >= 0, len >= 0, off <= bytesLen, len <= bytesLen - off else {
                    throw NveEventRingError.invalidBounds
                }
                return UnsafeRawBufferPointer(rebasing: raw[(arenaStart + off) ..< (arenaStart + off + len)])
            }

            func decodeUtf8Strict(_ slice: UnsafeRawBufferPointer) throws -> String {
                guard let s = String(bytes: slice, encoding: .utf8) else {
                    throw NveEventRingError.invalidUtf8
                }
                return s
            }

            var events: [NveEvent] = []
            events.reserveCapacity(min(eventCount, 4096))

            for i in 0 ..< eventCount {
                let base = wireHeaderBytes + i * eventRecordBytes
                let ts = UInt64(littleEndian: raw.loadUnaligned(fromByteOffset: base + 0, as: UInt64.self))
                let table = UInt64(littleEndian: raw.loadUnaligned(fromByteOffset: base + 8, as: UInt64.self))
                let surfaceId = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 16, as: UInt32.self))
                let nodeId = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 20, as: UInt32.self))
                let eventId = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 24, as: UInt32.self))
                let flags = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 28, as: UInt32.self))
                let p0 = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 32, as: UInt32.self))
                let p1 = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 36, as: UInt32.self))
                let p2 = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 40, as: UInt32.self))
                let p3 = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 44, as: UInt32.self))
                let p4 = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 48, as: UInt32.self))
                let p5 = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 52, as: UInt32.self))
                let bytesOff = Int(UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 56, as: UInt32.self)))
                let bytesLenRec = Int(UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: base + 60, as: UInt32.self)))
                let arenaSlice = try sliceArena(off: bytesOff, len: bytesLenRec)

                var name: String?
                let kind: NveEventKind
                let payload: NveEventPayload

                if table == UInt64(NVE_TABLE_CUSTOM) {
                    kind = .custom
                    let nlen = Int(p0)
                    guard nlen <= arenaSlice.count else {
                        throw NveEventRingError.invalidBounds
                    }
                    let nameBuf = UnsafeRawBufferPointer(rebasing: arenaSlice[0 ..< nlen])
                    name = try decodeUtf8Strict(nameBuf)
                    payload = .binValue(Data(arenaSlice[nlen ..< arenaSlice.count]))
                } else if table == UInt64(NVE_TABLE_INTERACTION) {
                    switch eventId {
                    case UInt32(NVE_INTERACTION_EVENT_PRESS.rawValue):
                        kind = .press
                        payload = .point(x: Float(bitPattern: p0), y: Float(bitPattern: p1))
                    case UInt32(NVE_INTERACTION_EVENT_LONG_PRESS.rawValue):
                        kind = .longPress
                        payload = .point(x: Float(bitPattern: p0), y: Float(bitPattern: p1))
                    case UInt32(NVE_INTERACTION_EVENT_FOCUS.rawValue):
                        kind = .focus
                        payload = .empty
                    case UInt32(NVE_INTERACTION_EVENT_BLUR.rawValue):
                        kind = .blur
                        payload = .empty
                    case UInt32(NVE_INTERACTION_EVENT_KEY_DOWN.rawValue),
                         UInt32(NVE_INTERACTION_EVENT_KEY_UP.rawValue):
                        kind = eventId == UInt32(NVE_INTERACTION_EVENT_KEY_DOWN.rawValue) ? .keyDown : .keyUp
                        let key: String
                        let code: String
                        if (flags & UInt32(NVE_EVENT_FLAG_INLINE_KEY_UTF8)) != 0 {
                            let klen = Int(p1)
                            guard klen <= Int(NVE_EVENT_INLINE_KEY_CAP) else {
                                throw NveEventRingError.invalidBounds
                            }
                            var kb: [UInt8] = []
                            kb.reserveCapacity(16)
                            for w in [p2, p3, p4, p5] {
                                kb.append(UInt8(truncatingIfNeeded: w))
                                kb.append(UInt8(truncatingIfNeeded: w >> 8))
                                kb.append(UInt8(truncatingIfNeeded: w >> 16))
                                kb.append(UInt8(truncatingIfNeeded: w >> 24))
                            }
                            guard let decodedKey = String(bytes: kb.prefix(klen), encoding: .utf8) else {
                                throw NveEventRingError.invalidUtf8
                            }
                            key = decodedKey
                            code = try decodeUtf8Strict(arenaSlice)
                        } else {
                            let klen = Int(p1)
                            guard klen <= arenaSlice.count else {
                                throw NveEventRingError.invalidBounds
                            }
                            key = try decodeUtf8Strict(UnsafeRawBufferPointer(rebasing: arenaSlice[0 ..< klen]))
                            code = try decodeUtf8Strict(UnsafeRawBufferPointer(rebasing: arenaSlice[klen ..< arenaSlice.count]))
                        }
                        payload = .key(
                            key: key,
                            code: code,
                            modifiers: UInt8(flags & 0x1F),
                            keyCode: p0,
                            isComposing: (flags & UInt32(NVE_EVENT_FLAG_COMPOSING)) != 0
                        )
                    case UInt32(NVE_INTERACTION_EVENT_POINTER_DOWN.rawValue),
                         UInt32(NVE_INTERACTION_EVENT_POINTER_MOVE.rawValue),
                         UInt32(NVE_INTERACTION_EVENT_POINTER_UP.rawValue),
                         UInt32(NVE_INTERACTION_EVENT_POINTER_ENTER.rawValue),
                         UInt32(NVE_INTERACTION_EVENT_POINTER_LEAVE.rawValue):
                        switch eventId {
                        case UInt32(NVE_INTERACTION_EVENT_POINTER_DOWN.rawValue): kind = .pointerDown
                        case UInt32(NVE_INTERACTION_EVENT_POINTER_MOVE.rawValue): kind = .pointerMove
                        case UInt32(NVE_INTERACTION_EVENT_POINTER_UP.rawValue): kind = .pointerUp
                        case UInt32(NVE_INTERACTION_EVENT_POINTER_ENTER.rawValue): kind = .pointerEnter
                        default: kind = .pointerLeave
                        }
                        let btn = UInt8((flags & UInt32(NVE_EVENT_FLAG_BUTTON_MASK)) >> UInt32(NVE_EVENT_FLAG_BUTTON_SHIFT))
                        let cap: UInt8 = (flags & UInt32(NVE_EVENT_FLAG_CAPTURE)) != 0 ? 0x08 : 0
                        payload = .pointer(
                            x: Float(bitPattern: p0),
                            y: Float(bitPattern: p1),
                            modifiers: UInt8(flags & 0x1F),
                            flags: btn | cap
                        )
                    case UInt32(NVE_INTERACTION_EVENT_WHEEL.rawValue):
                        kind = .wheel
                        payload = .wheel(
                            deltaX: Float(bitPattern: p0),
                            deltaY: Float(bitPattern: p1),
                            modifiers: UInt8(flags & 0x1F)
                        )
                    default:
                        throw NveEventRingError.invalidEventTable(table, eventId)
                    }
                } else if table == UInt64(NVE_TABLE_TEXT_INPUT) {
                    switch eventId {
                    case UInt32(NVE_TEXT_INPUT_EVENT_CHANGE.rawValue): kind = .textChange
                    case UInt32(NVE_TEXT_INPUT_EVENT_SUBMIT.rawValue): kind = .submit
                    case UInt32(NVE_TEXT_INPUT_EVENT_COMPOSITION_START.rawValue): kind = .compositionStart
                    case UInt32(NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE.rawValue): kind = .compositionUpdate
                    case UInt32(NVE_TEXT_INPUT_EVENT_COMPOSITION_END.rawValue): kind = .compositionEnd
                    default:
                        throw NveEventRingError.invalidEventTable(table, eventId)
                    }
                    payload = .text(try decodeUtf8Strict(arenaSlice))
                } else if table == UInt64(NVE_TABLE_SCROLL) && eventId == UInt32(NVE_SCROLL_EVENT_SCROLL.rawValue) {
                    kind = .scroll
                    payload = .scroll(
                        x: Float(bitPattern: p0),
                        y: Float(bitPattern: p1),
                        contentWidth: Float(bitPattern: p2),
                        contentHeight: Float(bitPattern: p3)
                    )
                } else if table == UInt64(NVE_TABLE_LAYOUT) && eventId == UInt32(NVE_LAYOUT_EVENT_LAYOUT.rawValue) {
                    kind = .layout
                    payload = .layout(
                        x: Float(bitPattern: p0),
                        y: Float(bitPattern: p1),
                        width: Float(bitPattern: p2),
                        height: Float(bitPattern: p3)
                    )
                } else {
                    throw NveEventRingError.invalidEventTable(table, eventId)
                }

                events.append(
                    NveEvent(
                        surfaceId: surfaceId,
                        nodeId: nodeId,
                        timestampUs: ts,
                        kind: kind,
                        name: name,
                        payload: payload
                    )
                )
            }
            return events
        }
    }

    /// Best-effort decoder returning `[]` on any truncated or malformed `NVEV` v2 payload.
    public static func decode(_ data: Data) -> [NveEvent] {
        (try? decodeStrict(data)) ?? []
    }

    private static func appendLE<T: FixedWidthInteger>(_ value: T, to data: inout Data) {
        withUnsafeBytes(of: value.littleEndian) { raw in
            data.append(contentsOf: raw)
        }
    }

    /// Encodes a batch of typed events into the binary `NVEV` v2 wire format.
    public static func encode(_ events: [NveEvent]) -> Data {
        var rawRecords = Data(count: events.count * eventRecordBytes)
        var byteArena = Data()
        if !events.isEmpty {
            rawRecords.withUnsafeMutableBytes { recBuf in
                for (idx, event) in events.enumerated() {
                    NveDirectEventWriter.encodeRecord(
                        event,
                        into: recBuf,
                        offset: idx * eventRecordBytes,
                        byteArena: &byteArena
                    )
                }
            }
        }
        var out = Data()
        out.reserveCapacity(wireHeaderBytes + rawRecords.count + byteArena.count)
        out.append(contentsOf: [0x4E, 0x56, 0x45, 0x56])
        appendLE(wireVersion, to: &out)
        appendLE(UInt32(events.count), to: &out)
        appendLE(UInt32(byteArena.count), to: &out)
        out.append(rawRecords)
        out.append(byteArena)
        return out
    }

    public static func encodeCustomEvent(
        surfaceId: UInt32,
        nodeId: UInt32 = 1,
        name: String,
        payload: Data,
        timestampUs: UInt64 = UInt64(Date().timeIntervalSince1970 * 1_000_000)
    ) -> Data {
        encode([
            NveEvent(
                surfaceId: surfaceId,
                nodeId: nodeId,
                timestampUs: timestampUs,
                kind: .custom,
                name: name,
                payload: .binValue(payload)
            ),
        ])
    }

    public static func encodeCustomEvent(
        surfaceId: UInt32,
        nodeId: UInt32 = 1,
        name: String,
        value: NveBinValue,
        timestampUs: UInt64 = UInt64(Date().timeIntervalSince1970 * 1_000_000)
    ) throws -> Data {
        let encodedValue = try value.encode()
        return encodeCustomEvent(
            surfaceId: surfaceId,
            nodeId: nodeId,
            name: name,
            payload: encodedValue,
            timestampUs: timestampUs
        )
    }
}
