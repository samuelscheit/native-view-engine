import CNveAbi
import Foundation
import NveSwiftUI
import XCTest

final class NveSceneAndBinValueTests: XCTestCase {
    func testRenderNodeCAbiLayoutMatches64ByteCacheLine() {
        XCTAssertEqual(MemoryLayout<NveRenderNode>.size, 64)
        XCTAssertEqual(MemoryLayout<NveRenderNode>.stride, 64)
        XCTAssertEqual(MemoryLayout<NveRenderNode>.alignment, 4)
        XCTAssertNotEqual(UInt64(NVE_SCENE_SCHEMA_HASH), 0)
    }

    func testSceneReaderZeroCopyReadsNodesDirtyTextAndChildren() throws {
        let textBytes = Array("Hello 🚀World".utf8)
        let children: [UInt32] = [2, 3]
        let dirty: [UInt32] = [1]
        let removed: [UInt32] = [99]

        let nodes = [
            NveRenderNode(
                node_id: 1,
                kind: UInt8(NVE_NODE_KIND_ROOT),
                flags: 0,
                opacity_u8: 255,
                _pad0: 0,
                x: 0,
                y: 0,
                width: 390,
                height: 844,
                bg_rgba: 0xFF0F_172A,
                text_rgba: 0,
                border_rgba: 0,
                border_radius: 0,
                border_width: 0,
                font_size: 16,
                text_offset: 0,
                text_len: 0,
                child_count: 2,
                child_offset: 0,
                z_index: 0
            ),
            NveRenderNode(
                node_id: 2,
                kind: UInt8(NVE_NODE_KIND_TEXT),
                flags: UInt8(NVE_NODE_LEAF_TEXT | NVE_NODE_INTERACTIVE),
                opacity_u8: 128,
                _pad0: 0,
                x: 16,
                y: 24,
                width: 200,
                height: 32,
                bg_rgba: 0xFF1E_293B,
                text_rgba: 0xFFFF_FFFF,
                border_rgba: 0xFF38_BDF8,
                border_radius: 12,
                border_width: 2,
                font_size: 18,
                text_offset: 0,
                text_len: UInt16("Hello 🚀".utf8.count),
                child_count: 0,
                child_offset: 0,
                z_index: 3
            ),
        ]

        try nodes.withUnsafeBufferPointer { nodesBuf in
            try textBytes.withUnsafeBufferPointer { textBuf in
                try children.withUnsafeBufferPointer { childBuf in
                    try dirty.withUnsafeBufferPointer { dirtyBuf in
                        try removed.withUnsafeBufferPointer { removedBuf in
                            let view = NveSceneView(
                                schema_hash: UInt64(NVE_SCENE_SCHEMA_HASH),
                                frame_id: 7,
                                node_count: UInt32(nodesBuf.count),
                                dirty_count: UInt32(dirtyBuf.count),
                                removed_count: UInt32(removedBuf.count),
                                text_slab_len: UInt32(textBuf.count),
                                children_slab_len: UInt32(childBuf.count),
                                table_count: 0,
                                nodes: nodesBuf.baseAddress,
                                text_slab: textBuf.baseAddress,
                                children_slab: childBuf.baseAddress,
                                dirty_indices: dirtyBuf.baseAddress,
                                removed_ids: removedBuf.baseAddress,
                                tables: nil
                            )

                            let reader = try NveSceneReader(view: view)
                            XCTAssertEqual(reader.frameId, 7)
                            XCTAssertEqual(reader.count, 2)
                            XCTAssertEqual(reader.dirtyCount, 1)
                            XCTAssertEqual(reader.removedCount, 1)
                            XCTAssertEqual(Array(reader.removedIds), [99])

                            let root = reader[0]
                            XCTAssertEqual(Array(try reader.children(for: root)), [2, 3])
                            XCTAssertEqual(try reader.text(for: root), "")

                            var visitedDirtyIds: [UInt32] = []
                            try reader.forEachDirty { idx, node in
                                XCTAssertEqual(idx, 1)
                                visitedDirtyIds.append(node.node_id)
                                XCTAssertTrue(NveSceneReader.isLeafText(node))
                                XCTAssertTrue(NveSceneReader.isInteractive(node))
                                XCTAssertFalse(NveSceneReader.clipsBounds(node))
                                XCTAssertEqual(try reader.text(for: node), "Hello 🚀")
                                XCTAssertEqual(NveSceneReader.opacity(for: node), 128.0 / 255.0, accuracy: 0.0001)
                            }
                            XCTAssertEqual(visitedDirtyIds, [2])
                        }
                    }
                }
            }
        }
    }

    func testSceneReaderRejectsBadHashNullPointersAndBoundsViolations() throws {
        let badView = NveSceneView(
            schema_hash: UInt64(NVE_SCENE_SCHEMA_HASH) ^ 0xFF,
            frame_id: 1,
            node_count: 0,
            dirty_count: 0,
            removed_count: 0,
            text_slab_len: 0,
            children_slab_len: 0,
            table_count: 0,
            nodes: nil,
            text_slab: nil,
            children_slab: nil,
            dirty_indices: nil,
            removed_ids: nil,
            tables: nil
        )
        XCTAssertThrowsError(try NveSceneReader(view: badView)) { err in
            XCTAssertEqual(
                err as? NveSceneReaderError,
                .schemaHashMismatch(
                    expected: UInt64(NVE_SCENE_SCHEMA_HASH),
                    actual: UInt64(NVE_SCENE_SCHEMA_HASH) ^ 0xFF
                )
            )
        }

        let nullNodesView = NveSceneView(
            schema_hash: UInt64(NVE_SCENE_SCHEMA_HASH),
            frame_id: 1,
            node_count: 1,
            dirty_count: 0,
            removed_count: 0,
            text_slab_len: 0,
            children_slab_len: 0,
            table_count: 0,
            nodes: nil,
            text_slab: nil,
            children_slab: nil,
            dirty_indices: nil,
            removed_ids: nil,
            tables: nil
        )
        XCTAssertThrowsError(try NveSceneReader(view: nullNodesView)) { err in
            XCTAssertEqual(err as? NveSceneReaderError, .nullBufferPointer("nodes"))
        }

        let invalidUtf8: [UInt8] = [0xFF, 0xFE]
        let oobNode = NveRenderNode(
            node_id: 5,
            kind: UInt8(NVE_NODE_KIND_TEXT),
            flags: UInt8(NVE_NODE_LEAF_TEXT),
            opacity_u8: 255,
            _pad0: 0,
            x: 0,
            y: 0,
            width: 10,
            height: 10,
            bg_rgba: 0,
            text_rgba: 0,
            border_rgba: 0,
            border_radius: 0,
            border_width: 0,
            font_size: 16,
            text_offset: 0,
            text_len: 2,
            child_count: 3,
            child_offset: 0,
            z_index: 0
        )
        try withUnsafePointer(to: oobNode) { nodePtr in
            try invalidUtf8.withUnsafeBufferPointer { textBuf in
                let view = NveSceneView(
                    schema_hash: UInt64(NVE_SCENE_SCHEMA_HASH),
                    frame_id: 1,
                    node_count: 1,
                    dirty_count: 0,
                    removed_count: 0,
                    text_slab_len: 2,
                    children_slab_len: 0,
                    table_count: 0,
                    nodes: nodePtr,
                    text_slab: textBuf.baseAddress,
                    children_slab: nil,
                    dirty_indices: nil,
                    removed_ids: nil,
                    tables: nil
                )
                let reader = try NveSceneReader(view: view)
                XCTAssertThrowsError(try reader.text(for: reader[0])) { err in
                    XCTAssertEqual(err as? NveSceneReaderError, .invalidUtf8(nodeId: 5))
                }
                XCTAssertThrowsError(try reader.children(for: reader[0])) { err in
                    XCTAssertEqual(
                        err as? NveSceneReaderError,
                        .childrenOutOfBounds(nodeId: 5, offset: 0, count: 3, slabLength: 0)
                    )
                }
            }
        }
    }

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
