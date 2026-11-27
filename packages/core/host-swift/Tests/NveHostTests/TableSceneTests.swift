import CNveHostTestABI
import NveHost
import XCTest

// The C-imported structs satisfy the runtime protocols with empty extensions — exactly what the
// generated `NveTableView` / `NveSceneView` conformances look like.
extension NveTestTableView: TableViewABI {}
extension NveTestSceneView: SceneViewABI {}

final class TableSceneTests: XCTestCase {
    private let schema = Key64(0xF00D_FACE_C0FF_EE11)
    private let opacity = FieldSpec(word: 0, shift: 0, bits: 8, kind: .u32)
    private let z = FieldSpec(word: 0, shift: 8, bits: 12, kind: .i32)
    private let visible = FieldSpec(word: 0, shift: 20, bits: 1, kind: .bool)
    private let width = FieldSpec(word: 1, kind: .length)
    private let color = FieldSpec(word: 2, kind: .color)
    private let label = FieldSpec(word: 3, kind: .str)
    private let scale = FieldSpec(word: 5, kind: .f32)

    private func scene() throws -> SceneReader<NveTestSceneView> {
        try SceneReader.open(nve_test_scene(), expectedSchema: schema)
    }

    func testFieldSpecExtraction() {
        XCTAssertEqual(FieldSpec(word: 0, shift: 8, bits: 8, kind: .u32).extract(0xABCD_1234), 0x12)
        XCTAssertEqual(FieldSpec(word: 0, shift: 0, bits: 32, kind: .u32).extract(0xABCD_1234), 0xABCD_1234)
        XCTAssertEqual(FieldSpec(word: 0, shift: 0, bits: 3, kind: .i32).extractSigned(7), -1)
        XCTAssertEqual(FieldSpec(word: 0, shift: 0, bits: 3, kind: .i32).extractSigned(4), -4)
        XCTAssertEqual(FieldSpec(word: 0, shift: 0, bits: 32, kind: .i32).extractSigned(0x8000_0000), .min)
        XCTAssertEqual(FieldSpec(word: 0, shift: 31, bits: 1, kind: .i32).extractSigned(0x8000_0000), -1)
        XCTAssertTrue(label.fits(words: 5))
        XCTAssertFalse(label.fits(words: 4))
        XCTAssertFalse(FieldSpec(word: 0, shift: 24, bits: 16, kind: .u32).fits(words: 1))
        XCTAssertFalse(FieldSpec(word: 0, shift: 0, bits: 0, kind: .u32).fits(words: 1))
        XCTAssertEqual(LengthValue(word: 0x3F80_0000), LengthValue(unit: .points, value: 1))
        XCTAssertEqual(LengthValue(word: 0x4248_0001), LengthValue(unit: .percent, value: 50))
        XCTAssertEqual(LengthValue(word: 2), LengthValue(unit: .auto, value: 0))
        XCTAssertEqual(LengthValue(word: 0x3F80_0003), LengthValue(unit: .auto, value: 0))
        XCTAssertEqual(Rgba(packed: 0x8040_2010).r, 0x10)
        XCTAssertEqual(Rgba(packed: 0x8040_2010).a, 0x80)
    }

    func testTableReaderOverCMemory() throws {
        let s = try scene()
        let t = try XCTUnwrap(s.table(key: Key64.of("nve.visual")))
        XCTAssertEqual(t.words, 6)
        XCTAssertEqual(t.rowCount, 2)
        XCTAssertFalse(t.isSparse)
        XCTAssertEqual(t.u32(0, opacity), 200)
        XCTAssertEqual(t.i32(0, z), -3)
        XCTAssertEqual(t.i32(1, z), 2047)
        XCTAssertEqual(t.bool(0, visible), true)
        XCTAssertEqual(t.bool(1, visible), false)
        XCTAssertEqual(t.length(0, width), LengthValue(unit: .percent, value: 50))
        XCTAssertEqual(t.length(1, width)?.unit, .auto)
        XCTAssertEqual(t.color(0, color)?.g, 0x20)
        XCTAssertEqual(t.color(0, color)?.b, 0x40)
        XCTAssertEqual(t.str(0, label), StrSpan(offset: 0, len: 3))
        XCTAssertEqual(t.f32(0, scale), 1.5)
        XCTAssertEqual(t.f32(1, scale)?.sign, .minus)
        XCTAssertNil(t.u32(2, opacity))
        XCTAssertNil(t.word(0, 6))
        XCTAssertNil(t.str(0, FieldSpec(word: 5, kind: .str)))
        XCTAssertEqual(t.row(of: 1), 1)
        XCTAssertNil(t.row(of: 2))
        XCTAssertEqual(t.row(1).count, 6)
        XCTAssertTrue(t.row(9).isEmpty)
        XCTAssertNil(s.table(key: Key64(1)))
        XCTAssertNil(s.table(1))
    }

    func testSparseAndInvalidTables() {
        let rows: [UInt32] = [1, 2, 3, 4]
        let owners: [UInt32] = [0x0001_0007, 0x0002_0003]
        rows.withUnsafeBufferPointer { rp in
            owners.withUnsafeBufferPointer { op in
                let t = TableReader(key: Key64(1), slot: 0, words: 2, rowCount: 2, rows: rp, owners: op)
                XCTAssertEqual(t?.isSparse, true)
                XCTAssertEqual(t?.row(of: 0x0002_0003), 1)
                XCTAssertNil(t?.row(of: 1))
                XCTAssertEqual(t?.owner(0), 0x0001_0007)
                XCTAssertNil(TableReader(key: Key64(1), slot: 0, words: 2, rowCount: 3, rows: rp))
                XCTAssertNil(TableReader(key: Key64(1), slot: 0, words: 33, rowCount: 0, rows: rp))
                XCTAssertNil(TableReader(key: Key64(1), slot: 0, words: 0, rowCount: 0, rows: rp))
                XCTAssertNil(TableReader(key: Key64(1), slot: 0, words: 2, rowCount: 2, rows: rp,
                                         owners: UnsafeBufferPointer(rebasing: op[0..<1])))
            }
        }
        var nullRows = NveTestTableView()
        nullRows.words = 6
        nullRows.row_count = 2
        XCTAssertNil(TableReader(view: nullRows))
    }

    func testSceneReaderOverCMemory() throws {
        let s = try scene()
        XCTAssertEqual(s.frameId, 7)
        XCTAssertEqual(s.nodeCount, 3)
        let n0 = try XCTUnwrap(s.node(0))
        XCTAssertEqual(s.text(offset: n0.text_offset, len: UInt32(n0.text_len)), "hé")
        XCTAssertEqual(s.text(offset: 3, len: 4), "🚀")
        XCTAssertNil(s.text(offset: 2, len: 1))  // splits "é"
        XCTAssertNil(s.text(offset: 4, len: 3))  // starts mid-rocket
        XCTAssertNil(s.text(offset: 7, len: 1))
        XCTAssertEqual(s.text(offset: 7, len: 0), "")
        XCTAssertNil(s.text(offset: .max, len: 2))
        XCTAssertEqual(s.children(offset: n0.child_offset, count: UInt32(n0.child_count)).map(Array.init), [2, 3])
        XCTAssertNil(s.children(offset: 1, count: 2))
        XCTAssertNil(s.node(3))
        var visited: [Int] = []
        s.forEachDirty { index, node in
            XCTAssertEqual(node.node_id, UInt32(index + 1))
            visited.append(index)
        }
        XCTAssertEqual(visited, [0, 2])
        XCTAssertEqual(Array(s.removedIds), [42])
    }

    func testSceneReaderRejects() throws {
        XCTAssertThrowsError(try SceneReader<NveTestSceneView>.open(nil, expectedSchema: schema)) {
            XCTAssertEqual($0 as? SceneError, .nullView)
        }
        XCTAssertThrowsError(try SceneReader.open(nve_test_scene(), expectedSchema: Key64(1))) {
            XCTAssertEqual($0 as? SceneError, .schemaMismatch(expected: Key64(1), actual: schema))
        }
        let base = try XCTUnwrap(nve_test_scene()).pointee
        var v = base
        v.children_slab = nil
        XCTAssertThrowsError(try SceneReader.open(v, expectedSchema: schema)) { XCTAssertEqual($0 as? SceneError, .nullSlab) }
        v = base
        v.node_count = 0
        v.nodes = nil
        XCTAssertNoThrow(try SceneReader.open(v, expectedSchema: schema))
        let bad: [UInt8] = [0xC0, 0xAF]
        try bad.withUnsafeBufferPointer { bp in
            var bv = base
            bv.text_slab = bp.baseAddress
            bv.text_slab_len = 2
            XCTAssertThrowsError(try SceneReader.open(bv, expectedSchema: schema)) {
                XCTAssertEqual($0 as? SceneError, .invalidUtf8)
            }
            _ = try XCTUnwrap(bp.baseAddress)
        }
        var badTable = try XCTUnwrap(base.tables).pointee
        badTable.words = 40
        try withUnsafePointer(to: badTable) { tp in
            var tv = base
            tv.tables = tp
            XCTAssertThrowsError(try SceneReader.open(tv, expectedSchema: schema)) {
                XCTAssertEqual($0 as? SceneError, .invalidTable(index: 0))
            }
            _ = try XCTUnwrap(Optional(tp))
        }
    }
}
