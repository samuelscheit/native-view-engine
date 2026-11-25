import CNveAbi
import Foundation
import NveSwiftUI
import SwiftUI
import XCTest

#if canImport(AppKit)
import AppKit
#endif

// MARK: - C-ABI Value Snapshot Helpers (mirrors Rust NveValueRef / NveDirectEventRef)

private func readCString(_ slice: NveStrSlice) -> String {
    guard let ptr = slice.ptr, slice.len > 0 else { return "" }
    return String(bytes: UnsafeBufferPointer(start: ptr, count: slice.len), encoding: .utf8) ?? ""
}

private func readCValue(_ value: NveValue) -> NveBinValue {
    switch value.tag {
    case UInt32(NVE_VAL_NULL.rawValue):
        return .null
    case UInt32(NVE_VAL_BOOL.rawValue):
        return .bool(value.data.bool_val != 0)
    case UInt32(NVE_VAL_I64.rawValue):
        return .int64(value.data.i64_val)
    case UInt32(NVE_VAL_F64.rawValue):
        return .float64(value.data.f64_val)
    case UInt32(NVE_VAL_STR.rawValue):
        return .string(readCString(value.data.str_val))
    case UInt32(NVE_VAL_BYTES.rawValue):
        let b = value.data.bytes_val
        guard let ptr = b.ptr, b.len > 0 else { return .bytes(Data()) }
        return .bytes(Data(bytes: ptr, count: b.len))
    case UInt32(NVE_VAL_ARRAY.rawValue):
        let arr = value.data.array_val
        guard let ptr = arr.ptr, arr.len > 0 else { return .array([]) }
        return .array((0 ..< arr.len).map { readCValue(ptr[$0]) })
    case UInt32(NVE_VAL_MAP.rawValue):
        let m = value.data.map_val
        guard let ptr = m.ptr, m.len > 0 else { return .map([]) }
        return .map((0 ..< m.len).map { (readCString(ptr[$0].key), readCValue(ptr[$0].value)) })
    default:
        return .null
    }
}

private struct RecordedDirectEvent: Equatable {
    let surfaceId: UInt32
    let nodeId: UInt32
    let timestampUs: UInt64
    let kind: UInt8
    let payloadTag: UInt8
    let name: String
    let x: Float
    let y: Float
    let width: Float
    let height: Float
    let text: String
    let value: NveBinValue
}

// MARK: - Isolated C-ABI Mock Fixture

/// A complete binary `NVP1` `DevStatus` frame ("x", 0 ms), as `nve dev` broadcasts it.
private let nvp1DevStatus = Data([0x4E, 0x56, 0x50, 0x31, 3, 1, 0, 0, 0, 0x78, 0, 0, 0, 0, 0, 0, 0, 0])

private final class MockSurfaceState: @unchecked Sendable {
    let lock = NSLock()
    let surfaceId: UInt32
    var destroyed = false
    var destroyCount = 0
    var renderedFrameIds: [UInt64] = []
    var renderOutput = Data([0x4E, 0x56, 0x45, 0x31])
    var setPropsHistory: [Data] = []
    var setPropsDirectHistory: [[(String, NveBinValue)]] = []
    var setPropsReturnValue: UInt32 = 1
    var dispatchedEventBuffers: [Data] = []
    var dispatchedDirectEvents: [[RecordedDirectEvent]] = []
    var dispatchEventsReturnValue: UInt32 = 1
    var hotPatchHistory: [Data] = []
    var hotPatchReturnValue: UInt32 = 1
    var pendingOutboundEvents = Data()
    var lastConstraints: NveSizeConstraints?
    var intrinsicSize = NveIntrinsicSize(width: 180, height: 90)

    init(surfaceId: UInt32) {
        self.surfaceId = surfaceId
    }
}

private final class MockAbiRegistry: @unchecked Sendable {
    static let shared = MockAbiRegistry()

    private let lock = NSLock()
    private var nextToken: Int = 0x1_0000
    private var instances: [UnsafeMutableRawPointer: MockSurfaceState] = [:]
    private var onCreateHandler: ((MockSurfaceState) -> Void)?
    private var createShouldReturnNil = false

    func configureNextCreate(
        returnNil: Bool = false,
        configure: ((MockSurfaceState) -> Void)? = nil
    ) {
        lock.lock()
        createShouldReturnNil = returnNil
        onCreateHandler = configure
        lock.unlock()
    }

    func resetNextCreate() {
        lock.lock()
        createShouldReturnNil = false
        onCreateHandler = nil
        lock.unlock()
    }

    func create(surfaceId: UInt32) -> NveAppHandle? {
        lock.lock()
        let shouldFail = createShouldReturnNil
        let configure = onCreateHandler
        createShouldReturnNil = false
        onCreateHandler = nil
        if shouldFail {
            lock.unlock()
            return nil
        }
        nextToken += 16
        let handle = UnsafeMutableRawPointer(bitPattern: nextToken)!
        let state = MockSurfaceState(surfaceId: surfaceId)
        instances[handle] = state
        lock.unlock()
        configure?(state)
        return handle
    }

    func state(for handle: NveAppHandle?) -> MockSurfaceState? {
        guard let handle else { return nil }
        lock.lock()
        let s = instances[handle]
        lock.unlock()
        return s
    }

    func destroy(handle: NveAppHandle?) {
        guard let handle else { return }
        lock.lock()
        let s = instances[handle]
        lock.unlock()
        guard let s else { return }
        s.lock.lock()
        s.destroyed = true
        s.destroyCount += 1
        s.lock.unlock()
    }
}

private enum MockVtables {
    static let surfaceApiDirect: UnsafePointer<NveSurfaceApi> = {
        let ptr = UnsafeMutablePointer<NveSurfaceApi>.allocate(capacity: 1)
        ptr.initialize(
            to: NveSurfaceApi(
                create: { sid in MockAbiRegistry.shared.create(surfaceId: sid) },
                destroy: { handle in MockAbiRegistry.shared.destroy(handle: handle) },
                render: { handle, frameId in
                    guard let state = MockAbiRegistry.shared.state(for: handle) else {
                        return NveBufferSlice(ptr: nil, len: 0)
                    }
                    state.lock.lock()
                    state.renderedFrameIds.append(frameId)
                    let bytes = state.renderOutput
                    state.lock.unlock()
                    if bytes.isEmpty {
                        return NveBufferSlice(ptr: nil, len: 0)
                    }
                    let copied = UnsafeMutablePointer<UInt8>.allocate(capacity: bytes.count)
                    bytes.copyBytes(to: copied, count: bytes.count)
                    return NveBufferSlice(ptr: UnsafePointer(copied), len: bytes.count)
                },
                dispatch_events: { handle, slice in
                    guard let state = MockAbiRegistry.shared.state(for: handle) else { return 0 }
                    let data = slice.ptr.map { Data(bytes: $0, count: slice.len) } ?? Data()
                    state.lock.lock()
                    state.dispatchedEventBuffers.append(data)
                    let dirty = state.dispatchEventsReturnValue
                    state.lock.unlock()
                    return dirty
                },
                dispatch_events_direct: { handle, batch in
                    guard let state = MockAbiRegistry.shared.state(for: handle) else { return 0 }
                    var captured: [RecordedDirectEvent] = []
                    if let base = batch.ptr, batch.len > 0 {
                        for i in 0 ..< Int(batch.len) {
                            let ev = base[i]
                            captured.append(
                                RecordedDirectEvent(
                                    surfaceId: ev.surface_id,
                                    nodeId: ev.node_id,
                                    timestampUs: ev.timestamp_us,
                                    kind: ev.kind,
                                    payloadTag: ev.payload_tag,
                                    name: readCString(ev.name),
                                    x: ev.x,
                                    y: ev.y,
                                    width: ev.width,
                                    height: ev.height,
                                    text: readCString(ev.text),
                                    value: readCValue(ev.value)
                                )
                            )
                        }
                    }
                    state.lock.lock()
                    state.dispatchedDirectEvents.append(captured)
                    let dirty = state.dispatchEventsReturnValue
                    state.lock.unlock()
                    return dirty
                }
            )
        )
        return UnsafePointer(ptr)
    }()

    static let surfaceApiFallback: UnsafePointer<NveSurfaceApi> = {
        let ptr = UnsafeMutablePointer<NveSurfaceApi>.allocate(capacity: 1)
        var copy = surfaceApiDirect.pointee
        copy.dispatch_events_direct = nil
        ptr.initialize(to: copy)
        return UnsafePointer(ptr)
    }()

    static let surfaceApiNilCreate: UnsafePointer<NveSurfaceApi> = {
        let ptr = UnsafeMutablePointer<NveSurfaceApi>.allocate(capacity: 1)
        ptr.initialize(
            to: NveSurfaceApi(
                create: nil,
                destroy: nil,
                render: nil,
                dispatch_events: nil,
                dispatch_events_direct: nil
            )
        )
        return UnsafePointer(ptr)
    }()

    static let layoutApi: UnsafePointer<NveLayoutApi> = {
        let ptr = UnsafeMutablePointer<NveLayoutApi>.allocate(capacity: 1)
        ptr.initialize(
            to: NveLayoutApi(
                measure: { handle, constraints in
                    guard let state = MockAbiRegistry.shared.state(for: handle) else {
                        return NveIntrinsicSize(width: 0, height: 0)
                    }
                    state.lock.lock()
                    state.lastConstraints = constraints
                    let size = state.intrinsicSize
                    state.lock.unlock()
                    return size
                },
                hit_test: { _, _, _, _ in UInt8(NVE_GESTURE_CLAIM.rawValue) }
            )
        )
        return UnsafePointer(ptr)
    }()

    static let componentApiDirect: UnsafePointer<NveComponentApi> = {
        let ptr = UnsafeMutablePointer<NveComponentApi>.allocate(capacity: 1)
        ptr.initialize(
            to: NveComponentApi(
                set_props: { handle, slice in
                    guard let state = MockAbiRegistry.shared.state(for: handle) else { return 0 }
                    let data = slice.ptr.map { Data(bytes: $0, count: slice.len) } ?? Data()
                    state.lock.lock()
                    state.setPropsHistory.append(data)
                    let dirty = state.setPropsReturnValue
                    state.lock.unlock()
                    return dirty
                },
                drain_emitted_events: { handle in
                    guard let state = MockAbiRegistry.shared.state(for: handle) else {
                        return NveBufferSlice(ptr: nil, len: 0)
                    }
                    state.lock.lock()
                    let outbound = state.pendingOutboundEvents
                    state.pendingOutboundEvents = Data()
                    state.lock.unlock()
                    if outbound.isEmpty {
                        return NveBufferSlice(ptr: nil, len: 0)
                    }
                    let copied = UnsafeMutablePointer<UInt8>.allocate(capacity: outbound.count)
                    outbound.copyBytes(to: copied, count: outbound.count)
                    return NveBufferSlice(ptr: UnsafePointer(copied), len: outbound.count)
                },
                set_props_direct: { handle, entriesPtr, count in
                    guard let state = MockAbiRegistry.shared.state(for: handle) else { return 0 }
                    var captured: [(String, NveBinValue)] = []
                    if let entriesPtr, count > 0 {
                        for i in 0 ..< Int(count) {
                            captured.append((readCString(entriesPtr[i].key), readCValue(entriesPtr[i].value)))
                        }
                    }
                    state.lock.lock()
                    state.setPropsDirectHistory.append(captured)
                    let dirty = state.setPropsReturnValue
                    state.lock.unlock()
                    return dirty
                }
            )
        )
        return UnsafePointer(ptr)
    }()

    static let componentApiFallback: UnsafePointer<NveComponentApi> = {
        let ptr = UnsafeMutablePointer<NveComponentApi>.allocate(capacity: 1)
        var copy = componentApiDirect.pointee
        copy.set_props_direct = nil
        ptr.initialize(to: copy)
        return UnsafePointer(ptr)
    }()

    static let devApi: UnsafePointer<NveDevApi> = {
        let ptr = UnsafeMutablePointer<NveDevApi>.allocate(capacity: 1)
        ptr.initialize(
            to: NveDevApi(
                apply_hot_patch: { handle, slice in
                    guard let state = MockAbiRegistry.shared.state(for: handle) else { return 0 }
                    let frame = slice.ptr.map { Data(bytes: $0, count: slice.len) } ?? Data()
                    state.lock.lock()
                    state.hotPatchHistory.append(frame)
                    let dirty = state.hotPatchReturnValue
                    state.lock.unlock()
                    return dirty
                },
                snapshot_state: { _ in NveBufferSlice(ptr: nil, len: 0) },
                restore_state: { _, _ in }
            )
        )
        return UnsafePointer(ptr)
    }()
}

private func mockFullInterface(_ id: UInt32) -> UnsafeRawPointer? {
    switch id {
    case UInt32(NVE_IFACE_SURFACE): return UnsafeRawPointer(MockVtables.surfaceApiDirect)
    case UInt32(NVE_IFACE_LAYOUT): return UnsafeRawPointer(MockVtables.layoutApi)
    case UInt32(NVE_IFACE_COMPONENT): return UnsafeRawPointer(MockVtables.componentApiDirect)
    case UInt32(NVE_IFACE_DEV): return UnsafeRawPointer(MockVtables.devApi)
    default: return nil
    }
}

private func mockFallbackOnlyInterface(_ id: UInt32) -> UnsafeRawPointer? {
    switch id {
    case UInt32(NVE_IFACE_SURFACE): return UnsafeRawPointer(MockVtables.surfaceApiFallback)
    case UInt32(NVE_IFACE_LAYOUT): return UnsafeRawPointer(MockVtables.layoutApi)
    case UInt32(NVE_IFACE_COMPONENT): return UnsafeRawPointer(MockVtables.componentApiFallback)
    case UInt32(NVE_IFACE_DEV): return UnsafeRawPointer(MockVtables.devApi)
    default: return nil
    }
}

private func mockSurfaceOnlyInterface(_ id: UInt32) -> UnsafeRawPointer? {
    id == UInt32(NVE_IFACE_SURFACE) ? UnsafeRawPointer(MockVtables.surfaceApiDirect) : nil
}

private func mockMissingSurfaceInterface(_ id: UInt32) -> UnsafeRawPointer? {
    id == UInt32(NVE_IFACE_LAYOUT) ? UnsafeRawPointer(MockVtables.layoutApi) : nil
}

private func mockNilCreateInterface(_ id: UInt32) -> UnsafeRawPointer? {
    id == UInt32(NVE_IFACE_SURFACE) ? UnsafeRawPointer(MockVtables.surfaceApiNilCreate) : nil
}

// MARK: - Tests

final class NveSurfaceControllerTests: XCTestCase {
    override func tearDown() {
        MockAbiRegistry.shared.resetNextCreate()
        super.tearDown()
    }

    private func makeMountedController(
        surfaceId: UInt32 = 42,
        backend: NveRenderBackend = .fabric,
        getInterface: NveGetInterfaceFn = mockFullInterface
    ) -> (NveSurfaceController, MockSurfaceState) {
        var captured: MockSurfaceState?
        MockAbiRegistry.shared.configureNextCreate { state in
            captured = state
        }
        let controller = NveSurfaceController(
            surfaceId: surfaceId,
            backend: backend,
            getInterface: getInterface
        )
        return (controller, captured!)
    }

    func testLifecycleMountDestroyReattachAndDeinit() {
        let c1 = NveSurfaceController(vtableSymbol: "nve_nonexistent_symbol_1")
        let c2 = NveSurfaceController(vtableSymbol: "nve_nonexistent_symbol_2")
        XCTAssertEqual(c2.surfaceId, c1.surfaceId + 1)
        XCTAssertFalse(c1.isMounted)
        XCTAssertFalse(c2.isMounted)

        var firstState: MockSurfaceState?
        var secondState: MockSurfaceState?
        do {
            let (controller, state1) = makeMountedController(surfaceId: 77, backend: .skia)
            firstState = state1
            XCTAssertTrue(controller.isMounted)
            XCTAssertEqual(controller.surfaceId, 77)
            XCTAssertEqual(controller.backend, .skia)
            XCTAssertEqual(state1.surfaceId, 77)

            MockAbiRegistry.shared.configureNextCreate { secondState = $0 }
            controller.attach(getInterface: mockFullInterface)
            XCTAssertTrue(state1.destroyed)
            XCTAssertEqual(state1.destroyCount, 1)
            XCTAssertTrue(controller.isMounted)
            XCTAssertNotNil(secondState)
            XCTAssertFalse(secondState?.destroyed ?? true)
        }
        XCTAssertEqual(firstState?.destroyCount, 1)
        XCTAssertTrue(secondState?.destroyed ?? false)
        XCTAssertEqual(secondState?.destroyCount, 1)

        let (controller, state) = makeMountedController(surfaceId: 88)
        controller.destroy()
        controller.destroy()
        XCTAssertFalse(controller.isMounted)
        XCTAssertEqual(state.destroyCount, 1)
    }

    func testAttachErrorStatesAndUnmountedSafeNoOps() {
        let missingSurface = NveSurfaceController(
            surfaceId: 10,
            getInterface: mockMissingSurfaceInterface
        )
        XCTAssertFalse(missingSurface.isMounted)

        let nilCreateFn = NveSurfaceController(
            surfaceId: 11,
            getInterface: mockNilCreateInterface
        )
        XCTAssertFalse(nilCreateFn.isMounted)

        MockAbiRegistry.shared.configureNextCreate(returnNil: true)
        let nullHandle = NveSurfaceController(
            surfaceId: 12,
            getInterface: mockFullInterface
        )
        XCTAssertFalse(nullHandle.isMounted)

        let badDylib = NveSurfaceController(
            surfaceId: 13,
            vtableSymbol: "nve_get_interface",
            dylibPath: "/nonexistent/path/libnve_missing.dylib"
        )
        XCTAssertFalse(badDylib.isMounted)

        XCTAssertFalse(badDylib.setProps(["title": "offline"]))
        XCTAssertTrue(badDylib.renderFrame().isEmpty)
        XCTAssertFalse(badDylib.applyHotPatch(nvp1DevStatus))
        XCTAssertTrue(badDylib.drainEmittedEvents().isEmpty)
        badDylib.dispatchCommand(name: "noop", args: ["a": 1])
    }

    func testSetPropsDirectDeduplicationDirtyRenderAndEventForwarding() throws {
        let (controller, state) = makeMountedController(surfaceId: 5)
        var forwarded: [NveEvent] = []
        controller.onEvent = { forwarded.append($0) }

        state.pendingOutboundEvents = try NveEventRingCodec.encodeCustomEvent(
            surfaceId: 5,
            nodeId: 1,
            name: "onPropsApplied",
            value: .map([("applied", .bool(true))]),
            timestampUs: 555
        )

        // Direct path invoked with sorted canonical entries, zero legacy set_props calls.
        XCTAssertTrue(controller.setProps(["b": 2, "a": 1]))
        XCTAssertTrue(state.setPropsHistory.isEmpty)
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(
            NveBinValue.map(state.setPropsDirectHistory[0]),
            NveBinValue.map([("a", .int64(1)), ("b", .int64(2))])
        )
        XCTAssertEqual(state.renderedFrameIds, [1])
        XCTAssertEqual(forwarded.count, 1)
        XCTAssertEqual(forwarded[0].name, "onPropsApplied")
        let applied = try forwarded[0].withBinValueView { $0.mapValue(forKey: "applied")?.boolValue }
        XCTAssertEqual(applied, true)

        // Identical props (regardless of key order) are deduplicated.
        XCTAssertFalse(controller.setProps(["a": 1, "b": 2]))
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(state.renderedFrameIds, [1])

        // Unsupported Swift value type (Date) is rejected without invoking C ABI.
        XCTAssertFalse(controller.setProps(["bad": Date()]))
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)

        // Clean update (set_props_direct returns 0) skips render but still drains events.
        state.setPropsReturnValue = 0
        state.pendingOutboundEvents = NveEventRingCodec.encodeCustomEvent(
            surfaceId: 5,
            nodeId: 2,
            name: "onCleanAck",
            payload: Data(),
            timestampUs: 777
        )
        XCTAssertFalse(controller.setProps(["a": 2]))
        XCTAssertEqual(state.setPropsDirectHistory.count, 2)
        XCTAssertEqual(state.renderedFrameIds, [1])
        XCTAssertEqual(forwarded.count, 2)
        XCTAssertEqual(forwarded[1].name, "onCleanAck")
    }

    func testFallbackToBinaryNveBinValueWhenDirectPointersAreNil() throws {
        let (controller, state) = makeMountedController(
            surfaceId: 8,
            getInterface: mockFallbackOnlyInterface
        )

        XCTAssertTrue(controller.setProps(["count": 42, "label": "bin"]))
        XCTAssertTrue(state.setPropsDirectHistory.isEmpty)
        XCTAssertEqual(state.setPropsHistory.count, 1)
        try NveBinValueView.withParsed(state.setPropsHistory[0]) { view in
            XCTAssertEqual(view.mapValue(forKey: "count")?.int64Value, 42)
            XCTAssertEqual(view.mapValue(forKey: "label")?.stringValue, "bin")
        }

        controller.dispatchCommand(name: "reset", args: ["animated": true])
        XCTAssertTrue(state.dispatchedDirectEvents.isEmpty)
        XCTAssertEqual(state.dispatchedEventBuffers.count, 1)
        let decoded = try NveEventRingCodec.decodeStrict(state.dispatchedEventBuffers[0])
        XCTAssertEqual(decoded.count, 1)
        XCTAssertEqual(decoded[0].name, "reset")
        let animated = try decoded[0].withBinValueView { $0.mapValue(forKey: "animated")?.boolValue }
        XCTAssertEqual(animated, true)
    }

    func testDispatchCommandDirectRenderFrameAndHotPatch() throws {
        let (controller, state) = makeMountedController(surfaceId: 9)
        var receivedEvents: [NveEvent] = []
        controller.onEvent = { receivedEvents.append($0) }

        state.pendingOutboundEvents = try NveEventRingCodec.encodeCustomEvent(
            surfaceId: 9,
            nodeId: 4,
            name: "onResetComplete",
            value: .map([("count", .int64(0))]),
            timestampUs: 1001
        )
        controller.dispatchCommand(name: "reset", args: ["animated": true])

        XCTAssertTrue(state.dispatchedEventBuffers.isEmpty)
        XCTAssertEqual(state.dispatchedDirectEvents.count, 1)
        XCTAssertEqual(state.dispatchedDirectEvents[0].count, 1)
        let ev = state.dispatchedDirectEvents[0][0]
        XCTAssertEqual(ev.surfaceId, 9)
        XCTAssertEqual(ev.kind, NveEventKind.custom.rawValue)
        XCTAssertEqual(ev.payloadTag, UInt8(NVE_EVENT_PAYLOAD_CUSTOM.rawValue))
        XCTAssertEqual(ev.name, "reset")
        XCTAssertEqual(ev.value, .map([("animated", .bool(true))]))
        XCTAssertEqual(state.renderedFrameIds, [1])
        XCTAssertEqual(receivedEvents.count, 1)
        XCTAssertEqual(receivedEvents[0].name, "onResetComplete")

        // Clean command -> does not trigger render.
        state.dispatchEventsReturnValue = 0
        controller.dispatchCommand(name: "ping")
        XCTAssertEqual(state.dispatchedDirectEvents.count, 2)
        XCTAssertEqual(state.renderedFrameIds, [1])

        // Explicit renderFrame increments frame counter and returns slice bytes.
        state.renderOutput = Data([1, 2, 3, 4, 5])
        XCTAssertEqual(controller.renderFrame(), Data([1, 2, 3, 4, 5]))
        XCTAssertEqual(state.renderedFrameIds, [1, 2])

        state.renderOutput = Data()
        XCTAssertTrue(controller.renderFrame().isEmpty)
        XCTAssertEqual(state.renderedFrameIds, [1, 2, 3])

        // Hot patch: dirty renders, clean skips render.
        state.hotPatchReturnValue = 1
        XCTAssertTrue(controller.applyHotPatch(nvp1DevStatus))
        XCTAssertEqual(state.hotPatchHistory, [nvp1DevStatus], "NVP1 bytes forwarded unchanged")
        XCTAssertEqual(state.renderedFrameIds, [1, 2, 3, 4])

        state.hotPatchReturnValue = 0
        XCTAssertFalse(controller.applyHotPatch(Data(nvp1DevStatus.dropLast())))
        XCTAssertEqual(state.renderedFrameIds, [1, 2, 3, 4])
    }

    func testOptionalInterfacesMissingGracefulFallback() {
        let (controller, state) = makeMountedController(
            surfaceId: 33,
            getInterface: mockSurfaceOnlyInterface
        )
        XCTAssertTrue(controller.isMounted)
        XCTAssertFalse(controller.setProps(["k": "v"]))
        XCTAssertTrue(controller.drainEmittedEvents().isEmpty)
        XCTAssertFalse(controller.applyHotPatch(nvp1DevStatus))

        XCTAssertEqual(controller.renderFrame(), state.renderOutput)
        XCTAssertEqual(controller.measure(), CGSize(width: 240, height: 120))
    }

    func testLayoutConstraintNegotiationAndClamping() throws {
        let (controller, state) = makeMountedController(surfaceId: 14)

        state.intrinsicSize = NveIntrinsicSize(width: 180, height: 90)
        let normal = controller.measure(minWidth: 50, minHeight: 40, maxWidth: 300, maxHeight: 200)
        XCTAssertEqual(normal, CGSize(width: 180, height: 90))
        let constraints = try XCTUnwrap(state.lastConstraints)
        XCTAssertEqual(constraints.min_w, 50)
        XCTAssertEqual(constraints.min_h, 40)
        XCTAssertEqual(constraints.max_w, 300)
        XCTAssertEqual(constraints.max_h, 200)

        state.intrinsicSize = NveIntrinsicSize(width: 10, height: 5)
        let clampedUp = controller.measure(minWidth: 80, minHeight: 44, maxWidth: 300, maxHeight: 200)
        XCTAssertEqual(clampedUp, CGSize(width: 80, height: 44))

        state.intrinsicSize = NveIntrinsicSize(width: 900, height: 700)
        let clampedDown = controller.measure(minWidth: 0, minHeight: 0, maxWidth: 320, maxHeight: 160)
        XCTAssertEqual(clampedDown, CGSize(width: 320, height: 160))

        state.intrinsicSize = NveIntrinsicSize(width: 420, height: 210)
        let unbounded = controller.measure(minWidth: 100, minHeight: 50, maxWidth: .infinity, maxHeight: .infinity)
        XCTAssertEqual(unbounded, CGSize(width: 420, height: 210))
        let unboundedConstraints = try XCTUnwrap(state.lastConstraints)
        XCTAssertTrue(unboundedConstraints.max_w.isInfinite)
        XCTAssertTrue(unboundedConstraints.max_h.isInfinite)

        controller.destroy()
        XCTAssertEqual(controller.measure(), CGSize(width: 240, height: 120))
        controller.setFallbackIntrinsicSize(CGSize(width: 310, height: 140))
        XCTAssertEqual(controller.measure(), CGSize(width: 310, height: 140))
        XCTAssertEqual(
            controller.measure(minWidth: 350, minHeight: 200, maxWidth: .infinity, maxHeight: .infinity),
            CGSize(width: 350, height: 200)
        )
        XCTAssertEqual(
            controller.measure(minWidth: 100, minHeight: 50, maxWidth: 200, maxHeight: 80),
            CGSize(width: 200, height: 80)
        )
    }

    func testRepresentableCoordinatorOwnership() {
        let (externalController, state) = makeMountedController(surfaceId: 50)
        do {
            let representable = NveSurfaceRepresentable(
                componentName: "NveCounterCard",
                props: ["count": 1],
                controller: externalController
            )
            let coordinator = representable.makeCoordinator()
            _ = coordinator
        }
        XCTAssertTrue(externalController.isMounted)
        XCTAssertFalse(state.destroyed)

        do {
            let internalRepresentable = NveSurfaceRepresentable(
                componentName: "NveOwnedCard",
                vtableSymbol: "nve_missing_symbol_owned"
            )
            _ = internalRepresentable.makeCoordinator()
        }
    }

    #if canImport(AppKit)
    @MainActor
    func testMacOSHostContainerViewAndRepresentableLayout() {
        let (controller, state) = makeMountedController(surfaceId: 61)
        state.intrinsicSize = NveIntrinsicSize(width: 220, height: 96)

        let container = NveHostContainerView(frame: CGRect(x: 0, y: 0, width: 220, height: 96))
        container.controller = controller
        XCTAssertTrue(container.controller === controller)

        let view = NveSurfaceRepresentable(
            componentName: "NveCounterCard",
            props: ["title": "macOS Test", "count": 3],
            controller: controller
        )
        let hostingView = NSHostingView(rootView: view)
        hostingView.frame = CGRect(x: 0, y: 0, width: 300, height: 200)
        hostingView.layoutSubtreeIfNeeded()

        XCTAssertEqual(hostingView.fittingSize, CGSize(width: 220, height: 96))
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(
            NveBinValue.map(state.setPropsDirectHistory[0]),
            NveBinValue.map([
                ("count", .int64(3)),
                ("title", .string("macOS Test")),
            ])
        )
    }

    @MainActor
    func testMacOSRepresentableDirectPropsZeroBoxingPath() {
        struct CardProps: Equatable, NveDirectEncodable {
            var title: String
            var count: Int64
            var directPropCount: Int { 2 }
            func encodeDirect(into builder: inout NveDirectPropsBuilder) throws {
                builder.write("title", title)
                builder.write("count", count)
            }
        }

        let (controller, state) = makeMountedController(surfaceId: 62)
        let view = NveSurfaceRepresentable(
            componentName: "NveCounterCard",
            directProps: CardProps(title: "TypedView", count: 11),
            controller: controller
        )
        let hostingView = NSHostingView(rootView: view)
        hostingView.frame = CGRect(x: 0, y: 0, width: 200, height: 100)
        hostingView.layoutSubtreeIfNeeded()

        XCTAssertTrue(state.setPropsHistory.isEmpty)
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(
            NveBinValue.map(state.setPropsDirectHistory[0]),
            NveBinValue.map([("title", .string("TypedView")), ("count", .int64(11))])
        )
    }
    #endif

    func testSetDirectPropsEquatableDeduplicationCleanUpdateFallbackAndCacheInvalidation() throws {
        struct TypedProps: Equatable, NveDirectEncodable {
            var count: Int64
            var label: String
            var directPropCount: Int { 2 }
            func encodeDirect(into builder: inout NveDirectPropsBuilder) throws {
                builder.write("count", count)
                builder.write("label", label)
            }
        }

        let (controller, state) = makeMountedController(surfaceId: 71)
        let p1 = TypedProps(count: 1, label: "first")
        XCTAssertTrue(controller.setDirectProps(p1))
        XCTAssertTrue(state.setPropsHistory.isEmpty)
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(state.renderedFrameIds, [1])

        // Identical Equatable value skips FFI & render completely.
        XCTAssertFalse(controller.setDirectProps(TypedProps(count: 1, label: "first")))
        XCTAssertEqual(state.setPropsDirectHistory.count, 1)
        XCTAssertEqual(state.renderedFrameIds, [1])

        // Clean update (return 0) still updates dedup state and skips render.
        state.setPropsReturnValue = 0
        let p2 = TypedProps(count: 2, label: "clean")
        XCTAssertFalse(controller.setDirectProps(p2))
        XCTAssertEqual(state.setPropsDirectHistory.count, 2)
        XCTAssertEqual(state.renderedFrameIds, [1])
        XCTAssertFalse(controller.setDirectProps(p2))
        XCTAssertEqual(state.setPropsDirectHistory.count, 2)

        // Switching to dynamic setProps invalidates direct cache so re-sending p2 succeeds.
        state.setPropsReturnValue = 1
        XCTAssertTrue(controller.setProps(["count": 99]))
        XCTAssertEqual(state.setPropsDirectHistory.count, 3)
        XCTAssertTrue(controller.setDirectProps(p2))
        XCTAssertEqual(state.setPropsDirectHistory.count, 4)

        // Fallback vtable (set_props_direct == nil) still encodes NveDirectEncodable to binary NveBinValue.
        let (fallbackCtrl, fallbackState) = makeMountedController(
            surfaceId: 72,
            getInterface: mockFallbackOnlyInterface
        )
        XCTAssertTrue(fallbackCtrl.setDirectProps(TypedProps(count: 55, label: "fallback")))
        XCTAssertTrue(fallbackState.setPropsDirectHistory.isEmpty)
        XCTAssertEqual(fallbackState.setPropsHistory.count, 1)
        try NveBinValueView.withParsed(fallbackState.setPropsHistory[0]) { view in
            XCTAssertEqual(view.mapValue(forKey: "count")?.int64Value, 55)
            XCTAssertEqual(view.mapValue(forKey: "label")?.stringValue, "fallback")
        }
    }
}
