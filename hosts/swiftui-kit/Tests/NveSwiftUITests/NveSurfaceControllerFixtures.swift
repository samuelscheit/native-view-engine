import CNveAbi
import Foundation
import NveSwiftUI

// MARK: - C-ABI Value Snapshot Helpers (mirrors Rust NveValueRef / NveDirectEventRef)

func readCString(_ slice: NveStrSlice) -> String {
    guard let ptr = slice.ptr, slice.len > 0 else { return "" }
    return String(bytes: UnsafeBufferPointer(start: ptr, count: slice.len), encoding: .utf8) ?? ""
}

func readCValue(_ value: NveValue) -> NveBinValue {
    switch value.tag {
    case UInt32(NVE_VAL_NULL.rawValue): return .null
    case UInt32(NVE_VAL_BOOL.rawValue): return .bool(value.data.bool_val != 0)
    case UInt32(NVE_VAL_I64.rawValue): return .int64(value.data.i64_val)
    case UInt32(NVE_VAL_F64.rawValue): return .float64(value.data.f64_val)
    case UInt32(NVE_VAL_STR.rawValue): return .string(readCString(value.data.str_val))
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

struct RecordedDirectEvent: Equatable {
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

let nvp1DevStatus = Data([0x4E, 0x56, 0x50, 0x31, 3, 1, 0, 0, 0, 0x78, 0, 0, 0, 0, 0, 0, 0, 0])

final class MockSurfaceState: @unchecked Sendable {
    let lock = NSLock()
    let surfaceId: UInt32
    var destroyed = false
    var destroyCount = 0
    var renderedFrameIds: [UInt64] = []
    var renderOutput = Data([0x4E, 0x56, 0x45, 0x31])
    var setPropsDirectHistory: [[(String, NveBinValue)]] = []
    var setPropsReturnValue: UInt32 = 1
    var dispatchedDirectEvents: [[RecordedDirectEvent]] = []
    var dispatchEventsReturnValue: UInt32 = 1
    var hotPatchHistory: [Data] = []
    var hotPatchReturnValue: UInt32 = 1
    var pendingOutboundEvents: [NveEvent] = []
    var drainedOutboundStorage: [NveDirectEvent] = []
    var lastConstraints: NveSizeConstraints?
    var intrinsicSize = NveIntrinsicSize(width: 180, height: 90)

    init(surfaceId: UInt32) {
        self.surfaceId = surfaceId
    }
}

final class MockAbiRegistry: @unchecked Sendable {
    static let shared = MockAbiRegistry()

    private let lock = NSLock()
    private var nextToken: Int = 0x1_0000
    private var instances: [UnsafeMutableRawPointer: MockSurfaceState] = [:]
    private var onCreateHandler: ((MockSurfaceState) -> Void)?
    private var createShouldReturnNil = false

    func configureNextCreate(returnNil: Bool = false, configure: ((MockSurfaceState) -> Void)? = nil) {
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

enum MockVtables {
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
                    if bytes.isEmpty { return NveBufferSlice(ptr: nil, len: 0) }
                    let copied = UnsafeMutablePointer<UInt8>.allocate(capacity: bytes.count)
                    bytes.copyBytes(to: copied, count: bytes.count)
                    return NveBufferSlice(ptr: UnsafePointer(copied), len: bytes.count)
                },
                dispatch_events: { handle, batch in
                    guard let state = MockAbiRegistry.shared.state(for: handle) else { return 0 }
                    var captured: [RecordedDirectEvent] = []
                    if let base = batch.ptr, batch.len > 0 {
                        for i in 0 ..< Int(batch.len) {
                            let ev = base[i]
                            captured.append(
                                RecordedDirectEvent(
                                    surfaceId: ev.surface_id, nodeId: ev.node_id, timestampUs: ev.timestamp_us,
                                    kind: ev.kind, payloadTag: ev.payload_tag, name: readCString(ev.name),
                                    x: ev.x, y: ev.y, width: ev.width, height: ev.height,
                                    text: readCString(ev.text), value: readCValue(ev.value)
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

    static let surfaceApiNilDispatch: UnsafePointer<NveSurfaceApi> = {
        let ptr = UnsafeMutablePointer<NveSurfaceApi>.allocate(capacity: 1)
        var copy = surfaceApiDirect.pointee
        copy.dispatch_events = nil
        ptr.initialize(to: copy)
        return UnsafePointer(ptr)
    }()

    static let surfaceApiNilCreate: UnsafePointer<NveSurfaceApi> = {
        let ptr = UnsafeMutablePointer<NveSurfaceApi>.allocate(capacity: 1)
        ptr.initialize(to: NveSurfaceApi(create: nil, destroy: nil, render: nil, dispatch_events: nil))
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
                set_props: { handle, entriesPtr, count in
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
                },
                drain_emitted_events: { handle in
                    guard let state = MockAbiRegistry.shared.state(for: handle) else {
                        return NveEventBatch(ptr: nil, len: 0, _pad: 0)
                    }
                    state.lock.lock()
                    let outbound = state.pendingOutboundEvents
                    state.pendingOutboundEvents = []
                    state.drainedOutboundStorage = []
                    if outbound.isEmpty {
                        state.lock.unlock()
                        return NveEventBatch(ptr: nil, len: 0, _pad: 0)
                    }
                    for ev in outbound {
                        var nameSlice = NveStrSlice(ptr: nil, len: 0)
                        if let name = ev.name, !name.isEmpty {
                            let d = Data(name.utf8)
                            let p = UnsafeMutablePointer<UInt8>.allocate(capacity: d.count)
                            d.copyBytes(to: p, count: d.count)
                            nameSlice = NveStrSlice(ptr: UnsafePointer(p), len: d.count)
                        }
                        var val = NveValue(tag: UInt32(NVE_VAL_NULL.rawValue), data: NveValueData())
                        var ptag = UInt8(NVE_EVENT_PAYLOAD_EMPTY.rawValue)
                        if case let .binValue(payloadData) = ev.payload {
                            ptag = UInt8(NVE_EVENT_PAYLOAD_CUSTOM.rawValue)
                            if !payloadData.isEmpty {
                                let p = UnsafeMutablePointer<UInt8>.allocate(capacity: payloadData.count)
                                payloadData.copyBytes(to: p, count: payloadData.count)
                                val = NveValue(
                                    tag: UInt32(NVE_VAL_BYTES.rawValue),
                                    data: NveValueData(bytes_val: NveBufferSlice(ptr: UnsafePointer(p), len: payloadData.count))
                                )
                            }
                        }
                        state.drainedOutboundStorage.append(
                            NveDirectEvent(
                                surface_id: ev.surfaceId, node_id: ev.nodeId, timestamp_us: ev.timestampUs,
                                kind: ev.kind.rawValue, payload_tag: ptag, modifiers: 0, event_flags: 0, key_code: 0,
                                name: nameSlice, x: 0, y: 0, width: 0, height: 0,
                                text: NveStrSlice(ptr: nil, len: 0), value: val
                            )
                        )
                    }
                    let count = state.drainedOutboundStorage.count
                    let outPtr = UnsafeMutablePointer<NveDirectEvent>.allocate(capacity: count)
                    for i in 0 ..< count { outPtr.advanced(by: i).initialize(to: state.drainedOutboundStorage[i]) }
                    state.lock.unlock()
                    return NveEventBatch(ptr: UnsafePointer(outPtr), len: UInt32(count), _pad: 0)
                }
            )
        )
        return UnsafePointer(ptr)
    }()

    static let componentApiNilProps: UnsafePointer<NveComponentApi> = {
        let ptr = UnsafeMutablePointer<NveComponentApi>.allocate(capacity: 1)
        var copy = componentApiDirect.pointee
        copy.set_props = nil
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

func mockFullInterface(_ id: UInt32) -> UnsafeRawPointer? {
    switch id {
    case UInt32(NVE_IFACE_SURFACE): return UnsafeRawPointer(MockVtables.surfaceApiDirect)
    case UInt32(NVE_IFACE_LAYOUT): return UnsafeRawPointer(MockVtables.layoutApi)
    case UInt32(NVE_IFACE_COMPONENT): return UnsafeRawPointer(MockVtables.componentApiDirect)
    case UInt32(NVE_IFACE_DEV): return UnsafeRawPointer(MockVtables.devApi)
    default: return nil
    }
}

func mockNilDirectFnsInterface(_ id: UInt32) -> UnsafeRawPointer? {
    switch id {
    case UInt32(NVE_IFACE_SURFACE): return UnsafeRawPointer(MockVtables.surfaceApiNilDispatch)
    case UInt32(NVE_IFACE_LAYOUT): return UnsafeRawPointer(MockVtables.layoutApi)
    case UInt32(NVE_IFACE_COMPONENT): return UnsafeRawPointer(MockVtables.componentApiNilProps)
    case UInt32(NVE_IFACE_DEV): return UnsafeRawPointer(MockVtables.devApi)
    default: return nil
    }
}

func mockSurfaceOnlyInterface(_ id: UInt32) -> UnsafeRawPointer? {
    id == UInt32(NVE_IFACE_SURFACE) ? UnsafeRawPointer(MockVtables.surfaceApiDirect) : nil
}

func mockMissingSurfaceInterface(_ id: UInt32) -> UnsafeRawPointer? {
    id == UInt32(NVE_IFACE_LAYOUT) ? UnsafeRawPointer(MockVtables.layoutApi) : nil
}

func mockNilCreateInterface(_ id: UInt32) -> UnsafeRawPointer? {
    id == UInt32(NVE_IFACE_SURFACE) ? UnsafeRawPointer(MockVtables.surfaceApiNilCreate) : nil
}
