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

    private var module: NveModule = NveModule(surface: nil, layout: nil, component: nil, dev: nil)
    private var handle: NveAppHandle?
    private var dylibHandle: UnsafeMutableRawPointer?
    private var frameCounter: UInt64 = 0
    private var lastPropsValue: NveBinValue?
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
        self.lastPropsValue = nil
    }

    /// Pushes typed props via `NveComponentApi.set_props_direct` (zero JSON), falling back to
    /// compact binary `NveBinValue` on `set_props` only if `set_props_direct == nil`.
    @discardableResult
    public func setProps(_ entries: [(String, NveBinValue)]) -> Bool {
        let canonical = NveBinValue.map(entries)
        if lastPropsValue == canonical {
            return false
        }
        guard let handle, let component = module.component?.pointee else {
            return false
        }

        let needsRender: UInt32
        if let directFn = component.set_props_direct {
            guard let status = try? NveDirectPropsWriter.withEntries(entries, { ptr, count in
                directFn(handle, ptr, count)
            }) else {
                return false
            }
            needsRender = status
        } else if let binFn = component.set_props {
            guard let binData = try? canonical.encode() else {
                return false
            }
            needsRender = binData.withUnsafeBytes { raw -> UInt32 in
                let slice = NveBufferSlice(
                    ptr: raw.bindMemory(to: UInt8.self).baseAddress,
                    len: raw.count
                )
                return binFn(handle, slice)
            }
        } else {
            return false
        }

        lastPropsValue = canonical
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

    /// Dispatches a batch of typed events via `NveSurfaceApi.dispatch_events_direct` when
    /// available, falling back to binary `EventRing` on `dispatch_events`.
    @discardableResult
    public func dispatchEvents(_ events: [NveEvent]) -> Bool {
        guard !events.isEmpty, let handle, let surface = module.surface?.pointee else {
            return false
        }
        let needsRender: UInt32
        if let directFn = surface.dispatch_events_direct {
            guard let status = try? NveDirectEventWriter.withBatch(events, { batch in
                directFn(handle, batch)
            }) else {
                return false
            }
            needsRender = status
        } else if let ringFn = surface.dispatch_events {
            let encoded = NveEventRingCodec.encode(events)
            needsRender = encoded.withUnsafeBytes { raw -> UInt32 in
                let slice = NveBufferSlice(
                    ptr: raw.bindMemory(to: UInt8.self).baseAddress,
                    len: raw.count
                )
                return ringFn(handle, slice)
            }
        } else {
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
        guard let handle, let surface = module.surface?.pointee else { return }
        let needsRender: UInt32
        if let directFn = surface.dispatch_events_direct {
            guard let status = try? NveDirectEventWriter.withCustomEvent(
                surfaceId: surfaceId,
                nodeId: nodeId,
                timestampUs: timestampUs,
                name: name,
                value: value,
                { batch in directFn(handle, batch) }
            ) else {
                return
            }
            needsRender = status
        } else if let ringFn = surface.dispatch_events {
            guard let encoded = try? NveEventRingCodec.encodeCustomEvent(
                surfaceId: surfaceId,
                nodeId: nodeId,
                name: name,
                value: value,
                timestampUs: timestampUs
            ) else {
                return
            }
            needsRender = encoded.withUnsafeBytes { raw -> UInt32 in
                let slice = NveBufferSlice(
                    ptr: raw.bindMemory(to: UInt8.self).baseAddress,
                    len: raw.count
                )
                return ringFn(handle, slice)
            }
        } else {
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

    /// Applies a Tier-1 RSX hot-reload patch via `NveDevApi.apply_hot_patch`.
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
        lastPropsValue = nil
        module = NveModule(surface: nil, layout: nil, component: nil, dev: nil)
        if let dylibHandle {
            dlclose(dylibHandle)
            self.dylibHandle = nil
        }
    }
}
