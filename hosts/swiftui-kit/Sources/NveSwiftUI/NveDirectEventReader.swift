#if canImport(CNveAbi)
import CNveAbi
import NveHost
#endif
import Foundation

// MARK: - Zero-Copy Direct C-ABI Event Batch Reader

public extension NveDirectEventWriter {
    static func decodeBatch(_ batch: NveEventBatch) -> [NveEvent] {
        guard let ptr = batch.ptr, batch.len > 0 else { return [] }
        let count = Int(batch.len)
        var out: [NveEvent] = []
        out.reserveCapacity(min(count, 4096))
        for i in 0 ..< count {
            let src = ptr[i]
            let kind = NveEventKind(rawValue: src.kind) ?? .custom
            var name: String?
            let payload: NveEventPayload
            switch src.payload_tag {
            case UInt8(NVE_EVENT_PAYLOAD_EMPTY.rawValue):
                payload = .empty
            case UInt8(NVE_EVENT_PAYLOAD_POINT.rawValue):
                payload = .point(x: src.x, y: src.y)
            case UInt8(NVE_EVENT_PAYLOAD_SCROLL.rawValue):
                payload = .scroll(x: src.x, y: src.y, contentWidth: src.width, contentHeight: src.height)
            case UInt8(NVE_EVENT_PAYLOAD_TEXT.rawValue):
                payload = .text(NveDirectPropsWriter.decodeStrSlice(src.text))
            case UInt8(NVE_EVENT_PAYLOAD_LAYOUT.rawValue):
                payload = .layout(x: src.x, y: src.y, width: src.width, height: src.height)
            case UInt8(NVE_EVENT_PAYLOAD_CUSTOM.rawValue):
                name = NveDirectPropsWriter.decodeStrSlice(src.name)
                if src.value.tag == UInt32(NVE_VAL_BYTES.rawValue) {
                    let b = src.value.data.bytes_val
                    payload = .binValue(b.ptr.map { Data(bytes: $0, count: b.len) } ?? Data())
                } else if src.value.tag != UInt32(NVE_VAL_NULL.rawValue) {
                    let encoded = (try? NveDirectPropsWriter.decodeCValue(src.value).encode()) ?? Data()
                    payload = .binValue(encoded)
                } else {
                    payload = .binValue(Data())
                }
            case UInt8(NVE_EVENT_PAYLOAD_KEY.rawValue):
                payload = .key(
                    key: NveDirectPropsWriter.decodeStrSlice(src.text),
                    code: NveDirectPropsWriter.decodeStrSlice(src.name),
                    modifiers: src.modifiers,
                    keyCode: src.key_code,
                    isComposing: (src.event_flags & UInt8(NVE_EVENT_FLAG_COMPOSING)) != 0
                )
            case UInt8(NVE_EVENT_PAYLOAD_POINTER.rawValue):
                payload = .pointer(x: src.x, y: src.y, modifiers: src.modifiers, flags: src.event_flags)
            case UInt8(NVE_EVENT_PAYLOAD_WHEEL.rawValue):
                payload = .wheel(deltaX: src.x, deltaY: src.y, modifiers: src.modifiers)
            default:
                continue
            }
            out.append(
                NveEvent(
                    surfaceId: src.surface_id,
                    nodeId: src.node_id,
                    timestampUs: src.timestamp_us,
                    kind: kind,
                    name: name,
                    payload: payload
                )
            )
        }
        return out
    }
}
