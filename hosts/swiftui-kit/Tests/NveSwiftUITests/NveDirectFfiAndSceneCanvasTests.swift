import CNveAbi
import Foundation
import NveSwiftUI
import SwiftUI
import XCTest

#if canImport(AppKit)
import AppKit
#endif

private func readSliceString(_ slice: NveStrSlice) -> String {
    guard let ptr = slice.ptr, slice.len > 0 else { return "" }
    return String(bytes: UnsafeBufferPointer(start: ptr, count: slice.len), encoding: .utf8) ?? ""
}

private func readDirectValue(_ value: NveValue) -> NveBinValue {
    switch value.tag {
    case UInt32(NVE_VAL_NULL.rawValue):
        return .null
    case UInt32(NVE_VAL_BOOL.rawValue):
        return .bool(value.data.bool_val != 0)
    case UInt32(NVE_VAL_I64.rawValue):
        return .int64(value.data.i64_val)
    case UInt32(NVE_VAL_F64.rawValue):
        return .float64(value.data.f64_val)
    case UInt32(NVE_VAL_STR.rawValue):
        return .string(readSliceString(value.data.str_val))
    case UInt32(NVE_VAL_BYTES.rawValue):
        let b = value.data.bytes_val
        guard let ptr = b.ptr, b.len > 0 else { return .bytes(Data()) }
        return .bytes(Data(bytes: ptr, count: b.len))
    case UInt32(NVE_VAL_ARRAY.rawValue):
        let arr = value.data.array_val
        guard let ptr = arr.ptr, arr.len > 0 else { return .array([]) }
        return .array((0 ..< arr.len).map { readDirectValue(ptr[$0]) })
    case UInt32(NVE_VAL_MAP.rawValue):
        let m = value.data.map_val
        guard let ptr = m.ptr, m.len > 0 else { return .map([]) }
        return .map((0 ..< m.len).map { (readSliceString(ptr[$0].key), readDirectValue(ptr[$0].value)) })
    default:
        return .null
    }
}

private func packRgba(_ r: UInt8, _ g: UInt8, _ b: UInt8, _ a: UInt8 = 255) -> UInt32 {
    UInt32(r) | (UInt32(g) << 8) | (UInt32(b) << 16) | (UInt32(a) << 24)
}

private enum SampleQuality: UInt32, Equatable {
    case auto = 0
    case uhd4k = 11
}

private struct SampleNestedMeta: Equatable, NveDirectEncodable {
    var tag: String
    var version: Int32

    var directPropCount: Int { 2 }

    func encodeDirect(into builder: inout NveDirectPropsBuilder) throws {
        builder.write("tag", tag)
        builder.write("version", version)
    }
}

private struct SampleTypedProps: Equatable, NveDirectEncodable {
    var title: String
    var count: Int64
    var ratio: Double
    var muted: Bool
    var quality: SampleQuality
    var optionalNote: String?
    var payload: [UInt8]
    var meta: SampleNestedMeta

    var directPropCount: Int { 8 }

    func encodeDirect(into builder: inout NveDirectPropsBuilder) throws {
        builder.write("title", title)
        builder.write("count", count)
        builder.write("ratio", ratio)
        builder.write("muted", muted)
        builder.write("quality", enum: quality)
        builder.write("optional_note", optionalNote)
        builder.write("payload", payload)
        try builder.write("meta", nested: meta)
    }
}

private struct EmptyTypedProps: Equatable, NveDirectEncodable {
    var directPropCount: Int { 0 }
    func encodeDirect(into builder: inout NveDirectPropsBuilder) throws {}
}

final class NveDirectFfiAndSceneCanvasTests: XCTestCase {
    func testZeroJsonSerializationAnywhereInNveSwiftUISources() throws {
        let testsDir = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // NveSwiftUITests
            .deletingLastPathComponent() // Tests
            .deletingLastPathComponent() // swiftui-kit
        let sourcesDir = testsDir.appendingPathComponent("Sources/NveSwiftUI")

        let files = try FileManager.default.contentsOfDirectory(
            at: sourcesDir,
            includingPropertiesForKeys: nil
        ).filter { $0.pathExtension == "swift" }

        XCTAssertFalse(files.isEmpty, "Expected Swift source files under Sources/NveSwiftUI")
        for file in files {
            let content = try String(contentsOf: file, encoding: .utf8)
            XCTAssertFalse(
                content.contains("JSONSerialization"),
                "\(file.lastPathComponent) must not use JSONSerialization"
            )
            XCTAssertFalse(
                content.contains("JSONDecoder"),
                "\(file.lastPathComponent) must not use JSONDecoder"
            )
            XCTAssertFalse(
                content.contains("JSONEncoder"),
                "\(file.lastPathComponent) must not use JSONEncoder"
            )
        }
    }

    func testDirectPropsWriterRoundtripsAllValueTypesIncludingNestedAndLargeOverflow() throws {
        let rawBlob = Data([0xDE, 0xAD, 0xBE, 0xEF, 0x01])
        let bigText = String(repeating: "SwiftUI-ZeroJSON-🚀-", count: 350) // > 4 KiB forces heap chunk

        let props: [String: Any] = [
            "nullVal": NSNull(),
            "boolTrue": true,
            "boolFalse": false,
            "nsBool": NSNumber(value: true),
            "nsInt": NSNumber(value: 1),
            "intVal": -9_007_199_254_740_991 as Int64,
            "doubleVal": 3.141592653589793,
            "stringVal": "Hello 🚀 NVE",
            "bytesVal": rawBlob,
            "bigVal": bigText,
            "nestedArr": [
                10,
                true,
                "item",
                ["innerKey": "innerVal", "innerNums": [1.5, 2.5]],
            ] as [Any],
            "nestedMap": [
                "flag": false,
                "payload": Data([9, 8, 7]),
            ] as [String: Any],
        ]

        let captured: NveBinValue = try NveDirectPropsWriter.withProps(props) { ptr, count in
            XCTAssertNotNil(ptr)
            XCTAssertEqual(count, 12)
            let entries = (0 ..< Int(count)).map { idx in
                (readSliceString(ptr![idx].key), readDirectValue(ptr![idx].value))
            }
            return .map(entries)
        }

        let expected = NveBinValue.map([
            ("bigVal", .string(bigText)),
            ("boolFalse", .bool(false)),
            ("boolTrue", .bool(true)),
            ("bytesVal", .bytes(rawBlob)),
            ("doubleVal", .float64(3.141592653589793)),
            ("intVal", .int64(-9_007_199_254_740_991)),
            (
                "nestedArr",
                .array([
                    .int64(10),
                    .bool(true),
                    .string("item"),
                    .map([
                        ("innerKey", .string("innerVal")),
                        ("innerNums", .array([.float64(1.5), .float64(2.5)])),
                    ]),
                ])
            ),
            (
                "nestedMap",
                .map([
                    ("flag", .bool(false)),
                    ("payload", .bytes(Data([9, 8, 7]))),
                ])
            ),
            ("nsBool", .bool(true)),
            ("nsInt", .int64(1)),
            ("nullVal", .null),
            ("stringVal", .string("Hello 🚀 NVE")),
        ])
        XCTAssertEqual(captured, expected)

        // Empty map passes nil pointer and count 0.
        try NveDirectPropsWriter.withProps([:]) { ptr, count in
            XCTAssertNil(ptr)
            XCTAssertEqual(count, 0)
        }

        // Depth > 64 is rejected cleanly.
        var deep = NveBinValue.null
        for _ in 0 ... nveBinValueMaxDepth {
            deep = .array([deep])
        }
        XCTAssertThrowsError(try NveDirectPropsWriter.withEntries([("deep", deep)]) { _, _ in }) { err in
            XCTAssertEqual(err as? NveDirectFfiError, .maxDepthExceeded)
        }
    }

    func testDirectEventWriterRoundtripsAllEventKindsAndPayloads() throws {
        let customBin = try NveBinValue.map([
            ("action", .string("submit")),
            ("items", .array([.int64(1), .int64(2)])),
        ]).encode()

        let batch: [NveEvent] = [
            NveEvent(surfaceId: 1, nodeId: 10, timestampUs: 100, kind: .focus, payload: .empty),
            NveEvent(
                surfaceId: 1,
                nodeId: 11,
                timestampUs: 200,
                kind: .press,
                payload: .point(x: 12.5, y: 34.25)
            ),
            NveEvent(
                surfaceId: 1,
                nodeId: 12,
                timestampUs: 300,
                kind: .scroll,
                payload: .scroll(x: 0, y: 145.5, contentWidth: 320, contentHeight: 900)
            ),
            NveEvent(
                surfaceId: 1,
                nodeId: 13,
                timestampUs: 400,
                kind: .textChange,
                payload: .text("Typing ✍️")
            ),
            NveEvent(
                surfaceId: 1,
                nodeId: 14,
                timestampUs: 500,
                kind: .layout,
                payload: .layout(x: 4, y: 8, width: 200, height: 100)
            ),
            NveEvent(
                surfaceId: 1,
                nodeId: 15,
                timestampUs: 600,
                kind: .custom,
                name: "onFormSubmit",
                payload: .binValue(customBin)
            ),
            NveEvent(
                surfaceId: 1,
                nodeId: 16,
                timestampUs: 700,
                kind: .custom,
                name: "onRawBytes",
                payload: .binValue(Data([0xFF, 0x01]))
            ),
        ]

        try NveDirectEventWriter.withBatch(batch) { cBatch in
            XCTAssertEqual(cBatch.len, 7)
            let ptr = try XCTUnwrap(cBatch.ptr)

            XCTAssertEqual(ptr[0].kind, NveEventKind.focus.rawValue)
            XCTAssertEqual(ptr[0].payload_tag, UInt8(NVE_EVENT_PAYLOAD_EMPTY.rawValue))

            XCTAssertEqual(ptr[1].kind, NveEventKind.press.rawValue)
            XCTAssertEqual(ptr[1].payload_tag, UInt8(NVE_EVENT_PAYLOAD_POINT.rawValue))
            XCTAssertEqual(ptr[1].x, 12.5)
            XCTAssertEqual(ptr[1].y, 34.25)

            XCTAssertEqual(ptr[2].kind, NveEventKind.scroll.rawValue)
            XCTAssertEqual(ptr[2].payload_tag, UInt8(NVE_EVENT_PAYLOAD_SCROLL.rawValue))
            XCTAssertEqual(ptr[2].y, 145.5)
            XCTAssertEqual(ptr[2].width, 320)
            XCTAssertEqual(ptr[2].height, 900)

            XCTAssertEqual(ptr[3].kind, NveEventKind.textChange.rawValue)
            XCTAssertEqual(ptr[3].payload_tag, UInt8(NVE_EVENT_PAYLOAD_TEXT.rawValue))
            XCTAssertEqual(readSliceString(ptr[3].text), "Typing ✍️")

            XCTAssertEqual(ptr[4].kind, NveEventKind.layout.rawValue)
            XCTAssertEqual(ptr[4].payload_tag, UInt8(NVE_EVENT_PAYLOAD_LAYOUT.rawValue))
            XCTAssertEqual(ptr[4].x, 4)
            XCTAssertEqual(ptr[4].y, 8)
            XCTAssertEqual(ptr[4].width, 200)
            XCTAssertEqual(ptr[4].height, 100)

            XCTAssertEqual(ptr[5].kind, NveEventKind.custom.rawValue)
            XCTAssertEqual(ptr[5].payload_tag, UInt8(NVE_EVENT_PAYLOAD_CUSTOM.rawValue))
            XCTAssertEqual(readSliceString(ptr[5].name), "onFormSubmit")
            XCTAssertEqual(
                readDirectValue(ptr[5].value),
                .map([
                    ("action", .string("submit")),
                    ("items", .array([.int64(1), .int64(2)])),
                ])
            )

            // Non-NveBinValue raw bytes fall back to NVE_VAL_BYTES cleanly.
            XCTAssertEqual(readSliceString(ptr[6].name), "onRawBytes")
            XCTAssertEqual(readDirectValue(ptr[6].value), .bytes(Data([0xFF, 0x01])))
        }
    }

}
