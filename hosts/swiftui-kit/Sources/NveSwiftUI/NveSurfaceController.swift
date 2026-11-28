import CNveAbi
import NveHost
import Foundation
import QuartzCore
import SwiftUI

/// Weak controller reference handed to the runtime wake callback, which may fire on any thread;
/// it is only dereferenced on the main queue.
private final class WeakController: @unchecked Sendable {
    weak var value: NveSurfaceController?
    init(_ value: NveSurfaceController) { self.value = value }
}

// MARK: - Surface Controller (`libnve_runtime` surface + attached `nve_get_interface` app)

/// One NVE surface hosted by `libnve_runtime`: attaches an app (static, explicit, process image,
/// or a `.dylib` the runtime loads and hot-reloads with state carried over), pairs with a remote
/// dev server, forwards props / input / hot patches / state snapshots, drives frames from the
/// runtime wake callback, and publishes the zero-copy scene through ``store``. Main-thread only
/// (the runtime binds the surface to its creating thread).
public final class NveSurfaceController: ObservableObject {
    private static var nextSurfaceId: UInt32 = 1
    private static let idLock = NSLock()

    public let surfaceId: UInt32
    public let session: NveRuntimeSession
    public let store: NveSceneStore
    public private(set) var resolutionSource: NveInterfaceResolutionSource?
    @Published public private(set) var isPaired = false

    public var onEvent: ((NveEvent) -> Void)?

    private var lastPropsValue: NveBinValue?
    private var lastDirectPropsEqual: ((Any) -> Bool)?
    private var lastTick: CFTimeInterval = 0
    private var pumpScheduled = false

    /// Creates a runtime surface and attaches the app: `getInterface` when given, else the
    /// `dylibPath` module (loaded by the runtime), else `vtableSymbol` from the static table or the
    /// process image. Throws when `libnve_runtime` is not linked or incompatible, or the module
    /// does not load.
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
        if getInterface == nil, let dylibPath, !dylibPath.isEmpty {
            try load(dylibPath: dylibPath)
        } else if let resolved = NveInterfaceResolver.resolve(
            symbol: vtableSymbol, getInterface: getInterface, staticTable: staticTable)
        {
            try attach(resolved)
        }
    }

    // MARK: App lifecycle
    //
    // The runtime swaps apps atomically (a rejected one leaves the running app untouched) and owns
    // every loaded module, unloading it only after its app is destroyed: the host never `dlopen`s
    // or `dlclose`s app code.

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
        releaseLocalApp()
    }

    private func adopt(_ source: NveInterfaceResolutionSource) {
        releaseLocalApp()
        resolutionSource = source
        pump()
    }

    /// Forgets the local app the runtime no longer runs (detached, replaced, or dropped by a
    /// successful pair) and the props it was last sent.
    private func releaseLocalApp() {
        resolutionSource = nil
        lastPropsValue = nil
        lastDirectPropsEqual = nil
    }

    public var isMounted: Bool { resolutionSource != nil }

    // MARK: Pairing

    /// Pairs with a remote `nve dev` server (`nve://pair?...` or `ws(s)://...`).
    /// A successful pair atomically replaces (and drops) the attached local app; a failed one
    /// keeps it running.
    public func pair(uri: String, token: String = "") throws {
        try session.pairConnect(uri: uri, token: token)
        releaseLocalApp()
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

    /// Encodes props into a borrowed `NveMapEntry` array and pushes them through the runtime.
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
        guard !events.isEmpty,
              let changed = try? NveDirectEventWriter.withBatch(events, { try session.dispatch($0) })
        else { return false }
        pump()
        return changed
    }

    @discardableResult
    public func dispatchEvent(_ event: NveEvent) -> Bool { dispatchEvents([event]) }

    /// Imperative command as a custom event carrying a typed `NveBinValue`.
    public func dispatchCommand(name: String, value: NveBinValue, nodeId: UInt32 = 1) {
        _ = try? NveDirectEventWriter.withCustomEvent(
            surfaceId: surfaceId, nodeId: nodeId, timestampUs: NveEvent.nowUs(), name: name, value: value,
            { try session.dispatch($0) })
        pump()
    }

    public func dispatchCommand(name: String, args: [String: Any] = [:]) {
        guard let entries = try? NveBinValue.canonicalEntries(from: args) else { return }
        dispatchCommand(name: name, value: .map(entries))
    }

    /// Applies one binary `NVP1` Tier-1 hot-patch frame (as broadcast by `nve dev`).
    @discardableResult
    public func applyHotPatch(_ patchNvp1: Data) -> Bool {
        let changed = (try? session.applyHotPatch(patchNvp1)) ?? false
        pump()
        return changed
    }

    // MARK: Layout

    /// SwiftUI `sizeThatFits`: the proposal on constrained axes; unconstrained axes take the
    /// laid-out root, else `fallbackIntrinsicSize`.
    public var fallbackIntrinsicSize = CGSize(width: 240, height: 120)

    public func measure(proposal: CGSize) -> CGSize {
        let root = (try? session.withScene { scene -> CGSize? in
            guard let i = scene.index(of: .root), let r = scene.rect(i), r.width > 0, r.height > 0 else { return nil }
            return CGSize(width: CGFloat(r.width), height: CGFloat(r.height))
        }) ?? nil
        let natural = root ?? fallbackIntrinsicSize
        return CGSize(
            width: proposal.width.isFinite ? proposal.width : natural.width,
            height: proposal.height.isFinite ? proposal.height : natural.height)
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
        guard let batch = try? session.drainEmittedEvents(), batch.len > 0, let onEvent else { return }
        for event in NveDirectEventWriter.decodeBatch(batch) { onEvent(event) }
    }
}
