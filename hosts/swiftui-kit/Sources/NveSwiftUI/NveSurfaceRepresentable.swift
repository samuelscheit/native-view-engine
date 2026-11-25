import CNveAbi
import Darwin
import Foundation
import SwiftUI

#if canImport(UIKit)
import UIKit
public typealias NvePlatformView = UIView
#elseif canImport(AppKit)
import AppKit
public typealias NvePlatformView = NSView
#endif

// MARK: - Core Types

/// Rendering backend used for an embedded NVE surface inside SwiftUI.
public enum NveRenderBackend: String, Sendable, Hashable {
    case fabric
    case skia
}

/// Native event kind mirroring `NativeEventKind` in `crates/nve-abi/src/events.rs`.
public enum NveEventKind: UInt8, Sendable, Hashable {
    case press = 1
    case longPress = 2
    case scroll = 3
    case textChange = 4
    case focus = 5
    case blur = 6
    case layout = 7
    case custom = 8
}

/// Gesture arbitration outcome mirroring `GestureDisposition` in `crates/nve-interop`.
public enum NveGestureDisposition: UInt8, Sendable, Hashable {
    case claim = 1
    case passThrough = 2
    case deferToParent = 3
}

/// Decoded outbound event drained from `NveComponentApi.drain_emitted_events`.
public struct NveEvent: Sendable, Equatable {
    public let surfaceId: UInt32
    public let nodeId: UInt32
    public let timestampUs: UInt64
    public let kind: NveEventKind
    public let name: String?
    public let payload: Data

    public init(
        surfaceId: UInt32,
        nodeId: UInt32,
        timestampUs: UInt64,
        kind: NveEventKind,
        name: String? = nil,
        payload: Data = Data()
    ) {
        self.surfaceId = surfaceId
        self.nodeId = nodeId
        self.timestampUs = timestampUs
        self.kind = kind
        self.name = name
        self.payload = payload
    }

    public func decodePayloadJSON() -> [String: Any]? {
        guard !payload.isEmpty else { return nil }
        return (try? JSONSerialization.jsonObject(with: payload)) as? [String: Any]
    }

    public func decodePayload<T: Decodable>(_ type: T.Type) throws -> T {
        try JSONDecoder().decode(type, from: payload)
    }
}

// MARK: - Binary EventRing Codec (matches `crates/nve-abi/src/events.rs`)

public enum NveEventRingCodec {
    public static func decode(_ data: Data) -> [NveEvent] {
        guard data.count >= 4 else { return [] }
        return data.withUnsafeBytes { raw in
            var pos = 0
            func readU16() -> UInt16? {
                guard pos + 2 <= raw.count else { return nil }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt16.self)
                pos += 2
                return UInt16(littleEndian: value)
            }
            func readU32() -> UInt32? {
                guard pos + 4 <= raw.count else { return nil }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt32.self)
                pos += 4
                return UInt32(littleEndian: value)
            }
            func readU64() -> UInt64? {
                guard pos + 8 <= raw.count else { return nil }
                let value = raw.loadUnaligned(fromByteOffset: pos, as: UInt64.self)
                pos += 8
                return UInt64(littleEndian: value)
            }
            func readU8() -> UInt8? {
                guard pos + 1 <= raw.count else { return nil }
                let value = raw[pos]
                pos += 1
                return value
            }
            func readF32() -> Float? {
                guard let bits = readU32() else { return nil }
                return Float(bitPattern: bits)
            }

            guard let count = readU32() else { return [] }
            var events: [NveEvent] = []
            events.reserveCapacity(Int(min(count, 4096)))

            for _ in 0..<count {
                guard
                    let surfaceId = readU32(),
                    let nodeId = readU32(),
                    let timestampUs = readU64(),
                    let kindByte = readU8(),
                    let payloadTag = readU8()
                else {
                    break
                }

                let kind = NveEventKind(rawValue: kindByte) ?? .custom
                var name: String?
                var payload = Data()

                var truncated = false
                switch payloadTag {
                case 0: // Empty
                    break
                case 1: // Point
                    guard let x = readF32(), let y = readF32() else {
                        truncated = true
                        break
                    }
                    payload = (try? JSONSerialization.data(withJSONObject: ["x": x, "y": y])) ?? Data()
                case 2: // Scroll
                    guard let x = readF32(), let y = readF32(), let cw = readF32(), let ch = readF32() else {
                        truncated = true
                        break
                    }
                    payload = (try? JSONSerialization.data(withJSONObject: [
                        "x": x, "y": y, "contentWidth": cw, "contentHeight": ch,
                    ])) ?? Data()
                case 3: // Text
                    guard let len = readU32(), Int(len) <= raw.count - pos else {
                        truncated = true
                        break
                    }
                    let slice = Data(raw[pos..<(pos + Int(len))])
                    pos += Int(len)
                    let text = String(data: slice, encoding: .utf8) ?? ""
                    payload = (try? JSONSerialization.data(withJSONObject: ["text": text])) ?? slice
                case 4: // Layout
                    guard let x = readF32(), let y = readF32(), let w = readF32(), let h = readF32() else {
                        truncated = true
                        break
                    }
                    payload = (try? JSONSerialization.data(withJSONObject: [
                        "x": x, "y": y, "width": w, "height": h,
                    ])) ?? Data()
                case 5: // Custom
                    guard let nameLen = readU16(), Int(nameLen) <= raw.count - pos else {
                        truncated = true
                        break
                    }
                    let nameData = Data(raw[pos..<(pos + Int(nameLen))])
                    pos += Int(nameLen)
                    name = String(data: nameData, encoding: .utf8)
                    guard let dataLen = readU32(), Int(dataLen) <= raw.count - pos else {
                        truncated = true
                        break
                    }
                    payload = Data(raw[pos..<(pos + Int(dataLen))])
                    pos += Int(dataLen)
                default:
                    truncated = true
                }

                if truncated {
                    break
                }

                events.append(
                    NveEvent(
                        surfaceId: surfaceId,
                        nodeId: nodeId,
                        timestampUs: timestampUs,
                        kind: kind,
                        name: name,
                        payload: payload
                    )
                )
            }
            return events
        }
    }

    private static func appendLE<T: FixedWidthInteger>(_ value: T, to data: inout Data) {
        withUnsafeBytes(of: value.littleEndian) { raw in
            data.append(contentsOf: raw)
        }
    }

    public static func encodeCustomEvent(
        surfaceId: UInt32,
        nodeId: UInt32 = 1,
        name: String,
        payload: Data,
        timestampUs: UInt64 = UInt64(Date().timeIntervalSince1970 * 1_000_000)
    ) -> Data {
        var out = Data()
        appendLE(UInt32(1), to: &out)
        appendLE(surfaceId, to: &out)
        appendLE(nodeId, to: &out)
        appendLE(timestampUs, to: &out)
        out.append(NveEventKind.custom.rawValue)
        out.append(5) // EventPayload::Custom

        let nameData = Data(name.utf8)
        appendLE(UInt16(nameData.count), to: &out)
        out.append(nameData)

        appendLE(UInt32(payload.count), to: &out)
        out.append(payload)
        return out
    }
}

// MARK: - Shared Channel Bus Client

/// Swift client bridging `NveChannelBus` topics (`theme`, `locale`, `auth`, `navigation`, custom).
public final class NveChannelClient: @unchecked Sendable {
    public struct Message: Sendable {
        public let topic: String
        public let payload: Data
        public let seq: UInt64
    }

    private let lock = NSLock()
    private var seq: UInt64 = 0
    private var retained: [String: Message] = [:]
    private var handlers: [String: [UUID: (Message) -> Void]] = [:]

    public init() {}

    @discardableResult
    public func subscribe(
        topic: String,
        replayRetained: Bool = true,
        handler: @escaping (Message) -> Void
    ) -> UUID {
        let token = UUID()
        let snapshot: Message?
        lock.lock()
        handlers[topic, default: [:]][token] = handler
        snapshot = replayRetained ? retained[topic] : nil
        lock.unlock()
        if let snapshot {
            handler(snapshot)
        }
        return token
    }

    public func unsubscribe(topic: String, token: UUID) {
        lock.lock()
        handlers[topic]?.removeValue(forKey: token)
        lock.unlock()
    }

    public func publish(topic: String, payload: Data, retain: Bool = false) {
        let message: Message
        let callbacks: [(Message) -> Void]
        lock.lock()
        seq += 1
        message = Message(topic: topic, payload: payload, seq: seq)
        if retain {
            retained[topic] = message
        }
        callbacks = Array(handlers[topic]?.values ?? [:].values)
        lock.unlock()
        for callback in callbacks {
            callback(message)
        }
    }

    public func publishJSON(topic: String, object: [String: Any], retain: Bool = false) {
        guard let data = try? JSONSerialization.data(withJSONObject: object) else { return }
        publish(topic: topic, payload: data, retain: retain)
    }
}

// MARK: - Surface Controller (`nve_get_interface` Lifecycle + Layout + Events)

/// Manages an instantiated `nve_get_interface` surface, typed props propagation, imperative commands,
/// intrinsic layout negotiation, and outbound event draining.
public final class NveSurfaceController: ObservableObject {
    private static var nextSurfaceId: UInt32 = 1
    private static let idLock = NSLock()

    public let surfaceId: UInt32
    public let vtableSymbol: String
    public let backend: NveRenderBackend

    private var module: NveModule = NveModule(surface: nil, layout: nil, component: nil, dev: nil)
    private var handle: NveAppHandle?
    private var dylibHandle: UnsafeMutableRawPointer?
    private var frameCounter: UInt64 = 0
    private var lastPropsData: Data?
    private var fallbackIntrinsicSize: CGSize = CGSize(width: 240, height: 120)

    public var onEvent: ((NveEvent) -> Void)?

    public init(
        surfaceId: UInt32? = nil,
        vtableSymbol: String = "nve_get_interface",
        backend: NveRenderBackend = .fabric,
        getInterface: NveGetInterfaceFn? = nil,
        dylibPath: String? = nil
    ) {
        if let surfaceId {
            self.surfaceId = surfaceId
        } else {
            Self.idLock.lock()
            let id = Self.nextSurfaceId
            Self.nextSurfaceId &+= 1
            Self.idLock.unlock()
            self.surfaceId = id
        }
        self.vtableSymbol = vtableSymbol
        self.backend = backend

        if let getInterface {
            attach(getInterface: getInterface)
        } else {
            resolveAndAttach(symbol: vtableSymbol, dylibPath: dylibPath)
        }
    }

    deinit {
        destroy()
    }

    public var isMounted: Bool {
        handle != nil && module.surface != nil
    }

    private func resolveAndAttach(symbol: String, dylibPath: String?) {
        let lib: UnsafeMutableRawPointer?
        if let dylibPath, !dylibPath.isEmpty {
            lib = dlopen(dylibPath, RTLD_NOW | RTLD_LOCAL)
            dylibHandle = lib
        } else {
            lib = dlopen(nil, RTLD_NOW)
        }
        guard let lib, let sym = dlsym(lib, symbol) else {
            return
        }
        let getIface = unsafeBitCast(sym, to: NveGetInterfaceFn.self)
        attach(getInterface: getIface)
    }

    public func attach(getInterface: NveGetInterfaceFn) {
        destroy()
        let resolved = nve_resolve_module(getInterface)
        guard let surface = resolved.surface, let createFn = surface.pointee.create else { return }
        guard let created = createFn(surfaceId) else { return }
        self.module = resolved
        self.handle = created
    }

    /// Pushes JSON-serialized props into `NveComponentApi.set_props` if changed, rendering on dirty.
    @discardableResult
    public func setProps(_ props: [String: Any]) -> Bool {
        guard JSONSerialization.isValidJSONObject(props),
              let data = try? JSONSerialization.data(withJSONObject: props, options: [.sortedKeys])
        else {
            return false
        }
        return setPropsJSON(data)
    }

    @discardableResult
    public func setPropsJSON(_ data: Data) -> Bool {
        if lastPropsData == data {
            return false
        }
        lastPropsData = data
        guard let handle, let setPropsFn = module.component?.pointee.set_props else {
            return false
        }
        let needsRender = data.withUnsafeBytes { raw -> UInt32 in
            let slice = NveBufferSlice(
                ptr: raw.bindMemory(to: UInt8.self).baseAddress,
                len: raw.count
            )
            return setPropsFn(handle, slice)
        }
        if needsRender != 0 {
            renderFrame()
        }
        drainAndForwardEvents()
        return needsRender != 0
    }

    /// Dispatches an imperative command to the mounted NVE component.
    public func dispatchCommand(name: String, args: [String: Any] = [:]) {
        let payload = (try? JSONSerialization.data(withJSONObject: args)) ?? Data()
        let encoded = NveEventRingCodec.encodeCustomEvent(
            surfaceId: surfaceId,
            name: name,
            payload: payload
        )
        guard let handle, let dispatchFn = module.surface?.pointee.dispatch_events else { return }
        let needsRender = encoded.withUnsafeBytes { raw -> UInt32 in
            let slice = NveBufferSlice(
                ptr: raw.bindMemory(to: UInt8.self).baseAddress,
                len: raw.count
            )
            return dispatchFn(handle, slice)
        }
        if needsRender != 0 {
            renderFrame()
        }
        drainAndForwardEvents()
    }

    /// Triggers a single frame render and returns the raw CommandBuffer bytes.
    @discardableResult
    public func renderFrame() -> Data {
        guard let handle, let renderFn = module.surface?.pointee.render else { return Data() }
        frameCounter += 1
        let slice = renderFn(handle, frameCounter)
        guard let ptr = slice.ptr, slice.len > 0 else { return Data() }
        return Data(bytes: ptr, count: slice.len)
    }

    /// Applies a Tier-1 JSON hot-reload patch via `NveDevApi.apply_hot_patch`.
    @discardableResult
    public func applyHotPatch(json: String) -> Bool {
        guard let handle, let patchFn = module.dev?.pointee.apply_hot_patch else { return false }
        let data = Data(json.utf8)
        let dirty = data.withUnsafeBytes { raw -> UInt32 in
            let slice = NveBufferSlice(
                ptr: raw.bindMemory(to: UInt8.self).baseAddress,
                len: raw.count
            )
            return patchFn(handle, slice)
        }
        if dirty != 0 {
            renderFrame()
        }
        return dirty != 0
    }

    /// Drains all queued outbound events via `NveComponentApi.drain_emitted_events`.
    public func drainEmittedEvents() -> [NveEvent] {
        guard let handle, let drainFn = module.component?.pointee.drain_emitted_events else { return [] }
        let slice = drainFn(handle)
        guard let ptr = slice.ptr, slice.len > 0 else { return [] }
        return NveEventRingCodec.decode(Data(bytes: ptr, count: slice.len))
    }

    public func drainAndForwardEvents() {
        guard let onEvent else { return }
        for event in drainEmittedEvents() {
            onEvent(event)
        }
    }

    /// Negotiates intrinsic size for SwiftUI's `sizeThatFits` using zero-cost `NveLayoutApi.measure`.
    public func measure(
        minWidth: CGFloat = 0,
        minHeight: CGFloat = 0,
        maxWidth: CGFloat = .infinity,
        maxHeight: CGFloat = .infinity
    ) -> CGSize {
        if let handle, let measureFn = module.layout?.pointee.measure {
            let constraints = NveSizeConstraints(
                min_w: Float(minWidth),
                min_h: Float(minHeight),
                max_w: maxWidth.isFinite ? Float(maxWidth) : .infinity,
                max_h: maxHeight.isFinite ? Float(maxHeight) : .infinity
            )
            let measured = measureFn(handle, constraints)
            let clampedW = max(minWidth, min(maxWidth, CGFloat(measured.width)))
            let clampedH = max(minHeight, min(maxHeight, CGFloat(measured.height)))
            return CGSize(width: clampedW, height: clampedH)
        }
        let width = maxWidth.isFinite ? max(minWidth, maxWidth) : max(minWidth, fallbackIntrinsicSize.width)
        let height = maxHeight.isFinite ? max(minHeight, maxHeight) : max(minHeight, fallbackIntrinsicSize.height)
        return CGSize(width: width, height: height)
    }

    public func setFallbackIntrinsicSize(_ size: CGSize) {
        fallbackIntrinsicSize = size
    }

    public func destroy() {
        if let handle, let destroyFn = module.surface?.pointee.destroy {
            destroyFn(handle)
        }
        handle = nil
        module = NveModule(surface: nil, layout: nil, component: nil, dev: nil)
        if let dylibHandle {
            dlclose(dylibHandle)
            self.dylibHandle = nil
        }
    }
}

// MARK: - Native Host Container View

public final class NveHostContainerView: NvePlatformView {
    public var controller: NveSurfaceController?

    public override init(frame: CGRect) {
        super.init(frame: frame)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }
}

// MARK: - SwiftUI Representable (`UIViewRepresentable` / `NSViewRepresentable`)

/// Cross-platform SwiftUI `View` hosting an NVE Rust component backed by `nve_get_interface`.
///
/// Supports:
/// - `sizeThatFits` intrinsic layout negotiation on iOS 16+ and macOS 13+
/// - Reactive JSON prop updates via `set_props`
/// - Imperative commands via `NveSurfaceController`
/// - Outbound event callbacks via `onEvent`
public struct NveSurfaceRepresentable {
    public let componentName: String
    public let vtableSymbol: String
    public let backend: NveRenderBackend
    public let props: [String: Any]
    public let controller: NveSurfaceController?
    public let onEvent: ((NveEvent) -> Void)?

    public init(
        componentName: String,
        vtableSymbol: String = "nve_get_interface",
        backend: NveRenderBackend = .fabric,
        props: [String: Any] = [:],
        controller: NveSurfaceController? = nil,
        onEvent: ((NveEvent) -> Void)? = nil
    ) {
        self.componentName = componentName
        self.vtableSymbol = vtableSymbol
        self.backend = backend
        self.props = props
        self.controller = controller
        self.onEvent = onEvent
    }

    public final class Coordinator {
        let controller: NveSurfaceController
        let ownsController: Bool

        init(
            externalController: NveSurfaceController?,
            vtableSymbol: String,
            backend: NveRenderBackend
        ) {
            if let externalController {
                self.controller = externalController
                self.ownsController = false
            } else {
                self.controller = NveSurfaceController(
                    vtableSymbol: vtableSymbol,
                    backend: backend
                )
                self.ownsController = true
            }
        }

        deinit {
            if ownsController {
                controller.destroy()
            }
        }
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(
            externalController: controller,
            vtableSymbol: vtableSymbol,
            backend: backend
        )
    }

    private func sync(view: NveHostContainerView, coordinator: Coordinator) {
        let active = coordinator.controller
        active.onEvent = onEvent
        view.controller = active
        _ = active.setProps(props)
    }

    private func computeSizeThatFits(
        _ proposal: ProposedViewSize,
        coordinator: Coordinator
    ) -> CGSize {
        let maxW = proposal.width ?? .infinity
        let maxH = proposal.height ?? .infinity
        return coordinator.controller.measure(
            minWidth: 0,
            minHeight: 0,
            maxWidth: maxW,
            maxHeight: maxH
        )
    }
}

#if canImport(UIKit)
extension NveSurfaceRepresentable: UIViewRepresentable {
    public func makeUIView(context: Context) -> NveHostContainerView {
        let view = NveHostContainerView(frame: .zero)
        sync(view: view, coordinator: context.coordinator)
        return view
    }

    public func updateUIView(_ uiView: NveHostContainerView, context: Context) {
        sync(view: uiView, coordinator: context.coordinator)
    }

    public func sizeThatFits(
        _ proposal: ProposedViewSize,
        uiView: NveHostContainerView,
        context: Context
    ) -> CGSize? {
        computeSizeThatFits(proposal, coordinator: context.coordinator)
    }

    public static func dismantleUIView(_ uiView: NveHostContainerView, coordinator: Coordinator) {
        uiView.controller = nil
        if coordinator.ownsController {
            coordinator.controller.destroy()
        }
    }
}
#elseif canImport(AppKit)
extension NveSurfaceRepresentable: NSViewRepresentable {
    public func makeNSView(context: Context) -> NveHostContainerView {
        let view = NveHostContainerView(frame: .zero)
        sync(view: view, coordinator: context.coordinator)
        return view
    }

    public func updateNSView(_ nsView: NveHostContainerView, context: Context) {
        sync(view: nsView, coordinator: context.coordinator)
    }

    public func sizeThatFits(
        _ proposal: ProposedViewSize,
        nsView: NveHostContainerView,
        context: Context
    ) -> CGSize? {
        computeSizeThatFits(proposal, coordinator: context.coordinator)
    }

    public static func dismantleNSView(_ nsView: NveHostContainerView, coordinator: Coordinator) {
        nsView.controller = nil
        if coordinator.ownsController {
            coordinator.controller.destroy()
        }
    }
}
#endif
