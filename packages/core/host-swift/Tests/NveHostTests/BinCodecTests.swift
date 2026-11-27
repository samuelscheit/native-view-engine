import NveHost
import XCTest

final class BinCodecTests: XCTestCase {
    private let golden = Golden.fromHex(Golden.binValueHex)

    private func encodeGolden() throws -> [UInt8] {
        var w = BinWriter()
        let map = w.beginMap(1)
        try w.writeMapKey("k")
        let arr = w.beginArray(6)
        w.writeI64(1)
        try w.writeStr("é")
        w.writeNull()
        w.writeBool(true)
        w.writeF64(1.5)
        try w.writeBytes([0xFF])
        try w.end(arr)
        try w.end(map)
        return w.bytes
    }

    func testGoldenRoundTrip() throws {
        XCTAssertEqual(try encodeGolden(), golden)
        try BinReader.validate(golden)
        var r = BinReader(golden)
        XCTAssertEqual(try r.beginMap(), 1)
        XCTAssertEqual(try r.readMapKey(), "k")
        XCTAssertEqual(try r.beginArray(), 6)
        XCTAssertEqual(try r.readI64(), 1)
        XCTAssertEqual(try r.readStr(), "é")
        XCTAssertTrue(try r.readNull())
        XCTAssertFalse(try r.readNull())
        XCTAssertTrue(try r.readBool())
        XCTAssertEqual(try r.readF64(), 1.5)
        XCTAssertEqual(Array(try r.readBytes()), [0xFF])
        XCTAssertTrue(r.isAtEnd)
    }

    func testPrimitivesAndTruncation() throws {
        var w = BinWriter()
        w.u8(0xAA)
        w.u16(0xBEEF)
        w.u32(0xDEAD_BEEF)
        w.u64(0x0123_4567_89AB_CDEF)
        w.i32(-2)
        w.i64(.min)
        w.f32(-0.0)
        w.f64(.infinity)
        var r = BinReader(w.bytes)
        XCTAssertEqual(try r.u8(), 0xAA)
        XCTAssertEqual(try r.u16(), 0xBEEF)
        XCTAssertEqual(try r.u32(), 0xDEAD_BEEF)
        XCTAssertEqual(try r.u64(), 0x0123_4567_89AB_CDEF)
        XCTAssertEqual(try r.i32(), -2)
        XCTAssertEqual(try r.i64(), .min)
        let nz = try r.f32()
        XCTAssertTrue(nz == 0 && nz.sign == .minus)
        XCTAssertEqual(try r.f64(), .infinity)
        XCTAssertThrowsError(try r.u8()) { XCTAssertEqual($0 as? BinError, .truncated) }
        XCTAssertThrowsError(try r.take(-1)) { XCTAssertEqual($0 as? BinError, .truncated) }
    }

    func testReadF64AcceptsI64() throws {
        var w = BinWriter()
        w.writeI64(-7)
        w.writeF64(.nan)
        var r = BinReader(w.bytes)
        XCTAssertEqual(try r.readF64(), -7)
        XCTAssertTrue(try r.readF64().isNaN)
    }

    private func assertInvalid(_ bytes: [UInt8], _ expected: BinError, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertThrowsError(try BinReader.validate(bytes), file: file, line: line) {
            XCTAssertEqual($0 as? BinError, expected, file: file, line: line)
        }
    }

    func testRejections() {
        assertInvalid([1, 2], .invalidBool(2))
        assertInvalid([8], .invalidTag(8))
        assertInvalid([4, 2, 0, 0, 0, 0xC0, 0xAF], .invalidUtf8)
        assertInvalid([4, 0xFF, 0xFF, 0xFF, 0xFF, 0x61], .truncated)
        assertInvalid([6, 4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF], .truncated)  // count bomb
        assertInvalid([7, 8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0], .truncated)  // map entry < 5 bytes
        assertInvalid([6, 6, 0, 0, 0, 1, 0, 0, 0, 0, 0], .trailingBytes(1))
        assertInvalid([0, 0], .trailingBytes(1))
        assertInvalid([6, 5, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0], .truncated)  // overrun
        var r = BinReader([0])
        XCTAssertThrowsError(try r.readI64()) { XCTAssertEqual($0 as? BinError, .unexpectedTag(expected: .i64, actual: .null)) }
    }

    private func nested(_ depth: Int) throws -> [UInt8] {
        var w = BinWriter()
        var marks: [BinWriter.Mark] = []
        for _ in 0..<depth { marks.append(w.beginArray(1)) }
        w.writeNull()
        for m in marks.reversed() { try w.end(m) }
        return w.bytes
    }

    func testDepthLimit() throws {
        try BinReader.validate(try nested(binValueMaxDepth))
        assertInvalid(try nested(binValueMaxDepth + 1), .maxDepth)
    }

    func testEveryTruncatedPrefixFails() {
        for n in 0..<golden.count {
            let prefix = Array(golden[0..<n])
            XCTAssertThrowsError(try BinReader.validate(prefix), "prefix \(n)")
            var r = BinReader(prefix)
            XCTAssertThrowsError(try r.skipValue(), "skip prefix \(n)")
        }
    }

    func testRawValueAndFuzz() throws {
        var r = BinReader(golden + [0])
        XCTAssertEqual(try r.readRawValue().count, golden.count)
        XCTAssertTrue(try r.readNull())
        var s: UInt64 = 0x9E37_79B9_7F4A_7C15
        func next() -> UInt64 {
            s ^= s << 13
            s ^= s >> 7
            s ^= s << 17
            return s
        }
        for iter in 0..<5000 {
            var buf = golden
            if iter % 2 == 0 {
                buf[Int(next() % UInt64(buf.count))] = UInt8(truncatingIfNeeded: next())
            } else {
                buf = (0..<Int(next() % 48)).map { _ in UInt8(next() % 9) }
            }
            let valid = (try? BinReader.validate(buf)) != nil
            var fr = BinReader(buf)
            if valid {
                _ = try fr.readRawValue()
                XCTAssertTrue(fr.isAtEnd)
            } else {
                _ = try? fr.readRawValue()
            }
        }
    }

    func testDynamicValueAndStatuses() throws {
        XCTAssertTrue(DynamicValue().isNull)
        XCTAssertEqual(try DynamicValue(bytes: golden).tag, .map)
        XCTAssertThrowsError(try DynamicValue(bytes: Array(golden[0..<10])))
        XCTAssertThrowsError(try DynamicValue(bytes: []))
        XCTAssertNoThrow(try HybridError.checkSchema(expected: Key64(1), actual: Key64(1)))
        XCTAssertThrowsError(try HybridError.checkSchema(expected: Key64(1), actual: Key64(2))) {
            XCTAssertEqual($0 as? HybridError, .schemaMismatch(expected: Key64(1), actual: Key64(2)))
        }
        XCTAssertEqual(HostStatus.internalError.rawValue, -8)
        XCTAssertEqual(HybridStatus.nullHandle.rawValue, 4)
    }

    func testUtf8Validation() {
        XCTAssertTrue(Utf8.isValid(Array("plain é — 🚀 \u{10FFFF}".utf8)))
        for bad: [UInt8] in [[0xC0, 0xAF], [0xE0, 0x80, 0xAF], [0xF0, 0x80, 0x80, 0xAF], [0xED, 0xA0, 0x80],
                             [0xF4, 0x90, 0x80, 0x80], [0xE2, 0x80], [0xE2, 0x28, 0xA1], [0x80], [0xF5, 0x80, 0x80, 0x80]] {
            XCTAssertFalse(Utf8.isValid(bad), "\(bad)")
        }
        let slice = Array("xé".utf8)[1...]
        XCTAssertTrue(Utf8.isValid(slice))  // non-zero startIndex
    }
}
