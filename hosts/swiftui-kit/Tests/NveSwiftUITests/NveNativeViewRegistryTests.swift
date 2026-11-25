import Foundation
import SwiftUI
import XCTest
@testable import NveSwiftUI

private struct NvnpTestBuilder {
    var data = Data([0x4E, 0x56, 0x4E, 0x50, 1, 0, 0])
    var count: UInt16 = 0

    private mutating func appendLE(_ v: UInt16) {
        data.append(UInt8(v & 0xFF)); data.append(UInt8((v >> 8) & 0xFF))
    }
    private mutating func appendLE(_ v: UInt32) {
        for shift in stride(from: 0, to: 32, by: 8) { data.append(UInt8((v >> shift) & 0xFF)) }
    }
    private mutating func appendLE(_ v: UInt64) {
        for shift in stride(from: 0, to: 64, by: 8) { data.append(UInt8((v >> shift) & 0xFF)) }
    }

    mutating func addNull(_ id: UInt16) { appendLE(id); data.append(0); count += 1 }
    mutating func addBool(_ id: UInt16, _ v: Bool) { appendLE(id); data.append(1); data.append(v ? 1 : 0); count += 1 }
    mutating func addU32(_ id: UInt16, _ v: UInt32) { appendLE(id); data.append(2); appendLE(v); count += 1 }
    mutating func addI64(_ id: UInt16, _ v: Int64) { appendLE(id); data.append(3); appendLE(UInt64(bitPattern: v)); count += 1 }
    mutating func addU64(_ id: UInt16, _ v: UInt64) { appendLE(id); data.append(4); appendLE(v); count += 1 }
    mutating func addF32(_ id: UInt16, _ v: Float) { appendLE(id); data.append(5); appendLE(v.bitPattern); count += 1 }
    mutating func addF64(_ id: UInt16, _ v: Double) { appendLE(id); data.append(6); appendLE(v.bitPattern); count += 1 }
    mutating func addStr(_ id: UInt16, _ s: String) {
        let utf8 = Data(s.utf8)
        appendLE(id); data.append(7); appendLE(UInt32(utf8.count)); data.append(utf8); count += 1
    }
    mutating func addBytes(_ id: UInt16, _ b: Data) {
        appendLE(id); data.append(8); appendLE(UInt32(b.count)); data.append(b); count += 1
    }

    func finish() -> Data {
        var out = data
        out[5] = UInt8(count & 0xFF)
        out[6] = UInt8((count >> 8) & 0xFF)
        return out
    }
}

private struct NvncTestBuilder {
    var data: Data
    var count: UInt8 = 0

    init(opcode: UInt16) {
        data = Data([0x4E, 0x56, 0x4E, 0x43, 1, UInt8(opcode & 0xFF), UInt8((opcode >> 8) & 0xFF), 0])
    }

    private mutating func appendLE(_ v: UInt32) {
        for shift in stride(from: 0, to: 32, by: 8) { data.append(UInt8((v >> shift) & 0xFF)) }
    }
    private mutating func appendLE(_ v: UInt64) {
        for shift in stride(from: 0, to: 64, by: 8) { data.append(UInt8((v >> shift) & 0xFF)) }
    }

    mutating func argU64(_ v: UInt64) { data.append(4); appendLE(v); count += 1 }
    mutating func argF32(_ v: Float) { data.append(5); appendLE(v.bitPattern); count += 1 }
    mutating func argStr(_ s: String) {
        let utf8 = Data(s.utf8)
        data.append(7); appendLE(UInt32(utf8.count)); data.append(utf8); count += 1
    }

    func finish() -> Data {
        var out = data
        out[7] = count
        return out
    }
}

final class NveNativeViewRegistryTests: XCTestCase {
    func testNvnpAllValueTypesAndStarterSpecs() throws {
        var all = NvnpTestBuilder()
        all.addNull(1)
        all.addBool(2, true)
        all.addU32(3, 0xDEAD_BEEF)
        all.addI64(4, -9_876_543_210)
        all.addU64(5, 0x1122_3344_5566_7788)
        all.addF32(6, 0.75)
        all.addF64(7, 3.1415926535)
        all.addStr(8, "hello UTF-8 ✓ 🚀")
        all.addBytes(9, Data([0xAA, 0xBB, 0xCC]))

        let dec = try NvnpDecoder.parse(all.finish())
        XCTAssertEqual(dec.fieldCount, 9)
        XCTAssertEqual(dec.find(1)?.isNull, true)
        XCTAssertEqual(dec.find(2)?.asBool, true)
        XCTAssertEqual(dec.find(3)?.asU32, 0xDEAD_BEEF)
        XCTAssertEqual(dec.find(4)?.asI64, -9_876_543_210)
        XCTAssertEqual(dec.find(5)?.asU64, 0x1122_3344_5566_7788)
        XCTAssertEqual(dec.find(6)?.asF32 ?? 0, 0.75, accuracy: 1e-6)
        XCTAssertEqual(dec.find(7)?.asF64 ?? 0, 3.1415926535, accuracy: 1e-9)
        XCTAssertEqual(dec.find(8)?.asStr, "hello UTF-8 ✓ 🚀")
        XCTAssertEqual(dec.find(9)?.asBytes, Data([0xAA, 0xBB, 0xCC]))

        // WebElement packed attrs
        var packed = Data()
        func pushStr(_ s: String) {
            let b = Data(s.utf8)
            var len = UInt32(b.count).littleEndian
            packed.append(Data(bytes: &len, count: 4))
            packed.append(b)
        }
        pushStr("viewBox"); pushStr("0 0 24 24")
        pushStr("aria-hidden"); pushStr("true")
        var webElem = NvnpTestBuilder()
        webElem.addStr(1, "svg")
        webElem.addU32(2, 2)
        webElem.addBytes(3, packed)
        let webElemProps = try NveWebElementProps.decode(from: try NvnpDecoder.parse(webElem.finish()))
        XCTAssertEqual(webElemProps.tag, "svg")
        XCTAssertEqual(webElemProps.attrs.count, 2)
        XCTAssertEqual(webElemProps.attrs[0].key, "viewBox")
        XCTAssertEqual(webElemProps.attrs[0].value, "0 0 24 24")
    }

    func testCorruptAndAdversarialBuffersRejected() {
        XCTAssertThrowsError(try NvnpDecoder.parse(Data([0x4E, 0x56, 0x4E]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .truncated)
        }
        XCTAssertThrowsError(try NvncDecoder.parse(Data([0x4E, 0x56, 0x4E]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .truncated)
        }
        XCTAssertThrowsError(try NvnpDecoder.parse(Data([0x42, 0x41, 0x44, 0x21, 1, 0, 0]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .invalidMagic)
        }
        XCTAssertThrowsError(try NvncDecoder.parse(Data([0x42, 0x41, 0x44, 0x21, 1, 0, 0, 0]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .invalidMagic)
        }
        XCTAssertThrowsError(try NvnpDecoder.parse(Data([0x4E, 0x56, 0x4E, 0x50, 2, 0, 0]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .unsupportedVersion)
        }
        XCTAssertThrowsError(try NvncDecoder.parse(Data([0x4E, 0x56, 0x4E, 0x43, 9, 1, 0, 0]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .unsupportedVersion)
        }
        // Invalid bool byte (2)
        XCTAssertThrowsError(try NvnpDecoder.parse(Data([0x4E, 0x56, 0x4E, 0x50, 1, 1, 0, 1, 0, 1, 2]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .invalidBool)
        }
        // Invalid tag (99)
        XCTAssertThrowsError(try NvnpDecoder.parse(Data([0x4E, 0x56, 0x4E, 0x50, 1, 1, 0, 1, 0, 99]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .invalidTag)
        }
        // Invalid UTF-8: overlong 0xC0 0xAF and surrogate 0xED 0xA0 0x80
        XCTAssertThrowsError(try NvnpDecoder.parse(Data([0x4E, 0x56, 0x4E, 0x50, 1, 1, 0, 1, 0, 7, 2, 0, 0, 0, 0xC0, 0xAF]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .invalidUtf8)
        }
        XCTAssertThrowsError(try NvncDecoder.parse(Data([0x4E, 0x56, 0x4E, 0x43, 1, 1, 0, 1, 7, 3, 0, 0, 0, 0xED, 0xA0, 0x80]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .invalidUtf8)
        }
        // Trailing bytes
        XCTAssertThrowsError(try NvnpDecoder.parse(Data([0x4E, 0x56, 0x4E, 0x50, 1, 0, 0, 0xFF]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .trailingBytes)
        }
        XCTAssertThrowsError(try NvncDecoder.parse(Data([0x4E, 0x56, 0x4E, 0x43, 1, 2, 0, 0, 0x00]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .trailingBytes)
        }
        // OOM count bombs
        XCTAssertThrowsError(try NvnpDecoder.parse(Data([0x4E, 0x56, 0x4E, 0x50, 1, 0xFF, 0x7F]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .tooManyFields)
        }
        XCTAssertThrowsError(try NvncDecoder.parse(Data([0x4E, 0x56, 0x4E, 0x43, 1, 1, 0, 200]))) {
            XCTAssertEqual($0 as? NveNativeViewCodecError, .tooManyFields)
        }
    }

    func testWildcardPrecedenceUrlSafetyAndDefaultSwiftUIAdapters() throws {
        XCTAssertTrue(isSafeNveWebViewURL("https://nve.dev/docs"))
        XCTAssertTrue(isSafeNveWebViewURL("http://localhost:8080"))
        XCTAssertTrue(isSafeNveWebViewURL("  ABOUT:BLANK \n"))
        XCTAssertFalse(isSafeNveWebViewURL("javascript:alert(1)"))
        XCTAssertFalse(isSafeNveWebViewURL("JaVa\tScRiPt:alert(1)"))
        XCTAssertFalse(isSafeNveWebViewURL("vbscript:msgbox(1)"))
        XCTAssertFalse(isSafeNveWebViewURL("data:text/html,<b>x</b>"))
        XCTAssertFalse(isSafeNveWebViewURL(""))

        let registry = NveNativeViewRegistry()
        registry.registerDefaultSwiftUINativeViews()

        // Wildcard precedence: exact > longest prefix > shorter prefix; "web." must not match "web.*"
        final class TagBox { let label: String; init(_ label: String) { self.label = label } }
        registry.registerAdapter("web.*", adapter: NveNativeViewAdapter(create: { _, _, _ in TagBox("web-wild") }))
        registry.registerAdapter("web.media.*", adapter: NveNativeViewAdapter(create: { _, _, _ in TagBox("media-wild") }))
        registry.registerAdapter("web.media.audio", adapter: NveNativeViewAdapter(create: { _, _, _ in TagBox("exact-audio") }))

        XCTAssertTrue(registry.supports("web.svg"))
        XCTAssertFalse(registry.supports("web."))
        XCTAssertEqual((try registry.mountOrUpdate(nodeId: 1, kind: "web.svg") as? TagBox)?.label, "web-wild")
        XCTAssertEqual((try registry.mountOrUpdate(nodeId: 2, kind: "web.media.video") as? TagBox)?.label, "media-wild")
        XCTAssertEqual((try registry.mountOrUpdate(nodeId: 3, kind: "web.media.audio") as? TagBox)?.label, "exact-audio")

        // BlurSurface (canonical + alias)
        var blur = NvnpTestBuilder()
        blur.addU32(1, NveBlurMaterial.ultraThin.rawValue)
        blur.addF32(2, 0.65)
        blur.addU32(3, 0x11223388)
        let blurState = try registry.mountOrUpdate(nodeId: 10, kind: NveNativeViewConstants.kindBlurAlias, nvnpData: blur.finish()) as? NveSwiftUIBlurSurfaceState
        XCTAssertNotNil(blurState)
        XCTAssertEqual(blurState?.props.material, .ultraThin)
        XCTAssertEqual(blurState?.props.clampedIntensity ?? 0, 0.65, accuracy: 1e-5)
        XCTAssertEqual(blurState?.props.tint, 0x11223388)
        XCTAssertNotNil(registry.view(for: 10))

        // WebView + NVNC commands + unsafe URL rejection
        var wv = NvnpTestBuilder()
        wv.addStr(1, "https://swiftui.nve.dev")
        wv.addBool(2, true)
        let wvState = try registry.mountOrUpdate(nodeId: 11, kind: NveNativeViewConstants.kindWebView, nvnpData: wv.finish()) as? NveSwiftUIWebViewState
        XCTAssertEqual(wvState?.lastRequestedUrl, "https://swiftui.nve.dev")
        XCTAssertNotNil(registry.view(for: 11))

        var badNav = NvncTestBuilder(opcode: 1); badNav.argStr("javascript:alert(1)")
        XCTAssertFalse(try registry.dispatchCommand(nodeId: 11, commandName: "navigate", nvncData: badNav.finish()))
        XCTAssertEqual(wvState?.lastRequestedUrl, "https://swiftui.nve.dev")

        var okNav = NvncTestBuilder(opcode: 1); okNav.argStr("https://guide.nve.dev")
        XCTAssertTrue(try registry.dispatchCommand(nodeId: 11, commandName: "navigate", nvncData: okNav.finish()))
        XCTAssertTrue(try registry.dispatchCommand(nodeId: 11, commandName: "reload", nvncData: NvncTestBuilder(opcode: 2).finish()))
        XCTAssertTrue(try registry.dispatchCommand(nodeId: 11, commandName: "go_back", nvncData: NvncTestBuilder(opcode: 3).finish()))
        var eval = NvncTestBuilder(opcode: 5); eval.argStr("document.title")
        XCTAssertTrue(try registry.dispatchCommand(nodeId: 11, commandName: "evaluate_script", nvncData: eval.finish()))
        XCTAssertEqual(wvState?.lastRequestedUrl, "https://guide.nve.dev")
        XCTAssertEqual(wvState?.reloadCount, 1)
        XCTAssertEqual(wvState?.goBackCount, 1)
        XCTAssertEqual(wvState?.lastEvaluatedScript, "document.title")

        // VideoPlayer + NVNC commands
        var vid = NvnpTestBuilder()
        vid.addStr(1, "https://cdn.nve.dev/clip.mp4")
        vid.addBool(2, false); vid.addBool(3, true); vid.addBool(4, true); vid.addBool(5, true)
        let vidState = try registry.mountOrUpdate(nodeId: 12, kind: NveNativeViewConstants.kindVideoAlias, nvnpData: vid.finish()) as? NveSwiftUIVideoPlayerState
        XCTAssertEqual(vidState?.props.sourceUri, "https://cdn.nve.dev/clip.mp4")
        XCTAssertNotNil(registry.view(for: 12))
        XCTAssertTrue(try registry.dispatchCommand(nodeId: 12, commandName: "play", nvncData: NvncTestBuilder(opcode: 1).finish()))
        var seek = NvncTestBuilder(opcode: 3); seek.argU64(42_500)
        XCTAssertTrue(try registry.dispatchCommand(nodeId: 12, commandName: "seek_ms", nvncData: seek.finish()))
        var vol = NvncTestBuilder(opcode: 4); vol.argF32(0.4)
        XCTAssertTrue(try registry.dispatchCommand(nodeId: 12, commandName: "set_volume", nvncData: vol.finish()))
        XCTAssertEqual(vidState?.isPlaying, true)
        XCTAssertEqual(vidState?.positionMs, 42_500)
        XCTAssertEqual(vidState?.volume ?? 0, 0.4, accuracy: 1e-5)

        registry.unmountAll()
        XCTAssertEqual(registry.mountedCount, 0)
        XCTAssertEqual(blurState?.unmounted, true)
        XCTAssertEqual(wvState?.unmounted, true)
        XCTAssertEqual(vidState?.unmounted, true)
    }
}
