import CNveAbi
import Darwin
import Foundation
import SwiftUI

// MARK: - Surface Controller (`nve_get_interface` Lifecycle + Direct FFI + Layout + Events)

/// Manages an instantiated `nve_get_interface` surface, zero-JSON direct props propagation,
/// direct event/command dispatch, intrinsic layout negotiation, and outbound event draining.
public final class NveSurfaceController: ObservableObject {
    private static var nextSurfaceId: UInt32 = 1
    private static let idLock = NSLock()

    public let surfaceId: UInt32
    public let vtableSymbol: String
    public let backend: NveRenderBackend

    private var module: NveModule = NveModule(surface: nil, layout: nil, component: nil, dev: nil, host: nil)
    private var handle: NveAppHandle?
    private var dylibHandle: UnsafeMutableRawPointer?
    public private(set) var resolutionSource: NveInterfaceResolutionSource?
    private var frameCounter: UInt64 = 0
    private var lastPropsValue: NveBinValue?
    private var lastDirectProps: Any?
    private var lastDirectPropsEqual: ((Any) -> Bool)?
    private var fallbackIntrinsicSize: CGSize = CGSize(width: 240, height: 120)

    public var onEvent: ((NveEvent) -> Void)?

    public init(
        surfaceId: UInt32? = nil,
        vtableSymbol: String = "nve_get_interface",
        backend: NveRenderBackend = .fabric,
        getInterface: NveGetInterfaceFn? = nil,
        dylibPath: String? = nil,
        staticTable: NveStaticInterfaceTable = .shared
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

        if let resolved = NveInterfaceResolver.resolve(
            symbol: vtableSymbol,
            getInterface: getInterface,
            dylibPath: dylibPath,
            staticTable: staticTable
        ) {
            attach(resolved: resolved)
        }
    }

    deinit {
        destroy()
    }

    public var isMounted: Bool {
        handle != nil && module.surface != nil
    }

    public func attach(getInterface: NveGetInterfaceFn) {
        attach(
            resolved: NveResolvedInterface(
                module: nve_resolve_module(getInterface),
                dylibHandle: nil,
                source: .explicit
            )
        )
    }

    /// Attaches a pre-resolved static or dynamic `NveResolvedInterface`.
    public func attach(resolved: NveResolvedInterface) {
        destroy()
        guard let surface = resolved.module.surface, let createFn = surface.pointee.create else {
            if let lib = resolved.dylibHandle {
                dlclose(lib)
            }
            return
        }
        guard let created = createFn(surfaceId) else {
            if let lib = resolved.dylibHandle {
                dlclose(lib)
            }
            return
        }
        self.module = resolved.module
        self.handle = created
        self.dylibHandle = resolved.dylibHandle
        self.resolutionSource = resolved.source
        self.lastPropsValue = nil
        self.lastDirectProps = nil
        self.lastDirectPropsEqual = nil
    }

    /// Zero-copy typed fast path for `NveDirectEncodable & Equatable` prop structs:
    /// skips `[String: Any]` boxing, `NveBinValue` heap-tree construction, and key sorting.
    @discardableResult
    public func setDirectProps<P: NveDirectEncodable & Equatable>(_ props: P) -> Bool {
        if let eq = lastDirectPropsEqual, eq(props) {
            return false
        }
        guard let needsRender = applyDirectProps(props) else {
            return false
        }
        lastPropsValue = nil
        lastDirectProps = props
        lastDirectPropsEqual = { other in (other as? P) == props }
        return needsRender
    }

    /// Zero-copy typed fast path for `NveDirectEncodable` prop structs (without `Equatable` dedup).
    @discardableResult
    public func setDirectProps<P: NveDirectEncodable>(_ props: P) -> Bool {
        guard let needsRender = applyDirectProps(props) else {
            return false
        }
        lastPropsValue = nil
        lastDirectProps = nil
        lastDirectPropsEqual = nil
        return needsRender
    }

    private func applyDirectProps<P: NveDirectEncodable>(_ props: P) -> Bool? {
        guard let handle, let directFn = module.component?.pointee.set_props_direct else {
            return nil
        }
        guard let needsRender = try? NveDirectPropsWriter.withEncodedProps(props, { ptr, count in
            directFn(handle, ptr, count)
        }) else {
            return nil
        }

        if needsRender != 0 {
            renderFrame()
        }
        drainAndForwardEvents()
        return needsRender != 0
    }

    /// Pushes typed props via required `NveComponentApi.set_props_direct` (zero JSON, no legacy fallback).
    @discardableResult
    public func setProps(_ entries: [(String, NveBinValue)]) -> Bool {
        let canonical = NveBinValue.map(entries)
        if lastPropsValue == canonical {
            return false
        }
        guard let handle, let directFn = module.component?.pointee.set_props_direct else {
            return false
        }
        guard let needsRender = try? NveDirectPropsWriter.withEntries(entries, { ptr, count in
            directFn(handle, ptr, count)
        }) else {
            return false
        }

        lastPropsValue = canonical
        lastDirectProps = nil
        lastDirectPropsEqual = nil
        if needsRender != 0 {
            renderFrame()
        }
        drainAndForwardEvents()
        return needsRender != 0
    }

    /// Pushes `[String: Any]` props into the mounted component via zero-JSON direct C-ABI structs.
    @discardableResult
    public func setProps(_ props: [String: Any]) -> Bool {
        guard let entries = try? NveBinValue.canonicalEntries(from: props) else {
            return false
        }
        return setProps(entries)
    }

    /// Dispatches a batch of typed events via required `NveSurfaceApi.dispatch_events_direct`.
    @discardableResult
    public func dispatchEvents(_ events: [NveEvent]) -> Bool {
        guard !events.isEmpty, let handle, let directFn = module.surface?.pointee.dispatch_events_direct else {
            return false
        }
        guard let needsRender = try? NveDirectEventWriter.withBatch(events, { batch in
            directFn(handle, batch)
        }) else {
            return false
        }

        if needsRender != 0 {
            renderFrame()
        }
        drainAndForwardEvents()
        return needsRender != 0
    }

    /// Dispatches a single typed event.
    @discardableResult
    public func dispatchEvent(_ event: NveEvent) -> Bool {
        dispatchEvents([event])
    }

    /// Dispatches an imperative command with a typed `NveBinValue` argument directly over the C ABI.
    public func dispatchCommand(
        name: String,
        value: NveBinValue,
        nodeId: UInt32 = 1,
        timestampUs: UInt64 = UInt64(Date().timeIntervalSince1970 * 1_000_000)
    ) {
        guard let handle, let directFn = module.surface?.pointee.dispatch_events_direct else { return }
        guard let needsRender = try? NveDirectEventWriter.withCustomEvent(
            surfaceId: surfaceId,
            nodeId: nodeId,
            timestampUs: timestampUs,
            name: name,
            value: value,
            { batch in directFn(handle, batch) }
        ) else {
            return
        }

        if needsRender != 0 {
            renderFrame()
        }
        drainAndForwardEvents()
    }

    /// Dispatches an imperative command with `[String: Any]` arguments (converted directly to
    /// `NveValue` / `NveBinValue`, zero JSON).
    public func dispatchCommand(name: String, args: [String: Any] = [:]) {
        guard let entries = try? NveBinValue.canonicalEntries(from: args) else { return }
        dispatchCommand(name: name, value: .map(entries))
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

    /// Applies one binary `NVP1` Tier-1 hot-patch frame (as broadcast by `nve dev`) via
    /// `NveDevApi.apply_hot_patch`.
    @discardableResult
    public func applyHotPatch(_ patchNvp1: Data) -> Bool {
        guard let handle, let patchFn = module.dev?.pointee.apply_hot_patch else { return false }
        let dirty = patchNvp1.withUnsafeBytes { raw -> UInt32 in
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
        resolutionSource = nil
        lastPropsValue = nil
        lastDirectProps = nil
        lastDirectPropsEqual = nil
        module = NveModule(surface: nil, layout: nil, component: nil, dev: nil, host: nil)
        if let dylibHandle {
            dlclose(dylibHandle)
            self.dylibHandle = nil
        }
    }
}
