/// Validated zero-copy reader over a borrowed `NveSceneView` (`scene_acquire` /
/// `scene_release` of `libnve_runtime`).
///
/// Generic over the generated C structs (`extension NveSceneView: SceneViewABI {}`,
/// `extension NveNodeRow: NodeRowABI {}`, `extension NveLayoutRect: LayoutRectABI {}`), so this
/// zero-dependency package never hard-codes the record layouts. `open` validates, in
/// O(changed + tables) (never O(capacity)), the schema hash, every pointer/count pair (null,
/// address-space fit, alignment), the text slab's UTF-8, every dirty / moved index (`< capacity`),
/// the strict order of scroll offsets and dirty resources, and every published table (layout,
/// slot `0..<128`, unique slot) once, like the C++ / Dart / TS / Java readers; child spans are
/// checked when read. Later reads are O(1) (scroll lookups O(log n)) with no allocation.

/// Shape of the generated C `NveNodeRow` (32 bytes, Rust `nve_store::NodeRow`).
public protocol NodeRowABI {
    /// 128-bit attached-table bitset as little-endian `u32` words (`[u32; 4]` imports as a tuple).
    var table_set: (UInt32, UInt32, UInt32, UInt32) { get }
    var parent: UInt32 { get }
    var child_off: UInt32 { get }
    var child_len: UInt32 { get }
    var generation: UInt8 { get }
    var primary_slot: UInt8 { get }
    var effects: UInt8 { get }
    var flags: UInt8 { get }
}

/// Shape of the generated C `NveLayoutRect` (16 bytes, absolute surface coordinates).
public protocol LayoutRectABI {
    var x: Float { get }
    var y: Float { get }
    var width: Float { get }
    var height: Float { get }
}

/// Shape of the generated C `NveScrollOffset` (12 bytes): the visual child shift of one scroll container.
public protocol ScrollOffsetABI {
    var node_id: UInt32 { get }
    var x: Float { get }
    var y: Float { get }
}

/// Shape of the generated C `NveSceneView` (pointers implicitly unwrapped, as Swift imports C).
public protocol SceneViewABI {
    associatedtype Row: NodeRowABI
    associatedtype Rect: LayoutRectABI
    associatedtype Table: TableViewABI
    associatedtype Scroll: ScrollOffsetABI

    var schema_hash: UInt64 { get }
    var frame_id: UInt64 { get }
    var node_capacity: UInt32 { get }
    var dirty_count: UInt32 { get }
    var removed_count: UInt32 { get }
    var text_slab_len: UInt32 { get }
    var children_slab_len: UInt32 { get }
    var table_count: UInt32 { get }
    var scroll_count: UInt32 { get }
    var scene_epoch: UInt32 { get }
    var dirty_resource_count: UInt32 { get }
    var moved_count: UInt32 { get }
    var rows: UnsafePointer<Row>! { get }
    var rects: UnsafePointer<Rect>! { get }
    var text_slab: UnsafePointer<UInt8>! { get }
    var children_slab: UnsafePointer<UInt32>! { get }
    var dirty_indices: UnsafePointer<UInt32>! { get }
    var removed_ids: UnsafePointer<UInt32>! { get }
    var tables: UnsafePointer<Table>! { get }
    var scroll_offsets: UnsafePointer<Scroll>! { get }
    var dirty_resources: UnsafePointer<UInt32>! { get }
    var moved_indices: UnsafePointer<UInt32>! { get }
}

public enum SceneError: Error, Equatable, Sendable {
    case nullView
    case schemaMismatch(expected: Key64, actual: Key64)
    case nullSlab
    case misalignedSlab
    case invalidUtf8
    /// Table `index` is malformed, its slot is `>= 128`, or an earlier table took its slot.
    case invalidTable(index: Int)
    /// A dirty or moved index is `>= node_capacity`.
    case nodeIndexOutOfRange
    /// Scroll-offset node ids or dirty resource ids are not strictly ascending.
    case unsortedIds
}

/// Node identity (Rust `nve_store::NodeId`): `index: 24 | generation: 8`; raw `0` is the null node.
public struct NodeId: Hashable, Sendable, CustomStringConvertible {
    public static let indexBits: UInt32 = 24
    public static let maxIndex: UInt32 = (1 << indexBits) - 1
    public static let null = NodeId(raw: 0)
    /// Root of every surface (index 1, generation 0).
    public static let root = NodeId(raw: 1)

    public let raw: UInt32

    public init(raw: UInt32) { self.raw = raw }

    /// `index` is masked to 24 bits exactly like Rust `NodeId::new`.
    public init(index: UInt32, generation: UInt8) {
        raw = UInt32(generation) << Self.indexBits | (index & Self.maxIndex)
    }

    /// Slot index into `rows` / `rects` / dense table rows.
    public var index: Int { Int(raw & Self.maxIndex) }
    public var generation: UInt8 { UInt8(truncatingIfNeeded: raw >> Self.indexBits) }
    public var isNull: Bool { raw == 0 }
    public var description: String { "NodeId(\(index)v\(generation))" }
}

/// `NveNodeRow.flags` bit of an allocated live slot (`NVE_NODE_ROW_FLAG_LIVE`).
public let nodeRowFlagLive: UInt8 = 1 << 0
/// `NveNodeRow.primary_slot` of a plain container (no realizing table).
public let primarySlotNone: UInt8 = 0xFF

public extension NodeRowABI {
    var isLive: Bool { flags & nodeRowFlagLive != 0 }
    var parentId: NodeId { NodeId(raw: parent) }

    /// `true` when table `slot` (`0..<128`) is attached to this node.
    func hasTable(slot: UInt8) -> Bool {
        guard slot < 128 else { return false }
        let bit = UInt32(1) << UInt32(slot & 31)
        switch slot >> 5 {
        case 0: return table_set.0 & bit != 0
        case 1: return table_set.1 & bit != 0
        case 2: return table_set.2 & bit != 0
        default: return table_set.3 & bit != 0
        }
    }
}

public struct SceneReader<View: SceneViewABI> {
    public let frameId: UInt64
    /// Bumped on every runtime scene reset: node / resource ids and caches of an earlier epoch are stale.
    public let sceneEpoch: UInt32
    /// `node_capacity` rows; slot 0 is the reserved null slot, live slots carry `nodeRowFlagLive`.
    public let rows: UnsafeBufferPointer<View.Row>
    /// `node_capacity` absolute layout boxes indexed by slot.
    public let rects: UnsafeBufferPointer<View.Rect>
    public let textSlab: UnsafeBufferPointer<UInt8>
    /// Raw child `NodeId`s (`NveNodeRow.child_off ..< child_off + child_len`).
    public let childrenSlab: UnsafeBufferPointer<UInt32>
    /// Slot indices created, or whose row, tables or children changed on `frameId`.
    public let dirtyIndices: UnsafeBufferPointer<UInt32>
    /// Live slot indices not in `dirtyIndices` whose box relative to the parent's rect (origin,
    /// width, height) changed on `frameId`: re-apply only position / size. A moved node's
    /// descendants are not listed (their parent-relative boxes held; `rects` are still absolute).
    public let movedIndices: UnsafeBufferPointer<UInt32>
    /// Raw `NodeId`s removed on `frameId`.
    public let removedIds: UnsafeBufferPointer<UInt32>
    public let tables: UnsafeBufferPointer<View.Table>
    /// Visual child shift of every scroll container (rects are absolute and pre-scroll).
    public let scrollOffsets: UnsafeBufferPointer<View.Scroll>
    /// Live resource ids (re)defined on `frameId` (created, recycled, rewritten): caches keyed by
    /// resource id drop exactly these.
    public let dirtyResources: UnsafeBufferPointer<UInt32>

    /// Validates the view behind `pointer` (as filled by `scene_acquire`).
    public static func open(_ pointer: UnsafePointer<View>?, expectedSchema: Key64) throws(SceneError) -> Self {
        guard let pointer else { throw .nullView }
        return try open(pointer.pointee, expectedSchema: expectedSchema)
    }

    public static func open(_ v: View, expectedSchema: Key64) throws(SceneError) -> Self {
        guard v.schema_hash == expectedSchema.value else {
            throw .schemaMismatch(expected: expectedSchema, actual: Key64(v.schema_hash))
        }
        let rows = try buffer(v.rows, v.node_capacity)
        let rects = try buffer(v.rects, v.node_capacity)
        let text = try buffer(v.text_slab, v.text_slab_len)
        let children = try buffer(v.children_slab, v.children_slab_len)
        let dirty = try buffer(v.dirty_indices, v.dirty_count)
        let moved = try buffer(v.moved_indices, v.moved_count)
        let removed = try buffer(v.removed_ids, v.removed_count)
        let tables = try buffer(v.tables, v.table_count)
        let scrolls = try buffer(v.scroll_offsets, v.scroll_count)
        let dirtyResources = try buffer(v.dirty_resources, v.dirty_resource_count)
        guard Utf8.isValid(UnsafeRawBufferPointer(text)) else { throw .invalidUtf8 }
        let outOfRange = { (index: UInt32) in index >= v.node_capacity }
        guard !dirty.contains(where: outOfRange), !moved.contains(where: outOfRange) else { throw .nodeIndexOutOfRange }
        guard strictlyAscending(scrolls, \.node_id), strictlyAscending(dirtyResources, \.self) else { throw .unsortedIds }
        var seen: (lo: UInt64, hi: UInt64) = (0, 0)
        for (i, t) in tables.enumerated() {
            guard TableReader(view: t) != nil, t.slot < 128 else { throw .invalidTable(index: i) }
            let bit = UInt64(1) << UInt64(t.slot & 63)
            let taken = t.slot < 64 ? seen.lo & bit : seen.hi & bit
            guard taken == 0 else { throw .invalidTable(index: i) }
            if t.slot < 64 { seen.lo |= bit } else { seen.hi |= bit }
        }
        return Self(
            frameId: v.frame_id, sceneEpoch: v.scene_epoch, rows: rows, rects: rects, textSlab: text,
            childrenSlab: children, dirtyIndices: dirty, movedIndices: moved, removedIds: removed, tables: tables,
            scrollOffsets: scrolls, dirtyResources: dirtyResources)
    }

    /// Visual child shift of scroll container `id` (`(0, 0)` when it has none), O(log scroll
    /// containers): `open` checked the ids are strictly ascending.
    public func scrollOffset(for id: NodeId) -> (x: Float, y: Float) {
        var lo = 0
        var hi = scrollOffsets.count
        while lo < hi {
            let mid = (lo + hi) / 2
            let at = scrollOffsets[mid].node_id
            if at == id.raw { return (scrollOffsets[mid].x, scrollOffsets[mid].y) }
            if at < id.raw { lo = mid + 1 } else { hi = mid }
        }
        return (0, 0)
    }

    public var nodeCapacity: Int { rows.count }

    public func row(_ index: Int) -> View.Row? { rows.indices.contains(index) ? rows[index] : nil }
    public func rect(_ index: Int) -> View.Rect? { rects.indices.contains(index) ? rects[index] : nil }
    public func isLive(_ index: Int) -> Bool { row(index)?.isLive ?? false }

    /// Current `NodeId` of live slot `index`.
    public func nodeId(at index: Int) -> NodeId? {
        guard let r = row(index), r.isLive, let i = UInt32(exactly: index), i <= NodeId.maxIndex else { return nil }
        return NodeId(index: i, generation: r.generation)
    }

    /// Slot of `id` when it is live with a matching generation (stale ids resolve to `nil`).
    public func index(of id: NodeId) -> Int? {
        guard !id.isNull, let r = row(id.index), r.isLive, r.generation == id.generation else { return nil }
        return id.index
    }

    public func hasTable(_ index: Int, slot: UInt8) -> Bool { row(index)?.hasTable(slot: slot) ?? false }

    /// `textSlab[offset ..< offset + len]`; `nil` when out of range or not on UTF-8 boundaries.
    public func textBytes(offset: UInt32, len: UInt32) -> UnsafeBufferPointer<UInt8>? {
        guard let (start, end) = Self.range(offset, len, within: textSlab.count), isBoundary(start), isBoundary(end)
        else { return nil }
        return UnsafeBufferPointer(rebasing: textSlab[start..<end])
    }

    public func text(offset: UInt32, len: UInt32) -> String? {
        textBytes(offset: offset, len: len).map { String(decoding: $0, as: UTF8.self) }
    }

    /// Text of a `Str` field span (e.g. `nve.text.text`).
    public func text(_ span: StrSpan) -> String? { text(offset: span.offset, len: span.len) }

    /// `childrenSlab[offset ..< offset + count]`; `nil` when out of range.
    public func children(offset: UInt32, count: UInt32) -> UnsafeBufferPointer<UInt32>? {
        guard let (start, end) = Self.range(offset, count, within: childrenSlab.count) else { return nil }
        return UnsafeBufferPointer(rebasing: childrenSlab[start..<end])
    }

    /// Raw child ids of slot `index`; `nil` for an out-of-range slot or child span.
    public func children(of index: Int) -> UnsafeBufferPointer<UInt32>? {
        row(index).flatMap { children(offset: $0.child_off, count: $0.child_len) }
    }

    /// Visits `(index, row, rect)` for every in-range dirty slot in O(dirty).
    public func forEachDirty(_ visit: (Int, View.Row, View.Rect) throws -> Void) rethrows {
        for raw in dirtyIndices where UInt64(raw) < UInt64(rows.count) {
            try visit(Int(raw), rows[Int(raw)], rects[Int(raw)])
        }
    }

    public func table(_ index: Int) -> TableReader? {
        tables.indices.contains(index) ? TableReader(view: tables[index]) : nil
    }

    /// Published table with `key`, if any (O(tables)).
    public func table(key: Key64) -> TableReader? {
        tables.first { Key64(lo: $0.key_lo, hi: $0.key_hi) == key }.flatMap { TableReader(view: $0) }
    }

    /// Published table attached at process-local `slot` (e.g. a row's `primary_slot`), O(tables).
    public func table(slot: UInt8) -> TableReader? {
        tables.first { $0.slot == UInt32(slot) }.flatMap { TableReader(view: $0) }
    }

    /// Row of live node `index` in `table`, O(1), only when the node's `table_set` attaches `table`
    /// (unattached dense rows hold zeros, not defaults; sparse rows resolve via the paged row index).
    public func tableRow(_ table: TableReader, forNode index: Int) -> Int? {
        guard isLive(index), table.slot < 128, hasTable(index, slot: UInt8(table.slot)) else { return nil }
        return table.row(ofNode: index)
    }

    /// Per-table `EventSet<E>` listener bitmask attached to live node `index` (`0` if unattached).
    public func eventSet(_ table: TableReader, forNode index: Int, eventsWord: UInt8) -> UInt32 {
        guard let r = tableRow(table, forNode: index) else { return 0 }
        return table.eventSet(r, eventsWord: eventsWord)
    }

    /// `true` when live node `index` subscribes to `bit` in `table`'s `eventsWord`.
    public func hasEvent(_ table: TableReader, forNode index: Int, eventsWord: UInt8, bit: UInt32) -> Bool {
        (eventSet(table, forNode: index, eventsWord: eventsWord) & bit) != 0
    }

    /// Nearest row along `index`'s live ancestor chain (starting at `index`) that explicitly sets
    /// `field` in an `#[nve_table(inherited)]` table (e.g. `NveTables.Typography`). Non-inheriting
    /// fields check only `index`. Returns `nil` when no ancestor explicitly sets `field` (caller
    /// falls back to the table's generated `defaults`). Cycle-guarded and allocation-free.
    public func inheritedRow(_ table: TableReader, forNode index: Int, field: FieldSpec) -> Int? {
        if !field.inherits {
            guard let r = tableRow(table, forNode: index), table.isExplicit(r, field) else { return nil }
            return r
        }
        var current = index
        var depth = 0
        while isLive(current), depth < nodeCapacity {
            if let r = tableRow(table, forNode: current), table.isExplicit(r, field) {
                return r
            }
            guard let parent = row(current)?.parentId, let next = self.index(of: parent), next != current else {
                break
            }
            current = next
            depth += 1
        }
        return nil
    }

    /// Single-pass zero-allocation ancestor walk for an `#[nve_table(inherited)]` table starting at
    /// `index`. Calls `visit(row, freshExplicitMask)` for each ancestor carrying a row with at
    /// least one previously unresolved explicit bit (word 0), so a host resolves all inherited
    /// fields (such as `NveTables.Typography`) in one traversal.
    public func forEachInheritedRow(
        _ table: TableReader,
        fromNode index: Int,
        _ visit: (_ row: Int, _ freshExplicitMask: UInt32) throws -> Void
    ) rethrows {
        var resolved: UInt32 = 0
        var current = index
        var depth = 0
        while isLive(current), depth < nodeCapacity {
            if let r = tableRow(table, forNode: current) {
                let mask = table.explicitMask(r)
                let fresh = mask & ~resolved
                if fresh != 0 {
                    try visit(r, fresh)
                    resolved |= mask
                }
            }
            guard let parent = row(current)?.parentId, let next = self.index(of: parent), next != current else {
                break
            }
            current = next
            depth += 1
        }
    }

    /// `offset ..< offset + len` when it lies inside `0 ..< size`, computed in `UInt64` so no
    /// wire value can trap on 32-bit hosts.
    private static func range(_ offset: UInt32, _ len: UInt32, within size: Int) -> (Int, Int)? {
        let end = UInt64(offset) + UInt64(len)
        guard end <= UInt64(size) else { return nil }
        return (Int(offset), Int(end))
    }

    private func isBoundary(_ at: Int) -> Bool { at == textSlab.count || textSlab[at] & 0xC0 != 0x80 }

    private static func strictlyAscending<T>(_ items: UnsafeBufferPointer<T>, _ id: KeyPath<T, UInt32>) -> Bool {
        var previous: UInt32?
        for item in items {
            let next = item[keyPath: id]
            if let previous, next <= previous { return false }
            previous = next
        }
        return true
    }

    private static func buffer<T>(_ p: UnsafePointer<T>?, _ count: UInt32) throws(SceneError) -> UnsafeBufferPointer<T> {
        if count == 0 { return UnsafeBufferPointer(start: nil, count: 0) }
        guard let p, let n = TableReader.fitsAddressSpace(UInt64(count), stride: MemoryLayout<T>.stride)
        else { throw .nullSlab }
        guard UInt(bitPattern: p) % UInt(MemoryLayout<T>.alignment) == 0 else { throw .misalignedSlab }
        return UnsafeBufferPointer(start: p, count: n)
    }
}
