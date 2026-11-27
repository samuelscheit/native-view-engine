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
    func testDirectEncodableTypedPropsZeroHeapStackSlabOverflowAndLiveCounterDylib() throws {
        // 1. Small typed payload stays completely on the 4 KiB stack slab (zero heap chunks!).
        let small = SampleTypedProps(
            title: "ZeroAlloc 🚀",
            count: -42,
            ratio: 2.5,
            muted: true,
            quality: .uhd4k,
            optionalNote: nil,
            payload: [0xCA, 0xFE],
            meta: SampleNestedMeta(tag: "fast", version: 3)
        )

        NveDirectArena.withArena { arena in
            let titleKey: StaticString = "title"
            let keySlice = arena.staticStringSlice(titleKey)
            XCTAssertEqual(keySlice.ptr, titleKey.utf8Start)
            XCTAssertEqual(keySlice.len, titleKey.utf8CodeUnitCount)
            XCTAssertEqual(arena.heapChunkCount, 0)
        }

        let capturedSmall: [(String, NveBinValue)] = try NveDirectPropsWriter.withEncodedProps(small) { ptr, count in
            XCTAssertEqual(count, 8)
            let base = try XCTUnwrap(ptr)
            return (0 ..< Int(count)).map { idx in
                (readSliceString(base[idx].key), readDirectValue(base[idx].value))
            }
        }
        XCTAssertEqual(
            NveBinValue.map(capturedSmall),
            NveBinValue.map([
                ("title", .string("ZeroAlloc 🚀")),
                ("count", .int64(-42)),
                ("ratio", .float64(2.5)),
                ("muted", .bool(true)),
                ("quality", .int64(11)),
                ("optional_note", .null),
                ("payload", .bytes(Data([0xCA, 0xFE]))),
                ("meta", .map([("tag", .string("fast")), ("version", .int64(3))])),
            ])
        )

        // 2. Empty typed props passes (nil, 0).
        try NveDirectPropsWriter.withEncodedProps(EmptyTypedProps()) { ptr, count in
            XCTAssertNil(ptr)
            XCTAssertEqual(count, 0)
        }

        // 3. Large >4 KiB payload cleanly spills to heap overflow chunks without moving earlier slices.
        let hugeTitle = String(repeating: "ArenaOverflow-⚡-", count: 400) // > 6 KiB
        let hugePayload = [UInt8](repeating: 0xAB, count: 5000)
        let large = SampleTypedProps(
            title: hugeTitle,
            count: 999,
            ratio: 0.75,
            muted: false,
            quality: .auto,
            optionalNote: "present",
            payload: hugePayload,
            meta: SampleNestedMeta(tag: "overflow", version: 9)
        )

        NveDirectArena.withArena { arena in
            _ = arena.copyString(hugeTitle)
            _ = arena.copyBytes(hugePayload)
            XCTAssertGreaterThanOrEqual(arena.heapChunkCount, 1)
        }

        let capturedLarge: [(String, NveBinValue)] = try NveDirectPropsWriter.withEncodedProps(large) { ptr, count in
            XCTAssertEqual(count, 8)
            let base = try XCTUnwrap(ptr)
            return (0 ..< Int(count)).map { idx in
                (readSliceString(base[idx].key), readDirectValue(base[idx].value))
            }
        }
        XCTAssertEqual(capturedLarge[0].1, .string(hugeTitle))
        XCTAssertEqual(capturedLarge[5].1, .string("present"))
        XCTAssertEqual(capturedLarge[6].1, .bytes(Data(hugePayload)))

        // 4. Live end-to-end round-trip against built Rust `libnve_example_kitchen_sink.dylib` when present.
        let workspaceRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // NveSwiftUITests
            .deletingLastPathComponent() // Tests
            .deletingLastPathComponent() // swiftui-kit
            .deletingLastPathComponent() // hosts
            .deletingLastPathComponent() // workspace root
        let kitchenSinkDylib = workspaceRoot.appendingPathComponent("target/debug/libnve_example_kitchen_sink.dylib")
        if FileManager.default.fileExists(atPath: kitchenSinkDylib.path) {
            struct KitchenSinkDirectProps: Equatable, NveDirectEncodable {
                var initialCount: Int64
                var label: String
                var directPropCount: Int { 2 }
                func encodeDirect(into builder: inout NveDirectPropsBuilder) throws {
                    builder.write("initialCount", initialCount)
                    builder.write("label", label)
                }
            }
            let controller = NveSurfaceController(
                surfaceId: 901,
                dylibPath: kitchenSinkDylib.path
            )
            XCTAssertTrue(controller.isMounted)
            _ = controller.setDirectProps(KitchenSinkDirectProps(initialCount: 7, label: "TypedKitchenSink"))
            let frameBytes = controller.renderFrame()
            XCTAssertGreaterThan(frameBytes.count, 8)
            controller.destroy()
        }
    }
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

    func testSceneStoreHitTestingScrollOffsetAndIncrementalDirtyUpdates() async throws {
        let store = NveSceneStore()

        // Scene layout:
        // root(1): [0,0, 300x200] -> children [2, 3, 4]
        // node(2): interactive button zIndex=10 at [10, 10, 100x40] (overlaps node 3)
        // node(3): interactive button zIndex=1  at [10, 10, 100x40]
        // node(4): ScrollView at [0, 60, 300x100] with clipsBounds -> child [5, 6]
        // node(5): interactive item at [10, 70, 280x60] (visible initially)
        // node(6): interactive item at [10, 200, 280x60] (clipped below scroll viewport until scrolled!)
        let textBytes = Array("BtnTopBtnLowItem1Item2".utf8)
        let children: [UInt32] = [2, 3, 4, 5, 6]
        var nodes: [NveRenderNode] = [
            NveRenderNode(
                node_id: 1,
                kind: UInt8(NVE_NODE_KIND_ROOT),
                flags: 0,
                opacity_u8: 255,
                _pad0: 0,
                x: 0,
                y: 0,
                width: 300,
                height: 200,
                bg_rgba: packRgba(15, 23, 42),
                text_rgba: 0,
                border_rgba: 0,
                border_radius: 0,
                border_width: 0,
                font_size: 16,
                text_offset: 0,
                text_len: 0,
                child_count: 3,
                child_offset: 0,
                z_index: 0
            ),
            NveRenderNode(
                node_id: 2,
                kind: UInt8(NVE_NODE_KIND_PRESSABLE),
                flags: UInt8(NVE_NODE_INTERACTIVE | NVE_NODE_LEAF_TEXT),
                opacity_u8: 230,
                _pad0: 0,
                x: 10,
                y: 10,
                width: 100,
                height: 40,
                bg_rgba: packRgba(37, 99, 235),
                text_rgba: packRgba(255, 255, 255),
                border_rgba: packRgba(147, 197, 253),
                border_radius: 8,
                border_width: 2,
                font_size: 14,
                text_offset: 0,
                text_len: 6,
                child_count: 0,
                child_offset: 0,
                z_index: 10
            ),
            NveRenderNode(
                node_id: 3,
                kind: UInt8(NVE_NODE_KIND_PRESSABLE),
                flags: UInt8(NVE_NODE_INTERACTIVE | NVE_NODE_LEAF_TEXT),
                opacity_u8: 255,
                _pad0: 0,
                x: 10,
                y: 10,
                width: 100,
                height: 40,
                bg_rgba: packRgba(220, 38, 38),
                text_rgba: packRgba(255, 255, 255),
                border_rgba: 0,
                border_radius: 0,
                border_width: 0,
                font_size: 14,
                text_offset: 6,
                text_len: 6,
                child_count: 0,
                child_offset: 0,
                z_index: 1
            ),
            NveRenderNode(
                node_id: 4,
                kind: UInt8(NVE_NODE_KIND_SCROLL_VIEW),
                flags: UInt8(NVE_NODE_CLIP_BOUNDS),
                opacity_u8: 255,
                _pad0: 0,
                x: 0,
                y: 60,
                width: 300,
                height: 100,
                bg_rgba: packRgba(30, 41, 59),
                text_rgba: 0,
                border_rgba: 0,
                border_radius: 6,
                border_width: 0,
                font_size: 14,
                text_offset: 0,
                text_len: 0,
                child_count: 2,
                child_offset: 3,
                z_index: 0
            ),
            NveRenderNode(
                node_id: 5,
                kind: UInt8(NVE_NODE_KIND_PRESSABLE),
                flags: UInt8(NVE_NODE_INTERACTIVE | NVE_NODE_LEAF_TEXT),
                opacity_u8: 255,
                _pad0: 0,
                x: 10,
                y: 70,
                width: 280,
                height: 60,
                bg_rgba: packRgba(51, 65, 85),
                text_rgba: packRgba(255, 255, 255),
                border_rgba: 0,
                border_radius: 6,
                border_width: 0,
                font_size: 14,
                text_offset: 12,
                text_len: 5,
                child_count: 0,
                child_offset: 0,
                z_index: 0
            ),
            NveRenderNode(
                node_id: 6,
                kind: UInt8(NVE_NODE_KIND_PRESSABLE),
                flags: UInt8(NVE_NODE_INTERACTIVE | NVE_NODE_LEAF_TEXT),
                opacity_u8: 255,
                _pad0: 0,
                x: 10,
                y: 200,
                width: 280,
                height: 60,
                bg_rgba: packRgba(16, 185, 129),
                text_rgba: packRgba(255, 255, 255),
                border_rgba: 0,
                border_radius: 6,
                border_width: 0,
                font_size: 14,
                text_offset: 17,
                text_len: 5,
                child_count: 0,
                child_offset: 0,
                z_index: 0
            ),
        ]

        func applyScene(frameId: UInt64, dirty: [UInt32], removed: [UInt32] = []) throws {
            try nodes.withUnsafeBufferPointer { nodesBuf in
                try textBytes.withUnsafeBufferPointer { textBuf in
                    try children.withUnsafeBufferPointer { childBuf in
                        try dirty.withUnsafeBufferPointer { dirtyBuf in
                            try removed.withUnsafeBufferPointer { removedBuf in
                                let view = NveSceneView(
                                    schema_hash: UInt64(NVE_SCENE_SCHEMA_HASH),
                                    frame_id: frameId,
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
                                try store.apply(view: view)
                            }
                        }
                    }
                }
            }
        }

        // Frame 1: initial full load
        try applyScene(frameId: 1, dirty: [0, 1, 2, 3, 4, 5])
        XCTAssertEqual(store.frameId, 1)
        XCTAssertEqual(store.nodeCount, 6)

        // Reverse z-order: node 2 (z=10) wins over later-in-tree node 3 (z=1).
        XCTAssertEqual(store.hitTest(at: CGPoint(x: 50, y: 25)), 2)

        // Scrollable hit-test finds ScrollView (4) inside [0, 60..160].
        XCTAssertEqual(store.hitTestScrollable(at: CGPoint(x: 50, y: 100)), 4)
        XCTAssertNil(store.hitTestScrollable(at: CGPoint(x: 50, y: 25)))

        // Before scrolling: item 5 is hittable at y=100; item 6 (at y=200) is clipped outside viewport!
        XCTAssertEqual(store.hitTest(at: CGPoint(x: 50, y: 100)), 5)
        XCTAssertNil(store.hitTest(at: CGPoint(x: 50, y: 220)))

        // Scroll down by 110px (contentHeight = (200 - 60) + 60 = 200, viewport = 100 -> clamped to 100).
        let scrolled = try XCTUnwrap(store.scrollBy(nodeId: 4, deltaX: 0, deltaY: 110))
        XCTAssertEqual(scrolled.offset.y, 100, accuracy: 0.01)
        XCTAssertEqual(scrolled.contentSize.height, 200, accuracy: 0.01)

        // After scrolling by +100, point y=120 maps to child y=220 -> hits node 6!
        XCTAssertEqual(store.hitTest(at: CGPoint(x: 50, y: 120)), 6)

        // Frame 2: incremental dirty update ONLY on node index 1 (hide node 2 via opacity_u8 = 0).
        nodes[1].opacity_u8 = 0
        try applyScene(frameId: 2, dirty: [1])
        XCTAssertEqual(store.frameId, 2)
        XCTAssertEqual(store.lastDirtyCount, 1)
        XCTAssertEqual(store.node(forId: 2)?.opacity_u8, 0)
        // Now node 2 is invisible, so hit-testing at (50, 25) falls through to node 3!
        XCTAssertEqual(store.hitTest(at: CGPoint(x: 50, y: 25)), 3)

        #if canImport(AppKit)
        // Verify NveSceneCanvasView renders cleanly inside an AppKit hosting view without crashing.
        await MainActor.run {
            let canvasView = NveSceneCanvasView(
                store: store,
                surfaceId: 1,
                backgroundColor: .black
            )
            let hosting = NSHostingView(rootView: canvasView)
            hosting.frame = CGRect(x: 0, y: 0, width: 300, height: 200)
            hosting.layoutSubtreeIfNeeded()
            let rep = hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds)
            XCTAssertNotNil(rep)
            if let rep {
                hosting.cacheDisplay(in: hosting.bounds, to: rep)
                XCTAssertEqual(rep.size.width, 300, accuracy: 0.01)
                XCTAssertEqual(rep.size.height, 200, accuracy: 0.01)
                XCTAssertGreaterThanOrEqual(rep.pixelsWide, 300)
                XCTAssertEqual(rep.pixelsWide % 300, 0)
                XCTAssertGreaterThanOrEqual(rep.pixelsHigh, 200)
                XCTAssertEqual(rep.pixelsHigh % 200, 0)
            }
        }
        #endif
    }
}
