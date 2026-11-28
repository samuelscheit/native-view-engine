import CNveAbi
import CoreGraphics
import Darwin
import Foundation
import NveHost

// The generated C structs satisfy the zero-dependency `NveHost` reader protocols unchanged.
extension NveTableView: @retroactive TableViewABI {}
extension NveNodeRow: @retroactive NodeRowABI {}
extension NveLayoutRect: @retroactive LayoutRectABI {}
extension NveScrollOffset: @retroactive ScrollOffsetABI {}
extension NveSceneView: @retroactive SceneViewABI {}

/// Validated zero-copy reader over the scene borrowed from `scene_acquire`.
public typealias NveScene = SceneReader<NveSceneView>

public enum NveRuntimeError: Error, Equatable {
    /// `nve_runtime_get_interface` is not linked into this process.
    case runtimeUnavailable
    /// The vtable does not match this build's ABI version, size, schema, or misses a slot.
    case abiMismatch
    case createFailed
    /// A slot returned a non-`NVE_RUNTIME_OK` status; `message` is `last_error`.
    case status(code: Int32, message: String)
    case scene(SceneError)

    public static let busy = NVE_RUNTIME_BUSY.rawValue
    public static let unsupported = NVE_RUNTIME_UNSUPPORTED.rawValue
    public static let notFound = NVE_RUNTIME_NOT_FOUND.rawValue

    /// `true` for `Unsupported` / `NotFound`: the attached app lacks the optional interface.
    public var isMissingCapability: Bool {
        guard case let .status(code, _) = self else { return false }
        return code == Int32(Self.unsupported) || code == Int32(Self.notFound)
    }
}

/// Host text measurement used by runtime layout (`set_text_measurer`). Runs on the owner thread
/// during `tick`; `maxWidth` is non-finite when unconstrained.
public protocol NveTextMeasuring: AnyObject {
    /// Measures `text` with the node's resolved typography (`style.font_family` is borrowed for
    /// the call); `maxWidth` is `.infinity` when unconstrained.
    func measure(text: String, style: NveRuntimeTextStyle, maxWidth: CGFloat) -> CGSize
}

public extension NveRuntimeTextStyle {
    /// Default typography at `size` / CSS `weight` (system font, `normal` line height, wrapping).
    static func sized(_ size: Float, weight: UInt16 = 400, italic: Bool = false) -> NveRuntimeTextStyle {
        var s = NveRuntimeTextStyle()
        s.font_size = size
        s.font_weight = weight
        s.font_style = UInt8(italic ? NVE_FONT_STYLE_ITALIC : NVE_FONT_STYLE_NORMAL)
        s.line_height_unit = UInt8(NVE_LINE_HEIGHT_AUTO)
        return s
    }

    var isItalic: Bool { UInt32(font_style) != NVE_FONT_STYLE_NORMAL }
}

/// One `libnve_runtime` surface: RAII over `NveRuntimeApi` + `NveRuntimeHandle`.
///
/// Owner-thread affinity: create and drive it from one thread (the main thread for SwiftUI);
/// `withScene`, `resource(_:in:)`, `metrics`, and `isPaired` are safe from any thread.
public final class NveRuntimeSession {
    public let api: UnsafePointer<NveRuntimeApi>
    public let handle: NveRuntimeHandle
    private var wakeBox: Unmanaged<WakeBox>?
    private var measurerBox: Unmanaged<MeasurerBox>?

    /// The `NveRuntimeApi` of the `libnve_runtime` linked into this process, if any.
    public static func linkedApi() -> UnsafePointer<NveRuntimeApi>? {
        guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "nve_runtime_get_interface") else {
            return nil
        }
        typealias GetInterface = @convention(c) (UInt32) -> UnsafePointer<NveRuntimeApi>?
        return unsafeBitCast(sym, to: GetInterface.self)(NVE_RUNTIME_ABI_VERSION)
    }

    /// `true` when `api` matches this build's ABI version, struct size, schema hash and has every slot.
    public static func isCompatible(_ api: UnsafePointer<NveRuntimeApi>) -> Bool {
        let a = api.pointee
        // Slot completeness is the generated `nve_runtime_api_slots_complete` (one list per ABI).
        return a.abi_version == NVE_RUNTIME_ABI_VERSION
            && Int(a.struct_size) == MemoryLayout<NveRuntimeApi>.size
            && a.schema_hash == NVE_RUNTIME_SCHEMA_HASH
            && nve_runtime_api_slots_complete(api) != 0
    }

    public init(
        api: UnsafePointer<NveRuntimeApi>? = NveRuntimeSession.linkedApi(),
        surfaceId: UInt32 = 1,
        size: CGSize = CGSize(width: 800, height: 600),
        scale: CGFloat = 1
    ) throws {
        guard let api else { throw NveRuntimeError.runtimeUnavailable }
        guard Self.isCompatible(api) else { throw NveRuntimeError.abiMismatch }
        var config = NveRuntimeConfig(
            surface_id: surfaceId, width: Float(size.width), height: Float(size.height),
            scale_factor: Float(scale), flags: 0, _reserved: 0)
        guard let handle = api.pointee.create(&config) else { throw NveRuntimeError.createFailed }
        self.api = api
        self.handle = handle
    }

    deinit {
        // `destroy` drains in-flight wake / measure callbacks before the boxes are released.
        api.pointee.destroy(handle)
        wakeBox?.release()
        measurerBox?.release()
    }

    // MARK: Lifecycle & frames

    public func setViewport(_ size: CGSize, scale: CGFloat) throws {
        try check(api.pointee.set_viewport(handle, Float(size.width), Float(size.height), Float(scale)))
    }

    /// Attaches an app module (`nve_get_interface`), or detaches with `nil`.
    public func attachApp(_ getInterface: NveGetInterfaceFn?) throws {
        try check(api.pointee.attach_app_interface(handle, getInterface))
    }

    /// Loads the app cdylib at `path` through the runtime (`load_app_module`: a private staged
    /// copy, so a rebuilt file always runs new code; the old module unloads after its app is
    /// destroyed) and attaches it, replacing the running app or pairing. `carryState` restores the
    /// running app's state into the new app before it goes live. A failure keeps the running app.
    public func loadAppModule(path: String, carryState: Bool) throws {
        var path = path
        try path.withUTF8 { p in
            try check(api.pointee.load_app_module(handle, NveStrSlice(ptr: p.baseAddress, len: p.count), carryState ? 1 : 0))
        }
    }

    public func submitNve2(_ bytes: [UInt8]) throws {
        try bytes.withUnsafeBufferPointer { try check(api.pointee.submit_nve2(handle, NveBufferSlice(ptr: $0.baseAddress, len: $0.count))) }
    }

    /// Renders, reconciles, lays out, and publishes; `true` when a new scene was published.
    @discardableResult
    public func tick(dt: Float = 0) throws -> Bool {
        var changed: UInt32 = 0
        try check(api.pointee.tick(handle, dt, &changed))
        return changed != 0
    }

    /// Dispatches host input; `true` when the app requested a re-render.
    @discardableResult
    public func dispatch(_ batch: NveEventBatch) throws -> Bool {
        var changed: UInt32 = 0
        try check(api.pointee.dispatch_events(handle, batch, &changed))
        return changed != 0
    }

    /// Topmost interactive node at `point` (`.null` when none).
    public func hitTest(_ point: CGPoint) throws -> NodeId {
        var raw: UInt32 = 0
        try check(api.pointee.hit_test(handle, Float(point.x), Float(point.y), &raw))
        return NodeId(raw: raw)
    }

    /// Scrolls `node` by a delta (zero queries) and returns the clamped absolute offset.
    @discardableResult
    public func scroll(_ node: NodeId, by delta: CGVector) throws -> CGPoint {
        var x: Float = 0
        var y: Float = 0
        try check(api.pointee.scroll_by(handle, node.raw, Float(delta.dx), Float(delta.dy), &x, &y))
        return CGPoint(x: CGFloat(x), y: CGFloat(y))
    }

    /// Runs `body` over the validated published scene; the borrow ends when `body` returns.
    public func withScene<R>(_ body: (NveScene) throws -> R) throws -> R {
        var view = NveSceneView()
        try check(api.pointee.scene_acquire(handle, &view))
        defer { _ = api.pointee.scene_release(handle) }
        let scene: NveScene
        do {
            scene = try SceneReader.open(view, expectedSchema: Key64(NVE_RUNTIME_SCHEMA_HASH))
        } catch {
            throw NveRuntimeError.scene(error)
        }
        return try body(scene)
    }

    public func metrics() throws -> NveRuntimeMetrics {
        var m = NveRuntimeMetrics()
        try check(api.pointee.get_metrics(handle, &m))
        return m
    }

    // MARK: Pairing

    public func pairConnect(uri: String, token: String = "") throws {
        var uri = uri
        var token = token
        try uri.withUTF8 { u in
            try token.withUTF8 { t in
                try check(api.pointee.pair_connect(
                    handle, NveStrSlice(ptr: u.baseAddress, len: u.count), NveStrSlice(ptr: t.baseAddress, len: t.count)))
            }
        }
    }

    public func pairDisconnect() throws { try check(api.pointee.pair_disconnect(handle)) }
    public var isPaired: Bool { api.pointee.pair_is_connected(handle) != 0 }

    // MARK: Embedded component, dev loop, resources

    /// Pushes props into the attached app; `true` when it requested a re-render.
    @discardableResult
    public func setProps(_ entries: UnsafeBufferPointer<NveMapEntry>) throws -> Bool {
        var changed: UInt32 = 0
        try check(api.pointee.set_props(handle, entries.baseAddress, UInt32(entries.count), &changed))
        return changed != 0
    }

    /// Outbound app events since the last drain, borrowed until the next owner-thread call.
    public func drainEmittedEvents() throws -> NveEventBatch {
        var batch = NveEventBatch(ptr: nil, len: 0, _pad: 0)
        try check(api.pointee.drain_emitted_events(handle, &batch))
        return batch
    }

    @discardableResult
    public func applyHotPatch(_ patchNvp1: Data) throws -> Bool {
        var changed: UInt32 = 0
        try patchNvp1.withUnsafeBytes { raw in
            let bytes = raw.bindMemory(to: UInt8.self)
            try check(api.pointee.apply_hot_patch(handle, NveBufferSlice(ptr: bytes.baseAddress, len: bytes.count), &changed))
        }
        return changed != 0
    }

    /// Copy of the attached app's state (empty when stateless).
    public func snapshotState() throws -> Data {
        var slice = NveBufferSlice(ptr: nil, len: 0)
        try check(api.pointee.snapshot_state(handle, &slice))
        guard let ptr = slice.ptr, slice.len > 0 else { return Data() }
        return Data(bytes: ptr, count: slice.len)
    }

    public func restoreState(_ state: Data) throws {
        try state.withUnsafeBytes { raw in
            let bytes = raw.bindMemory(to: UInt8.self)
            try check(api.pointee.restore_state(handle, NveBufferSlice(ptr: bytes.baseAddress, len: bytes.count)))
        }
    }

    /// Canonical payload (and kind) of resource `id` referenced by a published `Resource` field.
    /// `scene` proves the `scene_acquire` borrow the runtime's zero-copy slice lives in (it is only
    /// reachable inside `withScene`); the bytes are copied out before that borrow ends.
    public func resource(_ id: UInt32, in scene: NveScene) throws -> (kind: Key64, bytes: [UInt8]) {
        _ = scene
        var kind: UInt64 = 0
        var payload = NveBufferSlice(ptr: nil, len: 0)
        try check(api.pointee.resource_payload(handle, id, &kind, &payload))
        let bytes = payload.ptr.map { Array(UnsafeBufferPointer(start: $0, count: payload.len)) } ?? []
        return (Key64(kind), bytes)
    }

    // MARK: Callbacks

    /// Installs a wake handler, invoked on ANY thread when frames or pairing work become ready.
    /// It must only schedule work (e.g. hop to the main queue and `tick`).
    public func setWakeHandler(_ handler: (@Sendable () -> Void)?) throws {
        let next = handler.map { Unmanaged.passRetained(WakeBox($0)) }
        let status = next == nil
            ? api.pointee.set_wake_callback(handle, nil, nil)
            : api.pointee.set_wake_callback(handle, { ctx in
                guard let ctx else { return }
                Unmanaged<WakeBox>.fromOpaque(ctx).takeUnretainedValue().handler()
            }, next?.toOpaque())
        try swapBox(&wakeBox, next, status)
    }

    /// Installs a host text measurer for layout (`nil` restores the built-in heuristic).
    public func setTextMeasurer(_ measurer: NveTextMeasuring?) throws {
        let next = measurer.map { Unmanaged.passRetained(MeasurerBox($0)) }
        let status = next == nil
            ? api.pointee.set_text_measurer(handle, nil, nil)
            : api.pointee.set_text_measurer(handle, { ctx, text, style, maxWidth, outW, outH in
                guard let ctx, let style, let outW, let outH else { return }
                let s = Unmanaged<MeasurerBox>.fromOpaque(ctx).takeUnretainedValue().measure(text, style.pointee, maxWidth)
                outW.pointee = s.width
                outH.pointee = s.height
            }, next?.toOpaque())
        try swapBox(&measurerBox, next, status)
    }

    /// Commits a callback-box swap. On success the runtime has drained in-flight calls of the old
    /// box, so it may die; on failure the runtime still holds the old box and never saw `next`.
    private func swapBox<Box>(_ slot: inout Unmanaged<Box>?, _ next: Unmanaged<Box>?, _ status: Int32) throws {
        guard status == Int32(NVE_RUNTIME_OK.rawValue) else {
            next?.release()
            try check(status)
            return
        }
        slot?.release()
        slot = next
    }

    // MARK: Errors

    public var lastError: String {
        let needed = api.pointee.last_error(nil, 0)
        guard needed > 0 else { return "" }
        var buf = [UInt8](repeating: 0, count: needed + 1)
        let written = buf.withUnsafeMutableBufferPointer { api.pointee.last_error($0.baseAddress, $0.count) }
        return String(decoding: buf.prefix(min(written, needed)), as: UTF8.self)
    }

    private func check(_ status: Int32) throws {
        guard status != Int32(NVE_RUNTIME_OK.rawValue) else { return }
        throw NveRuntimeError.status(code: status, message: lastError)
    }
}

private final class WakeBox {
    let handler: @Sendable () -> Void
    init(_ handler: @escaping @Sendable () -> Void) { self.handler = handler }
}

private final class MeasurerBox {
    let measurer: NveTextMeasuring
    init(_ measurer: NveTextMeasuring) { self.measurer = measurer }

    func measure(_ text: NveStrSlice, _ style: NveRuntimeTextStyle, _ maxWidth: Float) -> (width: Float, height: Float) {
        let string = text.ptr.map { String(decoding: UnsafeBufferPointer(start: $0, count: text.len), as: UTF8.self) } ?? ""
        let size = measurer.measure(text: string, style: style, maxWidth: maxWidth.isFinite ? CGFloat(maxWidth) : .infinity)
        return (Float(size.width), Float(size.height))
    }
}
