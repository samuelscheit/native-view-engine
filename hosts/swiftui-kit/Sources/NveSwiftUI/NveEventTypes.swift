import Foundation

// MARK: - Core Event & Backend Types

/// Rendering backend used for an embedded NVE surface inside SwiftUI.
public enum NveRenderBackend: String, Sendable, Hashable {
    case fabric
    case skia
}

/// Native event kind mirroring `NativeEventKind` in `packages/core/abi/src/events.rs`.
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
    case invalidEventKind(UInt8)
    case invalidPayloadTag(UInt8)
    case invalidUtf8
    case trailingBytes(Int)
}

// MARK: - Binary EventRing Codec (matches `packages/core/abi/src/events.rs`, binary FFI)

public enum NveEventRingCodec {
    /// Strictly decodes a binary `EventRing` buffer, throwing on truncation, unknown enums,
    /// invalid UTF-8, or trailing garbage bytes.
    public static func decodeStrict(_ data: Data) throws -> [NveEvent] {
        guard data.count >= 4 else {
            throw NveEventRingError.truncated
        }
        return try data.withUnsafeBytes { raw in
            var pos = 0

            func readU8() throws -> UInt8 {
                guard pos < raw.count else { throw NveEventRingError.truncated }
                let value = raw[pos]
                pos += 1
                return value
            }

            func readU16() throws -> UInt16 {
                guard pos + 2 <= raw.count else { throw NveEventRingError.truncated }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt16.self)
                pos += 2
                return UInt16(littleEndian: value)
            }

            func readU32() throws -> UInt32 {
                guard pos + 4 <= raw.count else { throw NveEventRingError.truncated }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt32.self)
                pos += 4
                return UInt32(littleEndian: value)
            }

            func readU64() throws -> UInt64 {
                guard pos + 8 <= raw.count else { throw NveEventRingError.truncated }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt64.self)
                pos += 8
                return UInt64(littleEndian: value)
            }

            func readF32() throws -> Float {
                Float(bitPattern: try readU32())
            }

            func readUtf8(length: Int) throws -> String {
                guard length >= 0, length <= raw.count - pos else {
                    throw NveEventRingError.truncated
                }
                let slice = UnsafeRawBufferPointer(rebasing: raw[pos ..< (pos + length)])
                pos += length
                guard let str = String(bytes: slice, encoding: .utf8) else {
                    throw NveEventRingError.invalidUtf8
                }
                return str
            }

            let count = Int(try readU32())
            guard count <= (raw.count - pos) / 18 else {
                throw NveEventRingError.truncated
            }

            var events: [NveEvent] = []
            events.reserveCapacity(min(count, 4096))

            for _ in 0 ..< count {
                let surfaceId = try readU32()
                let nodeId = try readU32()
                let timestampUs = try readU64()
                let kindByte = try readU8()
                guard let kind = NveEventKind(rawValue: kindByte) else {
                    throw NveEventRingError.invalidEventKind(kindByte)
                }
                let payloadTag = try readU8()

                var name: String?
                let payload: NveEventPayload

                switch payloadTag {
                case 0:
                    payload = .empty
                case 1:
                    let x = try readF32()
                    let y = try readF32()
                    payload = .point(x: x, y: y)
                case 2:
                    let x = try readF32()
                    let y = try readF32()
                    let cw = try readF32()
                    let ch = try readF32()
                    payload = .scroll(x: x, y: y, contentWidth: cw, contentHeight: ch)
                case 3:
                    let len = Int(try readU32())
                    payload = .text(try readUtf8(length: len))
                case 4:
                    let x = try readF32()
                    let y = try readF32()
                    let w = try readF32()
                    let h = try readF32()
                    payload = .layout(x: x, y: y, width: w, height: h)
                case 5:
                    let nameLen = Int(try readU16())
                    name = try readUtf8(length: nameLen)
                    let dataLen = Int(try readU32())
                    guard dataLen <= raw.count - pos else {
                        throw NveEventRingError.truncated
                    }
                    let slice = Data(raw[pos ..< (pos + dataLen)])
                    pos += dataLen
                    payload = .binValue(slice)
                case 6:
                    let modifiers = try readU8()
                    let isComposing = try readU8() != 0
                    let keyCode = try readU32()
                    let keyLen = Int(try readU16())
                    let key = try readUtf8(length: keyLen)
                    let codeLen = Int(try readU16())
                    let code = try readUtf8(length: codeLen)
                    payload = .key(key: key, code: code, modifiers: modifiers, keyCode: keyCode, isComposing: isComposing)
                case 7:
                    let x = try readF32()
                    let y = try readF32()
                    let modifiers = try readU8()
                    let flags = try readU8()
                    payload = .pointer(x: x, y: y, modifiers: modifiers, flags: flags)
                case 8:
                    let dx = try readF32()
                    let dy = try readF32()
                    let modifiers = try readU8()
                    payload = .wheel(deltaX: dx, deltaY: dy, modifiers: modifiers)
                default:
                    throw NveEventRingError.invalidPayloadTag(payloadTag)
                }

                events.append(
                    NveEvent(
                        surfaceId: surfaceId,
                        nodeId: nodeId,
                        timestampUs: timestampUs,
                        kind: kind,
                        name: name,
                        payload: payload
                    )
                )
            }

            if pos != raw.count {
                throw NveEventRingError.trailingBytes(raw.count - pos)
            }
            return events
        }
    }

    /// Best-effort decoder that returns all valid events decoded before any truncation or corruption.
    public static func decode(_ data: Data) -> [NveEvent] {
        guard data.count >= 4 else { return [] }
        return data.withUnsafeBytes { raw in
            var pos = 0

            func readU8() -> UInt8? {
                guard pos < raw.count else { return nil }
                let value = raw[pos]
                pos += 1
                return value
            }

            func readU16() -> UInt16? {
                guard pos + 2 <= raw.count else { return nil }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt16.self)
                pos += 2
                return UInt16(littleEndian: value)
            }

            func readU32() -> UInt32? {
                guard pos + 4 <= raw.count else { return nil }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt32.self)
                pos += 4
                return UInt32(littleEndian: value)
            }

            func readU64() -> UInt64? {
                guard pos + 8 <= raw.count else { return nil }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt64.self)
                pos += 8
                return UInt64(littleEndian: value)
            }

            func readF32() -> Float? {
                readU32().map(Float.init(bitPattern:))
            }

            guard let count = readU32() else { return [] }
            var events: [NveEvent] = []
            events.reserveCapacity(Int(min(count, 4096)))

            for _ in 0 ..< count {
                guard
                    let surfaceId = readU32(),
                    let nodeId = readU32(),
                    let timestampUs = readU64(),
                    let kindByte = readU8(),
                    let payloadTag = readU8()
                else {
                    break
                }

                let kind = NveEventKind(rawValue: kindByte) ?? .custom
                var name: String?
                let payload: NveEventPayload

                switch payloadTag {
                case 0:
                    payload = .empty
                case 1:
                    guard let x = readF32(), let y = readF32() else { return events }
                    payload = .point(x: x, y: y)
                case 2:
                    guard let x = readF32(), let y = readF32(), let cw = readF32(), let ch = readF32() else {
                        return events
                    }
                    payload = .scroll(x: x, y: y, contentWidth: cw, contentHeight: ch)
                case 3:
                    guard let len = readU32(), Int(len) <= raw.count - pos else { return events }
                    let slice = UnsafeRawBufferPointer(rebasing: raw[pos ..< (pos + Int(len))])
                    pos += Int(len)
                    guard let text = String(bytes: slice, encoding: .utf8) else { return events }
                    payload = .text(text)
                case 4:
                    guard let x = readF32(), let y = readF32(), let w = readF32(), let h = readF32() else {
                        return events
                    }
                    payload = .layout(x: x, y: y, width: w, height: h)
                case 5:
                    guard let nameLen = readU16(), Int(nameLen) <= raw.count - pos else { return events }
                    let nameSlice = UnsafeRawBufferPointer(rebasing: raw[pos ..< (pos + Int(nameLen))])
                    pos += Int(nameLen)
                    guard let decodedName = String(bytes: nameSlice, encoding: .utf8) else {
                        return events
                    }
                    name = decodedName
                    guard let dataLen = readU32(), Int(dataLen) <= raw.count - pos else { return events }
                    payload = .binValue(Data(raw[pos ..< (pos + Int(dataLen))]))
                    pos += Int(dataLen)
                case 6:
                    guard
                        let modifiers = readU8(),
                        let composingByte = readU8(),
                        let keyCode = readU32(),
                        let keyLen = readU16(), Int(keyLen) <= raw.count - pos
                    else { return events }
                    let keySlice = UnsafeRawBufferPointer(rebasing: raw[pos ..< (pos + Int(keyLen))])
                    pos += Int(keyLen)
                    guard
                        let key = String(bytes: keySlice, encoding: .utf8),
                        let codeLen = readU16(), Int(codeLen) <= raw.count - pos
                    else { return events }
                    let codeSlice = UnsafeRawBufferPointer(rebasing: raw[pos ..< (pos + Int(codeLen))])
                    pos += Int(codeLen)
                    guard let code = String(bytes: codeSlice, encoding: .utf8) else { return events }
                    payload = .key(key: key, code: code, modifiers: modifiers, keyCode: keyCode, isComposing: composingByte != 0)
                case 7:
                    guard let x = readF32(), let y = readF32(), let modifiers = readU8(), let flags = readU8() else {
                        return events
                    }
                    payload = .pointer(x: x, y: y, modifiers: modifiers, flags: flags)
                case 8:
                    guard let dx = readF32(), let dy = readF32(), let modifiers = readU8() else { return events }
                    payload = .wheel(deltaX: dx, deltaY: dy, modifiers: modifiers)
                default:
                    return events
                }

                events.append(
                    NveEvent(
                        surfaceId: surfaceId,
                        nodeId: nodeId,
                        timestampUs: timestampUs,
                        kind: kind,
                        name: name,
                        payload: payload
                    )
                )
            }
            return events
        }
    }

    private static func appendLE<T: FixedWidthInteger>(_ value: T, to data: inout Data) {
        withUnsafeBytes(of: value.littleEndian) { raw in
            data.append(contentsOf: raw)
        }
    }

    private static func appendF32(_ value: Float, to data: inout Data) {
        appendLE(value.bitPattern, to: &data)
    }

    /// Encodes a batch of typed events into the binary `EventRing` wire format.
    public static func encode(_ events: [NveEvent]) -> Data {
        var out = Data()
        out.reserveCapacity(4 + events.count * 32)
        appendLE(UInt32(events.count), to: &out)

        for event in events {
            appendLE(event.surfaceId, to: &out)
            appendLE(event.nodeId, to: &out)
            appendLE(event.timestampUs, to: &out)
            out.append(event.kind.rawValue)

            switch event.payload {
            case .empty:
                out.append(0)
            case let .point(x, y):
                out.append(1)
                appendF32(x, to: &out)
                appendF32(y, to: &out)
            case let .scroll(x, y, contentWidth, contentHeight):
                out.append(2)
                appendF32(x, to: &out)
                appendF32(y, to: &out)
                appendF32(contentWidth, to: &out)
                appendF32(contentHeight, to: &out)
            case let .text(str):
                out.append(3)
                let bytes = Data(str.utf8)
                appendLE(UInt32(bytes.count), to: &out)
                out.append(bytes)
            case let .layout(x, y, width, height):
                out.append(4)
                appendF32(x, to: &out)
                appendF32(y, to: &out)
                appendF32(width, to: &out)
                appendF32(height, to: &out)
            case let .binValue(payload):
                out.append(5)
                let nameBytes = Data((event.name ?? "").utf8)
                appendLE(UInt16(nameBytes.count), to: &out)
                out.append(nameBytes)
                appendLE(UInt32(payload.count), to: &out)
                out.append(payload)
            case let .key(key, code, modifiers, keyCode, isComposing):
                out.append(6)
                out.append(modifiers)
                out.append(isComposing ? 1 : 0)
                appendLE(keyCode, to: &out)
                let keyBytes = Data(key.utf8)
                appendLE(UInt16(keyBytes.count), to: &out)
                out.append(keyBytes)
                let codeBytes = Data(code.utf8)
                appendLE(UInt16(codeBytes.count), to: &out)
                out.append(codeBytes)
            case let .pointer(x, y, modifiers, flags):
                out.append(7)
                appendF32(x, to: &out)
                appendF32(y, to: &out)
                out.append(modifiers)
                out.append(flags)
            case let .wheel(deltaX, deltaY, modifiers):
                out.append(8)
                appendF32(deltaX, to: &out)
                appendF32(deltaY, to: &out)
                out.append(modifiers)
            }
        }
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
