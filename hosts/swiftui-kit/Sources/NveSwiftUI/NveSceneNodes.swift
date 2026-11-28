import CoreGraphics
import Foundation
import NveHost

/// Packed `RGBA8888` (`r | g<<8 | b<<16 | a<<24`) as a `CGColor`-ready tuple; `nil` when transparent.
public struct NveColor: Equatable, Sendable {
    public let red: Double, green: Double, blue: Double, alpha: Double

    public init?(_ rgba: Rgba?) {
        guard let rgba, rgba.a > 0 else { return nil }
        red = Double(rgba.r) / 255
        green = Double(rgba.g) / 255
        blue = Double(rgba.b) / 255
        alpha = Double(rgba.a) / 255
    }
}

/// Inherited text style resolved from `nve.typography` rows along the parent chain.
public struct NveTextStyle: Equatable, Sendable {
    public var color: NveColor?
    public var fontSize: CGFloat
    public var fontWeight: UInt16
    public var italic: Bool
    public var align: NveTextAlign
    public var maxLines: Int
    public var decoration: UInt32
}

/// The published core tables of one borrowed scene, read through the generated `NveTables` specs.
public struct NveRuntimeTables {
    public let scene: NveScene
    public let visual, typography, text, image, scroll, accessibility, textInput, link, interaction: TableReader?

    public init(scene: NveScene) {
        self.scene = scene
        visual = scene.table(key: NveTables.Visual.key)
        typography = scene.table(key: NveTables.Typography.key)
        text = scene.table(key: NveTables.Text.key)
        image = scene.table(key: NveTables.Image.key)
        scroll = scene.table(key: NveTables.Scroll.key)
        accessibility = scene.table(key: NveTables.Accessibility.key)
        textInput = scene.table(key: NveTables.TextInput.key)
        link = scene.table(key: NveTables.Link.key)
        interaction = scene.table(key: NveTables.Interaction.key)
    }

    /// Row of node slot `index` in `table`, O(1): only when the node's `table_set` holds the table
    /// (unattached dense rows are zeros, not defaults); sparse rows via the published row index.
    public func row(_ table: TableReader?, _ index: Int) -> Int? {
        table.flatMap { scene.tableRow($0, forNode: index) }
    }

    public func string(_ table: TableReader?, _ index: Int, _ field: FieldSpec) -> String? {
        guard let r = row(table, index), let span = table?.str(r, field) else { return nil }
        return scene.text(span)
    }

    public func rect(_ index: Int) -> CGRect {
        guard let r = scene.rect(index) else { return .zero }
        return CGRect(x: CGFloat(r.x), y: CGFloat(r.y), width: CGFloat(r.width), height: CGFloat(r.height))
    }

    // MARK: Visual

    public func background(_ i: Int) -> NveColor? { NveColor(visualField(i) { $0.color($1, NveTables.Visual.background) }) }
    public func borderColor(_ i: Int) -> NveColor? { NveColor(visualField(i) { $0.color($1, NveTables.Visual.borderColor) }) }
    public func borderWidth(_ i: Int) -> CGFloat { CGFloat(max(0, visualField(i) { $0.f32($1, NveTables.Visual.borderWidth) } ?? 0)) }
    public func borderRadius(_ i: Int) -> CGFloat { CGFloat(max(0, visualField(i) { $0.f32($1, NveTables.Visual.borderRadius) } ?? 0)) }
    public func opacity(_ i: Int) -> Double { Double(min(1, max(0, visualField(i) { $0.f32($1, NveTables.Visual.opacity) } ?? 1))) }
    public func zIndex(_ i: Int) -> Int32 { visualField(i) { $0.i32($1, NveTables.Visual.zIndex) } ?? 0 }

    public func visibility(_ i: Int) -> NveVisibility {
        NveVisibility(rawValue: visualField(i) { $0.enumValue($1, NveTables.Visual.visibility) } ?? 0) ?? .inherit
    }

    /// `overflow != visible` on either axis, or a scroll container.
    public func clipsChildren(_ i: Int) -> Bool {
        if isScroll(i) { return true }
        let x = visualField(i) { $0.enumValue($1, NveTables.Visual.overflowX) } ?? 0
        let y = visualField(i) { $0.enumValue($1, NveTables.Visual.overflowY) } ?? 0
        return x != NveOverflow.visible.rawValue || y != NveOverflow.visible.rawValue
    }

    private func visualField<T>(_ i: Int, _ read: (TableReader, Int) -> T?) -> T? {
        guard let visual, let r = row(visual, i) else { return nil }
        return read(visual, r)
    }

    // MARK: Content

    public func text(_ i: Int) -> String? { string(text, i, NveTables.Text.text) }
    public func isScroll(_ i: Int) -> Bool { row(scroll, i) != nil }
    public func isTextInput(_ i: Int) -> Bool { row(textInput, i) != nil }
    public func isImage(_ i: Int) -> Bool { row(image, i) != nil }
    public func isLink(_ i: Int) -> Bool { row(link, i) != nil }
    public func isInteractive(_ i: Int) -> Bool { row(interaction, i) != nil || isLink(i) }

    public func interactionEvents(_ i: Int) -> UInt32 {
        interaction.map { scene.eventSet($0, forNode: i, eventsWord: NveTables.Interaction.eventsWord) } ?? 0
    }

    public func hasInteractionEvent(_ i: Int, bit: UInt32) -> Bool {
        interaction.map { scene.hasEvent($0, forNode: i, eventsWord: NveTables.Interaction.eventsWord, bit: bit) } ?? false
    }

    public func imageSource(_ i: Int) -> (resource: UInt32, fit: NveImageFit, tint: NveColor?)? {
        guard let image, let r = row(image, i), let src = image.resource(r, NveTables.Image.src), src != 0 else { return nil }
        let fit = NveImageFit(rawValue: image.enumValue(r, NveTables.Image.fit) ?? 0) ?? .cover
        let tint = NveColor(image.color(r, NveTables.Image.tint))
        return (src, fit, tint)
    }

    public func textInput(_ i: Int) -> (value: String, placeholder: String, secure: Bool)? {
        guard let textInput, let r = row(textInput, i) else { return nil }
        return (
            string(textInput, i, NveTables.TextInput.value) ?? "",
            string(textInput, i, NveTables.TextInput.placeholder) ?? "",
            textInput.bool(r, NveTables.TextInput.secure) == true
        )
    }

    // MARK: Typography (inherited)

    /// Resolves every typography field from the nearest ancestor that sets it explicitly.
    public func textStyle(_ i: Int) -> NveTextStyle {
        let defaults = NveTables.Typography.defaults
        let fallback = { (f: FieldSpec) in f.decodeBits(from: defaults) ?? 0 }
        var style = NveTextStyle(
            color: NveColor(Rgba(packed: fallback(NveTables.Typography.color))),
            fontSize: CGFloat(Float(bitPattern: fallback(NveTables.Typography.fontSize))),
            fontWeight: UInt16(truncatingIfNeeded: fallback(NveTables.Typography.fontWeight)),
            italic: false, align: .start, maxLines: 0, decoration: 0)
        guard let typography else { return style }
        scene.forEachInheritedRow(typography, fromNode: i) { r, fresh in
            func take(_ field: FieldSpec, _ apply: () -> Void) { if fresh & field.explicitMask != 0 { apply() } }
            take(NveTables.Typography.color) { style.color = NveColor(typography.color(r, NveTables.Typography.color)) }
            take(NveTables.Typography.fontSize) {
                if let s = typography.f32(r, NveTables.Typography.fontSize), s.isFinite, s > 0 { style.fontSize = CGFloat(s) }
            }
            take(NveTables.Typography.fontWeight) {
                style.fontWeight = UInt16(truncatingIfNeeded: typography.u32(r, NveTables.Typography.fontWeight) ?? 400)
            }
            take(NveTables.Typography.fontStyle) {
                style.italic = (typography.enumValue(r, NveTables.Typography.fontStyle) ?? 0) != NveFontStyle.normal.rawValue
            }
            take(NveTables.Typography.textAlign) {
                style.align = NveTextAlign(rawValue: typography.enumValue(r, NveTables.Typography.textAlign) ?? 0) ?? .start
            }
            take(NveTables.Typography.maxLines) { style.maxLines = Int(typography.u32(r, NveTables.Typography.maxLines) ?? 0) }
            take(NveTables.Typography.textDecoration) {
                style.decoration = typography.u32(r, NveTables.Typography.textDecoration) ?? 0
            }
        }
        return style
    }

    // MARK: Accessibility

    public func accessibility(_ i: Int) -> (role: NveAriaRole?, level: Int, label: String?, hint: String?, value: String?, hidden: Bool)? {
        guard let accessibility, let r = row(accessibility, i) else { return nil }
        return (
            accessibility.enumValue(r, NveTables.Accessibility.role).flatMap(NveAriaRole.init(rawValue:)),
            Int(accessibility.u32(r, NveTables.Accessibility.level) ?? 0),
            string(accessibility, i, NveTables.Accessibility.label),
            string(accessibility, i, NveTables.Accessibility.hint),
            string(accessibility, i, NveTables.Accessibility.valueText),
            accessibility.bool(r, NveTables.Accessibility.hidden) == true
        )
    }

    // MARK: Tree

    /// Live child slots of `i` in paint order (stable by `z_index`).
    public func paintOrderedChildren(_ i: Int) -> [Int] {
        guard let ids = scene.children(of: i) else { return [] }
        var out: [Int] = []
        out.reserveCapacity(ids.count)
        var needsSort = false
        var lastZ: Int32 = .min
        for raw in ids {
            guard let c = scene.index(of: NodeId(raw: raw)) else { continue }
            let z = zIndex(c)
            needsSort = needsSort || z < lastZ
            lastZ = max(lastZ, z)
            out.append(c)
        }
        if needsSort {
            out = out.enumerated().sorted { (zIndex($0.element), $0.offset) < (zIndex($1.element), $1.offset) }.map(\.element)
        }
        return out
    }
}
