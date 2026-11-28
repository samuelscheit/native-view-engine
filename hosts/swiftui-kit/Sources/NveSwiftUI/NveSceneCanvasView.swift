import Foundation
import NveHost
import SwiftUI

// MARK: - Zero-Copy Runtime Scene Renderer

public enum NveSceneRenderer {
    public static func color(_ c: NveColor?) -> Color? {
        c.map { Color(.sRGB, red: $0.red, green: $0.green, blue: $0.blue, opacity: $0.alpha) }
    }

    /// Paints the published scene of `store` (backgrounds, borders, radii, opacity, visibility,
    /// `z_index` order, overflow / scroll clipping + offsets, inherited typography, images with
    /// `object-fit` + tint, and text inputs) directly from the borrowed runtime tables.
    public static func draw(store: NveSceneStore, into context: inout GraphicsContext) {
        _ = store.withTables { tables in
            if let root = tables.scene.index(of: .root) {
                drawNode(tables, root, store: store, inheritedVisible: true, into: &context)
            }
        }
    }

    private static func drawNode(
        _ t: NveRuntimeTables, _ i: Int, store: NveSceneStore, inheritedVisible: Bool, into context: inout GraphicsContext
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
            if let image = t.imageSource(i) {
                drawImage(store: store, scene: t.scene, image, in: rect, clip: shape, into: &ctx)
            } else if let input = t.textInput(i) {
                drawTextInput(input, style: t.textStyle(i), in: rect, into: &ctx)
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
            drawNode(t, c, store: store, inheritedVisible: visible, into: &childCtx)
        }
    }

    static func resolvedText(_ text: String, style: NveTextStyle, color: Color) -> Text {
        var out = Text(text)
            .font(.system(size: style.fontSize, weight: weight(style.fontWeight)))
            .foregroundColor(color)
        if style.italic { out = out.italic() }
        if style.decoration & NveTextDecoration.underline != 0 { out = out.underline() }
        if style.decoration & NveTextDecoration.lineThrough != 0 { out = out.strikethrough() }
        return out
    }

    private static func weight(_ w: UInt16) -> Font.Weight {
        switch w {
        case ..<150: return .ultraLight
        case ..<250: return .thin
        case ..<350: return .light
        case ..<450: return .regular
        case ..<550: return .medium
        case ..<650: return .semibold
        case ..<750: return .bold
        case ..<850: return .heavy
        default: return .black
        }
    }

    /// Horizontal position of a text block in its box: `0` leading, `0.5` center, `1` trailing.
    private static func alignFraction(_ align: NveTextAlign) -> CGFloat {
        switch align {
        case .center: return 0.5
        case .end, .right: return 1
        default: return 0
        }
    }

    /// Wraps within the laid-out box (the runtime measured it with the same metrics), caps at
    /// `maxLines`, and aligns the wrapped block horizontally.
    private static func drawText(_ text: String, style: NveTextStyle, in rect: CGRect, into ctx: inout GraphicsContext) {
        guard let color = color(style.color), rect.width > 0 else { return }
        let resolved = ctx.resolve(resolvedText(text, style: style, color: color))
        var box = rect
        if style.maxLines > 0 {
            // One line of this style: the text itself may hold line breaks.
            let probe = ctx.resolve(resolvedText("Ag", style: style, color: color))
            let line = probe.measure(in: CGSize(width: CGFloat.infinity, height: .infinity)).height
            box.size.height = min(box.height, line * CGFloat(style.maxLines))
        }
        let used = resolved.measure(in: box.size)
        box.origin.x += max(0, box.width - used.width) * alignFraction(style.align)
        box.size.width = min(box.width, max(used.width, 1))
        ctx.draw(resolved, in: box)
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
        _ input: (value: String, placeholder: String, secure: Bool), style: NveTextStyle, in rect: CGRect,
        into ctx: inout GraphicsContext
    ) {
        let text: String
        let fill: Color
        if !input.value.isEmpty {
            text = input.secure ? String(repeating: "\u{2022}", count: input.value.count) : input.value
            fill = color(style.color) ?? .primary
        } else if !input.placeholder.isEmpty {
            text = input.placeholder
            fill = (color(style.color) ?? .primary).opacity(0.45)
        } else {
            return
        }
        let padX = min(8, rect.width * 0.15)
        let inner = rect.insetBy(dx: padX, dy: 0)
        var textCtx = ctx
        textCtx.clip(to: Path(rect))
        let resolved = textCtx.resolve(resolvedText(text, style: style, color: fill))
        let measured = resolved.measure(in: inner.size)
        let dy = inner.height > measured.height ? (inner.height - measured.height) / 2 : 0
        textCtx.draw(resolved, at: CGPoint(x: inner.minX, y: inner.minY + dy), anchor: .topLeading)
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

/// SwiftUI `Canvas` over a runtime surface: paints the zero-copy scene, exposes VoiceOver
/// elements, and routes press / scroll / remote input through the runtime (`hit_test`,
/// `scroll_by`) to the attached app (`dispatch_events`).
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
        ZStack(alignment: .topLeading) {
            interactiveCanvas
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
                        if elem.isInteractive { press(elem.id, at: CGPoint(x: elem.frame.midX, y: elem.frame.midY)) }
                    }
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
    private var interactiveCanvas: some View {
        let canvas = Canvas { context, size in
            if let backgroundColor {
                context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(backgroundColor))
            }
            NveSceneRenderer.draw(store: store, into: &context)
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
