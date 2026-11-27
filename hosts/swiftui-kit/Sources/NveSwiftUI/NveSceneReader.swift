import CNveAbi
import Foundation

/// Validation and bounds errors thrown by ``NveSceneReader``.
public enum NveSceneReaderError: Error, Equatable, Sendable {
    case schemaHashMismatch(expected: UInt64, actual: UInt64)
    case nullBufferPointer(String)
    case textOutOfBounds(nodeId: UInt32, offset: UInt32, length: UInt16, slabLength: Int)
    case invalidUtf8(nodeId: UInt32)
    case childrenOutOfBounds(nodeId: UInt32, offset: UInt32, count: UInt16, slabLength: Int)
}

/// Zero-copy, zero-decode Swift reader over a C ABI ``NveSceneView``.
public struct NveSceneReader {
    public let frameId: UInt64
    public let nodes: UnsafeBufferPointer<NveRenderNode>
    public let textSlab: UnsafeBufferPointer<UInt8>
    public let childrenSlab: UnsafeBufferPointer<UInt32>
    public let dirtyIndices: UnsafeBufferPointer<UInt32>
    public let removedIds: UnsafeBufferPointer<UInt32>
    public let tables: UnsafeBufferPointer<NveTableView>

    /// Validates `schema_hash` and non-null pointer invariants without copying any slab data.
    public init(view: NveSceneView) throws {
        let expected = UInt64(NVE_SCENE_SCHEMA_HASH)
        guard view.schema_hash == expected else {
            throw NveSceneReaderError.schemaHashMismatch(
                expected: expected,
                actual: view.schema_hash
            )
        }

        let nodeCount = Int(view.node_count)
        if nodeCount > 0 && view.nodes == nil {
            throw NveSceneReaderError.nullBufferPointer("nodes")
        }
        let textLen = Int(view.text_slab_len)
        if textLen > 0 && view.text_slab == nil {
            throw NveSceneReaderError.nullBufferPointer("text_slab")
        }
        let childrenLen = Int(view.children_slab_len)
        if childrenLen > 0 && view.children_slab == nil {
            throw NveSceneReaderError.nullBufferPointer("children_slab")
        }
        let dirtyCount = Int(view.dirty_count)
        if dirtyCount > 0 && view.dirty_indices == nil {
            throw NveSceneReaderError.nullBufferPointer("dirty_indices")
        }
        let removedCount = Int(view.removed_count)
        if removedCount > 0 && view.removed_ids == nil {
            throw NveSceneReaderError.nullBufferPointer("removed_ids")
        }
        let tableCount = Int(view.table_count)
        if tableCount > 0 && view.tables == nil {
            throw NveSceneReaderError.nullBufferPointer("tables")
        }

        self.frameId = view.frame_id
        self.nodes = UnsafeBufferPointer(start: view.nodes, count: nodeCount)
        self.textSlab = UnsafeBufferPointer(start: view.text_slab, count: textLen)
        self.childrenSlab = UnsafeBufferPointer(start: view.children_slab, count: childrenLen)
        self.dirtyIndices = UnsafeBufferPointer(start: view.dirty_indices, count: dirtyCount)
        self.removedIds = UnsafeBufferPointer(start: view.removed_ids, count: removedCount)
        self.tables = UnsafeBufferPointer(start: view.tables, count: tableCount)
    }

    @inlinable
    public var count: Int { nodes.count }

    @inlinable
    public var dirtyCount: Int { dirtyIndices.count }

    @inlinable
    public var removedCount: Int { removedIds.count }

    @inlinable
    public subscript(index: Int) -> NveRenderNode {
        nodes[index]
    }

    /// Visits only dirty nodes in `O(dirtyCount)` with zero allocations.
    @inlinable
    public func forEachDirty(_ body: (Int, NveRenderNode) throws -> Void) rethrows {
        for rawIdx in dirtyIndices {
            let idx = Int(rawIdx)
            if idx < nodes.count {
                try body(idx, nodes[idx])
            }
        }
    }

    /// Returns a zero-copy UTF-8 byte slice for `node` inside `textSlab`.
    public func textBytes(for node: NveRenderNode) throws -> UnsafeBufferPointer<UInt8> {
        let len = Int(node.text_len)
        if len == 0 {
            return UnsafeBufferPointer(start: nil, count: 0)
        }
        let start = Int(node.text_offset)
        let (end, overflow) = start.addingReportingOverflow(len)
        guard !overflow, start >= 0, end <= textSlab.count, let base = textSlab.baseAddress else {
            throw NveSceneReaderError.textOutOfBounds(
                nodeId: node.node_id,
                offset: node.text_offset,
                length: node.text_len,
                slabLength: textSlab.count
            )
        }
        return UnsafeBufferPointer(start: base.advanced(by: start), count: len)
    }

    /// Decodes `node`'s UTF-8 text from `textSlab`, throwing on invalid UTF-8 or out-of-bounds slices.
    public func text(for node: NveRenderNode) throws -> String {
        let slice = try textBytes(for: node)
        if slice.isEmpty {
            return ""
        }
        guard let decoded = String(bytes: slice, encoding: .utf8) else {
            throw NveSceneReaderError.invalidUtf8(nodeId: node.node_id)
        }
        return decoded
    }

    /// Returns a zero-copy child ID slice for `node` inside `childrenSlab`.
    public func children(for node: NveRenderNode) throws -> UnsafeBufferPointer<UInt32> {
        let count = Int(node.child_count)
        if count == 0 {
            return UnsafeBufferPointer(start: nil, count: 0)
        }
        let start = Int(node.child_offset)
        let (end, overflow) = start.addingReportingOverflow(count)
        guard !overflow, start >= 0, end <= childrenSlab.count, let base = childrenSlab.baseAddress else {
            throw NveSceneReaderError.childrenOutOfBounds(
                nodeId: node.node_id,
                offset: node.child_offset,
                count: node.child_count,
                slabLength: childrenSlab.count
            )
        }
        return UnsafeBufferPointer(start: base.advanced(by: start), count: count)
    }

    @inlinable
    public static func opacity(for node: NveRenderNode) -> Float {
        Float(node.opacity_u8) / 255.0
    }

    @inlinable
    public static func isLeafText(_ node: NveRenderNode) -> Bool {
        (node.flags & UInt8(NVE_NODE_LEAF_TEXT)) != 0
    }

    @inlinable
    public static func isInteractive(_ node: NveRenderNode) -> Bool {
        (node.flags & UInt8(NVE_NODE_INTERACTIVE)) != 0
    }

    @inlinable
    public static func clipsBounds(_ node: NveRenderNode) -> Bool {
        (node.flags & UInt8(NVE_NODE_CLIP_BOUNDS)) != 0
    }
}
