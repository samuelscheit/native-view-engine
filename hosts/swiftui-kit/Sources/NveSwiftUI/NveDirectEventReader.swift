#if canImport(CNveAbi)
import CNveAbi
#endif
import Foundation

// MARK: - Zero-Copy Direct C-ABI Event Batch Reader

public extension NveDirectEventWriter {
    static func decodeBatch(_ batch: NveEventBatch) -> [NveEvent] {
        guard let evPtr = batch.events, batch.event_count > 0 else { return [] }
        let count = Int(batch.event_count)
        let arenaLen = Int(batch.bytes_len)
        let rawBase = UnsafeRawPointer(evPtr)
        var out: [NveEvent] = []
        out.reserveCapacity(min(count, 4096))

        func sliceArena(off: Int, len: Int) -> Data {
            guard len > 0, let bp = batch.bytes, off >= 0, off + len <= arenaLen else {
                return Data()
            }
            return Data(bytes: bp.advanced(by: off), count: len)
        }

        for i in 0 ..< count {
            let base = i * 64
            let ts = UInt64(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 0, as: UInt64.self))
            let table = UInt64(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 8, as: UInt64.self))
            let surfaceId = UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 16, as: UInt32.self))
            let nodeId = UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 20, as: UInt32.self))
            let eventId = UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 24, as: UInt32.self))
            let flags = UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 28, as: UInt32.self))
            let p0 = UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 32, as: UInt32.self))
            let p1 = UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 36, as: UInt32.self))
            let p2 = UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 40, as: UInt32.self))
            let p3 = UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 44, as: UInt32.self))
            let p4 = UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 48, as: UInt32.self))
            let p5 = UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 52, as: UInt32.self))
            let bytesOff = Int(UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 56, as: UInt32.self)))
            let bytesLen = Int(UInt32(littleEndian: rawBase.loadUnaligned(fromByteOffset: base + 60, as: UInt32.self)))

            var name: String? = nil
            let kind: NveEventKind
            let payload: NveEventPayload

            if table == UInt64(NVE_TABLE_CUSTOM) {
                kind = .custom
                let raw = sliceArena(off: bytesOff, len: bytesLen)
                let nlen = min(Int(p0), raw.count)
                name = String(decoding: raw.prefix(nlen), as: UTF8.self)
                payload = .binValue(Data(raw.dropFirst(nlen)))
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
                case UInt32(NVE_INTERACTION_EVENT_KEY_DOWN.rawValue), UInt32(NVE_INTERACTION_EVENT_KEY_UP.rawValue):
                    kind = eventId == UInt32(NVE_INTERACTION_EVENT_KEY_DOWN.rawValue) ? .keyDown : .keyUp
                    let key: String
                    let code: String
                    if (flags & UInt32(NVE_EVENT_FLAG_INLINE_KEY_UTF8)) != 0 {
                        let klen = min(Int(p1), 16)
                        var kb: [UInt8] = []
                        kb.reserveCapacity(16)
                        for w in [p2, p3, p4, p5] {
                            kb.append(UInt8(truncatingIfNeeded: w))
                            kb.append(UInt8(truncatingIfNeeded: w >> 8))
                            kb.append(UInt8(truncatingIfNeeded: w >> 16))
                            kb.append(UInt8(truncatingIfNeeded: w >> 24))
                        }
                        key = String(decoding: kb.prefix(klen), as: UTF8.self)
                        code = String(decoding: sliceArena(off: bytesOff, len: bytesLen), as: UTF8.self)
                    } else {
                        let raw = sliceArena(off: bytesOff, len: bytesLen)
                        let klen = min(Int(p1), raw.count)
                        key = String(decoding: raw.prefix(klen), as: UTF8.self)
                        code = String(decoding: raw.dropFirst(klen), as: UTF8.self)
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
                    let map: [UInt32: NveEventKind] = [
                        UInt32(NVE_INTERACTION_EVENT_POINTER_DOWN.rawValue): .pointerDown,
                        UInt32(NVE_INTERACTION_EVENT_POINTER_MOVE.rawValue): .pointerMove,
                        UInt32(NVE_INTERACTION_EVENT_POINTER_UP.rawValue): .pointerUp,
                        UInt32(NVE_INTERACTION_EVENT_POINTER_ENTER.rawValue): .pointerEnter,
                        UInt32(NVE_INTERACTION_EVENT_POINTER_LEAVE.rawValue): .pointerLeave,
                    ]
                    kind = map[eventId] ?? .pointerMove
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
                    continue
                }
            } else if table == UInt64(NVE_TABLE_TEXT_INPUT) {
                switch eventId {
                case UInt32(NVE_TEXT_INPUT_EVENT_CHANGE.rawValue): kind = .textChange
                case UInt32(NVE_TEXT_INPUT_EVENT_SUBMIT.rawValue): kind = .submit
                case UInt32(NVE_TEXT_INPUT_EVENT_COMPOSITION_START.rawValue): kind = .compositionStart
                case UInt32(NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE.rawValue): kind = .compositionUpdate
                case UInt32(NVE_TEXT_INPUT_EVENT_COMPOSITION_END.rawValue): kind = .compositionEnd
                default: continue
                }
                payload = .text(String(decoding: sliceArena(off: bytesOff, len: bytesLen), as: UTF8.self))
            } else if table == UInt64(NVE_TABLE_SCROLL) {
                kind = .scroll
                payload = .scroll(
                    x: Float(bitPattern: p0),
                    y: Float(bitPattern: p1),
                    contentWidth: Float(bitPattern: p2),
                    contentHeight: Float(bitPattern: p3)
                )
            } else if table == UInt64(NVE_TABLE_LAYOUT) {
                kind = .layout
                payload = .layout(
                    x: Float(bitPattern: p0),
                    y: Float(bitPattern: p1),
                    width: Float(bitPattern: p2),
                    height: Float(bitPattern: p3)
                )
            } else {
                kind = .custom
                payload = .binValue(sliceArena(off: bytesOff, len: bytesLen))
            }

            out.append(
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
        return out
    }
}
