import CNveAbi
import Foundation
import NveSwiftUI
import XCTest

final class NveSceneAndBinValueTests: XCTestCase {
    func testBinValueRoundTripsAllWireTagsZeroCopy() throws {
        let payload = NveBinValue.map([
            ("null", .null),
            ("flag", .bool(true)),
            ("neg", .int64(-9_007_199_254_740_991)),
            ("pi", .float64(3.141592653589793)),
            ("greeting", .string("Hello NVE 🚀")),
            ("blob", .bytes(Data([0xDE, 0xAD, 0xBE, 0xEF]))),
            (
                "nested",
                .array([
                    .int64(10),
                    .map([("inner", .string("ok"))]),
                    .bool(false),
                ])
            ),
        ])

        let encoded = try payload.encode()
        try NveBinValueView.withParsed(encoded) { root in
            XCTAssertEqual(root.tag, .map)
            XCTAssertEqual(root.mapCount, 7)
            XCTAssertEqual(root.mapValue(forKey: "null")?.isNull, true)
            XCTAssertEqual(root.mapValue(forKey: "flag")?.boolValue, true)
            XCTAssertEqual(root.mapValue(forKey: "neg")?.int64Value, -9_007_199_254_740_991)
            XCTAssertEqual(root.mapValue(forKey: "pi")?.doubleValue, 3.141592653589793)
            XCTAssertEqual(root.mapValue(forKey: "greeting")?.stringValue, "Hello NVE 🚀")
            XCTAssertEqual(
                root.mapValue(forKey: "blob")?.bytesValue.map { Data($0) },
                Data([0xDE, 0xAD, 0xBE, 0xEF])
            )
            XCTAssertNil(root.mapValue(forKey: "missing"))

            let nested = try XCTUnwrap(root.mapValue(forKey: "nested"))
            XCTAssertEqual(nested.arrayCount, 3)
            XCTAssertEqual(nested.arrayElement(at: 0)?.int64Value, 10)
            XCTAssertEqual(nested.arrayElement(at: 1)?.mapValue(forKey: "inner")?.stringValue, "ok")
            XCTAssertEqual(nested.arrayElement(at: 2)?.boolValue, false)
            XCTAssertNil(nested.arrayElement(at: 3))
        }
    }

    func testBinValueRejectsDepthOverflowTruncationBadTagsAndInvalidUtf8() throws {
        // Depth 64 succeeds (root array at depth 0 .. 64th array at depth 63 containing null).
        var depth64 = NveBinValue.null
        for _ in 0 ..< nveBinValueMaxDepth {
            depth64 = .array([depth64])
        }
        let encoded64 = try depth64.encode()
        try NveBinValueView.withParsed(encoded64) { view in
            XCTAssertEqual(view.tag, .array)
        }

        // Depth 65 fails with maxDepthExceeded on encode AND parse.
        let depth65 = NveBinValue.array([depth64])
        XCTAssertThrowsError(try depth65.encode()) { err in
            XCTAssertEqual(err as? NveBinValueError, .maxDepthExceeded)
        }

        // Hand-craft a 65-deep array wire buffer to verify parser depth guard directly.
        var deepWire = Data()
        for level in 0 ..< 65 {
            let remainingContainers = 64 - level
            let payloadLen = UInt32(4 + remainingContainers * 9 + 1)
            deepWire.append(NveBinValueTag.array.rawValue)
            var leLen = payloadLen.littleEndian
            withUnsafeBytes(of: &leLen) { deepWire.append(contentsOf: $0) }
            var leCount = UInt32(1).littleEndian
            withUnsafeBytes(of: &leCount) { deepWire.append(contentsOf: $0) }
        }
        deepWire.append(NveBinValueTag.null.rawValue)
        XCTAssertThrowsError(try NveBinValueView.withParsed(deepWire) { _ in }) { err in
            XCTAssertEqual(err as? NveBinValueError, .maxDepthExceeded)
        }

        // Every strict prefix of a valid encoded map must fail with `.truncated`.
        let validMap = try NveBinValue.map([("k", .string("value"))]).encode()
        for cut in 0 ..< validMap.count {
            let prefix = validMap.prefix(cut)
            XCTAssertThrowsError(try NveBinValueView.withParsed(prefix) { _ in }) { err in
                XCTAssertEqual(err as? NveBinValueError, .truncated)
            }
        }

        // Trailing bytes rejected.
        var withTrailing = validMap
        withTrailing.append(0x00)
        XCTAssertThrowsError(try NveBinValueView.withParsed(withTrailing) { _ in }) { err in
            XCTAssertEqual(err as? NveBinValueError, .trailingBytes(1))
        }

        // Invalid tag, invalid bool, invalid UTF-8 rejected.
        XCTAssertThrowsError(try NveBinValueView.withParsed(Data([9])) { _ in }) { err in
            XCTAssertEqual(err as? NveBinValueError, .invalidTag(9))
        }
        XCTAssertThrowsError(try NveBinValueView.withParsed(Data([1, 2])) { _ in }) { err in
            XCTAssertEqual(err as? NveBinValueError, .invalidBool(2))
        }
        let badUtf8 = Data([4, 2, 0, 0, 0, 0xFF, 0xFE])
        XCTAssertThrowsError(try NveBinValueView.withParsed(badUtf8) { _ in }) { err in
            XCTAssertEqual(err as? NveBinValueError, .invalidUtf8)
        }
    }
}
