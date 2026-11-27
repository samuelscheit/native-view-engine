import CNveAbi
import CNveHostTestABI
import NveHost
import NveHostAbi
import XCTest

/// Re-encodes a C `NveValue` graph the way the Rust side reads it, checking each slice's C contract
/// (an empty slice has a null pointer, a non-empty one never does).
private func reencode(_ v: NveValue, into w: inout BinWriter) throws {
    func run(_ ptr: UnsafePointer<UInt8>?, _ len: Int) throws -> UnsafeBufferPointer<UInt8> {
        XCTAssertEqual(ptr == nil, len == 0, "slice pointer / length disagree")
        return UnsafeBufferPointer(start: ptr, count: len)
    }
    switch v.tag {
    case UInt32(NVE_VAL_NULL.rawValue): w.writeNull()
    case UInt32(NVE_VAL_BOOL.rawValue):
        XCTAssertLessThanOrEqual(v.data.bool_val, 1)
        w.writeBool(v.data.bool_val == 1)
    case UInt32(NVE_VAL_I64.rawValue): w.writeI64(v.data.i64_val)
    case UInt32(NVE_VAL_F64.rawValue): w.writeF64(v.data.f64_val)
    case UInt32(NVE_VAL_STR.rawValue):
        try w.writeStr(String(decoding: try run(v.data.str_val.ptr, v.data.str_val.len), as: UTF8.self))
    case UInt32(NVE_VAL_BYTES.rawValue): try w.writeBytes(try run(v.data.bytes_val.ptr, v.data.bytes_val.len))
    case UInt32(NVE_VAL_ARRAY.rawValue):
        let a = v.data.array_val
        XCTAssertEqual(a.ptr == nil, a.len == 0)
        let mark = try w.beginArray(count: a.len)
        for i in 0..<a.len { try reencode(a.ptr[i], into: &w) }
        try w.end(mark)
    case UInt32(NVE_VAL_MAP.rawValue):
        let m = v.data.map_val
        XCTAssertEqual(m.ptr == nil, m.len == 0)
        let mark = w.beginMap(UInt32(m.len))
        for i in 0..<m.len {
            try w.writeMapKey(String(decoding: try run(m.ptr[i].key.ptr, m.ptr[i].key.len), as: UTF8.self))
            try reencode(m.ptr[i].value, into: &w)
        }
        try w.end(mark)
    default: XCTFail("unknown NveValue tag \(v.tag)")
    }
}

private func reencoded(_ v: NveValue) throws -> [UInt8] {
    var w = BinWriter()
    try reencode(v, into: &w)
    return w.bytes
}

private func dynamic(_ build: (inout BinWriter) throws -> Void) throws -> DynamicValue {
    var w = BinWriter()
    try build(&w)
    return try DynamicValue(bytes: w.bytes)
}

private func allocations(_ body: () throws -> Void) rethrows -> Int64? {
    guard nve_test_alloc_count_begin() != 0 else { return nil }
    do {
        try body()
    } catch {
        _ = nve_test_alloc_count_end()
        throw error
    }
    return nve_test_alloc_count_end()
}

final class NveValueTreeTests: XCTestCase {
    private let golden = try? DynamicValue(bytes: Golden.fromHex(Golden.binValueHex))

    func testDynamicValueKeepsItsTrueTagsAndRoundTrips() throws {
        let golden = try XCTUnwrap(golden)
        let tree = NveValueTree()
        let root = try tree.value(golden)
        XCTAssertEqual(root.tag, UInt32(NVE_VAL_MAP.rawValue), "a dynamic map must never cross as Bytes")
        let items = root.data.map_val.ptr[0].value.data.array_val
        let tags = (0..<items.len).map { items.ptr[$0].tag }
        let expected = [NVE_VAL_I64, NVE_VAL_STR, NVE_VAL_NULL, NVE_VAL_BOOL, NVE_VAL_F64, NVE_VAL_BYTES].map { UInt32($0.rawValue) }
        XCTAssertEqual(tags, expected)
        XCTAssertEqual(try reencoded(root), golden.bytes)
    }

    func testEdgeValues() throws {
        let cases: [(String, DynamicValue, NveValueTag)] = try [
            ("null", DynamicValue(), NVE_VAL_NULL),
            ("false", dynamic { $0.writeBool(false) }, NVE_VAL_BOOL),
            ("i64.min", dynamic { $0.writeI64(.min) }, NVE_VAL_I64),
            ("-1", dynamic { $0.writeI64(-1) }, NVE_VAL_I64),
            ("-0.0", dynamic { $0.writeF64(-0.0) }, NVE_VAL_F64),
            ("nan", dynamic { $0.writeF64(.nan) }, NVE_VAL_F64),
            ("empty str", dynamic { try $0.writeStr("") }, NVE_VAL_STR),
            ("emoji str", dynamic { try $0.writeStr("hé — 🚀👩‍👩‍👧") }, NVE_VAL_STR),
            ("empty bytes", dynamic { try $0.writeBytes([UInt8]()) }, NVE_VAL_BYTES),
            ("empty array", dynamic { try $0.end($0.beginArray(0)) }, NVE_VAL_ARRAY),
            ("empty map", dynamic { try $0.end($0.beginMap(0)) }, NVE_VAL_MAP),
            ("nested", dynamic { w in
                let map = w.beginMap(3)
                try w.writeMapKey("")
                try w.end(w.beginMap(0))
                try w.writeMapKey("🔑")
                let outer = w.beginArray(2)
                let inner = w.beginArray(2)
                w.writeI64(.max)
                try w.writeBytes([0, 0xFF])
                try w.end(inner)
                try w.end(w.beginArray(0))
                try w.end(outer)
                try w.writeMapKey("s")
                try w.writeStr("")
                try w.end(map)
            }, NVE_VAL_MAP),
        ]
        for (name, value, tag) in cases {
            let tree = NveValueTree()
            let root = try tree.value(value)
            XCTAssertEqual(root.tag, UInt32(tag.rawValue), name)
            XCTAssertEqual(try reencoded(root), value.bytes, name)
        }
    }

    func testMaxDepthNestingAndManyChunks() throws {
        let deep = try dynamic { w in
            var marks: [BinWriter.Mark] = []
            for depth in 0..<binValueMaxDepth {
                if depth.isMultiple(of: 2) {
                    marks.append(w.beginArray(1))
                } else {
                    marks.append(w.beginMap(1))
                    try w.writeMapKey("d\(depth)")
                }
            }
            try w.writeStr("bottom")
            for mark in marks.reversed() { try w.end(mark) }
        }
        // Thousands of nodes force many chunks; every earlier pointer must stay valid.
        let wide = try dynamic { w in
            let a = w.beginArray(5000)
            for i in 0..<5000 {
                let m = w.beginMap(2)
                try w.writeMapKey("k\(i)")
                try w.writeStr(String(repeating: "é", count: i % 37))
                try w.writeMapKey("b")
                try w.writeBytes([UInt8](repeating: UInt8(truncatingIfNeeded: i), count: i % 5))
                try w.end(m)
            }
            try w.end(a)
        }
        let tree = NveValueTree()
        let roots = try [tree.value(deep), tree.value(wide), tree.value(deep)]
        XCTAssertEqual(try reencoded(roots[0]), deep.bytes)
        XCTAssertEqual(try reencoded(roots[1]), wide.bytes)
        XCTAssertEqual(try reencoded(roots[2]), deep.bytes)
    }

    func testScalarsCopiesAndEntries() throws {
        let tree = NveValueTree()
        XCTAssertEqual(try reencoded(tree.null()), [0])
        XCTAssertEqual(tree.bool(true).data.bool_val, 1)
        XCTAssertEqual(tree.int(.min).data.i64_val, .min)
        XCTAssertEqual(tree.float(-2.5).data.f64_val, -2.5)

        var source = "mutated later é 🚀"
        let str = tree.string(source)
        source += "!"
        XCTAssertEqual(try reencoded(str), try dynamic { try $0.writeStr("mutated later é 🚀") }.bytes)
        XCTAssertEqual(tree.strSlice("").len, 0)
        XCTAssertNil(tree.strSlice("").ptr)

        let reversed = [UInt8]([1, 2, 3]).reversed()  // non-contiguous
        XCTAssertEqual(try reencoded(tree.bytes(reversed)), try dynamic { try $0.writeBytes([3, 2, 1]) }.bytes)
        XCTAssertNil(tree.bufferSlice([UInt8]()).ptr)

        let encoded = try tree.encoded { w in
            let a = try w.beginArray(count: 2)
            w.writeI64(-7)
            try w.writeStr("x")
            try w.end(a)
        }
        XCTAssertEqual(encoded.tag, UInt32(NVE_VAL_BYTES.rawValue), "Struct / Vec props cross as Bytes")
        let payload = Array(UnsafeBufferPointer<UInt8>(start: encoded.data.bytes_val.ptr, count: encoded.data.bytes_val.len))
        XCTAssertNoThrow(try BinReader.validate(payload))

        for key: StaticString in ["source_url", "w", "é", "🚀", ""] {
            let e = tree.entry(key, tree.int(1))
            let text = String(decoding: UnsafeBufferPointer(start: e.key.ptr, count: e.key.len), as: UTF8.self)
            XCTAssertEqual(text, key.description)
            XCTAssertEqual(e.key.ptr == nil, e.key.len == 0)
        }
    }

    func testHybridRuntimeHelpers() throws {
        enum Quality: UInt32, HybridEnum { case auto = 0, uhd = 11 }
        XCTAssertEqual(try Quality.decode(11), .uhd)
        XCTAssertThrowsError(try Quality.decode(1)) {
            XCTAssertEqual($0 as? HybridError, .invalidEnum("Quality", 1))
        }
        XCTAssertNoThrow(try HybridStatus.check(HybridStatus.ok.rawValue))
        XCTAssertThrowsError(try HybridStatus.check(HybridStatus.invalidArg.rawValue)) {
            XCTAssertEqual($0 as? HybridError, .status(.invalidArg))
        }
        XCTAssertThrowsError(try HybridStatus.check(99)) { XCTAssertEqual($0 as? HybridError, .status(.err)) }
        XCTAssertNoThrow(try HybridError.checkAbiVersion(expected: 1, actual: 1))
        XCTAssertThrowsError(try HybridError.checkAbiVersion(expected: 1, actual: 2)) {
            XCTAssertEqual($0 as? HybridError, .abiVersionMismatch(expected: 1, actual: 2))
        }
        var w = BinWriter()
        XCTAssertThrowsError(try w.beginArray(count: Int(UInt32.max) + 1)) { XCTAssertEqual($0 as? BinError, .tooLarge) }
        XCTAssertTrue(w.bytes.isEmpty, "a rejected array writes nothing")
    }

    func testTreeAllocatesPerChunkNotPerNode() throws {
        #if DEBUG
        try XCTSkipIf(true, "allocation counts are only meaningful in optimized builds (swift test -c release)")
        #endif
        let golden = try XCTUnwrap(golden)
        _ = try NveValueTree().value(golden)  // warm up
        let count = try XCTUnwrap(try allocations {
            let tree = NveValueTree()
            for _ in 0..<20 { _ = try tree.value(golden) }  // ~20 * 9 nodes + 20 copies, < 1 KiB each
            withExtendedLifetime(tree) {}
        })
        XCTAssertLessThanOrEqual(count, 4, "the tree object plus its doubling chunks, never one per node")
    }
}
