import Foundation
import SwiftUI
#if canImport(WebKit) && !os(tvOS) && !os(watchOS)
import WebKit
#endif
#if canImport(AppKit)
import AppKit
#elseif canImport(UIKit) && !os(watchOS)
import UIKit
#endif

public enum NveNativeViewConstants {
    public static let nvnpMagic: [UInt8] = [0x4E, 0x56, 0x4E, 0x50]
    public static let nvnpVersion: UInt8 = 1, nvnpHeaderSize = 7, maxPropFields = 1024
    public static let nvncMagic: [UInt8] = [0x4E, 0x56, 0x4E, 0x43]
    public static let nvncVersion: UInt8 = 1, nvncHeaderSize = 8, maxCmdArgs = 64
    public static let kindPropKey = "nve.native_kind", kindPropKeyAlias = "nve.native.kind"
    public static let propsPropKey = "nve.native_props", propsPropKeyAlias = "nve.native.props"
    public static let kindBlurSurface = "nve.blur_surface", kindBlurAlias = "nve.blur"
    public static let kindWebView = "nve.web_view", kindWebViewAlias = "nve.webview"
    public static let kindVideoPlayer = "nve.video_player", kindVideoAlias = "nve.video"
    public static let kindWebWildcard = "web.*"
}

public enum NveNativeViewCodecError: Error, Equatable, Sendable {
    case truncated, invalidMagic, unsupportedVersion, invalidTag, invalidBool, invalidEnum
    case invalidUtf8, missingField, missingArg, unexpectedArgCount, typeMismatch, trailingBytes
    case unknownCommandOpcode, tooManyFields
}

public enum NveNativeValue: Equatable, Sendable {
    case null, bool(Bool), u32(UInt32), i64(Int64), u64(UInt64), f32(Float), f64(Double), str(String), bytes(Data)
    public var isNull: Bool { if case .null = self { return true }; return false }
    public var asBool: Bool? { if case let .bool(v) = self { return v }; return nil }
    public var asU32: UInt32? { if case let .u32(v) = self { return v }; return nil }
    public var asI64: Int64? { if case let .i64(v) = self { return v }; return nil }
    public var asU64: UInt64? { if case let .u64(v) = self { return v }; return nil }
    public var asF32: Float? { if case let .f32(v) = self { return v }; return nil }
    public var asF64: Double? { if case let .f64(v) = self { return v }; return nil }
    public var asStr: String? { if case let .str(v) = self { return v }; return nil }
    public var asBytes: Data? { if case let .bytes(v) = self { return v }; return nil }
}

private func nveIsValidUtf8(_ b: UnsafeBufferPointer<UInt8>, start: Int, count: Int) -> Bool {
    var i = start; let end = start + count
    func cont(_ idx: Int) -> Bool { (b[idx] & 0xC0) == 0x80 }
    while i < end {
        let b0 = b[i]
        if b0 <= 0x7F { i += 1 }
        else if b0 >= 0xC2 && b0 <= 0xDF { guard i + 1 < end && cont(i + 1) else { return false }; i += 2 }
        else if b0 == 0xE0 { guard i + 2 < end && b[i + 1] >= 0xA0 && b[i + 1] <= 0xBF && cont(i + 2) else { return false }; i += 3 }
        else if (b0 >= 0xE1 && b0 <= 0xEC) || (b0 >= 0xEE && b0 <= 0xEF) { guard i + 2 < end && cont(i + 1) && cont(i + 2) else { return false }; i += 3 }
        else if b0 == 0xED { guard i + 2 < end && b[i + 1] >= 0x80 && b[i + 1] <= 0x9F && cont(i + 2) else { return false }; i += 3 }
        else if b0 == 0xF0 { guard i + 3 < end && b[i + 1] >= 0x90 && b[i + 1] <= 0xBF && cont(i + 2) && cont(i + 3) else { return false }; i += 4 }
        else if b0 >= 0xF1 && b0 <= 0xF3 { guard i + 3 < end && cont(i + 1) && cont(i + 2) && cont(i + 3) else { return false }; i += 4 }
        else if b0 == 0xF4 { guard i + 3 < end && b[i + 1] >= 0x80 && b[i + 1] <= 0x8F && cont(i + 2) && cont(i + 3) else { return false }; i += 4 }
        else { return false }
    }
    return true
}

@inline(__always) private func readU16LE(_ b: UnsafeBufferPointer<UInt8>, _ o: Int) -> UInt16 { UInt16(b[o]) | (UInt16(b[o + 1]) << 8) }
@inline(__always) private func readU32LE(_ b: UnsafeBufferPointer<UInt8>, _ o: Int) -> UInt32 {
    UInt32(b[o]) | (UInt32(b[o + 1]) << 8) | (UInt32(b[o + 2]) << 16) | (UInt32(b[o + 3]) << 24)
}
@inline(__always) private func readU64LE(_ b: UnsafeBufferPointer<UInt8>, _ o: Int) -> UInt64 {
    UInt64(readU32LE(b, o)) | (UInt64(readU32LE(b, o + 4)) << 32)
}

private func decodeWireValue(_ b: UnsafeBufferPointer<UInt8>, pos: inout Int) throws -> NveNativeValue {
    guard pos < b.count else { throw NveNativeViewCodecError.truncated }
    let tag = b[pos]; pos += 1
    switch tag {
    case 0: return .null
    case 1:
        guard b.count - pos >= 1 else { throw NveNativeViewCodecError.truncated }
        let v = b[pos]; pos += 1; guard v <= 1 else { throw NveNativeViewCodecError.invalidBool }; return .bool(v == 1)
    case 2:
        guard b.count - pos >= 4 else { throw NveNativeViewCodecError.truncated }
        let v = readU32LE(b, pos); pos += 4; return .u32(v)
    case 3:
        guard b.count - pos >= 8 else { throw NveNativeViewCodecError.truncated }
        let v = Int64(bitPattern: readU64LE(b, pos)); pos += 8; return .i64(v)
    case 4:
        guard b.count - pos >= 8 else { throw NveNativeViewCodecError.truncated }
        let v = readU64LE(b, pos); pos += 8; return .u64(v)
    case 5:
        guard b.count - pos >= 4 else { throw NveNativeViewCodecError.truncated }
        let v = Float(bitPattern: readU32LE(b, pos)); pos += 4; return .f32(v)
    case 6:
        guard b.count - pos >= 8 else { throw NveNativeViewCodecError.truncated }
        let v = Double(bitPattern: readU64LE(b, pos)); pos += 8; return .f64(v)
    case 7, 8:
        guard b.count - pos >= 4 else { throw NveNativeViewCodecError.truncated }
        let n = Int(readU32LE(b, pos)); pos += 4
        guard n >= 0 && b.count - pos >= n else { throw NveNativeViewCodecError.truncated }
        if tag == 7 {
            guard nveIsValidUtf8(b, start: pos, count: n) else { throw NveNativeViewCodecError.invalidUtf8 }
            let s = String(decoding: UnsafeBufferPointer(start: b.baseAddress?.advanced(by: pos), count: n), as: UTF8.self)
            pos += n; return .str(s)
        }
        let d = n == 0 ? Data() : Data(bytes: b.baseAddress!.advanced(by: pos), count: n); pos += n; return .bytes(d)
    default: throw NveNativeViewCodecError.invalidTag
    }
}

public struct NvnpDecoder: Sendable {
    public let fields: [(id: UInt16, value: NveNativeValue)]
    public var fieldCount: Int { fields.count }
    public var isEmpty: Bool { fields.isEmpty }

    public static func parse(_ data: Data) throws -> NvnpDecoder {
        try data.withUnsafeBytes { raw in
            let b = raw.bindMemory(to: UInt8.self)
            guard b.count >= NveNativeViewConstants.nvnpHeaderSize else { throw NveNativeViewCodecError.truncated }
            for i in 0..<4 where b[i] != NveNativeViewConstants.nvnpMagic[i] { throw NveNativeViewCodecError.invalidMagic }
            guard b[4] == NveNativeViewConstants.nvnpVersion else { throw NveNativeViewCodecError.unsupportedVersion }
            let count = Int(readU16LE(b, 5))
            guard count <= NveNativeViewConstants.maxPropFields else { throw NveNativeViewCodecError.tooManyFields }
            var pos = NveNativeViewConstants.nvnpHeaderSize
            guard count <= (b.count - pos) / 3 else { throw NveNativeViewCodecError.truncated }
            var out: [(UInt16, NveNativeValue)] = []; out.reserveCapacity(count)
            for _ in 0..<count {
                guard b.count - pos >= 2 else { throw NveNativeViewCodecError.truncated }
                let id = readU16LE(b, pos); pos += 2; out.append((id, try decodeWireValue(b, pos: &pos)))
            }
            guard pos == b.count else { throw NveNativeViewCodecError.trailingBytes }
            return NvnpDecoder(fields: out)
        }
    }

    public func find(_ fieldId: UInt16) -> NveNativeValue? { fields.first(where: { $0.id == fieldId })?.value }
    private func req(_ id: UInt16) throws -> NveNativeValue { guard let v = find(id) else { throw NveNativeViewCodecError.missingField }; return v }
    public func requireBool(_ id: UInt16) throws -> Bool { guard let v = try req(id).asBool else { throw NveNativeViewCodecError.typeMismatch }; return v }
    public func requireU32(_ id: UInt16) throws -> UInt32 { guard let v = try req(id).asU32 else { throw NveNativeViewCodecError.typeMismatch }; return v }
    public func requireOptU32(_ id: UInt16) throws -> UInt32? {
        let v = try req(id); if v.isNull { return nil }; guard let n = v.asU32 else { throw NveNativeViewCodecError.typeMismatch }; return n
    }
    public func requireF32(_ id: UInt16) throws -> Float { guard let v = try req(id).asF32 else { throw NveNativeViewCodecError.typeMismatch }; return v }
    public func requireStr(_ id: UInt16) throws -> String { guard let v = try req(id).asStr else { throw NveNativeViewCodecError.typeMismatch }; return v }
    public func requireBytes(_ id: UInt16) throws -> Data { guard let v = try req(id).asBytes else { throw NveNativeViewCodecError.typeMismatch }; return v }
}

public struct NvncDecoder: Sendable {
    public let opcode: UInt16
    public let args: [NveNativeValue]
    public var argCount: Int { args.count }

    public static func parse(_ data: Data) throws -> NvncDecoder {
        try data.withUnsafeBytes { raw in
            let b = raw.bindMemory(to: UInt8.self)
            guard b.count >= NveNativeViewConstants.nvncHeaderSize else { throw NveNativeViewCodecError.truncated }
            for i in 0..<4 where b[i] != NveNativeViewConstants.nvncMagic[i] { throw NveNativeViewCodecError.invalidMagic }
            guard b[4] == NveNativeViewConstants.nvncVersion else { throw NveNativeViewCodecError.unsupportedVersion }
            let op = readU16LE(b, 5), count = Int(b[7])
            guard count <= NveNativeViewConstants.maxCmdArgs else { throw NveNativeViewCodecError.tooManyFields }
            var pos = NveNativeViewConstants.nvncHeaderSize
            guard count <= b.count - pos else { throw NveNativeViewCodecError.truncated }
            var out: [NveNativeValue] = []; out.reserveCapacity(count)
            for _ in 0..<count { out.append(try decodeWireValue(b, pos: &pos)) }
            guard pos == b.count else { throw NveNativeViewCodecError.trailingBytes }
            return NvncDecoder(opcode: op, args: out)
        }
    }

    public func expectArgCount(_ n: Int) throws { guard args.count == n else { throw NveNativeViewCodecError.unexpectedArgCount } }
    public func arg(at i: Int) -> NveNativeValue? { i >= 0 && i < args.count ? args[i] : nil }
    private func req(_ i: Int) throws -> NveNativeValue { guard let v = arg(at: i) else { throw NveNativeViewCodecError.missingArg }; return v }
    public func requireStr(_ i: Int) throws -> String { guard let v = try req(i).asStr else { throw NveNativeViewCodecError.typeMismatch }; return v }
    public func requireU64(_ i: Int) throws -> UInt64 { guard let v = try req(i).asU64 else { throw NveNativeViewCodecError.typeMismatch }; return v }
    public func requireF32(_ i: Int) throws -> Float { guard let v = try req(i).asF32 else { throw NveNativeViewCodecError.typeMismatch }; return v }
}

private func normalizeSchemeCheck(_ raw: String) -> String {
    var out = ""; out.reserveCapacity(raw.utf8.count)
    for byte in raw.utf8 where byte > 0x20 && byte != 0x7F {
        out.unicodeScalars.append(UnicodeScalar((byte >= 0x41 && byte <= 0x5A) ? byte + 32 : byte))
    }
    return out
}

public func isSafeNveWebViewURL(_ url: String) -> Bool {
    let n = normalizeSchemeCheck(url); return !n.isEmpty && (n.hasPrefix("https://") || n.hasPrefix("http://") || n == "about:blank")
}
public func isSafeNveMediaURI(_ uri: String) -> Bool {
    let n = normalizeSchemeCheck(uri); return !n.isEmpty && (n.hasPrefix("https://") || n.hasPrefix("http://") || n.hasPrefix("file://"))
}
public func nveColorFromRRGGBBAA(_ rgba: UInt32) -> Color {
    Color(.sRGB, red: Double((rgba >> 24) & 0xFF) / 255.0, green: Double((rgba >> 16) & 0xFF) / 255.0,
          blue: Double((rgba >> 8) & 0xFF) / 255.0, opacity: Double(rgba & 0xFF) / 255.0)
}

public enum NveBlurMaterial: UInt32, Equatable, Sendable {
    case ultraThin = 0, thin = 1, regular = 2, thick = 3, chrome = 4, systemMaterial = 5
    @available(watchOS 10.0, *)
    public var swiftUIMaterial: Material {
        switch self {
        case .ultraThin: return .ultraThinMaterial; case .thin: return .thinMaterial
        case .regular, .systemMaterial: return .regularMaterial; case .thick: return .thickMaterial
        case .chrome:
            #if os(tvOS) || os(watchOS)
            return .thickMaterial
            #else
            return .bar
            #endif
        }
    }
}

public struct NveBlurSurfaceProps: Equatable, Sendable {
    public var material: NveBlurMaterial; public var intensity: Float; public var tint: UInt32?
    public init(material: NveBlurMaterial = .regular, intensity: Float = 1.0, tint: UInt32? = nil) { self.material = material; self.intensity = intensity; self.tint = tint }
    public var clampedIntensity: Float { intensity.isFinite ? min(max(intensity, 0.0), 1.0) : 1.0 }
    public static func decode(from d: NvnpDecoder) throws -> NveBlurSurfaceProps {
        guard let m = NveBlurMaterial(rawValue: try d.requireU32(1)) else { throw NveNativeViewCodecError.invalidEnum }
        return NveBlurSurfaceProps(material: m, intensity: try d.requireF32(2), tint: try d.requireOptU32(3))
    }
}

public struct NveWebViewProps: Equatable, Sendable {
    public var url: String; public var allowsBackForward: Bool
    public init(url: String = "", allowsBackForward: Bool = true) { self.url = url; self.allowsBackForward = allowsBackForward }
    public static func decode(from d: NvnpDecoder) throws -> NveWebViewProps { NveWebViewProps(url: try d.requireStr(1), allowsBackForward: try d.requireBool(2)) }
}

public enum NveWebViewCommand: Equatable, Sendable {
    case navigate(String), reload, goBack, goForward, evaluateScript(String)
    public static func decode(from d: NvncDecoder) throws -> NveWebViewCommand {
        switch d.opcode {
        case 1: try d.expectArgCount(1); return .navigate(try d.requireStr(0))
        case 2: try d.expectArgCount(0); return .reload
        case 3: try d.expectArgCount(0); return .goBack
        case 4: try d.expectArgCount(0); return .goForward
        case 5: try d.expectArgCount(1); return .evaluateScript(try d.requireStr(0))
        default: throw NveNativeViewCodecError.unknownCommandOpcode
        }
    }
}

public struct NveVideoPlayerProps: Equatable, Sendable {
    public var sourceUri: String; public var autoplay, muted, looping, controls: Bool
    public init(sourceUri: String = "", autoplay: Bool = false, muted: Bool = false, looping: Bool = false, controls: Bool = true) {
        self.sourceUri = sourceUri; self.autoplay = autoplay; self.muted = muted; self.looping = looping; self.controls = controls
    }
    public static func decode(from d: NvnpDecoder) throws -> NveVideoPlayerProps {
        NveVideoPlayerProps(sourceUri: try d.requireStr(1), autoplay: try d.requireBool(2), muted: try d.requireBool(3), looping: try d.requireBool(4), controls: try d.requireBool(5))
    }
}

public enum NveVideoPlayerCommand: Equatable, Sendable {
    case play, pause, seekMs(UInt64), setVolume(Float)
    public static func decode(from d: NvncDecoder) throws -> NveVideoPlayerCommand {
        switch d.opcode {
        case 1: try d.expectArgCount(0); return .play
        case 2: try d.expectArgCount(0); return .pause
        case 3: try d.expectArgCount(1); return .seekMs(try d.requireU64(0))
        case 4: try d.expectArgCount(1); return .setVolume(try d.requireF32(0))
        default: throw NveNativeViewCodecError.unknownCommandOpcode
        }
    }
}

public struct NveWebElementProps: Equatable, Sendable {
    public var tag: String; public var attrs: [(key: String, value: String)]
    public static func == (l: Self, r: Self) -> Bool {
        l.tag == r.tag && l.attrs.count == r.attrs.count && zip(l.attrs, r.attrs).allSatisfy { $0.key == $1.key && $0.value == $1.value }
    }
    public static func decode(from d: NvnpDecoder) throws -> NveWebElementProps {
        let tag = try d.requireStr(1), count = Int(try d.requireU32(2)), raw = try d.requireBytes(3)
        guard count <= raw.count / 8 else { throw NveNativeViewCodecError.truncated }
        var attrs: [(String, String)] = []; attrs.reserveCapacity(count)
        try raw.withUnsafeBytes { ptr in
            let b = ptr.bindMemory(to: UInt8.self); var pos = 0
            func readStr() throws -> String {
                guard b.count - pos >= 4 else { throw NveNativeViewCodecError.truncated }
                let n = Int(readU32LE(b, pos)); pos += 4
                guard n >= 0 && b.count - pos >= n else { throw NveNativeViewCodecError.truncated }
                guard nveIsValidUtf8(b, start: pos, count: n) else { throw NveNativeViewCodecError.invalidUtf8 }
                let s = String(decoding: UnsafeBufferPointer(start: b.baseAddress?.advanced(by: pos), count: n), as: UTF8.self)
                pos += n; return s
            }
            for _ in 0..<count { attrs.append((try readStr(), try readStr())) }
            guard pos == b.count else { throw NveNativeViewCodecError.trailingBytes }
        }
        return NveWebElementProps(tag: tag, attrs: attrs)
    }
}

public struct NveNativeViewAdapter {
    public var create: (UInt32, String, NvnpDecoder?) -> AnyObject
    public var updateProps: ((AnyObject, UInt32, NvnpDecoder) -> Void)?
    public var dispatchCommand: ((AnyObject, UInt32, String, NvncDecoder) -> Bool)?
    public var unmount: ((AnyObject, UInt32) -> Void)?
    public var makeView: ((AnyObject) -> AnyView)?
    public init(
        create: @escaping (UInt32, String, NvnpDecoder?) -> AnyObject,
        updateProps: ((AnyObject, UInt32, NvnpDecoder) -> Void)? = nil,
        dispatchCommand: ((AnyObject, UInt32, String, NvncDecoder) -> Bool)? = nil,
        unmount: ((AnyObject, UInt32) -> Void)? = nil,
        makeView: ((AnyObject) -> AnyView)? = nil
    ) { self.create = create; self.updateProps = updateProps; self.dispatchCommand = dispatchCommand; self.unmount = unmount; self.makeView = makeView }
}

private func trimAsciiWs(_ s: String) -> String {
    var a = s.startIndex, b = s.endIndex
    func ws(_ c: Character) -> Bool { c == " " || c == "\t" || c == "\r" || c == "\n" }
    while a < b && ws(s[a]) { a = s.index(after: a) }
    while b > a && ws(s[s.index(before: b)]) { b = s.index(before: b) }
    return String(s[a..<b])
}

public final class NveNativeViewRegistry {
    private struct Entry { let kind: String; let instance: AnyObject; let adapter: NveNativeViewAdapter }
    private var exact: [String: NveNativeViewAdapter] = [:]
    private var wildcards: [(prefix: String, adapter: NveNativeViewAdapter)] = []
    private var mounted: [UInt32: Entry] = [:]
    public init() {}
    deinit { unmountAll() }

    public func registerAdapter(_ pattern: String, adapter: NveNativeViewAdapter) {
        let t = trimAsciiWs(pattern); guard !t.isEmpty else { return }
        if t.hasSuffix("*") {
            let p = String(t.dropLast())
            if let i = wildcards.firstIndex(where: { $0.prefix == p }) { wildcards[i].adapter = adapter; return }
            wildcards.append((p, adapter))
            wildcards.sort { $0.prefix.utf8.count != $1.prefix.utf8.count ? $0.prefix.utf8.count > $1.prefix.utf8.count : $0.prefix < $1.prefix }
        } else { exact[t] = adapter }
    }
    @discardableResult public func unregisterAdapter(_ pattern: String) -> Bool {
        let t = trimAsciiWs(pattern); guard !t.isEmpty else { return false }
        if t.hasSuffix("*") {
            let p = String(t.dropLast()), n = wildcards.count
            wildcards.removeAll { $0.prefix == p }; return wildcards.count != n
        }
        return exact.removeValue(forKey: t) != nil
    }
    public func findAdapter(for kind: String) -> NveNativeViewAdapter? {
        let t = trimAsciiWs(kind); guard !t.isEmpty else { return nil }
        if let hit = exact[t] { return hit }
        for e in wildcards where e.prefix.isEmpty || (t.utf8.count > e.prefix.utf8.count && t.hasPrefix(e.prefix)) { return e.adapter }
        return nil
    }
    public func supports(_ kind: String) -> Bool { findAdapter(for: kind) != nil }

    @discardableResult
    public func mountOrUpdate(nodeId: UInt32, kind: String, nvnpData: Data = Data()) throws -> AnyObject? {
        guard let adapter = findAdapter(for: kind) else { return nil }
        let dec: NvnpDecoder? = nvnpData.isEmpty ? nil : try NvnpDecoder.parse(nvnpData)
        if let prev = mounted[nodeId], prev.kind != kind { unmount(nodeId: nodeId) }
        let entry = mounted[nodeId] ?? { let e = Entry(kind: kind, instance: adapter.create(nodeId, kind, dec), adapter: adapter); mounted[nodeId] = e; return e }()
        if let dec { entry.adapter.updateProps?(entry.instance, nodeId, dec) }
        return entry.instance
    }
    @discardableResult
    public func dispatchCommand(nodeId: UInt32, commandName: String, nvncData: Data) throws -> Bool {
        guard let e = mounted[nodeId], let fn = e.adapter.dispatchCommand else { return false }
        return fn(e.instance, nodeId, commandName, try NvncDecoder.parse(nvncData))
    }
    @discardableResult public func unmount(nodeId: UInt32) -> Bool {
        guard let e = mounted.removeValue(forKey: nodeId) else { return false }
        e.adapter.unmount?(e.instance, nodeId); return true
    }
    public func unmountAll() { for (id, e) in mounted { e.adapter.unmount?(e.instance, id) }; mounted.removeAll() }
    public func instance(for nodeId: UInt32) -> AnyObject? { mounted[nodeId]?.instance }
    public func kind(for nodeId: UInt32) -> String? { mounted[nodeId]?.kind }
    public var mountedCount: Int { mounted.count }
    public func view(for nodeId: UInt32) -> AnyView? { mounted[nodeId].flatMap { $0.adapter.makeView?($0.instance) } }
}

public final class NveSwiftUIBlurSurfaceState: ObservableObject {
    @Published public private(set) var props = NveBlurSurfaceProps()
    public private(set) var unmounted = false
    public init() {}
    public func apply(_ p: NveBlurSurfaceProps) { props = p }
    public func markUnmounted() { unmounted = true }
}

public struct NveSwiftUIBlurSurfaceView: View {
    @ObservedObject public var state: NveSwiftUIBlurSurfaceState
    public init(state: NveSwiftUIBlurSurfaceState) { self.state = state }
    public var body: some View {
        ZStack {
            if #available(watchOS 10.0, *) {
                Rectangle().fill(state.props.material.swiftUIMaterial).opacity(Double(state.props.clampedIntensity))
            } else {
                Rectangle().fill(Color.secondary.opacity(0.35 * Double(state.props.clampedIntensity)))
            }
            if let tint = state.props.tint { nveColorFromRRGGBBAA(tint) }
        }
        .nveSpatialGlassSurface()
    }
}

public final class NveSwiftUIWebViewState: ObservableObject {
    @Published public private(set) var lastRequestedUrl = ""
    @Published public private(set) var allowsBackForward = true
    public private(set) var navigationCount = 0, reloadCount = 0, goBackCount = 0, goForwardCount = 0
    public private(set) var lastEvaluatedScript = ""; public private(set) var unmounted = false
    #if canImport(WebKit) && !os(tvOS) && !os(watchOS)
    public weak var attachedWebView: WKWebView?
    #endif
    public init() {}

    @discardableResult private func loadSafeUrl(_ url: String) -> Bool {
        guard isSafeNveWebViewURL(url), let u = URL(string: url) else { return false }
        lastRequestedUrl = url; navigationCount += 1
        #if canImport(WebKit) && !os(tvOS) && !os(watchOS)
        attachedWebView?.load(URLRequest(url: u))
        #else
        _ = u
        #endif
        return true
    }
    public func apply(_ p: NveWebViewProps) {
        allowsBackForward = p.allowsBackForward
        #if canImport(WebKit) && !os(tvOS) && !os(watchOS)
        attachedWebView?.allowsBackForwardNavigationGestures = allowsBackForward
        #endif
        if lastRequestedUrl != p.url { _ = loadSafeUrl(p.url) }
    }
    public func dispatch(_ cmd: NveWebViewCommand) -> Bool {
        switch cmd {
        case let .navigate(u): return loadSafeUrl(u)
        case .reload: reloadCount += 1; _ = { () in
            #if canImport(WebKit) && !os(tvOS) && !os(watchOS)
            attachedWebView?.reload()
            #endif
        }(); return true
        case .goBack: guard allowsBackForward else { return false }; goBackCount += 1; _ = { () in
            #if canImport(WebKit) && !os(tvOS) && !os(watchOS)
            if attachedWebView?.canGoBack == true { attachedWebView?.goBack() }
            #endif
        }(); return true
        case .goForward: guard allowsBackForward else { return false }; goForwardCount += 1; _ = { () in
            #if canImport(WebKit) && !os(tvOS) && !os(watchOS)
            if attachedWebView?.canGoForward == true { attachedWebView?.goForward() }
            #endif
        }(); return true
        case let .evaluateScript(js): lastEvaluatedScript = js; _ = { () in
            #if canImport(WebKit) && !os(tvOS) && !os(watchOS)
            attachedWebView?.evaluateJavaScript(js, completionHandler: nil)
            #endif
        }(); return true
        }
    }
    public func markUnmounted() {
        unmounted = true
        #if canImport(WebKit) && !os(tvOS) && !os(watchOS)
        attachedWebView?.stopLoading()
        #endif
    }
}

#if canImport(WebKit) && !os(tvOS) && !os(watchOS) && (canImport(AppKit) || canImport(UIKit))
private func makeConfiguredWKWebView(for state: NveSwiftUIWebViewState) -> WKWebView {
    let wv = WKWebView(frame: .zero, configuration: WKWebViewConfiguration())
    wv.allowsBackForwardNavigationGestures = state.allowsBackForward; state.attachedWebView = wv
    if isSafeNveWebViewURL(state.lastRequestedUrl), let u = URL(string: state.lastRequestedUrl) { wv.load(URLRequest(url: u)) }
    return wv
}
#if canImport(AppKit)
public struct NveSwiftUIWebViewRepresentable: NSViewRepresentable {
    @ObservedObject public var state: NveSwiftUIWebViewState
    public init(state: NveSwiftUIWebViewState) { self.state = state }
    public func makeNSView(context: Context) -> WKWebView { makeConfiguredWKWebView(for: state) }
    public func updateNSView(_ v: WKWebView, context: Context) { v.allowsBackForwardNavigationGestures = state.allowsBackForward }
}
#else
public struct NveSwiftUIWebViewRepresentable: UIViewRepresentable {
    @ObservedObject public var state: NveSwiftUIWebViewState
    public init(state: NveSwiftUIWebViewState) { self.state = state }
    public func makeUIView(context: Context) -> WKWebView { makeConfiguredWKWebView(for: state) }
    public func updateUIView(_ v: WKWebView, context: Context) { v.allowsBackForwardNavigationGestures = state.allowsBackForward }
}
#endif
#endif

public final class NveSwiftUIVideoPlayerState: ObservableObject {
    @Published public private(set) var props = NveVideoPlayerProps()
    @Published public private(set) var isPlaying = false
    @Published public private(set) var positionMs: UInt64 = 0
    @Published public private(set) var volume: Float = 1.0
    public private(set) var playCount = 0, pauseCount = 0, seekCount = 0; public private(set) var unmounted = false
    public var onCommand: ((NveVideoPlayerCommand) -> Void)?
    public init(onCommand: ((NveVideoPlayerCommand) -> Void)? = nil) { self.onCommand = onCommand }
    public func apply(_ n: NveVideoPlayerProps) {
        let uri = isSafeNveMediaURI(n.sourceUri) ? n.sourceUri : props.sourceUri
        props = NveVideoPlayerProps(sourceUri: uri, autoplay: n.autoplay, muted: n.muted, looping: n.looping, controls: n.controls)
        if props.autoplay && !props.sourceUri.isEmpty && !isPlaying { isPlaying = true; playCount += 1 }
    }
    public func dispatch(_ cmd: NveVideoPlayerCommand) -> Bool {
        switch cmd {
        case .play: isPlaying = true; playCount += 1
        case .pause: isPlaying = false; pauseCount += 1
        case let .seekMs(ms): positionMs = ms; seekCount += 1
        case let .setVolume(v): guard v.isFinite else { return false }; volume = min(max(v, 0.0), 1.0)
        }
        onCommand?(cmd); return true
    }
    public func markUnmounted() { unmounted = true; isPlaying = false }
}

public struct NveSwiftUIVideoPlayerView: View {
    @ObservedObject public var state: NveSwiftUIVideoPlayerState
    public init(state: NveSwiftUIVideoPlayerState) { self.state = state }
    public var body: some View { Color.black.accessibilityLabel(state.props.sourceUri) }
}

public extension NveNativeViewRegistry {
    func registerDefaultSwiftUINativeViews() {
        let blur = NveNativeViewAdapter(
            create: { _, _, _ in NveSwiftUIBlurSurfaceState() },
            updateProps: { i, _, d in if let s = i as? NveSwiftUIBlurSurfaceState, let p = try? NveBlurSurfaceProps.decode(from: d) { s.apply(p) } },
            unmount: { i, _ in (i as? NveSwiftUIBlurSurfaceState)?.markUnmounted() },
            makeView: { i in (i as? NveSwiftUIBlurSurfaceState).map { AnyView(NveSwiftUIBlurSurfaceView(state: $0)) } ?? AnyView(EmptyView()) }
        )
        let web = NveNativeViewAdapter(
            create: { _, _, _ in NveSwiftUIWebViewState() },
            updateProps: { i, _, d in if let s = i as? NveSwiftUIWebViewState, let p = try? NveWebViewProps.decode(from: d) { s.apply(p) } },
            dispatchCommand: { i, _, _, d in (i as? NveSwiftUIWebViewState).flatMap { s in (try? NveWebViewCommand.decode(from: d)).map(s.dispatch) } ?? false },
            unmount: { i, _ in (i as? NveSwiftUIWebViewState)?.markUnmounted() },
            makeView: { i in
                #if canImport(WebKit) && !os(tvOS) && !os(watchOS) && (canImport(AppKit) || canImport(UIKit))
                return (i as? NveSwiftUIWebViewState).map { AnyView(NveSwiftUIWebViewRepresentable(state: $0)) } ?? AnyView(EmptyView())
                #else
                return (i as? NveSwiftUIWebViewState).map { AnyView(NveWebViewUnavailablePlaceholder(state: $0)) } ?? AnyView(EmptyView())
                #endif
            }
        )
        let video = NveNativeViewAdapter(
            create: { _, _, _ in NveSwiftUIVideoPlayerState() },
            updateProps: { i, _, d in if let s = i as? NveSwiftUIVideoPlayerState, let p = try? NveVideoPlayerProps.decode(from: d) { s.apply(p) } },
            dispatchCommand: { i, _, _, d in (i as? NveSwiftUIVideoPlayerState).flatMap { s in (try? NveVideoPlayerCommand.decode(from: d)).map(s.dispatch) } ?? false },
            unmount: { i, _ in (i as? NveSwiftUIVideoPlayerState)?.markUnmounted() },
            makeView: { i in (i as? NveSwiftUIVideoPlayerState).map { AnyView(NveSwiftUIVideoPlayerView(state: $0)) } ?? AnyView(EmptyView()) }
        )
        for (k, a) in [(NveNativeViewConstants.kindBlurSurface, blur), (NveNativeViewConstants.kindBlurAlias, blur),
                       (NveNativeViewConstants.kindWebView, web), (NveNativeViewConstants.kindWebViewAlias, web),
                       (NveNativeViewConstants.kindVideoPlayer, video), (NveNativeViewConstants.kindVideoAlias, video)] {
            registerAdapter(k, adapter: a)
        }
    }
}
