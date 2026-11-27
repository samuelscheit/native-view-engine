import CoreGraphics
import Foundation
import ImageIO
import NveHost

/// Computed image placement geometry for a destination frame and image pixel size.
public struct NveImagePlacement: Equatable, Sendable {
    /// Cropped region in image pixel coordinates.
    public let sourceRect: CGRect
    /// Visible destination rectangle clamped within the target bounds.
    public let destRect: CGRect
    /// Unclipped draw rectangle for rendering the full `CGImage` inside a clipped `GraphicsContext`.
    public let drawRect: CGRect

    /// Placement for `nve.image.fit` (`NveImageFit`, CSS `object-fit`).
    public static func compute(
        destination: CGRect,
        imageSize: CGSize,
        fit: NveImageFit
    ) -> NveImagePlacement? {
        guard destination.width > 0,
              destination.height > 0,
              imageSize.width > 0,
              imageSize.height > 0
        else {
            return nil
        }

        let iw = imageSize.width
        let ih = imageSize.height
        let dw = destination.width
        let dh = destination.height
        // `scale-down` is `none` or `contain`, whichever yields the smaller image.
        let resolved: NveImageFit = fit == .scaleDown ? (iw > dw || ih > dh ? .contain : .none_) : fit

        switch resolved {
        case .fill:
            return NveImagePlacement(
                sourceRect: CGRect(x: 0, y: 0, width: iw, height: ih),
                destRect: destination,
                drawRect: destination
            )

        case .contain:
            let scale = min(dw / iw, dh / ih)
            let outW = max(0, iw * scale)
            let outH = max(0, ih * scale)
            let fitted = CGRect(
                x: destination.minX + (dw - outW) * 0.5,
                y: destination.minY + (dh - outH) * 0.5,
                width: outW,
                height: outH
            )
            return NveImagePlacement(
                sourceRect: CGRect(x: 0, y: 0, width: iw, height: ih),
                destRect: fitted,
                drawRect: fitted
            )

        case .cover:
            let scale = max(dw / iw, dh / ih)
            let srcW = min(max(0, dw / scale), iw)
            let srcH = min(max(0, dh / scale), ih)
            let scaledW = iw * scale
            let scaledH = ih * scale
            let drawRect = CGRect(
                x: destination.minX + (dw - scaledW) * 0.5,
                y: destination.minY + (dh - scaledH) * 0.5,
                width: scaledW,
                height: scaledH
            )
            return NveImagePlacement(
                sourceRect: CGRect(
                    x: (iw - srcW) * 0.5,
                    y: (ih - srcH) * 0.5,
                    width: srcW,
                    height: srcH
                ),
                destRect: destination,
                drawRect: drawRect
            )

        case .none_, .scaleDown:
            let unclippedX = destination.minX + (dw - iw) * 0.5
            let unclippedY = destination.minY + (dh - ih) * 0.5
            let clipX0 = max(unclippedX, destination.minX)
            let clipY0 = max(unclippedY, destination.minY)
            let clipX1 = min(unclippedX + iw, destination.maxX)
            let clipY1 = min(unclippedY + ih, destination.maxY)
            let visW = clipX1 - clipX0
            let visH = clipY1 - clipY0
            guard visW > 0, visH > 0 else { return nil }
            return NveImagePlacement(
                sourceRect: CGRect(
                    x: min(max(0, clipX0 - unclippedX), iw),
                    y: min(max(0, clipY0 - unclippedY), ih),
                    width: min(visW, iw),
                    height: min(visH, ih)
                ),
                destRect: CGRect(x: clipX0, y: clipY0, width: visW, height: visH),
                drawRect: CGRect(x: unclippedX, y: unclippedY, width: iw, height: ih)
            )
        }
    }
}

/// Thread-safe bounded-LRU `CGImage` cache supporting `data:image/png;base64,...`,
/// `data:image/rgba;width=W;height=H;base64,...`, and `file://...` URIs.
public final class NveImageCache: @unchecked Sendable {
    public static let shared = NveImageCache()

    public static let maxImageDimension: Int = 16_384
    public static let maxRawRgbaBytes: Int = 64 * 1024 * 1024

    private struct Entry {
        let image: CGImage
        let byteCount: Int
        var tick: UInt64
    }

    private let lock = NSLock()
    public let maxEntries: Int
    public let maxBytes: Int

    private var entries: [String: Entry] = [:]
    private var totalBytesStored: Int = 0
    private var tick: UInt64 = 0

    public init(maxEntries: Int = 128, maxBytes: Int = 64 * 1024 * 1024) {
        self.maxEntries = max(1, maxEntries)
        self.maxBytes = max(1, maxBytes)
    }

    public var count: Int {
        lock.lock()
        defer { lock.unlock() }
        return entries.count
    }

    public var totalBytes: Int {
        lock.lock()
        defer { lock.unlock() }
        return totalBytesStored
    }

    public func contains(key: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return entries[key] != nil
    }

    public func clear() {
        lock.lock()
        entries.removeAll(keepingCapacity: false)
        totalBytesStored = 0
        lock.unlock()
    }

    /// Returns a cached `CGImage` for `source` or decodes & inserts it under bounded LRU eviction.
    public func image(for source: String) -> CGImage? {
        guard !source.isEmpty else { return nil }
        return cached(key: source) { Self.decodeImage(from: source) }
    }

    /// Cached `CGImage` for encoded image bytes (`ImageSrc::EncodedBytes`: PNG / JPEG / HEIC / …)
    /// stored under `key` (e.g. the resource id).
    public func image(encoded bytes: [UInt8], key: String) -> CGImage? {
        guard !bytes.isEmpty else { return nil }
        return cached(key: key) {
            guard let source = CGImageSourceCreateWithData(Data(bytes) as CFData, nil) else { return nil }
            return CGImageSourceCreateImageAtIndex(source, 0, nil)
        }
    }

    private func cached(key: String, decode: () -> CGImage?) -> CGImage? {
        lock.lock()
        if var existing = entries[key] {
            tick &+= 1
            existing.tick = tick
            entries[key] = existing
            lock.unlock()
            return existing.image
        }
        lock.unlock()

        guard let decoded = decode() else { return nil }
        let bytes = max(1, decoded.bytesPerRow * decoded.height)
        lock.lock()
        insertLocked(key: key, image: decoded, bytes: bytes)
        lock.unlock()
        return decoded
    }

    private func insertLocked(key: String, image: CGImage, bytes: Int) {
        if let prev = entries.removeValue(forKey: key) {
            totalBytesStored = max(0, totalBytesStored - prev.byteCount)
        }
        if bytes > maxBytes {
            return
        }
        while !entries.isEmpty && (entries.count >= maxEntries || totalBytesStored + bytes > maxBytes) {
            if let oldestKey = entries.min(by: { $0.value.tick < $1.value.tick })?.key,
               let removed = entries.removeValue(forKey: oldestKey)
            {
                totalBytesStored = max(0, totalBytesStored - removed.byteCount)
            } else {
                break
            }
        }
        tick &+= 1
        entries[key] = Entry(image: image, byteCount: bytes, tick: tick)
        totalBytesStored += bytes
    }

    /// Decodes a `data:image/png;base64,...`, `data:image/rgba;width=W;height=H;base64,...`,
    /// or `file://...` URI directly into a `CGImage`.
    public static func decodeImage(from source: String) -> CGImage? {
        let trimmed = source.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let lower = trimmed.lowercased()

        if lower.hasPrefix("data:") {
            guard let commaIdx = trimmed.firstIndex(of: ",") else { return nil }
            let metaStart = trimmed.index(trimmed.startIndex, offsetBy: 5)
            guard metaStart < commaIdx else { return nil }
            let meta = String(trimmed[metaStart ..< commaIdx])
            let payloadStart = trimmed.index(after: commaIdx)
            guard payloadStart < trimmed.endIndex else { return nil }
            let payload = String(trimmed[payloadStart...])

            let parts = meta.split(separator: ";")
            guard let mimePart = parts.first else { return nil }
            let mime = mimePart.trimmingCharacters(in: .whitespaces).lowercased()

            var isBase64 = false
            var rawW: Int?
            var rawH: Int?
            for token in parts.dropFirst() {
                let p = token.trimmingCharacters(in: .whitespaces)
                if p.caseInsensitiveCompare("base64") == .orderedSame {
                    isBase64 = true
                } else if let eq = p.firstIndex(of: "=") {
                    let k = p[..<eq].trimmingCharacters(in: .whitespaces).lowercased()
                    let v = p[p.index(after: eq)...].trimmingCharacters(in: .whitespaces)
                    if k == "width" || k == "w" {
                        rawW = Int(v)
                    } else if k == "height" || k == "h" {
                        rawH = Int(v)
                    }
                }
            }
            guard isBase64,
                  let data = Data(base64Encoded: payload, options: .ignoreUnknownCharacters),
                  !data.isEmpty
            else {
                return nil
            }

            if mime == "image/rgba" {
                guard let w = rawW, let h = rawH else { return nil }
                return decodeRawRgba(data: data, width: w, height: h)
            }
            if mime.hasPrefix("image/") {
                return decodeEncodedData(data)
            }
            return nil
        }

        if lower.hasPrefix("file://") {
            let url: URL
            if let parsed = URL(string: trimmed), parsed.isFileURL {
                url = parsed
            } else {
                let rawPath = String(trimmed.dropFirst(7))
                let decodedPath = rawPath.removingPercentEncoding ?? rawPath
                url = URL(fileURLWithPath: decodedPath)
            }
            guard let fileData = try? Data(contentsOf: url), !fileData.isEmpty else {
                return nil
            }
            return decodeEncodedData(fileData)
        }

        return nil
    }

    private static func decodeEncodedData(_ data: Data) -> CGImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
              image.width > 0,
              image.height > 0
        else {
            return nil
        }
        return image
    }

    private static func decodeRawRgba(data: Data, width: Int, height: Int) -> CGImage? {
        guard width > 0,
              height > 0,
              width <= maxImageDimension,
              height <= maxImageDimension
        else {
            return nil
        }
        let (pixels, pOverflow) = width.multipliedReportingOverflow(by: height)
        guard !pOverflow else { return nil }
        let (expectedBytes, bOverflow) = pixels.multipliedReportingOverflow(by: 4)
        guard !bOverflow,
              expectedBytes <= maxRawRgbaBytes,
              data.count == expectedBytes
        else {
            return nil
        }
        guard let provider = CGDataProvider(data: data as CFData) else { return nil }
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        let bitmapInfo = CGBitmapInfo(
            rawValue: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.last.rawValue
        )
        return CGImage(
            width: width,
            height: height,
            bitsPerComponent: 8,
            bitsPerPixel: 32,
            bytesPerRow: width * 4,
            space: colorSpace,
            bitmapInfo: bitmapInfo,
            provider: provider,
            decode: nil,
            shouldInterpolate: true,
            intent: .defaultIntent
        )
    }
}
