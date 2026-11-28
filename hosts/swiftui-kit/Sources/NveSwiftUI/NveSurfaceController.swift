import CNveAbi
import Foundation
import NveHost
import NveHostAbi
import QuartzCore
import SwiftUI

/// Weak controller reference handed to runtime callbacks (`wake`, `intrinsic_size_invalidated`);
/// only dereferenced on the main queue.
private final class WeakController: @unchecked Sendable {
    weak var value: NveSurfaceController?
    init(_ value: NveSurfaceController) { self.value = value }
}

/// Measured surface dimensions and optional text baselines returned by ABI v6 `measure_surface`.
public struct NveSurfaceMeasurement: Equatable, Sendable {
    public let size: CGSize
    public let firstBaseline: CGFloat?
    public let lastBaseline: CGFloat?

    public init(size: CGSize, firstBaseline: CGFloat? = nil, lastBaseline: CGFloat? = nil) {
        self.size = size
        self.firstBaseline = firstBaseline
        self.lastBaseline = lastBaseline
    }
}

// MARK: - Surface Controller (`libnve_runtime` surface + attached `nve_get_interface` app)

/// One NVE surface hosted by `libnve_runtime`: attaches an app (static, explicit, process image,
/// or a `.dylib` the runtime loads and hot-reloads with state carried over), pairs with a remote
/// dev server, forwards props / input / hot patches / state snapshots, drives frames from the
/// runtime wake callback, negotiates Model A intrinsic sizing/baselines via ABI v6
/// `measure_surface`, and publishes the zero-copy scene through ``store``.
public final class NveSurfaceController: ObservableObject {
    private static var nextSurfaceId: UInt32 = 1
    private static let idLock = NSLock()

    public let surfaceId: UInt32
    public let session: NveRuntimeSession
    public let store: NveSceneStore
    public private(set) var resolutionSource: NveInterfaceResolutionSource?
    @Published public private(set) var isPaired = false
    /// Monotonically increments whenever the runtime fires `intrinsic_size_invalidated_callback`.
    @Published public private(set) var intrinsicSizeRevision: UInt64 = 0

    public var onEvent: ((NveEvent) -> Void)?

    /// `true` when `dispatchEvents` runs synchronously against an in-process / dylib app.
    public var dispatchesSynchronously: Bool { !isPaired }

    private var lastPropsValue: NveBinValue?
    private var lastDirectPropsEqual: ((Any) -> Bool)?
    private var inboundBatch = EventBatchBuilder()
    private var lastTick: CFTimeInterval = 0
    private var pumpScheduled = false
    private let intrinsicLock = NSLock()
    private var intrinsicInvalidationScheduled = false

    /// Creates a runtime surface and attaches the app: `getInterface` when given, else the
    /// `dylibPath` module (loaded by the runtime), else `vtableSymbol` from the static table or the
    /// process image.
    public init(
        surfaceId: UInt32? = nil,
        vtableSymbol: String = "nve_get_interface",
        getInterface: NveGetInterfaceFn? = nil,
        dylibPath: String? = nil,
        staticTable: NveStaticInterfaceTable = .shared,
        runtimeApi: UnsafePointer<NveRuntimeApi>? = NveRuntimeSession.linkedApi(),
        textMeasurer: NveTextMeasuring? = NveCoreTextMeasurer()
    ) throws {
        if let surfaceId {
            self.surfaceId = surfaceId
        } else {
            Self.idLock.lock()
            self.surfaceId = Self.nextSurfaceId
            Self.nextSurfaceId &+= 1
            Self.idLock.unlock()
        }
        session = try NveRuntimeSession(api: runtimeApi, surfaceId: self.surfaceId)
        store = NveSceneStore(session: session)
        try session.setTextMeasurer(textMeasurer)
        let weakSelf = WeakController(self)
        try session.setWakeHandler { DispatchQueue.main.async { weakSelf.value?.schedulePump() } }
        try installIntrinsicSizeInvalidationCallback()
        if getInterface == nil, let dylibPath, !dylibPath.isEmpty {
            try load(dylibPath: dylibPath)
        } else if let resolved = NveInterfaceResolver.resolve(
            symbol: vtableSymbol, getInterface: getInterface, staticTable: staticTable)
        {
            try attach(resolved)
        }
    }

    deinit {
        // Explicitly unregister and drain runtime callbacks before `session` tears down.
        _ = try? session.setIntrinsicSizeInvalidatedHandler(nil)
        _ = try? session.setWakeHandler(nil)
    }

    private func installIntrinsicSizeInvalidationCallback() throws {
        let weakSelf = WeakController(self)
        try session.setIntrinsicSizeInvalidatedHandler {
            weakSelf.value?.scheduleIntrinsicSizeInvalidation()
        }
    }

    private func scheduleIntrinsicSizeInvalidation() {
        intrinsicLock.lock()
        if intrinsicInvalidationScheduled {
            intrinsicLock.unlock()
            return
        }
        intrinsicInvalidationScheduled = true
        intrinsicLock.unlock()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.intrinsicLock.lock()
            self.intrinsicInvalidationScheduled = false
            self.intrinsicLock.unlock()
            self.intrinsicSizeRevision &+= 1
        }
    }

    // MARK: App lifecycle

    /// Attaches an in-process app (explicit, static, or process image), replacing any previous one.
    public func attach(_ resolved: NveResolvedInterface) throws {
        try session.attachApp(resolved.getInterface)
        adopt(resolved.source)
    }

    /// Loads and attaches the app `.dylib` at `dylibPath`, replacing any previous app.
    public func load(dylibPath: String) throws {
        try session.loadAppModule(path: dylibPath, carryState: false)
        adopt(.dynamicLibrary)
    }

    /// Tier-2 hot swap: the runtime loads the rebuilt `.dylib` and carries the running app's state
    /// into it before its first frame.
    public func reload(dylibPath: String) throws {
        try session.loadAppModule(path: dylibPath, carryState: true)
        adopt(.dynamicLibrary)
    }

    public func detach() throws {
        try session.attachApp(nil)
        _ = try? session.setIntrinsicSizeInvalidatedHandler(nil)
        releaseLocalApp()
    }

    private func adopt(_ source: NveInterfaceResolutionSource) {
        releaseLocalApp()
        resolutionSource = source
        _ = try? installIntrinsicSizeInvalidationCallback()
        pump()
    }

    private func releaseLocalApp() {
        resolutionSource = nil
        lastPropsValue = nil
        lastDirectPropsEqual = nil
    }

    public var isMounted: Bool { resolutionSource != nil }

    // MARK: Pairing

    public func pair(uri: String, token: String = "") throws {
        try session.pairConnect(uri: uri, token: token)
        releaseLocalApp()
        _ = try? installIntrinsicSizeInvalidationCallback()
        isPaired = session.isPaired
    }

    public func unpair() throws {
        try session.pairDisconnect()
        isPaired = false
    }

    // MARK: Props, input, dev loop

    /// Typed props (`NveDirectEncodable & Equatable`): skipped when equal to the last push.
    @discardableResult
    public func setDirectProps<P: NveDirectEncodable & Equatable>(_ props: P) -> Bool {
        if let eq = lastDirectPropsEqual, eq(props) { return false }
        guard let changed = push({ try NveDirectPropsWriter.withEncodedProps(props, $0) }) else { return false }
        lastPropsValue = nil
        lastDirectPropsEqual = { ($0 as? P) == props }
        return changed
    }

    /// Typed props without equality dedup.
    @discardableResult
    public func setDirectProps<P: NveDirectEncodable>(_ props: P) -> Bool {
        lastPropsValue = nil
        lastDirectPropsEqual = nil
        return push({ try NveDirectPropsWriter.withEncodedProps(props, $0) }) ?? false
    }

    @discardableResult
    public func setProps(_ entries: [(String, NveBinValue)]) -> Bool {
        let canonical = NveBinValue.map(entries)
        guard lastPropsValue != canonical,
              let changed = push({ try NveDirectPropsWriter.withEntries(entries, $0) })
        else { return false }
        lastPropsValue = canonical
        lastDirectPropsEqual = nil
        return changed
    }

    @discardableResult
    public func setProps(_ props: [String: Any]) -> Bool {
        guard let entries = try? NveBinValue.canonicalEntries(from: props) else { return false }
        return setProps(entries)
    }

    private func push(
        _ encode: ((UnsafePointer<NveMapEntry>?, UInt32) throws -> Bool) throws -> Bool
    ) -> Bool? {
        guard let changed = try? encode({ ptr, count in
            try session.setProps(UnsafeBufferPointer(start: ptr, count: Int(count)))
        }) else { return nil }
        pump()
        return changed
    }

    @discardableResult
    public func dispatchEvents(_ events: [NveEvent]) -> Bool {
        guard !events.isEmpty else { return false }
        inboundBatch.clear()
        for event in events { inboundBatch.push(event) }
        guard let changed = try? inboundBatch.withRawBatch({ try session.dispatch($0) }) else { return false }
        pump()
        return changed
    }

    @discardableResult
    public func dispatchEvent(_ event: NveEvent) -> Bool { dispatchEvents([event]) }

    public func dispatchCommand(name: String, value: NveBinValue, nodeId: UInt32 = 1) {
        inboundBatch.clear()
        guard (try? inboundBatch.pushCustomEvent(
            surfaceId: surfaceId, nodeId: nodeId, timestampUs: NveEvent.nowUs(), name: name, value: value
        )) != nil else { return }
        _ = try? inboundBatch.withRawBatch { try session.dispatch($0) }
        pump()
    }

    public func dispatchCommand(name: String, args: [String: Any] = [:]) {
        guard let entries = try? NveBinValue.canonicalEntries(from: args) else { return }
        dispatchCommand(name: name, value: .map(entries))
    }

    @discardableResult
    public func applyHotPatch(_ patchNvp1: Data) -> Bool {
        let changed = (try? session.applyHotPatch(patchNvp1)) ?? false
        pump()
        return changed
    }

    // MARK: Model A Layout Measurement & Baselines (ABI v6)

    /// Fallback intrinsic size used only when `measure_surface` fails or no scene is laid out yet.
    public var fallbackIntrinsicSize = CGSize(width: 240, height: 120)

    /// Queries ABI v6 `measure_surface(constraints, NVE_MEASURE_FLAG_DRY_RUN | (unspecified ? NVE_MEASURE_FLAG_IDEAL : 0))`
    /// and converts finite `first_baseline` / `last_baseline` (`NaN` -> `nil`).
    public func measureSurface(proposal: ProposedViewSize) -> NveSurfaceMeasurement {
        let isUnspecified = proposal.width == nil && proposal.height == nil
        let maxW: Float = if let w = proposal.width, w.isFinite { Float(max(0, w)) } else { .infinity }
        let maxH: Float = if let h = proposal.height, h.isFinite { Float(max(0, h)) } else { .infinity }
        let constraints = NveSizeConstraints(min_w: 0, min_h: 0, max_w: maxW, max_h: maxH)
        let flags = UInt32(NVE_MEASURE_FLAG_DRY_RUN) | (isUnspecified ? UInt32(NVE_MEASURE_FLAG_IDEAL) : 0)

        if let m = try? session.measureSurface(constraints: constraints, flags: flags),
           m.width.isFinite, m.height.isFinite
        {
            let zeroRequested = maxW == 0 && maxH == 0
            if store.frameId > 0 || m.width > 0 || m.height > 0 || zeroRequested {
                let first = m.first_baseline.isFinite ? CGFloat(m.first_baseline) : nil
                let last = m.last_baseline.isFinite ? CGFloat(m.last_baseline) : nil
                return NveSurfaceMeasurement(
                    size: CGSize(width: CGFloat(max(0, m.width)), height: CGFloat(max(0, m.height))),
                    firstBaseline: first,
                    lastBaseline: last
                )
            }
        }
        return fallbackMeasurement(proposal: proposal)
    }

    /// SwiftUI `sizeThatFits` convenience over `CGSize` (non-finite axes mean unspecified).
    public func measure(proposal: CGSize) -> CGSize {
        let proposed = ProposedViewSize(
            width: proposal.width.isFinite ? proposal.width : nil,
            height: proposal.height.isFinite ? proposal.height : nil
        )
        return measureSurface(proposal: proposed).size
    }

    private func fallbackMeasurement(proposal: ProposedViewSize) -> NveSurfaceMeasurement {
        let root = (try? session.withScene { scene -> CGSize? in
            guard let i = scene.index(of: .root), let r = scene.rect(i), r.width > 0, r.height > 0 else { return nil }
            return CGSize(width: CGFloat(r.width), height: CGFloat(r.height))
        }) ?? nil
        let natural = root ?? fallbackIntrinsicSize
        let w: CGFloat = if let pw = proposal.width, pw.isFinite { max(0, pw) } else { natural.width }
        let h: CGFloat = if let ph = proposal.height, ph.isFinite { max(0, ph) } else { natural.height }
        return NveSurfaceMeasurement(size: CGSize(width: w, height: h))
    }

    /// Propagates the hosting view's size (and display scale) to the runtime viewport.
    public func resize(_ size: CGSize, scale: CGFloat = 1) {
        guard size.width > 0, size.height > 0 else { return }
        try? session.setViewport(size, scale: scale)
        pump()
    }

    // MARK: Frames

    private func schedulePump() {
        guard !pumpScheduled else { return }
        pumpScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.pumpScheduled = false
            self?.pump()
        }
    }

    /// Ticks the runtime, absorbs a published frame, and forwards outbound app events.
    public func pump() {
        let now = CACurrentMediaTime()
        let dt = lastTick == 0 ? 0 : Float(min(now - lastTick, 0.25))
        lastTick = now
        if (try? session.tick(dt: dt)) == true { store.sync() }
        isPaired = session.isPaired
        // Always drain: undrained events would pile up in the runtime while nobody listens.
        guard let batch = try? session.drainEmittedEvents(), batch.event_count > 0, let onEvent,
              let reader = try? EventBatchReader.validate(batch: batch) else { return }
        for event in reader.decodeEvents() { onEvent(event) }
    }
}
