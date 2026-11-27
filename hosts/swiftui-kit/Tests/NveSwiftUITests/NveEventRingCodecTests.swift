import Foundation
import NveSwiftUI
import XCTest

final class NveEventRingCodecTests: XCTestCase {
    private func appendLE<T: FixedWidthInteger>(_ value: T, to data: inout Data) {
        withUnsafeBytes(of: value.littleEndian) { raw in
            data.append(contentsOf: raw)
        }
    }

    private func appendF32(_ value: Float, to data: inout Data) {
        appendLE(value.bitPattern, to: &data)
    }

    private func makeNvevHeader(eventCount: UInt32, bytesLen: UInt32, version: UInt16 = 2) -> Data {
        var out = Data([0x4E, 0x56, 0x45, 0x56])
        appendLE(version, to: &out)
        appendLE(eventCount, to: &out)
        appendLE(bytesLen, to: &out)
        return out
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

    func testTypedEventPayloadBinValueZeroCopyHelper() throws {
        let binData = try NveBinValue.map([
            ("count", .int64(7)),
            ("label", .string("clicks")),
        ]).encode()

        let event = NveEvent(
            surfaceId: 3,
            nodeId: 9,
            timestampUs: 99_000,
            kind: .custom,
            name: "onIncrement",
            payload: .binValue(binData)
        )

        let decodedCount: Int64? = try event.withBinValueView { view in
            view.mapValue(forKey: "count")?.int64Value
        } ?? nil
        let decodedLabel: String? = try event.withBinValueView { view in
            view.mapValue(forKey: "label")?.stringValue
        } ?? nil
        XCTAssertEqual(decodedCount, 7)
        XCTAssertEqual(decodedLabel, "clicks")

        let emptyEvent = NveEvent(surfaceId: 1, nodeId: 1, timestampUs: 0, kind: .focus)
        XCTAssertEqual(emptyEvent.payload, .empty)
        let nilView: Int? = try emptyEvent.withBinValueView { _ in 1 }
        XCTAssertNil(nilView)

        let invalidBinEvent = NveEvent(
            surfaceId: 1,
            nodeId: 1,
            timestampUs: 0,
            kind: .custom,
            name: "raw",
            payload: .binValue(Data([0xFF, 0x00, 0xAB]))
        )
        XCTAssertThrowsError(try invalidBinEvent.withBinValueView { _ in })
    }

    func testEncodeAndDecodeCustomEventRoundtrip() throws {
        let binPayload = NveBinValue.map([
            ("delta", .int64(2)),
            ("source", .string("button")),
        ])
        let encoded = try NveEventRingCodec.encodeCustomEvent(
            surfaceId: 11,
            nodeId: 22,
            name: "onCounterChange_π",
            value: binPayload,
            timestampUs: 1_234_567_890
        )

        let decoded = try NveEventRingCodec.decodeStrict(encoded)
        XCTAssertEqual(decoded.count, 1)
        let ev = try XCTUnwrap(decoded.first)
        XCTAssertEqual(ev.surfaceId, 11)
        XCTAssertEqual(ev.nodeId, 22)
        XCTAssertEqual(ev.timestampUs, 1_234_567_890)
        XCTAssertEqual(ev.kind, .custom)
        XCTAssertEqual(ev.name, "onCounterChange_π")

        try ev.withBinValueView { view in
            XCTAssertEqual(view.mapValue(forKey: "delta")?.int64Value, 2)
            XCTAssertEqual(view.mapValue(forKey: "source")?.stringValue, "button")
        }
    }

    func testEncodeAndDecodeAllPayloadVariantsInSingleBatch() throws {
        let binData = try NveBinValue.map([("ok", .bool(true))]).encode()
        let input: [NveEvent] = [
            NveEvent(surfaceId: 1, nodeId: 10, timestampUs: 100, kind: .focus, payload: .empty),
            NveEvent(
                surfaceId: 1,
                nodeId: 11,
                timestampUs: 200,
                kind: .press,
                payload: .point(x: 42.5, y: 18.25)
            ),
            NveEvent(
                surfaceId: 2,
                nodeId: 12,
                timestampUs: 300,
                kind: .scroll,
                payload: .scroll(x: 10.0, y: 250.5, contentWidth: 390.0, contentHeight: 1200.0)
            ),
            NveEvent(
                surfaceId: 2,
                nodeId: 13,
                timestampUs: 400,
                kind: .textChange,
                payload: .text("Hello 🌍")
            ),
            NveEvent(
                surfaceId: 3,
                nodeId: 14,
                timestampUs: 500,
                kind: .layout,
                payload: .layout(x: 4.0, y: 8.0, width: 320.0, height: 180.0)
            ),
            NveEvent(
                surfaceId: 3,
                nodeId: 15,
                timestampUs: 600,
                kind: .custom,
                name: "onBlob",
                payload: .binValue(binData)
            ),
            NveEvent(
                surfaceId: 4,
                nodeId: 16,
                timestampUs: 700,
                kind: .longPress,
                payload: .point(x: 1.0, y: 2.0)
            ),
            NveEvent(surfaceId: 4, nodeId: 17, timestampUs: 800, kind: .blur, payload: .empty),
        ]

        let ring = NveEventRingCodec.encode(input)
        let strictDecoded = try NveEventRingCodec.decodeStrict(ring)
        XCTAssertEqual(strictDecoded, input)

        let bestEffort = NveEventRingCodec.decode(ring)
        XCTAssertEqual(bestEffort, input)

        // Verify default parameter convenience cases construct cleanly.
        let scrollShort = NveEventPayload.scroll(x: 5, y: 10)
        XCTAssertEqual(scrollShort, .scroll(x: 5, y: 10, contentWidth: 0, contentHeight: 0))
        let layoutShort = NveEventPayload.layout(width: 100, height: 50)
        XCTAssertEqual(layoutShort, .layout(x: 0, y: 0, width: 100, height: 50))
    }

    func testDecodeRejectsCorruptAndTruncatedEventRings() throws {
        // Empty and short headers
        XCTAssertTrue(NveEventRingCodec.decode(Data()).isEmpty)
        XCTAssertTrue(NveEventRingCodec.decode(Data([1, 2, 3])).isEmpty)
        XCTAssertThrowsError(try NveEventRingCodec.decodeStrict(Data())) { err in
            XCTAssertEqual(err as? NveEventRingError, .truncated)
        }
        XCTAssertThrowsError(try NveEventRingCodec.decodeStrict(Data([1, 2, 3]))) { err in
            XCTAssertEqual(err as? NveEventRingError, .truncated)
        }

        // Zero count is valid
        let zeroCount = makeNvevHeader(eventCount: 0, bytesLen: 0)
        XCTAssertEqual(try NveEventRingCodec.decodeStrict(zeroCount), [])

        // Every strict prefix of a rich batch throws `.truncated` on decodeStrict and never crashes decode.
        let full = NveEventRingCodec.encode([
            NveEvent(surfaceId: 1, nodeId: 1, timestampUs: 1, kind: .press, payload: .point(x: 1, y: 2)),
            NveEvent(
                surfaceId: 1,
                nodeId: 2,
                timestampUs: 2,
                kind: .scroll,
                payload: .scroll(x: 3, y: 4, contentWidth: 100, contentHeight: 200)
            ),
            NveEvent(surfaceId: 1, nodeId: 3, timestampUs: 3, kind: .textChange, payload: .text("abc")),
            NveEvent(
                surfaceId: 1,
                nodeId: 4,
                timestampUs: 4,
                kind: .layout,
                payload: .layout(x: 1, y: 2, width: 30, height: 40)
            ),
            NveEvent(
                surfaceId: 1,
                nodeId: 5,
                timestampUs: 5,
                kind: .custom,
                name: "ev",
                payload: .binValue(Data([1, 2, 3]))
            ),
        ])
        for cut in 0 ..< full.count {
            let prefix = full.prefix(cut)
            XCTAssertThrowsError(try NveEventRingCodec.decodeStrict(prefix)) { err in
                XCTAssertEqual(err as? NveEventRingError, .truncated)
            }
            XCTAssertLessThan(NveEventRingCodec.decode(prefix).count, 5)
        }

        // Trailing bytes rejected by decodeStrict
        var withTrailing = full
        withTrailing.append(0xAA)
        XCTAssertThrowsError(try NveEventRingCodec.decodeStrict(withTrailing)) { err in
            XCTAssertEqual(err as? NveEventRingError, .trailingBytes(1))
        }

        // Bad magic and unsupported version rejected cleanly
        var badMagic = zeroCount
        badMagic[0] = 0x00
        XCTAssertThrowsError(try NveEventRingCodec.decodeStrict(badMagic)) { err in
            XCTAssertEqual(err as? NveEventRingError, .invalidMagic)
        }
        let badVer = makeNvevHeader(eventCount: 0, bytesLen: 0, version: 1)
        XCTAssertThrowsError(try NveEventRingCodec.decodeStrict(badVer)) { err in
            XCTAssertEqual(err as? NveEventRingError, .unsupportedVersion(1))
        }

        // Unknown table key rejected by decodeStrict and decode
        var badTable = NveEventRingCodec.encode([
            NveEvent(surfaceId: 5, nodeId: 6, timestampUs: 77, kind: .focus),
        ])
        let bogusTable: UInt64 = 0xDEAD_BEEF_CAFE_BABE
        withUnsafeBytes(of: bogusTable.littleEndian) { raw in
            badTable.replaceSubrange(14 + 8 ..< 14 + 16, with: raw)
        }
        XCTAssertThrowsError(try NveEventRingCodec.decodeStrict(badTable)) { err in
            XCTAssertEqual(err as? NveEventRingError, .invalidEventTable(bogusTable, 2))
        }
        XCTAssertTrue(NveEventRingCodec.decode(badTable).isEmpty)

        // Invalid UTF-8 in Text event rejected
        var badTextUtf8 = NveEventRingCodec.encode([
            NveEvent(surfaceId: 1, nodeId: 2, timestampUs: 3, kind: .textChange, payload: .text("ab")),
        ])
        badTextUtf8[badTextUtf8.count - 2] = 0xFF
        badTextUtf8[badTextUtf8.count - 1] = 0xFE
        XCTAssertThrowsError(try NveEventRingCodec.decodeStrict(badTextUtf8)) { err in
            XCTAssertEqual(err as? NveEventRingError, .invalidUtf8)
        }
        XCTAssertTrue(NveEventRingCodec.decode(badTextUtf8).isEmpty)

        // Out-of-bounds arena range inside record rejected cleanly
        var badBounds = NveEventRingCodec.encode([
            NveEvent(surfaceId: 1, nodeId: 1, timestampUs: 1, kind: .textChange, payload: .text("ok")),
        ])
        withUnsafeBytes(of: UInt32(9999).littleEndian) { raw in
            badBounds.replaceSubrange(14 + 60 ..< 14 + 64, with: raw)
        }
        XCTAssertThrowsError(try NveEventRingCodec.decodeStrict(badBounds)) { err in
            XCTAssertEqual(err as? NveEventRingError, .invalidBounds)
        }
        XCTAssertTrue(NveEventRingCodec.decode(badBounds).isEmpty)

        // Conflicting INLINE_KEY_UTF8 | KEY_SPLIT_BYTES flags rejected cleanly
        var badFlags = NveEventRingCodec.encode([
            NveEvent(surfaceId: 1, nodeId: 1, timestampUs: 1, kind: .keyDown, payload: .key(key: "a", code: "KeyA")),
        ])
        let conflictFlags: UInt32 = 0x20000 | 0x80000
        withUnsafeBytes(of: conflictFlags.littleEndian) { raw in
            badFlags.replaceSubrange(14 + 28 ..< 14 + 32, with: raw)
        }
        XCTAssertThrowsError(try NveEventRingCodec.decodeStrict(badFlags)) { err in
            XCTAssertEqual(err as? NveEventRingError, .conflictingFlags)
        }

        // Oversized header arena length rejected cleanly
        let oversized = makeNvevHeader(eventCount: 1, bytesLen: 9999) + Data(count: 64)
        XCTAssertThrowsError(try NveEventRingCodec.decodeStrict(oversized)) { err in
            XCTAssertEqual(err as? NveEventRingError, .truncated)
        }
        XCTAssertTrue(NveEventRingCodec.decode(oversized).isEmpty)
    }
}
