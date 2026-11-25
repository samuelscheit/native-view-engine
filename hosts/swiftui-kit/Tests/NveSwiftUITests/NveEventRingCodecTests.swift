import Foundation
import NveSwiftUI
import XCTest

final class NveEventRingCodecTests: XCTestCase {
    private struct CounterPayload: Codable, Equatable {
        let count: Int
        let label: String
    }

    private func appendLE<T: FixedWidthInteger>(_ value: T, to data: inout Data) {
        withUnsafeBytes(of: value.littleEndian) { raw in
            data.append(contentsOf: raw)
        }
    }

    private func appendF32(_ value: Float, to data: inout Data) {
        appendLE(value.bitPattern, to: &data)
    }

    private func appendEventHeader(
        surfaceId: UInt32,
        nodeId: UInt32,
        timestampUs: UInt64,
        kind: UInt8,
        payloadTag: UInt8,
        to data: inout Data
    ) {
        appendLE(surfaceId, to: &data)
        appendLE(nodeId, to: &data)
        appendLE(timestampUs, to: &data)
        data.append(kind)
        data.append(payloadTag)
    }

    func testEnumRawValues() {
        XCTAssertEqual(NveRenderBackend.fabric.rawValue, "fabric")
        XCTAssertEqual(NveRenderBackend.skia.rawValue, "skia")

        XCTAssertEqual(NveEventKind.press.rawValue, 1)
        XCTAssertEqual(NveEventKind.longPress.rawValue, 2)
        XCTAssertEqual(NveEventKind.scroll.rawValue, 3)
        XCTAssertEqual(NveEventKind.textChange.rawValue, 4)
        XCTAssertEqual(NveEventKind.focus.rawValue, 5)
        XCTAssertEqual(NveEventKind.blur.rawValue, 6)
        XCTAssertEqual(NveEventKind.layout.rawValue, 7)
        XCTAssertEqual(NveEventKind.custom.rawValue, 8)
    }

    func testEventPayloadDecodingHelpers() throws {
        let expected = CounterPayload(count: 7, label: "clicks")
        let data = try JSONEncoder().encode(expected)
        let event = NveEvent(
            surfaceId: 3,
            nodeId: 9,
            timestampUs: 99_000,
            kind: .custom,
            name: "onIncrement",
            payload: data
        )

        let decodedDict = try XCTUnwrap(event.decodePayloadJSON())
        XCTAssertEqual(decodedDict["count"] as? Int, 7)
        XCTAssertEqual(decodedDict["label"] as? String, "clicks")
        XCTAssertEqual(try event.decodePayload(CounterPayload.self), expected)

        let emptyEvent = NveEvent(surfaceId: 1, nodeId: 1, timestampUs: 0, kind: .focus)
        XCTAssertNil(emptyEvent.decodePayloadJSON())

        let binaryEvent = NveEvent(
            surfaceId: 1,
            nodeId: 1,
            timestampUs: 0,
            kind: .custom,
            name: "bin",
            payload: Data([0xFF, 0x00, 0xAB])
        )
        XCTAssertNil(binaryEvent.decodePayloadJSON())
        XCTAssertThrowsError(try binaryEvent.decodePayload(CounterPayload.self))
    }

    func testEncodeAndDecodeCustomEventRoundtrip() throws {
        let payload = try JSONSerialization.data(withJSONObject: ["delta": 2, "source": "button"])
        let encoded = NveEventRingCodec.encodeCustomEvent(
            surfaceId: 11,
            nodeId: 22,
            name: "onCounterChange_π",
            payload: payload,
            timestampUs: 1_234_567_890
        )

        let decoded = NveEventRingCodec.decode(encoded)
        XCTAssertEqual(decoded.count, 1)
        let ev = try XCTUnwrap(decoded.first)
        XCTAssertEqual(ev.surfaceId, 11)
        XCTAssertEqual(ev.nodeId, 22)
        XCTAssertEqual(ev.timestampUs, 1_234_567_890)
        XCTAssertEqual(ev.kind, .custom)
        XCTAssertEqual(ev.name, "onCounterChange_π")
        XCTAssertEqual(ev.payload, payload)
        let json = try XCTUnwrap(ev.decodePayloadJSON())
        XCTAssertEqual(json["delta"] as? Int, 2)
        XCTAssertEqual(json["source"] as? String, "button")
    }

    func testDecodeAllPayloadVariantsInSingleBatch() throws {
        var ring = Data()
        appendLE(UInt32(6), to: &ring)

        // 0: Empty (Focus)
        appendEventHeader(
            surfaceId: 1, nodeId: 10, timestampUs: 100,
            kind: NveEventKind.focus.rawValue, payloadTag: 0, to: &ring
        )

        // 1: Point (Press)
        appendEventHeader(
            surfaceId: 1, nodeId: 11, timestampUs: 200,
            kind: NveEventKind.press.rawValue, payloadTag: 1, to: &ring
        )
        appendF32(42.5, to: &ring)
        appendF32(18.25, to: &ring)

        // 2: Scroll
        appendEventHeader(
            surfaceId: 2, nodeId: 12, timestampUs: 300,
            kind: NveEventKind.scroll.rawValue, payloadTag: 2, to: &ring
        )
        appendF32(10.0, to: &ring)
        appendF32(250.5, to: &ring)
        appendF32(390.0, to: &ring)
        appendF32(1200.0, to: &ring)

        // 3: Text (UTF-8)
        let textBytes = Data("Hello 🌍".utf8)
        appendEventHeader(
            surfaceId: 2, nodeId: 13, timestampUs: 400,
            kind: NveEventKind.textChange.rawValue, payloadTag: 3, to: &ring
        )
        appendLE(UInt32(textBytes.count), to: &ring)
        ring.append(textBytes)

        // 4: Layout
        appendEventHeader(
            surfaceId: 3, nodeId: 14, timestampUs: 500,
            kind: NveEventKind.layout.rawValue, payloadTag: 4, to: &ring
        )
        appendF32(4.0, to: &ring)
        appendF32(8.0, to: &ring)
        appendF32(320.0, to: &ring)
        appendF32(180.0, to: &ring)

        // 5: Custom (raw binary)
        let customName = Data("onBlob".utf8)
        let customBytes = Data([9, 8, 7, 6])
        appendEventHeader(
            surfaceId: 3, nodeId: 15, timestampUs: 600,
            kind: NveEventKind.custom.rawValue, payloadTag: 5, to: &ring
        )
        appendLE(UInt16(customName.count), to: &ring)
        ring.append(customName)
        appendLE(UInt32(customBytes.count), to: &ring)
        ring.append(customBytes)

        let events = NveEventRingCodec.decode(ring)
        XCTAssertEqual(events.count, 6)

        XCTAssertEqual(events[0].kind, .focus)
        XCTAssertTrue(events[0].payload.isEmpty)

        XCTAssertEqual(events[1].kind, .press)
        let pt = try XCTUnwrap(events[1].decodePayloadJSON())
        XCTAssertEqual((pt["x"] as? NSNumber)?.floatValue ?? -1, 42.5, accuracy: 0.001)
        XCTAssertEqual((pt["y"] as? NSNumber)?.floatValue ?? -1, 18.25, accuracy: 0.001)

        XCTAssertEqual(events[2].kind, .scroll)
        let sc = try XCTUnwrap(events[2].decodePayloadJSON())
        XCTAssertEqual((sc["x"] as? NSNumber)?.floatValue ?? -1, 10.0, accuracy: 0.001)
        XCTAssertEqual((sc["y"] as? NSNumber)?.floatValue ?? -1, 250.5, accuracy: 0.001)
        XCTAssertEqual((sc["contentWidth"] as? NSNumber)?.floatValue ?? -1, 390.0, accuracy: 0.001)
        XCTAssertEqual((sc["contentHeight"] as? NSNumber)?.floatValue ?? -1, 1200.0, accuracy: 0.001)

        XCTAssertEqual(events[3].kind, .textChange)
        let txt = try XCTUnwrap(events[3].decodePayloadJSON())
        XCTAssertEqual(txt["text"] as? String, "Hello 🌍")

        XCTAssertEqual(events[4].kind, .layout)
        let layout = try XCTUnwrap(events[4].decodePayloadJSON())
        XCTAssertEqual((layout["x"] as? NSNumber)?.floatValue ?? -1, 4.0, accuracy: 0.001)
        XCTAssertEqual((layout["y"] as? NSNumber)?.floatValue ?? -1, 8.0, accuracy: 0.001)
        XCTAssertEqual((layout["width"] as? NSNumber)?.floatValue ?? -1, 320.0, accuracy: 0.001)
        XCTAssertEqual((layout["height"] as? NSNumber)?.floatValue ?? -1, 180.0, accuracy: 0.001)

        XCTAssertEqual(events[5].kind, .custom)
        XCTAssertEqual(events[5].name, "onBlob")
        XCTAssertEqual(events[5].payload, customBytes)
    }

    func testDecodeMalformedAndTruncatedBuffersDegradeSafely() {
        XCTAssertTrue(NveEventRingCodec.decode(Data()).isEmpty)
        XCTAssertTrue(NveEventRingCodec.decode(Data([1, 2, 3])).isEmpty)

        var zeroCount = Data()
        appendLE(UInt32(0), to: &zeroCount)
        XCTAssertTrue(NveEventRingCodec.decode(zeroCount).isEmpty)

        // Claims 2 events, only has 1 full empty event + 5 trailing bytes
        var truncated = Data()
        appendLE(UInt32(2), to: &truncated)
        appendEventHeader(
            surfaceId: 5, nodeId: 6, timestampUs: 77,
            kind: 250, // unknown kind -> falls back to .custom
            payloadTag: 0, // valid empty payload
            to: &truncated
        )
        truncated.append(contentsOf: [1, 2, 3, 4, 5])

        let decoded = NveEventRingCodec.decode(truncated)
        XCTAssertEqual(decoded.count, 1)
        XCTAssertEqual(decoded[0].surfaceId, 5)
        XCTAssertEqual(decoded[0].nodeId, 6)
        XCTAssertEqual(decoded[0].timestampUs, 77)
        XCTAssertEqual(decoded[0].kind, .custom)
        XCTAssertTrue(decoded[0].payload.isEmpty)

        // Unknown payload tag stops cleanly without appending a corrupt event.
        var unknownTag = Data()
        appendLE(UInt32(1), to: &unknownTag)
        appendEventHeader(
            surfaceId: 5, nodeId: 6, timestampUs: 77,
            kind: NveEventKind.custom.rawValue,
            payloadTag: 99,
            to: &unknownTag
        )
        XCTAssertTrue(NveEventRingCodec.decode(unknownTag).isEmpty)

        // Custom event whose claimed payload length exceeds remaining bytes must NOT emit a partial event.
        var oversized = Data()
        appendLE(UInt32(1), to: &oversized)
        appendEventHeader(
            surfaceId: 1, nodeId: 1, timestampUs: 1,
            kind: NveEventKind.custom.rawValue, payloadTag: 5, to: &oversized
        )
        appendLE(UInt16(2), to: &oversized)
        oversized.append(Data("ok".utf8))
        appendLE(UInt32(9999), to: &oversized)
        oversized.append(Data([1, 2]))

        XCTAssertTrue(NveEventRingCodec.decode(oversized).isEmpty)

        // Mid-payload truncation on Point (4 of 8 bytes) must NOT emit a partial event.
        var partialPoint = Data()
        appendLE(UInt32(1), to: &partialPoint)
        appendEventHeader(
            surfaceId: 1, nodeId: 2, timestampUs: 3,
            kind: NveEventKind.press.rawValue, payloadTag: 1, to: &partialPoint
        )
        appendF32(12.5, to: &partialPoint)
        XCTAssertTrue(NveEventRingCodec.decode(partialPoint).isEmpty)
    }
}
