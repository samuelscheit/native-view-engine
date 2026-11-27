/// Validated zero-copy reader over a published `NveSceneView`.
///
/// Generic over the generated C struct (`extension NveSceneView: SceneViewABI {}`), so this static
/// runtime never hard-codes the node record layout. `open` validates the schema hash, every
/// pointer/count pair, and the text slab's UTF-8 once; later reads are O(1) bounds checks.

/// Shape of the generated C `NveSceneView` (pointers implicitly unwrapped, as Swift imports C).
public protocol SceneViewABI {
    associatedtype Node
    associatedtype Table: TableViewABI

    var schema_hash: UInt64 { get }
    var frame_id: UInt64 { get }
    var node_count: UInt32 { get }
    var dirty_count: UInt32 { get }
    var removed_count: UInt32 { get }
    var text_slab_len: UInt32 { get }
    var children_slab_len: UInt32 { get }
    var table_count: UInt32 { get }
    var nodes: UnsafePointer<Node>! { get }
    var text_slab: UnsafePointer<UInt8>! { get }
    var children_slab: UnsafePointer<UInt32>! { get }
    var dirty_indices: UnsafePointer<UInt32>! { get }
    var removed_ids: UnsafePointer<UInt32>! { get }
    var tables: UnsafePointer<Table>! { get }
}

public enum SceneError: Error, Equatable, Sendable {
    case nullView
    case schemaMismatch(expected: Key64, actual: Key64)
    case nullSlab
    case invalidUtf8
    case invalidTable(index: Int)
}

public struct SceneReader<View: SceneViewABI> {
    public let frameId: UInt64
    public let nodes: UnsafeBufferPointer<View.Node>
    public let textSlab: UnsafeBufferPointer<UInt8>
    public let childrenSlab: UnsafeBufferPointer<UInt32>
    public let dirtyIndices: UnsafeBufferPointer<UInt32>
    public let removedIds: UnsafeBufferPointer<UInt32>
    public let tables: UnsafeBufferPointer<View.Table>

    /// Validates the view behind `pointer` (as returned by the C ABI).
    public static func open(_ pointer: UnsafePointer<View>?, expectedSchema: Key64) throws(SceneError) -> Self {
        guard let pointer else { throw .nullView }
        return try open(pointer.pointee, expectedSchema: expectedSchema)
    }

    public static func open(_ v: View, expectedSchema: Key64) throws(SceneError) -> Self {
        guard v.schema_hash == expectedSchema.value else {
            throw .schemaMismatch(expected: expectedSchema, actual: Key64(v.schema_hash))
        }
        guard let nodes = buffer(v.nodes, v.node_count), let text = buffer(v.text_slab, v.text_slab_len),
              let children = buffer(v.children_slab, v.children_slab_len),
              let dirty = buffer(v.dirty_indices, v.dirty_count), let removed = buffer(v.removed_ids, v.removed_count),
              let tables = buffer(v.tables, v.table_count)
        else { throw .nullSlab }
        guard Utf8.isValid(text) else { throw .invalidUtf8 }
        for (i, t) in tables.enumerated() where TableReader(view: t) == nil {
            throw .invalidTable(index: i)
        }
        return Self(
            frameId: v.frame_id, nodes: nodes, textSlab: text, childrenSlab: children, dirtyIndices: dirty,
            removedIds: removed, tables: tables)
    }

    public var nodeCount: Int { nodes.count }

    public func node(_ index: Int) -> View.Node? { nodes.indices.contains(index) ? nodes[index] : nil }

    /// `textSlab[offset ..< offset + len]`; `nil` when out of range or not on UTF-8 boundaries.
    public func textBytes(offset: UInt32, len: UInt32) -> UnsafeBufferPointer<UInt8>? {
        guard let (start, end) = Self.range(offset, len, within: textSlab.count), isBoundary(start), isBoundary(end)
        else { return nil }
        return UnsafeBufferPointer(rebasing: textSlab[start..<end])
    }

    public func text(offset: UInt32, len: UInt32) -> String? {
        textBytes(offset: offset, len: len).map { String(decoding: $0, as: UTF8.self) }
    }

    /// `childrenSlab[offset ..< offset + count]`; `nil` when out of range.
    public func children(offset: UInt32, count: UInt32) -> UnsafeBufferPointer<UInt32>? {
        guard let (start, end) = Self.range(offset, count, within: childrenSlab.count) else { return nil }
        return UnsafeBufferPointer(rebasing: childrenSlab[start..<end])
    }

    /// `offset ..< offset + len` when it lies inside `0 ..< size`, computed in `UInt64` so no
    /// wire value can trap on 32-bit hosts.
    private static func range(_ offset: UInt32, _ len: UInt32, within size: Int) -> (Int, Int)? {
        let end = UInt64(offset) + UInt64(len)
        guard end <= UInt64(size) else { return nil }
        return (Int(offset), Int(end))
    }

    /// Visits `(index, node)` for every in-range dirty index in O(dirty).
    public func forEachDirty(_ visit: (Int, View.Node) throws -> Void) rethrows {
        for raw in dirtyIndices where UInt64(raw) < UInt64(nodes.count) {
            try visit(Int(raw), nodes[Int(raw)])
        }
    }

    public func table(_ index: Int) -> TableReader? {
        tables.indices.contains(index) ? TableReader(view: tables[index]) : nil
    }

    /// Published table with `key`, if any (O(tables)).
    public func table(key: Key64) -> TableReader? {
        tables.first { Key64(lo: $0.key_lo, hi: $0.key_hi) == key }.flatMap { TableReader(view: $0) }
    }

    private func isBoundary(_ at: Int) -> Bool { at == textSlab.count || textSlab[at] & 0xC0 != 0x80 }

    private static func buffer<T>(_ p: UnsafePointer<T>?, _ count: UInt32) -> UnsafeBufferPointer<T>? {
        if count == 0 { return UnsafeBufferPointer(start: nil, count: 0) }
        guard let p, let n = Int(exactly: count), n <= Int.max / MemoryLayout<T>.stride else { return nil }
        return UnsafeBufferPointer(start: p, count: n)
    }
}
