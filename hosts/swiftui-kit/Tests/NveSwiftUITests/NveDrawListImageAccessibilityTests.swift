import CNveAbi
import NveHost
import CoreGraphics
import Foundation
import ImageIO
import NveSwiftUI
import SwiftUI
import UniformTypeIdentifiers
import XCTest

#if canImport(AppKit)
import AppKit
#endif

private func packRgba(_ r: UInt8, _ g: UInt8, _ b: UInt8, _ a: UInt8 = 255) -> UInt32 {
    UInt32(r) | (UInt32(g) << 8) | (UInt32(b) << 16) | (UInt32(a) << 24)
}

/// Minimal flat `nve.res.draw_list` `u32`-word builder matching `packages/modules/canvas/src/flat.rs`.
private struct DrawListBuilder {
    private var words: [UInt32] = [0]
    private var opCount: UInt32 = 0

    private mutating func appendU32(_ v: UInt32) {
        words.append(v)
    }

    private mutating func appendF32(_ v: Float) {
        words.append(v.bitPattern)
    }

    mutating func clear(_ rgba: UInt32) {
        appendU32(NveDrawListRenderer.opClear)
        appendU32(rgba)
        opCount += 1
    }

    mutating func fillRect(_ x: Float, _ y: Float, _ w: Float, _ h: Float, _ rgba: UInt32) {
        appendU32(NveDrawListRenderer.opFillRect)
        appendF32(x)
        appendF32(y)
        appendF32(w)
        appendF32(h)
        appendU32(rgba)
        opCount += 1
    }

    mutating func fillRoundedRect(
        _ x: Float,
        _ y: Float,
        _ w: Float,
        _ h: Float,
        _ radius: Float,
        _ rgba: UInt32
    ) {
        appendU32(NveDrawListRenderer.opFillRoundedRect)
        appendF32(x)
        appendF32(y)
        appendF32(w)
        appendF32(h)
        appendF32(radius)
        appendU32(rgba)
        opCount += 1
    }

    mutating func fillCircle(_ cx: Float, _ cy: Float, _ radius: Float, _ rgba: UInt32) {
        appendU32(NveDrawListRenderer.opFillCircle)
        appendF32(cx)
        appendF32(cy)
        appendF32(radius)
        appendU32(rgba)
        opCount += 1
    }

    mutating func strokeLine(
        _ x0: Float,
        _ y0: Float,
        _ x1: Float,
        _ y1: Float,
        _ width: Float,
        _ rgba: UInt32
    ) {
        appendU32(NveDrawListRenderer.opStrokeLine)
        appendF32(x0)
        appendF32(y0)
        appendF32(x1)
        appendF32(y1)
        appendF32(width)
        appendU32(rgba)
        opCount += 1
    }

    mutating func strokePolyline(_ points: [(Float, Float)], _ width: Float, _ rgba: UInt32) {
        appendU32(NveDrawListRenderer.opStrokePolyline)
        appendF32(width)
        appendU32(rgba)
        appendU32(UInt32(points.count))
        for (px, py) in points {
            appendF32(px)
            appendF32(py)
        }
        opCount += 1
    }

    mutating func clipRect(_ x: Float, _ y: Float, _ w: Float, _ h: Float) {
        appendU32(NveDrawListRenderer.opClipRect)
        appendF32(x)
        appendF32(y)
        appendF32(w)
        appendF32(h)
        opCount += 1
    }

    mutating func restoreClip() {
        appendU32(NveDrawListRenderer.opRestoreClip)
        opCount += 1
    }

    func finish() -> Data {
        var out = words
        out[0] = opCount
        var bytes = Data(capacity: out.count * 4)
        for w in out {
            withUnsafeBytes(of: w.littleEndian) { bytes.append(contentsOf: $0) }
        }
        return bytes
    }
}

private func encodeWords(_ words: [UInt32]) -> Data {
    var bytes = Data(capacity: words.count * 4)
    for w in words {
        withUnsafeBytes(of: w.littleEndian) { bytes.append(contentsOf: $0) }
    }
    return bytes
}

private func makeSolidPngData(r: UInt8, g: UInt8, b: UInt8, width: Int, height: Int) -> Data {
    var pixels = [UInt8](repeating: 0, count: width * height * 4)
    for i in 0 ..< (width * height) {
        pixels[i * 4] = r
        pixels[i * 4 + 1] = g
        pixels[i * 4 + 2] = b
        pixels[i * 4 + 3] = 255
    }
    let colorSpace = CGColorSpaceCreateDeviceRGB()
    let bitmapInfo = CGBitmapInfo(
        rawValue: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.last.rawValue
    )
    let provider = CGDataProvider(data: Data(pixels) as CFData)!
    let cgImage = CGImage(
        width: width,
        height: height,
        bitsPerComponent: 8,
        bitsPerPixel: 32,
        bytesPerRow: width * 4,
        space: colorSpace,
        bitmapInfo: bitmapInfo,
        provider: provider,
        decode: nil,
        shouldInterpolate: false,
        intent: .defaultIntent
    )!
    let out = NSMutableData()
    let dest = CGImageDestinationCreateWithData(out, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, cgImage, nil)
    CGImageDestinationFinalize(dest)
    return out as Data
}

private func rgbaPixel(in cgImage: CGImage, x: Int, y: Int) -> (UInt8, UInt8, UInt8, UInt8) {
    var pixel = [UInt8](repeating: 0, count: 4)
    let colorSpace = CGColorSpaceCreateDeviceRGB()
    let ctx = CGContext(
        data: &pixel,
        width: 1,
        height: 1,
        bitsPerComponent: 8,
        bytesPerRow: 4,
        space: colorSpace,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue
    )!
    ctx.draw(
        cgImage,
        in: CGRect(x: -CGFloat(x), y: -CGFloat(cgImage.height - 1 - y), width: CGFloat(cgImage.width), height: CGFloat(cgImage.height))
    )
    return (pixel[0], pixel[1], pixel[2], pixel[3])
}

final class NveDrawListImageAccessibilityTests: XCTestCase {
    func testSwiftSourcesRemainModularUnder550LinesAfterSplit() throws {
        let sourcesDir = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // NveSwiftUITests
            .deletingLastPathComponent() // Tests
            .deletingLastPathComponent() // swiftui-kit
            .appendingPathComponent("Sources/NveSwiftUI")

        let files = try FileManager.default.contentsOfDirectory(
            at: sourcesDir,
            includingPropertiesForKeys: nil
        ).filter { $0.pathExtension == "swift" }

        XCTAssertGreaterThanOrEqual(files.count, 8)
        for file in files {
            let content = try String(contentsOf: file, encoding: .utf8)
            let lineCount = content.components(separatedBy: .newlines).count
            XCTAssertLessThan(
                lineCount,
                550,
                "\(file.lastPathComponent) has \(lineCount) lines (expected < 550)"
            )
            if file.lastPathComponent.hasPrefix("NveDirect") {
                XCTAssertLessThan(
                    lineCount,
                    300,
                    "\(file.lastPathComponent) has \(lineCount) lines (expected < 300)"
                )
            }
        }
    }

    @MainActor
    func testDrawListRendererPaintsAll8OpcodesAndHandlesBase64Fallback() throws {
        var b = DrawListBuilder()
        b.clear(packRgba(10, 20, 30))
        b.fillRect(4, 4, 24, 24, packRgba(220, 38, 38))
        b.fillRoundedRect(34, 4, 24, 24, 6, packRgba(16, 185, 129))
        b.fillCircle(76, 16, 10, packRgba(37, 99, 235))
        b.strokeLine(4, 42, 92, 42, 4, packRgba(245, 158, 11))
        b.strokePolyline([(6, 62), (48, 52), (90, 62)], 4, packRgba(236, 72, 153))
        b.clipRect(10, 70, 30, 20)
        b.fillRect(0, 70, 96, 20, packRgba(255, 255, 255))
        b.restoreClip()
        b.fillRect(70, 74, 16, 12, packRgba(56, 189, 248))
        let drawData = b.finish()
        XCTAssertEqual(drawData.count, 56 * 4)
        XCTAssertEqual(drawData.count & 3, 0)

        let header = try NveDrawListRenderer.validate(drawData)
        XCTAssertEqual(header.opCount, 10)

        var executedOps = 0
        let canvas = Canvas { ctx, size in
            executedOps = (try? NveDrawListRenderer.draw(
                drawData,
                into: &ctx,
                in: CGRect(origin: .zero, size: size)
            )) ?? -1
        }.frame(width: 96, height: 96)

        let renderer = ImageRenderer(content: canvas)
        renderer.scale = 1.0
        let cg = try XCTUnwrap(renderer.cgImage)
        XCTAssertEqual(executedOps, 10)
        XCTAssertEqual(cg.width, 96)
        XCTAssertEqual(cg.height, 96)

        // Verify distinct pixel colors produced by each opcode:
        let bg = rgbaPixel(in: cg, x: 2, y: 2)
        XCTAssertEqual(bg.0, 10, accuracy: 2)
        XCTAssertEqual(bg.1, 20, accuracy: 2)
        XCTAssertEqual(bg.2, 30, accuracy: 2)

        let rectPx = rgbaPixel(in: cg, x: 16, y: 16)
        XCTAssertEqual(rectPx.0, 220, accuracy: 3)
        XCTAssertEqual(rectPx.1, 38, accuracy: 3)

        let rrectPx = rgbaPixel(in: cg, x: 46, y: 16)
        XCTAssertEqual(rrectPx.1, 185, accuracy: 3)

        let circlePx = rgbaPixel(in: cg, x: 76, y: 16)
        XCTAssertEqual(circlePx.2, 235, accuracy: 3)

        let linePx = rgbaPixel(in: cg, x: 48, y: 42)
        XCTAssertEqual(linePx.0, 245, accuracy: 3)
        XCTAssertEqual(linePx.1, 158, accuracy: 3)

        let polyPx = rgbaPixel(in: cg, x: 48, y: 52)
        XCTAssertGreaterThan(polyPx.0, 180)
        XCTAssertGreaterThan(polyPx.2, 100)

        // Inside ClipRect -> white; outside ClipRect -> Clear background; after RestoreClip -> sky blue
        let insideClip = rgbaPixel(in: cg, x: 25, y: 80)
        XCTAssertEqual(insideClip.0, 255, accuracy: 2)
        XCTAssertEqual(insideClip.1, 255, accuracy: 2)
        XCTAssertEqual(insideClip.2, 255, accuracy: 2)

        let outsideClip = rgbaPixel(in: cg, x: 55, y: 80)
        XCTAssertEqual(outsideClip.0, 10, accuracy: 2)
        XCTAssertEqual(outsideClip.1, 20, accuracy: 2)

        let afterRestore = rgbaPixel(in: cg, x: 78, y: 80)
        XCTAssertEqual(afterRestore.0, 56, accuracy: 3)
        XCTAssertEqual(afterRestore.2, 248, accuracy: 3)

        // Verify zero-copy `UnsafeBufferPointer<UInt8>` flat `nve.res.draw_list` rendering and
        // rejection of retired `nve-draw1;b64,` strings.
        var ptrExecutedOps = 0
        let ptrCanvas = Canvas { ctx, size in
            drawData.withUnsafeBytes { raw in
                let bytes = raw.bindMemory(to: UInt8.self)
                ptrExecutedOps = (try? NveDrawListRenderer.draw(
                    bytes,
                    into: &ctx,
                    in: CGRect(origin: .zero, size: size)
                )) ?? -1
            }
        }.frame(width: 96, height: 96)
        let ptrRenderer = ImageRenderer(content: ptrCanvas)
        ptrRenderer.scale = 1.0
        let ptrCg = try XCTUnwrap(ptrRenderer.cgImage)
        XCTAssertEqual(ptrExecutedOps, 10)
        XCTAssertEqual(rgbaPixel(in: ptrCg, x: 16, y: 16).0, 220, accuracy: 3)

        let retiredAttr = Data(("nve-draw1;b64," + drawData.base64EncodedString()).utf8)
        XCTAssertThrowsError(try NveDrawListRenderer.validate(retiredAttr))

        // Verify empty flat draw list (single zero word = 4 bytes) and unbalanced clip balancing
        let emptyFlat = DrawListBuilder().finish()
        XCTAssertEqual(emptyFlat, Data([0, 0, 0, 0]))
        XCTAssertEqual(try NveDrawListRenderer.validate(emptyFlat), NveDrawListHeader(opCount: 0))

        var clipTest = DrawListBuilder()
        clipTest.restoreClip() // unmatched extra restore must be a safe no-op
        clipTest.clipRect(0, 0, 16, 16) // intentionally left unclosed
        clipTest.fillRect(0, 0, 32, 32, packRgba(255, 0, 0))
        let clipData = clipTest.finish()
        var clipOps = 0
        let clipCanvas = Canvas { ctx, size in
            clipOps = (try? NveDrawListRenderer.draw(
                clipData,
                into: &ctx,
                in: CGRect(origin: .zero, size: size)
            )) ?? -1
        }.frame(width: 32, height: 32)
        let clipRenderer = ImageRenderer(content: clipCanvas)
        clipRenderer.scale = 1.0
        _ = try XCTUnwrap(clipRenderer.cgImage)
        XCTAssertEqual(clipOps, 3)
    }

    func testDrawListValidatorRejectsTruncatedCorruptNonFiniteAndOversizedBuffers() throws {
        // Exact byte-for-byte parity with `packages/modules/canvas/src/flat.rs` word layout
        let black = packRgba(0, 0, 0)
        let white = packRgba(255, 255, 255)
        var parity = DrawListBuilder()
        parity.clear(black)
        parity.fillRect(1.0, 2.0, 3.0, 4.0, white)
        parity.strokePolyline([(5.0, 6.0)], 2.5, white)
        parity.restoreClip()
        let expectedWords: [UInt32] = [
            4,
            1, black,
            2, Float(1.0).bitPattern, Float(2.0).bitPattern, Float(3.0).bitPattern, Float(4.0).bitPattern, white,
            6, Float(2.5).bitPattern, white, 1, Float(5.0).bitPattern, Float(6.0).bitPattern,
            8,
        ]
        XCTAssertEqual(parity.finish(), encodeWords(expectedWords))
        XCTAssertEqual(try NveDrawListRenderer.validate(parity.finish()), NveDrawListHeader(opCount: 4))

        // Truncated header (< 4 bytes)
        XCTAssertThrowsError(try NveDrawListRenderer.validate(Data())) { err in
            XCTAssertEqual(err as? NveDrawListError, .truncated)
        }
        XCTAssertThrowsError(try NveDrawListRenderer.validate(Data([1, 2, 3]))) { err in
            XCTAssertEqual(err as? NveDrawListError, .truncated)
        }

        // Retired legacy `NVE_DRAW1` 15-byte header is rejected
        var legacy = Data("NVE_DRAW1".utf8)
        legacy.append(contentsOf: [1, 0, 0, 0, 0, 0])
        XCTAssertThrowsError(try NveDrawListRenderer.validate(legacy)) { err in
            XCTAssertEqual(err as? NveDrawListError, .opCountExceedsLimit(0x5F45_564E))
        }

        // Op count exceeds limit (maxOps + 1 and UInt32.max without 32-bit overflow)
        let tooMany = UInt32(NveDrawListConstants.maxOps + 1)
        XCTAssertThrowsError(try NveDrawListRenderer.validate(encodeWords([tooMany]))) { err in
            XCTAssertEqual(err as? NveDrawListError, .opCountExceedsLimit(tooMany))
        }
        XCTAssertThrowsError(try NveDrawListRenderer.validate(encodeWords([UInt32.max]))) { err in
            XCTAssertEqual(err as? NveDrawListError, .opCountExceedsLimit(UInt32.max))
        }

        // Truncated opcode word and truncated op payloads
        XCTAssertThrowsError(try NveDrawListRenderer.validate(Data([1, 0, 0, 0, 1, 0]))) { err in
            XCTAssertEqual(err as? NveDrawListError, .truncated)
        }
        XCTAssertThrowsError(try NveDrawListRenderer.validate(Data([1, 0, 0, 0, 1, 0, 0, 0, 0xFF, 0x00]))) { err in
            XCTAssertEqual(err as? NveDrawListError, .truncated)
        }
        XCTAssertThrowsError(try NveDrawListRenderer.validate(encodeWords([1, 2, 0, 0, 0, 0]))) { err in
            XCTAssertEqual(err as? NveDrawListError, .truncated)
        }
        XCTAssertThrowsError(
            try NveDrawListRenderer.validate(encodeWords([1, 6, Float(1.0).bitPattern, white, 2, 0, 0]))
        ) { err in
            XCTAssertEqual(err as? NveDrawListError, .truncated)
        }

        // Unknown UInt32 opcodes (0, 9, 0x7F, and high-bits non-zero 0x0001_0001)
        for badOp: UInt32 in [0, 9, 0x7F, 0x0001_0001] {
            XCTAssertThrowsError(try NveDrawListRenderer.validate(encodeWords([1, badOp]))) { err in
                XCTAssertEqual(err as? NveDrawListError, .unknownOpcode(badOp))
            }
        }

        // Non-finite NaN & Infinity
        var nanBuilder = DrawListBuilder()
        nanBuilder.fillRect(.nan, 0, 10, 10, packRgba(255, 0, 0))
        XCTAssertThrowsError(try NveDrawListRenderer.validate(nanBuilder.finish())) { err in
            XCTAssertEqual(err as? NveDrawListError, .nonFiniteFloat)
        }

        var infBuilder = DrawListBuilder()
        infBuilder.fillCircle(0, .infinity, 5, packRgba(255, 0, 0))
        XCTAssertThrowsError(try NveDrawListRenderer.validate(infBuilder.finish())) { err in
            XCTAssertEqual(err as? NveDrawListError, .nonFiniteFloat)
        }

        var badPolyFloat = DrawListBuilder()
        badPolyFloat.strokePolyline([(1, .nan)], 2, white)
        XCTAssertThrowsError(try NveDrawListRenderer.validate(badPolyFloat.finish())) { err in
            XCTAssertEqual(err as? NveDrawListError, .nonFiniteFloat)
        }

        var badClipFloat = DrawListBuilder()
        badClipFloat.clipRect(0, 0, -.infinity, 10)
        XCTAssertThrowsError(try NveDrawListRenderer.validate(badClipFloat.finish())) { err in
            XCTAssertEqual(err as? NveDrawListError, .nonFiniteFloat)
        }

        // Oversized polyline vertex count (maxPolylinePoints + 1 and UInt32.max)
        let overPts = UInt32(NveDrawListConstants.maxPolylinePoints + 1)
        let badPoly = encodeWords([1, NveDrawListRenderer.opStrokePolyline, Float(2.0).bitPattern, white, overPts])
        XCTAssertThrowsError(try NveDrawListRenderer.validate(badPoly)) { err in
            XCTAssertEqual(err as? NveDrawListError, .sliceLenExceedsLimit(overPts))
        }
        let maxPoly = encodeWords([1, NveDrawListRenderer.opStrokePolyline, Float(2.0).bitPattern, white, UInt32.max])
        XCTAssertThrowsError(try NveDrawListRenderer.validate(maxPoly)) { err in
            XCTAssertEqual(err as? NveDrawListError, .sliceLenExceedsLimit(UInt32.max))
        }

        // Trailing bytes (both unaligned and word-aligned)
        var b = DrawListBuilder()
        b.clear(black)
        var trailing2 = b.finish()
        trailing2.append(contentsOf: [0xDE, 0xAD])
        XCTAssertThrowsError(try NveDrawListRenderer.validate(trailing2)) { err in
            XCTAssertEqual(err as? NveDrawListError, .trailingBytes(2))
        }
        var trailing4 = b.finish()
        trailing4.append(contentsOf: [0xDE, 0xAD, 0xBE, 0xEF])
        XCTAssertThrowsError(try NveDrawListRenderer.validate(trailing4)) { err in
            XCTAssertEqual(err as? NveDrawListError, .trailingBytes(4))
        }
        XCTAssertThrowsError(try NveDrawListRenderer.validate(Data([0, 0, 0, 0, 0xAA]))) { err in
            XCTAssertEqual(err as? NveDrawListError, .trailingBytes(1))
        }

        // Retired `nve-draw1;b64,` strings are never accepted as flat `nve.res.draw_list` bytes.
        XCTAssertThrowsError(try NveDrawListRenderer.validate(Data("nve-draw1;b64,AAAAAA==".utf8)))

        // Negative radii and stroke widths rejected with `.negativeFloat`; `-0.0` accepted.
        var negRRect = DrawListBuilder()
        negRRect.fillRoundedRect(0, 0, 10, 10, -0.5, white)
        XCTAssertThrowsError(try NveDrawListRenderer.validate(negRRect.finish())) { err in
            XCTAssertEqual(err as? NveDrawListError, .negativeFloat)
        }
        var negCircle = DrawListBuilder()
        negCircle.fillCircle(0, 0, -1, white)
        XCTAssertThrowsError(try NveDrawListRenderer.validate(negCircle.finish())) { err in
            XCTAssertEqual(err as? NveDrawListError, .negativeFloat)
        }
        var negLine = DrawListBuilder()
        negLine.strokeLine(0, 0, 5, 5, -0.25, white)
        XCTAssertThrowsError(try NveDrawListRenderer.validate(negLine.finish())) { err in
            XCTAssertEqual(err as? NveDrawListError, .negativeFloat)
        }
        var negPoly = DrawListBuilder()
        negPoly.strokePolyline([(0, 0)], -2, white)
        XCTAssertThrowsError(try NveDrawListRenderer.validate(negPoly.finish())) { err in
            XCTAssertEqual(err as? NveDrawListError, .negativeFloat)
        }
        var negZero = DrawListBuilder()
        negZero.fillCircle(0, 0, -0.0, white)
        XCTAssertEqual(try NveDrawListRenderer.validate(negZero.finish()), NveDrawListHeader(opCount: 1))
    }

    @MainActor
    func testDrawListRendererEnforcesMaxClipDepth66Clips2Restores() throws {
        var b = DrawListBuilder()
        b.clear(packRgba(0, 0, 0))
        for _ in 0 ..< (NveDrawListConstants.maxClipDepth - 1) {
            b.clipRect(0, 0, 96, 96)
        }
        // Clip #64 restricts to [0..20); clips #65 and #66 overflow the 64-deep cap.
        b.clipRect(0, 0, 20, 20)
        b.clipRect(0, 0, 8, 8)
        b.clipRect(0, 0, 4, 4)
        // Two restores unwind overflow only; clip #64 [0..20) stays active.
        b.restoreClip()
        b.restoreClip()
        b.fillRect(0, 0, 96, 96, packRgba(255, 0, 0))
        // Third restore pops clip #64 back to [0..96).
        b.restoreClip()
        b.fillRect(30, 30, 20, 20, packRgba(0, 0, 255))
        let data = b.finish()

        let canvas = Canvas { ctx, size in
            _ = try? NveDrawListRenderer.draw(data, into: &ctx, in: CGRect(origin: .zero, size: size))
        }.frame(width: 96, height: 96)
        let renderer = ImageRenderer(content: canvas)
        renderer.scale = 1.0
        let cg = try XCTUnwrap(renderer.cgImage)

        let inside64 = rgbaPixel(in: cg, x: 12, y: 12)
        XCTAssertEqual(inside64.0, 255, accuracy: 3)
        XCTAssertEqual(inside64.1, 0, accuracy: 3)

        let outside64 = rgbaPixel(in: cg, x: 70, y: 70)
        XCTAssertEqual(outside64.0, 0, accuracy: 3)
        XCTAssertEqual(outside64.2, 0, accuracy: 3)

        let after3rdRestore = rgbaPixel(in: cg, x: 40, y: 40)
        XCTAssertEqual(after3rdRestore.2, 255, accuracy: 3)
    }

    func testImageCacheDecodesPngRgbaFileUrisEnforcesLruAndComputesAll4ResizeModes() throws {
        let cache = NveImageCache(maxEntries: 2, maxBytes: 1024 * 1024)

        // 1. data:image/png;base64,...
        let redPng = makeSolidPngData(r: 255, g: 0, b: 0, width: 6, height: 4)
        let redUri = "data:image/png;base64,\(redPng.base64EncodedString())"
        let img1 = try XCTUnwrap(cache.image(for: redUri))
        XCTAssertEqual(img1.width, 6)
        XCTAssertEqual(img1.height, 4)
        XCTAssertEqual(cache.count, 1)

        // 2. data:image/rgba;width=2;height=2;base64,...
        let rawPixels = Data([
            0, 255, 0, 255, 0, 255, 0, 255,
            0, 255, 0, 255, 0, 255, 0, 255,
        ])
        let rgbaUri = "data:image/rgba;width=2;height=2;base64,\(rawPixels.base64EncodedString())"
        let img2 = try XCTUnwrap(cache.image(for: rgbaUri))
        XCTAssertEqual(img2.width, 2)
        XCTAssertEqual(img2.height, 2)
        XCTAssertEqual(cache.count, 2)

        // Touch redUri so rgbaUri becomes LRU
        XCTAssertNotNil(cache.image(for: redUri))

        // 3. file://... PNG -> evicts rgbaUri, keeps redUri
        let tmpDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("nve_swift_img_test_\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tmpDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmpDir) }
        let fileUrl = tmpDir.appendingPathComponent("blue.png")
        try makeSolidPngData(r: 0, g: 0, b: 255, width: 5, height: 5).write(to: fileUrl)

        let img3 = try XCTUnwrap(cache.image(for: fileUrl.absoluteString))
        XCTAssertEqual(img3.width, 5)
        XCTAssertEqual(img3.height, 5)
        XCTAssertEqual(cache.count, 2)
        XCTAssertTrue(cache.contains(key: redUri))
        XCTAssertTrue(cache.contains(key: fileUrl.absoluteString))
        XCTAssertFalse(cache.contains(key: rgbaUri))

        // Invalid / malformed URI returns nil cleanly
        XCTAssertNil(cache.image(for: "data:image/rgba;width=4;height=4;base64,AAAA"))
        XCTAssertNil(cache.image(for: "file:///nonexistent/nve_missing_image.png"))

        // Verify all 4 resize modes placement math
        let dst = CGRect(x: 10, y: 20, width: 100, height: 50)
        let stretch = try XCTUnwrap(
            NveImagePlacement.compute(destination: dst, imageSize: CGSize(width: 200, height: 200), fit: .fill)
        )
        XCTAssertEqual(stretch.destRect, dst)
        XCTAssertEqual(stretch.drawRect, dst)

        let contain = try XCTUnwrap(
            NveImagePlacement.compute(destination: dst, imageSize: CGSize(width: 200, height: 200), fit: .contain)
        )
        XCTAssertEqual(contain.destRect.minX, 35, accuracy: 0.01)
        XCTAssertEqual(contain.destRect.width, 50, accuracy: 0.01)
        XCTAssertEqual(contain.destRect.height, 50, accuracy: 0.01)

        let cover = try XCTUnwrap(
            NveImagePlacement.compute(destination: dst, imageSize: CGSize(width: 200, height: 200), fit: .cover)
        )
        XCTAssertEqual(cover.destRect, dst)
        XCTAssertEqual(cover.sourceRect.minY, 50, accuracy: 0.01)
        XCTAssertEqual(cover.sourceRect.height, 100, accuracy: 0.01)
        XCTAssertEqual(cover.drawRect.width, 100, accuracy: 0.01)
        XCTAssertEqual(cover.drawRect.height, 100, accuracy: 0.01)

        let center = try XCTUnwrap(
            NveImagePlacement.compute(destination: dst, imageSize: CGSize(width: 40, height: 20), fit: .none_)
        )
        XCTAssertEqual(center.destRect, CGRect(x: 40, y: 35, width: 40, height: 20))
    }
}
