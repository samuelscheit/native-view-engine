import Foundation
import NveHost
import SwiftUI

// MARK: - Zero-Copy Runtime Scene Renderer

public enum NveSceneRenderer {
    public static func color(_ c: NveColor?) -> Color? {
        c.map { Color(.sRGB, red: $0.red, green: $0.green, blue: $0.blue, opacity: $0.alpha) }
    }

    /// Paints the published scene of `store` (backgrounds, borders, radii, opacity, visibility,
    /// `z_index` order, overflow / scroll clipping + offsets, `nve.canvas` draw lists, inherited
    /// typography via `NveTypographyEngine`, images with `object-fit` + tint, and passive text
    /// inputs not in `skipTextInputIds`) directly from the borrowed runtime tables.
    public static func draw(
        store: NveSceneStore,
        into context: inout GraphicsContext,
        skipTextInputIds: Set<NodeId> = []
    ) {
        _ = store.withTables { tables in
            if let root = tables.scene.index(of: .root) {
                drawNode(
                    tables, root, store: store, inheritedVisible: true,
                    skipTextInputIds: skipTextInputIds, into: &context
                )
            }
        }
    }

    private static func drawNode(
        _ t: NveRuntimeTables,
        _ i: Int,
        store: NveSceneStore,
        inheritedVisible: Bool,
        skipTextInputIds: Set<NodeId>,
        into context: inout GraphicsContext
    ) {
        let opacity = t.opacity(i)
        guard opacity > 0, let id = t.scene.nodeId(at: i) else { return }
        let visible: Bool
        switch t.visibility(i) {
        case .visible: visible = true
        case .hidden: visible = false
        case .inherit: visible = inheritedVisible
        }
        var ctx = context
        if opacity < 1 { ctx.opacity *= opacity }

        let rect = t.rect(i)
        let hasArea = rect.width > 0 && rect.height > 0
        let radius = min(t.borderRadius(i), min(rect.width, rect.height) / 2)
        let shape = radius > 0 ? Path(roundedRect: rect, cornerRadius: radius) : Path(rect)

        if visible && hasArea {
            if let bg = color(t.background(i)) { ctx.fill(shape, with: .color(bg)) }
            if let canvas = t.canvasDrawList(i) {
                drawCanvas(store: store, scene: t.scene, resource: canvas.resource, in: rect, cornerRadius: radius, into: &ctx)
            } else if let image = t.imageSource(i) {
                drawImage(store: store, scene: t.scene, image, in: rect, clip: shape, into: &ctx)
            } else if let input = t.textInput(i) {
                if !skipTextInputIds.contains(id) {
                    drawTextInput(input, style: t.textStyle(i), in: rect, into: &ctx)
                }
            } else if let text = t.text(i), !text.isEmpty {
                drawText(text, style: t.textStyle(i), in: rect, into: &ctx)
            }
            let bw = t.borderWidth(i)
            if bw > 0, let border = color(t.borderColor(i)) {
                let inset = bw / 2
                let strokeRect = rect.width > bw && rect.height > bw ? rect.insetBy(dx: inset, dy: inset) : rect
                let path = radius > 0 ? Path(roundedRect: strokeRect, cornerRadius: max(0, radius - inset)) : Path(strokeRect)
                ctx.stroke(path, with: .color(border), lineWidth: bw)
            }
        }

        let children = t.paintOrderedChildren(i)
        guard !children.isEmpty else { return }
        var childCtx = ctx
        if t.clipsChildren(i) && hasArea { childCtx.clip(to: shape) }
        if t.isScroll(i) {
            let offset = store.scrollOffset(for: id)
            if offset != .zero { childCtx.translateBy(x: -offset.x, y: -offset.y) }
        }
        for c in children {
            drawNode(t, c, store: store, inheritedVisible: visible, skipTextInputIds: skipTextInputIds, into: &childCtx)
        }
    }

    private static func drawCanvas(
        store: NveSceneStore,
        scene: NveScene,
        resource: UInt32,
        in rect: CGRect,
        cornerRadius: CGFloat,
        into ctx: inout GraphicsContext
    ) {
        guard let bytes = store.drawList(resource: resource, in: scene), !bytes.isEmpty else { return }
        _ = bytes.withUnsafeBytes { raw in
            try? NveDrawListRenderer.draw(raw, into: &ctx, in: rect, cornerRadius: cornerRadius)
        }
    }

    /// Paints text through the exact `NveTypographyEngine` CoreText pipeline used by
    /// `NveCoreTextMeasurer` (honoring `fontFamily`, `letterSpacing`, `lineHeight`, `maxLines`,
    /// `textDecoration`, and CSS wrapping modes identically).
    private static func drawText(_ text: String, style: NveTextStyle, in rect: CGRect, into ctx: inout GraphicsContext) {
        guard let textColor = style.color, rect.width > 0, rect.height > 0 else { return }
        let spec = NveTypographySpec(textStyle: style)
        ctx.withCGContext { cg in
            NveTypographyEngine.shared.draw(text: text, spec: spec, color: textColor, in: rect, into: cg)
        }
    }

    /// Draws an image resource read from `scene`, the scene being drawn (never a second borrow).
    private static func drawImage(
        store: NveSceneStore, scene: NveScene, _ image: (resource: UInt32, fit: NveImageFit, tint: NveColor?),
        in rect: CGRect, clip: Path, into ctx: inout GraphicsContext
    ) {
        guard let cgImage = store.image(resource: image.resource, in: scene),
              let placement = NveImagePlacement.compute(
                  destination: rect, imageSize: CGSize(width: cgImage.width, height: cgImage.height), fit: image.fit)
        else { return }
        var imageCtx = ctx
        imageCtx.clip(to: clip)
        var swiftImage = Image(decorative: cgImage, scale: 1)
        if let tint = color(image.tint) {
            swiftImage = swiftImage.renderingMode(.template)
            imageCtx.draw(imageCtx.resolve(swiftImage).tinted(tint), in: placement.drawRect)
        } else {
            imageCtx.draw(swiftImage, in: placement.drawRect)
        }
    }

    private static func drawTextInput(
        _ input: NveTextInputProps, style: NveTextStyle, in rect: CGRect,
        into ctx: inout GraphicsContext
    ) {
        let baseColor = style.color ?? NveColor(Rgba(packed: 0xFF00_0000))!
        let text: String
        let fill: NveColor
        if !input.value.isEmpty {
            text = input.secure ? String(repeating: "\u{2022}", count: input.value.count) : input.value
            fill = baseColor
        } else if !input.placeholder.isEmpty {
            text = input.placeholder
            fill = NveColor(Rgba(packed: packRgba(
                r: UInt8(min(255, max(0, Int((baseColor.red * 255).rounded())))),
                g: UInt8(min(255, max(0, Int((baseColor.green * 255).rounded())))),
                b: UInt8(min(255, max(0, Int((baseColor.blue * 255).rounded())))),
                a: UInt8(min(255, max(1, Int((baseColor.alpha * 0.45 * 255).rounded()))))
            ))) ?? baseColor
        } else {
            return
        }
        let padX = min(8, rect.width * 0.15)
        let inner = rect.insetBy(dx: padX, dy: 0)
        guard inner.width > 0, inner.height > 0 else { return }
        var spec = NveTypographySpec(textStyle: style)
        if !input.multiline {
            spec.whiteSpace = .noWrap
            spec.maxLines = 1
        }
        let measured = NveTypographyEngine.shared.measure(
            text: text, spec: spec, maxWidth: input.multiline ? inner.width : .infinity
        )
        let dy = !input.multiline && inner.height > measured.height ? (inner.height - measured.height) / 2 : 0
        let drawBox = CGRect(x: inner.minX, y: inner.minY + dy, width: inner.width, height: max(1, inner.height - dy))
        ctx.withCGContext { cg in
            NveTypographyEngine.shared.draw(text: text, spec: spec, color: fill, in: drawBox, into: cg)
        }
    }

    private static func packRgba(r: UInt8, g: UInt8, b: UInt8, a: UInt8) -> UInt32 {
        UInt32(r) | (UInt32(g) << 8) | (UInt32(b) << 16) | (UInt32(a) << 24)
    }
}

private extension GraphicsContext.ResolvedImage {
    func tinted(_ color: Color) -> GraphicsContext.ResolvedImage {
        var copy = self
        copy.shading = .color(color)
        return copy
    }
}

// MARK: - SwiftUI `NveSceneCanvasView`

/// SwiftUI `Canvas` over a runtime surface: paints the zero-copy scene, mounts platform editable
/// text inputs inside `.accessibilityElement(children: .contain)`, exposes VoiceOver elements,
/// and routes press / scroll / remote input through the runtime (`hit_test`, `scroll_by`).
public struct NveSceneCanvasView: View {
    @ObservedObject public var controller: NveSurfaceController
    @ObservedObject private var store: NveSceneStore
    public let backgroundColor: Color?

    @State private var activeScroll: NodeId?
    @State private var lastDrag: CGPoint?

    public init(controller: NveSurfaceController, backgroundColor: Color? = nil) {
        self.controller = controller
        self.store = controller.store
        self.backgroundColor = backgroundColor
    }

    public var body: some View {
        let overlays = store.sceneOverlays(includeTextInputs: NvePlatformTextInputSupport.usesNativeOverlays)
        let skipIds = Set(overlays.inputs.map(\.id))

        ZStack(alignment: .topLeading) {
            interactiveCanvas(skipTextInputIds: skipIds)
            ForEach(overlays.accessibility.filter { !skipIds.contains($0.id) }) { elem in
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
                        if elem.isInteractive { press(elem.id, at: CGPoint(x: elem.frame.midX, y: elem.frame.midY)) }
                    }
            }
            if !overlays.inputs.isEmpty {
                NveTextInputOverlayLayer(controller: controller, inputs: overlays.inputs)
            }
        }
        .background(GeometryReader { geo in
            Color.clear.onAppear { controller.resize(geo.size) }.onChange(of: geo.size) { controller.resize($0) }
        })
        .nveTvRemoteControls(surfaceId: controller.surfaceId) { controller.dispatchEvent($0) }
        .nveWatchCrownScroll { dy in
            if let target = store.scrollTarget(at: CGPoint(x: 1, y: 1)) { scroll(target, by: CGVector(dx: 0, dy: dy)) }
        }
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder
    private func interactiveCanvas(skipTextInputIds: Set<NodeId>) -> some View {
        let canvas = Canvas { context, size in
            if let backgroundColor {
                context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(backgroundColor))
            }
            NveSceneRenderer.draw(store: store, into: &context, skipTextInputIds: skipTextInputIds)
        }
        .accessibilityHidden(true)
        .contentShape(Rectangle())

        #if os(tvOS)
        canvas.onTapGesture {
            controller.dispatchEvent(NveTvRemoteCommand.select.makeKeyDownEvent(surfaceId: controller.surfaceId))
        }
        #else
        canvas.gesture(
            DragGesture(minimumDistance: 0)
                .onChanged { value in
                    let dx = value.location.x - value.startLocation.x
                    let dy = value.location.y - value.startLocation.y
                    if activeScroll == nil && dx * dx + dy * dy >= 16 {
                        activeScroll = store.scrollTarget(at: value.startLocation)
                        lastDrag = value.startLocation
                    }
                    if let target = activeScroll {
                        let prev = lastDrag ?? value.startLocation
                        lastDrag = value.location
                        scroll(target, by: CGVector(dx: prev.x - value.location.x, dy: prev.y - value.location.y))
                    }
                }
                .onEnded { value in
                    defer {
                        activeScroll = nil
                        lastDrag = nil
                    }
                    guard activeScroll == nil, let hit = store.hitTest(at: value.location) else { return }
                    press(hit, at: value.location)
                }
        )
        #endif
    }

    private func scroll(_ node: NodeId, by delta: CGVector) {
        guard let offset = store.scroll(node, by: delta) else { return }
        controller.dispatchEvent(NveEvent(
            surfaceId: controller.surfaceId, nodeId: node.raw, timestampUs: NveEvent.nowUs(), kind: .scroll,
            payload: .scroll(x: Float(offset.x), y: Float(offset.y), contentWidth: 0, contentHeight: 0)))
    }

    private func press(_ node: NodeId, at location: CGPoint) {
        controller.dispatchEvent(NveEvent(
            surfaceId: controller.surfaceId, nodeId: node.raw, timestampUs: NveEvent.nowUs(), kind: .press,
            payload: .point(x: Float(location.x), y: Float(location.y))))
    }

    private func traits(for element: NveSceneAccessibilityElement) -> AccessibilityTraits {
        var result: AccessibilityTraits = []
        if element.isHeading { _ = result.insert(.isHeader) }
        if element.isImage { _ = result.insert(.isImage) }
        if element.role == .link { _ = result.insert(.isLink) }
        if element.isInteractive && !element.isTextField && element.role != .link { _ = result.insert(.isButton) }
        if !element.isInteractive && !element.isImage { _ = result.insert(.isStaticText) }
        return result
    }
}
