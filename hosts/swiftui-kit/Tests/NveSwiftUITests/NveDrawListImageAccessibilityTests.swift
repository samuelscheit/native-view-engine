import CNveAbi
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

/// Minimal `NVE_DRAW1` binary builder matching `crates/nve-components/src/canvas_ir.rs`.
private struct DrawListBuilder {
    private var buf: [UInt8] = []
    private var opCount: UInt32 = 0

    init() {
        buf.append(contentsOf: NveDrawListConstants.magic)
        appendU16(NveDrawListConstants.version)
        appendU32(0)
    }

    private mutating func appendU16(_ v: UInt16) {
        withUnsafeBytes(of: v.littleEndian) { buf.append(contentsOf: $0) }
    }

    private mutating func appendU32(_ v: UInt32) {
        withUnsafeBytes(of: v.littleEndian) { buf.append(contentsOf: $0) }
    }

    private mutating func appendF32(_ v: Float) {
        appendU32(v.bitPattern)
    }

    mutating func clear(_ rgba: UInt32) {
        buf.append(NveDrawListRenderer.opClear)
        appendU32(rgba)
        opCount += 1
    }

    mutating func fillRect(_ x: Float, _ y: Float, _ w: Float, _ h: Float, _ rgba: UInt32) {
        buf.append(NveDrawListRenderer.opFillRect)
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
        buf.append(NveDrawListRenderer.opFillRoundedRect)
        appendF32(x)
        appendF32(y)
        appendF32(w)
        appendF32(h)
        appendF32(radius)
        appendU32(rgba)
        opCount += 1
    }

    mutating func fillCircle(_ cx: Float, _ cy: Float, _ radius: Float, _ rgba: UInt32) {
        buf.append(NveDrawListRenderer.opFillCircle)
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
        buf.append(NveDrawListRenderer.opStrokeLine)
        appendF32(x0)
        appendF32(y0)
        appendF32(x1)
        appendF32(y1)
        appendF32(width)
        appendU32(rgba)
        opCount += 1
    }

    mutating func strokePolyline(_ points: [(Float, Float)], _ width: Float, _ rgba: UInt32) {
        buf.append(NveDrawListRenderer.opStrokePolyline)
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
        buf.append(NveDrawListRenderer.opClipRect)
        appendF32(x)
        appendF32(y)
        appendF32(w)
        appendF32(h)
        opCount += 1
    }

    mutating func restoreClip() {
        buf.append(NveDrawListRenderer.opRestoreClip)
        opCount += 1
    }

    func finish() -> Data {
        var out = buf
        withUnsafeBytes(of: opCount.littleEndian) { raw in
            for i in 0 ..< 4 {
                out[11 + i] = raw[i]
            }
        }
        return Data(out)
    }
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

        let header = try NveDrawListRenderer.validate(drawData)
        XCTAssertEqual(header.version, 1)
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

        // Verify `nve-draw1;b64,` base64 attribute decoding roundtrip
        let attrStr = NveDrawListConstants.attrPrefix + drawData.base64EncodedString()
        let decoded = try NveDrawListRenderer.decodeAttrString(attrStr)
        XCTAssertEqual(decoded, drawData)
    }

    func testDrawListValidatorRejectsTruncatedCorruptNonFiniteAndOversizedBuffers() {
        // Truncated header
        XCTAssertThrowsError(try NveDrawListRenderer.validate(Data([1, 2, 3]))) { err in
            XCTAssertEqual(err as? NveDrawListError, .truncated)
        }

        // Bad magic
        var badMagic = Data("NVE_DRAWX".utf8)
        badMagic.append(contentsOf: [1, 0, 0, 0, 0, 0])
        XCTAssertThrowsError(try NveDrawListRenderer.validate(badMagic)) { err in
            XCTAssertEqual(err as? NveDrawListError, .badMagic)
        }

        // Unsupported version
        var badVer = DrawListBuilder().finish()
        badVer[9] = 99
        XCTAssertThrowsError(try NveDrawListRenderer.validate(badVer)) { err in
            XCTAssertEqual(err as? NveDrawListError, .unsupportedVersion(99))
        }

        // Op count exceeds limit
        var hugeOps = DrawListBuilder().finish()
        let tooMany = UInt32(NveDrawListConstants.maxOps + 1).littleEndian
        withUnsafeBytes(of: tooMany) { raw in
            for i in 0 ..< 4 { hugeOps[11 + i] = raw[i] }
        }
        XCTAssertThrowsError(try NveDrawListRenderer.validate(hugeOps)) { err in
            XCTAssertEqual(err as? NveDrawListError, .opCountExceedsLimit(UInt32(NveDrawListConstants.maxOps + 1)))
        }

        // Unknown opcode
        var unknown = Data(NveDrawListConstants.magic)
        unknown.append(contentsOf: [1, 0, 1, 0, 0, 0, 0x7F])
        XCTAssertThrowsError(try NveDrawListRenderer.validate(unknown)) { err in
            XCTAssertEqual(err as? NveDrawListError, .unknownOpcode(0x7F))
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

        // Oversized polyline vertex count
        var badPoly = Data(NveDrawListConstants.magic)
        badPoly.append(contentsOf: [1, 0, 1, 0, 0, 0, NveDrawListRenderer.opStrokePolyline])
        withUnsafeBytes(of: Float(2.0).bitPattern.littleEndian) { badPoly.append(contentsOf: $0) }
        withUnsafeBytes(of: packRgba(255, 255, 255).littleEndian) { badPoly.append(contentsOf: $0) }
        let overPts = UInt32(NveDrawListConstants.maxPolylinePoints + 1).littleEndian
        withUnsafeBytes(of: overPts) { badPoly.append(contentsOf: $0) }
        XCTAssertThrowsError(try NveDrawListRenderer.validate(badPoly)) { err in
            XCTAssertEqual(
                err as? NveDrawListError,
                .polylinePointCountExceedsLimit(UInt32(NveDrawListConstants.maxPolylinePoints + 1))
            )
        }

        // Trailing bytes
        var b = DrawListBuilder()
        b.clear(packRgba(0, 0, 0))
        var trailing = b.finish()
        trailing.append(contentsOf: [0xDE, 0xAD])
        XCTAssertThrowsError(try NveDrawListRenderer.validate(trailing)) { err in
            XCTAssertEqual(err as? NveDrawListError, .trailingBytes(2))
        }
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
            NveImagePlacement.compute(destination: dst, imageSize: CGSize(width: 200, height: 200), mode: .stretch)
        )
        XCTAssertEqual(stretch.destRect, dst)
        XCTAssertEqual(stretch.drawRect, dst)

        let contain = try XCTUnwrap(
            NveImagePlacement.compute(destination: dst, imageSize: CGSize(width: 200, height: 200), mode: .contain)
        )
        XCTAssertEqual(contain.destRect.minX, 35, accuracy: 0.01)
        XCTAssertEqual(contain.destRect.width, 50, accuracy: 0.01)
        XCTAssertEqual(contain.destRect.height, 50, accuracy: 0.01)

        let cover = try XCTUnwrap(
            NveImagePlacement.compute(destination: dst, imageSize: CGSize(width: 200, height: 200), mode: .cover)
        )
        XCTAssertEqual(cover.destRect, dst)
        XCTAssertEqual(cover.sourceRect.minY, 50, accuracy: 0.01)
        XCTAssertEqual(cover.sourceRect.height, 100, accuracy: 0.01)
        XCTAssertEqual(cover.drawRect.width, 100, accuracy: 0.01)
        XCTAssertEqual(cover.drawRect.height, 100, accuracy: 0.01)

        let center = try XCTUnwrap(
            NveImagePlacement.compute(destination: dst, imageSize: CGSize(width: 40, height: 20), mode: .center)
        )
        XCTAssertEqual(center.destRect, CGRect(x: 40, y: 35, width: 40, height: 20))
    }

    @MainActor
    func testSceneStoreIncrementalIndexStabilityRenderingAndAccessibilityMetadata() throws {
        let store = NveSceneStore()
        let cache = NveImageCache(maxEntries: 8)

        var drawB = DrawListBuilder()
        drawB.fillRoundedRect(0, 0, 80, 40, 6, packRgba(37, 99, 235))
        drawB.fillCircle(40, 20, 10, packRgba(250, 204, 21))
        let drawBytes = Array(drawB.finish())

        let imgPng = makeSolidPngData(r: 16, g: 185, b: 129, width: 8, height: 8)
        let imgUri = "data:image/png;base64,\(imgPng.base64EncodedString())"

        let headingBytes = Array("Dashboard".utf8)
        let buttonBytes = Array("Deploy".utf8)
        let secretBytes = Array("pass1234".utf8)
        let imgBytes = Array(imgUri.utf8)

        var slab: [UInt8] = []
        let headingOff = 0
        slab.append(contentsOf: headingBytes)
        let buttonOff = slab.count
        slab.append(contentsOf: buttonBytes)
        let secretOff = slab.count
        slab.append(contentsOf: secretBytes)
        let imgOff = slab.count
        slab.append(contentsOf: imgBytes)
        let drawOff = slab.count
        slab.append(contentsOf: drawBytes)

        let children: [UInt32] = [2, 3, 4, 5, 6, 7]
        var nodes: [NveRenderNode] = [
            NveRenderNode(
                node_id: 1, kind: UInt8(NVE_NODE_KIND_ROOT), flags: 0, opacity_u8: 255, _pad0: 0,
                x: 0, y: 0, width: 320, height: 200,
                bg_rgba: packRgba(15, 23, 42), text_rgba: 0, border_rgba: 0,
                border_radius: 0, border_width: 0, font_size: 16,
                text_offset: 0, text_len: 0, child_count: 6, child_offset: 0, z_index: 0
            ),
            NveRenderNode(
                node_id: 2, kind: UInt8(NVE_NODE_KIND_TEXT), flags: UInt8(NVE_NODE_LEAF_TEXT), opacity_u8: 255, _pad0: 0,
                x: 10, y: 10, width: 180, height: 30,
                bg_rgba: 0, text_rgba: packRgba(248, 250, 252), border_rgba: 0,
                border_radius: 0, border_width: 0, font_size: 24,
                text_offset: UInt32(headingOff), text_len: UInt16(headingBytes.count), child_count: 0, child_offset: 0, z_index: 0
            ),
            NveRenderNode(
                node_id: 3, kind: UInt8(NVE_NODE_KIND_PRESSABLE), flags: UInt8(NVE_NODE_INTERACTIVE | NVE_NODE_LEAF_TEXT), opacity_u8: 255, _pad0: 0,
                x: 10, y: 48, width: 120, height: 36,
                bg_rgba: packRgba(37, 99, 235), text_rgba: packRgba(255, 255, 255), border_rgba: 0,
                border_radius: 6, border_width: 0, font_size: 14,
                text_offset: UInt32(buttonOff), text_len: UInt16(buttonBytes.count), child_count: 0, child_offset: 0, z_index: 0
            ),
            NveRenderNode(
                node_id: 4, kind: UInt8(NVE_NODE_KIND_TEXT_INPUT), flags: UInt8(NVE_NODE_INTERACTIVE), opacity_u8: 255, _pad0: 0,
                x: 10, y: 92, width: 180, height: 32,
                bg_rgba: packRgba(30, 41, 59), text_rgba: packRgba(255, 255, 255), border_rgba: packRgba(100, 116, 139),
                border_radius: 4, border_width: 1, font_size: 14,
                text_offset: UInt32(secretOff), text_len: UInt16(secretBytes.count), child_count: 0, child_offset: 0, z_index: 0
            ),
            NveRenderNode(
                node_id: 5, kind: UInt8(NVE_NODE_KIND_TEXT_INPUT), flags: UInt8(NVE_NODE_INTERACTIVE), opacity_u8: 255, _pad0: 0,
                x: 10, y: 132, width: 180, height: 32,
                bg_rgba: packRgba(30, 41, 59), text_rgba: packRgba(255, 255, 255), border_rgba: 0,
                border_radius: 4, border_width: 0, font_size: 14,
                text_offset: 0, text_len: 0, child_count: 0, child_offset: 0, z_index: 0
            ),
            NveRenderNode(
                node_id: 6, kind: UInt8(NVE_NODE_KIND_IMAGE), flags: 0, opacity_u8: 255, _pad0: 0,
                x: 210, y: 10, width: 80, height: 60,
                bg_rgba: 0, text_rgba: 0, border_rgba: 0,
                border_radius: 8, border_width: 0, font_size: 0,
                text_offset: UInt32(imgOff), text_len: UInt16(imgBytes.count), child_count: 0, child_offset: 0, z_index: 0
            ),
            NveRenderNode(
                node_id: 7, kind: UInt8(NVE_NODE_KIND_SKIA_CANVAS), flags: 0, opacity_u8: 255, _pad0: 0,
                x: 210, y: 90, width: 80, height: 40,
                bg_rgba: 0, text_rgba: 0, border_rgba: 0,
                border_radius: 6, border_width: 0, font_size: 0,
                text_offset: UInt32(drawOff), text_len: UInt16(drawBytes.count), child_count: 0, child_offset: 0, z_index: 0
            ),
        ]

        func apply(frame: UInt64, dirty: [UInt32]) throws {
            try nodes.withUnsafeBufferPointer { nb in
                try slab.withUnsafeBufferPointer { tb in
                    try children.withUnsafeBufferPointer { cb in
                        try dirty.withUnsafeBufferPointer { db in
                            let view = NveSceneView(
                                schema_hash: UInt64(NVE_SCENE_SCHEMA_HASH),
                                frame_id: frame,
                                node_count: UInt32(nb.count),
                                dirty_count: UInt32(db.count),
                                removed_count: 0,
                                text_slab_len: UInt32(tb.count),
                                children_slab_len: UInt32(cb.count),
                                _pad: 0,
                                nodes: nb.baseAddress,
                                text_slab: tb.baseAddress,
                                children_slab: cb.baseAddress,
                                dirty_indices: db.baseAddress,
                                removed_ids: nil
                            )
                            try store.apply(view: view)
                        }
                    }
                }
            }
        }

        // Frame 1: initial build -> indexRebuildCount == 1
        try apply(frame: 1, dirty: [0, 1, 2, 3, 4, 5, 6])
        XCTAssertEqual(store.indexRebuildCount, 1)
        XCTAssertEqual(store.nodeIndexById[7], 6)

        store.setTextInputConfig(placeholder: "Password", secureTextEntry: true, forNodeId: 4)
        store.setAccessibility(label: "Password", forNodeId: 4)
        store.setTextInputConfig(placeholder: "Search...", forNodeId: 5)
        store.setImageResizeMode(.cover, forNodeId: 6)
        store.setAccessibility(label: "Status Avatar", forNodeId: 6)

        // Frame 2 & 3: incremental dirty updates must NEVER trigger an O(N) index rebuild!
        nodes[2].bg_rgba = packRgba(29, 78, 216)
        try apply(frame: 2, dirty: [2])
        nodes[2].bg_rgba = packRgba(37, 99, 235)
        try apply(frame: 3, dirty: [2])
        XCTAssertEqual(store.indexRebuildCount, 1)

        // Repeated draws & hit-tests reuse store.nodeIndexById with zero rebuilds!
        let view = NveSceneCanvasView(
            store: store,
            surfaceId: 1,
            backgroundColor: .black,
            imageCache: cache
        ).frame(width: 320, height: 200)
        let renderer = ImageRenderer(content: view)
        renderer.scale = 1.0
        let cg = try XCTUnwrap(renderer.cgImage)
        XCTAssertEqual(store.indexRebuildCount, 1)
        XCTAssertEqual(store.hitTest(at: CGPoint(x: 50, y: 60)), 3)
        XCTAssertEqual(store.indexRebuildCount, 1)

        // Verify Image node (210..290, 10..70) painted emerald green (16, 185, 129)
        let imgPx = rgbaPixel(in: cg, x: 250, y: 40)
        XCTAssertEqual(imgPx.0, 16, accuracy: 4)
        XCTAssertEqual(imgPx.1, 185, accuracy: 4)
        XCTAssertEqual(imgPx.2, 129, accuracy: 4)

        // Verify SkiaCanvas node (210..290, 90..130): center circle (250, 110) yellow & corner (220, 100) blue
        let canvasCenter = rgbaPixel(in: cg, x: 250, y: 110)
        XCTAssertEqual(canvasCenter.0, 250, accuracy: 4)
        XCTAssertEqual(canvasCenter.1, 204, accuracy: 4)
        let canvasCorner = rgbaPixel(in: cg, x: 220, y: 100)
        XCTAssertEqual(canvasCorner.2, 235, accuracy: 4)

        // Verify VoiceOver accessibility elements derived from the live scene store
        let a11y = store.accessibilityElements()
        let heading = try XCTUnwrap(a11y.first { $0.id == 2 })
        XCTAssertEqual(heading.label, "Dashboard")
        XCTAssertTrue(heading.isHeading)

        let button = try XCTUnwrap(a11y.first { $0.id == 3 })
        XCTAssertEqual(button.label, "Deploy")
        XCTAssertTrue(button.isInteractive)
        XCTAssertEqual(button.role, .button)

        let secureField = try XCTUnwrap(a11y.first { $0.id == 4 })
        XCTAssertEqual(secureField.label, "Password")
        XCTAssertEqual(secureField.value, String(repeating: "\u{2022}", count: 8))
        XCTAssertTrue(secureField.isTextField)
        XCTAssertTrue(secureField.isSecure)

        let placeholderField = try XCTUnwrap(a11y.first { $0.id == 5 })
        XCTAssertEqual(placeholderField.label, "Search...")
        XCTAssertNil(placeholderField.value)
        XCTAssertTrue(placeholderField.isTextField)
        XCTAssertFalse(placeholderField.isSecure)

        let imageElem = try XCTUnwrap(a11y.first { $0.id == 6 })
        XCTAssertEqual(imageElem.label, "Status Avatar")
        XCTAssertTrue(imageElem.isImage)
    }
}
