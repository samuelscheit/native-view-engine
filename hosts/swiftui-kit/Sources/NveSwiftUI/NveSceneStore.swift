import CNveAbi
import Combine
import CoreGraphics
import Foundation
import NveHost

/// Accessibility role exposed for a scene node.
public enum NveAccessibilityRole: Equatable, Hashable, Sendable {
    case none, button, link, text, header, image, textField, scrollView
}

/// One VoiceOver element derived from the published scene (`nve.accessibility`, `nve.text`, …).
public struct NveSceneAccessibilityElement: Identifiable, Equatable, Sendable {
    public let id: NodeId
    public let frame: CGRect
    public let label: String
    public let value: String?
    public let hint: String?
    public let role: NveAccessibilityRole
    public let isInteractive: Bool
    public let isSecure: Bool

    public var isHeading: Bool { role == .header }
    public var isTextField: Bool { role == .textField }
    public var isImage: Bool { role == .image }
}

/// Mounted overlay descriptor for one live visible `nve.text_input` node in surface coordinates.
public struct NveTextInputDescriptor: Identifiable, Equatable, Sendable {
    public let id: NodeId
    public let frame: CGRect
    public let visibleClip: CGRect?
    public let props: NveTextInputProps
    public let style: NveTextStyle
    public let opacity: Double
    public let accessibilityLabel: String
    public let accessibilityHint: String?
}

/// Host-side view state over a runtime surface's zero-copy scene: publishes frame changes to
/// SwiftUI, tracks scroll offsets (owned by the runtime, mirrored per frame), and resolves image
/// and canvas draw-list resources. Scene slabs are never copied: every read borrows the scene.
public final class NveSceneStore: ObservableObject {
    /// Font size from which an un-annotated leaf text is announced as a heading.
    public static let inferredHeadingFontSize: CGFloat = 20
    /// Bounded cache ceiling for decoded `imageSources` and `drawLists` (matching `NveAppleSceneReconciler`).
    internal static let maxCachedResources = 256

    @Published public private(set) var frameId: UInt64 = 0
    public private(set) var scrollOffsets: [NodeId: CGPoint] = [:]
    public private(set) var lastDirtyCount = 0
    public private(set) var lastRemovedCount = 0

    public let session: NveRuntimeSession
    public let imageCache: NveImageCache
    /// Decoded `ImageSrc` per resource id, valid while the id's bytes are (see `sync()`).
    private var imageSources: [UInt32: CachedImageSource] = [:]
    /// Cached flat `nve.res.draw_list` bytes per resource id, invalidated alongside `imageSources`.
    private var drawLists: [UInt32: [UInt8]] = [:]
    internal var cachedDrawListCount: Int { drawLists.count }
    private var resourcesEpoch: UInt32 = 0
    /// Frame the resource caches are valid for (`nil`: none synced yet).
    private var resourcesFrame: UInt64?

    private struct CachedImageSource {
        let source: NveImageSource
        /// `imageCache` key of encoded bytes: content-derived, hashed once per resource version.
        let encodedKey: String?
    }

    public init(session: NveRuntimeSession, imageCache: NveImageCache = .shared) {
        self.session = session
        self.imageCache = imageCache
    }

    /// Runs `body` over the published scene; `nil` when no scene is published yet.
    public func withTables<R>(_ body: (NveRuntimeTables) -> R) -> R? {
        let result: R?? = try? session.withScene { scene in
            scene.nodeCapacity > 1 ? body(NveRuntimeTables(scene: scene)) : nil
        }
        return result ?? nil
    }

    /// Absorbs a newly published frame (call after `tick` reported a change).
    public func sync() {
        _ = try? session.withScene { scene in
            // Resource ids keep their bytes unless listed in `dirtyResources`; a new scene epoch
            // (ids reused by another app / pairing) or a skipped frame invalidates every entry. A
            // repeated sync of the same frame changes nothing.
            let sameEpoch = scene.sceneEpoch == resourcesEpoch
            if sameEpoch, scene.frameId == resourcesFrame {
                // already absorbed
            } else if sameEpoch, let last = resourcesFrame, scene.frameId == last &+ 1 {
                for id in scene.dirtyResources {
                    imageSources.removeValue(forKey: id)
                    drawLists.removeValue(forKey: id)
                }
            } else {
                imageSources.removeAll(keepingCapacity: true)
                drawLists.removeAll(keepingCapacity: true)
            }
            resourcesEpoch = scene.sceneEpoch
            resourcesFrame = scene.frameId
            // The runtime owns scroll offsets (incl. programmatic `requested_scroll_*`) and publishes
            // every container's visual child shift with the frame.
            scrollOffsets.removeAll(keepingCapacity: true)
            for s in scene.scrollOffsets {
                scrollOffsets[NodeId(raw: s.node_id)] = CGPoint(x: CGFloat(s.x), y: CGFloat(s.y))
            }
            lastDirtyCount = scene.dirtyIndices.count
            lastRemovedCount = scene.removedIds.count
            frameId = scene.frameId
        }
    }

    public func scrollOffset(for id: NodeId) -> CGPoint { scrollOffsets[id] ?? .zero }

    /// Scrolls `id` by `delta` (clamped by the runtime) and returns the runtime's new scroll
    /// position. The drawn offsets (``scrollOffset(for:)``) change only with the next published
    /// frame, so they never run ahead of the scene they shift.
    @discardableResult
    public func scroll(_ id: NodeId, by delta: CGVector) -> CGPoint? {
        try? session.scroll(id, by: delta)
    }

    /// Topmost interactive node at `point` (runtime hit test, scroll-aware).
    public func hitTest(at point: CGPoint) -> NodeId? {
        guard let id = try? session.hitTest(point), !id.isNull else { return nil }
        return id
    }

    /// Innermost scroll container under `point`.
    public func scrollTarget(at point: CGPoint) -> NodeId? {
        withTables { t -> NodeId? in
            var target: NodeId?
            var p = point
            var index = t.scene.index(of: .root)
            while let i = index, let id = t.scene.nodeId(at: i) {
                if t.isScroll(i) {
                    target = id
                    let o = scrollOffset(for: id)
                    p.x += o.x
                    p.y += o.y
                }
                index = t.paintOrderedChildren(i).last { t.rect($0).contains(p) }
            }
            return target
        } ?? nil
    }

    private static let imageSrcKind = Key64.of("nve.res.image_src")

    /// Decoded `ImageSrc` of resource `id`, cached until the runtime republishes the id's bytes.
    public func imageSource(_ id: UInt32) -> NveImageSource? { cachedImageSource(id, in: nil)?.source }

    /// Reads a missing entry from `scene` when the caller already borrows it (drawing), else
    /// borrows the published scene for the read.
    private func cachedImageSource(_ id: UInt32, in scene: NveScene?) -> CachedImageSource? {
        if let cached = imageSources[id] { return cached }
        guard let res = fetchResource(id, in: scene),
              res.kind == Self.imageSrcKind,
              let src = NveImageSource(resourceBytes: res.bytes)
        else { return nil }
        let key: String? = if case let .encoded(data) = src { "nve.res.image_src#\(Xxh64.hash(data))" } else { nil }
        let cached = CachedImageSource(source: src, encodedKey: key)
        if imageSources.count >= Self.maxCachedResources { imageSources.removeAll(keepingCapacity: true) }
        imageSources[id] = cached
        return cached
    }

    /// `CGImage` for an `nve.image` resource; `scene`: the scene the caller already borrows.
    public func image(resource id: UInt32, in scene: NveScene? = nil) -> CGImage? {
        guard let cached = cachedImageSource(id, in: scene) else { return nil }
        switch cached.source {
        case let .url(url): return imageCache.image(for: url)
        case let .encoded(bytes): return imageCache.image(encoded: bytes, key: cached.encodedKey ?? "")
        }
    }

    /// Flat `nve.res.draw_list` bytes for an `nve.canvas` resource (strictly kind-checked and cached).
    public func drawList(resource id: UInt32, in scene: NveScene? = nil) -> [UInt8]? {
        if let cached = drawLists[id] { return cached }
        guard let res = fetchResource(id, in: scene), res.kind == NveDrawList.kind, !res.bytes.isEmpty else {
            return nil
        }
        if drawLists.count >= Self.maxCachedResources { drawLists.removeAll(keepingCapacity: true) }
        drawLists[id] = res.bytes
        return res.bytes
    }

    private func fetchResource(_ id: UInt32, in scene: NveScene?) -> (kind: Key64, bytes: [UInt8])? {
        if let scene {
            return try? session.resource(id, in: scene)
        }
        return try? session.withScene { try session.resource(id, in: $0) }
    }

    // MARK: Mounted Editable Text Inputs

    /// Collects mounted text input overlays and VoiceOver accessibility elements in a single
    /// `SceneReader.open()` pass so SwiftUI body updates avoid redundant scene borrows.
    public func sceneOverlays(
        includeTextInputs: Bool
    ) -> (inputs: [NveTextInputDescriptor], accessibility: [NveSceneAccessibilityElement]) {
        withTables { tables in
            var inputs: [NveTextInputDescriptor] = []
            var a11y: [NveSceneAccessibilityElement] = []
            if let root = tables.scene.index(of: .root) {
                if includeTextInputs {
                    collectTextInputs(tables, root, inheritedVisible: true, opacity: 1, translate: .zero, clip: nil, into: &inputs)
                }
                collect(tables, root, translate: .zero, clip: nil, into: &a11y)
            }
            return (inputs, a11y)
        } ?? ([], [])
    }

    /// Returns every live visible `nve.text_input` node in paint order with surface-space bounds.
    public func textInputs() -> [NveTextInputDescriptor] {
        withTables { tables in
            var out: [NveTextInputDescriptor] = []
            if let root = tables.scene.index(of: .root) {
                collectTextInputs(tables, root, inheritedVisible: true, opacity: 1, translate: .zero, clip: nil, into: &out)
            }
            return out
        } ?? []
    }

    /// Reads the latest app-published `nve.text_input.value` for `nodeId` (if still live).
    public func textInputValue(for nodeId: NodeId) -> String? {
        withTables { tables -> String? in
            guard let index = tables.scene.index(of: nodeId) else { return nil }
            return tables.textInput(index)?.value
        } ?? nil
    }

    private func collectTextInputs(
        _ t: NveRuntimeTables,
        _ i: Int,
        inheritedVisible: Bool,
        opacity inheritedOpacity: Double,
        translate: CGPoint,
        clip: CGRect?,
        into out: inout [NveTextInputDescriptor]
    ) {
        let opacity = inheritedOpacity * t.opacity(i)
        guard opacity > 0, let id = t.scene.nodeId(at: i) else { return }
        let visible: Bool = switch t.visibility(i) {
        case .visible: true
        case .hidden: false
        case .inherit: inheritedVisible
        }
        let rect = t.rect(i).offsetBy(dx: translate.x, dy: translate.y)
        let clipped = clip.map { rect.intersection($0) } ?? rect
        let hasArea = !clipped.isNull && clipped.width > 0 && clipped.height > 0

        if visible && hasArea, let input = t.textInput(i) {
            let contentFrame = t.contentRect(i, borderBox: rect)
            let contentClipped = clip.map { contentFrame.intersection($0) } ?? contentFrame
            if !contentClipped.isNull, contentClipped.width > 0, contentClipped.height > 0 {
                let a11y = t.accessibility(i)
                let label = (a11y?.label?.isEmpty == false ? a11y?.label : nil) ?? input.placeholder
                let hint = a11y?.hint?.isEmpty == true ? nil : a11y?.hint
                out.append(NveTextInputDescriptor(
                    id: id,
                    frame: contentFrame,
                    visibleClip: clip,
                    props: input,
                    style: t.textStyle(i),
                    opacity: opacity,
                    accessibilityLabel: label,
                    accessibilityHint: hint
                ))
            }
        }

        let clipsChildren = t.clipsChildren(i)
        var next = translate
        if t.isScroll(i) {
            let o = scrollOffset(for: id)
            next.x -= o.x
            next.y -= o.y
        }
        for c in t.paintOrderedChildren(i) {
            collectTextInputs(
                t, c, inheritedVisible: visible, opacity: opacity, translate: next,
                clip: clipsChildren ? clipped : clip, into: &out)
        }
    }

    // MARK: Accessibility

    public func accessibilityElements() -> [NveSceneAccessibilityElement] {
        withTables { tables in
            var out: [NveSceneAccessibilityElement] = []
            if let root = tables.scene.index(of: .root) {
                collect(tables, root, translate: .zero, clip: nil, into: &out)
            }
            return out
        } ?? []
    }

    private func collect(
        _ t: NveRuntimeTables, _ i: Int, translate: CGPoint, clip: CGRect?, into out: inout [NveSceneAccessibilityElement]
    ) {
        guard t.opacity(i) > 0, let id = t.scene.nodeId(at: i) else { return }
        let a11y = t.accessibility(i)
        if a11y?.hidden == true { return }
        let rect = t.rect(i).offsetBy(dx: translate.x, dy: translate.y)
        let visible = clip.map { rect.intersection($0) } ?? rect
        let isVisible = !visible.isNull && visible.width > 0 && visible.height > 0
        let input = t.textInput(i)
        let role = Self.role(t, i, aria: a11y?.role, input: input != nil)
        let interactive = role == .button || role == .link || role == .textField
        var label = a11y?.label ?? ""
        var value = a11y?.value
        var hint = a11y?.hint
        if let input {
            if label.isEmpty { label = input.placeholder }
            if !input.value.isEmpty {
                value = input.secure ? String(repeating: "\u{2022}", count: input.value.count) : input.value
            }
            if hint == nil, !input.placeholder.isEmpty, label != input.placeholder { hint = input.placeholder }
        } else if label.isEmpty {
            label = interactive ? collectText(t, i) : (t.text(i) ?? "")
        }
        let expose = isVisible && (interactive || role == .header || (!label.isEmpty && role != .none) || a11y?.label != nil)
        if expose {
            out.append(NveSceneAccessibilityElement(
                id: id, frame: visible, label: label, value: value, hint: hint?.isEmpty == true ? nil : hint,
                role: role, isInteractive: interactive, isSecure: input?.secure == true))
            if interactive { return }  // A control's subtree is its label.
        }
        let clipsChildren = t.clipsChildren(i)
        var next = translate
        if t.isScroll(i) {
            let o = scrollOffset(for: id)
            next.x -= o.x
            next.y -= o.y
        }
        for c in t.paintOrderedChildren(i) {
            collect(t, c, translate: next, clip: clipsChildren ? visible : clip, into: &out)
        }
    }

    private static func role(_ t: NveRuntimeTables, _ i: Int, aria: NveAriaRole?, input: Bool) -> NveAccessibilityRole {
        switch aria {
        case .button?: return .button
        case .link?: return .link
        case .heading?: return .header
        case .image?: return .image
        case .textBox?, .searchBox?: return .textField
        case .scrollArea?: return .scrollView
        case .text?: return .text
        default: break
        }
        if input { return .textField }
        if t.isLink(i) { return .link }
        if t.isInteractive(i) { return .button }
        if t.isImage(i) { return .image }
        if t.isScroll(i) { return .scrollView }
        if t.text(i) != nil {
            return t.textStyle(i).fontSize >= inferredHeadingFontSize ? .header : .text
        }
        return .none
    }

    private func collectText(_ t: NveRuntimeTables, _ i: Int) -> String {
        if let text = t.text(i), !text.isEmpty { return text }
        return t.paintOrderedChildren(i).map { collectText(t, $0) }.filter { !$0.isEmpty }.joined(separator: " ")
    }
}

/// Decoded `nve.res.image_src` resource (`ImageSrc` `FlatResource` 4-byte-aligned word encoding).
public enum NveImageSource: Equatable, Sendable {
    case url(String)
    case encoded([UInt8])

    /// `FlatResource` layout: `u32 tag` (`0` = URL, `1` = encoded bytes) | `u32 byte_len` | payload
    /// zero-padded to a 4-byte boundary.
    public init?(resourceBytes b: [UInt8]) {
        guard b.count >= 8, (b.count & 3) == 0 else { return nil }
        let tag = UInt32(b[0]) | UInt32(b[1]) << 8 | UInt32(b[2]) << 16 | UInt32(b[3]) << 24
        let len = UInt32(b[4]) | UInt32(b[5]) << 8 | UInt32(b[6]) << 16 | UInt32(b[7]) << 24
        let maxLen: UInt32
        switch tag {
        case 0: maxLen = 256 * 1024
        case 1: maxLen = 32 * 1024 * 1024
        default: return nil
        }
        guard len <= maxLen else { return nil }
        let lenInt = Int(len)
        let padded = (lenInt + 3) & ~3
        guard padded == b.count - 8 else { return nil }
        for i in (8 + lenInt)..<b.count where b[i] != 0 { return nil }
        let payload = Array(b[8..<(8 + lenInt)])
        switch tag {
        case 0:
            guard let s = String(bytes: payload, encoding: .utf8) else { return nil }
            self = .url(s)
        case 1: self = .encoded(payload)
        default: return nil
        }
    }
}
