import CNveAbi
import Foundation

// MARK: - Zero-Copy Direct `NveSceneReader` Hit-Testing

public enum NveSceneHitTester {
    /// Reverse z-order hit-test directly over a zero-copy ``NveSceneReader``.
    /// Pass `indexById` from ``NveSceneStore/nodeIndexById`` to avoid an `O(N)` rebuild.
    public static func hitTest(
        reader: NveSceneReader,
        at point: CGPoint,
        scrollOffsets: [UInt32: CGPoint] = [:],
        interactiveOnly: Bool = true,
        indexById: [UInt32: Int]? = nil
    ) -> UInt32? {
        guard reader.count > 0 else { return nil }
        let lookup = indexById ?? buildIndexLookup(reader)
        let rootIdx = lookup[UInt32(NVE_ROOT_NODE_ID)] ?? 0
        return hitTestRecursive(
            reader: reader,
            indexById: lookup,
            nodeIndex: rootIdx,
            point: point,
            scrollOffsets: scrollOffsets,
            interactiveOnly: interactiveOnly
        )
    }

    /// Finds the innermost `NVE_NODE_KIND_SCROLL_VIEW` node containing `point`.
    public static func hitTestScrollable(
        reader: NveSceneReader,
        at point: CGPoint,
        scrollOffsets: [UInt32: CGPoint] = [:],
        indexById: [UInt32: Int]? = nil
    ) -> UInt32? {
        guard reader.count > 0 else { return nil }
        let lookup = indexById ?? buildIndexLookup(reader)
        let rootIdx = lookup[UInt32(NVE_ROOT_NODE_ID)] ?? 0
        return hitTestScrollableRecursive(
            reader: reader,
            indexById: lookup,
            nodeIndex: rootIdx,
            point: point,
            scrollOffsets: scrollOffsets
        )
    }

    static func buildIndexLookup(_ reader: NveSceneReader) -> [UInt32: Int] {
        var map: [UInt32: Int] = [:]
        map.reserveCapacity(reader.count)
        for i in 0 ..< reader.count {
            map[reader[i].node_id] = i
        }
        return map
    }

    static func orderedChildIndices(
        reader: NveSceneReader,
        indexById: [UInt32: Int],
        childIds: UnsafeBufferPointer<UInt32>
    ) -> [Int] {
        var indices: [Int] = []
        indices.reserveCapacity(childIds.count)
        var hasNonZeroZ = false
        for cid in childIds {
            if let idx = indexById[cid] {
                indices.append(idx)
                if reader[idx].z_index != 0 {
                    hasNonZeroZ = true
                }
            }
        }
        if !hasNonZeroZ || indices.count <= 1 {
            return indices
        }
        for i in 1 ..< indices.count {
            let key = indices[i]
            let keyZ = reader[key].z_index
            var j = i - 1
            while j >= 0 && reader[indices[j]].z_index > keyZ {
                indices[j + 1] = indices[j]
                j -= 1
            }
            indices[j + 1] = key
        }
        return indices
    }

    private static func hitTestRecursive(
        reader: NveSceneReader,
        indexById: [UInt32: Int],
        nodeIndex: Int,
        point: CGPoint,
        scrollOffsets: [UInt32: CGPoint],
        interactiveOnly: Bool
    ) -> UInt32? {
        let node = reader[nodeIndex]
        if node.opacity_u8 == 0 { return nil }
        let rect = CGRect(
            x: CGFloat(node.x),
            y: CGFloat(node.y),
            width: CGFloat(node.width),
            height: CGFloat(node.height)
        )
        let inside = rect.contains(point)
        let clips = node.kind == UInt8(NVE_NODE_KIND_SCROLL_VIEW) || NveSceneReader.clipsBounds(node)
        if !inside && (clips || node.node_id != UInt32(NVE_ROOT_NODE_ID)) {
            return nil
        }

        let childPoint: CGPoint
        if node.kind == UInt8(NVE_NODE_KIND_SCROLL_VIEW), let offset = scrollOffsets[node.node_id] {
            childPoint = CGPoint(x: point.x + offset.x, y: point.y + offset.y)
        } else {
            childPoint = point
        }

        if node.child_count > 0, let childIds = try? reader.children(for: node) {
            let children = orderedChildIndices(reader: reader, indexById: indexById, childIds: childIds)
            for childIdx in children.reversed() {
                if let hit = hitTestRecursive(
                    reader: reader,
                    indexById: indexById,
                    nodeIndex: childIdx,
                    point: childPoint,
                    scrollOffsets: scrollOffsets,
                    interactiveOnly: interactiveOnly
                ) {
                    return hit
                }
            }
        }

        if inside && (!interactiveOnly || NveSceneReader.isInteractive(node)) {
            return node.node_id
        }
        return nil
    }

    private static func hitTestScrollableRecursive(
        reader: NveSceneReader,
        indexById: [UInt32: Int],
        nodeIndex: Int,
        point: CGPoint,
        scrollOffsets: [UInt32: CGPoint]
    ) -> UInt32? {
        let node = reader[nodeIndex]
        if node.opacity_u8 == 0 { return nil }
        let rect = CGRect(
            x: CGFloat(node.x),
            y: CGFloat(node.y),
            width: CGFloat(node.width),
            height: CGFloat(node.height)
        )
        let inside = rect.contains(point)
        if !inside && node.node_id != UInt32(NVE_ROOT_NODE_ID) {
            return nil
        }

        let childPoint: CGPoint
        if node.kind == UInt8(NVE_NODE_KIND_SCROLL_VIEW), let offset = scrollOffsets[node.node_id] {
            childPoint = CGPoint(x: point.x + offset.x, y: point.y + offset.y)
        } else {
            childPoint = point
        }

        if node.child_count > 0, let childIds = try? reader.children(for: node) {
            let children = orderedChildIndices(reader: reader, indexById: indexById, childIds: childIds)
            for childIdx in children.reversed() {
                if let hit = hitTestScrollableRecursive(
                    reader: reader,
                    indexById: indexById,
                    nodeIndex: childIdx,
                    point: childPoint,
                    scrollOffsets: scrollOffsets
                ) {
                    return hit
                }
            }
        }

        if inside && node.kind == UInt8(NVE_NODE_KIND_SCROLL_VIEW) {
            return node.node_id
        }
        return nil
    }
}
