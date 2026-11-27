import CoreGraphics
import CoreText
import Foundation

/// Native glyph metrics for runtime layout: system font at the requested size / weight, wrapped
/// to `maxWidth` with CoreText, so laid-out text boxes match what `NveSceneRenderer` draws.
public final class NveCoreTextMeasurer: NveTextMeasuring {
    public init() {}

    public func measure(text: String, fontSize: CGFloat, fontWeight: UInt16, maxWidth: CGFloat) -> CGSize {
        guard !text.isEmpty, fontSize > 0 else { return .zero }
        guard let system = CTFontCreateUIFontForLanguage(.system, fontSize, nil) else { return .zero }
        let descriptor = CTFontDescriptorCreateCopyWithAttributes(
            CTFontCopyFontDescriptor(system),
            [kCTFontTraitsAttribute: [kCTFontWeightTrait: Self.weightTrait(fontWeight)]] as CFDictionary)
        let font = CTFontCreateWithFontDescriptor(descriptor, fontSize, nil)
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
