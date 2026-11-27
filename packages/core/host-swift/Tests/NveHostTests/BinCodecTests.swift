import NveHost
import XCTest

#if canImport(Darwin)
import Darwin

/// Live heap blocks across all malloc zones (`nil` where unsupported). This catches retained growth;
/// malloc exposes no public per-call counter, so churn is guarded by storage identity below.
private func liveHeapBlocks() -> Int? {
    var stats = malloc_statistics_t()
    malloc_zone_statistics(nil, &stats)
    return Int(stats.blocks_in_use)
}
#else
private func liveHeapBlocks() -> Int? { nil }
#endif

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

    func testContainerLimitIsEnforced() {
        // Rust validates children against `&bytes[..container_end]`: `Truncated`, not `invalidUtf8`.
        assertInvalid([6, 5, 0, 0, 0, 1, 0, 0, 0, 4, 2, 0, 0, 0, 0xC0, 0xAF], .truncated)
        assertInvalid([6, 10, 0, 0, 0, 1, 0, 0, 0, 6, 9, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0], .truncated)
    }

    func testMaxLengthsNeverTrap() {
        // u32::MAX lengths/counts are compared before becoming `Int` (would trap on arm64_32).
        assertInvalid([4, 0xFF, 0xFF, 0xFF, 0xFF], .truncated)
        assertInvalid([5, 0xFF, 0xFF, 0xFF, 0xFF], .truncated)
        assertInvalid([6, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0], .truncated)
        assertInvalid([7, 8, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0], .truncated)
        var r = BinReader([0xFF, 0xFF, 0xFF, 0xFF])
        XCTAssertThrowsError(try r.readMapKey()) { XCTAssertEqual($0 as? BinError, .truncated) }
        var s = BinReader([4, 0xFF, 0xFF, 0xFF, 0x7F])
        XCTAssertThrowsError(try s.skipValue()) { XCTAssertEqual($0 as? BinError, .truncated) }
    }

    func testBomRoundTrips() throws {
        for text in ["\u{FEFF}", "\u{FEFF}x", "\u{FEFF}\u{FEFF}x"] {
            var w = BinWriter()
            let m = w.beginMap(1)
            try w.writeMapKey(text)
            try w.writeStr(text)
            try w.end(m)
            var r = BinReader(w.bytes)
            XCTAssertEqual(try r.beginMap(), 1)
            XCTAssertEqual(try r.readMapKey(), text)
            XCTAssertEqual(try r.readStr(), text)
        }
        // Lone surrogates cannot occur: Swift `String` is always valid Unicode (`"\u{D800}"` does not
        // compile), so the writer needs no surrogate check; byte-level input is validated on read.
        XCTAssertThrowsError(try BinReader.validate([4, 3, 0, 0, 0, 0xED, 0xA0, 0x80])) {
            XCTAssertEqual($0 as? BinError, .invalidUtf8)
        }
    }

    func testStaleMarksThrowInsteadOfTrapping() throws {
        var w = BinWriter()
        let m = w.beginArray(1)
        w.writeNull()
        w.clear()
        XCTAssertThrowsError(try w.end(m)) { XCTAssertEqual($0 as? BinError, .invalidMark) }
        let open = w.beginArray(0)
        try w.end(open)
        w.clear()
        w.writeNull()
        XCTAssertThrowsError(try w.end(open)) { XCTAssertEqual($0 as? BinError, .invalidMark) }
    }

    func testValidationRetainsNothingAndWriterReusesItsStorage() throws {
        var big = BinWriter(capacity: 1 << 20)
        let m = big.beginArray(3)
        try big.writeBytes([UInt8](repeating: 0xAB, count: 256 * 1024))
        try big.writeStr(String(repeating: "x", count: 64 * 1024))
        try big.writeStr("hé — 🚀")
        try big.end(m)
        let payload = big.bytes
        try BinReader.validate(payload)
        var w = BinWriter(capacity: 256)
        for _ in 0..<2 {
            w.clear()
            try w.writeStr("warm-up")
        }
        let storage = w.bytes.withUnsafeBufferPointer { $0.baseAddress }
        let capacity = w.bytes.capacity
        let before = liveHeapBlocks()
        for i in 0..<2000 {
            try BinReader.validate(payload)
            w.clear()
            let a = w.beginArray(2)
            w.writeI64(Int64(i))
            try w.writeStr("steady state")
            try w.end(a)
        }
        // The writer never reallocates once warm: same buffer, same capacity after 2000 reuses.
        XCTAssertEqual(w.bytes.withUnsafeBufferPointer { $0.baseAddress }, storage)
        XCTAssertEqual(w.bytes.capacity, capacity)
        if let before, let after = liveHeapBlocks() {
            // A per-iteration retained allocation would add >= 2000 live blocks; allow runtime noise.
            XCTAssertLessThan(abs(after - before), 64, "heap blocks \(before) -> \(after)")
        }
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
