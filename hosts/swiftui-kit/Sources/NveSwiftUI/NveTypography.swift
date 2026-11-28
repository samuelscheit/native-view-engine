import CNveAbi
import CoreGraphics
import CoreText
import Foundation
import NveHost

#if canImport(AppKit)
import AppKit
#elseif canImport(UIKit)
import UIKit
#endif

/// Resolved CSS line-wrapping mode (mirrors `nve::host::TextWrap` in `host-cxx`).
public enum NveTextWrap: Equatable, Sendable {
    case none
    case word
    case breakWord
    case anywhere
    case breakAll

    public static func resolve(
        whiteSpace: NveWhiteSpace,
        wordBreak: NveWordBreak,
        overflowWrap: NveOverflowWrap
    ) -> NveTextWrap {
        if whiteSpace == .noWrap || whiteSpace == .pre { return .none }
        if wordBreak == .breakAll { return .breakAll }
        if wordBreak == .breakWord || overflowWrap == .anywhere { return .anywhere }
        if overflowWrap == .breakWord { return .breakWord }
        return .word
    }

    public var minContent: NveTextWrap {
        self == .breakWord ? .word : self
    }
}

/// Complete typography specification shared by both `NveCoreTextMeasurer` and `NveSceneRenderer`
/// so layout measurement and painted glyphs stay strictly in parity.
public struct NveTypographySpec: Equatable, Sendable {
    public var fontFamily: String
    public var fontSize: CGFloat
    public var fontWeight: UInt16
    public var italic: Bool
    public var letterSpacing: CGFloat
    public var lineHeightUnit: LengthValue.Unit
    public var lineHeight: Float
    public var maxLines: Int
    public var align: NveTextAlign
    public var decoration: UInt32
    public var whiteSpace: NveWhiteSpace
    public var wordBreak: NveWordBreak
    public var overflowWrap: NveOverflowWrap

    public init(
        fontFamily: String = "",
        fontSize: CGFloat = 16,
        fontWeight: UInt16 = 400,
        italic: Bool = false,
        letterSpacing: CGFloat = 0,
        lineHeightUnit: LengthValue.Unit = .auto,
        lineHeight: Float = 0,
        maxLines: Int = 0,
        align: NveTextAlign = .start,
        decoration: UInt32 = 0,
        whiteSpace: NveWhiteSpace = .normal,
        wordBreak: NveWordBreak = .normal,
        overflowWrap: NveOverflowWrap = .normal
    ) {
        self.fontFamily = fontFamily
        self.fontSize = fontSize
        self.fontWeight = fontWeight
        self.italic = italic
        self.letterSpacing = letterSpacing
        self.lineHeightUnit = lineHeightUnit
        self.lineHeight = lineHeight
        self.maxLines = maxLines
        self.align = align
        self.decoration = decoration
        self.whiteSpace = whiteSpace
        self.wordBreak = wordBreak
        self.overflowWrap = overflowWrap
    }

    public init(runtimeStyle s: NveRuntimeTextStyle) {
        let family = s.font_family.ptr.map {
            String(decoding: UnsafeBufferPointer(start: $0, count: s.font_family.len), as: UTF8.self)
        } ?? ""
        let lhUnit: LengthValue.Unit = switch UInt32(s.line_height_unit) {
        case NVE_LINE_HEIGHT_POINTS: .points
        case NVE_LINE_HEIGHT_PERCENT: .percent
        default: .auto
        }
        self.init(
            fontFamily: family,
            fontSize: CGFloat(s.font_size),
            fontWeight: s.font_weight,
            italic: s.isItalic,
            letterSpacing: s.letter_spacing.isFinite ? CGFloat(s.letter_spacing) : 0,
            lineHeightUnit: lhUnit,
            lineHeight: s.line_height.isFinite ? s.line_height : 0,
            maxLines: Int(s.max_lines),
            align: .start,
            decoration: 0,
            whiteSpace: NveWhiteSpace(rawValue: UInt32(s.white_space)) ?? .normal,
            wordBreak: NveWordBreak(rawValue: UInt32(s.word_break)) ?? .normal,
            overflowWrap: NveOverflowWrap(rawValue: UInt32(s.overflow_wrap)) ?? .normal
        )
    }

    public init(textStyle s: NveTextStyle) {
        self.init(
            fontFamily: s.fontFamily,
            fontSize: s.fontSize,
            fontWeight: s.fontWeight,
            italic: s.italic,
            letterSpacing: s.letterSpacing,
            lineHeightUnit: s.lineHeight.unit,
            lineHeight: s.lineHeight.value,
            maxLines: s.maxLines,
            align: s.align,
            decoration: s.decoration,
            whiteSpace: s.whiteSpace,
            wordBreak: s.wordBreak,
            overflowWrap: s.overflowWrap
        )
    }

    public var wrap: NveTextWrap {
        NveTextWrap.resolve(whiteSpace: whiteSpace, wordBreak: wordBreak, overflowWrap: overflowWrap)
    }

    /// Absolute line height in points (`0` for `normal` / `auto`).
    public var lineHeightPoints: CGFloat {
        guard lineHeight.isFinite, lineHeight > 0 else { return 0 }
        switch lineHeightUnit {
        case .points: return CGFloat(lineHeight)
        case .percent: return CGFloat(lineHeight) / 100.0 * fontSize
        case .auto: return 0
        }
    }
}

/// Shared CoreText font resolution and attributed-string builder (measurement and CGContext
/// painting live in `NveCoreTextMeasurer.swift`).
public final class NveTypographyEngine: @unchecked Sendable {
    public static let shared = NveTypographyEngine()
    static let maxCachedFonts = 128

    private struct FontKey: Hashable {
        let family: String
        let sizeBits: UInt32
        let weight: UInt16
        let italic: Bool
    }

    private enum GenericFamily {
        case serif, sansSerif, monospace, cursive, fantasy, systemUi
    }

    private var fonts: [FontKey: CTFont] = [:]
    private let lock = NSLock()
    internal static let srgbSpace = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()

    public init() {}

    /// Parses a CSS `font-family` list identically to `nve::host::for_each_font_family`.
    public static func forEachFontFamily(_ list: String, _ visit: (Substring, Bool) -> Bool) {
        var idx = list.startIndex
        let end = list.endIndex
        let isSpace: (Character) -> Bool = { $0 == " " || $0 == "\t" }
        while idx < end {
            while idx < end && isSpace(list[idx]) { idx = list.index(after: idx) }
            guard idx < end else { return }
            let name: Substring
            let quoted: Bool
            if list[idx] == "\"" || list[idx] == "'" {
                let quote = list[idx]
                let start = list.index(after: idx)
                let close = list[start..<end].firstIndex(of: quote) ?? end
                name = list[start..<close]
                quoted = true
                idx = close
            } else {
                let comma = list[idx..<end].firstIndex(of: ",") ?? end
                var trimmed = list[idx..<comma]
                while let last = trimmed.last, isSpace(last) { trimmed = trimmed.dropLast() }
                name = trimmed
                quoted = false
                idx = comma
            }
            if quoted {
                idx = list[idx..<end].firstIndex(of: ",") ?? end
            }
            if idx < end && list[idx] == "," { idx = list.index(after: idx) }
            if !name.isEmpty && !visit(name, quoted) { return }
        }
    }

    private static func genericFamily(_ name: Substring) -> GenericFamily? {
        switch name.lowercased() {
        case "serif", "ui-serif": return .serif
        case "sans-serif", "ui-sans-serif": return .sansSerif
        case "monospace", "ui-monospace": return .monospace
        case "cursive": return .cursive
        case "fantasy": return .fantasy
        case "system-ui": return .systemUi
        default: return nil
        }
    }

    /// CSS `100...900` → Apple/CoreText weight trait (`-0.8...0.62`).
    public static func weightTrait(_ weight: UInt16) -> CGFloat {
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

    public func font(size: CGFloat, weight: UInt16, italic: Bool, family: String = "") -> CTFont? {
        guard size.isFinite, size > 0 else { return nil }
        let key = FontKey(family: family, sizeBits: Float(size).bitPattern, weight: weight, italic: italic)
        lock.lock()
        if let cached = fonts[key] {
            lock.unlock()
            return cached
        }
        lock.unlock()

        let resolved = resolveFont(family: family, size: size, weight: weight, italic: italic)
        if let resolved {
            lock.lock()
            if fonts.count >= Self.maxCachedFonts { fonts.removeAll(keepingCapacity: true) }
            fonts[key] = resolved
            lock.unlock()
        }
        return resolved
    }

    private func resolveFont(family: String, size: CGFloat, weight: UInt16, italic: Bool) -> CTFont? {
        var matched: CTFont?
        if !family.isEmpty {
            Self.forEachFontFamily(family) { entry, quoted in
                if !quoted, let generic = Self.genericFamily(entry) {
                    matched = resolveGenericFont(generic, size: size, weight: weight, italic: italic)
                } else {
                    matched = resolveNamedFont(String(entry), size: size, weight: weight, italic: italic)
                }
                return matched == nil
            }
        }
        return matched ?? resolveSystemFont(size: size, weight: weight, italic: italic)
    }

    private func resolveGenericFont(_ generic: GenericFamily, size: CGFloat, weight: UInt16, italic: Bool) -> CTFont? {
        switch generic {
        case .systemUi, .sansSerif:
            return resolveSystemFont(size: size, weight: weight, italic: italic)
        case .monospace:
            return resolveSystemDesignFont(monospaced: true, size: size, weight: weight, italic: italic)
                ?? resolveNamedFont("Menlo", size: size, weight: weight, italic: italic)
                ?? resolveNamedFont("Courier New", size: size, weight: weight, italic: italic)
        case .serif:
            return resolveSystemDesignFont(monospaced: false, size: size, weight: weight, italic: italic)
                ?? resolveNamedFont("Georgia", size: size, weight: weight, italic: italic)
                ?? resolveNamedFont("Times New Roman", size: size, weight: weight, italic: italic)
        case .cursive:
            return resolveNamedFont("Snell Roundhand", size: size, weight: weight, italic: italic)
                ?? resolveNamedFont("Bradley Hand", size: size, weight: weight, italic: italic)
                ?? resolveNamedFont("Apple Chancery", size: size, weight: weight, italic: italic)
        case .fantasy:
            return resolveNamedFont("Papyrus", size: size, weight: weight, italic: italic)
        }
    }

    private func resolveSystemFont(size: CGFloat, weight: UInt16, italic: Bool) -> CTFont? {
        guard let system = CTFontCreateUIFontForLanguage(.system, size, nil) else { return nil }
        return applyTraits(to: system, size: size, weight: weight, italic: italic)
    }

    private func resolveSystemDesignFont(monospaced: Bool, size: CGFloat, weight: UInt16, italic: Bool) -> CTFont? {
        let w = Self.weightTrait(weight)
        #if canImport(AppKit)
        let base: NSFont
        if monospaced {
            base = NSFont.monospacedSystemFont(ofSize: size, weight: NSFont.Weight(rawValue: w))
        } else if let desc = NSFont.systemFont(ofSize: size, weight: NSFont.Weight(rawValue: w))
            .fontDescriptor.withDesign(.serif),
                  let f = NSFont(descriptor: desc, size: size)
        {
            base = f
        } else {
            return nil
        }
        return applyItalicIfNeeded(base as CTFont, size: size, italic: italic)
        #elseif canImport(UIKit)
        let base: UIFont
        if monospaced {
            base = UIFont.monospacedSystemFont(ofSize: size, weight: UIFont.Weight(rawValue: w))
        } else if let desc = UIFont.systemFont(ofSize: size, weight: UIFont.Weight(rawValue: w))
            .fontDescriptor.withDesign(.serif)
        {
            base = UIFont(descriptor: desc, size: size)
        } else {
            return nil
        }
        return applyItalicIfNeeded(base as CTFont, size: size, italic: italic)
        #else
        return nil
        #endif
    }

    private func resolveNamedFont(_ name: String, size: CGFloat, weight: UInt16, italic: Bool) -> CTFont? {
        let baseDesc = CTFontDescriptorCreateWithAttributes([kCTFontFamilyNameAttribute: name] as CFDictionary)
        let mandatory = NSSet(object: kCTFontFamilyNameAttribute) as CFSet
        guard let matched = CTFontDescriptorCreateMatchingFontDescriptor(baseDesc, mandatory) else {
            return nil
        }
        let base = CTFontCreateWithFontDescriptor(matched, size, nil)
        return applyTraits(to: base, size: size, weight: weight, italic: italic)
    }

    private func applyTraits(to base: CTFont, size: CGFloat, weight: UInt16, italic: Bool) -> CTFont {
        var traits: [CFString: Any] = [kCTFontWeightTrait: Self.weightTrait(weight)]
        if italic { traits[kCTFontSymbolicTrait] = CTFontSymbolicTraits.traitItalic.rawValue }
        let desc = CTFontDescriptorCreateCopyWithAttributes(
            CTFontCopyFontDescriptor(base),
            [kCTFontTraitsAttribute: traits] as CFDictionary
        )
        let font = CTFontCreateWithFontDescriptor(desc, size, nil)
        return applyItalicIfNeeded(font, size: size, italic: italic)
    }

    private func applyItalicIfNeeded(_ font: CTFont, size: CGFloat, italic: Bool) -> CTFont {
        guard italic, !CTFontGetSymbolicTraits(font).contains(.traitItalic) else { return font }
        return CTFontCreateCopyWithSymbolicTraits(font, size, nil, .traitItalic, .traitItalic) ?? font
    }

    // MARK: Attributed String & Paragraph Style

    public func makeParagraphStyle(spec: NveTypographySpec, forMinContent: Bool = false) -> CTParagraphStyle {
        let wrap = forMinContent ? spec.wrap.minContent : spec.wrap
        var lineBreak: CTLineBreakMode = switch wrap {
        case .none: .byClipping
        case .breakAll: .byCharWrapping
        case .anywhere: forMinContent ? .byCharWrapping : .byWordWrapping
        case .word, .breakWord: .byWordWrapping
        }
        var alignment: CTTextAlignment = switch spec.align {
        case .center: .center
        case .right, .end: .right
        case .left: .left
        case .justify: .justified
        case .start: .natural
        }
        var minLH = spec.lineHeightPoints
        var maxLH = spec.lineHeightPoints
        let hasLineHeight = minLH > 0
        var settings: [CTParagraphStyleSetting] = []
        return withUnsafePointer(to: &lineBreak) { lbPtr in
            withUnsafePointer(to: &alignment) { alignPtr in
                withUnsafePointer(to: &minLH) { minPtr in
                    withUnsafePointer(to: &maxLH) { maxPtr in
                        settings.append(CTParagraphStyleSetting(
                            spec: .lineBreakMode, valueSize: MemoryLayout<CTLineBreakMode>.size, value: lbPtr))
                        settings.append(CTParagraphStyleSetting(
                            spec: .alignment, valueSize: MemoryLayout<CTTextAlignment>.size, value: alignPtr))
                        if hasLineHeight {
                            settings.append(CTParagraphStyleSetting(
                                spec: .minimumLineHeight, valueSize: MemoryLayout<CGFloat>.size, value: minPtr))
                            settings.append(CTParagraphStyleSetting(
                                spec: .maximumLineHeight, valueSize: MemoryLayout<CGFloat>.size, value: maxPtr))
                        }
                        return CTParagraphStyleCreate(settings, settings.count)
                    }
                }
            }
        }
    }

    public func attributedString(
        text: String,
        spec: NveTypographySpec,
        color: CGColor? = nil,
        forMinContent: Bool = false
    ) -> (attributed: CFAttributedString, font: CTFont)? {
        guard let font = font(size: spec.fontSize, weight: spec.fontWeight, italic: spec.italic, family: spec.fontFamily) else {
            return nil
        }
        let para = makeParagraphStyle(spec: spec, forMinContent: forMinContent)
        var attrs: [CFString: Any] = [
            kCTFontAttributeName: font,
            kCTParagraphStyleAttributeName: para,
        ]
        if spec.letterSpacing.isFinite, spec.letterSpacing != 0 {
            attrs[kCTKernAttributeName] = spec.letterSpacing as CFNumber
        }
        if let color {
            attrs[kCTForegroundColorAttributeName] = color
        }
        if (spec.decoration & NveTextDecoration.underline) != 0 {
            attrs[kCTUnderlineStyleAttributeName] = CTUnderlineStyle.single.rawValue as CFNumber
        }
        let attributed = CFAttributedStringCreate(nil, text as CFString, attrs as CFDictionary)!
        return (attributed, font)
    }

    public static func cgColor(from c: NveColor) -> CGColor {
        CGColor(colorSpace: srgbSpace, components: [c.red, c.green, c.blue, c.alpha])
            ?? CGColor(red: c.red, green: c.green, blue: c.blue, alpha: c.alpha)
    }
}
