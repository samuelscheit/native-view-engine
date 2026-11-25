import Foundation
import SwiftUI

#if os(watchOS)
import WatchKit
#endif

// MARK: - Multi-Apple Platform Profile (`macOS` / `iOS` / `tvOS` / `watchOS` / `visionOS`)

/// Apple platform family supported by `NveSwiftUI`.
public enum NveApplePlatform: String, CaseIterable, Equatable, Hashable, Sendable {
    case macOS, iOS, tvOS, watchOS, visionOS

    public static var current: NveApplePlatform {
        #if os(macOS)
        return .macOS
        #elseif os(tvOS)
        return .tvOS
        #elseif os(watchOS)
        return .watchOS
        #elseif os(visionOS)
        return .visionOS
        #else
        return .iOS
        #endif
    }
}

/// Deterministic device & input capability profile for all 5 Apple platforms.
public struct NveAppleDeviceProfile: Equatable, Sendable {
    public let platform: NveApplePlatform
    public let isTenFootUI: Bool
    public let isCompactWatch: Bool
    public let isSpatial: Bool
    public let supportsWebView: Bool
    public let supportsPointer: Bool
    public let supportsTouch: Bool
    public let supportsDigitalCrown: Bool
    public let supportsSiriRemote: Bool
    public let defaultOverscanRatio: CGFloat
    public let recommendedMinimumHitTargetPt: CGFloat

    public static var current: NveAppleDeviceProfile { forPlatform(.current) }

    public static func forPlatform(_ platform: NveApplePlatform) -> NveAppleDeviceProfile {
        switch platform {
        case .macOS:
            return Self(platform: .macOS, isTenFootUI: false, isCompactWatch: false, isSpatial: false,
                        supportsWebView: true, supportsPointer: true, supportsTouch: false,
                        supportsDigitalCrown: false, supportsSiriRemote: false,
                        defaultOverscanRatio: 0.0, recommendedMinimumHitTargetPt: 28.0)
        case .iOS:
            return Self(platform: .iOS, isTenFootUI: false, isCompactWatch: false, isSpatial: false,
                        supportsWebView: true, supportsPointer: false, supportsTouch: true,
                        supportsDigitalCrown: false, supportsSiriRemote: false,
                        defaultOverscanRatio: 0.0, recommendedMinimumHitTargetPt: 44.0)
        case .tvOS:
            return Self(platform: .tvOS, isTenFootUI: true, isCompactWatch: false, isSpatial: false,
                        supportsWebView: false, supportsPointer: false, supportsTouch: false,
                        supportsDigitalCrown: false, supportsSiriRemote: true,
                        defaultOverscanRatio: NveTvOverscanInsets.defaultRatio, recommendedMinimumHitTargetPt: 66.0)
        case .watchOS:
            return Self(platform: .watchOS, isTenFootUI: false, isCompactWatch: true, isSpatial: false,
                        supportsWebView: false, supportsPointer: false, supportsTouch: true,
                        supportsDigitalCrown: true, supportsSiriRemote: false,
                        defaultOverscanRatio: 0.0, recommendedMinimumHitTargetPt: 38.0)
        case .visionOS:
            return Self(platform: .visionOS, isTenFootUI: false, isCompactWatch: false, isSpatial: true,
                        supportsWebView: true, supportsPointer: true, supportsTouch: true,
                        supportsDigitalCrown: false, supportsSiriRemote: false,
                        defaultOverscanRatio: 0.0, recommendedMinimumHitTargetPt: 60.0)
        }
    }
}

// MARK: - Zero-JSON `NK1` Binary KeyDown Codec & tvOS Siri Remote Mapping

/// Decoded zero-JSON `NK1` binary keydown payload matching `crates/nve-abi/src/events.rs`.
public struct NveNk1KeyDown: Equatable, Sendable {
    public let key: String
    public let meta, ctrl, shift, alt: Bool

    public init(key: String, meta: Bool = false, ctrl: Bool = false, shift: Bool = false, alt: Bool = false) {
        self.key = key; self.meta = meta; self.ctrl = ctrl; self.shift = shift; self.alt = alt
    }
}

/// Zero-JSON `NK1` wire codec (`[0x4E, 0x4B, 0x31, flags_u8, utf8_key...]`).
public enum NveNk1BinaryCodec {
    public static let magic: [UInt8] = [0x4E, 0x4B, 0x31]
    public static let flagMeta: UInt8 = 1 << 0, flagCtrl: UInt8 = 1 << 1
    public static let flagShift: UInt8 = 1 << 2, flagAlt: UInt8 = 1 << 3
    public static let keydownEventName = "keydown"

    public static func encode(
        key: String, meta: Bool = false, ctrl: Bool = false, shift: Bool = false, alt: Bool = false
    ) -> Data? {
        let keyBytes = Array(key.utf8)
        guard !keyBytes.isEmpty else { return nil }
        var flags: UInt8 = 0
        if meta { flags |= flagMeta }
        if ctrl { flags |= flagCtrl }
        if shift { flags |= flagShift }
        if alt { flags |= flagAlt }
        var out = Data(capacity: 4 + keyBytes.count)
        out.append(contentsOf: magic)
        out.append(flags)
        out.append(contentsOf: keyBytes)
        return out
    }

    public static func decode(_ data: Data) -> NveNk1KeyDown? {
        guard data.count >= 5 else { return nil }
        let b = Array(data)
        guard b[0] == magic[0], b[1] == magic[1], b[2] == magic[2] else { return nil }
        guard let key = String(bytes: b[4...], encoding: .utf8), !key.isEmpty else { return nil }
        let f = b[3]
        return NveNk1KeyDown(
            key: key, meta: (f & flagMeta) != 0, ctrl: (f & flagCtrl) != 0,
            shift: (f & flagShift) != 0, alt: (f & flagAlt) != 0
        )
    }
}

/// Siri Remote / Apple TV command mapped directly to W3C & NVE `NK1` keydown events.
public enum NveTvRemoteCommand: Equatable, Hashable, Sendable {
    case up, down, left, right, select, playPause, exit

    public var nk1KeyName: String {
        switch self {
        case .up: return "ArrowUp"
        case .down: return "ArrowDown"
        case .left: return "ArrowLeft"
        case .right: return "ArrowRight"
        case .select: return "Select"
        case .playPause: return "MediaPlayPause"
        case .exit: return "Back"
        }
    }

    public func makeKeyDownEvent(
        surfaceId: UInt32,
        nodeId: UInt32 = 1,
        timestampUs: UInt64 = UInt64(Date().timeIntervalSince1970 * 1_000_000)
    ) -> NveEvent {
        NveEvent(
            surfaceId: surfaceId, nodeId: nodeId, timestampUs: timestampUs,
            kind: .custom, name: NveNk1BinaryCodec.keydownEventName,
            payload: .binValue(NveNk1BinaryCodec.encode(key: nk1KeyName) ?? Data())
        )
    }
}

/// 5% TV title-safe overscan inset calculator matching `nve-tv` and Android TV Leanback.
public struct NveTvOverscanInsets: Equatable, Sendable {
    public static let defaultRatio: CGFloat = 0.05
    public static let maxRatio: CGFloat = 0.45
    public let top, leading, bottom, trailing: CGFloat

    public init(top: CGFloat, leading: CGFloat, bottom: CGFloat, trailing: CGFloat) {
        self.top = top; self.leading = leading; self.bottom = bottom; self.trailing = trailing
    }

    public static func forViewport(size: CGSize, ratio: CGFloat = defaultRatio) -> NveTvOverscanInsets {
        let safeRatio = ratio.isFinite ? min(max(ratio, 0), maxRatio) : defaultRatio
        let safeW = (size.width.isFinite && size.width > 0) ? size.width : 0
        let safeH = (size.height.isFinite && size.height > 0) ? size.height : 0
        return NveTvOverscanInsets(
            top: safeH * safeRatio, leading: safeW * safeRatio,
            bottom: safeH * safeRatio, trailing: safeW * safeRatio
        )
    }

    public var edgeInsets: EdgeInsets { EdgeInsets(top: top, leading: leading, bottom: bottom, trailing: trailing) }
}

// MARK: - watchOS Digital Crown & Screen Scale Helpers

/// Converts watchOS Digital Crown rotation deltas into vertical scroll points.
public enum NveWatchCrownScrollMapper {
    public static let defaultPointsPerDetent: CGFloat = 24.0

    public static func scrollDeltaPoints(
        from previousValue: Double,
        to newValue: Double,
        pointsPerDetent: CGFloat = defaultPointsPerDetent
    ) -> CGFloat {
        guard previousValue.isFinite, newValue.isFinite, pointsPerDetent.isFinite, pointsPerDetent > 0 else {
            return 0
        }
        return CGFloat(newValue - previousValue) * pointsPerDetent
    }

    /// Watch screen scale fallback avoiding unavailable `UIScreen.main` on watchOS / visionOS.
    public static var watchDeviceScreenScale: CGFloat {
        #if os(watchOS)
        return WKInterfaceDevice.current().screenScale
        #else
        return 2.0
        #endif
    }
}

// MARK: - Graceful WebView Fallback (tvOS / watchOS)

/// Graceful placeholder shown when `nve.web_view` mounts on tvOS or watchOS (WebKit unavailable).
public struct NveWebViewUnavailablePlaceholder: View {
    @ObservedObject public var state: NveSwiftUIWebViewState

    public init(state: NveSwiftUIWebViewState) { self.state = state }

    public var body: some View {
        VStack(spacing: 4) {
            Text("WebView unavailable on \(NveApplePlatform.current.rawValue)")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            if !state.lastRequestedUrl.isEmpty {
                Text(state.lastRequestedUrl).font(.caption2).foregroundStyle(.tertiary).lineLimit(1)
            }
        }
        .padding(8)
        .accessibilityLabel(Text(state.lastRequestedUrl.isEmpty ? "WebView unavailable" : state.lastRequestedUrl))
    }
}

// MARK: - Cross-Apple SwiftUI Modifiers

#if os(watchOS)
private struct NveWatchCrownScrollModifier: ViewModifier {
    let detentPoints: CGFloat
    let onScrollDelta: (CGFloat) -> Void
    @State private var crownValue: Double = 0

    func body(content: Content) -> some View {
        let crownBinding = Binding<Double>(
            get: { crownValue },
            set: { next in
                let delta = NveWatchCrownScrollMapper.scrollDeltaPoints(
                    from: crownValue, to: next, pointsPerDetent: detentPoints
                )
                crownValue = next
                if delta != 0 { onScrollDelta(delta) }
            }
        )
        return content.focusable(true).digitalCrownRotation(crownBinding)
    }
}
#endif

public extension View {
    /// Attaches tvOS Siri Remote directional, play/pause, exit commands, and optional 5% overscan.
    @ViewBuilder
    func nveTvRemoteControls(
        surfaceId: UInt32,
        nodeId: UInt32 = 1,
        applyTitleSafeOverscan: Bool = false,
        overscanRatio: CGFloat = NveTvOverscanInsets.defaultRatio,
        onEvent: @escaping (NveEvent) -> Void
    ) -> some View {
        #if os(tvOS)
        let wired = self
            .focusable(true)
            .onMoveCommand { direction in
                let cmd: NveTvRemoteCommand
                switch direction {
                case .up: cmd = .up
                case .down: cmd = .down
                case .left: cmd = .left
                case .right: cmd = .right
                @unknown default: return
                }
                onEvent(cmd.makeKeyDownEvent(surfaceId: surfaceId, nodeId: nodeId))
            }
            .onPlayPauseCommand {
                onEvent(NveTvRemoteCommand.playPause.makeKeyDownEvent(surfaceId: surfaceId, nodeId: nodeId))
            }
            .onExitCommand {
                onEvent(NveTvRemoteCommand.exit.makeKeyDownEvent(surfaceId: surfaceId, nodeId: nodeId))
            }
        if applyTitleSafeOverscan {
            GeometryReader { proxy in
                wired.padding(NveTvOverscanInsets.forViewport(size: proxy.size, ratio: overscanRatio).edgeInsets)
            }
        } else {
            wired
        }
        #else
        self
        #endif
    }

    @ViewBuilder
    func nveWatchCrownScroll(
        detentPoints: CGFloat = NveWatchCrownScrollMapper.defaultPointsPerDetent,
        onScrollDelta: @escaping (CGFloat) -> Void
    ) -> some View {
        #if os(watchOS)
        modifier(NveWatchCrownScrollModifier(detentPoints: detentPoints, onScrollDelta: onScrollDelta))
        #else
        self
        #endif
    }

    @ViewBuilder
    func nveSpatialHoverEffect() -> some View {
        #if os(visionOS)
        self.hoverEffect(.automatic)
        #else
        self
        #endif
    }

    @ViewBuilder
    func nveSpatialGlassSurface() -> some View {
        #if os(visionOS)
        self.glassBackgroundEffect()
        #else
        self
        #endif
    }

    @ViewBuilder
    func nveAdaptiveTextFieldStyle() -> some View {
        #if os(iOS) || os(macOS) || os(visionOS)
        self.textFieldStyle(.roundedBorder)
        #else
        self
        #endif
    }

    @ViewBuilder
    func nveAdaptiveSmallControl() -> some View {
        #if !os(tvOS)
        self.controlSize(.small)
        #else
        self
        #endif
    }
}
