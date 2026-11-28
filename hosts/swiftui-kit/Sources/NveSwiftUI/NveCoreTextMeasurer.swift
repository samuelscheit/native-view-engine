import CNveAbi
import CoreGraphics
import CoreText
import Foundation
import NveHost

public extension NveTypographyEngine {
    // MARK: CoreText Measurement

    func measure(text: String, spec: NveTypographySpec, maxWidth: CGFloat) -> CGSize {
        guard !text.isEmpty, spec.fontSize > 0, spec.fontSize.isFinite else { return .zero }
        let wrap = spec.wrap
        let isMinContent = maxWidth == 0
        let probe = isMinContent ? wrap.minContent : wrap
        guard let built = attributedString(text: text, spec: spec, color: nil, forMinContent: isMinContent) else {
            return .zero
        }
        var limit: CGFloat = .greatestFiniteMagnitude
        if probe != .none {
            if isMinContent {
                limit = minContentWidth(text: text, spec: spec, probe: probe)
            } else if maxWidth.isFinite, maxWidth > 0 {
                limit = maxWidth
            }
        }
        return suggestSize(attributed: built.attributed, limit: limit, maxLines: spec.maxLines)
    }

    private func minContentWidth(text: String, spec: NveTypographySpec, probe: NveTextWrap) -> CGFloat {
        var widest: CGFloat = 0
        var singleLineSpec = spec
        singleLineSpec.maxLines = 0
        if probe == .anywhere || probe == .breakAll {
            var seen = Set<Character>()
            for ch in text where !ch.isWhitespace && !ch.isNewline {
                guard seen.insert(ch).inserted else { continue }
                if let built = attributedString(text: String(ch), spec: singleLineSpec, forMinContent: true) {
                    let w = suggestSize(attributed: built.attributed, limit: .greatestFiniteMagnitude, maxLines: 0).width
                    widest = max(widest, w)
                }
            }
        } else {
            var seen = Set<Substring>()
            for word in text.split(whereSeparator: { $0.isWhitespace || $0.isNewline }) where !word.isEmpty {
                guard seen.insert(word).inserted else { continue }
                if let built = attributedString(text: String(word), spec: singleLineSpec, forMinContent: true) {
                    let w = suggestSize(attributed: built.attributed, limit: .greatestFiniteMagnitude, maxLines: 0).width
                    widest = max(widest, w)
                }
            }
        }
        return widest > 0 ? widest : .greatestFiniteMagnitude
    }

    private func suggestSize(attributed: CFAttributedString, limit: CGFloat, maxLines: Int) -> CGSize {
        let framesetter = CTFramesetterCreateWithAttributedString(attributed)
        let boundedWidth = min(limit, 1_000_000)
        var range = CFRange(location: 0, length: 0)
        if maxLines > 0 {
            let path = CGPath(rect: CGRect(x: 0, y: 0, width: boundedWidth, height: 1_000_000), transform: nil)
            let frame = CTFramesetterCreateFrame(framesetter, CFRange(location: 0, length: 0), path, nil)
            let lines = CTFrameGetLines(frame) as! [CTLine]
            if lines.count > maxLines {
                let lastRange = CTLineGetStringRange(lines[maxLines - 1])
                var visibleLen = lastRange.location + lastRange.length
                let ns = CFAttributedStringGetString(attributed) as NSString
                while visibleLen > 0 {
                    let ch = ns.character(at: visibleLen - 1)
                    if ch == 0x000A || ch == 0x000D { visibleLen -= 1 } else { break }
                }
                range = CFRange(location: 0, length: max(0, visibleLen))
            }
        }
        let constraint = CGSize(width: limit, height: .greatestFiniteMagnitude)
        let size = CTFramesetterSuggestFrameSizeWithConstraints(framesetter, range, nil, constraint, nil)
        return CGSize(width: ceil(size.width), height: ceil(size.height))
    }

    // MARK: CoreText CGContext Painting

    /// Paints `text` into a top-down (SwiftUI) `CGContext` within `rect` using the exact same
    /// CoreText font, paragraph style, kerning, line height, wrapping, max lines, and decorations
    /// measured by `measure(text:spec:maxWidth:)`.
    func draw(
        text: String,
        spec: NveTypographySpec,
        color: NveColor,
        in rect: CGRect,
        into cg: CGContext
    ) {
        guard !text.isEmpty, rect.width > 0, rect.height > 0 else { return }
        let cgCol = Self.cgColor(from: color)
        guard let built = attributedString(text: text, spec: spec, color: cgCol, forMinContent: false) else { return }
        let framesetter = CTFramesetterCreateWithAttributedString(built.attributed)
        let noWrap = spec.wrap == .none
        let layoutWidth: CGFloat = noWrap ? 1_000_000 : rect.width
        let frameH: CGFloat = max(rect.height, 1_000_000)
        let yShift = rect.height - frameH
        let path = CGPath(rect: CGRect(x: 0, y: 0, width: layoutWidth, height: frameH), transform: nil)
        let frame = CTFramesetterCreateFrame(framesetter, CFRange(location: 0, length: 0), path, nil)
        let lines = CTFrameGetLines(frame) as! [CTLine]
        guard !lines.isEmpty else { return }

        let visibleCount = spec.maxLines > 0 ? min(lines.count, spec.maxLines) : lines.count
        var origins = [CGPoint](repeating: .zero, count: visibleCount)
        CTFrameGetLineOrigins(frame, CFRange(location: 0, length: visibleCount), &origins)

        cg.saveGState()
        defer { cg.restoreGState() }
        cg.clip(to: rect)
        cg.translateBy(x: rect.minX, y: rect.maxY)
        cg.scaleBy(x: 1, y: -1)
        cg.textMatrix = .identity

        let alignFrac: CGFloat = switch spec.align {
        case .center: 0.5
        case .right, .end: 1.0
        default: 0.0
        }
        let strokeLineThrough = (spec.decoration & NveTextDecoration.lineThrough) != 0
        let strokeOverline = (spec.decoration & NveTextDecoration.overline) != 0
        let xHeight = CTFontGetXHeight(built.font)
        let thickness = max(1, CTFontGetUnderlineThickness(built.font))

        for idx in 0..<visibleCount {
            var line = lines[idx]
            if idx == visibleCount - 1, spec.maxLines > 0, lines.count > spec.maxLines {
                line = truncatedLastLine(
                    attributed: built.attributed,
                    line: line,
                    font: built.font,
                    color: cgCol,
                    maxWidth: rect.width
                )
            }
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            var leading: CGFloat = 0
            let lineWidth = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
            let originX = noWrap ? max(0, rect.width - lineWidth) * alignFrac : origins[idx].x
            let originY = origins[idx].y + yShift
            cg.textPosition = CGPoint(x: originX, y: originY)
            CTLineDraw(line, cg)

            if (strokeLineThrough || strokeOverline) && lineWidth > 0 {
                cg.saveGState()
                cg.setStrokeColor(cgCol)
                cg.setLineWidth(thickness)
                if strokeLineThrough {
                    let y = originY + xHeight * 0.45
                    cg.move(to: CGPoint(x: originX, y: y))
                    cg.addLine(to: CGPoint(x: originX + lineWidth, y: y))
                    cg.strokePath()
                }
                if strokeOverline {
                    // Keep the entire stroke inside `rect` (avoid half-stroke clip at `rect.minY`).
                    let y = min(rect.height - thickness * 0.5, originY + ascent - thickness * 0.5)
                    cg.move(to: CGPoint(x: originX, y: y))
                    cg.addLine(to: CGPoint(x: originX + lineWidth, y: y))
                    cg.strokePath()
                }
                cg.restoreGState()
            }
        }
    }

    /// Builds an unconstrained line from the remaining substring starting at `line` and truncates it
    /// to `maxWidth` with an ellipsis (`…`), so wrapped multi-line overflow genuinely truncates
    /// (instead of being a no-op on an already-wrapped `CTLine` whose width is `<= maxWidth`).
    private func truncatedLastLine(
        attributed: CFAttributedString,
        line: CTLine,
        font: CTFont,
        color: CGColor,
        maxWidth: CGFloat
    ) -> CTLine {
        let totalLen = CFAttributedStringGetLength(attributed)
        let lineRange = CTLineGetStringRange(line)
        let start = lineRange.location
        guard start >= 0, start < totalLen else { return line }
        let fullStr = CFAttributedStringGetString(attributed) as NSString
        let tail = fullStr.substring(with: NSRange(location: start, length: totalLen - start))
        let attrs = CFAttributedStringGetAttributes(attributed, start, nil)
        let tokenAttrs: [CFString: Any] = [kCTFontAttributeName: font, kCTForegroundColorAttributeName: color]
        let tokenStr = CFAttributedStringCreate(nil, "\u{2026}" as CFString, tokenAttrs as CFDictionary)!
        let tokenLine = CTLineCreateWithAttributedString(tokenStr)

        // If a hard newline (`\n`, `\r`, or Swift CRLF grapheme `\r\n`) caused the overflow on this
        // line, append `…` at the break so even a short first line visibly truncates.
        if let brk = tail.firstIndex(where: \.isNewline) {
            let prefix = String(tail[..<brk]) + "\u{2026}"
            let attrPrefix = CFAttributedStringCreate(nil, prefix as CFString, attrs)!
            let candidate = CTLineCreateWithAttributedString(attrPrefix)
            return CTLineCreateTruncatedLine(candidate, Double(maxWidth), .end, tokenLine) ?? candidate
        }
        let attrTail = CFAttributedStringCreate(nil, tail as CFString, attrs)!
        let unconstrained = CTLineCreateWithAttributedString(attrTail)
        if let truncated = CTLineCreateTruncatedLine(unconstrained, Double(maxWidth), .end, tokenLine) {
            return truncated
        }
        let forced = CFAttributedStringCreate(nil, (tail + "\u{2026}") as CFString, attrs)!
        let forcedLine = CTLineCreateWithAttributedString(forced)
        return CTLineCreateTruncatedLine(forcedLine, Double(maxWidth), .end, tokenLine) ?? forcedLine
    }
}

/// Native glyph metrics for runtime layout backed by `NveTypographyEngine` so `font_family`,
/// `letter_spacing`, `line_height`, `max_lines`, and CSS wrapping modes (`white_space`,
/// `word_break`, `overflow_wrap`) match `NveSceneRenderer` identically.
public final class NveCoreTextMeasurer: NveTextMeasuring {
    private let engine: NveTypographyEngine

    public init(engine: NveTypographyEngine = .shared) {
        self.engine = engine
    }

    func font(size: CGFloat, weight: UInt16, italic: Bool, family: String = "") -> CTFont? {
        engine.font(size: size, weight: weight, italic: italic, family: family)
    }

    public func measure(text: String, style: NveRuntimeTextStyle, maxWidth: CGFloat) -> CGSize {
        engine.measure(text: text, spec: NveTypographySpec(runtimeStyle: style), maxWidth: maxWidth)
    }

    /// CSS `100...900` → CoreText weight trait (`-1...1`).
    static func weightTrait(_ weight: UInt16) -> CGFloat {
        NveTypographyEngine.weightTrait(weight)
    }
}
