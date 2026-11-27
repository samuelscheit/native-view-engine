import CNveAbi
import Foundation
import NveHost
@testable import NveSwiftUI

/// Builds a published runtime scene in Swift memory: node rows / rects / slabs plus word-table
/// rows written through the generated `NveTables` field specs.
struct SceneBuilder {
    struct Table {
        let key: Key64
        let slot: UInt32
        let words: Int
        let sparse: Bool
        var rows: [UInt32] = []
        var owners: [UInt32] = []
    }

    var rows: [NveNodeRow] = [NveNodeRow()]
    var rects: [NveLayoutRect] = [NveLayoutRect()]
    var text: [UInt8] = []
    var children: [UInt32] = []
    var tables: [Table] = []

    /// Adds a live node at the next slot and returns its id.
    @discardableResult
    mutating func node(parent: NodeId = .null, rect: CGRect = .zero, generation: UInt8 = 0) -> NodeId {
        var row = NveNodeRow()
        row.parent = parent.raw
        row.generation = generation
        row.primary_slot = 0xFF
        row.flags = 1
        rows.append(row)
        rects.append(NveLayoutRect(x: Float(rect.minX), y: Float(rect.minY), width: Float(rect.width), height: Float(rect.height)))
        return NodeId(index: UInt32(rows.count - 1), generation: generation)
    }

    mutating func setChildren(_ parent: NodeId, _ kids: [NodeId]) {
        rows[parent.index].child_off = UInt32(children.count)
        rows[parent.index].child_len = UInt32(kids.count)
        children.append(contentsOf: kids.map(\.raw))
    }

    mutating func str(_ s: String) -> (UInt32, UInt32) {
        let off = UInt32(text.count)
        text.append(contentsOf: Array(s.utf8))
        return (off, UInt32(s.utf8.count))
    }

    /// Row words for `id` in table `key` (created dense-or-sparse on first use, filled with defaults).
    mutating func edit(_ key: Key64, words: Int, sparse: Bool, defaults: [UInt32], _ id: NodeId, _ body: (inout [UInt32]) -> Void) {
        let t: Int
        if let found = tables.firstIndex(where: { $0.key == key }) {
            t = found
        } else {
            tables.append(Table(key: key, slot: UInt32(tables.count), words: words, sparse: sparse))
            t = tables.count - 1
        }
        let slot = tables[t].slot
        let w = slot >> 5
        let bit = UInt32(1) << (slot & 31)
        switch w {
        case 0: rows[id.index].table_set.0 |= bit
        case 1: rows[id.index].table_set.1 |= bit
        case 2: rows[id.index].table_set.2 |= bit
        default: rows[id.index].table_set.3 |= bit
        }
        var row = defaults
        body(&row)
        if sparse {
            tables[t].owners.append(id.raw)
            tables[t].rows.append(contentsOf: row)
        } else {
            // Like the runtime: unattached dense rows are zero, not defaults.
            let need = (id.index + 1) * words
            if tables[t].rows.count < need {
                tables[t].rows.append(contentsOf: repeatElement(0, count: need - tables[t].rows.count))
            }
            tables[t].rows.replaceSubrange(id.index * words..<(id.index + 1) * words, with: row)
            // Like the runtime: dense tables publish owners too (row `i` belongs to node index `i`).
            if tables[t].owners.count <= id.index {
                tables[t].owners.append(contentsOf: repeatElement(0, count: id.index + 1 - tables[t].owners.count))
            }
            tables[t].owners[id.index] = id.raw
        }
    }

    static func put(_ row: inout [UInt32], _ f: FieldSpec, _ v: UInt32) {
        let mask: UInt32 = f.bits >= 32 ? .max : ((1 << UInt32(f.bits)) - 1) << UInt32(f.shift)
        row[Int(f.word)] = (row[Int(f.word)] & ~mask) | ((v << UInt32(f.shift)) & mask)
    }

    static func put(_ row: inout [UInt32], _ f: FieldSpec, _ v: Float) { put(&row, f, v.bitPattern) }

    static func put(_ row: inout [UInt32], _ f: FieldSpec, _ span: (UInt32, UInt32)) {
        row[Int(f.word)] = span.0
        row[Int(f.word) + 1] = span.1
    }

    static func rgba(_ r: UInt32, _ g: UInt32, _ b: UInt32, _ a: UInt32 = 255) -> UInt32 { r | g << 8 | b << 16 | a << 24 }

    mutating func visual(_ id: NodeId, _ body: (inout [UInt32]) -> Void) {
        edit(NveTables.Visual.key, words: NveTables.Visual.words, sparse: false, defaults: NveTables.Visual.defaults, id, body)
    }

    mutating func text(_ id: NodeId, _ s: String) {
        let span = str(s)
        edit(NveTables.Text.key, words: NveTables.Text.words, sparse: true, defaults: NveTables.Text.defaults, id) {
            Self.put(&$0, NveTables.Text.text, span)
        }
    }

    mutating func typography(_ id: NodeId, _ body: (inout [UInt32]) -> Void) {
        edit(NveTables.Typography.key, words: NveTables.Typography.words, sparse: true, defaults: NveTables.Typography.defaults, id, body)
    }

    mutating func sparse(_ key: Key64, words: Int, defaults: [UInt32], _ id: NodeId, _ body: (inout [UInt32]) -> Void = { _ in }) {
        edit(key, words: words, sparse: true, defaults: defaults, id, body)
    }
}

/// Recording `NveRuntimeApi` double serving a `SceneBuilder` scene (single global instance: C
/// function pointers cannot capture).
final class MockRuntime: @unchecked Sendable {
    static let shared = MockRuntime()
    static let handle = UnsafeMutableRawPointer(bitPattern: 0x1000)!

    var scene = SceneBuilder()
    var frameId: UInt64 = 1
    var removed: [UInt32] = []
    var dirty: [UInt32] = []
    var schema = NVE_RUNTIME_SCHEMA_HASH
    var tickChanged = true
    var failNextWith: Int32?
    var lastError = ""
    var hitResult: UInt32 = 0
    var scroll: [UInt32: (Float, Float)] = [:]
    var resources: [UInt32: [UInt8]] = [:]
    var scrollOffsets: [NveScrollOffset] = []
    var state: [UInt8] = []
    var emitted: [NveDirectEvent] = []
    private var drained: [NveDirectEvent] = []

    var creates = 0, destroys = 0, acquires = 0, releases = 0, ticks = 0, attaches = 0, detaches = 0
    var propCounts: [UInt32] = []
    var dispatched: [[NveEvent]] = []
    var viewports: [(Float, Float, Float)] = []
    var pairedUri = ""
    var paired = false
    var restored: [UInt8] = []
    var patches: [Int] = []
    var wake: (NveRuntimeWakeFn?, UnsafeMutableRawPointer?) = (nil, nil)
    var measurer: (NveRuntimeMeasureTextFn?, UnsafeMutableRawPointer?) = (nil, nil)

    private var pinned: [UnsafeMutableRawPointer] = []
    private var view = NveRuntimeSceneView()
    private var tableViews: [NveTableView] = []

    func reset() {
        let fresh = MockRuntime()
        scene = fresh.scene
        frameId = 1; removed = []; dirty = []; schema = NVE_RUNTIME_SCHEMA_HASH; tickChanged = true
        failNextWith = nil; lastError = ""; hitResult = 0; scroll = [:]; resources = [:]; scrollOffsets = []
        state = []; emitted = []; drained = []
        creates = 0; destroys = 0; acquires = 0; releases = 0; ticks = 0; attaches = 0; detaches = 0
        propCounts = []; dispatched = []; viewports = []; pairedUri = ""; paired = false; restored = []; patches = []
        wake = (nil, nil); measurer = (nil, nil)
    }

    fileprivate func pin<T>(_ values: [T]) -> UnsafePointer<T>? {
        guard !values.isEmpty else { return nil }
        let p = UnsafeMutablePointer<T>.allocate(capacity: values.count)
        p.initialize(from: values, count: values.count)
        pinned.append(UnsafeMutableRawPointer(p))
        return UnsafePointer(p)
    }

    fileprivate func publish() -> NveRuntimeSceneView {
        tableViews = scene.tables.map { t in
            NveTableView(
                key_lo: t.key.lo, key_hi: t.key.hi, slot: t.slot, words: UInt32(t.words),
                row_count: UInt32(t.rows.count / t.words), dirty_count: 0,
                rows: pin(t.rows), owners: pin(t.owners), dirty_rows: nil)
        }
        view = NveRuntimeSceneView(
            schema_hash: schema, frame_id: frameId, node_capacity: UInt32(scene.rows.count),
            dirty_count: UInt32(dirty.count), removed_count: UInt32(removed.count),
            text_slab_len: UInt32(scene.text.count), children_slab_len: UInt32(scene.children.count),
            table_count: UInt32(tableViews.count), scroll_count: UInt32(scrollOffsets.count), _reserved: 0,
            rows: pin(scene.rows), rects: pin(scene.rects),
            text_slab: pin(scene.text), children_slab: pin(scene.children), dirty_indices: pin(dirty),
            removed_ids: pin(removed), tables: pin(tableViews), scroll_offsets: pin(scrollOffsets))
        return view
    }

    fileprivate func status() -> Int32 {
        guard let code = failNextWith else { return 0 }
        failNextWith = nil
        return code
    }

    fileprivate func drain() -> NveEventBatch {
        drained = emitted
        emitted = []
        return drained.withUnsafeBufferPointer { NveEventBatch(ptr: $0.baseAddress, len: UInt32($0.count), _pad: 0) }
    }

    /// Fires the installed text measurer like runtime layout would.
    func measureText(_ s: String, size: Float, weight: UInt32, maxWidth: Float) -> (width: Float, height: Float)? {
        guard let fn = measurer.0 else { return nil }
        var s = s
        var w: Float = -1, h: Float = -1
        s.withUTF8 { fn(measurer.1, NveStrSlice(ptr: $0.baseAddress, len: $0.count), size, weight, maxWidth, &w, &h) }
        return (w, h)
    }

    static let api: UnsafePointer<NveRuntimeApi> = {
        var a = NveRuntimeApi()
        a.abi_version = NVE_RUNTIME_ABI_VERSION
        a.struct_size = UInt32(MemoryLayout<NveRuntimeApi>.size)
        a.schema_hash = NVE_RUNTIME_SCHEMA_HASH
        a.create = { _ in MockRuntime.shared.creates += 1; return MockRuntime.handle }
        a.destroy = { _ in MockRuntime.shared.destroys += 1 }
        a.set_wake_callback = { _, cb, ctx in
            let m = MockRuntime.shared
            let st = m.status()
            if st == 0 { m.wake = (cb, ctx) }
            return st
        }
        a.set_viewport = { _, w, h, s in MockRuntime.shared.viewports.append((w, h, s)); return 0 }
        a.attach_app_interface = { _, fn in
            if fn == nil { MockRuntime.shared.detaches += 1 } else { MockRuntime.shared.attaches += 1 }
            return MockRuntime.shared.status()
        }
        a.submit_nve2 = { _, _ in 0 }
        a.tick = { _, _, changed in
            let m = MockRuntime.shared
            m.ticks += 1
            changed?.pointee = m.tickChanged ? 1 : 0
            return m.status()
        }
        a.dispatch_events = { _, batch, changed in
            MockRuntime.shared.dispatched.append(NveDirectEventWriter.decodeBatch(batch))
            changed?.pointee = 1
            return 0
        }
        a.hit_test = { _, _, _, out in out?.pointee = MockRuntime.shared.hitResult; return 0 }
        a.scroll_by = { _, node, dx, dy, ox, oy in
            let m = MockRuntime.shared
            let cur = m.scroll[node] ?? (0, 0)
            let next = (max(0, cur.0 + dx), max(0, cur.1 + dy))
            m.scroll[node] = next
            ox?.pointee = next.0
            oy?.pointee = next.1
            return 0
        }
        a.scene_acquire = { _, out in
            let m = MockRuntime.shared
            m.acquires += 1
            out?.pointee = m.publish()
            return 0
        }
        a.scene_release = { _ in MockRuntime.shared.releases += 1; return 0 }
        a.get_metrics = { _, out in
            var metrics = NveRuntimeMetrics()
            metrics.frame_id = MockRuntime.shared.frameId
            out?.pointee = metrics
            return 0
        }
        a.pair_connect = { _, uri, _ in
            let m = MockRuntime.shared
            m.pairedUri = String(decoding: UnsafeBufferPointer(start: uri.ptr, count: uri.len), as: UTF8.self)
            m.paired = m.failNextWith == nil
            return m.status()
        }
        a.pair_disconnect = { _ in MockRuntime.shared.paired = false; return 0 }
        a.pair_is_connected = { _ in MockRuntime.shared.paired ? 1 : 0 }
        a.set_props = { _, _, count, changed in
            MockRuntime.shared.propCounts.append(count)
            changed?.pointee = 1
            return MockRuntime.shared.status()
        }
        a.drain_emitted_events = { _, out in out?.pointee = MockRuntime.shared.drain(); return 0 }
        a.apply_hot_patch = { _, patch, changed in
            MockRuntime.shared.patches.append(patch.len)
            changed?.pointee = 1
            return 0
        }
        a.snapshot_state = { _, out in
            let m = MockRuntime.shared
            out?.pointee = m.state.withUnsafeBufferPointer { NveBufferSlice(ptr: $0.baseAddress, len: $0.count) }
            return 0
        }
        a.restore_state = { _, s in
            MockRuntime.shared.restored = Array(UnsafeBufferPointer(start: s.ptr, count: s.len))
            return 0
        }
        a.set_text_measurer = { _, cb, ctx in
            let m = MockRuntime.shared
            let st = m.status()
            if st == 0 { m.measurer = (cb, ctx) }
            return st
        }
        a.resource_payload = { _, id, kind, out in
            let m = MockRuntime.shared
            guard m.acquires > m.releases else { return Int32(NVE_RUNTIME_INVALID_ARG.rawValue) }
            guard let bytes = m.resources[id] else { return Int32(NveRuntimeError.notFound) }
            kind?.pointee = 0x1234
            out?.pointee = NveBufferSlice(ptr: m.pin(bytes), len: bytes.count)
            return 0
        }
        a.last_error = { out, cap in
            let msg = Array(MockRuntime.shared.lastError.utf8)
            if let out, cap > 0 {
                let n = min(cap - 1, msg.count)
                msg.withUnsafeBufferPointer { out.update(from: $0.baseAddress!, count: n) }
                out[n] = 0
            }
            return msg.count
        }
        let p = UnsafeMutablePointer<NveRuntimeApi>.allocate(capacity: 1)
        p.initialize(to: a)
        return UnsafePointer(p)
    }()
}
