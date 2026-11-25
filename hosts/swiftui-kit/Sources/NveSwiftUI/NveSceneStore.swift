import CNveAbi
import Foundation
import SwiftUI

/// Explicit accessibility role override for a scene node.
public enum NveAccessibilityRole: Equatable, Hashable, Sendable {
    case none
    case button
    case text
    case header
    case image
    case textField
    case scrollView
}

/// Optional per-node rendering & VoiceOver accessibility metadata layered alongside
/// the compact 64-byte `NveRenderNode` struct.
public struct NveSceneNodeMetadata: Equatable, Sendable {
    public var resizeMode: NveImageResizeMode
    public var placeholder: String?
    public var secureTextEntry: Bool
    public var accessibilityLabel: String?
    public var accessibilityHint: String?
    public var accessibilityRole: NveAccessibilityRole?
    public var isDisabled: Bool

    public init(
        resizeMode: NveImageResizeMode = .cover,
        placeholder: String? = nil,
        secureTextEntry: Bool = false,
        accessibilityLabel: String? = nil,
        accessibilityHint: String? = nil,
        accessibilityRole: NveAccessibilityRole? = nil,
        isDisabled: Bool = false
    ) {
        self.resizeMode = resizeMode
        self.placeholder = placeholder
        self.secureTextEntry = secureTextEntry
        self.accessibilityLabel = accessibilityLabel
        self.accessibilityHint = accessibilityHint
        self.accessibilityRole = accessibilityRole
        self.isDisabled = isDisabled
    }
}

/// Snapshot of one VoiceOver accessibility element derived from the live `NveSceneStore`.
public struct NveSceneAccessibilityElement: Identifiable, Equatable, Sendable {
    public let id: UInt32
    public let frame: CGRect
    public let label: String
    public let value: String?
    public let hint: String?
    public let role: NveAccessibilityRole
    public let isInteractive: Bool
    public let isHeading: Bool
    public let isTextField: Bool
    public let isImage: Bool
    public let isSecure: Bool
    public let isDisabled: Bool

    public init(
        id: UInt32,
        frame: CGRect,
        label: String,
        value: String? = nil,
        hint: String? = nil,
        role: NveAccessibilityRole,
        isInteractive: Bool,
        isHeading: Bool,
        isTextField: Bool,
        isImage: Bool,
        isSecure: Bool,
        isDisabled: Bool
    ) {
        self.id = id
        self.frame = frame
        self.label = label
        self.value = value
        self.hint = hint
        self.role = role
        self.isInteractive = isInteractive
        self.isHeading = isHeading
        self.isTextField = isTextField
        self.isImage = isImage
        self.isSecure = isSecure
        self.isDisabled = isDisabled
    }
}

// MARK: - Scene Store (Flat 64-byte `NveRenderNode` Slab + Incremental Index Maintenance)

/// Retains a flat contiguous array of 64-byte `NveRenderNode` C structs plus `textSlab`,
/// `childrenSlab`, and an incrementally maintained `[UInt32: Int]` lookup map, applying
/// `NveSceneView` updates in `O(dirtyCount + removedCount)` with zero per-frame dictionary rebuilds.
public final class NveSceneStore: ObservableObject {
    public static let inferredHeadingFontSize: Float = 20.0

    @Published public private(set) var frameId: UInt64 = 0
    public private(set) var nodes: [NveRenderNode] = []
    public private(set) var textSlab: [UInt8] = []
    public private(set) var childrenSlab: [UInt32] = []
    public private(set) var lastDirtyCount: Int = 0
    public private(set) var lastRemovedCount: Int = 0
    public private(set) var scrollOffsets: [UInt32: CGPoint] = [:]
    public private(set) var nodeMetadata: [UInt32: NveSceneNodeMetadata] = [:]

    /// Incremented only when `idToIndex` undergoes a full `O(N)` rebuild (initial load or
    /// node-count resize). Incremental dirty updates and `draw`/`hitTest` passes never rebuild it.
    public private(set) var indexRebuildCount: Int = 0

    private var idToIndex: [UInt32: Int] = [:]

    public init() {}

    public var nodeCount: Int { nodes.count }

    /// Incrementally maintained `[nodeId: arrayIndex]` table reused directly by
    /// ``NveSceneRenderer`` and ``NveSceneHitTester`` with zero per-frame allocation.
    public var nodeIndexById: [UInt32: Int] { idToIndex }

    /// Sets per-node rendering and accessibility metadata for `nodeId`.
    public func setMetadata(_ metadata: NveSceneNodeMetadata, forNodeId nodeId: UInt32) {
        nodeMetadata[nodeId] = metadata
        objectWillChange.send()
    }

    public func metadata(forNodeId nodeId: UInt32) -> NveSceneNodeMetadata? {
        nodeMetadata[nodeId]
    }

    public func setImageResizeMode(_ mode: NveImageResizeMode, forNodeId nodeId: UInt32) {
        var meta = nodeMetadata[nodeId] ?? NveSceneNodeMetadata()
        meta.resizeMode = mode
        nodeMetadata[nodeId] = meta
        objectWillChange.send()
    }

    public func setTextInputConfig(
        placeholder: String? = nil,
        secureTextEntry: Bool = false,
        forNodeId nodeId: UInt32
    ) {
        var meta = nodeMetadata[nodeId] ?? NveSceneNodeMetadata()
        meta.placeholder = placeholder
        meta.secureTextEntry = secureTextEntry
        nodeMetadata[nodeId] = meta
        objectWillChange.send()
    }

    public func setAccessibility(
        label: String? = nil,
        hint: String? = nil,
        role: NveAccessibilityRole? = nil,
        isDisabled: Bool = false,
        forNodeId nodeId: UInt32
    ) {
        var meta = nodeMetadata[nodeId] ?? NveSceneNodeMetadata()
        meta.accessibilityLabel = label
        meta.accessibilityHint = hint
        meta.accessibilityRole = role
        meta.isDisabled = isDisabled
        nodeMetadata[nodeId] = meta
        objectWillChange.send()
    }

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
            indexRebuildCount += 1
            for removedId in reader.removedIds {
                scrollOffsets.removeValue(forKey: removedId)
                nodeMetadata.removeValue(forKey: removedId)
            }
            lastDirtyCount = reader.dirtyCount > 0 ? reader.dirtyCount : nodes.count
            lastRemovedCount = reader.removedCount
        } else {
            for removedId in reader.removedIds {
                idToIndex.removeValue(forKey: removedId)
                scrollOffsets.removeValue(forKey: removedId)
                nodeMetadata.removeValue(forKey: removedId)
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

    /// Performs reverse z-order hit-testing reusing the incrementally maintained `idToIndex`.
    public func hitTest(at point: CGPoint, interactiveOnly: Bool = true) -> UInt32? {
        withReader { reader -> UInt32? in
            NveSceneHitTester.hitTest(
                reader: reader,
                at: point,
                scrollOffsets: scrollOffsets,
                interactiveOnly: interactiveOnly,
                indexById: idToIndex
            )
        } ?? nil
    }

    /// Finds the innermost `ScrollView` node containing `point`, reusing `idToIndex`.
    public func hitTestScrollable(at point: CGPoint) -> UInt32? {
        withReader { reader -> UInt32? in
            NveSceneHitTester.hitTestScrollable(
                reader: reader,
                at: point,
                scrollOffsets: scrollOffsets,
                indexById: idToIndex
            )
        } ?? nil
    }

    /// Builds ordered VoiceOver accessibility elements for interactive, text, heading,
    /// text-field, and labeled image nodes in the live scene tree.
    public func accessibilityElements() -> [NveSceneAccessibilityElement] {
        guard !nodes.isEmpty else { return [] }
        return withReader { reader in
            var elements: [NveSceneAccessibilityElement] = []
            let rootIdx = idToIndex[UInt32(NVE_ROOT_NODE_ID)] ?? 0
            collectAccessibility(
                reader: reader,
                nodeIndex: rootIdx,
                scrollTranslate: .zero,
                clipRect: nil,
                out: &elements
            )
            return elements
        } ?? []
    }

    private func collectAccessibility(
        reader: NveSceneReader,
        nodeIndex: Int,
        scrollTranslate: CGPoint,
        clipRect: CGRect?,
        out: inout [NveSceneAccessibilityElement]
    ) {
        let node = reader[nodeIndex]
        guard node.opacity_u8 > 0 else { return }

        let rect = CGRect(
            x: CGFloat(node.x) + scrollTranslate.x,
            y: CGFloat(node.y) + scrollTranslate.y,
            width: CGFloat(node.width),
            height: CGFloat(node.height)
        )
        let visibleRect = clipRect.map { rect.intersection($0) } ?? rect
        let isVisible = !visibleRect.isNull && visibleRect.width > 0 && visibleRect.height > 0
        let meta = nodeMetadata[node.node_id]

        let role = meta?.accessibilityRole ?? inferRole(for: node)
        let isTextField = role == .textField || node.kind == UInt8(NVE_NODE_KIND_TEXT_INPUT)
        let isButton = role == .button
            || node.kind == UInt8(NVE_NODE_KIND_PRESSABLE)
            || (NveSceneReader.isInteractive(node) && !isTextField && node.kind != UInt8(NVE_NODE_KIND_SCROLL_VIEW))
        let isHeading = role == .header
        let isImage = role == .image || node.kind == UInt8(NVE_NODE_KIND_IMAGE)
        let isSecure = isTextField && (meta?.secureTextEntry ?? false)
        let isDisabled = meta?.isDisabled ?? false

        var label = meta?.accessibilityLabel ?? ""
        var value: String?
        var hint = meta?.accessibilityHint

        if isTextField {
            let rawVal = (try? reader.text(for: node)) ?? ""
            if label.isEmpty, let placeholder = meta?.placeholder, !placeholder.isEmpty {
                label = placeholder
            }
            if !rawVal.isEmpty {
                value = isSecure ? String(repeating: "\u{2022}", count: rawVal.count) : rawVal
            }
            if (hint?.isEmpty ?? true), let placeholder = meta?.placeholder, !placeholder.isEmpty, label != placeholder {
                hint = placeholder
            }
        } else if isButton {
            if label.isEmpty {
                label = collectTextLabel(reader: reader, nodeIndex: nodeIndex)
            }
        } else if NveSceneReader.isLeafText(node), node.kind != UInt8(NVE_NODE_KIND_SKIA_CANVAS),
                  node.kind != UInt8(NVE_NODE_KIND_IMAGE)
        {
            if label.isEmpty {
                label = (try? reader.text(for: node)) ?? ""
            }
        }

        let shouldExpose = isVisible && (
            isButton
                || isTextField
                || isHeading
                || (isImage && !label.isEmpty)
                || (NveSceneReader.isLeafText(node) && !label.isEmpty)
                || !(meta?.accessibilityLabel?.isEmpty ?? true)
        )

        if shouldExpose {
            out.append(
                NveSceneAccessibilityElement(
                    id: node.node_id,
                    frame: visibleRect,
                    label: label,
                    value: value,
                    hint: (hint?.isEmpty ?? true) ? nil : hint,
                    role: role,
                    isInteractive: (isButton || isTextField) && !isDisabled,
                    isHeading: isHeading,
                    isTextField: isTextField,
                    isImage: isImage,
                    isSecure: isSecure,
                    isDisabled: isDisabled
                )
            )
            if isButton && node.child_count > 0 && meta?.accessibilityLabel == nil {
                return
            }
        }

        guard node.child_count > 0, let childIds = try? reader.children(for: node) else {
            return
        }
        let isScroll = node.kind == UInt8(NVE_NODE_KIND_SCROLL_VIEW)
        let nextClip = (isScroll || NveSceneReader.clipsBounds(node)) ? visibleRect : clipRect
        var nextTranslate = scrollTranslate
        if isScroll, let offset = scrollOffsets[node.node_id] {
            nextTranslate.x -= offset.x
            nextTranslate.y -= offset.y
        }
        let ordered = NveSceneHitTester.orderedChildIndices(
            reader: reader,
            indexById: idToIndex,
            childIds: childIds
        )
        for childIdx in ordered {
            collectAccessibility(
                reader: reader,
                nodeIndex: childIdx,
                scrollTranslate: nextTranslate,
                clipRect: nextClip,
                out: &out
            )
        }
    }

    private func inferRole(for node: NveRenderNode) -> NveAccessibilityRole {
        switch node.kind {
        case UInt8(NVE_NODE_KIND_TEXT_INPUT):
            return .textField
        case UInt8(NVE_NODE_KIND_PRESSABLE):
            return .button
        case UInt8(NVE_NODE_KIND_IMAGE):
            return .image
        case UInt8(NVE_NODE_KIND_SCROLL_VIEW):
            return .scrollView
        case UInt8(NVE_NODE_KIND_TEXT):
            if NveSceneReader.isLeafText(node) && node.font_size >= Self.inferredHeadingFontSize {
                return .header
            }
            return .text
        default:
            return NveSceneReader.isLeafText(node) ? .text : .none
        }
    }

    private func collectTextLabel(reader: NveSceneReader, nodeIndex: Int) -> String {
        let node = reader[nodeIndex]
        if node.kind != UInt8(NVE_NODE_KIND_IMAGE),
           node.kind != UInt8(NVE_NODE_KIND_SKIA_CANVAS),
           node.text_len > 0,
           let text = try? reader.text(for: node),
           !text.isEmpty
        {
            return text
        }
        guard node.child_count > 0, let childIds = try? reader.children(for: node) else {
            return ""
        }
        var parts: [String] = []
        for cid in childIds {
            if let idx = idToIndex[cid] {
                let sub = collectTextLabel(reader: reader, nodeIndex: idx)
                if !sub.isEmpty {
                    parts.append(sub)
                }
            }
        }
        return parts.joined(separator: " ")
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
