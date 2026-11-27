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

/// Host-side view state over a runtime surface's zero-copy scene: publishes frame changes to
/// SwiftUI, tracks scroll offsets (owned by the runtime, mirrored per frame), and resolves image
/// resources. Scene slabs are never copied: every read borrows the published scene.
public final class NveSceneStore: ObservableObject {
    /// Font size from which an un-annotated leaf text is announced as a heading.
    public static let inferredHeadingFontSize: CGFloat = 20

    @Published public private(set) var frameId: UInt64 = 0
    public private(set) var scrollOffsets: [NodeId: CGPoint] = [:]
    public private(set) var lastDirtyCount = 0
    public private(set) var lastRemovedCount = 0

    public let session: NveRuntimeSession
    public let imageCache: NveImageCache
    private let sparseIndex = NveSparseRowIndex()
    private var imageSources: [UInt32: NveImageSource] = [:]

    public init(session: NveRuntimeSession, imageCache: NveImageCache = .shared) {
        self.session = session
        self.imageCache = imageCache
    }

    /// Runs `body` over the published scene; `nil` when no scene is published yet.
    public func withTables<R>(_ body: (NveRuntimeTables) -> R) -> R? {
        let result: R?? = try? session.withScene { scene in
            scene.nodeCapacity > 1 ? body(NveRuntimeTables(scene: scene, index: sparseIndex)) : nil
        }
        return result ?? nil
    }

    /// Absorbs a newly published frame (call after `tick` reported a change).
    public func sync() {
        var scrollNodes: [NodeId] = []
        let synced: Bool = (try? session.withScene { scene in
            let tables = NveRuntimeTables(scene: scene, index: sparseIndex)
            for raw in scene.removedIds { scrollOffsets.removeValue(forKey: NodeId(raw: raw)) }
            if let image = tables.image, !image.dirtyRows.isEmpty || !scene.removedIds.isEmpty {
                imageSources.removeAll(keepingCapacity: true)
            }
            if let scroll = tables.scroll {
                for r in 0..<scroll.rowCount {
                    guard let owner = scroll.owner(r) else { continue }
                    let id = scroll.isSparse ? NodeId(raw: owner) : scene.nodeId(at: Int(owner))
                    if let id, scene.index(of: id) != nil { scrollNodes.append(id) }
                }
            }
            lastDirtyCount = scene.dirtyIndices.count
            lastRemovedCount = scene.removedIds.count
            frameId = scene.frameId
            return true
        }) ?? false
        guard synced else { return }
        // The runtime owns scroll offsets (incl. programmatic `requested_scroll_*`); a zero delta
        // reads the clamped current value once the scene borrow is released.
        for id in scrollNodes {
            if let offset = try? session.scroll(id, by: .zero) { scrollOffsets[id] = offset }
        }
    }

    public func scrollOffset(for id: NodeId) -> CGPoint { scrollOffsets[id] ?? .zero }

    /// Scrolls `id` by `delta` (clamped by the runtime) and returns the new offset.
    @discardableResult
    public func scroll(_ id: NodeId, by delta: CGVector) -> CGPoint? {
        guard let offset = try? session.scroll(id, by: delta) else { return nil }
        scrollOffsets[id] = offset
        objectWillChange.send()
        return offset
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

    /// Decoded `ImageSrc` of resource `id`, cached until an `nve.image` row changes.
    public func imageSource(_ id: UInt32) -> NveImageSource? {
        if let cached = imageSources[id] { return cached }
        let bytes = try? session.withScene { try session.resource(id, in: $0).bytes }
        guard let bytes, let src = NveImageSource(resourceBytes: bytes) else { return nil }
        imageSources[id] = src
        return src
    }

    /// `CGImage` for an `nve.image` resource.
    public func image(resource id: UInt32) -> CGImage? {
        switch imageSource(id) {
        case let .url(url): return imageCache.image(for: url)
        case let .encoded(bytes): return imageCache.image(encoded: bytes, key: "nve.res.image_src#\(Xxh64.hash(bytes))")
        case nil: return nil
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

/// Decoded `nve.res.image_src` resource (`ImageSrc` wire encoding).
public enum NveImageSource: Equatable, Sendable {
    case url(String)
    case encoded([UInt8])

    /// Tag `0` = URL (`u32` LE length + UTF-8), tag `1` = encoded bytes (`u32` LE length + bytes).
    public init?(resourceBytes b: [UInt8]) {
        guard b.count >= 5 else { return nil }
        let len = Int(UInt32(b[1]) | UInt32(b[2]) << 8 | UInt32(b[3]) << 16 | UInt32(b[4]) << 24)
        guard len == b.count - 5 else { return nil }
        let payload = Array(b[5...])
        switch b[0] {
        case 0:
            guard let s = String(bytes: payload, encoding: .utf8) else { return nil }
            self = .url(s)
        case 1: self = .encoded(payload)
        default: return nil
        }
    }
}
