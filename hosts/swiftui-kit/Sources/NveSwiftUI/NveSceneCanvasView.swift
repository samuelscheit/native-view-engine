import CNveAbi
import Foundation
import SwiftUI

// MARK: - Scene Store (Flat 64-byte `NveRenderNode` Slab + Incremental Dirty Updates)

/// Retains a flat contiguous array of 64-byte `NveRenderNode` C structs plus `textSlab` and
/// `childrenSlab` with zero per-node dictionaries, applying `NveSceneView` updates in
/// `O(dirtyCount + removedCount)`.
public final class NveSceneStore: ObservableObject {
    @Published public private(set) var frameId: UInt64 = 0
    public private(set) var nodes: [NveRenderNode] = []
    public private(set) var textSlab: [UInt8] = []
    public private(set) var childrenSlab: [UInt32] = []
    public private(set) var lastDirtyCount: Int = 0
    public private(set) var lastRemovedCount: Int = 0
    public private(set) var scrollOffsets: [UInt32: CGPoint] = [:]

    private var idToIndex: [UInt32: Int] = [:]

    public init() {}

    public var nodeCount: Int { nodes.count }

    /// Applies a validated ``NveSceneReader`` incrementally using `removedIds` and `dirtyIndices`.
    public func apply(reader: NveSceneReader) throws {
        let isInitialOrResized = nodes.isEmpty || nodes.count != reader.count

        if isInitialOrResized {
            nodes = Array(reader.nodes)
            textSlab = Array(reader.textSlab)
            childrenSlab = Array(reader.childrenSlab)
            idToIndex.removeAll(keepingCapacity: true)
            idToIndex.reserveCapacity(nodes.count)
            for idx in 0 ..< nodes.count {
                idToIndex[nodes[idx].node_id] = idx
            }
            for removedId in reader.removedIds {
                scrollOffsets.removeValue(forKey: removedId)
            }
            lastDirtyCount = reader.dirtyCount > 0 ? reader.dirtyCount : nodes.count
            lastRemovedCount = reader.removedCount
        } else {
            for removedId in reader.removedIds {
                idToIndex.removeValue(forKey: removedId)
                scrollOffsets.removeValue(forKey: removedId)
            }
            if reader.dirtyCount > 0 {
                reader.forEachDirty { idx, node in
                    let prevId = nodes[idx].node_id
                    if prevId != node.node_id && idToIndex[prevId] == idx {
                        idToIndex.removeValue(forKey: prevId)
                    }
                    nodes[idx] = node
                    idToIndex[node.node_id] = idx
                }
                if textSlab.count != reader.textSlab.count || !reader.textSlab.elementsEqual(textSlab) {
                    textSlab = Array(reader.textSlab)
                }
                if childrenSlab.count != reader.childrenSlab.count
                    || !reader.childrenSlab.elementsEqual(childrenSlab)
                {
                    childrenSlab = Array(reader.childrenSlab)
                }
            }
            lastDirtyCount = reader.dirtyCount
            lastRemovedCount = reader.removedCount
        }

        frameId = reader.frameId
    }

    /// Convenience wrapper validating `view` via ``NveSceneReader`` before applying.
    public func apply(view: NveSceneView) throws {
        let reader = try NveSceneReader(view: view)
        try apply(reader: reader)
    }

    public func node(forId nodeId: UInt32) -> NveRenderNode? {
        guard let idx = idToIndex[nodeId], idx < nodes.count else { return nil }
        return nodes[idx]
    }

    public func scrollOffset(for nodeId: UInt32) -> CGPoint {
        scrollOffsets[nodeId] ?? .zero
    }

    /// Updates scroll offset for a `ScrollView` node clamped to its children's content bounds.
    @discardableResult
    public func scrollBy(
        nodeId: UInt32,
        deltaX: CGFloat,
        deltaY: CGFloat
    ) -> (offset: CGPoint, contentSize: CGSize)? {
        guard let node = node(forId: nodeId),
              node.kind == UInt8(NVE_NODE_KIND_SCROLL_VIEW)
        else {
            return nil
        }
        let contentSize = computeContentSize(for: node)
        let maxX = max(0, contentSize.width - CGFloat(node.width))
        let maxY = max(0, contentSize.height - CGFloat(node.height))
        let current = scrollOffsets[nodeId] ?? .zero
        let next = CGPoint(
            x: min(max(0, current.x + deltaX), maxX),
            y: min(max(0, current.y + deltaY), maxY)
        )
        scrollOffsets[nodeId] = next
        objectWillChange.send()
        return (next, contentSize)
    }

    /// Performs reverse z-order hit-testing directly over the flat `NveRenderNode` slab.
    public func hitTest(at point: CGPoint, interactiveOnly: Bool = true) -> UInt32? {
        withReader { reader -> UInt32? in
            NveSceneHitTester.hitTest(
                reader: reader,
                at: point,
                scrollOffsets: scrollOffsets,
                interactiveOnly: interactiveOnly
            )
        } ?? nil
    }

    /// Finds the innermost `ScrollView` node containing `point`.
    public func hitTestScrollable(at point: CGPoint) -> UInt32? {
        withReader { reader -> UInt32? in
            NveSceneHitTester.hitTestScrollable(
                reader: reader,
                at: point,
                scrollOffsets: scrollOffsets
            )
        } ?? nil
    }

    /// Scopes a zero-copy ``NveSceneReader`` over the retained flat slabs.
    public func withReader<R>(_ body: (NveSceneReader) throws -> R) rethrows -> R? {
        guard !nodes.isEmpty else { return nil }
        return try nodes.withUnsafeBufferPointer { nodesBuf in
            try textSlab.withUnsafeBufferPointer { textBuf in
                try childrenSlab.withUnsafeBufferPointer { childBuf in
                    let view = NveSceneView(
                        schema_hash: UInt64(NVE_SCENE_SCHEMA_HASH),
                        frame_id: frameId,
                        node_count: UInt32(nodesBuf.count),
                        dirty_count: 0,
                        removed_count: 0,
                        text_slab_len: UInt32(textBuf.count),
                        children_slab_len: UInt32(childBuf.count),
                        _pad: 0,
                        nodes: nodesBuf.baseAddress,
                        text_slab: textBuf.baseAddress,
                        children_slab: childBuf.baseAddress,
                        dirty_indices: nil,
                        removed_ids: nil
                    )
                    guard let reader = try? NveSceneReader(view: view) else {
                        return nil
                    }
                    return try body(reader)
                }
            }
        }
    }

    private func computeContentSize(for node: NveRenderNode) -> CGSize {
        var maxW = CGFloat(node.width)
        var maxH = CGFloat(node.height)
        let start = Int(node.child_offset)
        let end = start + Int(node.child_count)
        if start >= 0 && end <= childrenSlab.count {
            for i in start ..< end {
                let childId = childrenSlab[i]
                if let child = self.node(forId: childId) {
                    let right = CGFloat(child.x - node.x + child.width)
                    let bottom = CGFloat(child.y - node.y + child.height)
                    if right > maxW { maxW = right }
                    if bottom > maxH { maxH = bottom }
                }
            }
        }
        return CGSize(width: maxW, height: maxH)
    }
}

// MARK: - Zero-Copy Direct `NveSceneReader` Hit-Testing

public enum NveSceneHitTester {
    /// Reverse z-order hit-test directly over a zero-copy ``NveSceneReader``.
    public static func hitTest(
        reader: NveSceneReader,
        at point: CGPoint,
        scrollOffsets: [UInt32: CGPoint] = [:],
        interactiveOnly: Bool = true
    ) -> UInt32? {
        guard reader.count > 0 else { return nil }
        let indexById = buildIndexLookup(reader)
        let rootIdx = indexById[UInt32(NVE_ROOT_NODE_ID)] ?? 0
        return hitTestRecursive(
            reader: reader,
            indexById: indexById,
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
        scrollOffsets: [UInt32: CGPoint] = [:]
    ) -> UInt32? {
        guard reader.count > 0 else { return nil }
        let indexById = buildIndexLookup(reader)
        let rootIdx = indexById[UInt32(NVE_ROOT_NODE_ID)] ?? 0
        return hitTestScrollableRecursive(
            reader: reader,
            indexById: indexById,
            nodeIndex: rootIdx,
            point: point,
            scrollOffsets: scrollOffsets
        )
    }

    private static func buildIndexLookup(_ reader: NveSceneReader) -> [UInt32: Int] {
        var map: [UInt32: Int] = [:]
        map.reserveCapacity(reader.count)
        for i in 0 ..< reader.count {
            map[reader[i].node_id] = i
        }
        return map
    }

    private static func orderedChildIndices(
        reader: NveSceneReader,
        indexById: [UInt32: Int],
        node: NveRenderNode
    ) -> [Int] {
        guard node.child_count > 0, let childIds = try? reader.children(for: node) else {
            return []
        }
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
        // Stable insertion sort by ascending z_index (preserves tree order on ties).
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
        if node.opacity_u8 == 0 {
            return nil
        }
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

        let children = orderedChildIndices(reader: reader, indexById: indexById, node: node)
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
        if node.opacity_u8 == 0 {
            return nil
        }
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

        let children = orderedChildIndices(reader: reader, indexById: indexById, node: node)
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

        if inside && node.kind == UInt8(NVE_NODE_KIND_SCROLL_VIEW) {
            return node.node_id
        }
        return nil
    }
}

// MARK: - Zero-Copy Direct `NveSceneReader` SwiftUI Canvas Renderer

public enum NveSceneRenderer {
    /// Decodes a little-endian wire `ColorRgba` (`[r, g, b, a]`) into a SwiftUI `Color`.
    @inlinable
    public static func color(fromRgba rgba: UInt32) -> Color? {
        let a = (rgba >> 24) & 0xFF
        guard a > 0 else { return nil }
        let r = Double(rgba & 0xFF) / 255.0
        let g = Double((rgba >> 8) & 0xFF) / 255.0
        let b = Double((rgba >> 16) & 0xFF) / 255.0
        return Color(.sRGB, red: r, green: g, blue: b, opacity: Double(a) / 255.0)
    }

    /// Renders a flat 64-byte `NveRenderNode` tree directly from an ``NveSceneReader`` onto a
    /// SwiftUI `GraphicsContext` (rounded rects, borders, opacity, leaf text, scroll/bounds clip).
    public static func draw(
        reader: NveSceneReader,
        into context: inout GraphicsContext,
        size: CGSize,
        scrollOffsets: [UInt32: CGPoint] = [:]
    ) {
        guard reader.count > 0 else { return }
        var indexById: [UInt32: Int] = [:]
        indexById.reserveCapacity(reader.count)
        for i in 0 ..< reader.count {
            indexById[reader[i].node_id] = i
        }
        let rootIdx = indexById[UInt32(NVE_ROOT_NODE_ID)] ?? 0
        drawNode(
            reader: reader,
            indexById: indexById,
            nodeIndex: rootIdx,
            into: &context,
            scrollOffsets: scrollOffsets
        )
    }

    public static func draw(
        store: NveSceneStore,
        into context: inout GraphicsContext,
        size: CGSize
    ) {
        _ = store.withReader { reader in
            draw(reader: reader, into: &context, size: size, scrollOffsets: store.scrollOffsets)
        }
    }

    private static func drawNode(
        reader: NveSceneReader,
        indexById: [UInt32: Int],
        nodeIndex: Int,
        into context: inout GraphicsContext,
        scrollOffsets: [UInt32: CGPoint]
    ) {
        let node = reader[nodeIndex]
        guard node.opacity_u8 > 0 else { return }

        var nodeCtx = context
        if node.opacity_u8 < 255 {
            nodeCtx.opacity *= Double(node.opacity_u8) / 255.0
        }

        let rect = CGRect(
            x: CGFloat(node.x),
            y: CGFloat(node.y),
            width: CGFloat(node.width),
            height: CGFloat(node.height)
        )
        let hasArea = rect.width > 0 && rect.height > 0
        let radius = max(0, CGFloat(node.border_radius))

        // 1. Background fill
        if hasArea, let bgColor = color(fromRgba: node.bg_rgba) {
            let path = radius > 0
                ? Path(roundedRect: rect, cornerRadius: radius)
                : Path(rect)
            nodeCtx.fill(path, with: .color(bgColor))
        }

        // 2. Border stroke
        if hasArea, node.border_width > 0, let borderColor = color(fromRgba: node.border_rgba) {
            let bw = CGFloat(node.border_width)
            let inset = bw * 0.5
            let strokeRect = (rect.width > bw && rect.height > bw)
                ? rect.insetBy(dx: inset, dy: inset)
                : rect
            let strokePath = radius > 0
                ? Path(roundedRect: strokeRect, cornerRadius: max(0, radius - inset))
                : Path(strokeRect)
            nodeCtx.stroke(strokePath, with: .color(borderColor), lineWidth: bw)
        }

        // 3. Leaf text glyphs
        if hasArea,
           NveSceneReader.isLeafText(node),
           node.text_len > 0,
           let textColor = color(fromRgba: node.text_rgba),
           let textStr = try? reader.text(for: node),
           !textStr.isEmpty
        {
            let fontSize = (node.font_size.isFinite && node.font_size > 0)
                ? CGFloat(node.font_size)
                : 16.0
            let resolved = nodeCtx.resolve(
                Text(textStr)
                    .font(.system(size: fontSize))
                    .foregroundColor(textColor)
            )
            nodeCtx.draw(resolved, in: rect)
        }

        // 4. Children (with clip & scroll offset translation for ScrollView / CLIP_BOUNDS)
        guard node.child_count > 0, let childIds = try? reader.children(for: node) else {
            return
        }

        var childCtx = nodeCtx
        let isScroll = node.kind == UInt8(NVE_NODE_KIND_SCROLL_VIEW)
        if hasArea && (isScroll || NveSceneReader.clipsBounds(node)) {
            let clipPath = radius > 0
                ? Path(roundedRect: rect, cornerRadius: radius)
                : Path(rect)
            childCtx.clip(to: clipPath)
        }
        if isScroll, let offset = scrollOffsets[node.node_id], offset != .zero {
            childCtx.translateBy(x: -offset.x, y: -offset.y)
        }

        var ordered: [Int] = []
        ordered.reserveCapacity(childIds.count)
        var hasNonZeroZ = false
        for cid in childIds {
            if let idx = indexById[cid] {
                ordered.append(idx)
                if reader[idx].z_index != 0 {
                    hasNonZeroZ = true
                }
            }
        }
        if hasNonZeroZ && ordered.count > 1 {
            for i in 1 ..< ordered.count {
                let key = ordered[i]
                let keyZ = reader[key].z_index
                var j = i - 1
                while j >= 0 && reader[ordered[j]].z_index > keyZ {
                    ordered[j + 1] = ordered[j]
                    j -= 1
                }
                ordered[j + 1] = key
            }
        }

        for childIdx in ordered {
            drawNode(
                reader: reader,
                indexById: indexById,
                nodeIndex: childIdx,
                into: &childCtx,
                scrollOffsets: scrollOffsets
            )
        }
    }
}

// MARK: - SwiftUI `NveSceneCanvasView`

/// Direct zero-JSON SwiftUI `Canvas` view backed by ``NveSceneStore`` / ``NveSceneReader``.
///
/// Renders 64-byte `NveRenderNode` slabs directly (rounded rects, borders, opacity, text,
/// scroll/bounds clips) and dispatches press & scroll ``NveEvent``s directly to `controller`
/// (`dispatch_events_direct`) and/or `onEvent`.
public struct NveSceneCanvasView: View {
    @ObservedObject public var store: NveSceneStore
    public let surfaceId: UInt32
    public let controller: NveSurfaceController?
    public let backgroundColor: Color?
    public let onEvent: ((NveEvent) -> Void)?

    @State private var activeScrollNodeId: UInt32?
    @State private var lastDragLocation: CGPoint?

    public init(
        store: NveSceneStore,
        surfaceId: UInt32 = 1,
        controller: NveSurfaceController? = nil,
        backgroundColor: Color? = nil,
        onEvent: ((NveEvent) -> Void)? = nil
    ) {
        self.store = store
        self.surfaceId = surfaceId
        self.controller = controller
        self.backgroundColor = backgroundColor
        self.onEvent = onEvent
    }

    public var body: some View {
        Canvas { context, size in
            if let backgroundColor {
                context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(backgroundColor))
            }
            NveSceneRenderer.draw(store: store, into: &context, size: size)
        }
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 0)
                .onChanged { value in
                    let start = value.startLocation
                    let current = value.location
                    let dx = current.x - start.x
                    let dy = current.y - start.y
                    let movedEnough = (dx * dx + dy * dy) >= 16.0

                    if activeScrollNodeId == nil && movedEnough {
                        activeScrollNodeId = store.hitTestScrollable(at: start)
                        lastDragLocation = start
                    }
                    if let scrollId = activeScrollNodeId {
                        let prev = lastDragLocation ?? start
                        let stepX = -(current.x - prev.x)
                        let stepY = -(current.y - prev.y)
                        lastDragLocation = current
                        if let updated = store.scrollBy(
                            nodeId: scrollId,
                            deltaX: stepX,
                            deltaY: stepY
                        ) {
                            emit(
                                NveEvent(
                                    surfaceId: controller?.surfaceId ?? surfaceId,
                                    nodeId: scrollId,
                                    timestampUs: UInt64(Date().timeIntervalSince1970 * 1_000_000),
                                    kind: .scroll,
                                    payload: .scroll(
                                        x: Float(updated.offset.x),
                                        y: Float(updated.offset.y),
                                        contentWidth: Float(updated.contentSize.width),
                                        contentHeight: Float(updated.contentSize.height)
                                    )
                                )
                            )
                        }
                    }
                }
                .onEnded { value in
                    defer {
                        activeScrollNodeId = nil
                        lastDragLocation = nil
                    }
                    if activeScrollNodeId != nil {
                        return
                    }
                    if let hitId = store.hitTest(at: value.location, interactiveOnly: true) {
                        emit(
                            NveEvent(
                                surfaceId: controller?.surfaceId ?? surfaceId,
                                nodeId: hitId,
                                timestampUs: UInt64(Date().timeIntervalSince1970 * 1_000_000),
                                kind: .press,
                                payload: .point(
                                    x: Float(value.location.x),
                                    y: Float(value.location.y)
                                )
                            )
                        )
                    }
                }
        )
    }

    private func emit(_ event: NveEvent) {
        _ = controller?.dispatchEvent(event)
        onEvent?(event)
    }
}
