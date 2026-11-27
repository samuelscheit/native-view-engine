import CNveAbi
import NveHost
import XCTest

// The generated C structs satisfy the runtime protocols with empty extensions.
extension NveTableView: TableViewABI {}
extension NveSceneView: SceneViewABI {}

/// `nve codegen abi` output against this package's static runtime: generated table keys must be
/// `xxh64(name)`, every field spec must fit its row, and `SceneReader` must open a frame built
/// from the generated `NveSceneView` / `NveRenderNode` / `NveTableView`.
final class GeneratedAbiTests: XCTestCase {
    func testGeneratedTablesMatchTheRuntime() {
        XCTAssertEqual(NveTables.all.count, Int(NVE_TABLE_COUNT))
        for t in NveTables.all {
            XCTAssertEqual(t.key, Key64.of(t.name), t.name)
            XCTAssertTrue((1...32).contains(t.words), t.name)
            for f in t.fields {
                XCTAssertLessThanOrEqual(Int(f.word) + (f.kind == .str ? 2 : 1), t.words, t.name)
            }
        }
        XCTAssertEqual(NveTables.Layout.key, Key64(NVE_TABLE_LAYOUT_KEY))
        XCTAssertEqual(NveTables.Layout.defaults.count, NveTables.Layout.words)
    }

    func testGeneratedStrFieldReadsItsSpan() throws {
        var row = [UInt32](repeating: 0, count: NveTables.Accessibility.words)
        let label = NveTables.Accessibility.label
        row[Int(label.word)] = 5
        row[Int(label.word) + 1] = 7
        try row.withUnsafeBufferPointer { rows in
            let table = try XCTUnwrap(TableReader(
                key: NveTables.Accessibility.key, slot: 0, words: rows.count, rowCount: 1, rows: rows))
            XCTAssertEqual(table.str(0, label), StrSpan(offset: 5, len: 7))
        }
    }

    func testSceneReaderOpensAGeneratedFrame() throws {
        let row = NveTables.Layout.defaults
        let text = Array("hi".utf8)
        var node = NveRenderNode()
        node.node_id = NVE_ROOT_NODE_ID
        node.primary_slot = UInt8(NVE_PRIMARY_SLOT_NONE)
        node.text_len = 2
        try row.withUnsafeBufferPointer { rows in
            try text.withUnsafeBufferPointer { slab in
                try withUnsafePointer(to: node) { nodePtr in
                    let key = NveTables.Layout.key
                    var table = NveTableView(
                        key_lo: key.lo, key_hi: key.hi, slot: 0, words: UInt32(rows.count), row_count: 1,
                        dirty_count: 0, rows: rows.baseAddress, owners: nil, dirty_rows: nil)
                    try withUnsafePointer(to: &table) { tablePtr in
                        var view = NveSceneView()
                        view.schema_hash = NVE_SCENE_SCHEMA_HASH
                        view.node_count = 1
                        view.text_slab_len = 2
                        view.table_count = 1
                        view.nodes = nodePtr
                        view.text_slab = slab.baseAddress
                        view.tables = tablePtr
                        try withUnsafePointer(to: &view) { viewPtr in
                            let scene = try SceneReader.open(viewPtr, expectedSchema: Key64(NVE_SCENE_SCHEMA_HASH))
                            XCTAssertEqual(scene.nodes.count, 1)
                            let layout = try XCTUnwrap(scene.table(key: NveTables.Layout.key))
                            XCTAssertEqual(layout.f32(0, NveTables.Layout.flexShrink), 1.0)
                            XCTAssertThrowsError(
                                try SceneReader.open(viewPtr, expectedSchema: Key64(NVE_CORE_ABI_SCHEMA_HASH)))
                        }
                    }
                }
            }
        }
    }
}
