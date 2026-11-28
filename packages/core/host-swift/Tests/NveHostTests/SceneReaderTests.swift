import CNveHostTestABI
import NveHost
import XCTest

final class SceneReaderTests: XCTestCase {
    private let schema = Key64(0xF00D_FACE_C0FF_EE11)

    private func scene() throws -> SceneReader<NveTestRuntimeSceneView> {
        try SceneReader.open(nve_test_scene(), expectedSchema: schema)
    }

    private func assertRejects(_ v: NveTestRuntimeSceneView, _ expected: SceneError, line: UInt = #line) {
        XCTAssertThrowsError(try SceneReader.open(v, expectedSchema: schema), line: line) {
            XCTAssertEqual($0 as? SceneError, expected, line: line)
        }
    }

    func testNodeIdPacking() {
        let id = NodeId(index: 0x12_3456, generation: 0xAB)
        XCTAssertEqual(id.raw, 0xAB12_3456)
        XCTAssertEqual(id.index, 0x12_3456)
        XCTAssertEqual(id.generation, 0xAB)
        XCTAssertEqual(NodeId(index: 0xFFFF_FFFF, generation: 1).raw, 0x01FF_FFFF)  // masked like Rust
        XCTAssertEqual(NodeId.root.index, 1)
        XCTAssertEqual(NodeId.root.generation, 0)
        XCTAssertTrue(NodeId.null.isNull)
        XCTAssertFalse(NodeId.root.isNull)
        XCTAssertEqual(NodeId(raw: 0x0200_0003).description, "NodeId(3v2)")
    }

    func testRowsRectsAndIdentity() throws {
        let s = try scene()
        XCTAssertEqual(s.frameId, 7)
        XCTAssertEqual(s.sceneEpoch, 5)
        XCTAssertEqual(Array(s.dirtyResources), [3, 9])
        XCTAssertEqual(Array(s.movedIndices), [2])
        XCTAssertEqual(s.nodeCapacity, 4)
        XCTAssertFalse(s.isLive(0))  // reserved null slot
        XCTAssertTrue(s.isLive(1))
        XCTAssertTrue(s.isLive(2))
        XCTAssertFalse(s.isLive(3))
        XCTAssertFalse(s.isLive(4))
        XCTAssertFalse(s.isLive(-1))
        XCTAssertNil(s.row(4))
        XCTAssertNil(s.rect(-1))
        XCTAssertEqual(s.nodeId(at: 1), .root)
        XCTAssertEqual(s.nodeId(at: 2), NodeId(index: 2, generation: 1))
        XCTAssertNil(s.nodeId(at: 3))
        XCTAssertNil(s.nodeId(at: 0))
        XCTAssertEqual(s.index(of: .root), 1)
        XCTAssertEqual(s.index(of: NodeId(index: 2, generation: 1)), 2)
        XCTAssertNil(s.index(of: NodeId(index: 2, generation: 0)))  // stale generation
        XCTAssertNil(s.index(of: NodeId(raw: 0x0200_0003)))  // dead slot
        XCTAssertNil(s.index(of: .null))
        XCTAssertNil(s.index(of: NodeId(index: NodeId.maxIndex, generation: 0)))
        let leaf = try XCTUnwrap(s.row(2))
        XCTAssertEqual(leaf.parentId, .root)
        XCTAssertEqual(leaf.primary_slot, 5)
        XCTAssertEqual(leaf.effects, 4)
        XCTAssertEqual(s.row(1)?.primary_slot, primarySlotNone)
        let r = try XCTUnwrap(s.rect(2))
        XCTAssertEqual([r.x, r.y, r.width, r.height], [10, 20, 100, 50])
        XCTAssertEqual(s.rect(1)?.width, 320)
    }

    func testTableBitset() throws {
        let s = try scene()
        XCTAssertTrue(s.hasTable(2, slot: 5))
        XCTAssertTrue(s.hasTable(2, slot: 33))
        XCTAssertTrue(s.hasTable(2, slot: 127))
        XCTAssertFalse(s.hasTable(2, slot: 32))
        XCTAssertFalse(s.hasTable(2, slot: 64))
        XCTAssertFalse(s.hasTable(2, slot: 128))
        XCTAssertFalse(s.hasTable(2, slot: 255))
        XCTAssertFalse(s.hasTable(9, slot: 5))
        XCTAssertTrue(s.hasTable(1, slot: 5))
        XCTAssertFalse(s.hasTable(1, slot: 6))
        // Primary-slot lookup resolves the realizing table without knowing its key.
        let primary = try XCTUnwrap(s.row(2)?.primary_slot)
        XCTAssertEqual(s.table(slot: primary)?.key, Key64.of("nve.visual"))
        XCTAssertNil(s.table(slot: 0))
        XCTAssertNil(s.table(slot: primarySlotNone))
    }

    func testTextChildrenDirtyRemoved() throws {
        let s = try scene()
        XCTAssertEqual(s.text(offset: 0, len: 3), "hé")
        XCTAssertEqual(s.text(StrSpan(offset: 3, len: 4)), "🚀")
        XCTAssertNil(s.text(offset: 2, len: 1))  // splits "é"
        XCTAssertNil(s.text(offset: 4, len: 3))  // starts mid-rocket
        XCTAssertNil(s.text(offset: 7, len: 1))
        XCTAssertEqual(s.text(offset: 7, len: 0), "")
        XCTAssertNil(s.text(offset: .max, len: .max))
        XCTAssertNil(s.text(offset: 1, len: .max))
        XCTAssertEqual(s.children(of: 1).map(Array.init), [0x0100_0002, 0x0200_0003])
        XCTAssertEqual(s.children(of: 2).map(Array.init), [])
        XCTAssertNil(s.children(of: 7))
        XCTAssertNil(s.children(offset: 1, count: 2))
        XCTAssertNil(s.children(offset: .max, count: .max))
        // Child ids resolve through generations: the dead child drops out.
        let live = try XCTUnwrap(s.children(of: 1)).compactMap { s.index(of: NodeId(raw: $0)) }
        XCTAssertEqual(live, [2])
        var visited: [Int] = []
        s.forEachDirty { index, row, rect in
            XCTAssertTrue(row.isLive)
            XCTAssertEqual(rect.width, s.rect(index)?.width)
            visited.append(index)
        }
        XCTAssertEqual(visited, [1, 2])  // 99 is out of range and skipped
        XCTAssertEqual(s.removedIds.map(NodeId.init(raw:)), [NodeId(index: 3, generation: 2)])
        XCTAssertEqual(s.scrollOffset(for: .root).y, 12.5)
        XCTAssertEqual(s.scrollOffset(for: NodeId(index: 2, generation: 1)).y, 0, "no scroll entry")
    }

    func testRejectsHostileViews() throws {
        XCTAssertThrowsError(try SceneReader<NveTestRuntimeSceneView>.open(nil, expectedSchema: schema)) {
            XCTAssertEqual($0 as? SceneError, .nullView)
        }
        XCTAssertThrowsError(try SceneReader.open(nve_test_scene(), expectedSchema: Key64(1))) {
            XCTAssertEqual($0 as? SceneError, .schemaMismatch(expected: Key64(1), actual: schema))
        }
        let base = try XCTUnwrap(nve_test_scene()).pointee
        var v = base
        v.rows = nil
        assertRejects(v, .nullSlab)
        v = base
        v.rects = nil
        assertRejects(v, .nullSlab)
        v = base
        v.children_slab = nil
        assertRejects(v, .nullSlab)
        v = base
        v.dirty_indices = nil
        assertRejects(v, .nullSlab)
        v = base
        v.removed_ids = nil
        assertRejects(v, .nullSlab)
        v = base
        v.tables = nil
        assertRejects(v, .nullSlab)
        v = base
        v.scroll_offsets = nil
        assertRejects(v, .nullSlab)
        v = base
        v.dirty_resources = nil
        assertRejects(v, .nullSlab)
        v = base
        XCTAssertGreaterThan(v.moved_count, 0)
        v.moved_indices = nil
        assertRejects(v, .nullSlab)
        // Counts past the address space are refused before a span is formed.
        v = base
        v.node_capacity = .max
        let fits = UInt64(UInt32.max) <= UInt64(Int.max / MemoryLayout<NveTestNodeRow>.stride)
        if !fits { assertRejects(v, .nullSlab) }
        // Empty views are valid with null pointers.
        var empty = NveTestRuntimeSceneView()
        empty.schema_hash = schema.value
        let e = try SceneReader.open(empty, expectedSchema: schema)
        XCTAssertEqual(e.nodeCapacity, 0)
        XCTAssertNil(e.nodeId(at: 1))
        XCTAssertNil(e.children(of: 1))
        e.forEachDirty { _, _, _ in XCTFail("no dirty rows") }
    }

    func testRejectsMisalignedSlabs() throws {
        let base = try XCTUnwrap(nve_test_scene()).pointee
        let words: [UInt32] = [0, 0, 0]
        try words.withUnsafeBytes { raw in
            let odd = try XCTUnwrap(raw.baseAddress).advanced(by: 1)
            var v = base
            v.children_slab = odd.assumingMemoryBound(to: UInt32.self)
            assertRejects(v, .misalignedSlab)
            v = base
            v.dirty_indices = odd.assumingMemoryBound(to: UInt32.self)
            v.dirty_count = 1
            assertRejects(v, .misalignedSlab)
            v = base
            v.moved_indices = odd.assumingMemoryBound(to: UInt32.self)
            v.moved_count = 1
            assertRejects(v, .misalignedSlab)
            v = base
            v.dirty_resources = odd.assumingMemoryBound(to: UInt32.self)
            v.dirty_resource_count = 1
            assertRejects(v, .misalignedSlab)
        }
    }

    func testRejectsInvalidUtf8AndTables() throws {
        let base = try XCTUnwrap(nve_test_scene()).pointee
        let bad: [UInt8] = [0xC0, 0xAF]
        bad.withUnsafeBufferPointer { bp in
            var v = base
            v.text_slab = bp.baseAddress
            v.text_slab_len = 2
            assertRejects(v, .invalidUtf8)
        }
        var badTable = try XCTUnwrap(base.tables).pointee
        badTable.words = 40
        withUnsafePointer(to: badTable) { tp in
            var v = base
            v.tables = tp
            assertRejects(v, .invalidTable(index: 0))
        }
    }

    func testReadsAreAllocationFree() throws {
        #if DEBUG
        try XCTSkipIf(true, "allocation counts are only meaningful in optimized builds (swift test -c release)")
        #endif
        let s = try scene()
        var sum: UInt64 = 0
        func pass(_ iterations: Int) {
            for _ in 0..<iterations {
                s.forEachDirty { index, row, rect in sum &+= UInt64(index) &+ UInt64(row.child_len) &+ UInt64(rect.width) }
                for raw in s.children(of: 1) ?? UnsafeBufferPointer(start: nil, count: 0) {
                    sum &+= UInt64(s.index(of: NodeId(raw: raw)) ?? 0)
                }
                sum &+= UInt64(s.textBytes(offset: 0, len: 3)?.count ?? 0)
                sum &+= s.hasTable(2, slot: 127) ? 1 : 0
                sum &+= UInt64(s.table(slot: 5)?.rowCount ?? 0)
            }
        }
        pass(2)  // warm up first-use runtime metadata
        guard nve_test_alloc_count_begin() != 0 else { throw XCTSkip("no malloc hook on this platform") }
        pass(64)
        let allocs = nve_test_alloc_count_end()
        XCTAssertGreaterThan(sum, 0)
        XCTAssertEqual(allocs, 0, "SceneReader reads allocated \(allocs) times in 64 passes")
    }
}
