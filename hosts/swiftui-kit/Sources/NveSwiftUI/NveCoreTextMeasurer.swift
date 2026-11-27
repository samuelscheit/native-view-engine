import CoreGraphics
import CoreText
import Foundation

/// Native glyph metrics for runtime layout: system font at the requested size / weight, wrapped
/// to `maxWidth` with CoreText, so laid-out text boxes match what `NveSceneRenderer` draws.
public final class NveCoreTextMeasurer: NveTextMeasuring {
    public init() {}

    /// Fonts by `(size bits, weight)`: layout measures every text node with a handful of styles.
    private var fonts: [UInt64: CTFont] = [:]
    private let lock = NSLock()

    private func font(size: CGFloat, weight: UInt16) -> CTFont? {
        let key = UInt64(Float(size).bitPattern) << 16 | UInt64(weight)
        lock.lock()
        defer { lock.unlock() }
        if let cached = fonts[key] { return cached }
        guard let system = CTFontCreateUIFontForLanguage(.system, size, nil) else { return nil }
        let descriptor = CTFontDescriptorCreateCopyWithAttributes(
            CTFontCopyFontDescriptor(system),
            [kCTFontTraitsAttribute: [kCTFontWeightTrait: Self.weightTrait(weight)]] as CFDictionary)
        let font = CTFontCreateWithFontDescriptor(descriptor, size, nil)
        fonts[key] = font
        return font
    }

    public func measure(text: String, fontSize: CGFloat, fontWeight: UInt16, maxWidth: CGFloat) -> CGSize {
        guard !text.isEmpty, fontSize > 0, fontSize.isFinite, let font = font(size: fontSize, weight: fontWeight) else {
            return .zero
        }
        let attributed = CFAttributedStringCreate(nil, text as CFString, [kCTFontAttributeName: font] as CFDictionary)!
        let framesetter = CTFramesetterCreateWithAttributedString(attributed)
        let constraint = CGSize(width: maxWidth.isFinite ? maxWidth : .greatestFiniteMagnitude, height: .greatestFiniteMagnitude)
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
