import CoreGraphics
import CoreText
import CNveAbi
import Foundation

/// Native glyph metrics for runtime layout: the system font at the node's size / weight / italic
/// (exactly what `NveSceneRenderer` draws: it does not paint family, letter spacing, line
/// height, or max lines yet, so measuring them would break paint parity), wrapped to `maxWidth`
/// with CoreText. The runtime already passes `.infinity` for `nowrap` / `pre`.
public final class NveCoreTextMeasurer: NveTextMeasuring {
    public init() {}

    /// Fonts by `(size bits, weight, italic)`: layout measures every text node with a handful of styles.
    /// Bounded: animated or arbitrary sizes must not grow it without limit.
    private var fonts: [UInt64: CTFont] = [:]
    static let maxCachedFonts = 128
    private let lock = NSLock()

    func font(size: CGFloat, weight: UInt16, italic: Bool) -> CTFont? {
        let key = UInt64(Float(size).bitPattern) << 17 | UInt64(weight) << 1 | (italic ? 1 : 0)
        lock.lock()
        defer { lock.unlock() }
        if let cached = fonts[key] { return cached }
        guard let system = CTFontCreateUIFontForLanguage(.system, size, nil) else { return nil }
        var traits: [CFString: Any] = [kCTFontWeightTrait: Self.weightTrait(weight)]
        if italic { traits[kCTFontSymbolicTrait] = CTFontSymbolicTraits.traitItalic.rawValue }
        let descriptor = CTFontDescriptorCreateCopyWithAttributes(
            CTFontCopyFontDescriptor(system), [kCTFontTraitsAttribute: traits] as CFDictionary)
        let font = CTFontCreateWithFontDescriptor(descriptor, size, nil)
        if fonts.count >= Self.maxCachedFonts { fonts.removeAll(keepingCapacity: true) }
        fonts[key] = font
        return font
    }

    public func measure(text: String, style: NveRuntimeTextStyle, maxWidth: CGFloat) -> CGSize {
        let fontSize = CGFloat(style.font_size)
        guard !text.isEmpty, fontSize > 0, fontSize.isFinite,
              let font = font(size: fontSize, weight: style.font_weight, italic: style.isItalic) else {
            return .zero
        }
        let attributed = CFAttributedStringCreate(nil, text as CFString, [kCTFontAttributeName: font] as CFDictionary)!
        let framesetter = CTFramesetterCreateWithAttributedString(attributed)
        // A non-positive width (collapsed box) wraps nothing rather than one glyph per line.
        let width = maxWidth.isFinite && maxWidth > 0 ? maxWidth : .greatestFiniteMagnitude
        let constraint = CGSize(width: width, height: .greatestFiniteMagnitude)
        let size = CTFramesetterSuggestFrameSizeWithConstraints(framesetter, CFRange(location: 0, length: 0), nil, constraint, nil)
        return CGSize(width: ceil(size.width), height: ceil(size.height))
    }

    /// CSS `100...900` → CoreText weight trait (`-1...1`).
    static func weightTrait(_ weight: UInt16) -> CGFloat {
        switch weight {
        case ..<150: return -0.8
        case ..<250: return -0.6
        case ..<350: return -0.4
        case ..<450: return 0
        case ..<550: return 0.23
        case ..<650: return 0.3
        case ..<750: return 0.4
        case ..<850: return 0.56
        default: return 0.62
        }
    }
}
