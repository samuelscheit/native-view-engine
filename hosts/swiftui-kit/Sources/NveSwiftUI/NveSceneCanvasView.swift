import CNveAbi
import Foundation
import SwiftUI

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
    /// SwiftUI `GraphicsContext` (rounded rects, borders, opacity, `SkiaCanvas` `NVE_DRAW1`,
    /// cached `Image` resize modes, `TextInput` value/placeholder/secure masking, leaf text,
    /// and scroll/bounds clipping).
    ///
    /// Pass `indexById` from ``NveSceneStore/nodeIndexById`` to eliminate per-frame `O(N)`
    /// dictionary rebuilds.
    public static func draw(
        reader: NveSceneReader,
        into context: inout GraphicsContext,
        size: CGSize,
        scrollOffsets: [UInt32: CGPoint] = [:],
        indexById: [UInt32: Int]? = nil,
        nodeMetadata: [UInt32: NveSceneNodeMetadata] = [:],
        imageCache: NveImageCache = .shared
    ) {
        guard reader.count > 0 else { return }
        let lookup = indexById ?? NveSceneHitTester.buildIndexLookup(reader)
        let rootIdx = lookup[UInt32(NVE_ROOT_NODE_ID)] ?? 0
        drawNode(
            reader: reader,
            indexById: lookup,
            nodeIndex: rootIdx,
            into: &context,
            scrollOffsets: scrollOffsets,
            nodeMetadata: nodeMetadata,
            imageCache: imageCache
        )
    }

    /// Renders directly from ``NveSceneStore`` reusing its incrementally maintained
    /// `[UInt32: Int]` index map (zero per-frame `O(N)` rebuild).
    public static func draw(
        store: NveSceneStore,
        into context: inout GraphicsContext,
        size: CGSize,
        imageCache: NveImageCache = .shared
    ) {
        let prebuiltIndex = store.nodeIndexById
        _ = store.withReader { reader in
            draw(
                reader: reader,
                into: &context,
                size: size,
                scrollOffsets: store.scrollOffsets,
                indexById: prebuiltIndex,
                nodeMetadata: store.nodeMetadata,
                imageCache: imageCache
            )
        }
    }

    private static func drawNode(
        reader: NveSceneReader,
        indexById: [UInt32: Int],
        nodeIndex: Int,
        into context: inout GraphicsContext,
        scrollOffsets: [UInt32: CGPoint],
        nodeMetadata: [UInt32: NveSceneNodeMetadata],
        imageCache: NveImageCache
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
        let meta = nodeMetadata[node.node_id]

        // 1. Background fill
        if hasArea, let bgColor = color(fromRgba: node.bg_rgba) {
            let path = radius > 0
                ? Path(roundedRect: rect, cornerRadius: radius)
                : Path(rect)
            nodeCtx.fill(path, with: .color(bgColor))
        }

        // 2. Specialized node contents (`SkiaCanvas`, `Image`, `TextInput`, or leaf `Text`)
        if hasArea {
            switch node.kind {
            case UInt8(NVE_NODE_KIND_SKIA_CANVAS):
                if node.text_len > 0, let rawBytes = try? reader.textBytes(for: node) {
                    _ = try? NveDrawListRenderer.draw(
                        rawBytes,
                        into: &nodeCtx,
                        in: rect,
                        cornerRadius: radius
                    )
                }

            case UInt8(NVE_NODE_KIND_IMAGE):
                if node.text_len > 0,
                   let uri = try? reader.text(for: node),
                   !uri.isEmpty,
                   let cgImage = imageCache.image(for: uri)
                {
                    let mode = meta?.resizeMode ?? .cover
                    if let placement = NveImagePlacement.compute(
                        destination: rect,
                        imageSize: CGSize(width: cgImage.width, height: cgImage.height),
                        mode: mode
                    ) {
                        var imgCtx = nodeCtx
                        let clipPath = radius > 0
                            ? Path(roundedRect: rect, cornerRadius: radius)
                            : Path(rect)
                        imgCtx.clip(to: clipPath)
                        let swiftImg = Image(decorative: cgImage, scale: 1.0)
                        imgCtx.draw(swiftImg, in: placement.drawRect)
                    }
                }

            case UInt8(NVE_NODE_KIND_TEXT_INPUT):
                drawTextInput(
                    reader: reader,
                    node: node,
                    rect: rect,
                    meta: meta,
                    into: &nodeCtx
                )

            default:
                if NveSceneReader.isLeafText(node),
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
            }
        }

        // 3. Border stroke
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

        let ordered = NveSceneHitTester.orderedChildIndices(
            reader: reader,
            indexById: indexById,
            childIds: childIds
        )
        for childIdx in ordered {
            drawNode(
                reader: reader,
                indexById: indexById,
                nodeIndex: childIdx,
                into: &childCtx,
                scrollOffsets: scrollOffsets,
                nodeMetadata: nodeMetadata,
                imageCache: imageCache
            )
        }
    }

    private static func drawTextInput(
        reader: NveSceneReader,
        node: NveRenderNode,
        rect: CGRect,
        meta: NveSceneNodeMetadata?,
        into context: inout GraphicsContext
    ) {
        let fontSize = (node.font_size.isFinite && node.font_size > 0)
            ? CGFloat(node.font_size)
            : 14.0
        let rawVal = (try? reader.text(for: node)) ?? ""

        let displayText: String
        let displayColor: Color

        if !rawVal.isEmpty {
            displayText = (meta?.secureTextEntry ?? false)
                ? String(repeating: "\u{2022}", count: rawVal.count)
                : rawVal
            displayColor = color(fromRgba: node.text_rgba)
                ?? Color(.sRGB, red: 241.0 / 255.0, green: 245.0 / 255.0, blue: 249.0 / 255.0, opacity: 1.0)
        } else if let placeholder = meta?.placeholder, !placeholder.isEmpty {
            displayText = placeholder
            let alphaByte = (node.text_rgba >> 24) & 0xFF
            if alphaByte > 0 {
                let r = Double(node.text_rgba & 0xFF) / 255.0
                let g = Double((node.text_rgba >> 8) & 0xFF) / 255.0
                let b = Double((node.text_rgba >> 16) & 0xFF) / 255.0
                let mutedAlpha = min(max(Double(alphaByte) * 0.55, 96.0), 170.0) / 255.0
                displayColor = Color(.sRGB, red: r, green: g, blue: b, opacity: mutedAlpha)
            } else {
                displayColor = Color(
                    .sRGB,
                    red: 148.0 / 255.0,
                    green: 163.0 / 255.0,
                    blue: 184.0 / 255.0,
                    opacity: 140.0 / 255.0
                )
            }
        } else {
            return
        }

        let padX = min(8.0, rect.width * 0.15)
        let innerRect = CGRect(
            x: rect.minX + padX,
            y: rect.minY,
            width: max(1.0, rect.width - padX * 2.0),
            height: rect.height
        )
        var textCtx = context
        textCtx.clip(to: Path(rect))
        let resolved = textCtx.resolve(
            Text(displayText)
                .font(.system(size: fontSize))
                .foregroundColor(displayColor)
        )
        let measured = resolved.measure(in: innerRect.size)
        let dy = innerRect.height > measured.height ? (innerRect.height - measured.height) * 0.5 : 0
        textCtx.draw(resolved, at: CGPoint(x: innerRect.minX, y: innerRect.minY + dy), anchor: .topLeading)
    }
}

// MARK: - SwiftUI `NveSceneCanvasView`

/// Direct binary SwiftUI `Canvas` view backed by ``NveSceneStore`` / ``NveSceneReader``.
///
/// Renders 64-byte `NveRenderNode` slabs directly (rounded rects, borders, opacity, `SkiaCanvas`,
/// cached `Image`s, `TextInput`s, text, scroll/bounds clips), exposes VoiceOver accessibility
/// elements, and dispatches press & scroll ``NveEvent``s directly to `controller` / `onEvent`.
public struct NveSceneCanvasView: View {
    @ObservedObject public var store: NveSceneStore
    public let surfaceId: UInt32
    public let controller: NveSurfaceController?
    public let backgroundColor: Color?
    public let imageCache: NveImageCache
    public let onEvent: ((NveEvent) -> Void)?

    @State private var activeScrollNodeId: UInt32?
    @State private var lastDragLocation: CGPoint?

    public init(
        store: NveSceneStore,
        surfaceId: UInt32 = 1,
        controller: NveSurfaceController? = nil,
        backgroundColor: Color? = nil,
        imageCache: NveImageCache = .shared,
        onEvent: ((NveEvent) -> Void)? = nil
    ) {
        self.store = store
        self.surfaceId = surfaceId
        self.controller = controller
        self.backgroundColor = backgroundColor
        self.imageCache = imageCache
        self.onEvent = onEvent
    }

    public var body: some View {
        ZStack(alignment: .topLeading) {
            interactiveCanvas

            // VoiceOver accessibility overlay (non-hit-testing so pointer gestures hit Canvas directly).
            ForEach(store.accessibilityElements()) { elem in
                Color.clear
                    .frame(width: elem.frame.width, height: elem.frame.height)
                    .position(x: elem.frame.midX, y: elem.frame.midY)
                    .allowsHitTesting(false)
                    .nveSpatialHoverEffect()
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(Text(elem.label))
                    .accessibilityValue(Text(elem.value ?? ""))
                    .accessibilityHint(Text(elem.hint ?? ""))
                    .accessibilityAddTraits(traits(for: elem))
                    .accessibilityAction {
                        if elem.isInteractive {
                            emitPress(
                                nodeId: elem.id,
                                location: CGPoint(x: elem.frame.midX, y: elem.frame.midY)
                            )
                        }
                    }
            }
        }
        .nveTvRemoteControls(surfaceId: controller?.surfaceId ?? surfaceId, onEvent: emit)
        .nveWatchCrownScroll { deltaY in
            handleWatchCrownScroll(deltaY: deltaY)
        }
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder
    private var interactiveCanvas: some View {
        let baseCanvas = Canvas { context, size in
            if let backgroundColor {
                context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(backgroundColor))
            }
            NveSceneRenderer.draw(
                store: store,
                into: &context,
                size: size,
                imageCache: imageCache
            )
        }
        .accessibilityHidden(true)
        .contentShape(Rectangle())

        #if os(tvOS)
        baseCanvas.onTapGesture {
            emit(NveTvRemoteCommand.select.makeKeyDownEvent(surfaceId: controller?.surfaceId ?? surfaceId))
        }
        #else
        baseCanvas.gesture(
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
                        emitScrollDelta(nodeId: scrollId, deltaX: stepX, deltaY: stepY)
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
                        emitPress(nodeId: hitId, location: value.location)
                    }
                }
        )
        #endif
    }

    private func handleWatchCrownScroll(deltaY: CGFloat) {
        if let scrollId = store.hitTestScrollable(at: CGPoint(x: 1, y: 1)) {
            emitScrollDelta(nodeId: scrollId, deltaX: 0, deltaY: deltaY)
        }
    }

    private func emitScrollDelta(nodeId: UInt32, deltaX: CGFloat, deltaY: CGFloat) {
        if let updated = store.scrollBy(nodeId: nodeId, deltaX: deltaX, deltaY: deltaY) {
            emit(
                NveEvent(
                    surfaceId: controller?.surfaceId ?? surfaceId,
                    nodeId: nodeId,
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

    private func traits(for element: NveSceneAccessibilityElement) -> AccessibilityTraits {
        var result: AccessibilityTraits = []
        if element.isHeading {
            _ = result.insert(.isHeader)
        }
        if element.isImage {
            _ = result.insert(.isImage)
        }
        if element.isInteractive && !element.isTextField {
            _ = result.insert(.isButton)
        }
        if !element.isInteractive && !element.isImage && !element.isTextField {
            _ = result.insert(.isStaticText)
        }
        return result
    }

    private func emitPress(nodeId: UInt32, location: CGPoint) {
        emit(
            NveEvent(
                surfaceId: controller?.surfaceId ?? surfaceId,
                nodeId: nodeId,
                timestampUs: UInt64(Date().timeIntervalSince1970 * 1_000_000),
                kind: .press,
                payload: .point(x: Float(location.x), y: Float(location.y))
            )
        )
    }

    private func emit(_ event: NveEvent) {
        _ = controller?.dispatchEvent(event)
        onEvent?(event)
    }
}
